# ADR 0014: Every setting is configurable, at runtime, without a text editor

- **Status:** Accepted
- **Date:** 2026-07-20

## Context

Early Phase 1 work hardcoded a speech rate. Listening to it immediately showed
why that was wrong: speech-dispatcher's default rate is slow for anyone
accustomed to a screen reader, and the right value is a matter of personal
preference that varies by an enormous margin. Experienced screen reader users
routinely run several times faster than a first-time listener can follow.

The same is true of nearly everything Beacon decides: how much it says, how
often it repeats, which voice, whether braille is on. There is no correct
setting, only a correct setting *for a given player on a given day*.

This pulls against [ADR 0013](0013-delivery-and-packaging.md), which promises no
configuration file to hand-edit. Both must hold at once.

## Decision

Two rules, together:

1. **Nothing needs configuring to start.** Every setting has a default that
   works. A first run never requires editing a file.
2. **Everything is configurable, at runtime.** No restart, and no text editor.

Implemented as typed settings with defaults, an optional TOML file, and a
string-keyed `get`/`set` interface so one handler serves the settings menu,
keyboard shortcuts, and the IPC command channel alike.

## Rationale

- **A text editor is a barrier, not a fallback.** Telling a blind user to edit
  a config file to fix speech that is too fast to understand is circular: they
  have to get through the tool to learn it is misconfigured. Settings must be
  reachable from inside the running program, self-voiced.
- **String keys mean no bespoke code per setting.** A voice command of "set
  speech rate 80", a menu item, and an IPC message all route through the same
  path. Adding a setting does not mean adding three handlers.
- **Clamping beats rejecting.** A player asking for verbosity 9 wants it as
  loud as it goes, not an error message. Values are clamped into range; only
  genuinely unparseable input is an error.
- **Runtime changes persist.** A setting found once should not have to be found
  again, so adjustments are written back to the settings file.
- **Defaults are validated by listening, not chosen from a specification.** The
  default speech rate was set by playing it to a listener and asking. It is
  recorded as a starting point for community tuning, not as a claim about what
  is correct.

## Consequences

- Every field added to `Settings` must also be added to the string interface,
  or it is unreachable from a menu or a voice command. There is a test that
  walks `Settings::keys()` and round-trips `get` then `set` for each, so a field
  added to only one place fails the build.
- Partial settings files must not lose unrelated values, so every struct is
  `#[serde(default)]`. A user hand-editing one field keeps the rest.
- `deny_unknown_fields` is set, so a typo in a hand-edited file is reported
  rather than silently ignored. A silently ignored setting is worse than an
  error, because the player concludes the feature does not work.
- A missing settings file is not an error. That is the normal first run.
- Settings that affect the arbiter are converted into its `Config` rather than
  read directly, keeping the deterministic core free of file and environment
  access. See [ADR 0012](0012-determinism-and-replay.md).

## Alternatives considered

- **Command line flags only.** Fine for development, useless mid-game, and they
  do not persist.
- **A settings file as the only interface.** Simplest to build, and it puts the
  barrier exactly where this audience cannot get past it.
- **Ship opinionated defaults and no configuration.** Defensible for a general
  tool. Wrong here: the variation between users is the point, not noise around
  a mean.
