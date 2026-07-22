# Beacon — a dedicated accessible SNES emulator with per-frame plugins

Design document. Supersedes the `alttp-navi` proof of concept.

Working name: **Beacon** (the host emulator), **navi** (the A Link to the Past plugin).
The name comes from the navigation-tone metaphor in the Twilight Princess notes — the
beacon is the thing the player actually steers by.

---

## 1. Why the proof of concept isn't good enough

`~/Me/alttp-navi` works, and the domain knowledge in it is the most valuable asset here —
the SNES memory map, tile identification chain, proximity zone model, and ROM parser
represent real reverse-engineering effort that should be preserved. The problems are all
architectural, and all of them trace back to the same root: **the tool is a spectator, not
a participant.**

### 1.1 The sampling problem

The PoC talks to RetroArch over UDP with `READ_CORE_MEMORY` at 30 Hz. The game runs at
60 Hz. This produces four distinct failure modes:

- **Missed events.** Anything that exists for fewer than ~33 ms is invisible. That includes
  hitstun frames, invulnerability windows, one-frame dialog ID writes, and transient
  sprite-table entries. The PoC cannot see them, and no amount of tuning fixes that.
- **Torn state.** Roughly 50 addresses plus the sprite table are read as separate UDP
  round trips. The emulator does not stop between them, so a single "snapshot" can straddle
  a frame boundary and mix pre- and post-update values. Position from frame N, room ID
  from frame N+1 — silently inconsistent.
- **Latency.** Round-trip time is added to every read, on top of the sampling delay. For an
  incoming-attack cue, which is the whole point of the combat requirement in the Twilight
  Princess doc, this is the difference between usable and useless.
- **No determinism.** There is no way to replay a session and get the same output, so there
  is no way to write a regression test. The PoC has no test suite and structurally cannot
  have one.

### 1.2 The event spam problem

This is the more important failure, and the doom notes describe it precisely:

> it's an auditory mess because it triggers every elevational change as a lift floor

and

> if there are multiple triggers — zero in on the closest trigger

The PoC detects things well and *arbitrates* them barely at all. Events are sorted by
priority and printed. There is no rate limiting, no collapsing of many similar detections
into the nearest one, no barge-in for urgent information, and no user-tunable verbosity.
A tool that says everything is as unusable as one that says nothing. **Relevance filtering
is a first-class architectural concern, not a polish item**, and it deserves to be a host
service rather than something every plugin re-implements badly.

### 1.3 The delivery problem

Current setup: install RetroArch, install a specific core, hand-edit `retroarch.cfg` to
enable network commands, `pip install -e .`, source a ROM, source a separate text dump,
and read output on stdout with no screen reader integration and no audio cues. Every one
of those steps is a place a blind user gives up. The target is: download one file, point it
at a ROM, play.

---

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Scope | **SNES only** | Settled. This is what makes the decision below possible. |
| Emulator | **bsnes-jg embedded directly as a C++ library**, not via libretro | Scope is SNES-only, so the libretro C ABI buys nothing and costs real capability — see §2.2. bsnes-jg is the live bsnes line: v2.1.1 released July 2026, cycle-accurate, GPLv3, and genuinely built as a library. |
| Not libretro | — | Its ABI **cannot express memory watchpoints or execution hooks at all**, and never will. Confirmed by inspection: zero debug-oriented environment calls out of 91. |
| Not zsnes | — | Last release 2007, hand-written x86 assembly, unmaintained, inaccurate, not embeddable. Not a viable base. |
| Not snes9x | — | Its licence is genuinely non-commercial-only and is not OSI-approved or GPL-compatible. |
| Host language | **Rust** | Static linking, vendored C dependencies, and cross-compilation are all first-class. `mlua` and the audio stack are mature. |
| Plugin model | **Declarative TOML profile + Lua `on_frame` hooks** | The profile covers the mechanical 80%; Lua covers the logic. Lua embeds statically and is the twenty-year standard for emulator scripting. |
| Platforms | **Windows and Linux from day one**, macOS later | Windows is where NVDA and JAWS users are; Linux is where development happens. |
| Licence | **GPLv3** | Forced by statically linking bsnes-jg. Not a constraint worth fighting. |
| ROM | User-supplied, identified by SHA-1 | No copyrighted data ever ships. |

### 2.1 Corrections to assumptions made before research

Three things worth knowing before any code is written:

- **Tolk is abandoned.** Its README says so outright. The `tts` Rust crate routes Windows
  screen-reader output *through* Tolk, so it inherits that. Beacon should talk to the NVDA
  controller client and the JAWS COM interface directly instead — it is a few hundred lines
  and removes an abandonware dependency from the critical path.
- **SDL3's Rust bindings are explicitly mid-migration** and warn about missing features.
  Beacon does not need SDL: `cpal` for audio, `gilrs` for controllers, and a minimal window
  and blitter cover it, with far less churn risk.
- **Piper relicensed from MIT to GPL-3.0** when it moved to `OHF-Voice/piper1-gpl`. Since
  Beacon is GPLv3 anyway this is harmless, but it rules Piper out for anyone wanting to
  build a permissive fork.
- **bsnes-jg has no debugger.** near's bsnes never had one either — the debugging fork is
  bsnes-plus, a separate lineage. So watchpoints are a patch we write, not a feature we
  inherit. Costed in §3.4.
