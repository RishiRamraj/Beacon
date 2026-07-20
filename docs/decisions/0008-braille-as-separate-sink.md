# ADR 0008: Braille is a distinct sink, never a mirror of the speech stream

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

Braille output is close to free to plumb. `nvdaController_brailleMessage()` lives on the same
DLL Beacon already loads for speech ([ADR 0007](0007-speech-backends.md)), and JAWS exposes
braille through the same COM interface it uses for `SayString`.

The temptation is therefore to subscribe braille to the arbitrated speech stream and call it
done. That would not work. **Braille is slow.** A fluent reader manages roughly 60–120 wpm, a
typical display shows 40 cells, and `brailleMessage` writes a *transient* message that the
next call overwrites. Piping the speech stream to it produces flicker: messages replaced
before they can be read, which is worse than silence because the user knows information is
passing them by.

## Decision

Ship braille, but as a **distinct sink with its own shape**, not as a filter over speech:

- **Its own verbosity, far stricter than speech.** Realistically CRITICAL and INTERACTION
  only. AMBIENT never reaches it.
- **Status, not events.** Braille suits a persistent status line (health, room name, current
  navigation target) that the user rests a finger on and checks deliberately, rather than a
  stream pushed at them. This is a different mental model from the speech sink, not a subset
  of it.
- **Spelling, paired with speech.** Synthesizers routinely mangle Zelda item and NPC names.
  Speaking "Magic Boomerang" *while* brailling the exact string is complementary rather than
  duplicative.

Scope: **Phase 1, Windows only, experimental.**

## Rationale

- The bandwidth mismatch is the whole argument. Speech and braille consume information at
  different rates and in different modes, so a shared stream serves neither.
- Exact spelling is something speech alone structurally cannot give, so this is a capability
  gain rather than redundancy.
- Platform coverage is uneven and should not be overstated:
  - **Windows (NVDA, JAWS)** — supported, cheap, as above.
  - **Linux** — **speech-dispatcher does not carry braille at all.** That is BRLTTY via
    BrlAPI, a separate integration and a real piece of work. Later, not Phase 1.
  - **macOS** — VoiceOver drives braille but exposes no usable public push API. Out of reach;
    do not promise it.

## Consequences

- The arbiter gains a second, independently configured consumer with its own verbosity gate.
  See [ADR 0005](0005-event-arbitration-in-host.md).
- A status-line model means the sink holds state and decides when to refresh, rather than
  reacting to intents one at a time. That is more implementation work than a mirror would be.
- **Deferred:** Linux braille via BRLTTY/BrlAPI is out of Phase 1.
- **Explicitly out of reach:** macOS braille. Do not promise it.
- **Accepted limitation on testing.** Braille displays cost roughly **$2,000–$6,000** and we
  do not have one. Development proceeds against NVDA's built-in **Braille Viewer**, a
  simulated display, which is enough to build and sanity-check against but not enough to call
  the feature verified. **Ship it flagged experimental until a user with real hardware
  confirms it**, and treat deafblind players as the population worth recruiting for that
  test. See [ADR 0011](0011-community-driven-iteration.md).

## Alternatives considered

- **Subscribe braille to the speech stream** — rejected; flicker, since `brailleMessage` is
  transient and speech outruns braille reading speed.
- **Skip braille entirely** — rejected; the plumbing is nearly free, and exact spelling of
  item and NPC names is a real capability speech cannot provide.
- **Buy a display to verify before shipping** — rejected on cost; ship experimental and
  recruit a tester instead.
- **Promise cross-platform braille** — rejected; Linux needs a separate BRLTTY integration and
  macOS has no public push API.
