# ADR 0002: Embed bsnes-jg directly as a C++ library

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

The proof of concept was a spectator: it read emulator memory over UDP at 30 Hz against a
60 Hz game, producing missed events, torn state that straddled frame boundaries, added
round-trip latency, and no determinism and therefore no possible regression suite.

Fixing that requires owning the frame loop and reading memory in-process. Given the SNES-only
scope ([ADR 0001](0001-snes-only-scope.md)), the emulator can be a linked library rather than
a separate process behind an ABI. The candidates were libretro cores generally, zsnes,
snes9x, bsnes-plus, MesenCE, and bsnes-jg.

## Decision

Embed **bsnes-jg** (`gitlab.com/jgemu/bsnes`) directly as a statically linked C++ library.
Not via libretro. Not bsnes-plus, not MesenCE, not zsnes, not snes9x.

## Rationale

Verified by building it, not inferred from documentation:

- `make ENABLE_STATIC=1 DISABLE_MODULE=1 USE_VENDORED_SAMPLERATE=1` produces
  **`objs/libbsnes.a`, ~4.9 MB**, and builds cleanly. `DISABLE_MODULE=1` builds without the
  Jolly Good headers present at all, confirming the core is genuinely independent of that
  frontend API.
- bsnes-jg is the live bsnes line: **v2.1.1 released July 2026**, cycle-accurate, GPLv3, and
  built as a library.
- It is the only option that is simultaneously actively maintained, modern in accuracy,
  SNES-focused, and already a real library.

**Why not libretro.** Its ABI cannot express memory watchpoints or CPU execution hooks at
all, and never will. Of its **91 environment calls, zero are debug-oriented**. The one
fine-grained facility, `SET_MEMORY_MAPS`, hands the frontend a static array of address
descriptors: push-only, one-shot, no callback field in either struct. RetroAchievements is
the proof rather than the counterexample; its `Delta`/`Prior` operators are a per-frame
snapshot diff with no program-counter or instruction operand anywhere in the condition
language. Separately, bsnes-jg's own libretro core does not implement `SET_MEMORY_MAPS` at
all, exposing only four coarse region IDs, so even the theoretical ceiling was unavailable.

**Why not zsnes.** Last release 2007, hand-written x86 assembly, unmaintained, inaccurate,
not embeddable.

**Why not snes9x.** Its licence is genuinely non-commercial-only, not OSI-approved and not
GPL-compatible. It stays permanently out of the tree, not even as an optional link.

**Why not bsnes-plus.** It builds Qt-free as `libsnes.a` with a full debugger already in the
core (read/write/exec breakpoints across CPUBus, APURAM, VRAM, OAM, CGRAM, SA-1 and SuperFX,
mirror-aware, with value predicates), an existing `extern "C"` API, and a performance profile.
But it is based on **bsnes v073**, 2010-era accuracy, is GPLv2, has one maintainer, and has
had **no commits since March 2025**. Adopting it trades a 30-line patch for permanent
ownership of a much larger, older codebase.

**Why not MesenCE.** It was built and measured, not assumed. `make core` succeeded in
**2m50s, exit 0, zero errors**, on a machine with **no `dotnet` installed**, so the headless
claim is true. Its debug API is genuinely exported as plain C (`SetBreakpoints`,
`GetDebugEvents`, `GetDebugEventCount`, `GetMemoryState`, `SetMemoryValue`) with
`SnesWorkRam` / `SnesVideoRam` / `SnesSpriteRam` / `SnesCgRam` confirmed memory types, which
is more than bsnes-jg exposes. Its licence is GPL-3.0, identical, so no differentiator. The
packaging numbers decide it:

| | bsnes-jg | MesenCE |
|---|---|---|
| Artifact | `libbsnes.a`, **4.9 MB static** | `MesenCore.so`, **14.2 MB shared** |
| Shared-library dependencies | ~none | **50** |
| Exported symbols | 37, namespaced | **7,940** (688 C, 7,252 mangled C++) |

Those 50 dependencies are the entire desktop stack: SDL2, the full X11 set, Wayland, DRM,
GBM, ALSA, PulseAudio, libsamplerate. That is the opposite direction from a single-file
install. **7,252 mangled C++ symbols** in the dynamic symbol table means no visibility
control at all, so the unstable-internal-boundary concern is visible in the binary. And the
settling argument: MesenCE's entire appeal was *no fork needed*, but `core` and `ui` are the
same makefile target, so SDL/X11/Wayland are compiled **into** the core. A lean static build
means patching the makefile to drop `SDLOBJ`/`LINUXOBJ` and repairing the fallout, which is a
fork of a larger multi-system codebase. Once both options require a fork, 30 lines on a
dependency-free 4.9 MB static library is plainly cheaper.

## Consequences

- The licence is forced to GPLv3 by static linking. See [ADR 0009](0009-gplv3.md).
- There is no `extern "C"` surface, so a hand-written shim is mandatory. See
  [ADR 0003](0003-rust-host-and-cpp-shim.md).
- bsnes-jg has no debugger. near's bsnes never had one; the debugging lineage is bsnes-plus.
  Watchpoints are a patch we write, not a feature we inherit. See
  [ADR 0010](0010-defer-emulator-hook-patch.md).
- `getMemoryRaw` exposes MainRAM (128 KiB WRAM) and VideoRAM plus cartridge RAM and RTC. It
  does **not** expose ARAM, OAM, or CGRAM. Acceptable here: ALttP's sprite table lives in
  WRAM around `$7E0D00`, not OAM, and CGRAM and ARAM are irrelevant to accessibility.
  Exposing OAM would be a one-line addition to the hook patch.
- **Accepted risk:** bsnes-jg is CPU-heavy with no fast mode. Speed hacks were removed
  outright, the only knobs are `setCoprocDelayedSync` and `setCoprocPreferHLE`, and upstream's
  advice for "too slow" is to use a different emulator. There is no low-end fallback inside
  this codebase. Worth measuring in Phase 0 to know the floor, but informational rather than
  go/no-go.
- **Accepted risk:** bus factor of one. bsnes-jg's emulation work is effectively one person.
  GPLv3 means the code cannot be taken away and our patch is small enough to rebase onto a
  successor.
- Clone from GitLab. The GitHub mirror's default branch is `libretro`, which is the wrong
  tree. The wanted artifact is `libbsnes.a`; `libbsnes-jg.a` is the Jolly Good frontend
  module.
- MesenCE is retained as a documented, verified fallback rather than a live option. Both
  triggers for switching (performance, an unpleasant C++ shim) have been accepted as risks,
  so this decision is settled rather than provisional.

## Alternatives considered

- **libretro** — rejected; ABI structurally cannot express watchpoints or exec hooks, and
  portability is worthless at SNES-only scope.
- **zsnes** — rejected; 2007, x86 assembly, unmaintained, inaccurate, not embeddable.
- **snes9x** — rejected; non-commercial licence, GPL-incompatible.
- **bsnes-plus** — rejected; working debugger but 2010-era accuracy, GPLv2, one maintainer,
  no commits since March 2025.
- **MesenCE** — rejected on packaging: 14.2 MB shared object, 50 shared-library dependencies,
  7,940 exported symbols, and a fork required anyway to get a lean static build.