- **bsnes-jg has no performance profile.** Speed hacks were removed; its README says
  outright that if it is too slow, use a different emulator. There is no low-end fallback
  inside this codebase. See §10.

### 2.2 Why the libretro layer was dropped

Scoping to SNES-only removes the only thing libretro was buying — core portability — while
leaving its costs in place. Two of those costs are severe:

- **libretro's ABI cannot express memory watchpoints or CPU execution hooks.** Not "does not
  yet"; there is no mechanism. Of its 91 environment calls, none is debug-oriented. The one
  fine-grained facility, `SET_MEMORY_MAPS`, hands the frontend a static array of address
  descriptors — it is push-only, one-shot, and has no callback field in either struct. It
  describes memory; it cannot notify you about it.
- **RetroAchievements is the proof, not the counterexample.** It looks like it does
  watchpoints, but its `Delta`/`Prior` operators are a per-frame snapshot diff: read the
  buffer, compare to last frame, set a changed flag. There is no program-counter or
  instruction operand in its condition language anywhere. The most sophisticated memory
  instrumentation in the libretro ecosystem is a per-frame poller, because that is the
  ceiling.

Worth noting separately: bsnes-jg's *own* libretro core does not implement `SET_MEMORY_MAPS`
at all. It exposes only the four coarse region IDs. So even the theoretical ceiling wasn't
available in practice.

### 2.3 What direct embedding unlocks

Reading WRAM in-process every frame already fixes every sampling failure in §1.1 — that
alone justifies the move. But going direct also puts **watchpoints and execution hooks**
within reach, and those change what the tool can know:

- **Write-watchpoints replace frame-diffing.** Instead of comparing ~50 addresses between
  frames and inferring "health decreased", a write to `$7EF36D` calls you the instant it
  happens, with the program counter that did it. Exact rather than inferred, and cheaper.
- **Execution hooks answer *why*, not just *what*.** Hooking the damage-handling routine or
  the dialog-open routine is categorically more reliable than guessing from sprite state.
  **This is the credible path to the incoming-attack cue** — the hardest requirement in the
  Twilight Princess document, and one that frame-diffing sprite positions will never do
  well.

These are deliberately **not** in the critical path for Phase 0 (see §9). The point is that
the architecture leaves the door open, where libretro bricks it shut.

### 2.4 Prior art, and the gap

- **Toby Accessibility Mod** (DOOM, by Alando1, named for blind gamer Toby Ott, currently
  V9.0) is the closest design precedent and should be studied directly. It has narrated
  menus, an **area scanner**, a **map marker system**, a **pathfinder** that routes to
  markers and exits, and snap-to-target aiming. Nearly every feature requested in the
  Twilight Princess doc exists there already, in shipped form.
- **`pokemon-access`** / **`pokecrystal-access`** prove this exact architecture works:
  Lua scripts reading game RAM per frame inside an emulator, speaking through NVDA. Game
  Boy only, and built on a long-dead emulator, but the pattern is validated.
- **RetroArch's built-in accessibility** reads in-game text by **OCR on the framebuffer**.
  That is a fundamentally weaker approach than RAM instrumentation and is not competition.
- **No SNES per-frame RAM-instrumentation accessibility project appears to exist**, and
  none for A Link to the Past. The gap is real.

One correction to the notes: the Diablo 1 accessibility work referenced in `zelda.md`
appears to be an **in-progress effort in DevilutionX**, not a shipped mod. The most useful
artefact from it is the SDL screen-reader discussion, which is where Tolk was evaluated and
rejected for the same reasons above.

---

## 3. Architecture

### 3.1 The frame loop

The entire design rests on one property: Beacon owns the loop, so the plugin runs
*synchronously between frames* against real memory, with no sampling, no round trips, and
no tearing.

```
loop {
    input.pump();                       // controller + keyboard + IPC commands
    emu.run();                          // Bsnes::run() — advance exactly one frame
    let wram = emu.main_ram();          // &[u8; 128 KiB], zero-copy borrow
    let intents = plugin.on_frame(wram, frame_no);
    let chosen  = arbiter.resolve(intents);
    speech.dispatch(chosen.utterances);
    audio.update_beacons(chosen.beacons);
    video.present();
}
```

`Bsnes::getMemoryRaw(MainRAM)` returns a pointer to the emulator's 128 KiB of SNES WRAM.
The plugin reads it directly. What took fifty UDP round trips becomes a pointer
dereference.

### 3.2 Embedding bsnes-jg

Verified by building it, not inferred from documentation:

- Canonical repo is **GitLab** (`gitlab.com/jgemu/bsnes`). The GitHub mirror's default
  branch is `libretro`, which is the wrong tree — do not clone it by accident.
- `make ENABLE_STATIC=1 DISABLE_MODULE=1 USE_VENDORED_SAMPLERATE=1` produces
  **`objs/libbsnes.a`** (~4.9 MB), and builds cleanly. Note the artifact is `libbsnes.a`;
  `libbsnes-jg.a` is a different target — the Jolly Good frontend module, which we do not
  want.
- `DISABLE_MODULE=1` builds **without the Jolly Good headers present at all**, confirming
  the core is genuinely independent of that frontend API. We are embedding an emulator, not
  adopting a plugin framework.

The public API is `src/bsnes.hpp`, `namespace Bsnes` — `load/save/unload/power/reset/run`,
`runAhead(n)`, `serializeSize/serialize/unserialize`, `setAudioSpec/setVideoSpec/setInputSpec`
(each a POD struct carrying a `void*` userdata plus a C function pointer), and
`getMemoryRaw(type)`.

