//! The window and the winit event loop: a thin device shell over [`Session`].
//!
//! Everything that runs the game lives in the session. This module's whole job
//! is to open a window, turn keyboard and gamepad events into session calls, and
//! blit the framebuffer. Nothing about how the game behaves lives here, which is
//! what lets the same session be driven headless by the MCP server instead.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;
use std::sync::mpsc::Receiver;

use accesskit_winit::{Adapter, Event as AccessEvent, WindowEvent as AccessWindowEvent};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::access;
use crate::input::{self, Input};
use crate::mcp;
use crate::session::Session;

/// Default window scale over the SNES's 256x224.
const DEFAULT_SCALE: u32 = 3;

pub struct App {
    session: Session,
    input: Input,
    /// When present, an agent is attached over the control socket; its tool calls
    /// arrive here and are run against the session each event-loop wake.
    control_rx: Option<Receiver<mcp::Request>>,

    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    context: Option<softbuffer::Context<Rc<Window>>>,
    /// Show only the map, no game picture (`--map-only`).
    map_only: bool,
    /// Tracks the map's visibility so the window is resized when it toggles.
    map_shown: bool,

    /// Publishes Beacon's accessibility tree, so the platform's own screen reader can
    /// read the menu. `None` until the window exists.
    access: Option<Adapter>,
    /// Handed to the adapter so it can wake the event loop when assistive technology
    /// asks for the tree or requests an action.
    access_proxy: EventLoopProxy<AccessEvent>,
    /// The platform's own menu bar, where the platform has one. Kept alive because dropping
    /// it takes the menu with it.
    #[cfg(any(windows, target_os = "macos"))]
    native: Option<crate::native::NativeMenu>,
    /// Every label the bar was built from, so it can be rebuilt when they change.
    #[cfg(any(windows, target_os = "macos"))]
    native_sig: Vec<String>,
    /// Whether the last attempt to build the bar failed, so it is not retried every wake.
    #[cfg(any(windows, target_os = "macos"))]
    native_failed: bool,
    /// Whether the window has been shown yet. It is created hidden and shown one iteration
    /// later: see the note in `resumed`.
    shown: bool,
    /// Set when something other than a new frame means the window needs painting again.
    redraw_wanted: bool,
    /// Whether the window has the keyboard, as last reported. `None` until it is first told.
    focused: Option<bool>,
    /// What was last published, so the tree is only pushed when it changes. Compared rather
    /// than diffed because a tree update is cheap and a comparison is cheaper than working
    /// out what moved.
    published: Published,
}

/// The base game panel size, the SNES 256x224 scaled up.
const GAME_W: u32 = 256 * DEFAULT_SCALE;
const GAME_H: u32 = 224 * DEFAULT_SCALE;

/// The window size for the current presentation: the map alone (square), the
/// game plus a square map panel beside it, or the game alone.
fn window_size(map_only: bool, map_beside: bool) -> winit::dpi::LogicalSize<u32> {
    if map_only {
        winit::dpi::LogicalSize::new(GAME_H, GAME_H)
    } else if map_beside {
        winit::dpi::LogicalSize::new(GAME_W + GAME_H, GAME_H)
    } else {
        winit::dpi::LogicalSize::new(GAME_W, GAME_H)
    }
}

/// Nearest-neighbour scales a source image into a rectangle of the destination.
#[allow(clippy::too_many_arguments)]
fn blit(
    dst: &mut [u32],
    dst_w: usize,
    x0: usize,
    y0: usize,
    dw: usize,
    dh: usize,
    src: &[u32],
    sw: usize,
    sh: usize,
    stride: usize,
) {
    // An empty source means there is nothing to draw — the emulator has been told not to
    // compose a picture — and the destination has already been cleared, so leave it black
    // rather than scaling nothing into it a pixel at a time.
    if dw == 0 || dh == 0 || sw == 0 || sh == 0 || src.is_empty() {
        return;
    }
    for y in 0..dh {
        let row = (y * sh / dh) * stride;
        for x in 0..dw {
            let px = src.get(row + x * sw / dw).copied().unwrap_or(0);
            if let Some(d) = dst.get_mut((y0 + y) * dst_w + (x0 + x)) {
                *d = px;
            }
        }
    }
}

