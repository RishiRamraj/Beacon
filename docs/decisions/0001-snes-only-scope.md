# ADR 0001: SNES-only scope

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

Beacon supersedes the `alttp-navi` proof of concept, which talked to RetroArch over UDP at
30 Hz while the game ran at 60 Hz. The valuable asset in the PoC is domain knowledge about
one console and one game: the SNES memory map, the tile identification chain, the proximity
zone model, and the ROM parser for A Link to the Past.

The Twilight Princess notes describe a much larger game on a much harder platform. They are
the clearest available statement of what blind players actually need, but they describe a
target, not a starting point.

The obvious alternative shape is a multi-console tool built on a portability layer. That
choice would have to be made before anything else, because everything downstream depends on
it.

## Decision

Beacon is a **SNES-only** emulator. A Link to the Past is the proving ground. Support for
other consoles, if ever wanted, is a separate executable built on a different emulator
library.

## Rationale

- Scope is what makes direct embedding possible. With one console there is nothing for a
  core-portability ABI to buy, so libretro can be dropped and its costs with it. See
  [ADR 0002](0002-embed-bsnes-jg-directly.md).
- The valuable reverse-engineering work in the PoC is SNES-specific. Generalising first
  would strand it.
- No SNES per-frame RAM-instrumentation accessibility project appears to exist, and none for
  A Link to the Past. The gap is real and specific.
- The reusable layers are already system-agnostic by construction: the plugin layer, the
  arbiter, and the speech and audio stacks would carry over unchanged to another executable.
- Phase 4's generality proof is a **second SNES title**, not a second console. Validating the
  plugin API against a structurally different game matters more than validating a core
  abstraction, and a second core would drag in symbol collisions for no learning.

## Consequences

- Beacon will never be a "universal accessible emulator". That is accepted.
- The whole emulator decision is locked to what is available for the SNES specifically.
- Users who want another console get another binary, not a core download. There is no core
  to install, no core to keep in sync, and no version-skew class of bug.
- The Twilight Princess requirements are treated as a requirements document rather than a
  roadmap. Reaching that target is a later project that this architecture is meant to make
  approachable, not a commitment here.

## Alternatives considered

- **Multi-console via libretro** — rejected; the portability it buys is worth nothing at this
  scope and its ABI costs real capability ([ADR 0002](0002-embed-bsnes-jg-directly.md)).
- **Target the Twilight Princess platform first** — rejected; much larger game, much harder
  platform, and none of the PoC's domain work transfers.
- **Start generic, specialise later** — rejected; a generic core abstraction would have to be
  designed against exactly one real implementation, which is speculation.