**It is C++ only — there is no `extern "C"` surface.** A hand-written shim is mandatory.

### 3.3 The Rust boundary

Signatures use `std::string`, `std::vector`, `std::stringstream`, and `std::istream**`, so
`cxx` and `autocxx` both fight the boundary rather than easing it — and `autocxx`, the one
tool that would auto-walk a large header, is semi-dormant (no release since March 2025, with
an unanswered "is this maintained?" issue open since February 2026).

The right shape is the boring one: **a hand-written `extern "C"` shim flattening the API to
POD, plus `bindgen`.** The surface we need is perhaps a dozen functions, so this is a small,
one-time, easily-audited file. Two known traps:

- `cc` auto-links `stdc++` when compiling C++, but linking a *prebuilt* `.a` links no
  standard library — emit `cargo:rustc-link-lib=dylib=stdc++` explicitly.
- Unwinding through plain `extern "C"` is undefined behaviour in practice. `catch(...)` in
  the shim and return error codes.

There is **no existing Rust binding to any SNES emulator core** — no `bsnes`, `snes9x`, or
`mesen` crate exists. The closest live precedent for the compile-C++-into-the-crate pattern
is `sameboy-sys`, which also happens to be a GPLv3 Rust crate, so it is a useful reference
for the licensing shape as well.

### 3.4 Memory access, and the hook patch

`getMemoryRaw` exposes **MainRAM (128 KiB WRAM)** and **VideoRAM**, plus cartridge RAM and
RTC. It does **not** expose ARAM, OAM, or CGRAM — those buffers exist internally but are not
public.

For this project that is a smaller problem than it looks: ALttP's sprite table, which navi
already reads, lives in WRAM (`$7E0D00`-ish), not OAM. CGRAM and ARAM are irrelevant to
accessibility. If OAM is ever needed, exposing it is a one-line addition to the same patch
below.

**The hook patch.** bsnes-jg's bus chokepoint is unusually clean — `src/memory.hpp` has
`Bus::read` and `Bus::write` as two inline one-liners, each with a single definition and all
44 call sites routed through them. The execution hook point is `WDC65816::instruction()`.
Three insertion points, plus a callback registry: on the order of **30 lines**.

Two things de-risk this considerably:

- **bsnes-plus is this exercise already completed**, and its `breakpoint_test()` call sites
  form a working map of every place a SNES core needs a hook — including the SMP, PPU, SA-1,
  and SuperFX points we would need if instrumentation ever goes beyond the CPU bus. We use
  it as a reference implementation without adopting it.
- The patch is **additive and localised**, so rebasing onto new bsnes-jg releases should stay
  cheap. It is nonetheless a fork we own; that cost is real and is listed in §10.

### 3.5 Why not bsnes-plus or MesenCE directly

Both were seriously considered, because both ship working watchpoints today.

**bsnes-plus** builds Qt-free as `libsnes.a` with a full debugger already in the core —
read/write/exec breakpoints across CPUBus, APURAM, VRAM, OAM, CGRAM, SA-1 and SuperFX,
mirror-aware and with value predicates, plus an existing `extern "C"` API and a performance
profile for weak hardware. Genuinely tempting. But it is based on **bsnes v073** — 2010-era
accuracy — is GPLv2, has one maintainer, and has had no commits since March 2025. Adopting
an unmaintained fork as the foundation of a multi-year project trades a 30-line patch for
permanent ownership of a much larger, older codebase. Wrong trade.

**MesenCE** (the active fork of Mesen2, which was archived in June 2026) is the strongest
alternative, and **it was built and measured rather than assumed.** Results:

- `make core` succeeds in **2m50s, exit 0, zero errors**, on a machine with **no `dotnet`
  installed** — the headless claim is true, the C# UI is genuinely not required.
- The debug API is real, not just present in source: `SetBreakpoints`, `GetDebugEvents`,
  `GetDebugEventCount`, `GetMemoryState`, `SetMemoryValue` are all exported as plain C, and
  `SnesWorkRam` / `SnesVideoRam` / `SnesSpriteRam` / `SnesCgRam` are confirmed memory types.
  Sprite RAM and CGRAM are more than bsnes-jg exposes at all.
- Licence is **GPL-3.0**, identical to bsnes-jg — a non-differentiator. Sour is still
  committing to the fork personally, so continuity is better than "abandoned upstream"
  suggests.

The packaging numbers are what decide it:

| | bsnes-jg | MesenCE |
|---|---|---|
| Artifact | `libbsnes.a`, **4.9 MB static** | `MesenCore.so`, **14.2 MB shared** |
| Shared-library dependencies | ~none | **50** |
| Exported symbols | 37, namespaced | **7,940** (688 C, 7,252 mangled C++) |

Those 50 dependencies are the entire desktop stack — SDL2, the full X11 set, Wayland, DRM,
GBM, ALSA, PulseAudio, libsamplerate — which is the opposite direction from a
single-file install. And 7,252 mangled C++ symbols in the dynamic symbol table means there
is no visibility control: the "unstable internal boundary" concern is visible in the
binary.