impl App {
    pub fn new(
        session: Session,
        input: Input,
        map_only: bool,
        control_rx: Option<Receiver<mcp::Request>>,
        access_proxy: EventLoopProxy<AccessEvent>,
    ) -> Self {
        App {
            session,
            input,
            control_rx,
            window: None,
            surface: None,
            context: None,
            map_only,
            map_shown: false,
            access: None,
            access_proxy,
            #[cfg(any(windows, target_os = "macos"))]
            native: None,
            #[cfg(any(windows, target_os = "macos"))]
            native_sig: Vec::new(),
            #[cfg(any(windows, target_os = "macos"))]
            native_failed: false,
            shown: false,
            redraw_wanted: true,
            focused: None,
            published: Published::default(),
        }
    }

    /// Carries out an action a screen reader asked for.
    ///
    /// Its user's own habits, not Beacon's keys: clicking an item, or expanding one, or
    /// moving the reader's focus. Each is the same verb the keyboard drives, so the menu
    /// behaves identically however it is reached — including still speaking in Beacon's
    /// voice, which is what keeps it usable with the reader turned off.
    fn access_action(&mut self, request: accesskit::ActionRequest) {
        use accesskit::Action;
        // A row of the input configuration. Focusing or clicking one moves the cursor to it,
        // and nothing more: what a row does is bind the NEXT key pressed, which is not
        // something a reader can ask for on the player's behalf.
        if let Some(row) = access::row_index(request.target_node) {
            if matches!(request.action, Action::Focus | Action::Click) {
                self.session.config_select(row);
            }
            return;
        }
        let Some(index) = access::entry_index(request.target_node) else {
            return;
        };
        match request.action {
            Action::Focus => self.session.menu_select(index),
            Action::Click | Action::Expand => {
                self.session.menu_select(index);
                self.session.menu_choose();
            }
            Action::Collapse => self.session.menu_back(),
            _ => {}
        }
    }

