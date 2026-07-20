# ADR 0012: Determinism and golden-file replay testing

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

The proof of concept had no test suite and structurally could not have one. It
polled a running emulator over UDP, so no two runs produced the same output, and
there was no way to assert that a change improved anything. Bugs were found by
playing and fixed by guessing. Nothing ratcheted: a fix could silently undo an
earlier one and no one would know until a player noticed.

Beacon owns the frame loop and bsnes-jg exposes `serialize` / `unserialize`, so
a session is reproducible in principle: a savestate plus an input log replays
identically. That property is worth protecting deliberately, because it is easy
to lose by accident and hard to recover once lost.

## Decision

Treat determinism as a load-bearing property of the whole system, not a
convenient side effect. A recorded session must replay frame for frame and
produce a byte-identical event stream, and that is asserted in tests.

Concretely, three constraints bind across subsystems:

- **No component on the frame path reads the clock.** Time is passed in by the
  caller. `Arbiter::resolve` takes `now: Duration` for exactly this reason.
- **Expensive work runs at fixed phase**, not opportunistically. Cone scans on
  `frame % 6 == 0`, pathfinding on `frame % 30 == 2`. "When there is time" is
  banned, because it makes output depend on machine speed.
- **Plugin state serialises alongside core savestates.** Otherwise loading a
  state desynchronises zone latches and tracked objects from the emulator, and
  rewind produces output that never occurred in any real playthrough.

## Rationale

- It is the only mechanism that makes quality ratchet. Every bug found becomes a
  permanent test, so the same bug cannot return.
- It converts vague accessibility complaints into reproducible artefacts. A
  community member can send a savestate and an input log rather than a
  description, which matters because the reporters cannot see the screen and
  cannot describe the visual context.
- It is nearly free if designed in from the start, and expensive to retrofit.
  Threading a clock parameter through later means touching every call site.
- It makes the arbiter tunable with confidence. Changing a rate limit and
  re-running fixtures shows exactly what became noisier or quieter.

## Consequences

- The arbiter is a pure function of its inputs and its own accumulated state.
  Nothing in it may consult wall-clock time, use a hash map iteration order that
  varies between runs, or depend on thread scheduling. There is a test asserting
  that identical inputs produce identical outputs.
- Amortised work must pick a fixed phase and document it. This costs a little
  latency jitter versus opportunistic scheduling, which is accepted.
- Plugin authors take on an obligation: any state carried across frames must be
  serialisable. This is a real constraint on the plugin API and is worth the
  cost.
- Audio and speech sinks sit outside the deterministic boundary, since real
  output is inherently timing dependent. Tests assert on the utterance stream
  the arbiter produces, not on what a synthesiser did with it.

## Alternatives considered

- **Test the plugins in isolation with hand-written state fixtures.** Cheaper,
  but it tests the plugin's view of memory rather than the memory the game
  actually produces, so it misses exactly the misreadings that matter.
- **Snapshot-test the rendered output.** Catches emulation regressions but says
  nothing about what the player is told, which is the part Beacon is
  responsible for.
- **Accept non-determinism and rely on manual testing.** This is what the proof
  of concept did, and why it is being replaced.
