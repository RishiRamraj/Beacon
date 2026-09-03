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
use std::path::{Path, PathBuf};
use std::time::Duration;

use beacon_config::Settings;
use beacon_emu::{Emulator, FrameInfo};
use beacon_output::sink::{Fanout, SpeechSink};
use beacon_output::{Arbiter, Intent, Priority, Utterance};
use beacon_plugin::{LuaPlugin, Plugin, PluginSpec};

use crate::action::{self, Action, Bindable};
use crate::audio::Audio;
use crate::beacons::BeaconMixer;
use crate::config_modal::{Bound, ConfigModal};
use crate::menu::{self, Act, Backed, Chosen, Menu};
use crate::rom;
use crate::state::{SlotStore, SLOTS};

/// How many recent spoken lines to retain for an agent to read back. Bounded so
/// a long GUI session does not accumulate them without limit.
const SPEECH_LOG_CAP: usize = 512;

/// NTSC frame rate. Session time comes from the frame counter, not the clock, so
/// a replay of the same inputs arbitrates identically.
const NTSC_FPS: f64 = 60.098;

pub struct Session {
    /// `None` when no game is loaded.
    ///
    /// Beacon starts this way when given no ROM, so the menu can be used to find one — which
    /// is the only way to open a game on a desktop where nobody types a path. It is also what
    /// makes swapping ROMs work at all: bsnes-jg allows exactly one live instance, so the old
    /// emulator has to be DROPPED before the new one is created, and there is no way to drop a
    /// field that is not an Option.
    emu: Option<Emulator>,
    audio: Audio,
    arbiter: Arbiter,
    speech: Fanout,
    plugin: Box<dyn Plugin>,
    /// The spec the plugin was built from, kept so it can be reloaded. `None` for
    /// a session with no matching plugin.
    reload_spec: Option<PluginSpec>,
    /// The headerless ROM, handed to the plugin on load and reload.
    rom: std::rc::Rc<Vec<u8>>,
    /// Where the ROM came from, if one is loaded. Kept for two things the menu needs: the
    /// directory to offer other ROMs from, and something to call the one that is running.
    rom_path: Option<PathBuf>,
    settings: Settings,

    /// Savestates for the loaded game. `None` with no game: slots are per game, and a shared
    /// "unknown" store would let one game's states be loaded into another.
    slots: Option<SlotStore>,
    active_slot: u8,
    paused: bool,
    /// Once the player has paused or stepped, wall-clock timing no longer
    /// reflects the machine's real speed, so the "too slow" heuristic is retired.
    timing_disturbed: bool,
    /// `Some` while the input configuration is open; the game is suspended then.
    config: Option<ConfigModal>,
    /// `Some` while the menu is open; the game is suspended then, for the same
    /// reason — nothing should walk into a pit while a list is being read.
    menu: Option<Menu>,
    /// Whether the game was already paused when the menu opened, so closing it puts
    /// things back rather than starting a game the player deliberately stopped.
    paused_before_menu: bool,
    /// Whether the plugin's map view is showing.
    show_map: bool,
    /// The plugin's navigation state last frame, so the map can be brought up on
    /// the off->on edge (guidance starting) without fighting a manual hide.
    nav_was_active: bool,
    /// The plugin's last rendered map, and its dimensions.
    map_buffer: Vec<u32>,
    map_dims: (u32, u32),

    /// Buttons currently held, supplied by whatever is driving the session.
    held_buttons: u16,
    /// Set by the quit action; the driver checks it and shuts down.
    quit: bool,
    /// When true, all output is silenced: the game and beacon audio is submitted
    /// as silence and speech is not spoken (still logged for the MCP speech log).
    /// A single toggle to hush Beacon without closing it.
    muted: bool,

    audio_scratch: Vec<f32>,
    /// Synthesises the plugin's spatial-audio beacons into the audio stream.
    beacon_mixer: BeaconMixer,
    last_spoken: Option<String>,
    /// Recent spoken lines, for an agent to read what a player would have heard.
    speech_log: VecDeque<String>,
    frames: u64,
    warned_slow: bool,
    /// Underrun count captured after warmup, so the "too slow" warning ignores
    /// the startup priming burst and measures only sustained starvation.
    underrun_baseline: Option<u64>,
}

