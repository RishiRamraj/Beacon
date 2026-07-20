# ADR 0006: Sonify timing, speak content

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

[ADR 0005](0005-event-arbitration-in-host.md) defines a careful priority and barge-in model.
But the default output path is a screen reader, and delegating speech to a screen reader hands
that model to *another process's queue*. NVDA has no idea that an incoming-attack warning
outranks a cone scan. `cancelSpeech()` can force barge-in, but it is coarse and it races.

That is a structural conflict, not a plumbing bug. Better plumbing does not resolve it,
because the queue we need to control is not ours.

There is a second, harder constraint underneath it: a spoken sentence takes longer than the
event it describes. "Enemy attacking from the north" does not finish before the sword lands.

## Decision

Stop sending time-critical information through speech at all. Split the output by what the
information is for:

- **Tones carry timing.** Incoming attacks, pit edges, low health, and alignment with a
  navigation target are **sonified** through our own spatial mixer, where latency and
  interruption are entirely under our control.
- **Speech carries content.** Menus, item names, dialog, area descriptions, progress. None of
  it is frame-critical, and all of it benefits from the user's own configured voice, rate, and
  punctuation.

## Rationale

- With this split, the screen reader's unpredictable queue stops mattering, because nothing
  latency-sensitive travels through it.
- A player reacts to an earcon far faster than to a sentence. This is what the Toby
  Accessibility Mod does, and it is why it works.
- Beacons are positioned in a Link-relative frame; the host converts game coordinates to
  listener space using facing direction. The tone pans toward centre and **changes pitch or
  repetition rate as the player's facing aligns with the target**. This is the Toby DOOM and
  World of Warcraft convention, so blind players already know how to read it.
- **Steam Audio** (Apache-2.0, actively developed) provides HRTF. The permissive licence makes
  it the cleanest choice for static embedding and it has the best HRTF quality of the options.
  Apache-2.0 is one-way compatible into GPLv3, so it is fine here ([ADR 0009](0009-gplv3.md)).
- Game audio and beacons are separate mix sources, with game audio ducked slightly under
  speech.

## Consequences

- Sonification is not a nice-to-have layer on top of speech; it is where the safety-critical
  information lives. The spatial mixer becomes load-bearing and must be correct.
- Earcon vocabulary becomes a design surface of its own. Players have to learn what each tone
  means, and that vocabulary needs to stay small and consistent across plugins.
- **Accepted risk:** Windows speech is the weakest dependency, but this decision substantially
  defuses it. A degraded speech path costs comfort rather than playability. See
  [ADR 0007](0007-speech-backends.md).
- A cheap fallback path (stereo pan plus pitch, no HRTF) is required for weak hardware and for
  users who find HRTF disorienting.
- The incoming-attack cue is a tone, which is what makes it feasible at all; its *detection*
  is the thing that may eventually demand execution hooks. See
  [ADR 0010](0010-defer-emulator-hook-patch.md).

## Alternatives considered

- **Speak everything, use `cancelSpeech()` for barge-in** — rejected; coarse, racy, and the
  sentence does not finish in time regardless.
- **Self-voice everything to own the queue** — rejected as a default; it discards the user's
  configured voice and their braille. Self-voicing remains a supported mode, chosen for its
  own reason (spatial panning of speech), not as a workaround. See
  [ADR 0007](0007-speech-backends.md).
- **OpenAL Soft for HRTF** — rejected; LGPL with no blanket static-linking exception. Workable
  given Beacon is GPLv3, but Steam Audio is simply less friction.
