//! Per-game instrumentation, loaded as data rather than compiled in.
//!
//! A plugin is the accessibility knowledge for one game: which addresses hold
//! health and position, what counts as an event, what a scan should describe.
//! That knowledge is the actual product here, and it must not require rebuilding
//! Beacon to add or change. So a plugin is a TOML manifest plus a Lua script,
//! selected automatically by hashing the ROM. See
//! `docs/decisions/0004-plugin-model-toml-profile-plus-lua.md` for why, and
//! `docs/plugins.md` for the author-facing reference to the manifest and the Lua
//! host API.
//!
//! # What lives where
//!
//! - [`Manifest`] — the declarative part: game identity, the SHA-1s it matches,
//!   and named memory watches. Parsed from TOML.
//! - [`lua::LuaPlugin`] — the logic: a Lua script that reads memory each frame
//!   and **proposes** utterances. It never speaks; the [`beacon_output`] arbiter
//!   decides what survives.
//! - [`Registry`] — the set of known plugins, and the ROM-hash lookup that picks
//!   one without the user choosing anything.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub mod canvas;
pub mod lua;

pub use beacon_output::Intent;
pub use lua::LuaPlugin;

/// One game's instrumentation, whatever its implementation.
///
/// The host drives this and knows nothing about Lua: a native plugin, were one
/// ever needed, would implement the same trait. Both entry points take the
/// frame's work RAM because a plugin holds no reference to emulator memory
/// between calls; the borrow is only valid for the duration of the call.
pub trait Plugin {
    /// The game's name, for logging and for announcing what loaded.
    fn name(&self) -> &str;

    /// Reads a frame and proposes what might be worth saying.
    ///
    /// Proposes only. Being generous here is safe because the arbiter decides
    /// what is actually spoken; a plugin suppressing its own output would just
    /// reimplement arbitration badly.
    fn on_frame(&mut self, ram: &[u8], frame: u64) -> Vec<Intent>;

    /// Handles a user command (scan, where, status, ...).
    ///
    /// The returned intents are answers to a direct request, so the host speaks
    /// them immediately rather than putting them through rate limiting. An
    /// unknown command returns nothing.
    fn command(&mut self, name: &str, ram: &[u8]) -> Vec<Intent>;

    /// The plugin's own bindable commands, beyond the standard scan/where/status.
    ///
    /// These are what the host offers the user to bind keys to, so a plugin can
    /// expose game-specific actions ("read the current sign", "list inventory")
    /// without the host knowing anything about them. Declared in the manifest.
    fn commands(&self) -> &[CommandDecl] {
        &[]
    }

    /// Whether the plugin renders a map (defines `on_draw`).
    fn has_map(&self) -> bool {
        false
    }

    /// Renders the plugin's map for the current frame into `out`, returning its
    /// dimensions, or `None` if the plugin draws nothing.
    ///
    /// Called only while the map view is open. `out` is filled with `0x00RRGGBB`
    /// pixels, row-major, so the host blits it exactly as it blits a frame.
    fn draw(&mut self, _ram: &[u8], _frame: u64, _out: &mut Vec<u32>) -> Option<(u32, u32)> {
        None
    }

    /// Evaluates a snippet in the plugin's environment against the current frame,
    /// returning its result as a string. A debugging aid for probing memory and
    /// the plugin's own state; not every plugin supports it.
    fn eval(&mut self, _code: &str, _ram: &[u8]) -> Result<String, String> {
        Err("this plugin does not support eval".to_string())
    }
}

/// A bindable command a plugin declares.
///
/// The `id` is what the host sends back to [`Plugin::command`] and what a key
/// binds to (as `command:<id>`); the `label` is what is spoken when the user is
/// choosing what to bind. Kept in the manifest so the host can list a plugin's
/// commands without running any Lua.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    pub id: String,
    pub label: String,
}

/// The three commands the host itself always offers, independent of any plugin.
pub const STANDARD_COMMANDS: [&str; 3] = ["scan", "where", "status"];

