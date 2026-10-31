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

Navigation is modelled on the [Toby Accessibility Mod](https://www.moddb.com/mods/toby-doom-accessibility-mod)
for Doom: the same menu of points of interest, the same sound to follow along a route, the
same world sounds — Toby's own recordings, if you have Toby — and the same keys. See
[Navigation](#navigation).

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

**Windows**, cross-compiled from Linux with MinGW-w64 (`g++-mingw-w64`, the win32 thread
model):

```sh
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

`beacon.exe` needs `libstdc++-6.dll` and `libgcc_s_seh-1.dll` from the same MinGW
(`/usr/lib/gcc/x86_64-w64-mingw32/*-win32/`) beside it, plus the speech DLLs below.

## Running

Beacon ships no game data. **Bring your own ROM.**

```sh
./target/release/beacon "/path/to/your.sfc"
```

On Linux, speech goes through speech-dispatcher, so you get the voice and rate
you have already configured. If it is not running, Beacon says so and plays on.

On Windows, speech goes through your screen reader — NVDA, JAWS, or any other Tolk
supports — falling back to SAPI. Keep `Tolk.dll`, `nvdaControllerClient64.dll` and
`SAAPI64.dll` beside `beacon.exe`; the Windows download already has them.

### Controls

Written to read well with a screen reader: no tables, every key named as a word. The same
text ships with the Windows build as `CONTROLS.txt`.

Beacon plays the game with the keyboard laid out like a Super Nintendo controller. All the
other keys are Beacon's own, and they follow the
[Toby Accessibility Mod](https://www.moddb.com/mods/toby-doom-accessibility-mod) for Doom
wherever Toby has the same thing. Every key except the game controls can be changed with F12.

#### Playing the game

- Arrow keys: move.
- Letter X: the A button. Talk, lift, open, read.
- Letter Z: the B button. Swing the sword.
- Letter S: the X button. The world map.
- Letter A: the Y button. Use the item in hand.
- Letter Q: the L button.
- Letter W: the R button.
- Enter: Start. The item menu.
- Right Shift: Select.

The letters Z, X, A and S sit in the same shape as the controller's buttons: Z and X along
the bottom row, A and S above them.

#### Finding your way

- M: the navigation menu. It lists where you can go: your quest objective, your markers, and
  places you have found, such as unexplored areas, unopened chests, keys, pickups, exits and
  stairs, and what is in this room. Each list starts with Stop navigation. Pick a place and a
  beeping sound leads you there. Walk toward the beep. It changes when you are facing it.
- L: turn the quest guide on or off. It leads you to the next step of the story.
- Y: stop guiding.
- V: scan. Says what is around you, how far, and in which direction.
- G: turn to face the nearest enemy.
- Apostrophe: turn the wall sounds on or off. When on, a wind sound tells you how far the
  walls are in each direction.
- Backtick, the key left of 1: show or hide the map.

#### Which way you are facing

- C: say which way you are facing.
- F: face north.
- E: turn left.
- R: turn right.
- T: turn around.

#### Checking your status

- H: health.
- B: rupees, bombs, arrows and magic.
- N: sword, shield and armour.
- K: keys.
- I: the item in your hand.
- U: exact coordinates.
- Delete: where am I.
- Semicolon: the current quest objective.
- D: read the text box on screen again.
- Slash: repeat the last thing said.

#### Saving and other keys

- F6: save.
- F9: load.
- F7 and F8: previous and next save slot.
- Escape: Beacon's menu, for opening games, settings and quitting.
- Equals: change how much Beacon says.
- J: mute.
- F4: pause.
- F12: change key bindings.
- F11: quit.
- F10: advance one frame, for debugging a plugin.

#### In any menu

- Up and down arrows: move through the list.
- X, Enter or right arrow: choose.
- Z, Backspace or left arrow: go back.
- Escape: close.

#### Using a USB controller

Any USB controller should work alongside the keyboard, including Super Nintendo style
controllers. The buttons work by position, the same as on a Super Nintendo: the right button
is A and the bottom button is B. The left stick also moves.

To use Beacon's own commands, hold Select and press another button. Tapping Select on its own
still sends Select to the game.

- Select and Start: Beacon's menu.
- Select and A: the navigation menu.
- Select and B: stop guiding.
- Select and X: the quest guide.
- Select and Y: scan.
- Select and L: read the text box.
- Select and R: face the nearest enemy.
- Select and up: say which way you are facing.
- Select and down: health.
- Select and left: repeat the last thing said.
- Select and right: where am I.

In menus on the controller, up and down move, A chooses, B goes back, and Start closes.

A controller with more buttons also has its stick buttons, L2, R2 and guide button for
Beacon's commands. A controller Beacon does not recognise, or whose buttons come out wrong,
can be given an SDL controller mapping in the `input.pad_mappings` setting, or in the
`SDL_GAMECONTROLLERCONFIG` environment variable.

**Rebinding.** Press the input-configuration key (`f12`, or the left stick button).
Press a Select chord there to bind it, like any key.
The game pauses; use up/down to choose an action — each is spoken with its current
binding — then press the key or button to assign it, delete to clear, escape to
finish. Everything is announced, so it works without sight, and changes are saved
immediately.

**Savestates.** Ten slots per game, kept under your config directory and keyed by
ROM so games never collide. Save and load act on the active slot; next/previous
move between slots, announced as you go. Frame advance and pause are there for
debugging a plugin — stepping one frame at a time to watch memory change.

### Navigation

Two ways to play, side by side:

- **The quest guide** (`l`, or *Quest objective* in the menu) leads you through the story:
  authored waypoints and goals, beat by beat, from the house to the castle and on.
- **Exploration, Toby's way** (`m`). One menu holds every destination, as Toby's does:
  the quest objective, ten markers of your own, and lists of what you have found —
  *Unexplored*, *Unopened*, *Keys*, *Pickups*, *Exits and stairs*, *This room*. Every list
  holds only places you can actually reach, nearest first by walking distance, and reads each
  the way Toby does: "Chest - South-east - 13 tiles". Beacon remembers every room and screen
  you have been on, so a chest passed two rooms back is still listed, and the rooms and
  screens next to ones you have visited are *Unexplored*. The menu is drawn over the map.

Choose a destination and Toby's **pathfinder** leads you there by ear: one sound rides the
route a few steps ahead of you and beeps from where it is; walk toward it and it pulls you
round each corner. Facing it, the beep changes. Corners and arrival have their own sounds.

The **world makes Toby's sounds**, from where things are: pickups and chests, your markers,
exits and stairs, doors and locked doors, spike floors, a ticking edge ahead of a pit, a bump
where you walk into a wall, and — off by default, on with `'` — Toby's "wind" in each
direction, pitched by how far the wall is. **Enemies** each carry a tone; a lock tick says
one is straight ahead of you; hits and kills have their own sounds; `g` turns you to face the
nearest, and `v` counts everything in view by name, distance and direction.

**Toby's sounds.** Beacon ships none of Toby's recordings — they belong to its authors — but
if you have the [Toby Accessibility Mod](https://www.moddb.com/mods/toby-doom-accessibility-mod),
point `sounds.dir` at its folder (the unpacked mod, or its git checkout) and Beacon plays
them. Without it, every cue has a synthesized stand-in in the same spirit.

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

Set `BEACON_STATS=1` to print frame rate, audio underruns and loop timing every two seconds,
and to log any frame whose plugin work runs long — the first thing to look at if a game feels
slow.

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

To keep **playing in the window** while an agent assists, use `--control` instead:
Beacon serves the same protocol on a local Unix socket (under `$XDG_RUNTIME_DIR`),
so an agent can attach to your live session — pause it, read state, adjust
settings — without taking over the window.

```sh
beacon rom.sfc --control        # window runs; agent attaches over the socket
```

Several clients can attach at once, so an agent assisting you and a tool of your
own (`scripts/waypoints.py`, below) can both be connected while you play.

The tools are self-describing (`tools/list`); highlights: `get_state`,
`read_memory`, `step`, `set_buttons`, `run_command`, `save_state` / `load_state`,
`list_actions`, `bind` / `unbind`, `set_setting`, `get_map` (the plugin's map as a
PNG the agent can see), the configuration walk (`open_config`, `config_navigate`,
`config_bind`, `config_close`), and the plugin-dev loop `reload_plugin` /
`eval_lua`. See [ADR 0018](docs/decisions/0018-mcp-debug-server.md).

`scripts/mcp_smoke.py <rom>` drives the server end to end — a savestate
round-trip, stepping, commands, settings, and binding — as a quick check that
agent control works against your ROM.

`scripts/waypoints.py` is the socket put to work: a line shell for editing the
ALttP plugin's authored guide routes against the game as you play it. Attach it to
a `--control` session, walk Link to a spot, and `move 14` puts waypoint 14 exactly
where you are standing — no reading coordinates out of a debugger. It edits
`plugins/alttp/waypoints.lua` in place and can `reload` the plugin to try the
change immediately.

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

[beacons]
enabled = true     # spatial-audio tones for where things are
volume_max = 0.5   # 0 to 1, loudness when a source is closest
volume_min = 0.05  # 0 to 1, loudness at the far edge of range
music_duck = 0.5   # 0 to 1, how far game audio ducks under a beacon

[audio]
buffer_ms = 50     # 20 to 500, delay before game sound and tones are heard; raise if it crackles

[sounds]
dir = ""           # your Toby Accessibility Mod folder, for its sounds; empty = Beacon's own
volume = 0.8       # 0 to 1, loudness of the recorded cues

[input]
pad_mappings = ""  # SDL controller mapping lines, for a pad Beacon does not recognise
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
manifest format and the complete Lua host API (`mem`, `rom`, `watch`, `say`,
`on_command`, `log`, `beacon` for spatial-audio tones, `sound` for recorded cues, `menu`
for lists of the plugin's own in Beacon's menu, `nav` for grid pathfinding in the host,
`input` for acting briefly on the player's behalf, and `on_draw` for the map), with
semantics and defaults. The reference plugin,
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
