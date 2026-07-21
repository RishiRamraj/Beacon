# ADR 0017: Plugins draw a visual interpretation of memory (map mode)

- **Status:** Proposed (design only; not yet implemented)
- **Date:** 2026-07-20

## Context

The alttp-navi proof of concept had a **map mode**: it drew Link's position, rooms, and
objects as an image, so a sighted developer could see, at a glance, what the tool believed the
game state to be. That view was the fastest way to tell a real detection bug from a
misread address, and its `map_renderer.py` was called out in
[ADR 0004](0004-plugin-model-toml-profile-plus-lua.md) as worth keeping as a host-side debug
overlay.

Beacon should give plugins the same ability: to render their *interpretation* of memory as a
picture. The audience is threefold — plugin authors debugging, sighted helpers assisting a
blind player, and partially-sighted players who can use a high-contrast schematic the actual
game does not provide.

This is deliberately **not** the spatial-audio navigation of Phase 3. It is a visual debug and
assistance surface, orthogonal to what the player hears.

## Decision (proposed)

Expose a **canvas** to Lua and show what the plugin draws on it in a toggleable view.

- **Lua API.** A plugin optionally defines `on_draw(canvas)`, called when the map view is
  visible. The canvas is a fixed-size RGBA buffer the host owns, with a small primitive set:
  `canvas:clear(rgb)`, `canvas:pixel(x, y, rgb)`, `canvas:rect(x, y, w, h, rgb)`,
  `canvas:line(...)`, and `canvas:text(x, y, string)`. Primitives only — no image loading, no
  file access — so the surface stays a pure function of memory and the sandbox stays closed.
- **A bindable action.** `toggle_map` opens and closes the view, bound like any other action
  ([ADR 0016](0016-dynamic-keybinding-and-actions.md)). It is a first-class action so a
  controller-only user can reach it too.
- **Rendering.** The host blits the canvas into a second window (or a split of the main one).
  `on_draw` runs only while the view is open, and at a bounded rate (every N frames), so a
  hidden map costs nothing and a visible one cannot dominate the frame budget.
- **Determinism.** Drawing reads memory and the frame number, never a clock, so the same
  inputs produce the same picture — consistent with
  [ADR 0012](0012-determinism-and-replay.md) and testable by hashing the buffer.
- **Introspection for tooling.** The rendered buffer is retrievable as an image by the debug
  server ([ADR 0018](0018-mcp-debug-server.md)), so an agent assisting with a plugin can *see*
  what the plugin drew, not only read numbers.

## Why this shape

- A canvas with primitives is the smallest thing that reproduces `map_renderer.py` without
  reopening the sandbox. Anything richer (sprite sheets, image files) adds attack surface and
  a setup step for a feature that is fundamentally "plot some rectangles from RAM".
- Gating on a toggle and a fixed phase keeps the cost off the hot path when the map is not
  shown, and keeps it bounded and deterministic when it is.
- Making it a bound action rather than a fixed key keeps the whole surface consistent with the
  input model and reachable from a controller.

## Open questions

- **Text rendering.** A built-in bitmap font is simplest and dependency-free; whether plugins
  need font choice is unclear. Start with one font.
- **Second window vs overlay.** A separate window is simplest and does not fight the game view;
  an overlay is more compact. Lean second window, revisit.
- **Coordinate model.** Fixed canvas size with the plugin scaling, versus a host-provided
  logical space. Lean fixed size, documented.

## Alternatives considered

- **Host-side renderer per game** (the map drawn by host code) — rejected; it puts
  game-specific knowledge back in the host, the exact coupling the plugin model removed.
- **Reuse the speech/JSON path to describe the map in text** — rejected as a *replacement*; a
  picture is the point for the sighted-assistance and debug cases. The two coexist.
- **Full immediate-mode GUI in Lua** — rejected; far more surface than "draw my read of RAM"
  needs.
