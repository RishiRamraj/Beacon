# ADR 0019: Scan describes the nearest objects on demand, before any spatial audio

- **Status:** Accepted (initial implementation)
- **Date:** 2026-07-21

## Context

Navigation is the hard, valuable half of the project ([ADR 0011](0011-community-driven-iteration.md)):
telling a player what is around them and where. The design's Phase 3 reaches for spatial-audio
beacons and HRTF, but that is a large, hardware-flavoured build, and the plan is explicitly to
start from navi's existing model and iterate with the community rather than plan the whole thing
up front.

The first thing a player needs is simply to *know what is nearby*. For A Link to the Past that
information is in the sprite table — sixteen slots of active objects and enemies, at documented
RAM addresses ($0DD0 state, $0E20 type, $0D00/$0D20 and $0D10/$0D30 for position, $0E50 health).
Those addresses are public knowledge and were verified against the running game through the MCP
tools before anything was built on them.

## Decision

Implement **scan** in the alttp plugin as an **on-demand** description of the nearest active
sprites, and draw those sprites on the map.

- **On demand, not automatic.** Scan answers a keypress. It does *not* announce objects as they
  come and go each frame — that is the "auditory mess" the whole arbiter exists to prevent, and
  automatic proximity awareness needs tuning that only real players can give
  ([ADR 0005](0005-event-arbitration-in-host.md), [ADR 0011](0011-community-driven-iteration.md)).
- **Nearest first, capped.** Sprites are sorted by Manhattan distance from Link; scan reports the
  count and describes up to the three nearest. A busy room is a sentence, not a monologue.
- **Direction and rough distance, not raw numbers.** Each is described by an eight-point compass
  direction and a distance word (`right beside you` / `close` / `nearby` / `in the distance`),
  because "north-east, close" is navigable and "dx 40, dy -30" is not.
- **A crude enemy/object split.** Sprites with non-zero health are called "enemy", the rest
  "object". It is a heuristic, honest about its limits: naming specific sprite types needs a
  verified type table, which is future work (and exactly what the MCP RE workflow can build).
- **The map shows them.** `on_draw` plots each active sprite around Link — enemies red, objects
  cyan — so the same reading is visible as well as spoken, for debugging and sighted assistance.

## Consequences

- This is the *start* of navigation, deliberately small and shippable, so the community has
  something concrete to react to. Automatic proximity cues, spatial-audio beacons, and
  pathfinding build on top of the same sprite reading.
- The sprite addresses are alttp-specific and live in the plugin, not the host — a second game's
  scan is a different plugin, no host change.
- The enemy/object heuristic and the distance thresholds are tuning knobs, expected to change
  once players use them.

## Update — automatic proximity, then screen-entry callouts (2026-07-21)

Automatic enemy awareness first shipped as a ring-based proximity cue (speak when the nearest
enemy crosses into a closer ring). In play that proved too chatty. It was replaced, on player
feedback, with a quieter rule: **announce each enemy once, by name and direction, as it enters the
visible screen** ("Green Soldier, north-east"). A per-slot latch speaks the entrance once and
resets when the enemy leaves, so it stays quiet while an enemy is simply present, and the arbiter's
`enemy` category rate-limits a room that fills at once. The enemy's *type* comes from a sprite-type
→ name table and an enemy classification ported from alttp-navi, and the same table now names
sprites in `scan`.

The continuous **spatial-audio beacon** ([ADR 0021](0021-spatial-audio-beacons.md)) on the nearest
enemy carries the ongoing "where is it" sense, so speech no longer needs to. That division — speech
for discrete events (an enemy *appearing*), audio for continuous position — is the intended
"sonify timing, speak content" split ([ADR 0006](0006-sonify-timing-speak-content.md)).

## Alternatives considered

- **Automatic proximity chatter first** — rejected as the *opening* move; scan came first so there
  was a verified sprite reading to build on, and automatic cues followed conservatively (see the
  update above) rather than as an untuned firehose.
- **Spatial-audio beacons first** — deferred; it is a much larger build (a real-time tone mixer,
  eventually HRTF), and knowing *what is nearby* is the prerequisite value. Scan comes first,
  audio direction later, behind the same sprite reading.
- **Naming every sprite type** — deferred; a reliable type→name table is worth building through
  the MCP reverse-engineering workflow, but generic "enemy/object" with direction and distance is
  already useful and does not risk being confidently wrong.
