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
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::action::{self, Action, Bindable};
use crate::audio::Audio;
use crate::input::{self, Input};
use crate::state::{SlotStore, SLOTS};

/// NTSC frame rate. Used to derive session time from the frame counter, which
/// keeps arbitration deterministic: the same inputs produce the same output
/// regardless of how fast the host actually ran.
const NTSC_FPS: f64 = 60.098;

/// Default window scale over the SNES's 256x224.
const DEFAULT_SCALE: u32 = 3;

/// What the app is currently doing with input.
///
/// Configuration is a distinct mode, not a flag over the play loop: while it is
/// open the game is suspended and every key is captured for binding, so a keypress
/// meant to assign a control can never leak through to the game.
enum Mode {
    Playing,
    InputConfig {
        actions: Vec<Bindable>,
        index: usize,
    },
}

pub struct App {
    emu: Emulator,
    audio: Audio,
    input: Input,
    arbiter: Arbiter,
    speech: Fanout,
    plugin: Box<dyn Plugin>,
    settings: Settings,

    slots: SlotStore,
    active_slot: u8,
    /// Emulation halted. Frame advance steps through it one frame at a time.
    paused: bool,
    /// Once the player has paused or stepped, wall-clock timing no longer
    /// reflects the machine's real speed, so the "too slow" heuristic is retired
    /// for the session rather than firing spuriously.
    timing_disturbed: bool,
    mode: Mode,

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
        rom_id: &str,
    ) -> Self {
        App {
            emu,
            audio,
            input: Input::new(),
            arbiter,
            speech,
            plugin,
            settings,
            slots: SlotStore::new(rom_id),
            active_slot: 0,
            paused: false,
            timing_disturbed: false,
            mode: Mode::Playing,
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

    /// Advances the emulator by exactly one frame and runs the plugin over it.
    ///
    /// The gamepad is polled once per event-loop wake in [`about_to_wait`], not
    /// here, so held state is already current.
    ///
    /// [`about_to_wait`]: ApplicationHandler::about_to_wait
    fn step_one_frame(&mut self) {
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

    /// Runs frames until the audio queue is full.
    ///
    /// Audio paces emulation: a starved buffer is an audible click, and for a
    /// player navigating by sound a click is indistinguishable from a cue. While
    /// paused, or with the configuration open, nothing runs here — frame advance
    /// steps the emulator directly instead.
    fn run_frames(&mut self) {
        if !matches!(self.mode, Mode::Playing) || self.paused {
            return;
        }

        // Bounded so that a stall cannot spin here forever and freeze the UI.
        const MAX_CATCH_UP: u32 = 8;
        for _ in 0..MAX_CATCH_UP {
            if self.audio.is_ahead() {
                break;
            }
            self.step_one_frame();
        }

        // Sustained underruns mean this machine cannot hold 60 fps. Say so once:
        // the alternative is a player hearing clicks and mistaking them for
        // navigation cues. Skipped once timing has been disturbed by pausing.
        if !self.timing_disturbed
            && !self.warned_slow
            && self.frames > 300
            && self.audio.underruns() > 50
        {
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
    /// Used for direct answers to commands and for Beacon's own responses: the
    /// player asked, so rate limiting and verbosity are not the tool's business.
    fn say_now(&mut self, text: impl Into<String>) {
        self.say(Utterance {
            text: text.into(),
            priority: Priority::Navigation,
            interrupt: true,
        });
    }

    /// Writes settings to disk, so a change made while playing outlives the run.
    fn persist_settings(&self) {
        if let Some(path) = Settings::default_path() {
            if let Err(e) = self.settings.save(&path) {
                eprintln!("could not save settings: {e}");
            }
        }
    }

    // --- Actions ----------------------------------------------------------

    /// Resolves an input name to an action via the keymap and runs it.
    ///
    /// Shared by keyboard and gamepad: both name their inputs the same way to the
    /// keymap ("KeyC", "Pad:LeftThumb"), so binding is uniform across devices.
    fn resolve_action(&mut self, name: &str, event_loop: &ActiveEventLoop) {
        let Some(action_id) = self.settings.keymap.action_for(name).map(str::to_string) else {
            return;
        };
        if let Some(action) = Action::from_id(&action_id) {
            self.handle_action(action, event_loop);
        }
    }

    fn handle_action(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        match action {
            Action::Quit => {
                self.say_now("Goodbye.");
                event_loop.exit();
            }
            Action::CycleVerbosity => self.cycle_verbosity(),
            Action::RepeatLast => match self.last_spoken.clone() {
                Some(text) => self.say_now(text),
                None => self.say_now("Nothing to repeat."),
            },
            Action::SaveState => self.save_state(),
            Action::LoadState => self.load_state(),
            Action::NextSlot => self.change_slot(1),
            Action::PrevSlot => self.change_slot(-1),
            Action::Pause => self.toggle_pause(),
            Action::FrameAdvance => self.frame_advance(),
            Action::OpenInputConfig => self.open_input_config(),
            Action::Command(name) => self.run_command(&name),
        }
    }

    fn cycle_verbosity(&mut self) {
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
        self.persist_settings();
    }

    /// Runs a plugin command against the current frame's memory.
    ///
    /// The plugin's answer is a direct response to a keypress, spoken immediately.
    /// Empty output — an unimplemented command, or one with nothing to say — falls
    /// back to a spoken acknowledgement, so a bound key is never silent.
    fn run_command(&mut self, name: &str) {
        let intents = match self.emu.main_ram() {
            Ok(ram) => self.plugin.command(name, ram),
            Err(_) => Vec::new(),
        };
        if intents.is_empty() {
            self.say_now("Nothing to report.");
        } else {
            for intent in intents {
                self.say_now(intent.text);
            }
        }
    }

    fn save_state(&mut self) {
        let slot = self.active_slot;
        match self.emu.save_state() {
            Ok(data) => match self.slots.save(slot, &data) {
                Ok(()) => self.say_now(format!("Saved to slot {slot}.")),
                Err(e) => {
                    eprintln!("save slot {slot}: {e}");
                    self.say_now("Could not save.");
                }
            },
            Err(e) => {
                eprintln!("save state: {e}");
                self.say_now("Could not save.");
            }
        }
    }

    fn load_state(&mut self) {
        let slot = self.active_slot;
        match self.slots.load(slot) {
            Ok(Some(data)) => match self.emu.load_state(&data) {
                Ok(()) => self.say_now(format!("Loaded slot {slot}.")),
                Err(e) => {
                    eprintln!("load slot {slot}: {e}");
                    self.say_now("Could not load.");
                }
            },
            Ok(None) => self.say_now(format!("Slot {slot} is empty.")),
            Err(e) => {
                eprintln!("load slot {slot}: {e}");
                self.say_now("Could not load.");
            }
        }
    }

    fn change_slot(&mut self, delta: i32) {
        let n = SLOTS as i32;
        self.active_slot = (((self.active_slot as i32 + delta) % n + n) % n) as u8;
        let state = if self.slots.occupied(self.active_slot) {
            "occupied"
        } else {
            "empty"
        };
        self.say_now(format!("Slot {}, {state}.", self.active_slot));
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.timing_disturbed = true;
        self.say_now(if self.paused { "Paused." } else { "Resumed." });
    }

    /// Steps one frame, pausing first if running. A debugging aid: it lets a
    /// plugin author watch memory change frame by frame.
    fn frame_advance(&mut self) {
        self.paused = true;
        self.timing_disturbed = true;
        self.step_one_frame();
        self.say_now(format!("Frame {}.", self.frames));
    }

    // --- Input configuration modal ---------------------------------------

    fn open_input_config(&mut self) {
        // Freeze the game and release any held control, so nothing moves while
        // the player is choosing bindings.
        self.paused = true;
        self.timing_disturbed = true;
        self.input.clear_keyboard();

        let actions = action::bindable_actions(self.plugin.as_ref());
        self.mode = Mode::InputConfig { actions, index: 0 };
        self.say_now(
            "Input configuration. Up and down to choose an action, then press a key to bind it. \
             Delete to clear a binding, escape to finish.",
        );
        self.announce_config_item();
    }

    /// The action currently selected in the configuration, as (id, label).
    fn selected(&self) -> Option<(String, String)> {
        match &self.mode {
            Mode::InputConfig { actions, index } => {
                let item = &actions[*index];
                Some((item.id.clone(), item.label.clone()))
            }
            _ => None,
        }
    }

    fn announce_config_item(&mut self) {
        let Some((id, label)) = self.selected() else {
            return;
        };
        let keys = self.settings.keymap.keys_for(&id);
        let bound = if keys.is_empty() {
            "unbound".to_string()
        } else {
            keys.iter()
                .map(|k| input::key_label(k))
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.say_now(format!("{label}. {bound}."));
    }

    fn move_config_selection(&mut self, delta: i32) {
        if let Mode::InputConfig { actions, index } = &mut self.mode {
            let n = actions.len() as i32;
            *index = (((*index as i32 + delta) % n + n) % n) as usize;
        }
        self.announce_config_item();
    }

    /// Handles a keyboard key while the configuration is open.
    ///
    /// Arrow keys and escape/delete drive the modal; any other key binds. These
    /// few are therefore reserved and cannot themselves be bound here — the
    /// settings file remains the escape hatch for that rare case.
    fn config_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Escape => self.close_input_config(),
            KeyCode::ArrowDown => self.move_config_selection(1),
            KeyCode::ArrowUp => self.move_config_selection(-1),
            KeyCode::Delete | KeyCode::Backspace => self.clear_selected_binding(),
            other if input::is_game_button(other) => {
                self.say_now("That key controls the game and can't be reassigned.");
            }
            other => match input::key_name(other) {
                Some(name) => self.bind_selected_to(name),
                None => self.say_now("That key can't be bound."),
            },
        }
    }

    /// Handles a gamepad button while the configuration is open.
    ///
    /// The d-pad navigates and Start finishes, so the modal is fully operable
    /// from the controller. Any free pad button binds; a game button is refused.
    fn config_pad(&mut self, name: &str) {
        match name {
            "Pad:DPadDown" => self.move_config_selection(1),
            "Pad:DPadUp" => self.move_config_selection(-1),
            "Pad:Start" => self.close_input_config(),
            _ if input::is_game_pad_name(name) => {
                self.say_now("That button controls the game and can't be reassigned.");
            }
            _ => self.bind_selected_to(name),
        }
    }

    /// Binds a validated (non-game) input name to the selected action.
    fn bind_selected_to(&mut self, name: &str) {
        let Some((id, label)) = self.selected() else {
            return;
        };
        self.settings.keymap.bind(name, &id);
        self.persist_settings();
        self.say_now(format!("{} bound to {label}.", input::key_label(name)));
    }

    fn clear_selected_binding(&mut self) {
        let Some((id, label)) = self.selected() else {
            return;
        };
        for key in self.settings.keymap.keys_for(&id) {
            self.settings.keymap.unbind(&key);
        }
        self.persist_settings();
        self.say_now(format!("{label} unbound."));
    }

    fn close_input_config(&mut self) {
        self.mode = Mode::Playing;
        self.input.clear_keyboard();
        self.paused = false;
        self.say_now("Configuration saved.");
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
                    let pressed = event.state == ElementState::Pressed;
                    match self.mode {
                        Mode::Playing => {
                            self.input.on_key(code, pressed);
                            // Actions fire on press, and never for a game key, so
                            // the two keyspaces cannot contend.
                            if pressed && !input::is_game_button(code) {
                                if let Some(name) = input::key_name(code) {
                                    self.resolve_action(name, event_loop);
                                }
                            }
                        }
                        Mode::InputConfig { .. } => {
                            if pressed {
                                self.config_key(code);
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
            match self.mode {
                Mode::Playing => {
                    // Game buttons are held state, handled by the frame loop;
                    // only the pad's extra buttons resolve to actions.
                    if !input::is_game_pad_name(name) {
                        self.resolve_action(name, event_loop);
                    }
                }
                Mode::InputConfig { .. } => self.config_pad(name),
            }
        }

        self.run_frames();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
