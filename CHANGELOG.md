# Changelog

## 0.1.0-alpha.1 — 2026-09-03

The first version worth handing to someone. A Link to the Past is playable from the
opening to the Sanctuary with spoken guidance, spatial audio and screen reader support,
on Linux and Windows.

### The emulator

- bsnes-jg as a library, driven frame by frame, with the plugin instrumenting WRAM as it
  runs rather than reading it over a socket from outside.
- Audio paces emulation; the event loop sleeps against the audio queue rather than
  spinning. Video rendering can be turned off (`Settings` → Video), which is worth about
  an eighth of a CPU core to a player who cannot see it.
- Savestates in ten slots per game, keyed by ROM hash.

### Reading the game

- Spoken narration of surroundings, objectives, pickups, health, room changes and game
  text — read page by page as the game draws it, including choices.
- Spatial audio beacons per class (enemy, item, person, hazard, guide) with distance
  falloff, wall muffling and rhythmic signatures.
- Screens that are not gameplay: the file select and name entry, copy and erase, the
  pause item grid, and the game-over menu's three choices.
- Cues for what Link is facing (bush, block, pot, chest) and for pressing into something
  and going nowhere ("Stuck.").

### Guidance

- Route planning over the game's own collision data, in the room and across the overworld,
  taken from the game's classifier rather than from observation.
- Routes turn at right angles only, because the guide names one of four directions: high
  tone when Link faces the way to walk, middle when the route is to a side, low when he
  faces away. Destinations can require a facing too — a chest opens from below only.
- Authored waypoint chains for the intro, the courtyard and the castle escape.

### Accessibility surfaces

- Beacon's own spoken menu, an AccessKit tree (AT-SPI, UI Automation, NSAccessibility), and
  a native menu bar on Windows. The input mapping dialog is a real modal dialog with every
  action and its binding.
- Narration either through Beacon's own voice (speech-dispatcher on Linux, Tolk on Windows)
  or through the screen reader as a live region — `Settings` → Announce through screen
  reader.
- Braille output, and an MCP control surface so an agent can drive and inspect a session.

### Known gaps

Named rather than hidden; the full list is in the project's TODO.

- macOS is untouched: the native menu path is written and has never been compiled.
- Game buttons (the SNES controls) cannot be remapped — only Beacon's own actions.
- Guidance past the Sanctuary falls back to a room-graph heuristic; no chains are authored
  for Eastern Palace onward.
- The pause screen reads its item grid but not equipment, consumable counts, the map and
  quest panels, or the bottle menu.
- Neither Windows speech route has been confirmed by a screen reader user.
