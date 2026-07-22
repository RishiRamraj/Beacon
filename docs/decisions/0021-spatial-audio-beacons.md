# ADR 0021: Spatial-audio beacons by stereo panning, HRTF deferred

- **Status:** Accepted (implemented)
- **Date:** 2026-07-21

## Context

Speech tells a player *what*; a positioned tone tells them *where*, continuously, without words.
The design's Phase 3 reaches for spatial audio with Steam Audio HRTF (§6.1), and the `beacon.set`
API was sketched in §4.1. This is the first, honest step of that: get positioned sound working
simply, put it in players' hands, and leave room to improve the rendering later
([ADR 0011](0011-community-driven-iteration.md)).

## Decision

**A plugin places beacons; the host renders them.** `beacon.set(id, opts)` and
`beacon.clear(id)` give the plugin a live set of positioned tones, re-set each frame as things
move. The host reads the set through `Plugin::beacons()` and synthesises the sound.

- **Stereo panning, not HRTF, for now.** Each beacon is a sine tone, panned left/right by
  direction with constant-power gains and scaled by loudness. This is dependency-free and
  immediately useful. HRTF (Steam Audio) is a strictly better renderer that can slot in behind the
  same `beacon` API and `BeaconMixer` interface later. Front and back are indistinguishable in
  stereo — the spoken cue ("Enemy north") carries that axis until HRTF arrives.
- **The plugin owns scale; the host stays game-agnostic.** A beacon carries `x`/`y` offsets and a
  `volume`. Only the *ratio* of `x`/`y` is used, for the pan direction, so the host assumes
  nothing about a game's units. The plugin computes `volume` (0-1) from distance in units it
  understands. `pitch` scales the tone.
- **Synthesised on the frame path, not the audio thread.** The mixer generates the frame's worth
  of samples and adds them to the drained game audio before it is queued, then clamps so the sum
  cannot clip. The real-time audio callback stays a plain ring-buffer drain. Each beacon keeps a
  continuous oscillator phase between frames, so a moving source glides rather than clicks.
- **Configurable and off-switchable.** `beacons.enabled` (default on) and `beacons.volume`
  (default 0.3, deliberately modest) are runtime settings like any other. Spatial audio is not to
  everyone's taste, and it must be one setting away from silence.

The alttp plugin uses it for a **nearest-enemy beacon**: within range, a tone pans toward the
closest enemy and grows louder as it closes, complementing the spoken proximity cue and the map.

## Why this shape

- Panning is the smallest thing that conveys direction, and it is correct as far as it goes;
  HRTF improves *how well*, not *whether*. Shipping panning now gets real feedback on thresholds,
  loudness, and whether a continuous tone is right — the things only players can judge — without
  first taking on a large C++ dependency.
- Keeping the host ignorant of game units (pan by ratio, volume from the plugin) preserves the
  same game-agnosticism as every other capability: the mixer is generic, the placement is the
  plugin's.
- Synthesising off the audio thread keeps that thread trivial and avoids sharing plugin state
  across threads — consistent with the single-threaded emulator core.

## Consequences

- `Plugin` gains `beacons()`, and `BeaconState` is a new type on the plugin API surface.
- The tone is continuous. Whether a pulsed tone (rate by proximity) reads better, and whether
  distinct timbres per beacon `tone` help, are open tuning questions for the community.
- There is no dedicated key to toggle spatial audio yet; it is a setting. A bindable action can be
  added if players want one at their fingertips.

## Alternatives considered

- **Steam Audio HRTF first** — deferred; a large build and dependency for a rendering upgrade,
  when panning already conveys direction. It fits behind this interface when the time comes.
- **Host computes distance attenuation from `x`/`y`** — rejected; it would bake a pixel scale into
  the host. Letting the plugin set `volume` keeps the host generic.
- **Synthesising in the audio callback** — rejected; it would put plugin-derived state on the
  real-time thread. Frame-path synthesis with a continuous phase is simpler and click-free.
- **Off by default** — rejected; it is the flagship of this phase and should be heard, but modest
  in volume and trivial to silence.
