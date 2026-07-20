# ADR 0013: Delivery and packaging

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

The proof of concept required installing RetroArch, installing a specific
emulator core, hand-editing `retroarch.cfg` to enable network commands,
`pip install`-ing a Python package, sourcing a ROM, and sourcing a separate text
dump. Every one of those steps is a place a blind user gives up, and several of
them require sighted help to diagnose when they go wrong.

Installation difficulty is not a secondary concern for this audience. A tool
that is excellent once running and hard to install reaches far fewer people than
a mediocre tool that just works.

## Decision

**One file to download, point it at a ROM, play.** No installer, no runtime, no
configuration file to hand-edit, no separate data files.

Per platform:

- **Windows** — a single static `.exe` plus a `plugins/` directory.
- **Linux** — an AppImage.
- **macOS** — a signed and notarised `.app`. Deferred.

## Rationale

- Embedding bsnes-jg as a static library ([ADR 0002](0002-embed-bsnes-jg-directly.md))
  means there is no core to install and no version skew between frontend and
  core.
- The cartridge databases bsnes-jg needs (`boards.bml`, `SuperFamicom.bml`) are
  embedded with `include_bytes!` rather than shipped alongside. Without this the
  binary silently fails to load any ROM, which would be a miserable first
  experience.
- Lua embeds statically ([ADR 0004](0004-plugin-model-toml-profile-plus-lua.md)),
  so scripting adds no runtime dependency.
- Dialog text is extracted from the ROM at load time rather than shipped as a
  separate dump, removing a setup step the proof of concept required.

## Consequences

- **A fully static Linux build is not realistic, and we should not claim it.**
  `cpal` reaches PipeWire and PulseAudio through `dlopen`, so the honest promise
  is "one file that runs", not "statically linked". AppImage is the correct
  vehicle.
- **macOS costs money.** Notarisation requires a paid Apple developer account
  and recurring effort, for the smallest share of the audience. Deferred
  deliberately, not overlooked.
- Windows is the primary target because NVDA and JAWS users are there. This is a
  change of direction from the Linux-first proof of concept.
- Embedding databases adds a few hundred kilobytes to the binary. Irrelevant
  against the benefit.
- Beacon ships **no game data of any kind**. ROMs are user supplied, identified
  by SHA-1 so the right plugin profile is selected automatically. ROM images and
  anything derived from them are excluded by `.gitignore`, and a pre-commit
  check is warranted because an accidentally committed ROM is both a legal
  problem and effectively unfixable without rewriting history.
- GPLv3 obliges shipping or offering complete corresponding source, including
  build scripts, for every binary released. See [ADR 0009](0009-gplv3.md).

## Alternatives considered

- **Distribute via package managers (apt, winget, Homebrew).** Better for
  updates, but packaging lag and per-distro divergence mean the user's first
  experience depends on someone else's release cadence. Worth adding later,
  alongside direct downloads rather than instead of them.
- **Ship a small launcher that downloads components on first run.** Fewer bytes
  up front, but it turns a first run into a network-dependent operation with its
  own failure modes to diagnose.
- **Flatpak on Linux.** Reasonable, and worth adding, but AppImage requires
  nothing installed to try.
