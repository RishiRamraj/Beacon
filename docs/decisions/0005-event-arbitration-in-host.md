# ADR 0005: Event arbitration is a host service

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

The proof of concept's larger failure was not sampling, it was noise. It detected things well
and arbitrated them barely at all: events were sorted by priority and printed. There was no
rate limiting, no collapsing of many similar detections into the nearest one, no barge-in for
urgent information, and no user-tunable verbosity.

The doom notes state the problem precisely:

> it's an auditory mess because it triggers every elevational change as a lift floor

and

> if there are multiple triggers — zero in on the closest trigger

A tool that says everything is as unusable as one that says nothing.

## Decision

**Relevance filtering is a first-class architectural concern, not a polish item.** All
arbitration lives in the host, shared by every plugin. Plugins propose utterances with
metadata via `say(...)`; they never speak. The arbiter provides:

- **Priority classes.** Four, each able to interrupt those below: **CRITICAL** (incoming
  attack, death, low health), **NAVIGATION** (destination reached, zone entered, blocked by
  obstacle), **INTERACTION** (facing a chest, NPC in soft-target range), **AMBIENT** (cone
  scan results, scenery).
- **Rate limiting.** Per-category token buckets. A category that has spent its budget is
  **silently dropped rather than queued**, because stale spatial information is worse than
  none.
- **Nearest-only collapse.** Intents sharing a `collapse_key` within a frame collapse to the
  single instance with the smallest `distance`. Twelve floor triggers in a room produce one
  utterance about the nearest, not twelve.
- **Hysteresis.** The PoC's zone state machine (`None → approach → nearby → facing`) is
  promoted to a host primitive with an added **dead band** on the ring boundaries: downgrade
  thresholds sit slightly outside upgrade thresholds.
- **De-duplication and barge-in.** Identical text within a sliding window is dropped. A
  CRITICAL utterance cancels whatever is currently speaking rather than queueing behind it.
- **Verbosity.** A user-facing setting from 0 (critical only) to 3 (everything), gating by
  priority class, adjustable **mid-game by hotkey**.

## Rationale

- This is the direct, mechanical answer to "zero in on the closest trigger": collapse by key,
  keep the minimum distance.
- Putting it in the host means every plugin gets consistent behaviour for free, and it is the
  one place a fix benefits every game.
- Dropping rather than queueing rate-limited spatial events is correct because the
  information decays: an announcement about where something was is misleading.
- The dead band exists because a player standing on a ring boundary would otherwise get
  chatter as the state machine oscillates.
- Mid-game verbosity is non-negotiable: tolerance for chatter varies enormously between
  players and between a first playthrough and a tenth.
- Fixed-phase amortisation in the frame loop keeps arbitration deterministic, which is what
  makes golden-file replay tests possible from Phase 2 onward.

## Consequences

- The `say` metadata contract (priority, category, `collapse_key`, `distance`, `rate_limit`)
  becomes a stable plugin API surface. Changing its semantics changes every plugin's
  behaviour at once, for better or worse.
- Plugins lose direct control over output. A plugin cannot guarantee a specific utterance
  reaches the user; the arbiter may drop it. That is the point, and it is accepted.
- Tuning burden moves to the host: bucket sizes, window lengths, and dead-band widths are
  global defaults that will need iteration against real players. See
  [ADR 0011](0011-community-driven-iteration.md).
- Arbitration ships in **Phase 1, before any real plugin exists**, deliberately, so no plugin
  ever learns the habit of speaking directly.
- The braille sink does **not** simply subscribe to this stream; it has its own, far stricter
  verbosity. See [ADR 0008](0008-braille-as-separate-sink.md).
- Priority ordering only works end to end because time-critical information is sonified
  rather than spoken. See [ADR 0006](0006-sonify-timing-speak-content.md).

## Alternatives considered

- **Per-plugin arbitration** — rejected; every plugin re-implements it badly and behaviour
  becomes inconsistent between games.
- **Priority sort and print, as in the PoC** — rejected; produces the auditory mess this ADR
  exists to fix.
- **Queue rate-limited events instead of dropping them** — rejected; stale spatial
  information is worse than none.
- **Fixed verbosity chosen at launch** — rejected; needs to change mid-game, by hotkey.
- **Bare thresholds without a dead band** — rejected; causes chatter at ring boundaries.