**The argument that settles it:** MesenCE's entire appeal was *no fork needed*. But `core`
and `ui` are the same makefile target, so SDL/X11/Wayland are compiled **into** the core
library. Getting a lean static build means patching the makefile to drop `SDLOBJ`/`LINUXOBJ`
and repairing whatever that breaks — **a fork of a larger multi-system codebase.** Once both
options require a fork, bsnes-jg's 30 lines on a dependency-free 4.9 MB static library is
plainly the cheaper one.

MesenCE was retained as the documented exit if performance or the C++ shim proved
unacceptable. **Both of those risks have since been accepted (§10.1), so the choice is
settled rather than provisional.** This section stays as the record of why, and as a
verified — rather than hypothetical — fallback should something genuinely unforeseen appear.

The deciding factor is that **bsnes-jg is the only option that is simultaneously actively
maintained, modern in accuracy, SNES-focused, and already a real library.** Owning a small
patch on healthy upstream beats inheriting someone else's abandoned one.

### 3.6 Frame budget

At 60 fps the budget is 16.6 ms. bsnes-jg's accuracy profile is CPU-hungry but leaves
comfortable headroom on modern hardware. The plugin gets a **hard budget (target 2 ms)
enforced by a watchdog**: exceed it and the host logs, skips the remainder, and continues.
Never drop a frame for a plugin.

Expensive work is **amortised at fixed phase** rather than run every frame — cone scans on
frames where `frame % 6 == 0`, pathfinding on `frame % 30 == 2`, and so on. Fixed phase
rather than "when there's time" keeps behaviour deterministic and therefore testable.

### 3.7 Determinism and testing

Because Beacon owns the loop and bsnes-jg exposes `serialize` / `unserialize`, a session is
fully reproducible: savestate plus an input log replays identically.

This makes the single biggest quality lever available: **golden-file regression tests.**
Record a movie through a dungeon, replay it headless, assert the emitted event stream
matches a fixture. Every bug found becomes a permanent test. The PoC cannot do this at all,
and its absence is a large part of why it is "not good enough" — there is no ratchet.

Plugin state must be serialised alongside the core state so that savestates and rewind
don't desynchronise the plugin's zone latches and tracked objects.

---

## 4. The plugin model

Three tiers, deliberately, so that simple games need no code at all.

### Tier 0 — ROM identification

Every profile declares the SHA-1s it supports. On load, Beacon hashes the ROM (headerless,
after stripping any 512-byte SMC header) and selects the matching profile automatically.
The user never picks anything.

### Tier 1 — the declarative profile

A TOML file covering memory watches and simple event rules. This is a near-mechanical
translation of the PoC's `constants.py` and much of `events.py`.

```toml
[game]
name   = "The Legend of Zelda: A Link to the Past"
sha1   = ["6d4f10a8b10e10dbe624cb23cf03b88bb8252973"]
region = "NTSC-U"

[watch]
health     = { addr = 0x7EF36D, size = 1 }
max_health = { addr = 0x7EF36C, size = 1 }
room       = { addr = 0x7E00A0, size = 2 }
link_x     = { addr = 0x7E0022, size = 2 }
link_y     = { addr = 0x7E0020, size = 2 }
direction  = { addr = 0x7E002F, size = 1 }

[[event]]
when       = "health decreased"
say        = "Hit. {health_hearts} hearts."
priority   = "critical"
rate_limit = "400ms"

[[event]]
when       = "room changed"
say        = "{room_name}"
priority   = "navigation"
```

### Tier 2 — Lua

For everything with real logic: proximity rings, cone scanning with line-of-sight
occlusion, object tracking, pathfinding. Embedded via `mlua` with the `vendored` feature so
Lua compiles from source into the binary — no runtime dependency, no DLL.

Use **Lua 5.4** rather than 5.5. 5.5 shipped in December 2025; 5.4 has the ecosystem and
`mlua` supports both, so there is no upside to being early here.

### 4.1 Host API exposed to Lua

```lua
-- memory (bounds-checked views over the live frame)
mem.u8(addr)  mem.u16(addr)  mem.u24(addr)  mem.slice(addr, len)
rom.u8(addr)  rom.slice(addr, len)          -- static ROM data
cache.get(key)                              -- precomputed ROM tables (see 6.2)

-- output: propose, do not speak
say(text, { priority = "navigation",
            category = "proximity",
            collapse_key = "chest",         -- see 5.3
            distance = 42,
            rate_limit = "1s" })

-- spatial audio
beacon.set(id, { x = dx, y = dy, tone = "nav", pitch = 1.0, loop = true })
beacon.clear(id)

-- haptics — the doom notes call out controller vibration in the modern remakes
rumble(strength, duration_ms)

-- interaction
on_command("scan", function() ... end)       -- reachable by voice or key
menu.open({ title = "Travel to", items = {...}, on_pick = function(i) ... end })

-- lifecycle
state.save()  state.load()                   -- serialised with core savestates
log(level, msg)
```

Note the shape of `say`: the plugin **proposes** an utterance with metadata. It never
speaks. All arbitration is the host's job, which is what makes it consistent across games.

---

## 5. Event arbitration — the fix for the "auditory mess"

This is the most important subsystem in the design and the largest single improvement over
the PoC. Everything here is a host service, shared by every plugin.

### 5.1 Priority classes

Four classes, each able to interrupt those below it:

- **CRITICAL** — incoming attack, death, low health. Barges in, cancelling current speech.
- **NAVIGATION** — destination reached, zone entered, blocked by obstacle.
- **INTERACTION** — facing a chest, NPC in soft-target range.
- **AMBIENT** — cone scan results, scenery.

