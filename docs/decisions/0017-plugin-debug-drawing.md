# ADR 0017: Plugins draw a visual interpretation of memory (map mode)

- **Status:** Accepted (implemented)
- **Date:** 2026-07-20 (implemented 2026-07-21)

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

- **Lua API.** A plugin optionally defines `on_draw(canvas)`, called while the map view is
  open. The canvas is a fixed 256x256 `0x00RRGGBB` buffer the host owns, with a small
  primitive set: `canvas:clear(rgb)`, `canvas:pixel(x,y,rgb)`, `canvas:rect(x,y,w,h,rgb)`,
  `canvas:line(x0,y0,x1,y1,rgb)`, and `canvas:text(x,y,string,rgb)`, plus `canvas.width` /
  `canvas.height`. Primitives only — no image loading, no file access — so the surface stays a
  pure function of memory and the sandbox stays closed.
- **A bindable action.** `toggle_map` (default `m`) shows and hides the view, bound like any
  other action ([ADR 0016](0016-dynamic-keybinding-and-actions.md)), so a controller-only user
  reaches it too. A game whose plugin draws nothing says so rather than showing a blank view.
- **Rendering.** The host blits the canvas into the **main window**, replacing the game picture
  while the map is shown — simpler than a second window and enough for a debug/assist view.
  `on_draw` runs only while the map is open, so a hidden map costs nothing.
- **Determinism.** Drawing reads memory (and the frame number is passed), never a clock, so the
  same inputs produce the same picture — consistent with
  [ADR 0012](0012-determinism-and-replay.md).
- **Introspection for tooling.** The `get_map` MCP tool renders the map and returns it as a
  **PNG image content block** ([ADR 0018](0018-mcp-debug-server.md)), so an agent assisting with
  a plugin can *see* what the plugin drew, not only read numbers. The PNG is encoded by hand
  (stored-deflate zlib, no image-crate dependency), matching the project's lean-dependency
  stance.

## Why this shape

- A canvas with primitives is the smallest thing that reproduces `map_renderer.py` without
  reopening the sandbox. Anything richer (sprite sheets, image files) adds attack surface and
  a setup step for a feature that is fundamentally "plot some rectangles from RAM".
- Gating on a toggle and a fixed phase keeps the cost off the hot path when the map is not
  shown, and keeps it bounded and deterministic when it is.
- Making it a bound action rather than a fixed key keeps the whole surface consistent with the
  input model and reachable from a controller.

## As built, and what is left

- **Text** uses one built-in 5x7 bitmap font, currently covering digits, uppercase, space, and
  a little punctuation — enough for coordinates and room numbers. Lowercase and more glyphs are
  additive when something needs them.
- **The map replaces the game view** in the main window rather than opening a second one. If a
  side-by-side view is wanted later, it is a presentation change that does not touch the plugin
  API.
- **The canvas is a fixed 256x256.** A plugin scales into it via `canvas.width` /
  `canvas.height`. A configurable size can come later without breaking the primitives.
- **Redraw runs every frame while shown.** A fixed-phase throttle (every N frames) is available
  if a heavy map ever needs it, but nothing does yet.

## Alternatives considered

- **Host-side renderer per game** (the map drawn by host code) — rejected; it puts
  game-specific knowledge back in the host, the exact coupling the plugin model removed.
- **Reuse the speech/JSON path to describe the map in text** — rejected as a *replacement*; a
  picture is the point for the sighted-assistance and debug cases. The two coexist.
- **Full immediate-mode GUI in Lua** — rejected; far more surface than "draw my read of RAM"
  needs.
