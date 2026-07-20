# Beacon

A SNES emulator designed with accessibility as a first class feature.

> The name is cooler in the original Klingon: *wovmoHwI'*, "one who causes to be light".

Beacon runs SNES games and instruments them while they run, so that blind and
visually impaired players get spoken and spatial information about what is
happening on screen. Game specific knowledge lives in plugins, so instrumenting
a new game does not mean modifying the emulator.

**Status: phase 0.** The emulator core is embedded and running, and the frame
hook works. There is no UI, no speech, and no plugin runtime yet. It is not
playable.

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
./target/release/beacon "/path/to/your.sfc" 3600
```

The phase 0 harness boots the ROM, taps Start to walk the title screen, and
reports state transitions it detects by diffing work RAM between frames:

```
loaded A Link to the Past (USA).sfc
  region NTSC  work RAM 128 KiB
frame    81  module 0xff -> 0x00  intro
frame   961  module 0x00 -> 0x01  file select
frame  1041  module 0x01 -> 0x04  name file

ran 3600 frames in 18.66s  =  193 fps  (3.2x realtime)
savestate: 295810 bytes
```

## Layout

| Path | Purpose |
|---|---|
| `crates/bsnes-sys` | Raw FFI and the C ABI shim over bsnes-jg's C++ API |
| `crates/beacon-emu` | Safe emulator wrapper: frame loop, memory, savestates, input |
| `crates/beacon` | The host binary |
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