/// The most custom commands a plugin may declare, beyond the standard three.
pub const MAX_CUSTOM_COMMANDS: usize = 10;

/// A plugin that does nothing, used when no plugin matches the ROM.
///
/// The game still runs; there is simply nothing instrumenting it. Having a
/// no-op plugin rather than an `Option` keeps the host's frame loop uniform: it
/// always has a plugin to call.
pub struct NullPlugin;

impl Plugin for NullPlugin {
    fn name(&self) -> &str {
        "no plugin"
    }
    fn on_frame(&mut self, _ram: &[u8], _frame: u64) -> Vec<Intent> {
        Vec::new()
    }
    fn command(&mut self, _name: &str, _ram: &[u8]) -> Vec<Intent> {
        Vec::new()
    }
}

/// A named memory location a plugin cares about.
///
/// Declaring watches in the manifest keeps the addresses in one place, readable
/// by anyone auditing the plugin, and lets the Lua read them by name rather than
/// scattering magic numbers through the script.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watch {
    /// SNES address, e.g. `0x7EF36D`. WRAM addresses are resolved against the
    /// live frame; see [`lua`].
    pub addr: u32,
    /// Width in bytes: 1, 2, or 3. Defaults to one.
    #[serde(default = "one_byte")]
    pub size: u8,
}

fn one_byte() -> u8 {
    1
}

/// Game identity and the ROMs a plugin matches.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    pub name: String,
    /// Lowercase SHA-1 hex strings of the headerless ROMs this plugin supports.
    /// A game may have several revisions and regional releases, so this is a
    /// list. No copyrighted data ever ships; only these hashes.
    #[serde(default)]
    pub sha1: Vec<String>,
    /// Region, informational for now (e.g. "NTSC-U").
    #[serde(default)]
    pub region: Option<String>,
}

/// The declarative half of a plugin: everything expressible without code.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub game: Game,
    /// Lua script filename, relative to the manifest.
    pub script: String,
    /// Named memory watches, exposed to the Lua as a `watch` table.
    #[serde(default)]
    pub watch: BTreeMap<String, Watch>,
    /// Custom bindable commands this plugin provides. `[[command]]` in TOML.
    #[serde(default, rename = "command")]
    pub commands: Vec<CommandDecl>,
}

