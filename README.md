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

| Key | Action | Key | Action |
|---|---|---|---|
| arrows | d-pad | enter | start |
| z x a s | B A Y X | right shift | select |
| q w | L R | | |
| c | scan | e | where am I |
| h | status | v | cycle verbosity |
| r | repeat last | esc | quit |

A gamepad works too, and both are live at once. The left stick doubles as a
d-pad, which some players find easier than the hat.

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
iterate without rebuilding. The reference plugin,
[`plugins/alttp/`](plugins/alttp/), is the worked example; the Lua host API
(`mem.u8/u16/u24/slice`, `say`, `on_command`, `log`, `watch`) is documented in
[ADR 0015](docs/decisions/0015-plugin-runtime.md) and
[ADR 0004](docs/decisions/0004-plugin-model-toml-profile-plus-lua.md).

## Layout

| Path | Purpose |
|---|---|
| `crates/bsnes-sys` | Raw FFI and the C ABI shim over bsnes-jg's C++ API |
| `crates/beacon-emu` | Safe emulator wrapper: frame loop, memory, savestates, input |
| `crates/beacon-output` | Event arbitration and the speech / JSON sinks |
| `crates/beacon-config` | User settings, typed and string-keyed for runtime changes |
| `crates/beacon-plugin` | Plugin runtime: manifest loader, ROM matching, Lua host API |
| `crates/beacon` | The host binary |
| `plugins/alttp` | The A Link to the Past reference plugin (TOML + Lua) |
| `vendor/bsnes-jg` | Emulator core, as a submodule pinned to a release tag |
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
