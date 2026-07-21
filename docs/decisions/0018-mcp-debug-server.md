# ADR 0018: An MCP server so an agent can drive the whole experience

- **Status:** Accepted (implemented)
- **Date:** 2026-07-20 (revised 2026-07-21)

## Context

This began as a debugging aid: writing a plugin is reverse engineering — find the address
that holds health, work out what a module number means, confirm an event fires — and that
loop is slow by hand. An agent that could reach into a running Beacon (read memory, step a
frame, watch what the plugin emitted) would speed it up enormously.

In building it the scope grew, deliberately. The same interface that lets an agent *debug* a
plugin lets an agent *operate Beacon* — press buttons, run commands, save and load, rebind
keys, walk the input configuration, and read back what was spoken. For the target audience
that is the larger prize: **an end user can hand their entire setup and play to an agent.** A
blind player who finds key-by-key configuration tedious can say what they want and have it
done.

The [session core](../design.md) extracted alongside this (a `Session` independent of the
winit window) is what makes it possible: the same logic a keyboard drives can be driven by an
agent, with no display.

## Decision

`beacon <rom> --mcp` runs the session **headless** and serves the Model Context Protocol on
**stdio**.

- **Stdio, not a socket.** MCP clients spawn their server and speak to it over stdio; that is
  the path of least resistance for an agent, and needs no port or auth. The original design
  reached for a socket to keep a GUI running alongside; that motivation fell away once MCP mode
  was headless — see below. `stdout` carries the protocol, so the JSON event sink is disabled
  there and the agent reads speech through a tool instead.
- **Headless, but audio and speech still play.** A blind player does not need the video window,
  and dropping it is what frees stdio and sidesteps needing a display. Audio and
  speech-dispatcher still run, so the human hears the game while the agent operates it.
- **Small threading.** A reader thread runs the protocol and forwards each tool call down a
  channel; the main thread owns the `Session` and is the only thing that touches it, running
  frames when nothing is pending. No shared mutable state, no lock — the emulator is
  single-threaded, as it must be.
- **Tools implemented:** `get_state`, `recent_speech`, `read_memory`, `step`, `pause`,
  `resume`, `set_buttons`, `run_command`, `save_state`, `load_state`, `set_slot`,
  `list_actions`, `get_bindings`, `bind`, `unbind`, `get_setting`, `set_setting`, the
  configuration walk (`open_config`, `config_navigate`, `config_bind`, `config_clear`,
  `config_close`), `get_map` (the plugin's map as a PNG, [ADR 0017](0017-plugin-debug-drawing.md)),
  and the plugin-dev loop `reload_plugin` and `eval_lua`. Tools that speak return what was
  spoken, so the agent perceives exactly what the player would hear — including driving the
  configuration modal entirely headless.
- **The plugin-dev loop.** `reload_plugin` re-reads a drop-in plugin from disk and rebuilds it,
  so an author edits the Lua, reloads, and sees the effect without restarting or losing the
  game's position (the plugin's own state resets, re-deriving from the next frame). `eval_lua`
  runs a snippet in the plugin's environment against the current frame, for probing memory and
  state. Both reuse the plugin runtime; a built-in plugin has no disk source, so reloading it
  just re-instantiates.
- **Determinism preserved.** Every tool acts on state the agent set up (buttons held, frames
  stepped, slot loaded), so a finding is reproducible and can become a golden-file test.

## Why this shape

- Returning spoken text from every acting tool makes the agent a first-class *listener*, not
  just a controller. Configuring bindings, running a scan, or stepping a frame all report what
  the player would have heard, which is the only sense that matters here.
- Reusing the exact action, modal, and memory-addressing code (the session core, and
  `beacon_plugin::wram_offset`) means the agent and the player cannot diverge: there is one
  implementation, driven two ways.

## Deferred

- **A socket transport** for attaching to an already-running windowed session. Not needed for
  the headless-agent use case; revisit if driving a live GUI session is wanted. This is the last
  open item under debug mode — the tool surface itself is complete.

## Alternatives considered

- **A bespoke JSON-RPC protocol** — rejected; MCP is the standard an agent already speaks.
- **Reuse `--json` alone** — rejected; it is one-way and read-only, and cannot step, inspect an
  address, rebind, or drive the configuration.
- **A socket + keep the GUI running** — deferred, not rejected; stdio + headless is simpler and
  covers the primary use case, and the two can coexist later.
- **An async runtime (tokio + an MCP SDK)** — rejected; the emulator loop is synchronous, so a
  single blocking reader thread and a channel is the whole concurrency story, with far fewer
  dependencies.
