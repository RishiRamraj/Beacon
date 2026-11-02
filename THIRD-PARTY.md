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
| Tolk (with SAAPI64) | LGPL-3.0 | Yes | Windows screen reader output (NVDA, JAWS, others, SAPI). Loaded at run time if present beside the executable, not linked; shipped in the Windows package with its licence |
| lewton | Apache-2.0/MIT | Yes | Decoding Ogg Vorbis sounds |
| wgpu (with naga) | Apache-2.0/MIT | Yes | Drawing through the graphics card (DirectX 12, Vulkan, OpenGL), so capture tools see a game |
| pollster | Apache-2.0/MIT | Yes | Waiting on wgpu's start-up calls |
| MinGW-w64 GCC runtime (libstdc++, libgcc_s) | GPL-3.0 with the GCC Runtime Library Exception | Yes | C++ runtime for the Windows build, shipped beside `beacon.exe` |

## Permanently excluded

| Component | Reason |
|---|---|
| snes9x | Non-commercial-only licence. Not OSI approved and not GPL compatible, so it cannot be linked even optionally. |

## The Toby Accessibility Mod

Beacon's navigation is modelled on the Toby Accessibility Mod for Doom (Alando1, Proydoha,
Jarewill and others), and its cues use Toby's sound names. **None of Toby's files ship with
Beacon**: they belong to their authors, and some (the hit markers) were taken from commercial
games. Beacon plays them only from a copy of Toby the player already has, set as `sounds.dir`;
without one, it synthesizes stand-ins of its own.

## Game ROMs

Beacon ships no game data of any kind and never will. Users supply their own
ROMs. ROM images and anything derived from them are excluded by `.gitignore`.
