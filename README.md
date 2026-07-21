# Beacon

A SNES emulator designed with accessibility as a first class feature.

> The name is cooler in the original Klingon: *wovmoHwI'*, "one who causes to be light".

Beacon runs SNES games and instruments them while they run, so that blind and
visually impaired players get spoken and spatial information about what is
happening on screen. Game specific knowledge lives in plugins, so instrumenting
a new game does not mean modifying the emulator.

**Status: playable, early.** The emulator runs games with video, audio, keyboard
and gamepad input, and speaks what it detects through your screen reader. Game
knowledge lives in plugins — a TOML manifest plus a Lua script — selected
automatically by hashing the ROM. A Link to the Past ships built-in; other games
are drop-in (see [Plugins](#plugins)).

## What makes it different

Existing approaches read a running emulator from the outside, over a socket, at
a lower rate than the game actually runs. That structurally cannot see anything
briefer than a sampling interval, and its reads can straddle a frame boundary
and mix state from two different frames.

Beacon owns the frame loop. `run_frame()` advances the emulator by exactly one
video frame, and instrumentation then reads work RAM directly, in process,
between frames. What took fifty round trips becomes a pointer dereference.

## Building

Requires a Rust toolchain, a C++17 compiler, and GNU make. The emulator core is
a git submodule and is built automatically by `cargo build`.

```sh
git clone --recurse-submodules https://github.com/RishiRamraj/Beacon
cd Beacon
cargo build --release
```

If you already cloned without submodules:

```sh
git submodule update --init
```

## Running

Beacon ships no game data. **Bring your own ROM.**

```sh
./target/release/beacon "/path/to/your.sfc"
```

On Linux, speech goes through speech-dispatcher, so you get the voice and rate
you have already configured. If it is not running, Beacon says so and plays on.

### Controls

Game controls are fixed; everything else is an **action** and every action is
rebindable, from the keyboard or a controller.

| Game (fixed) | | Actions (default keys, rebindable) | |
|---|---|---|---|
| arrows | d-pad | c / e / h | scan / where am I / status |
| z x a s | B A Y X | t / g | save state / load state |
| q w | L R | n / b | next / previous save slot |
| enter | start | p / f | pause / frame advance |
| right shift | select | v / r | cycle verbosity / repeat last |
| | | m | show/hide the plugin map |
| | | k | open input configuration |
| | | esc | quit |

A gamepad works too, and both are live at once; the left stick doubles as a
d-pad. Actions are reachable from the pad's spare buttons — by default the left
stick button opens the input configuration, so a controller-only player can
rebind everything without a keyboard.

**Rebinding.** Press the input-configuration key (`k`, or the left stick button).
The game pauses; use up/down to choose an action — each is spoken with its current
binding — then press the key or button to assign it, delete to clear, escape to
finish. Everything is announced, so it works without sight, and changes are saved
immediately.

**Savestates.** Ten slots per game, kept under your config directory and keyed by
ROM so games never collide. Save and load act on the active slot; next/previous
move between slots, announced as you go. Frame advance and pause are there for
debugging a plugin — stepping one frame at a time to watch memory change.

### Other modes

```sh
beacon rom.sfc --headless 3600   # no window, for benchmarking and replay tests
beacon rom.sfc --json --quiet    # line delimited JSON events on stdout
beacon rom.sfc --rate 80         # override speech rate for this run
```

`--json` emits an event per line, so a screen reader, a custom voice pipeline,
or any other tool can subscribe:

```json
{"type":"speak","text":"file select","priority":"Navigation","interrupt":false}
```

**stdout carries only the event stream.** Diagnostics and emulator logs go to
stderr, so piping stdout into a parser is safe.

### Agent control (MCP)

```sh
beacon rom.sfc --mcp        # no window; serve the Model Context Protocol on stdio
```

`--mcp` runs the game with audio and speech but no video window, and speaks the
[Model Context Protocol](https://modelcontextprotocol.io) on stdio. An agent can
then drive the whole thing — press buttons, run commands, save and load, rebind
keys, walk the input configuration, read memory, step frames — and read back
everything Beacon spoke, so it perceives exactly what the player would hear. The
intent is that a player can hand their setup and play to an agent rather than
configure key by key.

The tools are self-describing (`tools/list`); highlights: `get_state`,
`read_memory`, `step`, `set_buttons`, `run_command`, `save_state` / `load_state`,
`list_actions`, `bind` / `unbind`, `set_setting`, `get_map` (the plugin's map as a
PNG the agent can see), the configuration walk (`open_config`, `config_navigate`,
`config_bind`, `config_close`), and the plugin-dev loop `reload_plugin` /
`eval_lua`. See [ADR 0018](docs/decisions/0018-mcp-debug-server.md).

`scripts/mcp_smoke.py <rom>` drives the server end to end — a savestate
round-trip, stepping, commands, settings, and binding — as a quick check that
agent control works against your ROM.

`scripts/capture_map.py <alttp.sfc>` shows the extreme case: an agent boots the
ROM, skips the intro, creates a save file (navigating the name-entry screen),
starts a new game, advances the opening until Link is controllable, and fetches
the plugin's map — all over MCP, with no window. Its output:

![The A Link to the Past plugin's map: a header reading ROOM 260, three health
hearts, and Link shown as a dot facing north with his coordinates.](docs/images/alttp-map.png)

That is the plugin's *interpretation* of memory, not the game's own screen: the
room it read, Link's position and facing, and his health — drawn by the plugin,
legible to an agent.

## Settings

Everything is configurable, and nothing needs configuring to start. Settings
live in `beacon/settings.toml` in your config directory and can also be changed
while playing, because telling someone to edit a file to fix speech they cannot
follow is circular.

```toml
[speech]
rate = 60          # -100 slowest, 100 fastest

[arbiter]
verbosity = 2      # 0 critical only, 3 everything
max_per_frame = 2
```

## Plugins

A plugin is the accessibility knowledge for one game. It is two files in a
directory:

- a **TOML manifest** — the game's name, the ROM SHA-1s it matches, and named
  memory watches;
- a **Lua script** — reads memory each frame and *proposes* what to say. It never
  speaks directly; the host decides what actually survives, so behaviour stays
  consistent across games.

Beacon identifies your ROM by its headerless SHA-1 and loads the matching plugin
with no configuration. The A Link to the Past plugin is compiled in. To add your
own, drop a directory into `plugins/` beside the executable:

```
plugins/
  mygame/
    mygame.toml
    mygame.lua
```

A drop-in that matches the same ROM as a built-in overrides it, so you can
iterate without rebuilding.

**[docs/plugins.md](docs/plugins.md) is the plugin authoring guide** — the full
manifest format and the complete Lua host API (`mem.u8/u16/u24/slice`, `say`,
`on_command`, `log`, `watch`), with semantics and defaults. The reference plugin,
[`plugins/alttp/`](plugins/alttp/), is the worked example to read alongside it.

## Layout

| Path | Purpose |
|---|---|
| `crates/bsnes-sys` | Raw FFI and the C ABI shim over bsnes-jg's C++ API |
| `crates/beacon-emu` | Safe emulator wrapper: frame loop, memory, savestates, input |
| `crates/beacon-output` | Event arbitration and the speech / JSON sinks |
| `crates/beacon-config` | User settings, typed and string-keyed for runtime changes |
| `crates/beacon-plugin` | Plugin runtime: manifest loader, ROM matching, Lua host API |
| `crates/beacon-mcp` | Minimal MCP server over stdio, for agent control |
| `crates/beacon` | The host binary (session core, winit shell, MCP runner) |
| `plugins/alttp` | The A Link to the Past reference plugin (TOML + Lua) |
| `vendor/bsnes-jg` | Emulator core, as a submodule pinned to a release tag |
| `docs/plugins.md` | Plugin authoring guide: manifest format and Lua host API |
| `docs/design.md` | Full design document |
| `docs/decisions/` | Architecture decision records |

## Documentation

Start with [docs/design.md](docs/design.md) for the architecture, then
[docs/decisions/](docs/decisions/README.md) for why each choice was made.
Decisions are recorded as ADRs so that the reasoning, including the rejected
alternatives and the accepted costs, survives longer than anyone's memory of it.

## Licence

GPLv3, shared with bsnes-jg. See [LICENSE](LICENSE) and
[THIRD-PARTY.md](THIRD-PARTY.md).