### 5.2 Rate limiting

Per-category token buckets. A category that has spent its budget is silently dropped rather
than queued, because stale spatial information is worse than none.

### 5.3 Nearest-only collapse

The direct answer to *"zero in on the closest trigger."* Intents sharing a `collapse_key`
within a frame collapse to the single instance with the smallest `distance`. Twelve floor
triggers in a room produce one utterance about the nearest, not twelve.

### 5.4 Hysteresis

The PoC's zone state machine (`None → approach → nearby → facing`) is sound and should be
promoted to a host primitive, with an added **dead band** on the ring boundaries so a player
standing on the edge does not get chatter. Downgrade thresholds sit slightly outside upgrade
thresholds.

### 5.5 De-duplication and barge-in

Identical text within a sliding window is dropped. A CRITICAL utterance cancels whatever is
currently speaking rather than queueing behind it.

### 5.6 Verbosity

A user-facing setting from 0 (critical only) to 3 (everything), gating by priority class,
adjustable **mid-game by hotkey**. This is non-negotiable: tolerance for chatter varies
enormously between players and between a first playthrough and a tenth.

---

## 6. Audio, speech, and integration

### 6.1 Spatial audio

**Steam Audio** (Apache-2.0, actively developed) for HRTF. The permissive licence makes it
the cleanest choice for static embedding, and it has the best HRTF quality of the options.

Beacons are positioned in a Link-relative frame; the host converts game coordinates to
listener space using facing direction. Per the Twilight Princess doc, the tone should pan
toward centre and **change pitch or repetition rate as the player's facing aligns with the
target** — this is the Toby DOOM / World of Warcraft convention and blind players already
know how to read it.

Design notes:

- Game audio and beacons are separate mix sources. Duck game audio slightly under speech.
- A cheap fallback path (stereo pan plus pitch, no HRTF) for weak hardware or users who
  find HRTF disorienting.
- OpenAL Soft is the alternative, but is LGPL with no blanket static-linking exception —
  workable given Beacon is GPLv3, but Steam Audio is simply less friction.

### 6.2 Speech output

There is **no well-maintained cross-platform "speak this string" library in 2026.** The
design accounts for that rather than pretending otherwise: a `SpeechSink` trait with
per-platform implementations, none of which is on the critical path alone.

- **Windows** — NVDA controller client DLL directly, JAWS COM, SAPI5 fallback. Explicitly
  *not* via Tolk.
- **Linux** — speech-dispatcher over its SSIP socket protocol directly, which avoids a C
  dependency entirely.
- **macOS** — `AVSpeechSynthesizer` (not `NSSpeechSynthesizer`, which is legacy).
- **Always, on every platform** — line-delimited JSON events on a local socket.

That last sink is the important one. It means the tool never has to win an argument about
which speech engine is best; a user running Piper, a custom voice pipeline, or an unusual
screen reader can subscribe to the stream and do whatever they like.

On Windows, use `nvdaControllerClient.dll` directly — `nvdaController_speakText`,
`cancelSpeech`, `brailleMessage`, `testIfRunning`. It is documented, maintained by NV Access,
and redistributable, so we ship it. This is precisely what Tolk wrapped; calling it directly
removes the abandonware without losing anything. JAWS is COM (`SayString(text, flush)`), and
SAPI5 covers players running no screen reader at all — which is a real population and must
not be treated as an error case.

Braille is covered separately in §6.6 — the plumbing is trivial but the design is not.

### 6.3 Sonify timing, speak content

This is the most important decision in the audio design, and it resolves a conflict that
would otherwise be structural.

**The problem:** §5 defines a careful priority and barge-in model. But delegating speech to
a screen reader hands that model to *another process's queue* — NVDA has no idea that an
incoming-attack warning outranks a cone scan. `cancelSpeech()` can force barge-in, but it is
coarse and it races.

**The resolution is not better plumbing.** It is to stop sending time-critical information
through speech at all:

- **Tones carry timing.** Incoming attacks, pit edges, low health, and alignment with a
  navigation target are **sonified** through our own spatial mixer, where latency and
  interruption are absolutely under our control. A player reacts to an earcon far faster than
  to *"enemy attacking from the north"* — the sentence does not finish before the sword
  lands. This is what the Toby Accessibility Mod does, and it is why it works.
- **Speech carries content.** Menus, item names, dialog, area descriptions, progress. None
  of it is frame-critical, and all of it benefits from the user's own configured voice, rate,
  and punctuation.

With that split, the screen reader's unpredictable queue stops mattering, because nothing
latency-sensitive goes through it.

### 6.4 Self-voicing as a first-class option

Screen-reader output should be the **default** — familiar voice, zero configuration, braille
for free. But self-voicing must be a supported mode, not merely a fallback, for one reason
that no screen reader can match: **we can pan speech spatially.** Saying "chest" out of the
left channel is a powerful cue, and every screen reader is mono by design.

So: two speech modes, user-selectable, defaulting to the screen reader.

### 6.5 Not fighting the screen reader

The failure mode everyone forgets. NVDA will announce our window, may intercept keystrokes,
and runs its own speech queue alongside ours.

Ship an **NVDA app module** that puts NVDA into sleep mode for the Beacon window. It stops
double-announcing and releases its key hooks, while still accepting our controller-client
calls. This is an established pattern in accessible Windows games and is a small file.

