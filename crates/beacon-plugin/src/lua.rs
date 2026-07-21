//! The Lua side of a plugin: the host API, and the frame and command entry
//! points.
//!
//! The author-facing reference for everything here — manifest format, every
//! `mem`/`say`/`on_command` signature, defaults, and the priority classes — is
//! `docs/plugins.md`. Keep the two in step: this module is the implementation of
//! the contract that document describes.
//!
//! A plugin script runs against a small set of globals the host installs:
//!
//! - `mem.u8/u16/u24(addr)` and `mem.slice(addr, len)` — bounds-checked reads of
//!   the current frame's work RAM. Addresses are SNES addresses (`0x7ExxxX`), so
//!   they read exactly as they appear in a memory map or a disassembly.
//! - `watch` — the manifest's named watches, e.g. `watch.health.addr`.
//! - `say(text, opts)` — **propose** an utterance. It is never spoken directly;
//!   the host arbiter decides. `opts` carries `priority`, `category`,
//!   `collapse_key`, `distance`, and `rate_limit`.
//! - `on_command(name, fn)` — register a handler for a user command.
//! - `log(level, message)` — diagnostics to stderr.
//!
//! The script defines a global `on_frame(frame)` that reads memory and calls
//! `say`. Anything it wants to remember between frames it keeps in its own Lua
//! state, which persists for the life of the plugin.
//!
//! # Memory, not a copy the plugin can hold
//!
//! The frame's RAM is copied into a host-owned buffer before each call and the
//! `mem` functions read through it. A plugin therefore cannot stash a reference
//! to emulator memory and read it later, when it would be stale or invalid: the
//! only memory it can see is the frame it was handed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use beacon_output::{Intent, Priority};
use mlua::{Function, Lua, Table, Value};

use crate::{CommandDecl, Error, Manifest, Plugin, PluginSpec};

/// Work RAM is 128 KiB: bank $7E is the first 64 KiB, $7F the second.
const WRAM_LEN: usize = 128 * 1024;

/// Resolves a SNES address to an offset into work RAM.
///
/// Handles the two WRAM banks directly and the low-RAM mirror that the first
/// 8 KiB is visible through in banks $00-$3F and $80-$BF. Anything else — ROM,
/// hardware registers, unmapped space — returns `None`, which surfaces to Lua as
/// `nil` rather than a wrong value.
fn wram_offset(addr: u32) -> Option<usize> {
    let bank = addr >> 16;
    let low = (addr & 0xFFFF) as usize;
    match bank {
        0x7E => Some(low),
        0x7F => Some(0x10000 + low),
        0x00..=0x3F | 0x80..=0xBF if low < 0x2000 => Some(low),
        _ => None,
    }
}

/// The shared frame buffer the `mem` closures read from.
type Ram = Rc<RefCell<Vec<u8>>>;
/// Where `say` deposits proposed intents until the host drains them.
type Intents = Rc<RefCell<Vec<Intent>>>;

/// A loaded Lua plugin.
pub struct LuaPlugin {
    lua: Lua,
    name: String,
    ram: Ram,
    intents: Intents,
    commands: Vec<CommandDecl>,
}

impl LuaPlugin {
    /// Instantiates a plugin from its spec: builds the Lua state, installs the
    /// host API, and runs the script once so it can register commands and define
    /// `on_frame`.
    pub fn load(spec: &PluginSpec) -> Result<Self, Error> {
        let lua = Lua::new();
        let ram: Ram = Rc::new(RefCell::new(vec![0u8; WRAM_LEN]));
        let intents: Intents = Rc::new(RefCell::new(Vec::new()));

        install_mem(&lua, &ram)?;
        install_say(&lua, &intents)?;
        install_log(&lua)?;
        install_commands_table(&lua)?;
        install_watch(&lua, &spec.manifest)?;

        // Running the chunk defines on_frame and registers commands. A syntax or
        // load-time error is a broken plugin; report it with the chunk name so
        // the author can find it.
        lua.load(spec.lua_source.as_str())
            .set_name(spec.chunk_name.as_str())
            .exec()?;

        Ok(LuaPlugin {
            lua,
            name: spec.manifest.game.name.clone(),
            ram,
            intents,
            commands: spec.manifest.commands.clone(),
        })
    }

