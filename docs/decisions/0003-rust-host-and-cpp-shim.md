# ADR 0003: Rust host with a hand-written `extern "C"` C++ shim

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

Beacon embeds bsnes-jg as a static C++ library ([ADR 0002](0002-embed-bsnes-jg-directly.md))
and owns the frame loop itself. The host needs to link a prebuilt `.a`, vendor C
dependencies, cross-compile to Windows and Linux, embed a Lua interpreter, and drive audio
and controllers.

bsnes-jg's public API is `src/bsnes.hpp` in `namespace Bsnes`:
`load/save/unload/power/reset/run`, `runAhead(n)`, `serializeSize/serialize/unserialize`,
`setAudioSpec/setVideoSpec/setInputSpec` (each a POD struct carrying a `void*` userdata plus a
C function pointer), and `getMemoryRaw(type)`. **It is C++ only. There is no `extern "C"`
surface.** Signatures use `std::string`, `std::vector`, `std::stringstream`, and
`std::istream**`.

## Decision

Write the host in **Rust**. Bridge to bsnes-jg with a **hand-written `extern "C"` shim that
flattens the API to POD types, plus `bindgen`**. Do not use `cxx` or `autocxx`.

## Rationale

- Rust makes static linking, vendored C dependencies, and cross-compilation first-class,
  which is exactly what the single-file delivery promise needs.
- `mlua` and the audio stack are mature, so the plugin runtime and audio path are not
  research projects.
- The C++ types in the signatures mean `cxx` and `autocxx` both fight the boundary rather
  than easing it. `autocxx`, the one tool that would auto-walk a large header, is
  semi-dormant: **no release since March 2025**, with an unanswered "is this maintained?"
  issue open since February 2026.
- The surface actually needed is **perhaps a dozen functions**, so the shim is a small,
  one-time, easily-audited file. That is a better trade than a code generator plus its own
  maintenance risk.
- There is **no existing Rust binding to any SNES emulator core**: no `bsnes`, `snes9x`, or
  `mesen` crate exists. The closest live precedent for compiling C++ into the crate is
  `sameboy-sys`, which is also a GPLv3 Rust crate and therefore a useful reference for the
  licensing shape as well.

Two known traps, both handled in the shim:

- `cc` auto-links `stdc++` when compiling C++, but linking a *prebuilt* `.a` links no
  standard library. Emit `cargo:rustc-link-lib=dylib=stdc++` explicitly.
- Unwinding through plain `extern "C"` is undefined behaviour in practice. `catch(...)` in
  the shim and return error codes.

Related library choices follow the same "avoid churn and abandonware" logic:

- **No SDL.** SDL3's Rust bindings are explicitly mid-migration and warn about missing
  features. `cpal` for audio, `gilrs` for controllers, and a minimal window and blitter cover
  what Beacon needs with far less churn risk.

## Consequences

- **Accepted risk:** the shim is a hand-written trust boundary. `unsafe` Rust over a C++ API
  with no stable ABI guarantee. Mitigation is discipline: keep it to about a dozen functions,
  POD-only, `catch(...)` at the boundary, and test it directly.
- Every bsnes-jg upgrade requires re-checking the shim against the header, since C++ gives no
  ABI stability guarantee.
- We own a small amount of build glue (static `.a` linking, explicit `stdc++`) that a
  higher-level binding generator would otherwise have handled.
- The shim is Phase 0 work and is on the critical path: nothing else matters until the Rust
  frontend runs a ROM through it.
- The absence of any prior Rust SNES core binding means no reference implementation to copy;
  `sameboy-sys` is the nearest pattern, not a drop-in.

## Alternatives considered

- **`cxx`** — rejected; requires shaping both sides around its type system, and the existing
  API's `std::string` / `std::vector` / `std::istream**` signatures fight it.
- **`autocxx`** — rejected; the only tool that would auto-walk the header, but semi-dormant
  with no release since March 2025.
- **C++ host, no Rust** — rejected; loses static linking ergonomics, cross-compilation, and
  the mature `mlua` and audio crates.
- **SDL3 for windowing, audio, and input** — rejected; Rust bindings explicitly mid-migration
  with documented missing features.
