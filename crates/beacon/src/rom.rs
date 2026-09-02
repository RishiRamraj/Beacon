//! Reading a ROM and picking its plugin.
//!
//! Lifted out of `main` so the session can do it again without restarting: opening
//! a ROM from the menu replaces the emulator, the plugin and the save slots, which
//! needs exactly what starting up needs.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use beacon_plugin::{LuaPlugin, NullPlugin, Plugin, PluginSpec, Registry};

/// Directories searched for drop-in plugins, in addition to the built-ins.
///
/// A `plugins/` directory beside the executable is the shipped layout; one in
/// the working directory is the convenience during development. Both are
/// optional: a missing directory is not an error.
pub fn plugin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("plugins"));
        }
    }
    dirs.push(PathBuf::from("plugins"));
    dirs
}

/// Reads the ROM, stripped of any copier header, for hashing and for plugins to
/// decode static game data. Empty (with a message) if the file cannot be read.
pub fn read(rom_path: &Path) -> Rc<Vec<u8>> {
    match std::fs::read(rom_path) {
        Ok(bytes) => Rc::new(beacon_emu::strip_copier_header(&bytes).to_vec()),
        Err(e) => {
            eprintln!("could not read ROM: {e}");
            Rc::new(Vec::new())
        }
    }
}

/// Picks the plugin matching a ROM hash, falling back to no instrumentation.
///
/// The user never chooses: identification is by headerless SHA-1. A ROM with no
/// matching plugin still plays, just silently, and a plugin that fails to load
/// is reported rather than fatal. The plugin is handed the ROM so it can decode
/// static game data at load.
pub fn select_plugin(
    sha1: Option<&str>,
    rom: &Rc<Vec<u8>>,
) -> (Box<dyn Plugin>, Option<PluginSpec>) {
    let Some(sha1) = sha1 else {
        return (Box::new(NullPlugin), None);
    };

    let mut registry = Registry::builtin();
    for dir in plugin_dirs() {
        registry.load_dir(&dir);
    }

    match registry.select(sha1) {
        Some(spec) => match LuaPlugin::load(spec, rom.clone()) {
            Ok(plugin) => {
                eprintln!("plugin: {}", plugin.name());
                // Keep the spec so the session can reload the plugin later.
                (Box::new(plugin), Some(spec.clone()))
            }
            Err(e) => {
                eprintln!("plugin failed to load, running without instrumentation: {e}");
                (Box::new(NullPlugin), None)
            }
        },
        None => {
            eprintln!("no plugin matches this ROM (sha1 {sha1}); running without instrumentation");
            (Box::new(NullPlugin), None)
        }
    }
}

/// The ROMs in a directory, as `(label, path)` sorted by label.
///
/// Labelled by file stem rather than file name, because ".sfc" read out on every
/// entry of a long list is noise. Only the two SNES extensions, so a directory of
/// mixed files does not offer up things the emulator cannot load.
pub fn files_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("sfc") || e.eq_ignore_ascii_case("smc"))
                .unwrap_or(false)
        })
        .filter_map(|p| {
            let label = p.file_stem()?.to_str()?.to_string();
            Some((label, p))
        })
        .collect();
    out.sort_by_key(|(label, _)| label.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_snes_roms_labelled_without_their_extension() {
        let dir = std::env::temp_dir().join(format!("beacon-rom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["Zelda.sfc", "metroid.SMC", "notes.txt", "save.srm"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }

        let found = files_in(&dir);
        let labels: Vec<&str> = found.iter().map(|(l, _)| l.as_str()).collect();
        // Sorted case-insensitively, so a list read aloud is in the order a
        // listener would expect rather than upper case first.
        assert_eq!(labels, vec!["metroid", "Zelda"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_that_is_not_there_offers_nothing() {
        assert!(files_in(Path::new("/no/such/directory/at/all")).is_empty());
    }
}