impl Manifest {
    /// Parses and validates a manifest from TOML text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let manifest: Manifest =
            toml::from_str(text).map_err(|e| Error::Manifest(e.to_string()))?;
        manifest.validate_commands()?;
        Ok(manifest)
    }

    /// Rejects a command list that is too long, collides with the standard
    /// commands, or repeats an id — all author mistakes worth catching at load.
    fn validate_commands(&self) -> Result<(), Error> {
        if self.commands.len() > MAX_CUSTOM_COMMANDS {
            return Err(Error::Manifest(format!(
                "at most {MAX_CUSTOM_COMMANDS} custom commands, found {}",
                self.commands.len()
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.commands {
            if STANDARD_COMMANDS.contains(&c.id.as_str()) {
                return Err(Error::Manifest(format!(
                    "command id '{}' is reserved (scan/where/status are built in)",
                    c.id
                )));
            }
            if !seen.insert(c.id.as_str()) {
                return Err(Error::Manifest(format!("duplicate command id '{}'", c.id)));
            }
        }
        Ok(())
    }

    /// Whether this plugin claims a ROM with the given headerless SHA-1.
    pub fn matches(&self, rom_sha1: &str) -> bool {
        self.game
            .sha1
            .iter()
            .any(|h| h.eq_ignore_ascii_case(rom_sha1))
    }
}

/// Resolves a SNES address to an offset into the 128 KiB of work RAM.
///
/// Handles the two WRAM banks directly ($7E, $7F) and the low-RAM mirror the
/// first 8 KiB is visible through in banks $00-$3F and $80-$BF. Anything else —
/// ROM, hardware registers, unmapped space — returns `None`. This is the single
/// definition of Beacon's memory addressing, shared by the Lua `mem` API and any
/// other reader (the MCP debug server), so they can never drift.
pub fn wram_offset(addr: u32) -> Option<usize> {
    let bank = addr >> 16;
    let low = (addr & 0xFFFF) as usize;
    match bank {
        0x7E => Some(low),
        0x7F => Some(0x10000 + low),
        0x00..=0x3F | 0x80..=0xBF if low < 0x2000 => Some(low),
        _ => None,
    }
}

/// The SHA-1 a plugin is matched against.
///
/// Callers pass the ROM already stripped of any 512-byte copier header, so the
/// hash is of the headerless image. That matters: the same game with and without
/// a header would otherwise hash differently and fail to match.
pub fn rom_sha1(headerless_rom: &[u8]) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(headerless_rom);
    h.digest().to_string()
}

/// A manifest paired with the Lua source it names, ready to instantiate.
///
/// Keeping the source alongside the manifest lets built-in plugins embed both in
/// the binary while external ones read both from disk, without the loader caring
/// which it is holding.
#[derive(Debug, Clone)]
pub struct PluginSpec {
    pub manifest: Manifest,
    pub lua_source: String,
    /// A human-readable origin used in Lua error messages, e.g.
    /// `"alttp.lua (built-in)"`.
    pub chunk_name: String,
    /// The plugin directory a drop-in was read from, or `None` for a built-in.
    /// This is what makes reloading from disk possible.
    pub dir: Option<PathBuf>,
}

impl PluginSpec {
    /// Re-reads this plugin from disk, picking up any edits.
    ///
    /// A drop-in is re-read from its directory, so a plugin author's changes to
    /// the manifest or the Lua take effect. A built-in has no directory, so this
    /// returns it unchanged — reloading it just re-instantiates, which still
    /// resets a plugin's state, useful on its own.
    pub fn reloaded(&self) -> Result<PluginSpec, Error> {
        match &self.dir {
            Some(dir) => read_plugin_dir(dir),
            None => Ok(self.clone()),
        }
    }

    /// Whether this plugin can be re-read from disk (a drop-in, not a built-in).
    pub fn is_reloadable_from_disk(&self) -> bool {
        self.dir.is_some()
    }
}

/// Reads a plugin from a directory: its `*.toml` manifest and the Lua it names.
fn read_plugin_dir(dir: &Path) -> Result<PluginSpec, Error> {
    let manifest_path = find_manifest(dir).ok_or_else(|| Error::NoManifest(dir.to_owned()))?;
    let text = std::fs::read_to_string(&manifest_path)?;
    let manifest = Manifest::parse(&text)?;

    let script_path = dir.join(&manifest.script);
    let lua_source = std::fs::read_to_string(&script_path).map_err(|e| Error::ScriptRead {
        path: script_path.clone(),
        source: e,
    })?;

    Ok(PluginSpec {
        manifest,
        lua_source,
        chunk_name: script_path.display().to_string(),
        dir: Some(dir.to_owned()),
    })
}

/// The alttp reference plugin, compiled in so a fresh binary instruments the
/// game it was designed around with no setup. External plugins can still
/// override it by matching the same ROM.
const ALTTP_MANIFEST: &str = include_str!("../../../plugins/alttp/alttp.toml");
const ALTTP_LUA: &str = include_str!("../../../plugins/alttp/alttp.lua");

/// The set of known plugins and the ROM-hash lookup over them.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    specs: Vec<PluginSpec>,
}

impl Registry {
    /// A registry with only the built-in plugins.
    pub fn builtin() -> Self {
        let mut r = Registry::default();
        // The built-in manifest is authored in-tree, so a parse failure is a
        // build-time mistake, not a user's problem: fail loudly.
        let manifest = Manifest::parse(ALTTP_MANIFEST).expect("built-in alttp manifest must parse");
        r.specs.push(PluginSpec {
            manifest,
            lua_source: ALTTP_LUA.to_string(),
            chunk_name: "alttp.lua (built-in)".to_string(),
            dir: None,
        });
        r
    }

    /// Adds external plugins from a directory, if it exists.
    ///
    /// Each immediate subdirectory holding a `*.toml` manifest is one plugin. A
    /// missing directory is not an error: most users never add one. A single
    /// malformed plugin is skipped with a warning rather than aborting the load,
    /// because one broken drop-in should not stop the others working.
    pub fn load_dir(&mut self, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                eprintln!("plugins: cannot read {}: {e}", dir.display());
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Err(e) = self.load_plugin_dir(&path) {
                    eprintln!("plugins: skipping {}: {e}", path.display());
                }
            }
        }
    }

    fn load_plugin_dir(&mut self, dir: &Path) -> Result<(), Error> {
        self.specs.push(read_plugin_dir(dir)?);
        Ok(())
    }

    /// Selects the plugin matching a headerless ROM hash.
    ///
    /// Later-registered plugins win, so a user's drop-in overrides a built-in
    /// for the same ROM. Returns `None` when nothing matches: the game still
    /// runs, just without instrumentation.
    pub fn select(&self, rom_sha1: &str) -> Option<&PluginSpec> {
        self.specs
            .iter()
            .rev()
            .find(|s| s.manifest.matches(rom_sha1))
    }

    /// Every plugin known, for a "what's installed?" listing.
    pub fn specs(&self) -> &[PluginSpec] {
        &self.specs
    }
}

