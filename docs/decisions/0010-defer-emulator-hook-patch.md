# ADR 0010: Defer the bsnes-jg hook patch until a feature demands it

- **Status:** Accepted (deferred implementation)
- **Date:** 2026-07-20

## Context

Direct embedding ([ADR 0002](0002-embed-bsnes-jg-directly.md)) puts memory watchpoints and
CPU execution hooks within reach, which libretro's ABI bricks shut. Those hooks change what
the tool can know:

- **Write-watchpoints replace frame-diffing.** Instead of comparing ~50 addresses between
  frames and inferring "health decreased", a write to `$7EF36D` calls you the instant it
  happens, with the program counter that did it. Exact rather than inferred, and cheaper.
- **Execution hooks answer *why*, not just *what*.** Hooking the damage-handling routine or
  the dialog-open routine is categorically more reliable than guessing from sprite state, and
  it is the credible path to the incoming-attack cue, the hardest requirement in the Twilight
  Princess document.

But bsnes-jg has no debugger. near's bsnes never had one either; the debugging fork is
bsnes-plus, a separate lineage. So this is a patch we write and then own.

## Decision

Run **stock bsnes-jg through Phases 0–2**, with per-frame WRAM reads only. Write the hook
patch when a specific feature demands it, which in practice means the incoming-attack cue in
Phase 3. If per-frame polling turns out to be sufficient for everything, the patch never gets
written, and that is a good outcome.

The patch itself is designed and costed, just not scheduled.

## Rationale

- Reading WRAM in-process every frame already fixes **every** sampling failure in the PoC.
  The PoC's problems came from UDP and 30 Hz sampling, not from a lack of watchpoints.
- `Bsnes::getMemoryRaw(MainRAM)` returns a pointer to the emulator's 128 KiB of SNES WRAM,
  borrowed zero-copy. What took fifty UDP round trips becomes a pointer dereference.
- **Don't become a fork maintainer before something is actually being bought.**
- The patch is small and the insertion points are known. bsnes-jg's bus chokepoint is
  unusually clean: `src/memory.hpp` has `Bus::read` and `Bus::write` as two inline one-liners,
  each with a single definition, with **all 44 call sites** routed through them. The execution
  hook point is `WDC65816::instruction()`. Three insertion points plus a callback registry, on
  the order of **30 lines**.
- **bsnes-plus is this exercise already completed.** Its `breakpoint_test()` call sites form a
  working map of every place a SNES core needs a hook, including the SMP, PPU, SA-1, and
  SuperFX points needed if instrumentation ever goes beyond the CPU bus. Used as a reference
  implementation without adopting the codebase.
- The patch is additive and localised, so rebasing onto new bsnes-jg releases should stay
  cheap.
- Deferring keeps the answer empirical. Phases 0–2 measure whether per-frame polling suffices
  rather than guessing.

## Consequences

- **Deferred, explicitly:** watchpoints and execution hooks are not in the critical path for
  Phase 0. The hook patch is deliberately not scheduled at all.
- **Live, explicitly open question:** whether the hook patch is ever needed. Phases 0–2 answer
  it empirically.
- **Accepted risk:** if written, we own a fork. About 30 lines at clean chokepoints, rebased
  per upstream release. Real cost, accepted.
- If the patch is written, GPLv3 obliges shipping or offering it as part of the complete
  corresponding source. See [ADR 0009](0009-gplv3.md).
- Phase 3's incoming-attack cue carries schedule risk, since it may be the trigger that forces
  the patch to be written mid-phase.
- `getMemoryRaw` does not expose OAM, but ALttP's sprite table lives in WRAM around `$7E0D00`,
  not OAM, and CGRAM and ARAM are irrelevant to accessibility. If OAM is ever needed, exposing
  it is a one-line addition to the same patch.
- Because Beacon owns the loop and bsnes-jg exposes `serialize`/`unserialize`, sessions stay
  reproducible without any patch: savestate plus input log replays identically, which is what
  makes golden-file regression tests possible from Phase 2.

## Alternatives considered

- **Write the hook patch in Phase 0** — rejected; takes on fork maintenance before any feature
  needs it, and delays the thing that actually matters (the host skeleton running).
- **Adopt bsnes-plus for its existing debugger** — rejected; 2010-era accuracy, GPLv2, one
  maintainer, no commits since March 2025. See [ADR 0002](0002-embed-bsnes-jg-directly.md).
- **Adopt MesenCE for its exported debug API** — rejected on packaging grounds, same ADR.
- **Rule watchpoints out permanently** — rejected; execution hooks are the credible path to
  the incoming-attack cue, and the architecture deliberately leaves the door open.
