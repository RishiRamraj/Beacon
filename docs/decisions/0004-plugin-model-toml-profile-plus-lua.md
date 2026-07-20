# ADR 0004: Plugin model is a declarative TOML profile plus Lua `on_frame` hooks

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

Per-game accessibility knowledge is what Beacon actually delivers: which addresses hold
health and room ID, what counts as an event, where objects are relative to the player. That
knowledge has two very different shapes. Most of it is mechanical (address, size, "when this
decreases, say this"), and a minority of it is genuine logic (proximity rings, cone scanning
with line-of-sight occlusion, object tracking, pathfinding).

The PoC encoded both in Python: `constants.py` for the memory map and lookup tables,
`events.py` and `proximity.py` for the logic. It is a working specification, not throwaway
code.

The delivery goal is one file plus a `plugins/` directory, with no runtime to install
([ADR 0002](0002-embed-bsnes-jg-directly.md) removed the core-install step; the plugin layer
must not put one back).

## Decision

Three tiers:

- **Tier 0 — ROM identification.** Every profile declares the SHA-1s it supports. On load
  Beacon hashes the ROM headerless, after stripping any 512-byte SMC header, and selects the
  matching profile automatically. The user never picks anything.
- **Tier 1 — declarative TOML profile.** Memory watches and simple event rules, each event
  carrying `when`, `say`, `priority`, and optional `rate_limit`.
- **Tier 2 — Lua.** Embedded via `mlua` with the `vendored` feature, using **Lua 5.4**.

The Lua host API exposes bounds-checked memory views (`mem.u8/u16/u24/slice`), static ROM
reads (`rom.u8/slice`), precomputed ROM tables (`cache.get`), `say(...)` with metadata,
`beacon.set`/`beacon.clear`, `rumble`, `on_command`, `menu.open`, `state.save`/`state.load`,
and `log`.

Crucially, `say` **proposes** an utterance with metadata (priority, category, `collapse_key`,
`distance`, `rate_limit`). The plugin never speaks.

## Rationale

- The profile covers the mechanical 80%, so simple games need no code at all.
- Lua covers the logic. It embeds statically and is the twenty-year standard for emulator
  scripting; `pokemon-access` and `pokecrystal-access` validate exactly this pattern (Lua
  reading game RAM per frame inside an emulator, speaking through NVDA).
- `mlua` with `vendored` compiles Lua from source into the binary: no runtime dependency, no
  DLL, consistent with the single-file promise.
- **Lua 5.4 rather than 5.5.** 5.5 shipped in December 2025; 5.4 has the ecosystem and `mlua`
  supports both, so there is no upside to being early.
- SHA-1 matching means no copyrighted data ships and the user does no configuration.
- `constants.py` translates to TOML near-mechanically, and the roughly 1,300 lines of ROM
  parsing in `rom/` move **offline** into a `beacon-romdump` tool that writes a binary cache
  beside the profile. That keeps a large one-time-cost body of code out of the hot path and
  out of the Lua port entirely, the single biggest reduction in porting work available.
- Propose-don't-speak is what makes behaviour consistent across games: arbitration is a host
  service, so no plugin re-implements it badly. See
  [ADR 0005](0005-event-arbitration-in-host.md).

## Consequences

- Two authoring surfaces to document, version, and keep coherent. A profile author has to
  know when a rule outgrows TOML.
- The host API is a compatibility surface. Once plugins exist, changing it breaks them.
- Plugin state must be serialised alongside core state so savestates and rewind do not
  desynchronise zone latches and tracked objects.
- The plugin gets a **hard budget, target 2 ms, enforced by a watchdog**: exceed it and the
  host logs, skips the remainder, and continues. Never drop a frame for a plugin. Expensive
  work is amortised at **fixed phase** (cone scans on `frame % 6 == 0`, pathfinding on
  `frame % 30 == 2`) rather than "when there's time", so behaviour stays deterministic and
  therefore testable.
- `map_renderer.py` survives as a host-side developer debug overlay; `text.py` and the
  external `text.txt` dump are deleted, since `rom/dialog.py` already extracts dialog from the
  ROM at load time, removing a setup step.
- Phase 4 tests this model against a second SNES game chosen to be structurally different
  from ALttP.

## Alternatives considered

- **All-Lua, no declarative tier** — rejected; forces code on games that only need a memory
  map, and duplicates boilerplate per plugin.
- **All-declarative, no scripting** — rejected; cone scanning, occlusion, object tracking, and
  pathfinding are not expressible as watch rules.
- **Native (Rust/C ABI) plugins** — rejected; loses the drop-in `plugins/` directory and the
  low barrier that lets community members write profiles.
- **Lua 5.5** — rejected; shipped December 2025, no ecosystem advantage over 5.4.
- **Manual profile selection by the user** — rejected; every configuration step is a place a
  blind user gives up.
- **Porting the ROM parser into Lua** — rejected; moved offline to `beacon-romdump` instead.
