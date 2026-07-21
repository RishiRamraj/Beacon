//! The emulator session: everything that runs a game and speaks about it,
//! independent of how it is presented or driven.
//!
//! This is the core the winit window wraps ([`crate::app`]) and the one an agent
//! drives over MCP. It owns the emulator, audio, the plugin, arbitration, speech,
//! settings, and savestates, and exposes the verbs that act on them — step a
//! frame, run an action, drive the configuration, read memory. It knows nothing
//! about windows, key codes, or event loops: the shell above translates devices
//! into these calls.
//!
//! Held buttons come in through [`set_held_buttons`](Session::set_held_buttons)
//! rather than being read from a device here, so the same session runs whether a
//! keyboard, a gamepad, or an agent is supplying them.

use std::collections::VecDeque;
use std::time::Duration;

use beacon_config::Settings;
use beacon_emu::{Emulator, FrameInfo};
use beacon_output::sink::{Fanout, SpeechSink};
use beacon_output::{Arbiter, Intent, Priority, Utterance};
use beacon_plugin::{LuaPlugin, Plugin, PluginSpec};

use crate::action::{self, Action, Bindable};
use crate::audio::Audio;
use crate::config_modal::{Bound, ConfigModal};
use crate::state::{SlotStore, SLOTS};

/// How many recent spoken lines to retain for an agent to read back. Bounded so
/// a long GUI session does not accumulate them without limit.
const SPEECH_LOG_CAP: usize = 512;

/// NTSC frame rate. Session time comes from the frame counter, not the clock, so
/// a replay of the same inputs arbitrates identically.
const NTSC_FPS: f64 = 60.098;

pub struct Session {
    emu: Emulator,
    audio: Audio,
    arbiter: Arbiter,
    speech: Fanout,
    plugin: Box<dyn Plugin>,
    /// The spec the plugin was built from, kept so it can be reloaded. `None` for
    /// a session with no matching plugin.
    reload_spec: Option<PluginSpec>,
    settings: Settings,

    slots: SlotStore,
    active_slot: u8,
    paused: bool,
    /// Once the player has paused or stepped, wall-clock timing no longer
    /// reflects the machine's real speed, so the "too slow" heuristic is retired.
    timing_disturbed: bool,
    /// `Some` while the input configuration is open; the game is suspended then.
    config: Option<ConfigModal>,
    /// Whether the plugin's map view is showing.
    show_map: bool,
    /// The plugin's last rendered map, and its dimensions.
    map_buffer: Vec<u32>,
    map_dims: (u32, u32),

    /// Buttons currently held, supplied by whatever is driving the session.
    held_buttons: u16,
    /// Set by the quit action; the driver checks it and shuts down.
    quit: bool,

