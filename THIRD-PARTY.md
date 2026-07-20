# Third party components

Beacon is distributed under the GPLv3. Components it incorporates, and their
licences, are listed here. See [ADR 0009](docs/decisions/0009-gplv3.md) for the
licensing decision and its obligations.

## Incorporated now

### bsnes-jg

- **Upstream:** <https://gitlab.com/jgemu/bsnes>
- **Licence:** GPL-3.0-or-later
- **How:** vendored as the git submodule `vendor/bsnes-jg`, pinned to a release
  tag, built as a static library and linked into the Beacon binary.
- **Copyright:** 2004-2020 byuu, 2020-2024 Rupert Carmichael

Statically linking bsnes-jg is what makes Beacon GPLv3. This was a deliberate
choice, not an accident of dependency selection.

bsnes-jg itself bundles several components, which are therefore also present in
any Beacon binary. Their notices are in the submodule and are preserved intact:

| Component | Purpose |
|---|---|
| SameBoy | Game Boy emulation, for Super Game Boy support |
| libco | Cooperative threading |
| snes_spc | SPC700 DSP emulation |
| byuuML | BML markup parsing |
| libsamplerate | Audio resampling (vendored, built with `USE_VENDORED_SAMPLERATE=1`) |

### Cartridge databases

`Database/boards.bml` and `Database/SuperFamicom.bml` are embedded into the
Beacon executable via `include_bytes!` so that no loose data files need to be
installed. They are part of bsnes-jg and carry its licence.

## Planned

Listed here so licence compatibility is checked before adoption, not after.

| Component | Licence | Compatible with GPLv3 | Purpose |
|---|---|---|---|
| Lua | MIT | Yes | Plugin scripting |
| mlua | MIT | Yes | Rust bindings to Lua |
| Steam Audio | Apache-2.0 | Yes, one way into GPLv3 | HRTF spatial audio |
| cpal | Apache-2.0/MIT | Yes | Audio output |
| gilrs | Apache-2.0/MIT | Yes | Controller input |
| NVDA controller client | LGPL-2.1 | Yes | Windows screen reader output |

## Permanently excluded

| Component | Reason |
|---|---|
| snes9x | Non-commercial-only licence. Not OSI approved and not GPL compatible, so it cannot be linked even optionally. |
| Tolk | Abandoned upstream. Beacon talks to NVDA and JAWS directly instead. See [ADR 0007](docs/decisions/0007-speech-backends.md). |

## Game ROMs

Beacon ships no game data of any kind and never will. Users supply their own
ROMs. ROM images and anything derived from them are excluded by `.gitignore`.
