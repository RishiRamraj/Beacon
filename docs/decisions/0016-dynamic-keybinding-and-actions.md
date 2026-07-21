# ADR 0016: Dynamic key bindings, an action layer, and an input configuration modal

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

[ADR 0004](0004-plugin-model-toml-profile-plus-lua.md) left one seam where the host was not
game-agnostic: the command set was fixed in the binary. Three keys (scan, where, status) were
hardcoded, and there was no way to bind savestates, pause, or anything a plugin might want to
expose. Adding a game that needed a new command meant editing the host.

Several needs converged:

- **Host functions worth binding** — save and load state, save-slot selection, pause.
- **Frame advance** — stepping one frame at a time, to debug a plugin by watching memory
  change frame by frame.
- **Plugin custom commands** — a plugin should be able to offer game-specific actions ("read
  the current sign") that the user binds to a key, without the host knowing what they are.
- **Controller-only play** — a blind player may use a gamepad and no keyboard, so actions and
  the configuration itself must be reachable from the pad.

## Decision

**An action layer.** Every binding maps an input to an `Action`: a built-in host function
(`SaveState`, `LoadState`, `NextSlot`, `PrevSlot`, `Pause`, `FrameAdvance`, `CycleVerbosity`,
`RepeatLast`, `OpenInputConfig`, `Quit`) or a plugin command (`Command(id)`). Action ids are
strings — a bare name for a built-in, `command:<id>` for a plugin command — so the keymap is
a plain string-to-string map with no host types leaking into config.

**A persisted, runtime-editable keymap** ([`beacon_config::Keymap`]) mapping input names to
action ids, serialized as `[keys]` in the settings file. Full defaults ship; most users never
open it. A rebind rewrites the whole map and saves immediately, so a binding found once stays
found — the same principle as [ADR 0014](0014-everything-configurable-at-runtime.md).

**Plugin custom commands** are declared in the manifest (`[[command]]` with `id` and `label`,
capped at ten) and read back by the host through `Plugin::commands()`. The host lists them —
by the plugin's own labels — for the user to bind, and dispatches the bound key to
`Plugin::command(id)`. The host never knows what the command *means*.

**Game inputs and action inputs are disjoint by construction.** The SNES buttons are a fixed
mapping (keyboard and gamepad); actions may only be bound to keys and pad buttons the game
does not use. Binding refuses a game input. This preserves the original safety property — an
accessibility key can never also press a game button mid-combat — while making everything
else rebindable.

**Gamepad parity.** Actions bind to the pad's *extra* buttons (triggers 2, stick clicks,
mode) under `Pad:*` names, resolved through the same keymap as keys. A controller-only player
gets scan/where/status and the configuration on the pad by default, and can bind the rest.

**A separate input configuration modal**, not a rebind mode woven into play. Opening it pauses
the game and captures every input; the game cannot move while binding. It is fully
blind-operable and works from either device: up/down (or d-pad) choose an action, spoken with
its current binding; pressing any free key or pad button binds it; delete clears; escape (or
Start) finishes. A handful of navigation inputs are reserved inside the modal and can only be
rebound by editing the file — an acceptable corner.

**Savestate slots** ([`state::SlotStore`]) write the emulator's serialized state to per-ROM
slot files (`<config>/states/<rom-sha1>/<n>.state`), keyed by ROM hash so games never
collide. Ten slots, an active-slot cursor moved by next/prev, spoken confirmations.

**Frame advance and pause.** Pause halts the frame loop; frame advance steps exactly one frame
(pausing first). Once either is used, the "machine too slow" audio-underrun heuristic is
retired for the session, since wall-clock timing no longer reflects real speed.

## Consequences

- The host is now fully game-agnostic: a new game is a new plugin directory, including any
  commands it wants bound, with no host change.
- The keymap is a compatibility surface. Input names (`"KeyC"`, `"Pad:LeftThumb"`) are stable
  strings written to disk; the table that defines them must not churn.
- Game controls themselves are still fixed. Rebindable SNES buttons are a future extension;
  they are deliberately out of scope here because keeping them fixed is what makes action
  binding safe.
- Plugin state is still not serialized with savestates (see
  [ADR 0015](0015-plugin-runtime.md)); loading a slot re-derives plugin latches on the next
  frame.

## Alternatives considered

- **Rebind woven into the play loop** — rejected in favour of a distinct modal; capturing a
  binding keystroke while the game is also listening invites leaks and mistakes.
- **Chorded controller hotkeys** (hold Select + button, as RetroArch does) — rejected for now;
  the pad's genuinely-unused buttons are enough, and chords are harder to discover and to
  announce to a blind player. Revisit if a controller lacks spare buttons.
- **Commands declared only in Lua** — rejected; declaring them in the manifest lets the host
  list a plugin's commands, with labels, without running any Lua, which a configuration UI
  needs.
- **Savestates in a flat directory** — rejected; keying by ROM hash stops one game's slots
  from shadowing another's.