    audio_scratch: Vec<f32>,
    last_spoken: Option<String>,
    /// Recent spoken lines, for an agent to read what a player would have heard.
    speech_log: VecDeque<String>,
    frames: u64,
    warned_slow: bool,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        emu: Emulator,
        audio: Audio,
        arbiter: Arbiter,
        speech: Fanout,
        plugin: Box<dyn Plugin>,
        reload_spec: Option<PluginSpec>,
        settings: Settings,
        rom_id: &str,
    ) -> Self {
        Session {
            emu,
            audio,
            arbiter,
            speech,
            plugin,
            reload_spec,
            settings,
            slots: SlotStore::new(rom_id),
            active_slot: 0,
            paused: false,
            timing_disturbed: false,
            config: None,
            show_map: false,
            map_buffer: Vec::new(),
            map_dims: (0, 0),
            held_buttons: 0,
            quit: false,
            audio_scratch: Vec::with_capacity(4096),
            last_spoken: None,
            speech_log: VecDeque::new(),
            frames: 0,
            warned_slow: false,
        }
    }

    // --- Driving the frame loop ------------------------------------------

    /// Sets the buttons held this tick. The frame loop reads these; the driver
    /// (a device layer or an agent) writes them.
    pub fn set_held_buttons(&mut self, mask: u16) {
        self.held_buttons = mask;
    }

    /// Session time derived from the frame count rather than the wall clock.
    fn session_time(&self) -> Duration {
        Duration::from_secs_f64(self.frames as f64 / NTSC_FPS)
    }

    /// Advances the emulator by exactly one frame and runs the plugin over it.
    pub fn step_one_frame(&mut self) {
        self.emu.set_buttons(0, self.held_buttons);

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

        // Keep the map live while it is on screen; it costs nothing when hidden.
        if self.show_map {
            self.render_map();
        }
    }

    /// Runs frames until the audio queue is full.
    ///
    /// Audio paces emulation: a starved buffer is an audible click, and for a
    /// player navigating by sound a click is indistinguishable from a cue. While
    /// paused, or with the configuration open, nothing runs here.
    pub fn run_frames(&mut self) {
        if self.paused || self.config.is_some() {
            return;
        }

        // Bounded so that a stall cannot spin here forever.
        const MAX_CATCH_UP: u32 = 8;
        for _ in 0..MAX_CATCH_UP {
            if self.audio.is_ahead() {
                break;
            }
            self.step_one_frame();
        }

        if !self.timing_disturbed
            && !self.warned_slow
            && self.frames > 300
            && self.audio.underruns() > 50
        {
            self.warned_slow = true;
            self.say_now("Audio is struggling. This machine may be too slow for full speed.");
        }
    }

    // --- Speech ----------------------------------------------------------

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
        if self.speech_log.len() >= SPEECH_LOG_CAP {
            self.speech_log.pop_front();
        }
        self.speech_log.push_back(utterance.text.clone());
        if let Err(e) = self.speech.speak(&utterance) {
            eprintln!("speech: {e}");
        }
    }

    /// Speaks something immediately, bypassing arbitration.
    ///
    /// Used for direct answers and for Beacon's own responses: the player asked,
    /// so rate limiting and verbosity are not the tool's business.
    pub fn say_now(&mut self, text: impl Into<String>) {
        self.say(Utterance {
            text: text.into(),
            priority: Priority::Navigation,
            interrupt: true,
        });
    }

    fn persist_settings(&self) {
        if let Some(path) = Settings::default_path() {
            if let Err(e) = self.settings.save(&path) {
                eprintln!("could not save settings: {e}");
            }
        }
    }

    // --- Actions ---------------------------------------------------------

    /// Resolves an input name to an action via the keymap and runs it.
    ///
    /// Shared by keyboard and gamepad: both name their inputs the same way
    /// ("KeyC", "Pad:LeftThumb"), so binding is uniform across devices.
    pub fn resolve_action(&mut self, name: &str) {
        let Some(action_id) = self.settings.keymap.action_for(name).map(str::to_string) else {
            return;
        };
        if let Some(action) = Action::from_id(&action_id) {
            self.handle_action(action);
        }
    }

    /// Runs an action. Quit sets a flag the driver observes rather than exiting
    /// directly, so the session stays independent of any event loop.
    pub fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                self.say_now("Goodbye.");
                self.quit = true;
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
            Action::ToggleMap => self.toggle_map(),
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

    /// Runs a plugin command against the current frame's memory and speaks the
    /// answer immediately. Empty output falls back to an acknowledgement, so a
    /// bound key is never silent.
    pub fn run_command(&mut self, name: &str) {
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

    pub fn save_state(&mut self) {
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

    pub fn load_state(&mut self) {
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
    pub fn frame_advance(&mut self) {
        self.paused = true;
        self.timing_disturbed = true;
        self.step_one_frame();
        self.say_now(format!("Frame {}.", self.frames));
    }

    /// Shows or hides the plugin's map view.
    fn toggle_map(&mut self) {
        if !self.plugin.has_map() {
            self.say_now("This game has no map.");
            return;
        }
        self.show_map = !self.show_map;
        if self.show_map {
            // Render at once, so a map opened while paused is not blank.
            self.render_map();
            self.say_now("Map shown.");
        } else {
            self.say_now("Map hidden.");
        }
    }

    /// Renders the plugin's map for the current frame into the map buffer,
    /// returning its dimensions. `None` if the plugin draws no map.
    pub fn render_map(&mut self) -> Option<(u32, u32)> {
        let frame = self.frames;
        // Disjoint field borrows: `ram` reads `emu`, `draw` writes `plugin` and
        // the buffer.
        let dims = match self.emu.main_ram() {
            Ok(ram) => self.plugin.draw(ram, frame, &mut self.map_buffer),
            Err(_) => None,
        };
        if let Some(d) = dims {
            self.map_dims = d;
        }
        dims
    }

    /// The current map as (width, height, pixels), if it is showing and drawn.
    pub fn map_view(&self) -> Option<(u32, u32, &[u32])> {
        if self.show_map && !self.map_buffer.is_empty() {
            Some((self.map_dims.0, self.map_dims.1, &self.map_buffer))
        } else {
            None
        }
    }

    /// The last rendered map pixels, for encoding by the MCP server.
    pub fn map_pixels(&self) -> &[u32] {
        &self.map_buffer
    }

    // --- Input configuration ---------------------------------------------

    pub fn open_input_config(&mut self) {
        // Freeze the game so nothing moves while choosing bindings.
        self.paused = true;
        self.timing_disturbed = true;
        self.held_buttons = 0;

        let modal = ConfigModal::new(action::bindable_actions(self.plugin.as_ref()));
        let opening = modal.announce(&self.settings.keymap);
        self.config = Some(modal);
        self.say_now(
            "Input configuration. Up and down to choose an action, then press a key to bind it. \
             Delete to clear a binding, escape to finish.",
        );
        self.say_now(opening);
    }

    /// Whether the configuration modal is open.
    pub fn in_config(&self) -> bool {
        self.config.is_some()
    }

    /// Moves the configuration selection, announcing the new item.
    pub fn config_navigate(&mut self, delta: i32) {
        let Some(modal) = self.config.as_mut() else {
            return;
        };
        let said = modal.navigate(delta, &self.settings.keymap);
        self.say_now(said);
    }

    /// Binds an input name to the selected action, or reports why it cannot be.
    pub fn config_bind(&mut self, name: &str) {
        let Some(modal) = self.config.as_ref() else {
            return;
        };
        let said = match modal.bind(name, &mut self.settings.keymap) {
            Bound::Ok(msg) => {
                self.persist_settings();
                msg
            }
            Bound::Refused(msg) => msg,
        };
        self.say_now(said);
    }

    /// Clears the selected action's bindings.
    pub fn config_clear(&mut self) {
        let Some(modal) = self.config.as_ref() else {
            return;
        };
        let said = modal.clear(&mut self.settings.keymap);
        self.persist_settings();
        self.say_now(said);
    }

    /// Closes the configuration and resumes play.
    pub fn config_close(&mut self) {
        self.config = None;
        self.held_buttons = 0;
        self.paused = false;
        self.say_now("Configuration saved.");
    }

    // --- Queries used by the winit shell ---------------------------------

    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// The current video frame's geometry.
    pub fn frame_info(&self) -> FrameInfo {
        self.emu.frame_info()
    }

    /// The current video frame's pixels.
    pub fn framebuffer(&self) -> &[u32] {
        self.emu.framebuffer()
    }

    // --- The agent-facing control surface (used by the MCP server) -------
    //
    // These are the same verbs the device shell drives, plus the reads an agent
    // needs to see what a player would. Keeping them here, on the one core, means
    // a keyboard, a controller, and an agent all act through identical logic.

    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn active_slot_index(&self) -> u8 {
        self.active_slot
    }

    pub fn plugin_name(&self) -> &str {
        self.plugin.name()
    }

    /// Drains the recent spoken lines, so an agent reads each only once.
    pub fn take_speech(&mut self) -> Vec<String> {
        self.speech_log.drain(..).collect()
    }

    /// Reads work RAM by SNES address, sharing the plugin's addressing. `None`
    /// if any byte of the range is outside mapped WRAM.
    pub fn read_wram(&self, addr: u32, len: usize) -> Option<Vec<u8>> {
        let ram = self.emu.main_ram().ok()?;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let offset = beacon_plugin::wram_offset(addr.wrapping_add(i as u32))?;
            out.push(*ram.get(offset)?);
        }
        Some(out)
    }

    /// Pauses and advances exactly `n` frames, running the plugin over each. Used
    /// by an agent stepping through a situation; unlike frame advance it does not
    /// announce each frame.
    pub fn step_frames(&mut self, n: u32) {
        self.paused = true;
        self.timing_disturbed = true;
        for _ in 0..n {
            self.step_one_frame();
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.timing_disturbed = true;
    }

    /// Sets the active save slot, wrapping into range.
    pub fn set_active_slot(&mut self, slot: u8) {
        self.active_slot = slot % SLOTS;
    }

    /// The current bindings, as (input name, action id) pairs.
    pub fn bindings(&self) -> Vec<(String, String)> {
        self.settings
            .keymap
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The bindable actions, with labels and their current keys.
    pub fn bindable_actions(&self) -> Vec<Bindable> {
        action::bindable_actions(self.plugin.as_ref())
    }

    /// The keys currently bound to an action id.
    pub fn keys_for_action(&self, action_id: &str) -> Vec<String> {
        self.settings.keymap.keys_for(action_id)
    }

    /// Binds an input to an action, refusing a game control, and persists.
    pub fn bind(&mut self, name: &str, action_id: &str) -> Result<(), String> {
        if crate::input::is_game_input_name(name) {
            return Err(format!("{name} controls the game and can't be reassigned"));
        }
        self.settings.keymap.bind(name, action_id);
        self.persist_settings();
        Ok(())
    }

    /// Removes any binding for an input, and persists.
    pub fn unbind(&mut self, name: &str) {
        self.settings.keymap.unbind(name);
        self.persist_settings();
    }

    /// Reads a setting by name.
    pub fn get_setting(&self, key: &str) -> Result<String, String> {
        self.settings.get(key).map_err(|e| e.to_string())
    }

    /// Sets a setting by name, keeping the arbiter in step and persisting.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.settings.set(key, value).map_err(|e| e.to_string())?;
        // Verbosity lives in two places; keep the live arbiter aligned with the
        // stored setting.
        self.arbiter.set_verbosity(self.settings.arbiter.verbosity);
        self.persist_settings();
        Ok(())
    }

    /// Rebuilds the plugin from its source, picking up edits on disk.
    ///
    /// The tight edit-run loop for a plugin author: change the Lua, reload, see
    /// the effect, without restarting the emulator or losing the game's position.
    /// The plugin's own Lua state (its `prev`, its latches) resets, which is
    /// expected — it re-derives from the next frame.
    pub fn reload_plugin(&mut self) -> Result<String, String> {
        let Some(spec) = &self.reload_spec else {
            return Err("no plugin is loaded to reload".to_string());
        };
        let fresh = spec.reloaded().map_err(|e| e.to_string())?;
        let plugin = LuaPlugin::load(&fresh).map_err(|e| e.to_string())?;

        let name = plugin.name().to_string();
        let from_disk = fresh.is_reloadable_from_disk();
        self.plugin = Box::new(plugin);
        self.reload_spec = Some(fresh);
        // The old map belongs to the old plugin; drop it and let it redraw.
        self.map_buffer.clear();
        if self.show_map {
            self.render_map();
        }
        Ok(if from_disk {
            format!("reloaded {name} from disk")
        } else {
            format!("re-instantiated built-in {name} (no disk source to reread)")
        })
    }

    /// Evaluates a Lua snippet in the plugin's environment against the current
    /// frame, returning its result. For an agent probing memory and plugin state.
    pub fn eval_lua(&mut self, code: &str) -> Result<String, String> {
        let ram = match self.emu.main_ram() {
            Ok(r) => r,
            Err(e) => return Err(e.to_string()),
        };
        self.plugin.eval(code, ram)
    }
}

#[cfg(test)]
mod tests {
    // The frame-loop and speech paths need a real emulator and audio device, so
    // they are exercised through the running app and the MCP integration rather
    // than here. The parts that can be tested without hardware — the keymap, the
    // action id mapping, and the configuration modal — are covered in
    // `beacon_config`, `action`, and `config_modal` respectively.
    //
    // Naming an input's game-ness, the check the modal relies on, is asserted
    // here since it is the seam between this module and `input`.
    use crate::input::is_game_input_name;

    #[test]
    fn game_inputs_are_recognised_by_name_across_devices() {
        assert!(is_game_input_name("KeyX")); // SNES A
        assert!(is_game_input_name("ArrowUp"));
        assert!(is_game_input_name("Pad:South"));
        assert!(!is_game_input_name("KeyD"));
        assert!(!is_game_input_name("Pad:C"));
        assert!(!is_game_input_name("F5"));
    }
}