Menus should be **self-voicing** rather than exposed through a platform accessibility tree
(UIA on Windows, AT-SPI on Linux). Fewer moving parts, identical behaviour on every platform,
and no dependency on a correctly-implemented accessibility hierarchy.

### 6.6 Braille

**Yes, but as a distinct sink — never as a mirror of the speech stream.**

The plumbing is genuinely cheap: `nvdaController_brailleMessage()` on the DLL we already
load for speech, and JAWS exposes braille through the same COM interface it uses for
`SayString`. That part is close to free.

The design is where the cost hides. **Braille is slow** — a fluent reader manages roughly
60–120 wpm, a typical display shows 40 cells, and `brailleMessage` writes a *transient*
message that the next call overwrites. Subscribing braille to the speech stream produces
flicker: messages replaced before they can be read, which is worse than silence because the
user knows information is passing them by.

So the braille sink needs its own shape:

- **Its own verbosity, far stricter than speech.** Realistically CRITICAL and INTERACTION
  only. AMBIENT never reaches it.
- **Status, not events.** Braille suits a persistent status line — health, room name,
  current navigation target — that the user rests a finger on and checks deliberately, rather
  than a stream pushed at them. This is a different mental model from the speech sink, not a
  filter over it.
- **Spelling, paired with speech.** The strongest use case: synthesizers routinely mangle
  Zelda item and NPC names. Speaking "Magic Boomerang" *while* brailling the exact string is
  complementary, not duplicative — and it is something speech alone structurally cannot give.

**Platform coverage is uneven and should not be overstated:**

- **Windows (NVDA, JAWS)** — supported, cheap, as above.
- **Linux** — **speech-dispatcher does not carry braille at all.** That is BRLTTY via
  BrlAPI, a separate integration and a real piece of work. Later, not Phase 1.
- **macOS** — VoiceOver drives braille but exposes no usable public push API. Out of reach;
  do not promise it.

**Testing is the honest limitation.** Braille displays cost roughly $2,000–$6,000 and we do
not have one. Development can proceed against NVDA's built-in **Braille Viewer**, a simulated
display, which is enough to build and sanity-check against. It is not enough to call the
feature verified. **Ship it flagged experimental until a user with real hardware confirms
it** — and treat deafblind players as the population worth recruiting for that test.

Scope: **Phase 1, Windows only, experimental.**

### 6.7 Voice input and commands

The same socket is bidirectional. Commands arrive as JSON:

```json
{"cmd": "scan"}
{"cmd": "navigate", "target": "shop"}
{"cmd": "verbosity", "level": 2}
```

This is what "works with existing voice-to-text systems" means concretely — Talon, Dragon,
Vosk, or anything else drives Beacon by writing to a socket, with no integration work on
either side. Every command also has a keyboard and controller binding, so voice is optional
rather than required.

---

## 7. Migrating the proof of concept

The Python is a working specification, not throwaway code. Roughly:

- **`constants.py` (memory map, lookup tables)** → the TOML profile. Near-mechanical.
- **`rom/` (~1,300 lines of ROM parsing)** → moves **offline**. A `beacon-romdump` tool
  parses the ROM once into a binary cache beside the profile, exposed to Lua as `cache`.
  This keeps a large, complex, one-time-cost body of code out of the hot path and out of
  the Lua port entirely — the biggest single reduction in porting work available.
- **`proximity.py` and `events.py`** → Lua. This is the real porting effort, and it should
  shrink, since arbitration, hysteresis, and zone latching move into the host.
- **`map_renderer.py`** → keep, as a host-side developer debug overlay.
- **`text.py` plus the external `text.txt` dump** → delete. `rom/dialog.py` already knows
  how to extract dialog from the ROM; do that at load time and remove a setup step.

---

## 8. Delivery

- **Windows** — a single static `.exe` plus a `plugins/` directory. No installer, no
  runtime, no configuration file to hand-edit.
- **Linux** — AppImage. Note honestly that a *fully* static Linux build is not realistic:
  `cpal` reaches PipeWire and PulseAudio through `dlopen`. AppImage is the correct answer
  and the promise should be "one file that runs", not "statically linked".
- **macOS** — signed and notarised `.app`, deferred. This needs a paid Apple developer
  account; flagging it as a real cost rather than a footnote.

Since bsnes-jg is compiled into the binary, there is no core to install, no core to keep in
sync with the frontend, and no version-skew class of bug. If another console is ever wanted,
it is a separate executable built on a different emulator library — the plugin layer, the
arbiter, and the speech and audio stacks are all system-agnostic and would carry over
unchanged.

---

## 9. Phases

- **Phase 0 — host skeleton.** Rust frontend, the `extern "C"` shim over `Bsnes::`,
  `libbsnes.a` statically linked, ROM loads, video, audio, controller input, savestates.
  A throwaway plugin that prints Link's health proves the frame hook. **Stock bsnes-jg, no
  patch** — per-frame WRAM reads only. Nothing else matters until this runs.
- **Phase 1 — output.** `SpeechSink` backends, the JSON socket, and the whole arbiter from
  section 5, including verbosity. Plus the Windows braille sink (§6.6), experimental, built
  against NVDA's Braille Viewer. All of this lands before any real plugin exists, so the
  plugin never learns bad habits.
