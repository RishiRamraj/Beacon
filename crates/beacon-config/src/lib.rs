//! User settings.
//!
//! Two principles shape this, and they pull against each other:
//!
//! 1. **Nothing needs configuring to start.** Every setting has a default that
//!    works. A first run must never require editing a file, because for this
//!    audience that is where people give up.
//! 2. **Everything is configurable, at runtime.** Tolerance for chatter,
//!    speech rate, and voice vary enormously between players and between a
//!    first playthrough and a tenth. Requiring a restart, or a text editor, to
//!    change them is a barrier in itself.
//!
//! So: typed settings with defaults, an optional TOML file, and a string keyed
//! [`Settings::set`] so a hotkey, a menu, or a voice command can adjust
//! anything by name without a bespoke handler per field.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Everything a user can tune.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub speech: Speech,
    pub arbiter: ArbiterSettings,
    pub braille: Braille,
    pub beacons: Beacons,
    /// Key bindings. Serialized as `[keys]`; the field is `keymap` to avoid
    /// colliding with [`Settings::keys`], the list of scalar setting names.
    #[serde(rename = "keys")]
    pub keymap: Keymap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Speech {
    pub enabled: bool,
    /// Speech rate, -100 (slowest) to 100 (fastest).
    ///
    /// The default is deliberately faster than speech-dispatcher's, which is
    /// slow for anyone accustomed to a screen reader. Validated by listening,
    /// not chosen from a specification.
    pub rate: i8,
    /// Output module, e.g. "espeak-ng". Empty means whatever the system
    /// already uses, which respects a screen reader user's existing setup.
    pub module: String,
    /// Voice name within the module. Empty means the module default.
    pub voice: String,
    /// Emit line delimited JSON events on stdout for external tooling.
    pub json_events: bool,
}

