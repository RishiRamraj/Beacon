# Writing a Beacon plugin

A plugin is the accessibility knowledge for one game: which memory holds health
and position, what is worth announcing, what a command should answer. This is the
reference for authoring one. For *why* the plugin model is shaped this way, see
[ADR 0004](decisions/0004-plugin-model-toml-profile-plus-lua.md) and
[ADR 0015](decisions/0015-plugin-runtime.md); this document is the how.

The complete worked example is [`plugins/alttp/`](../plugins/alttp/) — read it
alongside this page.

---

## The two files

A plugin is a directory containing two files:

```
plugins/
  mygame/
    mygame.toml     # the manifest: identity, ROM match, memory watches
    mygame.lua      # the script: reads memory each frame, proposes what to say
```

The directory name is yours to choose. The manifest may be named anything ending
in `.toml`; it names the script.

Beacon loads plugins from a `plugins/` directory beside the executable (and, in a
development checkout, from `plugins/` in the working directory). The A Link to the
Past plugin is also compiled into the binary. **A drop-in plugin that matches the
same ROM as a built-in overrides it**, so you can iterate on a plugin without
rebuilding Beacon.

---

## The manifest

```toml
# Top-level keys must come before the first [table] header — a TOML rule.
script = "mygame.lua"

[game]
name   = "My Game"
region = "NTSC-U"          # informational, optional
# Headerless SHA-1s this plugin instruments. See "ROM identification" below.
sha1   = ["0123456789abcdef0123456789abcdef01234567"]

# Named memory watches, optional. Exposed to Lua as the `watch` table.
[watch]
health = { addr = 0x7EF36D, size = 1 }
room   = { addr = 0x7E00A0, size = 2 }

# Custom commands, optional. Up to ten. The user binds keys to these.
[[command]]
id    = "coordinates"
label = "Report exact coordinates"
```