/// The loaded game's work RAM, or `None` when there is no game.
///
/// Every plugin call needs it, and with no game there is nothing to narrate — so callers treat
/// this the same way they already treated a failed RAM read, which is to produce no intents.
///
/// A free function over the field rather than a method on `Session`, because a method borrows
/// the whole of it: several callers pass the RAM straight into `self.plugin`, which needs the
/// plugin borrowed mutably at the same time. Taking just the one field keeps those borrows
/// disjoint, which is what the code did before this became an Option.
fn ram_of(emu: &Option<Emulator>) -> Option<&[u8]> {
    emu.as_ref().and_then(|emu| emu.main_ram().ok())
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        emu: Option<Emulator>,
        audio: Audio,
        arbiter: Arbiter,
        speech: Fanout,
        plugin: Box<dyn Plugin>,
        reload_spec: Option<PluginSpec>,
        rom: std::rc::Rc<Vec<u8>>,
        rom_path: Option<PathBuf>,
        settings: Settings,
        rom_id: Option<&str>,
    ) -> Self {
        Session {
            emu,
            audio,
            arbiter,
            speech,
            plugin,
            reload_spec,
            rom,
            rom_path,
            settings,
            slots: rom_id.map(SlotStore::new),
            active_slot: 0,
            paused: false,
            timing_disturbed: false,
            config: None,
            menu: None,
            paused_before_menu: false,
            show_map: false,
            nav_was_active: false,
            map_buffer: Vec::new(),
            map_dims: (0, 0),
            held_buttons: 0,
            quit: false,
            muted: false,
            audio_scratch: Vec::with_capacity(4096),
            beacon_mixer: BeaconMixer::new(beacon_emu::AUDIO_SAMPLE_RATE),
            last_spoken: None,
            speech_log: VecDeque::new(),
            frames: 0,
            warned_slow: false,
            underrun_baseline: None,
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

    /// Whether save slot `slot` holds a state. False with no game, so the menu's Save and
    /// Load levels come out empty rather than offering slots that cannot be written.
    fn occupied(&self, slot: u8) -> bool {
        self.slots
            .as_ref()
            .map(|store| store.occupied(slot))
            .unwrap_or(false)
    }

    /// Advances the emulator by exactly one frame and runs the plugin over it.
    pub fn step_one_frame(&mut self) {
        let Some(emu) = self.emu.as_mut() else {
            return;
        };
        emu.set_buttons(0, self.held_buttons);

        emu.run_frame();
        self.frames += 1;

        self.audio_scratch.clear();
        emu.drain_audio(&mut self.audio_scratch);
        if !self.audio_scratch.is_empty() {
            if self.muted {
                // Silence game and beacons alike, but keep the sample count so the
                // audio queue still paces emulation at normal speed.
                self.audio_scratch.iter_mut().for_each(|s| *s = 0.0);
            } else if self.settings.beacons.enabled {
                // Mix the plugin's spatial-audio beacons into the game audio before
                // it is queued. The beacons are owned here, so the mixer and the
                // buffer can be borrowed together.
                let beacons = self.plugin.beacons();
                self.beacon_mixer.mix(
                    &beacons,
                    &mut self.audio_scratch,
                    self.settings.beacons.volume_min,
                    self.settings.beacons.volume_max,
                    self.settings.beacons.music_duck,
                );
            }
            let scratch = std::mem::take(&mut self.audio_scratch);
            self.audio.submit(&scratch);
            self.audio_scratch = scratch;
        }

        // Instrumentation runs here: between frames, against real memory.
        let frame = self.frames;
        let intents = match ram_of(&self.emu) {
            Some(ram) => self.plugin.on_frame(ram, frame),
            None => Vec::new(),
        };
        self.dispatch(intents);

        // Bring the map up on its own the moment the plugin's navigation starts, so
        // the route shows without also pressing the map key. Edge-triggered on the
        // off->on transition, so hiding the map by hand while navigating stays hidden.
        let nav = self.plugin.navigation_active();
        if nav && !self.nav_was_active && self.plugin.has_map() {
            self.show_map = true;
        }
        self.nav_was_active = nav;

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

        // The audio pipeline underruns while it primes at startup (window and
        // stream setup, the plugin decoding its ROM tables); that is not the
        // machine being slow. Measure from a baseline taken after a warmup, so
        // only *sustained* starvation during play warns.
        const WARMUP_FRAMES: u64 = 600;
        if !self.timing_disturbed && !self.warned_slow && self.frames > WARMUP_FRAMES {
            let baseline = *self
                .underrun_baseline
                .get_or_insert_with(|| self.audio.underruns());
            if self.audio.underruns().saturating_sub(baseline) > 60 {
                self.warned_slow = true;
                self.say_now("Audio is struggling. This machine may be too slow for full speed.");
            }
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
        // Muted hushes the voice but still records the line, so `repeat_last` and
        // the MCP speech log keep working while silent.
        if self.muted {
            return;
        }
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
            Action::Mute => self.toggle_mute(),
            Action::ToggleMap => self.toggle_map(),
            Action::OpenInputConfig => self.open_input_config(),
            Action::OpenMenu => self.open_menu(),
            Action::Command(name) => self.run_command(&name),
        }
    }

    /// Toggles the global mute. The confirmation is spoken outside the muted
    /// window — before muting, after unmuting — so the player always hears it.
    fn toggle_mute(&mut self) {
        if self.muted {
            self.muted = false;
            self.say_now("Sound on.");
        } else {
            self.say_now("Muted.");
            self.muted = true;
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
    /// answer immediately. Empty output stays silent — a command with nothing to
    /// say says nothing, rather than a filler acknowledgement.
    pub fn run_command(&mut self, name: &str) {
        let intents = match ram_of(&self.emu) {
            Some(ram) => self.plugin.command(name, ram),
            None => Vec::new(),
        };
        for intent in intents {
            self.say_now(intent.text);
        }
    }

    pub fn save_state(&mut self) {
        let slot = self.active_slot;
        let (Some(emu), Some(store)) = (self.emu.as_mut(), self.slots.as_ref()) else {
            self.say_now("No game loaded.");
            return;
        };
        match emu.save_state() {
            Ok(data) => match store.save(slot, &data) {
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
        let (Some(emu), Some(store)) = (self.emu.as_mut(), self.slots.as_ref()) else {
            self.say_now("No game loaded.");
            return;
        };
        match store.load(slot) {
            Ok(Some(data)) => match emu.load_state(&data) {
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
        let state = if self.occupied(self.active_slot) {
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

    /// Shows the map from the start, if the plugin draws one. For `--map`.
    pub fn show_map_at_start(&mut self) {
        if self.plugin.has_map() {
            self.show_map = true;
            self.render_map();
        }
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
        let dims = match ram_of(&self.emu) {
            Some(ram) => self.plugin.draw(ram, frame, &mut self.map_buffer),
            None => None,
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

    /// Whether the map is currently shown, so the window can size for it.
    pub fn map_shown(&self) -> bool {
        self.show_map
    }

    /// The last rendered map pixels, for encoding by the MCP server.
    pub fn map_pixels(&self) -> &[u32] {
        &self.map_buffer
    }

    // --- Menu -------------------------------------------------------------

    /// What the menu needs to list its levels: which slots are taken, and the ROMs
    /// sitting beside the one that is running.
    ///
    /// Gathered fresh each time a level is entered, so a slot that filled since the
    /// menu opened reads as occupied.
    pub(crate) fn menu_context(&self) -> menu::Context {
        menu::Context {
            slots: (0..SLOTS).map(|s| self.occupied(s)).collect(),
            // Beside the loaded ROM, or where Beacon was started when there is none — which
            // is the case that matters, since starting with no game is how a player reaches
            // Open in the first place.
            verbosity: self.settings.arbiter.verbosity,
            speech: self.settings.speech.enabled,
            beacons: self.settings.beacons.enabled,
            braille: self.settings.braille.enabled,
            json_events: self.settings.speech.json_events,
            muted: self.muted,
            map_shown: self.show_map,
            // Everything the plugin offers, so a command is reachable without knowing its key.
            commands: self
                .plugin
                .commands()
                .iter()
                .map(|c| (c.id.clone(), c.label.clone()))
                .collect(),
            roms: match self.rom_path.as_ref().and_then(|p| p.parent()) {
                Some(dir) => rom::files_in(dir),
                None => std::env::current_dir()
                    .map(|dir| rom::files_in(&dir))
                    .unwrap_or_default(),
            },
        }
    }

    pub fn open_menu(&mut self) {
        // Freeze the game. Reading a list by ear takes as long as it takes, and
        // nothing should be walking into a pit meanwhile. Remembered, because a
        // player who had already paused did so on purpose and closing a menu is no
        // reason to set the game going again.
        self.paused_before_menu = self.paused;
        self.paused = true;
        self.timing_disturbed = true;
        self.held_buttons = 0;

        let open = Menu::open(&self.menu_context());
        let first = open.announce();
        self.menu = Some(open);
        self.say_now(menu::HELP);
        self.say_now(first);
    }

    /// Whether the menu is open.
    pub fn in_menu(&self) -> bool {
        self.menu.is_some()
    }

    /// The open menu, for the shell to publish to the platform's accessibility layer.
    pub fn menu(&self) -> Option<&Menu> {
        self.menu.as_ref()
    }

    /// Moves the selection, announcing what it landed on.
    pub fn menu_navigate(&mut self, delta: i32) {
        let Some(open) = self.menu.as_mut() else {
            return;
        };
        let said = open.navigate(delta);
        self.say_now(said);
    }

    /// Moves the selection to an absolute position, announcing what it landed on.
    ///
    /// For assistive technology, which drives a menu by naming a node rather than a
    /// direction: a reader's own review keys, or a click on an item.
    pub fn menu_select(&mut self, index: usize) {
        let Some(open) = self.menu.as_mut() else {
            return;
        };
        let said = open.select(index);
        self.say_now(said);
    }

    /// Chooses the selected entry: descends a level, or carries out its act.
    pub fn menu_choose(&mut self) {
        let ctx = self.menu_context();
        let Some(open) = self.menu.as_mut() else {
            return;
        };
        let chosen = open.choose(&ctx);
        match chosen {
            Chosen::Moved(said) | Chosen::Nothing(said) => self.say_now(said),
            Chosen::Act(act) => {
                // Choosing finishes with the menu. Every act either leaves Beacon,
                // replaces what is running, or is done the moment it is spoken, and
                // none of them wants a list still open behind it.
                self.menu = None;
                self.paused = self.paused_before_menu;
                self.perform(act);
            }
        }
    }

    /// Goes back a level, or closes if already at the top.
    pub fn menu_back(&mut self) {
        let Some(open) = self.menu.as_mut() else {
            return;
        };
        match open.back() {
            Backed::Moved(said) => self.say_now(said),
            Backed::Close => self.menu_close(),
        }
    }

    /// Closes the menu and resumes play.
    pub fn menu_close(&mut self) {
        self.menu = None;
        self.held_buttons = 0;
        self.paused = self.paused_before_menu;
        self.say_now("Menu closed.");
    }

    /// Carries out a menu act. Public so a platform's own menu — which does its own
    /// navigating and reports only what was chosen — reaches the same verbs.
    pub(crate) fn perform(&mut self, act: Act) {
        match act {
            Act::Exit => {
                self.say_now("Goodbye.");
                self.quit = true;
            }
            // Through the active slot rather than around it, so the slot the menu
            // acted on is the one the save and load KEYS then act on. Two ways to
            // reach the same slots that disagreed about which was current would be
            // worse than either alone.
            Act::SaveSlot(slot) => {
                self.active_slot = slot;
                self.save_state();
            }
            Act::LoadSlot(slot) => {
                self.active_slot = slot;
                self.load_state();
            }
            Act::MapKeys => self.open_input_config(),
            Act::OpenRom(path) => self.open_rom(&path),
            // Through set_setting, so the arbiter is kept in step and the change is persisted —
            // a setting changed from the menu should still be set next time Beacon starts.
            Act::SetSetting { key, value, said } => match self.set_setting(key, &value) {
                Ok(()) => self.say_now(said),
                Err(e) => {
                    eprintln!("set {key}: {e}");
                    self.say_now("Could not change that setting.");
                }
            },
            Act::ToggleMute => self.toggle_mute(),
            Act::ToggleMap => self.toggle_map(),
            Act::FrameAdvance => self.frame_advance(),
            Act::ReloadPlugin => match self.reload_plugin() {
                Ok(msg) => self.say_now(msg),
                Err(e) => {
                    eprintln!("reload plugin: {e}");
                    self.say_now("Could not reload the plugin.");
                }
            },
            Act::Command(name) => self.run_command(&name),
        }
    }

    /// Replaces what is running with another ROM.
    ///
    /// Everything derived from the old ROM goes with it: the emulator, the plugin
    /// its hash chose, and the save slots, which are per game. Keeping any of them
    /// would be worse than refusing — a state from one game loaded into another is
    /// not a save, and a plugin reading another game's memory narrates nonsense.
    ///
    /// A ROM that will not load leaves the session exactly as it was, so a mistyped
    /// or corrupt file costs the player nothing but the announcement.
    pub fn open_rom(&mut self, path: &Path) {
        // The old emulator goes before the new one is built. bsnes-jg permits exactly one live
        // instance, so creating the second while the first still exists fails with
        // AlreadyInstantiated — which is what Open did until this was an Option, meaning it
        // could never have worked and said only "Could not open that ROM."
        self.emu = None;
        let emu = match Emulator::load_at(path, self.audio.sample_rate()) {
            Ok(emu) => emu,
            Err(e) => {
                eprintln!("open {}: {e}", path.display());
                self.say_now("Could not open that ROM.");
                return;
            }
        };
        let bytes = rom::read(path);
        let sha1 = (!bytes.is_empty()).then(|| beacon_plugin::rom_sha1(&bytes));
        let (plugin, spec) = rom::select_plugin(sha1.as_deref(), &bytes);

        self.emu = Some(emu);
        self.plugin = plugin;
        self.reload_spec = spec;
        self.rom = bytes;
        self.slots = Some(SlotStore::new(sha1.as_deref().unwrap_or("unknown")));
        self.rom_path = Some(path.to_path_buf());
        self.active_slot = 0;
        self.frames = 0;
        self.held_buttons = 0;
        self.paused = false;
        self.show_map = false;
        self.nav_was_active = false;
        self.timing_disturbed = true;
        self.apply_plugin_default_keys();

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("that ROM");
        // The plugin is named too, and "no plugin" is the important case: it is the
        // difference between a game that will describe itself and one that will not.
        self.say_now(format!("Loaded {name}. {}.", self.plugin.name()));
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
        // With no game, the SNES's own dimensions, so the window still opens at a sensible
        // size and the menu has somewhere to be drawn.
        let Some(emu) = self.emu.as_ref() else {
            return FrameInfo {
                width: 256,
                height: 224,
                pitch: 256 * 4,
            };
        };
        emu.frame_info()
    }

    /// The current video frame's pixels.
    pub fn framebuffer(&self) -> &[u32] {
        match self.emu.as_ref() {
            Some(emu) => emu.framebuffer(),
            None => &[],
        }
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
        let ram = ram_of(&self.emu)?;
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

    /// Applies the plugin's suggested default keys for its commands.
    ///
    /// A suggestion fills a gap: it is applied only if the command has no binding
    /// and the key is free and not a game control. A user's own binding — of the
    /// command or of the key — always wins. Not persisted; it is a runtime
    /// default, re-applied each launch.
    /// Gives any built-in action the player has never bound its default keys.
    /// The rule itself is in [`action::apply_defaults`].
    pub fn apply_builtin_default_keys(&mut self) {
        action::apply_defaults(&mut self.settings.keymap);
    }

    pub fn apply_plugin_default_keys(&mut self) {
        // Collect first, so the plugin borrow ends before the keymap is touched.
        let suggestions: Vec<(String, String)> = self
            .plugin
            .commands()
            .iter()
            .filter_map(|c| {
                c.key
                    .as_ref()
                    .map(|k| (k.clone(), format!("command:{}", c.id)))
            })
            .collect();

        for (key, action) in suggestions {
            let command_unbound = self.settings.keymap.keys_for(&action).is_empty();
            let key_free = self.settings.keymap.action_for(&key).is_none();
            if command_unbound && key_free && !crate::input::is_game_input_name(&key) {
                self.settings.keymap.bind(&key, &action);
            }
        }
    }

    /// The keys currently bound to an action id.
    pub fn keys_for_action(&self, action_id: &str) -> Vec<String> {
        self.settings.keymap.keys_for(action_id)
    }

    /// The plugin's active spatial-audio beacons, for diagnostics.
    pub fn active_beacons(&self) -> Vec<beacon_plugin::BeaconState> {
        self.plugin.beacons()
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
        let plugin = LuaPlugin::load(&fresh, self.rom.clone()).map_err(|e| e.to_string())?;

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
        let Some(ram) = ram_of(&self.emu) else {
            return Err("no game loaded".to_string());
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