    /// Copies the frame's RAM into the buffer the Lua reads through.
    fn stage_ram(&mut self, ram: &[u8]) {
        let mut buf = self.ram.borrow_mut();
        buf.clear();
        buf.extend_from_slice(ram);
    }

    /// Takes whatever `say` collected during the last call.
    fn drain(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents.borrow_mut())
    }
}

impl Plugin for LuaPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn commands(&self) -> &[CommandDecl] {
        &self.commands
    }

    fn on_frame(&mut self, ram: &[u8], frame: u64) -> Vec<Intent> {
        self.stage_ram(ram);

        // A plugin need not define on_frame — a purely command-driven one is
        // valid — so a missing global is silence, not an error.
        let on_frame: Value = match self.lua.globals().get("on_frame") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("plugin {}: {e}", self.name);
                return Vec::new();
            }
        };
        if let Value::Function(f) = on_frame {
            if let Err(e) = f.call::<()>(frame) {
                // A raising plugin must not take the host down with it. Report
                // and carry on; the game keeps running.
                eprintln!("plugin {} on_frame: {e}", self.name);
            }
        }

        self.drain()
    }

    fn command(&mut self, name: &str, ram: &[u8]) -> Vec<Intent> {
        self.stage_ram(ram);

        let handler = self
            .lua
            .globals()
            .get::<Table>("__beacon_commands")
            .and_then(|t| t.get::<Value>(name));

        if let Ok(Value::Function(f)) = handler {
            if let Err(e) = f.call::<()>(()) {
                eprintln!("plugin {} command {name}: {e}", self.name);
            }
        }

        self.drain()
    }
}

/// Installs the `mem` table: bounds-checked reads over the staged frame.
fn install_mem(lua: &Lua, ram: &Ram) -> Result<(), Error> {
    let mem = lua.create_table()?;

    let r = ram.clone();
    mem.set(
        "u8",
        lua.create_function(move |_, addr: u32| {
            let buf = r.borrow();
            Ok(wram_offset(addr).and_then(|o| buf.get(o).copied()))
        })?,
    )?;

    let r = ram.clone();
    mem.set(
        "u16",
        lua.create_function(move |_, addr: u32| {
            let buf = r.borrow();
            Ok(wram_offset(addr).and_then(|o| read_le(&buf, o, 2)))
        })?,
    )?;

    let r = ram.clone();
    mem.set(
        "u24",
        lua.create_function(move |_, addr: u32| {
            let buf = r.borrow();
            Ok(wram_offset(addr).and_then(|o| read_le(&buf, o, 3)))
        })?,
    )?;

    let r = ram.clone();
    mem.set(
        "slice",
        lua.create_function(move |lua, (addr, len): (u32, usize)| {
            let buf = r.borrow();
            // Out of range is an empty string, not an error: a scan reading past
            // the edge of a region should read nothing, not abort the frame.
            let bytes = wram_offset(addr)
                .and_then(|o| buf.get(o..o + len))
                .unwrap_or(&[]);
            lua.create_string(bytes)
        })?,
    )?;

    lua.globals().set("mem", mem)?;
    Ok(())
}

/// Reads a little-endian unsigned integer of `width` bytes.
fn read_le(buf: &[u8], offset: usize, width: usize) -> Option<u32> {
    let slice = buf.get(offset..offset + width)?;
    let mut v = 0u32;
    for (i, b) in slice.iter().enumerate() {
        v |= (*b as u32) << (8 * i);
    }
    Some(v)
}

/// Installs `say`: the propose-don't-speak entry point.
fn install_say(lua: &Lua, intents: &Intents) -> Result<(), Error> {
    let collector = intents.clone();
    let say = lua.create_function(move |_, (text, opts): (String, Option<Table>)| {
        let intent = build_intent(text, opts)?;
        collector.borrow_mut().push(intent);
        Ok(())
    })?;
    lua.globals().set("say", say)?;
    Ok(())
}

