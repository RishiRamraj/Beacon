# ADR 0018: A debug-mode MCP server so an agent can help debug plugins

- **Status:** Proposed (design only; not yet implemented)
- **Date:** 2026-07-20

## Context

Writing a plugin is reverse engineering: find the address that holds health, work out what a
module number means, confirm an event fires when it should. That loop is slow by hand — run,
watch, guess an address, edit, run again.

An AI agent could drive much of it, if it could reach into a running Beacon: read memory at an
address, step a frame and see what changed, watch what the plugin emitted, try a Lua snippet,
reload the plugin. The **Model Context Protocol** is the standard way to expose exactly that —
a typed set of tools and resources an agent connects to.

The determinism already in place ([ADR 0012](0012-determinism-and-replay.md)) and the plugin
introspection already added ([ADR 0016](0016-dynamic-keybinding-and-actions.md),
`Plugin::commands()`) make Beacon unusually well suited to this: a session can be stepped and
inspected reproducibly.

## Decision (proposed)

Add a **debug mode** that runs an MCP server exposing Beacon's state and controls.

- **Opt-in and local.** `beacon <rom> --debug` starts the server; it is off otherwise. It
  binds a local endpoint only. Debug mode is the umbrella the debug-facing features live under:
  frame stepping, memory inspection, and the map buffer
  ([ADR 0017](0017-plugin-debug-drawing.md)) are all reached through it.
- **Transport.** A local socket rather than stdio, so the windowed emulator can keep running
  while an agent is attached. (Stdio remains an option for a purely headless debug run.)
- **Tools (agent-invoked actions):**
  - `read_memory(addr, len)`, `read_watch(name)` — inspect WRAM through the same addressing
    the plugin sees.
  - `step(n)`, `pause`, `resume` — drive the frame loop.
  - `save_state` / `load_state(slot)` — reproduce a situation exactly.
  - `run_command(id)` — invoke a plugin command and get its intents back.
  - `reload_plugin` — re-read the Lua from disk without restarting, to tighten the edit loop.
  - `eval_lua(chunk)` — run a snippet in the plugin's environment for probing.
- **Resources (agent-readable state):**
  - current frame, module/state summary, the last frame's proposed intents and what the
    arbiter did with them (the drop reasons already recorded);
  - the plugin's declared commands and watches;
  - the map-mode buffer as an image, so the agent can *see* the plugin's interpretation.
- **Determinism preserved.** Every tool is a function of state the agent set up (loaded slot,
  stepped frames), so a debugging finding is reproducible and can become a golden-file test.

## Why this shape

- Tools + resources is the MCP-native split: things that change state versus things that
  report it. It maps cleanly onto Beacon's existing verbs (step, read, command) and its
  existing telemetry (intents, drop reasons).
- A socket rather than stdio keeps the GUI usable while attached, which matters because much
  plugin debugging is "watch the screen while the agent pokes memory".
- `reload_plugin` and `eval_lua` are what turn this from an inspector into a genuine
  development loop; they are safe because debug mode is explicit, local, and the Lua sandbox
  is already closed.

## Open questions

- **Auth for the socket.** Local-only may suffice; a token is cheap insurance. Decide before
  implementing.
- **`eval_lua` scope.** Full plugin environment is most useful and most powerful. Whether to
  offer a read-only variant is open.
- **Overlap with `--json`.** The existing JSON event stream is a subset of what the server
  would expose; the server likely subsumes it for debugging while `--json` stays the
  lightweight production integration.

## Alternatives considered

- **A bespoke REST/JSON-RPC protocol** — rejected; MCP is the standard an agent already speaks,
  so there is no reason to invent one.
- **Reuse `--json` alone** — rejected; it is one-way and read-only, and cannot step, inspect an
  arbitrary address, or reload a plugin.
- **A separate external debugger process attaching over the emulator hook** — rejected as
  premature; the hook patch is deliberately unscheduled
  ([ADR 0010](0010-defer-emulator-hook-patch.md)), and in-process access already exposes
  everything the agent needs.