- **Phase 2 — plugin runtime. _Landed._** Lua via `mlua` (vendored Lua 5.4), the TOML
  manifest loader, ROM SHA-1 matching, and the host API (`mem.u8/u16/u24/slice`, `say`,
  `on_command`, `log`, and the manifest's `watch` table). navi's event detection is ported
  as `plugins/alttp/alttp.lua`, selected automatically by ROM hash; the native `alttp.rs`
  stand-in is retired. Plugins are built-in (the alttp reference is embedded) or drop-in
  (`plugins/` beside the executable), with drop-ins overriding built-ins on the same ROM.
  Deferred within this phase, tracked in [ADR 0015](decisions/0015-plugin-runtime.md): the
  Tier-1 declarative event DSL (`[[event]]` rules), the 2 ms per-frame watchdog, `beacon.*`
  spatial-audio and `rumble` bindings (Phase 3 territory), and golden-file replay tests,
  which want the savestate + input-log harness to exist first.
- **Phase 2.5 — input, controls, and debugging aids. _Landed._** Dynamic key bindings over an
  action layer, a blind-operable input configuration modal reachable from keyboard or
  controller, plugin-declared custom commands, savestate slots, pause, and frame advance. See
  [ADR 0016](decisions/0016-dynamic-keybinding-and-actions.md). Two debug-facing capabilities
  are designed and scheduled but not yet built: plugins drawing a visual interpretation of
  memory (map mode, [ADR 0017](decisions/0017-plugin-debug-drawing.md)) and a debug-mode MCP
  server that lets an agent inspect memory, step frames, and reload a plugin
  ([ADR 0018](decisions/0018-mcp-debug-server.md)). Both live under a single opt-in **debug
  mode**.
- **Phase 2.6 — agent control. _Landed._** The MCP server from ADR 0018 is built. `--mcp`
  runs the session headless (audio and speech still play) and serves the Model Context
  Protocol on stdio, so an agent can press buttons, run commands, save and load, rebind keys,
  walk the input configuration, read memory, step frames, and read back everything spoken. An
  end user can hand their whole setup and play to an agent. Enabled by extracting a
  windowing-independent `Session` core that the winit shell and the MCP runner both drive.
- **Phase 2.7 — map mode. _Landed._** Plugins draw their interpretation of memory onto a
  `canvas` ([ADR 0017](decisions/0017-plugin-debug-drawing.md)): an `on_draw(canvas)` Lua hook
  with clear/pixel/rect/line/text primitives and a built-in font. A `toggle_map` action (default
  `m`) shows it in the window; the MCP `get_map` tool returns it as a hand-encoded PNG, so an
  agent sees what the plugin believes the state to be.
- **Phase 2.8 — plugin-dev loop. _Landed._** `reload_plugin` re-reads a drop-in plugin from
  disk and rebuilds it (edit the Lua, reload, see the effect — no restart, game position kept),
  and `eval_lua` runs a snippet in the plugin's environment against the current frame. With
  these the debug-mode tool surface is complete; the only deferred item is a socket transport
  for attaching to a live windowed session ([ADR 0018](decisions/0018-mcp-debug-server.md)).
- **Phase 3 — navigation. _Begun._** Two slices have landed on a verified sprite reading
  ([ADR 0019](decisions/0019-scan-nearest-first.md)): an on-demand **scan** describing the
  nearest active sprites by compass direction and rough distance (and drawing them on the map),
  and **automatic enemy proximity** — the plugin speaks when the nearest enemy crosses into a
  closer ring ("Enemy north, close"), hysteresis-gated so it announces on approach, not every
  frame, and rate-limited by the arbiter. The sprite addresses were verified against the running
  game through the MCP tools before being built on. Still ahead, on top of that same reading:
  spatial-audio beacons and arrival tones, a destination menu, soft targeting, a sprite-type
  naming table, and pathfinding. **Start from
  navi's existing spatial model** — the two-ring proximity zones and forward cone scan — and
  get it into community hands before building anything more ambitious. Pathfinding comes
  *after* real players report what the existing model actually fails at (§10.3). This is the
  Toby-DOOM-parity milestone and the point at which the tool becomes genuinely playable.
  **Reading the game's own text** has also landed
  ([ADR 0020](decisions/0020-rom-access-and-game-text.md)): plugins gained read-only `rom`
  access, and the alttp plugin decodes ALttP's dialogue table from the ROM at load, speaking a
  message when its box opens (dialogue and menus alike) with a `read_text` command to re-read.
  **Spatial-audio beacons** have landed too
  ([ADR 0021](decisions/0021-spatial-audio-beacons.md)): a plugin places positioned tones
  (`beacon.set`/`clear`) and the host renders them with constant-power stereo panning, mixed into
  the game audio. The alttp plugin puts a tone on the nearest enemy that pans toward it and grows
  louder as it closes. This is the simple first step — HRTF (Steam Audio) is a later upgrade
  behind the same interface, per the start-simple-and-iterate approach.
- **Phase 4 — proof of generality.** A second SNES game plugin, chosen specifically to
  stress the plugin API against something structurally different from ALttP. Packaging,
  documentation, ROM hash database.

**The hook patch is deliberately not scheduled.** Phases 0–2 run on stock bsnes-jg with
per-frame WRAM reads, which already fixes every sampling failure in §1.1 — the PoC's
problems came from UDP and 30 Hz sampling, not from a lack of watchpoints. The patch gets
written when a specific feature demands it, which in practice means the incoming-attack cue
in Phase 3. Don't become a fork maintainer before something is actually being bought. If
per-frame polling turns out to be sufficient for everything, the patch never gets written
and that is a good outcome.

Phase 4's second game should be a **different SNES title, not a different console** —
validating the plugin API matters more than validating the core abstraction, and adding a
second core drags in the symbol-collision problem for no learning.

---

## 10. Risks, and how they are dispositioned

Reviewed and closed. The bar for reopening any of these is new evidence, not new worry.

### 10.1 Accepted

- **bsnes-jg is CPU-heavy and has no fast mode.** Speed hacks were removed outright; the
  only knobs are `setCoprocDelayedSync` and `setCoprocPreferHLE`, and upstream's advice for
  "too slow" is to use a different emulator. **Accepted** — fast machines keep getting
  cheaper, and accuracy is worth more to this project than reach onto old hardware. Still
  worth *measuring* in Phase 0 to know where the floor is, but the number is now
  informational rather than a go/no-go.
- **Bus factor of one.** bsnes-jg's emulation work is effectively one person. **Accepted** —
  GPLv3 means the code cannot be taken away, and our patch is small enough to carry forward
  or rebase onto a successor.
- **We would own a fork.** ~30 lines at clean chokepoints, rebased per upstream release.
  **Accepted**, and still deferred until a feature demands it (§9).
- **The C++ shim is a hand-written trust boundary.** `unsafe` Rust over a C++ API with no
  stable ABI guarantee. **Accepted.** Keep it to a dozen functions, POD-only, `catch(...)`
  at the boundary, and test it directly.
- **Windows speech is the weakest dependency.** **Accepted**, and substantially defused by
  §6.3 — nothing latency-critical travels through speech, so a degraded speech path costs
  comfort rather than playability. The JSON socket remains the insurance policy.
- **JAWS cannot be tested continuously.** **Accepted** — NVDA in CI on every commit, JAWS
  manually and occasionally. See §10.3: the community likely closes more of this gap than CI
  could.
- **macOS is deferred.** Notarisation costs money and recurring effort for the smallest
  share of the audience. Confirmed as the right call.

### 10.2 Live

- **Pathfinding.** Still the highest-uncertainty item, but it is no longer a blocker on the
  critical path — see §10.3. It needs a walkability map derived from tile attribute data the
  PoC already reads; deriving that reliably across overworld and dungeon tilesets is real
  work.
- **Whether the hook patch is ever needed at all.** Genuinely open, and deliberately so.
  Phases 0–2 answer it empirically.

### 10.3 The community changes the plan

There is an existing community of blind players to iterate with. This is the most valuable
asset in the project and it resolves more risk than any technical decision in this document.

**It changes the pathfinding approach.** Rather than treating pathfinding as a problem to be
solved before Phase 3 can ship, **start by porting navi's existing spatial model** — the
two-ring proximity zones and the forward cone scan, which already work — and let real players
tell us what is actually missing. The PoC's spatial awareness may prove sufficient for large
parts of the game, and where it isn't, the failure reports will describe the *specific*
navigation problem to solve rather than the general one. Building a full pathfinder on
speculation is how this stalls.

**It also closes two test gaps that money otherwise would have to:**

- **JAWS.** Community members will collectively hold JAWS licences. Manual verification by
  real users on real configurations beats anything a CI runner could do, and costs nothing.
- **Braille hardware.** §6.6 flagged that we cannot verify the braille sink without a
  $2,000–$6,000 display. Deafblind community members are precisely the testers that feature
  needs, and recruiting one or two removes the blocker entirely.

**Implication for phasing:** get something playable into community hands as early as
possible, and treat their feedback as the primary signal for Phase 3's priorities. Ship
rough and iterate publicly rather than polishing in private.

### 10.4 Consequence: the emulator choice is settled

The two documented triggers for switching to MesenCE were poor performance and an
unpleasant C++ shim. **Both are now accepted rather than open**, so bsnes-jg is a settled
decision, not a provisional one. §3.5 stays in this document as a record of *why*, and as a
fallback if something genuinely unforeseen appears — not as a decision still being weighed.
### 10.5 Scope

The Twilight Princess document describes a much larger game on a much harder platform. It is
valuable as a **requirements document** — the clearest statement available of what blind
players actually need — but ALttP is the right proving ground, and Beacon's architecture is
what would eventually make the larger target approachable.

Legally the project is clean: no ROM data, no circumvention, no redistribution of anything
we do not have the right to redistribute.

**Licence: GPLv3. Decided and closed.** Statically linking bsnes-jg makes the whole binary
GPLv3, and that is accepted rather than worked around. Practical consequences to honour:

- Ship or offer the complete corresponding source, including our bsnes-jg patch if we ever
  write one, and including the build scripts needed to reproduce the binary.
- Keep upstream copyright notices intact, and carry `LICENSE` plus a `THIRD-PARTY.md`
  naming bsnes-jg (GPLv3), Lua (MIT), and Steam Audio (Apache-2.0).
- Apache-2.0 is one-way compatible **into** GPLv3, so Steam Audio is fine. Lua's MIT is
  fine. Nothing currently in the dependency list conflicts.
- snes9x stays permanently out of the tree — its non-commercial clause is GPL-incompatible,
  so it cannot be linked even optionally.
- This rules out a permissive fork later. If that ever becomes desirable, the base would
  have to change to ares (ISC), which is why §3.5 keeps the alternatives documented.