| Key | Required | Meaning |
|---|---|---|
| `script` | yes | Lua filename, relative to the manifest. |
| `game.name` | yes | Shown when the plugin loads. |
| `game.sha1` | to match a ROM | List of headerless ROM SHA-1s this plugin claims. |
| `game.region` | no | Informational only, for now. |
| `watch` | no | Named addresses, each `{ addr, size }`. `size` defaults to 1. |
| `command` | no | Up to ten `{ id, label }` custom commands; see [Custom commands](#custom-commands). |

Unknown keys are **rejected**, not ignored, so a typo is caught at load rather
than silently doing nothing.

### ROM identification

Beacon identifies a ROM by the SHA-1 of its *headerless* image — after stripping a
512-byte copier header if present. The user never picks a plugin; the matching one
is selected automatically, and a ROM with no match still plays, just without
instrumentation.

To find a ROM's hash:

```sh
# If the file size is a multiple of 1024, it is already headerless:
sha1sum yourgame.sfc
# If size % 1024 == 512, strip the header first:
tail -c +513 yourgame.sfc | sha1sum
```

A game with several revisions or regional releases lists several hashes. **No
copyrighted data ever ships — only these hashes.**

---

## The script

The script runs once when the plugin loads. During that run it defines a global
`on_frame` and registers any commands. Everything it needs between frames it keeps
in its own Lua state, which persists for the life of the plugin.

```lua
-- Runs once at load. Set up anything you need here.
local prev = nil

-- Called once per video frame, in order, with the frame number.
function on_frame(frame)
  local health = mem.u8(watch.health.addr)
  if prev ~= nil and health < prev then
    say("Hit.", { priority = "critical", category = "combat" })
  end
  prev = health
end

-- Optional: answer a user command.
on_command("status", function()
  say(string.format("%d health.", mem.u8(watch.health.addr)),
      { priority = "navigation" })
end)
```

`on_frame` is optional — a purely command-driven plugin is valid. If a call raises,
Beacon logs it with the script name and the game keeps running; one bad frame never
takes the emulator down.

---

## Host API reference

These globals are installed before your script runs. This is the **entire** API
available today. (The design anticipates more — spatial-audio beacons, rumble,
menus, savestate hooks — but those are not yet implemented; see
[ADR 0015](decisions/0015-plugin-runtime.md).)

### `mem` — reading the current frame

Bounds-checked reads of the frame's work RAM. Addresses are **SNES addresses**, so
they read exactly as they appear in a memory map or a disassembly.

| Call | Returns |
|---|---|
| `mem.u8(addr)` | Byte at `addr`, or `nil` if unmapped. |
| `mem.u16(addr)` | Little-endian 16-bit value, or `nil`. |
| `mem.u24(addr)` | Little-endian 24-bit value, or `nil`. |
| `mem.slice(addr, len)` | A Lua string of `len` bytes, or `""` if out of range. |

Mapped addresses are work RAM: banks `$7E` and `$7F` (the full 128 KiB), and the
low-RAM mirror visible at `$0000`–`$1FFF` in banks `$00`–`$3F` and `$80`–`$BF`.
Anything else — ROM, hardware registers, unmapped space — reads as `nil` rather
than a wrong value. Index a `slice` result with Lua's `string.byte(s, i)` (1-based).

Your script sees **only the frame it was handed**. The memory is staged per call,
so you cannot keep a reference and read it later, when it would be stale.

### `rom` — reading static game data

Read-only access to the headerless ROM, for decoding static data (dialogue
tables, lookup tables) at load. Offsets are raw file offsets into the headerless
image; a plugin maps SNES addresses to offsets itself.

| Call | Returns |
|---|---|
| `rom.u8(offset)` | Byte at `offset`, or `nil` if out of range. |
| `rom.slice(offset, len)` | A Lua string of `len` bytes (empty if out of range). |
| `rom.size` | The ROM length in bytes. |

Decode once at load — reading a large `rom.slice` and indexing it with
`string.byte` is far faster than a `rom.u8` call per byte. The alttp plugin uses
this to decode the game's dialogue table; see [`plugins/alttp/alttp.lua`](../plugins/alttp/alttp.lua)
and [ADR 0020](decisions/0020-rom-access-and-game-text.md).

### `watch` — named addresses from the manifest

The manifest's `[watch]` table, as a Lua table: `watch.<name>.addr` and
`watch.<name>.size`. Keeping addresses in the manifest puts every one a reviewer
needs in one legible place; reading them by name keeps magic numbers out of the
script.

### `say(text, opts)` — propose an utterance

`say` **proposes**; it never speaks. Everything proposed in a frame goes to the host
arbiter, which decides what is actually spoken based on priority, rate limits,
verbosity, and de-duplication. This is deliberate: arbitration is a host service so
that behaviour is consistent across every game, and no plugin reimplements it badly.
Being generous with `say` is therefore safe.

`text` is a string. `opts` is an optional table:

| Option | Type | Default | Meaning |
|---|---|---|---|
| `priority` | string | `"ambient"` | `"critical"`, `"navigation"` (or `"nav"`), `"interaction"`, `"ambient"`. |
| `category` | string | the priority name | Rate-limit bucket. Chatty categories can't crowd out quiet ones. |
| `collapse_key` | string | — | Intents sharing a key in one frame collapse to the single nearest. |
| `distance` | number | +∞ | Picks the winner when collapsing; smaller wins. |
| `rate_limit` | string | — | Suppress identical text for this long: `"400ms"`, `"1s"`, `"2.5s"`, or a bare number of milliseconds. |

The **priority classes**, highest first:

- **critical** — incoming attack, death, low health. Barges in, cancelling current
  speech, and bypasses rate limiting.
- **navigation** — arrival, zone entry, blocked by an obstacle.
- **interaction** — facing a chest, an NPC in range.
- **ambient** — scenery, scan flavour. First to be dropped when it gets busy.

Verbosity gates by class: at the lowest setting only critical is spoken, at the
highest everything is. So an unmarked `say` (which defaults to ambient) is easy for
a player to silence — a safe default.

`collapse_key` is the answer to "zero in on the closest trigger": emit one intent
per nearby object with a shared key and a `distance`, and the player hears about the
nearest, not all twelve.

### `on_command(name, fn)` — answer a user command

Registers a handler for a named command. When the player issues it, `fn` is called
against the current frame; whatever it `say`s is spoken **immediately**, bypassing
rate limiting, because it answers a direct request.

The three **standard** commands every game shares, and their default keys (all
rebindable):

| Command | Default key | Convention |
|---|---|---|
| `"where"` | `e` | Where am I — location and facing. |
| `"status"` | `h` | Health and resources. |
| `"scan"` | `c` | Describe surroundings. |

A command with no handler falls back to a spoken acknowledgement, so a bound key is
never silent. You may register handlers for only the commands your game supports.

### Custom commands

Beyond the standard three, a plugin may declare up to ten **custom** commands —
game-specific actions like "read the current sign" or "list inventory". Declare each
in the manifest with an `id` and a human `label`, and implement it with
`on_command(id, fn)`:

```toml
[[command]]
id    = "coordinates"
label = "Report exact coordinates"
key   = "KeyD"                # optional: a default key, filled if still unbound
```

The optional `key` is a *suggestion*: the host binds it only if that key and the
command are both still unbound and the key is not a game control. A user's own
binding always wins, and it is never persisted over their choices. Use the stable
key names (`"KeyD"`, `"F5"`, …).

```lua
on_command("coordinates", function()
  say(string.format("X %d, Y %d.", mem.u16(watch.link_x.addr), mem.u16(watch.link_y.addr)),
      { priority = "navigation" })
end)
```

The user binds a key or gamepad button to it in Beacon's input configuration, where
it is listed by the `label` you gave. The `id`s `scan`, `where`, and `status` are
reserved. See [ADR 0016](decisions/0016-dynamic-keybinding-and-actions.md) for how
binding works.

### `log(level, message)` — diagnostics

`log("info", "loaded 42 rooms")` or `log("something happened")`. Routed to stderr,
never to the stdout JSON stream, so it is safe to log freely.

### `on_draw(canvas)` — map mode (optional)

Define `on_draw` and the player can toggle a **map view** (default key `m`) showing
your visual interpretation of memory — Link's position, the current room, health,
whatever helps. It is not the game's own map; it is a picture of what your plugin
believes the state to be, for debugging and for sighted assistance. Through the MCP
server's `get_map` tool the same picture reaches an agent as a PNG.

`canvas` is a fixed 256x256 surface. Colours are 24-bit `0xRRGGBB`. Coordinates are
integers, so use floor division (`//`) when scaling.

| Call | Draws |
|---|---|
| `canvas:clear(rgb)` | Fill the whole canvas. |
| `canvas:pixel(x, y, rgb)` | One pixel. |
| `canvas:rect(x, y, w, h, rgb)` | A filled rectangle. |
| `canvas:line(x0, y0, x1, y1, rgb)` | A line. |
| `canvas:text(x, y, string, rgb)` | Text in the built-in 5x7 font. |
| `canvas.width`, `canvas.height` | The canvas size. |

The font currently covers digits, uppercase letters, space, and basic punctuation —
enough for coordinates and room numbers. `on_draw` runs only while the map is shown,
so it costs nothing when hidden. See [`plugins/alttp/alttp.lua`](../plugins/alttp/alttp.lua)
for a worked example, and [ADR 0017](decisions/0017-plugin-debug-drawing.md).

The alttp plugin's `on_draw` produces this — the room, Link's position and facing,
and his health, drawn from memory:

![The alttp map: ROOM 260, three health hearts, and Link as a dot facing north
with his coordinates.](images/alttp-map.png)

## Debugging with an agent

`beacon yourgame.sfc --mcp` runs headless and serves the Model Context Protocol on
stdio, so an agent can help you develop a plugin: step frames, read memory by
address, run your commands, and read back everything spoken — all reproducibly.

The tight edit-run loop is there too. Drop your plugin in a `plugins/` directory
(not the built-in), and:

- **`reload_plugin`** re-reads it from disk and rebuilds it, so you edit the Lua or
  manifest, reload, and see the effect — no restart, and the game keeps its position.
- **`eval_lua`** runs a snippet in your plugin's environment against the current
  frame, e.g. `return mem.u8(watch.health.addr)`, for probing while you work.

See [ADR 0018](decisions/0018-mcp-debug-server.md) for the full tool set.

---

## Things to know

- **State is not yet saved with savestates.** Your Lua state (e.g. a latched
  low-health warning) persists frame to frame, but loading a savestate does not
  restore it; it re-derives on the next frame from memory. Keep per-frame-derivable
  state and this is invisible. See [ADR 0015](decisions/0015-plugin-runtime.md).
- **There is no frame-time watchdog yet.** A runaway loop in `on_frame` will hang
  the emulator. Keep per-frame work bounded.
- **Determinism.** `on_frame` receives the frame number, not a clock, and the host
  arbitrates on session time derived from that number. Keep your logic a pure
  function of memory and frame count and a recorded session will replay identically.

---

## Testing a plugin

Run headless against your ROM and watch what it proposes on stderr:

```sh
beacon yourgame.sfc --headless 3600
```

Add `--json` to see the arbitrated event stream on stdout (one JSON object per
line), which is exactly what an external voice pipeline would consume:

```sh
beacon yourgame.sfc --headless 3600 --json --quiet 2>/dev/null
```
