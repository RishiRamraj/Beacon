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

        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
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