impl Default for Speech {
    fn default() -> Self {
        Speech {
            enabled: true,
            rate: 60,
            module: String::new(),
            voice: String::new(),
            json_events: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArbiterSettings {
    /// 0 = critical only, 3 = everything.
    pub verbosity: u8,
    /// Utterances allowed through per frame.
    pub max_per_frame: usize,
    /// Burst size for a category's rate limit.
    pub bucket_capacity: f32,
    /// Sustained rate per category, utterances per second.
    pub bucket_refill_per_sec: f32,
}

impl Default for ArbiterSettings {
    fn default() -> Self {
        ArbiterSettings {
            verbosity: 2,
            max_per_frame: 2,
            bucket_capacity: 3.0,
            bucket_refill_per_sec: 1.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Braille {
    /// Off by default: unverified against real hardware, and a braille display
    /// fed the speech stream is worse than one fed nothing.
    pub enabled: bool,
    /// Stricter than speech. Braille is slow to read and messages overwrite
    /// each other, so only the most important things belong here.
    pub verbosity: u8,
}

impl Default for Braille {
    fn default() -> Self {
        Braille {
            enabled: false,
            verbosity: 1,
        }
    }
}

// Not `deny_unknown_fields`: an older config may carry a now-removed `volume`
// key, and it should be ignored on load rather than failing the whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beacons {
    /// Spatial-audio beacons: positioned tones a plugin places so the player
    /// hears where things are. On by default, but easily silenced.
    pub enabled: bool,
    /// Loudest a beacon gets, when whatever it marks is nearest. 0 to 1.
    pub volume_max: f32,
    /// Quietest a beacon gets, at the edge of its range. 0 to 1. A plugin's
    /// distance curve is mapped into `[volume_min, volume_max]`.
    pub volume_min: f32,
    /// How far the game audio is dipped while any beacon is sounding, so the
    /// cues cut through the music. 1.0 leaves the game at full volume; 0.5 is
    /// roughly -6 dB. Applied to the game buffer before beacons are added.
    pub music_duck: f32,
}

impl Default for Beacons {
    fn default() -> Self {
        Beacons {
            enabled: true,
            // Loud enough to carry over the music, with a wide floor-to-peak
            // range so a source audibly swells as the player closes on it.
            volume_max: 0.5,
            volume_min: 0.05,
            // Dip the music ~6 dB while a beacon plays so the cue stays clear.
            music_duck: 0.5,
        }
    }
}

/// Key bindings: physical key name to action id.
///
/// Keys are named as the host names them (e.g. `"KeyC"`, `"F5"`, `"Escape"`);
/// this crate does not know about `winit`, it just stores strings. Action ids
/// are the host's vocabulary too: a bare name for a built-in action
/// (`"save_state"`), or `"command:<id>"` for a plugin command (`"command:scan"`).
///
/// The map is the **complete** binding set, not a set of overrides: writing a
/// `[keys]` table in the settings file replaces the defaults wholesale, and a
/// runtime rebind writes the whole map back. Most users never touch it and get
/// [`Keymap::default`].
///
/// Game controls (the SNES buttons) are deliberately **not** here. They are a
/// separate, fixed mapping so that a rebindable action can never silently steal
/// a key the game needs mid-combat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keymap {
    bindings: BTreeMap<String, String>,
}

impl Keymap {
    /// The action bound to a key, if any.
    pub fn action_for(&self, key: &str) -> Option<&str> {
        self.bindings.get(key).map(String::as_str)
    }

    /// Binds a key to an action, replacing any previous binding of that key.
    pub fn bind(&mut self, key: impl Into<String>, action: impl Into<String>) {
        self.bindings.insert(key.into(), action.into());
    }

    /// Removes any binding for a key.
    pub fn unbind(&mut self, key: &str) {
        self.bindings.remove(key);
    }

    /// The keys currently bound to an action, sorted. Several keys may map to
    /// one action, so this is a list.
    pub fn keys_for(&self, action: &str) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, a)| a.as_str() == action)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Every binding, key to action.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.bindings.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl Default for Keymap {
    /// The out-of-the-box bindings.
    ///
    /// Chosen from the letter keys that are neither SNES buttons nor claimed by
    /// screen readers (function keys and the number row are avoided). Every one
    /// is rebindable; these are only a starting point. Plugin custom commands
    /// are unbound by default, since only the plugin knows what they are.
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        let mut bind = |k: &str, a: &str| {
            bindings.insert(k.to_string(), a.to_string());
        };
        bind("KeyC", "command:scan");
        bind("KeyE", "command:where");
        bind("KeyH", "command:status");
        bind("KeyV", "cycle_verbosity");
        bind("KeyR", "repeat_last");
        bind("Escape", "quit");
        bind("KeyT", "save_state");
        bind("KeyG", "load_state");
        bind("KeyN", "next_slot");
        bind("KeyB", "prev_slot");
        bind("KeyP", "pause");
        bind("KeyF", "frame_advance");
        bind("KeyM", "toggle_map");
        bind("KeyK", "bind");

        // Gamepad defaults, on the pad's extra buttons so a controller-only
        // player can reach the essentials and the configuration without a
        // keyboard. Everything else they bind themselves.
        bind("Pad:LeftThumb", "bind");
        bind("Pad:RightThumb", "command:scan");
        bind("Pad:LeftTrigger2", "command:where");
        bind("Pad:RightTrigger2", "command:status");
        Keymap { bindings }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(String),
    Serialise(String),
    /// No setting by that name.
    UnknownKey(String),
    /// The value did not parse as the setting's type.
    BadValue {
        key: String,
        value: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "settings file: {e}"),
            Error::Parse(m) => write!(f, "could not parse settings: {m}"),
            Error::Serialise(m) => write!(f, "could not write settings: {m}"),
            Error::UnknownKey(k) => write!(f, "no setting called '{k}'"),
            Error::BadValue { key, value } => {
                write!(f, "'{value}' is not a valid value for '{key}'")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

impl Settings {
    /// Loads settings, falling back to defaults if the file does not exist.
    ///
    /// A missing file is not an error: that is the normal first run.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| Error::Parse(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Writes settings, creating parent directories as needed.
    ///
    /// Called after a runtime change so adjustments made mid-game persist. A
    /// setting a player had to find once should not have to be found again.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::Serialise(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// The directory settings and related state (savestates, keymaps) live in.
    ///
    /// The parent of [`default_path`](Settings::default_path), so a caller
    /// wanting a sibling directory — `states/`, say — has one place to root it.
    pub fn config_dir() -> Option<PathBuf> {
        Self::default_path().and_then(|p| p.parent().map(Path::to_path_buf))
    }

    /// The conventional settings path for this user.
    pub fn default_path() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(dir).join("beacon/settings.toml"));
        }
        if let Ok(dir) = std::env::var("APPDATA") {
            return Some(PathBuf::from(dir).join("Beacon/settings.toml"));
        }
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".config/beacon/settings.toml"))
    }

