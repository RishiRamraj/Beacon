//! The window, the frame loop, and the wiring between them.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use beacon_config::Settings;
use beacon_emu::Emulator;
use beacon_output::sink::{Fanout, SpeechSink};
use beacon_output::{Arbiter, Intent, Priority, Utterance};
use beacon_plugin::Plugin;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use crate::audio::Audio;
use crate::input::{Command, Input};

/// NTSC frame rate. Used to derive session time from the frame counter, which
/// keeps arbitration deterministic: the same inputs produce the same output
/// regardless of how fast the host actually ran.
const NTSC_FPS: f64 = 60.098;

/// Default window scale over the SNES's 256x224.
const DEFAULT_SCALE: u32 = 3;

pub struct App {
    emu: Emulator,
    audio: Audio,
    input: Input,
    arbiter: Arbiter,
    speech: Fanout,
    plugin: Box<dyn Plugin>,
    settings: Settings,

    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    context: Option<softbuffer::Context<Rc<Window>>>,

    audio_scratch: Vec<f32>,
    last_spoken: Option<String>,
    frames: u64,
    /// Latched so the warning is said once, not every time audio starves.
    warned_slow: bool,
}

impl App {
    pub fn new(
        emu: Emulator,
        audio: Audio,
        arbiter: Arbiter,
        speech: Fanout,
        plugin: Box<dyn Plugin>,
        settings: Settings,
    ) -> Self {
        App {
            emu,
            audio,
            input: Input::new(),
            arbiter,
            speech,
            plugin,
            settings,
            window: None,
            surface: None,
            context: None,
            audio_scratch: Vec::with_capacity(4096),
            last_spoken: None,
            frames: 0,
            warned_slow: false,
        }
    }

    /// Session time derived from the frame count rather than the wall clock, so
    /// that a replay of the same inputs arbitrates identically.
    fn session_time(&self) -> Duration {
        Duration::from_secs_f64(self.frames as f64 / NTSC_FPS)
    }

    /// Runs frames until the audio queue is full.
    ///
    /// Audio paces emulation: a starved buffer is an audible click, and for a
    /// player navigating by sound a click is indistinguishable from a cue.
    fn run_frames(&mut self) {
        // Bounded so that a stall cannot spin here forever and freeze the UI.
        const MAX_CATCH_UP: u32 = 8;

        for _ in 0..MAX_CATCH_UP {
            if self.audio.is_ahead() {
                break;
            }

            self.input.poll_gamepad();
            self.emu.set_buttons(0, self.input.buttons());

            self.emu.run_frame();
            self.frames += 1;

            self.audio_scratch.clear();
            self.emu.drain_audio(&mut self.audio_scratch);
            if !self.audio_scratch.is_empty() {
                let scratch = std::mem::take(&mut self.audio_scratch);
                self.audio.submit(&scratch);
                self.audio_scratch = scratch;
            }

            // Instrumentation runs here: between frames, against real memory.
            let frame = self.frames;
            let intents = match self.emu.main_ram() {
                Ok(ram) => self.plugin.on_frame(ram, frame),
                Err(_) => Vec::new(),
            };
            self.dispatch(intents);
        }

        // Sustained underruns mean this machine cannot hold 60 fps. Say so:
        // the alternative is a player hearing clicks and mistaking them for
        // navigation cues.
        if !self.warned_slow && self.frames > 300 && self.audio.underruns() > 50 {
            self.warned_slow = true;
            self.say_now("Audio is struggling. This machine may be too slow for full speed.");
        }
    }

    /// Puts intents through the arbiter and speaks whatever survives.
    fn dispatch(&mut self, intents: Vec<Intent>) {
        if intents.is_empty() {
            return;
        }
        let now = self.session_time();
        for utterance in self.arbiter.resolve(intents, now) {
            self.say(utterance);
        }
    }

    fn say(&mut self, utterance: Utterance) {
        self.last_spoken = Some(utterance.text.clone());
        if let Err(e) = self.speech.speak(&utterance) {
            eprintln!("speech: {e}");
        }
    }

    /// Speaks something immediately, bypassing arbitration.
    ///
    /// Used for direct answers to commands: the player asked, so rate limiting
    /// and verbosity are not the tool's business.
    fn say_now(&mut self, text: impl Into<String>) {
        self.say(Utterance {
            text: text.into(),
            priority: Priority::Navigation,
            interrupt: true,
        });
    }

    /// Runs a plugin command against the current frame's memory.
    ///
    /// The plugin's answer is a direct response to a keypress, so it is spoken
    /// immediately rather than arbitrated. `fallback` covers a plugin that does
    /// not implement the command, or has nothing to say: silence would read as a
    /// broken key.
    fn run_command(&mut self, name: &str, fallback: &str) {
        let intents = match self.emu.main_ram() {
            Ok(ram) => self.plugin.command(name, ram),
            Err(_) => Vec::new(),
        };
        if intents.is_empty() {
            self.say_now(fallback);
        } else {
            for intent in intents {
                self.say_now(intent.text);
            }
        }
    }

    fn handle_command(&mut self, cmd: Command, event_loop: &ActiveEventLoop) {
        match cmd {
            Command::Quit => {
                self.say_now("Goodbye.");
                event_loop.exit();
            }
            Command::Where => self.run_command("where", "Nothing to report."),
            Command::Status => self.run_command("status", "Nothing to report."),
            Command::Scan => self.run_command("scan", "Scan is not available for this game."),
            Command::RepeatLast => match self.last_spoken.clone() {
                Some(text) => self.say_now(text),
                None => self.say_now("Nothing to repeat."),
            },
            Command::CycleVerbosity => {
                let next = (self.settings.arbiter.verbosity + 1) % 4;
                self.settings.arbiter.verbosity = next;
                self.arbiter.set_verbosity(next);

                let name = match next {
                    0 => "critical only",
                    1 => "navigation",
                    2 => "interaction",
                    _ => "everything",
                };
                self.say_now(format!("Verbosity {next}, {name}."));

                // Persist, so a setting found once stays found.
                if let Some(path) = Settings::default_path() {
                    if let Err(e) = self.settings.save(&path) {
                        eprintln!("could not save settings: {e}");
                    }
                }
            }
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

        let info = self.emu.frame_info();
        let (src_w, src_h) = (info.width as usize, info.height as usize);
        if src_w == 0 || src_h == 0 {
            return;
        }

        // `pitch` is a byte stride; the framebuffer is 32-bit pixels.
        let stride = (info.pitch as usize / 4).max(src_w);
        let src = self.emu.framebuffer();

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
        self.say_now("Beacon ready.");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.input
                        .on_key(code, event.state == ElementState::Pressed);
                }
                for cmd in self.input.take_commands() {
                    self.handle_command(cmd, event_loop);
                }
            }

            WindowEvent::RedrawRequested => self.present(),

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.run_frames();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
