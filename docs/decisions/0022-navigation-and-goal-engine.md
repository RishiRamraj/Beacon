# ADR 0022: A data-driven navigation and goal engine in the plugin

- **Status:** Accepted (implemented)
- **Date:** 2026-08-02

## Context

Early design deferred pathfinding on purpose ([ADR 0011](0011-community-driven-iteration.md),
design.md §9–§10): build the speech/beacon model first, ship it, and let real players say what
navigation actually needs before writing a pathfinder on speculation. That feedback arrived, and the
reference A Link to the Past plugin now carries a full navigation system — turn-by-turn guidance
through the overworld and dungeons toward the current quest objective. This record documents its
shape, because it is a large subsystem and the earlier docs said it did not exist yet.

Everything here lives **in the plugin** (`plugins/alttp/alttp.lua`), built on the existing host API
(`mem`, `say`, `beacon`, `on_draw`). The host gained only one small hook for it (map auto-show,
below). Guidance is game knowledge, so it belongs to the plugin, not the host.

## Decision

The guiding principle is **uniform data structures dispatched over, not per-case conditional logic**.
The whole quest is data; a handful of small drivers walk that data. The pieces:

- **A goal engine.** `GOALS` is one ordered table of plain records — the entire quest (lamp, uncle,
  Zelda, sanctuary, then each dungeon through Ganon). Each record carries a `done` predicate
  expressed as data (`{addr, n}` = "byte ≥ n", `{addr, bit=mask}`, or a list satisfied if any holds)
  and its routing fields. `current_goal(v)` is "the first unmet record". There are no per-goal
  `if` ladders; `route_to(s, g, v)` is the single dispatcher that reads a goal's fields and routes.

- **Waypoint chains.** A goal's `chain` is an ordered list of `{tx, ty, ...}` records. A waypoint
  with a `room` is a dungeon point (routed to by the in-room pathfinder); without one it is an
  overworld point (routed cross-screen). Optional fields — `say`, `arrival`, `cue`, `level` — are
  read uniformly. The chain is the route; adding a waypoint is adding a row, not code.

- **A* pathfinding, one routine.** `plan_path` is a 4-connected A* over the live collision grid
  (dungeon `$7F2000`; overworld decoded from ROM). `simplify` string-pulls the result to its
  corners. `planned_route = simplify(plan_path(...))` is the **single** routine both the live guide
  and the map renderer call, so a drawn route is exactly the one Link walks. A trailing `level`
  argument lets it plan on either floor of a two-level room.

- **The dungeon leg is stateless.** In whatever room Link is in, the guide heads for the *last
  reachable* waypoint of that room (an A* reachability probe, so a point behind a locked door leads
  to the door until it opens). No progress bookkeeping, so it is backtrack-proof by construction.
  Two-level rooms filter candidates to Link's current floor (`$7E00EE`), so a lower-floor point does
  not pull him across the upper floor's overlay — he is led to the stairs, and the lower point takes
  over once he descends.

- **Room objectives override the goal, as data.** `ROOM_OBJECTIVES` is a list of short-term
  detectors (clear the enemies, grab a dropped key, open a chest). Each is a record with an
  `active(s)` test, a spoken `cue`, and a `target(s)`. `room_objective(s)` returns the first active
  one; while it holds it takes precedence over the chain. Scope is data too: `FORCE_KILL_ROOMS` and
  `CHEST_ROOMS` key the kill/chest objectives to the rooms where they matter, gated on permanent
  progress bits so they never re-arm on a backtrack.

- **The guide starts and heals itself.** Navigation turns on by itself at the opening — once Link is
  up out of bed and controllable in his house — so the player is not left to discover a key. It
  re-aims on any context change, and also self-heals when left on but idle at an unchanged
  signature (which a savestate load produces), so it never sits silently on.

- **The map follows the guide.** The one host change: `Plugin::navigation_active()` (the alttp
  plugin reports its `nav_active` global) lets the host bring the map up on the moment guidance
  starts, edge-triggered so a manual hide is respected. See
  [ADR 0017](0017-plugin-debug-drawing.md).

Guidance is a global on/off the player flips once (the `advance` command / default `l`), plus
supporting commands (`pathfind`, `pathfind_stop`, `explore`, `mark`, `guide_to_mark`, `objective`),
all declared in the manifest like any custom command.

## Why this shape

- **Data beats conditionals.** A quest expressed as records is read, reviewed, and extended by
  editing tables; the drivers stay small and general. A new room's guidance is a new waypoint or a
  new objective record, not a new branch. This is the property to protect as the plugin grows.
- **One route routine** means the map cannot disagree with the audio guide — both are `planned_route`.
- **Stateless dungeon routing** (last reachable waypoint) removes a whole class of "stuck after
  backtracking / after a locked door opened" bugs that progress-tracking versions kept hitting.
- **In the plugin, not the host.** Routing needs game-specific collision, room, and progress
  semantics; keeping it in the plugin preserves the host's game-agnosticism (every other capability
  is shaped the same way).

## Consequences

- The plugin is now large. Its correctness rests on the collision decode and the RAM addresses it
  reads; both are covered by the `beacon-plugin` test suite and were verified against the running
  game and the `zelda3src` reference reconstruction.
- The host API grew by exactly one method, `navigation_active()`, with a `false` default so plugins
  without navigation are unaffected.
- Authoring a route is a playthrough activity: drive to each spot, capture a waypoint, paste it into
  the chain. Chains are authored per area/dungeon over time.

## Alternatives considered

- **Pathfinding in the host** — rejected; it would bake game-specific collision and room semantics
  into a host that is otherwise game-agnostic.
- **Per-goal / per-room conditional logic** — rejected; it was the original shape and did not scale.
  Folding each case into a uniform record dispatched by one driver is what keeps the system legible.
- **Progress-tracked dungeon routing** — rejected; targeting the last *reachable* waypoint each frame
  is simpler and backtrack-proof, where tracking "which waypoint next" repeatedly broke on
  backtracks and doors.
- **Building it before shipping Phase 3** — that was rejected back in [ADR 0011](0011-community-driven-iteration.md);
  this system is the result of doing it *after* real feedback, as that ADR intended.