/// Turns a `say(text, opts)` call into an [`Intent`].
fn build_intent(text: String, opts: Option<Table>) -> mlua::Result<Intent> {
    let Some(opts) = opts else {
        // No metadata: least urgent, its own category. A plugin that says
        // nothing about priority gets the safe, easily-gated default.
        return Ok(Intent::new(text, Priority::Ambient, "general"));
    };

    let priority = opts
        .get::<Option<String>>("priority")?
        .map(|s| parse_priority(&s))
        .unwrap_or(Priority::Ambient);

    // Category defaults to the priority name, so rate limiting still separates
    // classes of message even when a plugin does not name categories.
    let category = opts
        .get::<Option<String>>("category")?
        .unwrap_or_else(|| priority_name(priority).to_string());

    let mut intent = Intent::new(text, priority, category);

    if let Some(key) = opts.get::<Option<String>>("collapse_key")? {
        let distance = opts.get::<Option<f64>>("distance")?.unwrap_or(f64::MAX) as f32;
        intent = intent.collapse(key, distance);
    }

    if let Some(spec) = opts.get::<Option<String>>("rate_limit")? {
        if let Some(window) = parse_duration(&spec) {
            intent = intent.dedup_for(window);
        }
    }

    Ok(intent)
}

fn parse_priority(s: &str) -> Priority {
    match s.to_ascii_lowercase().as_str() {
        "critical" => Priority::Critical,
        "navigation" | "nav" => Priority::Navigation,
        "interaction" => Priority::Interaction,
        _ => Priority::Ambient,
    }
}

fn priority_name(p: Priority) -> &'static str {
    match p {
        Priority::Critical => "critical",
        Priority::Navigation => "navigation",
        Priority::Interaction => "interaction",
        Priority::Ambient => "ambient",
    }
}

/// Parses a human duration like `"400ms"`, `"1s"`, or `"1500"` (milliseconds).
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if let Some(ms) = s.strip_suffix("ms") {
        return ms.trim().parse::<u64>().ok().map(Duration::from_millis);
    }
    if let Some(secs) = s.strip_suffix('s') {
        return secs.trim().parse::<f64>().ok().map(Duration::from_secs_f64);
    }
    s.parse::<u64>().ok().map(Duration::from_millis)
}

/// Installs `log(level, message)`, routed to stderr so it never touches the
/// stdout JSON stream.
fn install_log(lua: &Lua) -> Result<(), Error> {
    let log = lua.create_function(|_, (a, b): (String, Option<String>)| {
        match b {
            Some(msg) => eprintln!("[plugin/{a}] {msg}"),
            None => eprintln!("[plugin] {a}"),
        }
        Ok(())
    })?;
    lua.globals().set("log", log)?;
    Ok(())
}

/// Installs the command registry and `on_command(name, fn)`.
fn install_commands_table(lua: &Lua) -> Result<(), Error> {
    let commands = lua.create_table()?;
    lua.globals().set("__beacon_commands", commands)?;

    let register = lua.create_function(|lua, (name, func): (String, Function)| {
        let commands: Table = lua.globals().get("__beacon_commands")?;
        commands.set(name, func)?;
        Ok(())
    })?;
    lua.globals().set("on_command", register)?;
    Ok(())
}