    /// Every setting name, for a settings menu or a "what can I change?"
    /// command.
    pub fn keys() -> &'static [&'static str] {
        &[
            "speech.enabled",
            "speech.rate",
            "speech.module",
            "speech.voice",
            "speech.json_events",
            "arbiter.verbosity",
            "arbiter.max_per_frame",
            "arbiter.bucket_capacity",
            "arbiter.bucket_refill_per_sec",
            "braille.enabled",
            "braille.verbosity",
            "beacons.enabled",
            "beacons.volume_max",
            "beacons.volume_min",
            "beacons.music_duck",
        ]
    }

    /// Reads a setting by name, formatted for speaking back to the user.
    pub fn get(&self, key: &str) -> Result<String> {
        Ok(match key {
            "speech.enabled" => self.speech.enabled.to_string(),
            "speech.rate" => self.speech.rate.to_string(),
            "speech.module" => self.speech.module.clone(),
            "speech.voice" => self.speech.voice.clone(),
            "speech.json_events" => self.speech.json_events.to_string(),
            "arbiter.verbosity" => self.arbiter.verbosity.to_string(),
            "arbiter.max_per_frame" => self.arbiter.max_per_frame.to_string(),
            "arbiter.bucket_capacity" => self.arbiter.bucket_capacity.to_string(),
            "arbiter.bucket_refill_per_sec" => self.arbiter.bucket_refill_per_sec.to_string(),
            "braille.enabled" => self.braille.enabled.to_string(),
            "braille.verbosity" => self.braille.verbosity.to_string(),
            "beacons.enabled" => self.beacons.enabled.to_string(),
            "beacons.volume_max" => self.beacons.volume_max.to_string(),
            "beacons.volume_min" => self.beacons.volume_min.to_string(),
            "beacons.music_duck" => self.beacons.music_duck.to_string(),
            other => return Err(Error::UnknownKey(other.to_string())),
        })
    }

    /// Sets a setting by name.
    ///
    /// String keyed so one handler serves the settings menu, keyboard
    /// shortcuts, and the IPC command channel. A voice command of
    /// "set speech rate 80" needs no bespoke code path.
    ///
    /// Values are clamped rather than rejected where a range exists: a player
    /// asking for verbosity 9 wants it as loud as it goes, not an error.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        fn parse<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
            value.parse::<T>().map_err(|_| Error::BadValue {
                key: key.to_string(),
                value: value.to_string(),
            })
        }

        match key {
            "speech.enabled" => self.speech.enabled = parse(key, value)?,
            "speech.rate" => self.speech.rate = parse::<i32>(key, value)?.clamp(-100, 100) as i8,
            "speech.module" => self.speech.module = value.to_string(),
            "speech.voice" => self.speech.voice = value.to_string(),
            "speech.json_events" => self.speech.json_events = parse(key, value)?,
            "arbiter.verbosity" => self.arbiter.verbosity = parse::<u8>(key, value)?.min(3),
            "arbiter.max_per_frame" => {
                self.arbiter.max_per_frame = parse::<usize>(key, value)?.max(1)
            }
            "arbiter.bucket_capacity" => {
                self.arbiter.bucket_capacity = parse::<f32>(key, value)?.max(0.0)
            }
            "arbiter.bucket_refill_per_sec" => {
                self.arbiter.bucket_refill_per_sec = parse::<f32>(key, value)?.max(0.0)
            }
            "braille.enabled" => self.braille.enabled = parse(key, value)?,
            "braille.verbosity" => self.braille.verbosity = parse::<u8>(key, value)?.min(3),
            "beacons.enabled" => self.beacons.enabled = parse(key, value)?,
            "beacons.volume_max" => {
                self.beacons.volume_max = parse::<f32>(key, value)?.clamp(0.0, 1.0)
            }
            "beacons.volume_min" => {
                self.beacons.volume_min = parse::<f32>(key, value)?.clamp(0.0, 1.0)
            }
            "beacons.music_duck" => {
                self.beacons.music_duck = parse::<f32>(key, value)?.clamp(0.0, 1.0)
            }
            other => return Err(Error::UnknownKey(other.to_string())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_need_no_file() {
        let s = Settings::load(Path::new("/nonexistent/beacon/settings.toml")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn round_trips_through_toml() {
        let mut s = Settings::default();
        s.speech.rate = 85;
        s.arbiter.verbosity = 3;
        s.braille.enabled = true;
        s.keymap.bind("KeyD", "command:custom1");

        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn keymap_defaults_are_present_and_editable() {
        let mut k = Keymap::default();
        assert_eq!(k.action_for("Escape"), Some("quit"));
        assert_eq!(k.action_for("KeyF"), Some("frame_advance"));

        // A rebind replaces the key's action; several keys can share an action.
        k.bind("KeyC", "command:custom1");
        assert_eq!(k.action_for("KeyC"), Some("command:custom1"));
        k.bind("KeyO", "command:custom1");
        let mut keys = k.keys_for("command:custom1");
        keys.sort();
        assert_eq!(keys, vec!["KeyC".to_string(), "KeyO".to_string()]);

        k.unbind("KeyC");
        assert_eq!(k.action_for("KeyC"), None);
    }

    #[test]
    fn absent_keys_table_yields_default_bindings() {
        // A settings file with no [keys] must still be fully bound.
        let s: Settings = toml::from_str("[speech]\nrate = 20\n").unwrap();
        assert_eq!(s.keymap.action_for("Escape"), Some("quit"));
    }

    #[test]
    fn partial_files_keep_defaults_for_everything_else() {
        // A user hand-editing one value must not lose the rest.
        let s: Settings = toml::from_str("[speech]\nrate = 20\n").unwrap();
        assert_eq!(s.speech.rate, 20);
        assert_eq!(s.arbiter.verbosity, ArbiterSettings::default().verbosity);
        assert!(s.speech.enabled);
    }

    #[test]
    fn every_advertised_key_reads_and_writes() {
        // Guards against a field being added to the struct but not the string
        // interface, which would make it unreachable from a menu or a voice
        // command.
        let mut s = Settings::default();
        for key in Settings::keys() {
            let value = s.get(key).unwrap_or_else(|e| panic!("get {key}: {e}"));
            s.set(key, &value)
                .unwrap_or_else(|e| panic!("set {key} = {value:?}: {e}"));
        }
    }

    #[test]
    fn set_clamps_rather_than_rejecting() {
        let mut s = Settings::default();
        s.set("speech.rate", "999").unwrap();
        assert_eq!(s.speech.rate, 100);
        s.set("arbiter.verbosity", "9").unwrap();
        assert_eq!(s.arbiter.verbosity, 3);
    }

    #[test]
    fn unknown_keys_and_bad_values_are_reported() {
        let mut s = Settings::default();
        assert!(matches!(
            s.set("speech.speed", "10"),
            Err(Error::UnknownKey(_))
        ));
        assert!(matches!(
            s.set("speech.rate", "quickly"),
            Err(Error::BadValue { .. })
        ));
    }

    #[test]
    fn saves_and_reloads() {
        let dir = std::env::temp_dir().join("beacon-settings-test");
        let path = dir.join("settings.toml");
        let _ = std::fs::remove_dir_all(&dir);

        let mut s = Settings::default();
        s.set("speech.rate", "72").unwrap();
        s.save(&path).unwrap();

        assert_eq!(Settings::load(&path).unwrap().speech.rate, 72);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
