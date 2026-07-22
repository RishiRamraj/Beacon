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

use crate::canvas::{self, Canvas};
use crate::{CommandDecl, Error, Manifest, Plugin, PluginSpec};

/// Work RAM is 128 KiB: bank $7E is the first 64 KiB, $7F the second.
const WRAM_LEN: usize = 128 * 1024;

use crate::wram_offset;

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
    canvas: Canvas,
    has_draw: bool,
}

impl LuaPlugin {
    /// Instantiates a plugin from its spec: builds the Lua state, installs the
    /// host API, and runs the script once so it can register commands and define
    /// `on_frame`.
    ///
    /// `rom` is the headerless ROM, exposed to Lua as `rom` so a plugin can
    /// decode static game data (dialogue tables, lookup tables) at load. Pass an
    /// empty `Rc` when no ROM is available; reads then return `nil`.
    pub fn load(spec: &PluginSpec, rom: Rc<Vec<u8>>) -> Result<Self, Error> {
        let lua = Lua::new();
        let ram: Ram = Rc::new(RefCell::new(vec![0u8; WRAM_LEN]));
        let intents: Intents = Rc::new(RefCell::new(Vec::new()));

        let canvas = Canvas::new();

        install_mem(&lua, &ram)?;
        install_say(&lua, &intents)?;
        install_log(&lua)?;
        install_commands_table(&lua)?;
        install_watch(&lua, &spec.manifest)?;
        install_canvas(&lua, &canvas)?;
        install_rom(&lua, rom)?;

        // Running the chunk defines on_frame and registers commands. A syntax or
        // load-time error is a broken plugin; report it with the chunk name so
        // the author can find it.
        lua.load(spec.lua_source.as_str())
            .set_name(spec.chunk_name.as_str())
            .exec()?;

        // Whether the plugin draws a map is fixed at load: it defined on_draw or
        // it did not.
        let has_draw = matches!(lua.globals().get("on_draw"), Ok(Value::Function(_)));

        Ok(LuaPlugin {
            lua,
            name: spec.manifest.game.name.clone(),
            ram,
            intents,
            commands: spec.manifest.commands.clone(),
            canvas,
            has_draw,
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

    fn has_map(&self) -> bool {
        self.has_draw
    }

    fn draw(&mut self, ram: &[u8], frame: u64, out: &mut Vec<u32>) -> Option<(u32, u32)> {
        if !self.has_draw {
            return None;
        }
        self.stage_ram(ram);

        let globals = self.lua.globals();
        let on_draw = match globals.get::<Value>("on_draw") {
            Ok(Value::Function(f)) => f,
            _ => return None,
        };
        let canvas_table: Value = globals.get("__beacon_canvas").ok()?;
        if let Err(e) = on_draw.call::<()>((canvas_table, frame)) {
            eprintln!("plugin {} on_draw: {e}", self.name);
            return None;
        }

        self.canvas.copy_into(out);
        Some((canvas::WIDTH, canvas::HEIGHT))
    }

    fn eval(&mut self, code: &str, ram: &[u8]) -> Result<String, String> {
        self.stage_ram(ram);
        // Evaluate as an expression first (so "mem.u8(0x10)" returns a value);
        // fall back to executing it as a statement block.
        let value = self
            .lua
            .load(code)
            .set_name("eval")
            .eval::<Value>()
            .or_else(|_| {
                self.lua
                    .load(code)
                    .set_name("eval")
                    .exec()
                    .map(|_| Value::Nil)
            })
            .map_err(|e| e.to_string())?;
        Ok(describe_value(&value))
    }
}

/// Renders a Lua value as a short human string for an eval result.
fn describe_value(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy().to_string(),
        Value::Table(_) => "<table>".to_string(),
        Value::Function(_) => "<function>".to_string(),
        other => format!("<{}>", other.type_name()),
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

/// Installs the `canvas` table: the map-drawing primitives.
///
/// Methods are called with the colon syntax (`canvas:pixel(...)`), so each
/// receives the table as its first argument, which is ignored — the pixels live
/// in the shared [`Canvas`], not the table.
fn install_canvas(lua: &Lua, canvas: &Canvas) -> Result<(), Error> {
    let table = lua.create_table()?;
    table.set("width", canvas::WIDTH)?;
    table.set("height", canvas::HEIGHT)?;

    let c = canvas.clone();
    table.set(
        "clear",
        lua.create_function(move |_, (_t, color): (Table, u32)| {
            c.clear(color);
            Ok(())
        })?,
    )?;

    let c = canvas.clone();
    table.set(
        "pixel",
        lua.create_function(move |_, (_t, x, y, color): (Table, i64, i64, u32)| {
            c.pixel(x, y, color);
            Ok(())
        })?,
    )?;

    let c = canvas.clone();
    table.set(
        "rect",
        lua.create_function(
            move |_, (_t, x, y, w, h, color): (Table, i64, i64, i64, i64, u32)| {
                c.rect(x, y, w, h, color);
                Ok(())
            },
        )?,
    )?;

    let c = canvas.clone();
    table.set(
        "line",
        lua.create_function(
            move |_, (_t, x0, y0, x1, y1, color): (Table, i64, i64, i64, i64, u32)| {
                c.line(x0, y0, x1, y1, color);
                Ok(())
            },
        )?,
    )?;

    let c = canvas.clone();
    table.set(
        "text",
        lua.create_function(
            move |_, (_t, x, y, s, color): (Table, i64, i64, String, u32)| {
                c.text(x, y, &s, color);
                Ok(())
            },
        )?,
    )?;

    lua.globals().set("__beacon_canvas", table)?;
    Ok(())
}

/// Installs the `rom` table: read-only access to the headerless ROM by file
/// offset, for a plugin decoding static game data at load.
///
/// Offsets are raw file offsets into the headerless image; a plugin maps SNES
/// addresses to offsets itself (the mapping is game-specific). An out-of-range
/// read is `nil`, and `rom.size` is the length in bytes.
fn install_rom(lua: &Lua, rom: Rc<Vec<u8>>) -> Result<(), Error> {
    let table = lua.create_table()?;
    table.set("size", rom.len() as u32)?;

    let r = rom.clone();
    table.set(
        "u8",
        lua.create_function(move |_, off: usize| Ok(r.get(off).copied()))?,
    )?;

    let r = rom.clone();
    table.set(
        "slice",
        lua.create_function(move |lua, (off, len): (usize, usize)| {
            let bytes = r.get(off..off.saturating_add(len)).unwrap_or(&[]);
            lua.create_string(bytes)
        })?,
    )?;

    lua.globals().set("rom", table)?;
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
        plugin_from_rom(lua, Vec::new())
    }

    fn plugin_from_rom(lua: &str, rom: Vec<u8>) -> LuaPlugin {
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
            dir: None,
        };
        LuaPlugin::load(&spec, std::rc::Rc::new(rom)).unwrap()
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
    fn on_draw_renders_into_the_buffer() {
        let mut p = plugin_from(
            r#"
            function on_draw(canvas)
              canvas:clear(0x000000)
              canvas:rect(10, 10, 4, 4, 0xFF0000)
            end
            "#,
        );
        assert!(p.has_map());
        let mut out = Vec::new();
        let dims = p.draw(&vec![0u8; WRAM_LEN], 0, &mut out);
        assert_eq!(dims, Some((256, 256)));
        // The filled rectangle is red where drawn, black elsewhere.
        assert_eq!(out[(10 * 256 + 10) as usize], 0xFF0000);
        assert_eq!(out[0], 0x000000);
    }

    #[test]
    fn a_plugin_without_on_draw_has_no_map() {
        let mut p = plugin_from("function on_frame(frame) end");
        assert!(!p.has_map());
        assert_eq!(p.draw(&vec![0u8; WRAM_LEN], 0, &mut Vec::new()), None);
    }

    #[test]
    fn rom_reads_bytes_by_offset() {
        let mut p = plugin_from_rom(
            r#"
            function on_frame(frame)
              local s = rom.slice(0, 2) -- bytes 0xAA 0xBB as a string
              say(string.format("%d %d %d", rom.size, rom.u8(1), string.byte(s, 1)))
            end
            "#,
            vec![0xAA, 0xBB, 0xCC],
        );
        let out = p.on_frame(&vec![0u8; WRAM_LEN], 0);
        // size 3, byte[1]=0xBB=187, slice byte 1 = 0xAA=170
        assert_eq!(out[0].text, "3 187 170");
    }

    #[test]
    fn rom_out_of_range_is_nil() {
        let mut p = plugin_from_rom(
            r#"
            function on_frame(frame)
              if rom.u8(99) == nil then say("nil") else say("value") end
            end
            "#,
            vec![1, 2, 3],
        );
        assert_eq!(p.on_frame(&vec![0u8; WRAM_LEN], 0)[0].text, "nil");
    }

    #[test]
    fn eval_reads_memory_and_returns_values() {
        let mut p = plugin_from("");
        let ram = ram_with(&[(0x7EF36D, 12)]);
        assert_eq!(p.eval("return mem.u8(0x7EF36D)", &ram).unwrap(), "12");
        assert_eq!(p.eval("return 2 + 3", &ram).unwrap(), "5");
        assert_eq!(p.eval("return 'hi'", &ram).unwrap(), "hi");
        // A statement block (no return) evaluates to nil, not an error.
        assert_eq!(p.eval("local x = 1", &ram).unwrap(), "nil");
        // A syntax error surfaces as an Err, not a panic.
        assert!(p.eval("this is not lua", &ram).is_err());
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
