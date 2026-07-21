//! Savestate slots on disk.
//!
//! A savestate is the emulator's serialized state ([`Emulator::save_state`]).
//! This stores a handful of them per game, in numbered slots, so a player can
//! keep several and return to any. Slots are keyed by the ROM's hash, so states
//! for one game never collide with another's, and the same ROM finds its states
//! again on the next run.
//!
//! [`Emulator::save_state`]: beacon_emu::Emulator::save_state

use std::path::PathBuf;

/// The number of slots, `0` through `SLOTS - 1`.
pub const SLOTS: u8 = 10;

/// Where savestates for one game live and how to name them.
pub struct SlotStore {
    dir: Option<PathBuf>,
}

impl SlotStore {
    /// A store rooted at `<config>/states/<rom_id>`.
    ///
    /// `rom_id` is the ROM's headerless SHA-1, so two games never share slots.
    /// If no config directory can be found the store still works — every
    /// operation just reports that saving is unavailable, rather than the host
    /// having to special-case its absence.
    pub fn new(rom_id: &str) -> Self {
        let dir = beacon_config::Settings::config_dir().map(|c| c.join("states").join(rom_id));
        SlotStore { dir }
    }

    /// The file backing a slot, if a directory is known.
    fn slot_path(&self, slot: u8) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{slot}.state")))
    }

    /// Writes a savestate to a slot, creating the directory as needed.
    pub fn save(&self, slot: u8, data: &[u8]) -> std::io::Result<()> {
        let path = self.slot_path(slot).ok_or_else(no_config)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)
    }

    /// Reads a slot's savestate, or `None` if the slot is empty.
    ///
    /// A missing file is not an error: it is simply an empty slot, which the
    /// caller reports as "no save in slot N" rather than a failure.
    pub fn load(&self, slot: u8) -> std::io::Result<Option<Vec<u8>>> {
        let Some(path) = self.slot_path(slot) else {
            return Err(no_config());
        };
        match std::fs::read(path) {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Whether a slot currently holds a savestate.
    pub fn occupied(&self, slot: u8) -> bool {
        self.slot_path(slot).is_some_and(|p| p.exists())
    }
}

fn no_config() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no config directory for savestates",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_load_back_and_empty_slots_are_none() {
        // Root a store at a temp dir by hand, bypassing the config lookup.
        let dir = std::env::temp_dir().join(format!("beacon-slots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SlotStore {
            dir: Some(dir.clone()),
        };

        assert!(store.load(3).unwrap().is_none(), "slot starts empty");
        assert!(!store.occupied(3));

        store.save(3, b"savestate-bytes").unwrap();
        assert!(store.occupied(3));
        assert_eq!(store.load(3).unwrap().unwrap(), b"savestate-bytes");

        // A different slot is still empty.
        assert!(store.load(4).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
