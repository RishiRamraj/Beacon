//! The window and the winit event loop: a thin device shell over [`Session`].
//!
//! Everything that runs the game lives in the session. This module's whole job
//! is to open a window, turn keyboard and gamepad events into session calls, and
//! blit the framebuffer. Nothing about how the game behaves lives here, which is
//! what lets the same session be driven headless by the MCP server instead.

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::input::{self, Input};
use crate::session::Session;

/// Default window scale over the SNES's 256x224.
const DEFAULT_SCALE: u32 = 3;

pub struct App {
    session: Session,
    input: Input,

    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    context: Option<softbuffer::Context<Rc<Window>>>,
}

impl App {
    pub fn new(session: Session, input: Input) -> Self {
        App {
            session,
            input,
            window: None,
            surface: None,
            context: None,
        }
    }

    /// Exits if the session asked to quit while handling an action.
    fn honour_quit(&self, event_loop: &ActiveEventLoop) {
        if self.session.quit_requested() {
            event_loop.exit();
        }
    }

    /// Translates a keyboard key, while the configuration is open, to a session
    /// call. Arrows, escape, and delete drive the modal; anything else binds.
    fn config_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Escape => self.session.config_close(),
            KeyCode::ArrowDown => self.session.config_navigate(1),
            KeyCode::ArrowUp => self.session.config_navigate(-1),
            KeyCode::Delete | KeyCode::Backspace => self.session.config_clear(),
            other => match input::key_name(other) {
                Some(name) => self.session.config_bind(name),
                None => self.session.say_now("That key can't be bound."),
            },
        }
    }

    /// Translates a gamepad button, while the configuration is open. The d-pad
    /// navigates and Start finishes, so the modal works from the controller.
    fn config_pad(&mut self, name: &str) {
        match name {
            "Pad:DPadDown" => self.session.config_navigate(1),
            "Pad:DPadUp" => self.session.config_navigate(-1),
            "Pad:Start" => self.session.config_close(),
            _ => self.session.config_bind(name),
        }
    }

    /// Scales the emulator framebuffer into the window, nearest neighbour.
    fn present(&mut self) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else {
            return;
        };

        let size = window.inner_size();
        let (Some(win_w), Some(win_h)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };

        if surface.resize(win_w, win_h).is_err() {
            return;
        }

        let info = self.session.frame_info();
        let (src_w, src_h) = (info.width as usize, info.height as usize);
        if src_w == 0 || src_h == 0 {
            return;
        }

        // `pitch` is a byte stride; the framebuffer is 32-bit pixels.
        let stride = (info.pitch as usize / 4).max(src_w);
        let src = self.session.framebuffer();

        let Ok(mut buf) = surface.buffer_mut() else {
            return;
        };

        let (dst_w, dst_h) = (size.width as usize, size.height as usize);
        for y in 0..dst_h {
            let sy = y * src_h / dst_h;
            let row = sy * stride;
            for x in 0..dst_w {
                let sx = x * src_w / dst_w;
                buf[y * dst_w + x] = src.get(row + sx).copied().unwrap_or(0);
            }
        }

        let _ = buf.present();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Beacon")
            .with_inner_size(winit::dpi::LogicalSize::new(
                256 * DEFAULT_SCALE,
                224 * DEFAULT_SCALE,
            ));

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("could not create window: {e}");
                event_loop.exit();
                return;
            }
        };

        match softbuffer::Context::new(Rc::clone(&window))
            .and_then(|ctx| softbuffer::Surface::new(&ctx, Rc::clone(&window)).map(|s| (ctx, s)))
        {
            Ok((ctx, surface)) => {
                self.context = Some(ctx);
                self.surface = Some(surface);
            }
            Err(e) => {
                eprintln!("could not create drawing surface: {e}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
        self.session.say_now("Beacon ready.");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if self.session.in_config() {
                        if pressed {
                            self.config_key(code);
                        }
                    } else {
                        self.input.on_key(code, pressed);
                        // Actions fire on press, and never for a game key, so the
                        // two keyspaces cannot contend.
                        if pressed && !input::is_game_button(code) {
                            if let Some(name) = input::key_name(code) {
                                self.session.resolve_action(name);
                                self.honour_quit(event_loop);
                            }
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => self.present(),

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Poll the pad once per wake, before running frames. This must happen
        // regardless of pause or mode, so a controller-only player can act,
        // step, and reach the configuration without a keyboard.
        for name in self.input.poll_gamepad() {
            if self.session.in_config() {
                self.config_pad(name);
            } else if !input::is_game_pad_name(name) {
                // Game buttons are held state, handled by the frame loop; only
                // the pad's extra buttons resolve to actions.
                self.session.resolve_action(name);
            }
        }
        self.honour_quit(event_loop);

        // While configuring, keyboard key-ups route to the modal rather than to
        // button state, so held bits would otherwise linger. Zero them, so a key
        // held when the modal opened does not stick down on resume.
        if self.session.in_config() {
            self.input.clear_keyboard();
        }
        self.session.set_held_buttons(self.input.buttons());
        self.session.run_frames();

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
