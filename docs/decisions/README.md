# Architecture Decision Records

Decisions extracted from [`../design.md`](../design.md). The design document remains the
narrative source; these records are the per-decision index.

| # | Title | Status |
|---|---|---|
| [0001](0001-snes-only-scope.md) | SNES-only scope | Accepted |
| [0002](0002-embed-bsnes-jg-directly.md) | Embed bsnes-jg directly as a C++ library | Accepted |
| [0003](0003-rust-host-and-cpp-shim.md) | Rust host with a hand-written `extern "C"` C++ shim | Accepted |
| [0004](0004-plugin-model-toml-profile-plus-lua.md) | Plugin model is a declarative TOML profile plus Lua `on_frame` hooks | Accepted |
| [0005](0005-event-arbitration-in-host.md) | Event arbitration is a host service | Accepted |
| [0006](0006-sonify-timing-speak-content.md) | Sonify timing, speak content | Accepted |
| [0007](0007-speech-backends.md) | Per-platform speech sinks behind a `SpeechSink` trait, plus a JSON socket | Accepted |
| [0008](0008-braille-as-separate-sink.md) | Braille is a distinct sink, never a mirror of the speech stream | Accepted |
| [0009](0009-gplv3.md) | Licence is GPLv3 | Accepted |
| [0010](0010-defer-emulator-hook-patch.md) | Defer the bsnes-jg hook patch until a feature demands it | Accepted (deferred implementation) |
| [0011](0011-community-driven-iteration.md) | Iterate with the blind-player community rather than planning around them | Accepted |
