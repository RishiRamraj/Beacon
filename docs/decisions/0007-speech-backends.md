# ADR 0007: Per-platform speech sinks behind a `SpeechSink` trait, plus a JSON socket

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

There is **no well-maintained cross-platform "speak this string" library in 2026.** The
obvious candidate on Windows, Tolk, is abandoned; its README says so outright. The `tts` Rust
crate routes Windows screen-reader output *through* Tolk, so it inherits that abandonment.

Beacon must also serve users running no screen reader at all, which is a real population and
must not be treated as an error case, and users running unusual setups (Piper, custom voice
pipelines, uncommon screen readers) whose needs we cannot enumerate in advance.

## Decision

A `SpeechSink` trait with per-platform implementations, **none of which is on the critical
path alone**:

- **Windows** — the NVDA controller client DLL directly, JAWS COM, SAPI5 fallback. Explicitly
  *not* via Tolk.
- **Linux** — speech-dispatcher over its SSIP socket protocol directly.
- **macOS** — `AVSpeechSynthesizer`.
- **Always, on every platform** — line-delimited JSON events on a local socket.

Screen-reader output is the **default**. Self-voicing is a supported first-class mode, not
merely a fallback.

## Rationale

- On Windows, use `nvdaControllerClient.dll` directly: `nvdaController_speakText`,
  `cancelSpeech`, `brailleMessage`, `testIfRunning`. It is documented, maintained by NV
  Access, and redistributable, so we ship it. This is precisely what Tolk wrapped, so calling
  it directly removes the abandonware without losing anything, at a cost of a few hundred
  lines.
- JAWS is COM (`SayString(text, flush)`). SAPI5 covers players running no screen reader.
- Speaking SSIP directly on Linux avoids a C dependency entirely.
- `AVSpeechSynthesizer` rather than `NSSpeechSynthesizer`, which is legacy.
- **The JSON socket is the important sink.** It means the tool never has to win an argument
  about which speech engine is best: a user running Piper, a custom voice pipeline, or an
  unusual screen reader subscribes to the stream and does whatever they like. It is the
  insurance policy behind the whole speech stack.
- The socket is **bidirectional**, so commands arrive as JSON (`{"cmd": "scan"}`,
  `{"cmd": "navigate", "target": "shop"}`, `{"cmd": "verbosity", "level": 2}`). That is what
  "works with existing voice-to-text systems" means concretely: Talon, Dragon, Vosk, or
  anything else drives Beacon by writing to a socket, with no integration work on either side.
  Every command also has a keyboard and controller binding, so voice is optional.
- Self-voicing earns first-class status for a reason no screen reader can match: **we can pan
  speech spatially.** Saying "chest" out of the left channel is a powerful cue, and every
  screen reader is mono by design.
- Beacon must not fight the screen reader. NVDA will announce our window, may intercept
  keystrokes, and runs its own speech queue alongside ours. Ship an **NVDA app module** that
  puts NVDA into sleep mode for the Beacon window: it stops double-announcing and releases key
  hooks while still accepting our controller-client calls. This is an established pattern in
  accessible Windows games and is a small file.
- Menus are **self-voicing** rather than exposed through a platform accessibility tree (UIA on
  Windows, AT-SPI on Linux): fewer moving parts, identical behaviour everywhere, and no
  dependency on a correctly-implemented accessibility hierarchy.
- Piper relicensed from MIT to GPL-3.0 when it moved to `OHF-Voice/piper1-gpl`. Harmless here
  since Beacon is GPLv3 ([ADR 0009](0009-gplv3.md)), but it rules Piper out for anyone wanting
  a permissive fork.

## Consequences

- Four or five separate speech integrations to write and maintain, each against a different
  API style (DLL, COM, socket protocol, Objective-C framework).
- **Accepted risk:** Windows speech is the weakest dependency. Substantially defused by
  [ADR 0006](0006-sonify-timing-speak-content.md), since nothing latency-critical travels
  through speech, so a degraded speech path costs comfort rather than playability.
- **Accepted risk:** JAWS cannot be tested continuously. NVDA runs in CI on every commit;
  JAWS is manual and occasional. The community likely closes more of this gap than CI could.
  See [ADR 0011](0011-community-driven-iteration.md).
- Shipping `nvdaControllerClient.dll` and an NVDA app module adds files to the Windows
  distribution, which slightly dilutes the single-`.exe` promise. Accepted.
- Two speech modes means two sets of behaviour to test and document.
- macOS speech follows the general macOS deferral. See [ADR 0009](0009-gplv3.md) for the
  packaging cost note; the platform itself is deferred.

## Alternatives considered

- **Tolk** — rejected; abandoned by its own README.
- **The `tts` Rust crate** — rejected; routes Windows output through Tolk and inherits the
  abandonment.
- **A single cross-platform speech library** — rejected; none well-maintained exists in 2026.
- **`NSSpeechSynthesizer` on macOS** — rejected; legacy.
- **Exposing menus through UIA / AT-SPI** — rejected; more moving parts and a dependency on
  correct platform accessibility hierarchies for no behavioural gain.
- **Self-voicing as the default** — rejected; screen-reader output gives a familiar voice,
  zero configuration, and braille for free.