    /// Builds the platform's menu bar, or rebuilds it when its labels have changed.
    ///
    /// Rebuilt rather than built once, because what it lists arrives later than the window
    /// does: Beacon is normally started with no ROM, so the plugin's commands and the save
    /// slots do not exist when the bar is first attached. Built once, the Game level was
    /// permanently empty — which is how it shipped — and the slots permanently wrong.
    #[cfg(any(windows, target_os = "macos"))]
    fn sync_native_menu(&mut self) {
        let tree = crate::menu::full(&self.session.menu_context());
        let sig = crate::native::signature(&tree);
        // A failed attempt counts as an attempt. Without that, a platform that will never give
        // up a menu bar is asked again on every wake, sixty times a second, complaining each
        // time — so the failure is reported once per thing there was to build.
        if (self.native.is_some() || self.native_failed) && sig == self.native_sig {
            return;
        }
        self.native_sig = sig;
        self.native_failed = false;

        // The old bar goes first, and this is not tidiness. Dropping a muda menu calls
        // SetMenu(hwnd, NULL) for every window it was attached to, so attaching the new one
        // and letting the assignment drop the old ended with the new bar torn straight back
        // off — the menu vanished the moment anything changed what it listed, which is any
        // toggle, any save, any ROM opened.
        self.native = None;

        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let Some(window) = self.window.as_ref() else {
                return;
            };
            match window.window_handle().map(|h| h.as_raw()) {
                // Safe: the handle came from the window this App is holding alive.
                Ok(RawWindowHandle::Win32(win32)) => {
                    let built =
                        unsafe { crate::native::NativeMenu::attach(win32.hwnd.get(), &tree) };
                    match built {
                        Ok(menu) => self.native = Some(menu),
                        Err(e) => {
                            self.native_failed = true;
                            eprintln!("could not attach the menu bar: {e}");
                        }
                    }
                }
                Ok(other) => {
                    self.native_failed = true;
                    eprintln!("no menu bar: unexpected window handle {other:?}");
                }
                Err(e) => {
                    self.native_failed = true;
                    eprintln!("no menu bar: {e}");
                }
            }
        }
        #[cfg(target_os = "macos")]
        match crate::native::NativeMenu::attach_app(&tree) {
            Ok(menu) => self.native = Some(menu),
            Err(e) => {
                self.native_failed = true;
                eprintln!("could not install the application menu: {e}");
            }
        }
    }

    /// Beacon's accessibility tree as it stands: the window, and the menu if one is open.
    fn tree(&self) -> accesskit::TreeUpdate {
        let size = self
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width as f64, s.height as f64)
            })
            .unwrap_or((0.0, 0.0));
        let announcement = self.session.announcement().map(str::to_string);
        // The dialog first: it is modal, and while it is up nothing behind it can be reached.
        if let Some(view) = self.session.config_view() {
            return access::dialog_tree("Beacon", size, &view, announcement.as_deref());
        }
        match self.session.menu() {
            Some(menu) => access::menu_tree("Beacon", size, menu, announcement.as_deref()),
            None => access::window_only("Beacon", size, announcement.as_deref()),
        }
    }

    /// Publishes the tree if the menu has changed since it was last published.
    ///
    /// Called once per wake rather than at each place the menu is touched, so a change
    /// made by an agent over the control socket reaches the screen reader the same way a
    /// keypress does. Nothing is published while no assistive technology is listening —
    /// `update_if_active` is what decides that, not this.
    fn publish_access(&mut self) {
        // The announcement is part of what is published, so a new line reaches the reader
        // even when the menu has not moved. It carries its own counter, so saying the same
        // thing twice still counts as a change.
        let said = self.session.announcement().map(str::to_string);
        let now = self.session.menu().map(|menu| {
            (
                menu.title().to_string(),
                menu.shown()
                    .into_iter()
                    .map(|e| e.label)
                    .collect::<Vec<_>>(),
                menu.selected(),
            )
        });
        // The dialog's rows change as bindings are made, so its contents are compared, not
        // merely whether it is open.
        let dialog = self
            .session
            .config_view()
            .map(|view| (view.rows, view.selected));
        let state = Published {
            menu: now,
            dialog,
            said,
        };
        if state == self.published {
            return;
        }
        self.published = state;
        let update = self.tree();
        if let Some(adapter) = self.access.as_mut() {
            adapter.update_if_active(|| update);
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

    /// Translates a keyboard key, while the menu is open, to a session call.
    ///
    /// Nothing here binds or falls through to the game: a key with no meaning in the
    /// menu is answered rather than swallowed, because a keypress that does nothing
    /// and says nothing is indistinguishable from the menu having crashed.
    fn menu_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Escape => self.session.menu_close(),
            KeyCode::ArrowDown => self.session.menu_navigate(1),
            KeyCode::ArrowUp => self.session.menu_navigate(-1),
            KeyCode::ArrowRight | KeyCode::Enter | KeyCode::NumpadEnter => {
                self.session.menu_choose()
            }
            KeyCode::ArrowLeft | KeyCode::Backspace => self.session.menu_back(),
            _ => self
                .session
                .say_now("Up and down to move, right to choose, left to go back."),
        }
    }

    /// Translates a gamepad button, while the menu is open, so the whole menu works
    /// from a controller alone — including reaching the ROM list and quitting.
    fn menu_pad(&mut self, name: &str) {
        match name {
            "Pad:DPadDown" => self.session.menu_navigate(1),
            "Pad:DPadUp" => self.session.menu_navigate(-1),
            "Pad:DPadRight" | "Pad:South" => self.session.menu_choose(),
            "Pad:DPadLeft" | "Pad:East" => self.session.menu_back(),
            "Pad:Start" => self.session.menu_close(),
            _ => {}
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

        let (win_w, win_h) = (size.width as usize, size.height as usize);

        // The game picture. Empty when the emulator has been told not to draw one, so the
        // panel stays black rather than showing whichever frame was last composed — and the
        // scaling work is skipped too, which is the point of turning it off.
        let info = self.session.frame_info();
        // `pitch` is a byte stride; the framebuffer is 32-bit pixels.
        let game_stride = (info.pitch as usize / 4).max(info.width as usize);
        let (gw, gh) = (info.width as usize, info.height as usize);
        let game: &[u32] = if self.session.video_enabled() {
            self.session.framebuffer()
        } else {
            &[]
        };

        // The map, if shown, sits in a square panel to the right of the game.
        let map = self.session.map_view();

        let Ok(mut buf) = surface.buffer_mut() else {
            return;
        };
        buf.fill(0); // letterbox any area a panel does not cover

        match (self.map_only, map) {
            // Map only: it fills the window; no game picture.
            (true, Some((mw, mh, mpix))) => {
                let (mw, mh) = (mw as usize, mh as usize);
                blit(&mut buf, win_w, 0, 0, win_w, win_h, mpix, mw, mh, mw);
            }
            // Game with the map in a square panel to its right.
            (false, Some((mw, mh, mpix))) => {
                let map_side = win_h.min(win_w / 2);
                let game_w = win_w - map_side;
                blit(
                    &mut buf,
                    win_w,
                    0,
                    0,
                    game_w,
                    win_h,
                    game,
                    gw,
                    gh,
                    game_stride,
                );
                let map_y = (win_h - map_side) / 2;
                let (mw, mh) = (mw as usize, mh as usize);
                blit(
                    &mut buf, win_w, game_w, map_y, map_side, map_side, mpix, mw, mh, mw,
                );
            }
            // No map: the game fills the window.
            (_, None) => {
                blit(
                    &mut buf,
                    win_w,
                    0,
                    0,
                    win_w,
                    win_h,
                    game,
                    gw,
                    gh,
                    game_stride,
                );
            }
        }

        let _ = buf.present();
    }
}

