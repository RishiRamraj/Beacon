# ADR 0011: Iterate with the blind-player community rather than planning around them

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

There is an existing community of blind players to iterate with. It is the most valuable
asset in the project and it resolves more risk than any technical decision in the design.

Two problems in the plan were shaped by *not* having that community in the picture.
Pathfinding was treated as a prerequisite for Phase 3, and it is the highest-uncertainty item
in the design: it needs a walkability map derived from tile attribute data the PoC already
reads, and deriving that reliably across overworld and dungeon tilesets is real work. And two
test gaps, JAWS coverage and braille hardware, looked like things only money could close.

Prior art suggests the ambitious version is achievable. The Toby Accessibility Mod for DOOM
(by Alando1, named for blind gamer Toby Ott, currently V9.0) ships narrated menus, an area
scanner, a map marker system, a pathfinder that routes to markers and exits, and snap-to-target
aiming. Nearly every feature requested in the Twilight Princess doc exists there already. But
"achievable" is not the same as "needed first".

## Decision

Get something playable into community hands as early as possible and treat their feedback as
the primary signal for Phase 3's priorities. **Ship rough and iterate publicly rather than
polishing in private.**

Concretely:

- Phase 3 **starts from navi's existing spatial model**, the two-ring proximity zones and the
  forward cone scan, which already work.
- **Pathfinding comes after real players report what the existing model actually fails at**,
  not before Phase 3 can ship.
- The community closes the **JAWS** and **braille hardware** test gaps.

## Rationale

- The PoC's spatial awareness may prove sufficient for large parts of the game. Where it is
  not, failure reports describe the *specific* navigation problem to solve rather than the
  general one.
- **Building a full pathfinder on speculation is how this stalls.** Pathfinding stays a live
  risk, but it is no longer a blocker on the critical path.
- **JAWS.** Community members will collectively hold JAWS licences. Manual verification by
  real users on real configurations beats anything a CI runner could do, and costs nothing.
  NVDA still runs in CI on every commit; JAWS is manual and occasional.
- **Braille hardware.** Displays cost roughly **$2,000–$6,000** and we do not have one.
  Deafblind community members are precisely the testers that feature needs, and recruiting one
  or two removes the blocker entirely. See
  [ADR 0008](0008-braille-as-separate-sink.md).
- Toby DOOM parity is the Phase 3 milestone and the point at which the tool becomes genuinely
  playable, so that is the natural moment for the feedback loop to start paying off.
- Existing precedent supports the pattern: `pokemon-access` and `pokecrystal-access` prove
  this architecture works, on Game Boy and a long-dead emulator, but validated.

## Consequences

- Beacon ships in a rough, incomplete state on purpose. Early users will hit problems that a
  longer private polish phase would have caught.
- Phase 3's contents are not fully knowable in advance. The roadmap past "port navi's spatial
  model" is deliberately underspecified.
- Verified quality on JAWS and braille depends on volunteers, which means it is not on a
  schedule we control. Both are explicitly flagged rather than claimed: the braille sink ships
  **experimental** until a user with real hardware confirms it.
- Recruiting and supporting testers is ongoing work, not a one-off.
- **Accepted risk:** JAWS cannot be tested continuously.
- **Live risk:** pathfinding remains the highest-uncertainty item. Deferring it does not solve
  it; it only removes it from the critical path.
- The user-facing verbosity control from
  [ADR 0005](0005-event-arbitration-in-host.md) becomes more important, since community
  tolerance for chatter is exactly the thing we cannot predict.

## Alternatives considered

- **Solve pathfinding before shipping Phase 3** — rejected; highest-uncertainty item, built on
  speculation about what players need, and the likeliest way for the project to stall.
- **Polish privately and release when complete** — rejected; forfeits the single asset that
  resolves the most risk.
- **Buy JAWS licences and a braille display for CI** — rejected on cost, and inferior:
  real users on real configurations beat a CI runner.
- **Copy the Toby mod's full feature set up front** — not rejected as a target, but rejected as
  a plan; it is a study reference, and feature order should follow reported need.
