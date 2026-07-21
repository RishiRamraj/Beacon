# ADR 0015: Plugin runtime — built-in plus drop-in, memory staged by copy, watchdog deferred

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

[ADR 0004](0004-plugin-model-toml-profile-plus-lua.md) decided *what* a plugin is: a
declarative TOML manifest plus a Lua script, selected by ROM SHA-1. Phase 2 builds the
runtime that loads and runs one. That raised a handful of implementation decisions ADR 0004
did not settle: where plugins are found, how Lua sees emulator memory safely, what the frame
entry point looks like, and what parts of the full design ship now versus later.

The reference plugin is `plugins/alttp/`, a direct port of the native `alttp.rs` stand-in
written in Phase 1. Porting it — rather than inventing a new example — is what validates the
host API against something real, exactly as ADR 0004 intended.

## Decision

**A `beacon-plugin` crate**, with a `Plugin` trait (`name`, `on_frame`, `command`) the host
drives. `LuaPlugin` implements it; `NullPlugin` is the no-op used when no plugin matches, so
the frame loop always has something to call rather than threading an `Option` through.

**Plugins are built-in or drop-in.** The alttp manifest and script are embedded with
`include_str!`, so a fresh binary instruments the game it was designed around with zero
setup. A `plugins/` directory beside the executable (and, for development, in the working
directory) is also scanned. A drop-in matching the same ROM **overrides** a built-in, so a
user can iterate on a plugin without rebuilding Beacon. A missing directory is not an error;
a single malformed drop-in is skipped with a warning rather than aborting the load.

**Memory is staged by copy.** Before each `on_frame`/`command`, the frame's 128 KiB of work
RAM is copied into a host-owned buffer that the `mem.*` closures read through. A plugin
therefore cannot retain a reference to emulator memory and read it later, when it would be
stale or dangling. `mem.u8/u16/u24/slice` take **SNES addresses** (`0x7ExxxX`, plus the
low-RAM mirror), resolved per read, so a script's constants match a memory map or a
disassembly exactly. An out-of-range read is `nil` (or an empty string for `slice`), never a
wrong value or a panic.

**`say` proposes; the host disposes.** A `say(text, opts)` call becomes an `Intent`
collected during the call and drained afterward. `on_frame` output goes through the arbiter;
`command` output is spoken immediately, because it answers a direct keypress. `opts` maps
onto the existing `Intent`: `priority`, `category` (defaulting to the priority name),
`collapse_key` + `distance`, and `rate_limit` (parsed from `"400ms"` / `"1s"`).

**A raising plugin never takes down the host.** Every Lua call is wrapped; an error is logged
with the chunk name and the frame continues. The game keeps running even if the plugin is
broken.

## Deferred, deliberately

- **The Tier-1 declarative event DSL** (`[[event]]` rules like `when = "health decreased"`).
  The manifest carries `[game]`, `script`, and `[watch]` today. alttp is nearly all logic, so
  it is ported entirely in Lua; the declarative tier earns its keep on a *simpler* game and
  can land then, against a real second plugin, rather than being speculated at now.
- **The 2 ms per-frame watchdog** from ADR 0004's consequences. Enforcing a wall-clock budget
  mid-Lua-call needs an instruction-count debug hook and interacts with determinism
  ([ADR 0012](0012-determinism-and-replay.md)); it is not worth building before a plugin
  exists that could blow the budget. A runaway plugin is currently a hang, documented as a
  known gap.
- **`beacon.set/clear` (spatial audio) and `rumble`.** These belong to Phase 3 navigation and
  have no consumer yet.
- **Golden-file replay tests.** They want the savestate + input-log harness, which does not
  exist yet. The plugin path is already deterministic (memory in, intents out, no clock), so
  the tests are additive when the harness lands.

## Consequences

- The host API is now a compatibility surface, as ADR 0004 warned. It is small and additive
  so far, which keeps that surface cheap to extend.
- Embedding the alttp plugin couples the binary to one reference plugin. That is acceptable
  while alttp *is* the reference; Phase 4's second game will test that the API generalises,
  and nothing stops the embed being dropped later in favour of pure drop-ins.
- Plugin state lives in Lua upvalues and persists for the plugin's life. It is **not** yet
  serialised with core savestates, so a load will desync a plugin's latches (e.g. the
  low-health warning) until the next frame re-reads them. Acceptable for the current
  per-frame-derived state; revisit when a plugin tracks something not recoverable from one
  frame.
- The copy-per-frame is ~128 KiB at 60 Hz. Measured cost is nil: headless throughput stayed
  at ~190 fps / 3.2× realtime with the Lua plugin live, unchanged from Phase 0.

## Alternatives considered

- **Zero-copy memory via a raw pointer valid only during the call** — rejected; it trades a
  provably-safe copy whose cost is unmeasurable for `unsafe` and a footgun, on a frame path
  with 3× headroom to spare.
- **`Option<Box<dyn Plugin>>` instead of `NullPlugin`** — rejected; sprinkles `if let Some`
  through the frame loop for no benefit over a no-op.
- **External-only plugins, nothing embedded** — rejected; it reintroduces a setup step for
  the one game Beacon ships knowing about, against the single-file promise.
- **Building the declarative event tier now** — rejected; designing a DSL against a game that
  does not need it invites a bad DSL. See deferred, above.