/// Everything the accessibility tree is built from, as last published.
#[derive(Default, PartialEq)]
struct Published {
    /// The menu's level, entries and cursor.
    menu: Option<(String, Vec<String>, usize)>,
    /// The input configuration's rows and cursor.
    dialog: Option<(Vec<String>, usize)>,
    /// The live-region announcement, when narration goes through the reader.
    said: Option<String>,
}

impl ApplicationHandler<AccessEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // Size for the map up front if it is already on (from --map), so the
        // window does not have to jump on the first frame.
        self.map_shown = self.session.map_shown();
        let attrs = Window::default_attributes()
            .with_title("Beacon")
            .with_inner_size(window_size(self.map_only, self.map_shown));

        // Created hidden where the accessibility adapter demands it, and only there.
        //
        // The adapter must exist before the window is first shown, and it panics rather than
        // degrading if it does not — which is what happened on Windows. Linux could not have
        // caught that: the guard is `window.is_visible() == Some(true)`, and winit answers None
        // on X11 and Wayland, so it cannot fire there however wrong the order is.
        //
        // Which is also why this is conditional. On X11 a window created hidden STAYS hidden:
        // winit's set_visible does not map it, from `resumed` or from a later turn of the loop,
        // so hiding it there traded a panic on one platform for an invisible window on another.
        // Linux has no constraint to satisfy, so it keeps creating the window visible.
        #[cfg(any(windows, target_os = "macos"))]
        let attrs = attrs.with_visible(false);

        // Windows: no OLE drag and drop, because Beacon's voice comes first.
        //
        // winit calls OleInitialize for drag and drop, which demands the thread be a
        // single-threaded apartment, and PANICS if it is not. Tolk — loaded before the window,
        // because speech is set up before anything is drawn — takes the main thread as
        // multi-threaded on its way to SAPI, so the two cannot both have their way. Beacon
        // handles no dropped files, so there is nothing here to lose: a ROM dragged onto
        // `beacon.exe` arrives as an argument from the shell, which is untouched by this.
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_drag_and_drop(false)
        };

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("could not create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // Before the surface, so the adapter sees the window at its initial size, and before
        // the window is shown, which the adapter requires.
        self.access = Some(Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.access_proxy.clone(),
        ));

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
        // The adapter needs focus and size changes to keep the platform's view of the
        // window in step. It reads only; keys are still handled below as before.
        if let (Some(adapter), Some(window)) = (self.access.as_mut(), self.window.as_ref()) {
            adapter.process_event(window, &event);
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if self.session.in_config() {
                        if pressed {
                            self.config_key(code);
                        }
                    } else if self.session.in_menu() {
                        if pressed {
                            self.menu_key(code);
                            // Exit is a menu entry, so quitting can come from here.
                            self.honour_quit(event_loop);
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

            // Said out loud because it decides whether anything works at all: keys reach the
            // window that has the focus, and a screen reader reads the window that has the
            // focus. When neither happened on Windows, this is the line that would have
            // answered why in one word.
            WindowEvent::Focused(has) => {
                if self.focused != Some(has) {
                    self.focused = Some(has);
                    eprintln!("window: {}", if has { "focused" } else { "not focused" });
                }
            }

            // The surface follows the window's size inside `present`, so a resize only needs
            // to ask for one. Explicit because redraws are no longer unconditional: with a
            // paused game nothing else would ask, and the picture would stay the old size.
            WindowEvent::Resized(_) => self.redraw_wanted = true,

            _ => {}
        }
    }

    /// Requests from assistive technology, which arrive on this thread by way of the proxy.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AccessEvent) {
        match event.window_event {
            // Asked for on activation, and whenever a screen reader attaches later. The
            // whole tree, since the platform has nothing yet to update against.
            AccessWindowEvent::InitialTreeRequested => {
                // Something is reading, so announcing through it is a route that exists.
                self.session.set_at_listening(true);
                let update = self.tree();
                if let Some(adapter) = self.access.as_mut() {
                    adapter.update_if_active(|| update);
                }
            }
            // A screen reader driving the menu itself — clicking an item, or moving focus
            // with its own review keys. Honoured, so the menu works the way its user's
            // reader works and not only the way Beacon's keys do.
            AccessWindowEvent::ActionRequested(request) => {
                self.access_action(request);
                // File, Exit is reachable from here too, so a quit asked for by a screen
                // reader has to be honoured like one asked for by a key.
                self.honour_quit(event_loop);
            }
            // Nothing is listening any more. The next request builds the tree again, so
            // there is nothing to tear down; forgetting what was published means a reader
            // that comes back is sent the tree rather than skipped as unchanged.
            AccessWindowEvent::AccessibilityDeactivated => {
                self.session.set_at_listening(false);
                self.published = Published::default();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Run any control-socket requests first, so an attached agent stays
        // responsive even while the game is paused (this wake still fires).
        if let Some(rx) = self.control_rx.as_ref() {
            mcp::drain(&mut self.session, rx);
            self.honour_quit(event_loop);
        }

        // Poll the pad once per wake, before running frames. This must happen
        // regardless of pause or mode, so a controller-only player can act,
        // step, and reach the configuration without a keyboard.
        for name in self.input.poll_gamepad() {
            if self.session.in_config() {
                self.config_pad(name);
            } else if self.session.in_menu() {
                self.menu_pad(name);
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
        if self.session.in_config() || self.session.in_menu() {
            self.input.clear_keyboard();
        }
        // Anything chosen from the platform's own menu, BEFORE the bar is rebuilt. It does
        // its own navigating and reports only what was picked, by id — and a rebuilt bar has
        // new ids, so a choice resolved against the wrong one would be silently dropped.
        #[cfg(any(windows, target_os = "macos"))]
        if let Some(native) = self.native.as_ref() {
            while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
                if let Some(act) = native.act_for(&event.id) {
                    self.session.perform(act);
                }
            }
            self.honour_quit(event_loop);
        }

        // The menu bar, built on the first wake and rebuilt whenever what it lists changes.
        #[cfg(any(windows, target_os = "macos"))]
        self.sync_native_menu();

        // Show the window once, now the loop is turning — where it was created hidden so the
        // accessibility adapter could be built first. A no-op on platforms that created it
        // visible, which is why it needs no cfg of its own.
        if !self.shown {
            if let Some(window) = self.window.as_ref() {
                window.set_visible(true);
                // And ASK FOR THE KEYBOARD. Beacon is a console-subsystem executable on
                // Windows, so a console window opens beside the game and keeps the focus: keys
                // went to the console, and a screen reader inspecting the focused window
                // inspected the console too — which is why nothing responded to a key and the
                // accessibility tree was never once asked for. Showing a window does not focus
                // it; this says so out loud.
                window.focus_window();
                self.shown = true;
            }
        }

        // After input and before frames: the menu's state is settled by now, whether it
        // moved by key, by pad, or by an agent over the control socket.
        self.publish_access();

        self.session.set_held_buttons(self.input.buttons());
        let drew = self.session.run_frames();

        // Sleep until there is something to do, rather than asking again immediately.
        //
        // The frame loop is paced by the audio queue, which limits how fast the emulator
        // runs and not how fast this loop spins: with ControlFlow::Poll and a full queue,
        // `run_frames` returned at once and everything here ran again, hundreds of thousands
        // of times a second, for a core at 100% and no benefit at all. The wait is short
        // enough that input and the accessibility tree are still handled promptly, and winit
        // wakes early anyway when a real event arrives.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + self.session.idle_for(),
        ));

        if let Some(window) = self.window.as_ref() {
            // Grow or shrink the window when the map is toggled, so the game
            // panel keeps its size and the map appears beside it.
            let shown = self.session.map_shown();
            if shown != self.map_shown {
                self.map_shown = shown;
                // In map-only mode the window stays square regardless.
                let _ = window.request_inner_size(window_size(self.map_only, shown));
                self.redraw_wanted = true;
            }
            // Only when the picture can have changed. A frame produces a new one; a resize
            // or a map toggle needs the old one blitted again. Redrawing on every turn of
            // the loop meant blitting and presenting far faster than the screen can show it.
            if drew || self.redraw_wanted {
                self.redraw_wanted = false;
                window.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_sizes_match_the_presentation() {
        let game = window_size(false, false);
        assert_eq!((game.width, game.height), (GAME_W, GAME_H));

        // Map beside: game plus a square panel the height of the game.
        let beside = window_size(false, true);
        assert_eq!((beside.width, beside.height), (GAME_W + GAME_H, GAME_H));

        // Map only: a square, whatever the map-shown flag says.
        let only = window_size(true, false);
        assert_eq!((only.width, only.height), (GAME_H, GAME_H));
        assert_eq!(window_size(true, true), only);
    }

    #[test]
    fn blit_copies_at_one_to_one() {
        let src = vec![1, 2, 3, 4]; // 2x2
        let mut dst = vec![0u32; 16]; // 4x4
        blit(&mut dst, 4, 1, 1, 2, 2, &src, 2, 2, 2);
        assert_eq!(dst[4 + 1], 1); // row 1, col 1
        assert_eq!(dst[4 + 2], 2);
        assert_eq!(dst[2 * 4 + 1], 3); // row 2
        assert_eq!(dst[2 * 4 + 2], 4);
        assert_eq!(dst[0], 0); // outside the placed rect, untouched
    }

    #[test]
    fn blit_scales_up_nearest_neighbour() {
        let src = vec![7]; // 1x1
        let mut dst = vec![0u32; 9]; // 3x3
        blit(&mut dst, 3, 0, 0, 3, 3, &src, 1, 1, 1);
        assert!(dst.iter().all(|&p| p == 7));
    }

    #[test]
    fn blit_clips_rather_than_panics() {
        let src = vec![9; 4];
        let mut dst = vec![0u32; 4]; // 2x2
                                     // Destination rect runs off the buffer; must not panic.
        blit(&mut dst, 2, 1, 1, 4, 4, &src, 2, 2, 2);
        assert_eq!(dst[3], 9); // the one in-bounds pixel (row 1, col 1) got written
    }

    #[test]
    fn side_by_side_layout_splits_the_window() {
        // Replicates present()'s region math: a game frame on the left, a square
        // map panel on the right, into one buffer.
        let (win_w, win_h) = (20usize, 8usize);
        let map_side = win_h.min(win_w / 2); // 8
        let game_w = win_w - map_side; // 12
        let mut buf = vec![0u32; win_w * win_h];
        let game = vec![0x111111u32; 4]; // 2x2
        let map = vec![0x222222u32; 4]; // 2x2
        blit(&mut buf, win_w, 0, 0, game_w, win_h, &game, 2, 2, 2);
        let map_y = (win_h - map_side) / 2;
        blit(
            &mut buf, win_w, game_w, map_y, map_side, map_side, &map, 2, 2, 2,
        );
        // Left region is the game, right region is the map, no overlap.
        assert_eq!(buf[0], 0x111111);
        assert_eq!(buf[game_w - 1], 0x111111);
        assert_eq!(buf[game_w], 0x222222);
        assert_eq!(buf[win_w - 1], 0x222222);
    }
}
