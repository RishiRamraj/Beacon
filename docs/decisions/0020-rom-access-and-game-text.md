# ADR 0020: ROM access for plugins, and reading the game's own text

- **Status:** Accepted (implemented)
- **Date:** 2026-07-21

## Context

Beacon spoke *about* the game — health, enemies, position — but not the game's own words: the
dialogue boxes, signs, item descriptions, and menu text a sighted player reads on screen. That
is a large part of what the game communicates, and without it a blind player misses the story
and much of the interface. The alttp-navi proof of concept could produce this text; the task was
to bring it to Beacon.

ALttP holds its dialogue as a compressed table in the ROM (a 95-character alphabet, a 97-entry
dictionary, and command bytes across two banks). At runtime WRAM `$7E1CF0` holds the id of the
message currently displayed. So reading the game's text needs two things Beacon lacked: a way for
a plugin to read the **ROM** (not just WRAM), and the decoder.

## Decision

**Add read-only ROM access to the plugin API.** The host retains the headerless ROM and exposes
it to Lua as `rom` — `rom.u8(offset)`, `rom.slice(offset, len)`, and `rom.size`, by raw file
offset. This is the `rom` capability the design always anticipated. A plugin maps SNES addresses
to file offsets itself, because that mapping is game-specific.

**Decode the dialogue in the plugin, at load.** The ALttP decoder is ported into `alttp.lua`
(from alttp-navi), reading the whole ROM once via `rom.slice` and building a table of messages
keyed by id. Game-specific decoding lives in the plugin, not the host — the same separation as
every other piece of game knowledge. Decoding happens once at load (a single slice plus a Lua
loop over the dialogue banks), fast enough to need no caching.

**Read the current message at runtime.** When a text or menu box opens (module `$0E`), the plugin
reads `$7E1CF0` and speaks the decoded message. A `read_text` command re-reads the current box on
demand. Because ALttP's menu text (the save menu, for instance) lives in the same table, this
covers menus as well as dialogue.

## Why this shape

- **ROM access in the plugin, not decoding in the host.** Keeping the decoder in Lua preserves
  game-agnosticism: the host gained a generic `rom` reader, and all ALttP knowledge stayed in the
  ALttP plugin. A second game's text is a different decoder in a different plugin.
- **Decode at load, not offline.** [ADR 0004](0004-plugin-model-toml-profile-plus-lua.md)
  imagined a `beacon-romdump` tool writing a cache. For dialogue that is unnecessary — the decode
  is milliseconds — and an offline step is a setup barrier, which for this audience is where
  people give up. In-process decode-at-load keeps the single-file, zero-setup promise. The offline
  cache remains available if some future plugin needs genuinely heavy ROM parsing.
- **No copyrighted data ships.** The decoder and the address tables are code; the text comes from
  the user's own ROM at runtime. Nothing decoded is embedded.

## Verification

The Lua decoder was checked against alttp-navi's Python decoder on the real ROM: **byte-for-byte
identical**, 397 messages, including the save menu, NPC dialogue, and item descriptions. The
runtime path was confirmed live through the MCP tools — driving into the game, the opening
telepathy message was spoken automatically as its box opened, and `read_text` returned it in full.

## Consequences

- `LuaPlugin::load` now takes the ROM; the session retains it to hand back on `reload_plugin`.
  Tests that build a plugin pass an empty ROM, so `rom` reads return `nil` — a plugin that needs
  the ROM degrades to silence rather than failing.
- The dialogue table lives in the plugin's Lua state (a few tens of KB). It is exposed as a
  `dialog` global within that plugin's own namespace, which also makes it inspectable with
  `eval_lua` when developing.
- The message-box trigger is `module → $0E`. If some in-game text uses a different path, it will
  be missed until the trigger is broadened; `read_text` is the manual fallback meanwhile.

## Alternatives considered

- **Decode the ROM host-side in Rust** — rejected; it puts game-specific code in the host, the
  coupling the plugin model exists to avoid.
- **Read the on-screen text tilemap from WRAM** instead of decoding the ROM — considered; it would
  avoid ROM access and might catch more cases, but it needs a font-tile table and is far more
  fragile than the proven id-plus-table approach alttp-navi already validated. Revisit only if the
  module-`$0E` trigger proves too narrow.
- **An offline `beacon-romdump` cache** — deferred; unnecessary for text, see above.