/// Exposes the manifest's named watches to Lua as a `watch` table.
fn install_watch(lua: &Lua, manifest: &Manifest) -> Result<(), Error> {
    let watch = lua.create_table()?;
    for (name, w) in &manifest.watch {
        let entry = lua.create_table()?;
        entry.set("addr", w.addr)?;
        entry.set("size", w.size)?;
        watch.set(name.as_str(), entry)?;
    }
    lua.globals().set("watch", watch)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_from(lua: &str) -> LuaPlugin {
        let manifest = Manifest::parse(
            r#"
            script = "t.lua"
            [game]
            name = "Test"
            [watch]
            health = { addr = 0x7EF36D, size = 1 }
            "#,
        )
        .unwrap();
        let spec = PluginSpec {
            manifest,
            lua_source: lua.to_string(),
            chunk_name: "test.lua".to_string(),
        };
        LuaPlugin::load(&spec).unwrap()
    }

    fn ram_with(pairs: &[(u32, u8)]) -> Vec<u8> {
        let mut ram = vec![0u8; WRAM_LEN];
        for (addr, v) in pairs {
            ram[wram_offset(*addr).unwrap()] = *v;
        }
        ram
    }

    #[test]
    fn say_becomes_an_intent_with_metadata() {
        let mut p = plugin_from(
            r#"
            function on_frame(frame)
              say("Hit.", { priority = "critical", category = "combat", rate_limit = "400ms" })
            end
            "#,
        );
        let out = p.on_frame(&vec![0u8; WRAM_LEN], 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Hit.");
        assert_eq!(out[0].priority, Priority::Critical);
        assert_eq!(out[0].category, "combat");
        assert_eq!(out[0].dedup_for, Some(Duration::from_millis(400)));
    }

    #[test]
    fn mem_reads_resolve_snes_addresses() {
        let mut p = plugin_from(
            r#"
            function on_frame(frame)
              local h = mem.u8(0x7EF36D)
              local xy = mem.u16(0x7E0020)
              say(string.format("%d %d", h, xy))
            end
            "#,
        );
        let ram = ram_with(&[(0x7EF36D, 12), (0x7E0020, 0x34), (0x7E0021, 0x12)]);
        let out = p.on_frame(&ram, 0);
        assert_eq!(out[0].text, "12 4660"); // 0x1234 = 4660, little-endian.
    }

    #[test]
    fn out_of_range_reads_are_nil() {
        let mut p = plugin_from(
            r#"
            function on_frame(frame)
              if mem.u8(0x008000) == nil then say("nil") else say("value") end
            end
            "#,
        );
        // $00:8000 is ROM, not WRAM, so it reads as nil.
        let out = p.on_frame(&vec![0u8; WRAM_LEN], 0);
        assert_eq!(out[0].text, "nil");
    }

    #[test]
    fn watch_table_carries_manifest_addresses() {
        let mut p = plugin_from(
            r#"
            function on_frame(frame)
              say(string.format("%d", watch.health.addr))
            end
            "#,
        );
        let out = p.on_frame(&vec![0u8; WRAM_LEN], 0);
        assert_eq!(out[0].text, format!("{}", 0x7EF36D));
    }

    #[test]
    fn commands_register_and_run() {
        let mut p = plugin_from(
            r#"
            on_command("status", function()
              say("healthy", { priority = "navigation" })
            end)
            "#,
        );
        let out = p.command("status", &vec![0u8; WRAM_LEN]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "healthy");
        assert_eq!(out[0].priority, Priority::Navigation);

        // An unregistered command is simply silent.
        assert!(p.command("nonexistent", &vec![0u8; WRAM_LEN]).is_empty());
    }

    #[test]
    fn plugin_state_persists_between_frames() {
        let mut p = plugin_from(
            r#"
            local prev = nil
            function on_frame(frame)
              local h = mem.u8(0x7EF36D)
              if prev ~= nil and h < prev then say("hit") end
              prev = h
            end
            "#,
        );
        assert!(p.on_frame(&ram_with(&[(0x7EF36D, 24)]), 0).is_empty());
        let out = p.on_frame(&ram_with(&[(0x7EF36D, 16)]), 1);
        assert_eq!(out[0].text, "hit");
    }

    #[test]
    fn duration_parsing() {
        assert_eq!(parse_duration("400ms"), Some(Duration::from_millis(400)));
        assert_eq!(parse_duration("1s"), Some(Duration::from_secs(1)));
        assert_eq!(parse_duration("2.5s"), Some(Duration::from_secs_f64(2.5)));
        assert_eq!(parse_duration("750"), Some(Duration::from_millis(750)));
        assert_eq!(parse_duration("nonsense"), None);
    }
}