/// Finds the single `*.toml` manifest in a plugin directory.
fn find_manifest(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "toml"))
}

/// Everything that can go wrong loading or running a plugin.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// A plugin directory contained no `*.toml` manifest.
    NoManifest(PathBuf),
    /// The manifest did not parse.
    Manifest(String),
    /// The Lua script named by a manifest could not be read.
    ScriptRead {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The Lua raised, either at load or during a call.
    Lua(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NoManifest(p) => write!(f, "no .toml manifest in {}", p.display()),
            Error::Manifest(m) => write!(f, "manifest: {m}"),
            Error::ScriptRead { path, source } => {
                write!(f, "cannot read script {}: {source}", path.display())
            }
            Error::Lua(m) => write!(f, "lua: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<mlua::Error> for Error {
    fn from(e: mlua::Error) -> Self {
        Error::Lua(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_matches_case_insensitively() {
        let m = Manifest::parse(
            r#"
            script = "x.lua"
            [game]
            name = "Test"
            sha1 = ["ABCDEF0123456789"]
            [watch]
            health = { addr = 0x7EF36D, size = 1 }
            "#,
        )
        .unwrap();

        assert_eq!(m.game.name, "Test");
        assert_eq!(m.watch["health"].addr, 0x7EF36D);
        assert!(m.matches("abcdef0123456789"), "match is case-insensitive");
        assert!(!m.matches("deadbeef"));
    }

    #[test]
    fn watch_size_defaults_to_one() {
        let m = Manifest::parse(
            r#"
            script = "x.lua"
            [game]
            name = "Test"
            [watch]
            flag = { addr = 0x7E0010 }
            "#,
        )
        .unwrap();
        assert_eq!(m.watch["flag"].size, 1);
    }

    #[test]
    fn manifest_parses_custom_commands() {
        let m = Manifest::parse(
            r#"
            script = "x.lua"
            [game]
            name = "Test"
            [[command]]
            id = "read_sign"
            label = "Read the current sign"
            "#,
        )
        .unwrap();
        assert_eq!(m.commands.len(), 1);
        assert_eq!(m.commands[0].id, "read_sign");
        assert_eq!(m.commands[0].label, "Read the current sign");
    }

    #[test]
    fn custom_commands_may_not_shadow_standard_ones() {
        let err = Manifest::parse(
            r#"
            script = "x.lua"
            [game]
            name = "Test"
            [[command]]
            id = "scan"
            label = "nope"
            "#,
        );
        assert!(err.is_err(), "scan is reserved");
    }

    #[test]
    fn too_many_custom_commands_are_rejected() {
        let mut toml = String::from("script = \"x.lua\"\n[game]\nname = \"Test\"\n");
        for i in 0..(MAX_CUSTOM_COMMANDS + 1) {
            toml.push_str(&format!("[[command]]\nid = \"c{i}\"\nlabel = \"l{i}\"\n"));
        }
        assert!(Manifest::parse(&toml).is_err());
    }

    #[test]
    fn unknown_manifest_keys_are_rejected() {
        // Catches a plugin author's typo rather than silently ignoring it.
        let err = Manifest::parse(
            r#"
            script = "x.lua"
            [game]
            name = "Test"
            colour = "blue"
            "#,
        );
        assert!(err.is_err());
    }

    #[test]
    fn rom_hash_is_stable_and_lowercase_hex() {
        let h = rom_sha1(b"hello");
        assert_eq!(h, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }

    #[test]
    fn builtin_registry_selects_alttp_by_hash() {
        let r = Registry::builtin();
        // The hash the built-in manifest claims.
        let sha1 = &r.specs()[0].manifest.game.sha1[0].clone();
        assert!(r.select(sha1).is_some());
        assert!(r
            .select("0000000000000000000000000000000000000000")
            .is_none());
    }

    #[test]
    fn alttp_scan_describes_a_nearby_sprite() {
        // Drives the real built-in alttp plugin with synthetic sprite RAM, so the
        // scan logic (sprite table, direction, distance) is exercised as shipped.
        let r = Registry::builtin();
        let mut plugin = LuaPlugin::load(&r.specs()[0]).unwrap();

        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // module: overworld
        set(0x7E0011, 0x00); // submodule 0: in play
        set(0x7EF36C, 24); // max health
        set(0x7EF36D, 24); // health
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
                             // One active sprite, 0x40 pixels east of Link, no health -> "object".
        set(0x7E0DD0, 0x09); // slot 0 state: active
        set(0x7E0D10, 0x40);
        set(0x7E0D30, 0x01); // sprite X = 0x0140
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01); // sprite Y = 0x0100
        set(0x7E0E50, 0x00); // no health

        // First frame primes `prev`; the second gives scan a state to read.
        plugin.on_frame(&ram, 0);
        plugin.on_frame(&ram, 1);
        let out = plugin.command("scan", &ram);

        let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.iter().any(|t| t.contains("1 nearby")), "{texts:?}");
        assert!(
            texts
                .iter()
                .any(|t| t.contains("object") && t.contains("east")),
            "{texts:?}"
        );
    }

    #[test]
    fn alttp_enemy_proximity_speaks_on_approach_not_every_frame() {
        let r = Registry::builtin();
        let mut plugin = LuaPlugin::load(&r.specs()[0]).unwrap();

        // An in-play frame with one enemy (health > 0) `dx` pixels east of Link.
        let frame = |dx: u16| -> Vec<u8> {
            let mut ram = vec![0u8; 128 * 1024];
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E0010, 0x09);
            set(0x7E0011, 0x00);
            set(0x7EF36C, 24);
            set(0x7EF36D, 24);
            set(0x7E0022, 0x00);
            set(0x7E0023, 0x01); // Link X = 0x0100
            set(0x7E0020, 0x00);
            set(0x7E0021, 0x01); // Link Y = 0x0100
            let ex = 0x0100u16 + dx;
            set(0x7E0DD0, 0x09); // active
            set(0x7E0D10, (ex & 0xFF) as u8);
            set(0x7E0D30, (ex >> 8) as u8);
            set(0x7E0D00, 0x00);
            set(0x7E0D20, 0x01); // enemy Y = 0x0100
            set(0x7E0E50, 4); // health -> enemy
            ram
        };
        let announces = |out: &[Intent]| out.iter().any(|i| i.text.starts_with("Enemy"));

        plugin.on_frame(&frame(200), 0); // prime: enemy far (past "nearby")
        assert!(
            announces(&plugin.on_frame(&frame(100), 1)),
            "should speak when the enemy enters the near ring"
        );
        assert!(
            !announces(&plugin.on_frame(&frame(100), 2)),
            "should stay quiet while the enemy holds its ring"
        );
        assert!(
            announces(&plugin.on_frame(&frame(50), 3)),
            "should speak again when the enemy gets closer"
        );
    }
}
