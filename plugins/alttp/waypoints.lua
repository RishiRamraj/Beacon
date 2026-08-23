-- Beacon waypoint chains for The Legend of Zelda: A Link to the Past.
--
-- A data-only module. The host loads it (declared in the manifest's `modules`)
-- into the same Lua state BEFORE alttp.lua, exactly like data.lua — its own chunk
-- with its own local budget. It hands its chains to the script through the global
-- `WAYPOINTS` namespace; alttp.lua compiles each one with WP.chain. Pure data: no
-- game logic, no host-API calls, no per-frame work.
--
-- A chain is an ordered list of world-tile waypoints the guide leads Link through
-- in turn. Every field but tx/ty is optional:
--   tx, ty      world tile coordinates (world pixels >> 3)
--   room        dungeon room id ($7E00A0). Present = a dungeon waypoint, absent = overworld.
--   level       floor within the room: 0 upper, 1 lower ($7E00EE)
--   say         spoken as the guide sets off toward this waypoint
--   arrival     spoken on reaching it, in place of the generic arrival line
--   cue         true = speak `arrival` when Link passes near, but never route here
--   via         true = mandatory; the chain may not advance past this waypoint
--   after_lift  only a target once Link is carrying something
--   push        a push obstacle: the direction to face (0 north, 2 south, 4 west, 6 east)
--   track       sprite type to follow — the waypoint rides that sprite's live position
--   track_dx, track_dy  tile offset from the tracked sprite (e.g. onto its push side)
--   gate        a predicate: may the guide aim at this waypoint yet?
--   done        a predicate: has its errand already been carried out?
--   note        why this waypoint is here. Prose, for whoever edits the chain next.
--
-- `gate` and `done` are declarative clauses rather than code — WP.compile in
-- alttp.lua turns each into the closure the chain driver calls, which is what
-- keeps this file pure data the editor can rewrite without parsing Lua. A clause
-- is {name, args...}, evaluated against a focus tile; the focus is the waypoint's
-- own tx/ty/level unless `at` moves it:
--   {"tile_outside", lo, hi}  the focus tile's collision attribute is outside
--                             [lo, hi] — how the game reports a finished errand,
--                             because it rewrites the tile out of its class: a
--                             locked door leaves 0xF0-0xFF when opened, a chest
--                             leaves 0x58-0x5D when looted, a push-block leaves
--                             0x70-0x7F when shoved. An unreadable tile counts as
--                             outside.
--   {"tile_inside", lo, hi}   the inverse
--   {"at", tx, ty, level, clause}   the same clause, about some other tile
--   {"any", clause, ...}      any one holds
--   {"all", clause, ...}      all hold
--   {"not", clause}           the inverse
--   {"keys", n}               Link holds at least n small keys ($7EF36F), default 1
--   {"pushed", latch}         the tracked sprite has reached its end stop
--                             ($7E0ED0 + slot), default 0x90
--   {"byte", addr, min, max}  a WRAM/save byte within a range (max defaults 0xFF)
--   {"bit", addr, mask}       any bit of mask is set
-- An unrecognised clause reads as true, so a typo in this file can never wedge the
-- guide by gating a waypoint shut forever.
--
-- Everything from `WAYPOINTS = {` down is machine-editable: scripts/waypoints.py
-- reads Link's live position over MCP and regenerates that half from the data.
-- Prose belongs in a `note` field, not in a comment, or the editor will drop it.
-- This header is preserved verbatim.

WAYPOINTS = {

UNCLE_APPROACH = {
  note = "The overworld approach from Link's house to the castle entrance, as the player's own authored cues, mapped live by playing. The uncle beat drives this chain once Link is in the castle overworld area, so the guide speaks these cues rather than restating the objective. Each is an `arrival` line, spoken on reaching the spot, with the guide leading there silently over the sonar path. The last waypoint is the castle-entrance bush.",
  { tx = 280, ty = 316, arrival = "South of the castle.", cue = true },
  { tx = 304, ty = 213, arrival = "Pick up the bush." },
  { tx = 304, ty = 212, arrival = "Enter the tunnel.", after_lift = true },
},

COURTYARD = {
  note = "The courtyard crossing after Uncle. With the sword in hand Link leaves the uncle room back out to the courtyard, cuts through the two bushes, then enters the castle proper by the door just south of him. The guide leads to each waypoint in turn rather than straight at the door, because the intended path goes through the bushes and routing direct would skip them. Coordinates are world tiles read live from the game. From the first waypoint carrying a `room` the chain is inside the castle and the in-room pathfinder takes over, carrying on to Zelda's cell.",
  { tx = 282, ty = 225 },
  { tx = 256, ty = 225 },
  { tx = 335, ty = 379, room = 0x55, level = 0,
    note = "The sewer room where the uncle is met." },
  { tx = 72, ty = 415, room = 0x61 },
  { tx = 47, ty = 392, room = 0x60, level = 1 },
  { tx = 57, ty = 335, room = 0x50, level = 1 },
  { tx = 95, ty = 11, room = 0x01, level = 1 },
  { room = 0x72, kind = "clear", via = true, gate = {"not", {"chest_opened"}},
    note = "Room 0x72's guard drops the key for its chest, so the fight comes first. Gated on the chest being shut, which is the room's old forced-kill rule stated as data: that bit is permanent, so once the chest is opened this step stops being eligible and a backtrack — which respawns the guard, the room having no clear-tag — never re-arms it." },
  { tx = 159, ty = 472, room = 0x72, level = 0,
    note = "South-door exit, upper floor, after the guard, the key and the chest." },
  { tx = 149, ty = 507, room = 0x72, level = 1,
    note = "Lower floor, reached down those stairs." },
  { tx = 129, ty = 560, room = 0x82, level = 1 },
  { tx = 79, ty = 518, room = 0x81, level = 1 },
  { tx = 104, ty = 495, room = 0x71, level = 1,
    note = "Lower-floor anchor by the chest, where the route to the next room (0x70) and its key-carrying soldier begins." },
  { tx = 79, ty = 486, room = 0x71, level = 0, kind = "gate", gate = {"keys"},
    note = "The locked door itself, up on the UPPER floor (a 2x2 at 79-80,485-486), reached by a clean straight climb up the swap stair and north, with no floor-flip back to L1 and so no wall-cross. Room 0x71 is open enough on the lower floor that the pathfinder can reach the far waypoint without ever crossing this door, so a pure collision block is not enough and the guide has to be told the dependency. gate: only a target once Link holds a key to open it. done: clears once the door's own tile stops reading as locked." },
  { tx = 84, ty = 455, room = 0x71, level = 1, gate = {"at", 79, 486, 0, {"tile_outside", 0xF0, 0xFF}},
    note = "Floor-1 door out of 0x71. gate: not a target until the locked door above (79,486) is actually OPEN, so Link is led to unlock that door first rather than aimed here early, which would force the pathfinder up-and-back through the wall. Once the door is open the way here is clear." },
  { room = 0x70, kind = "clear", via = true,
    note = "Room 0x70 gates the way through on a fight: two guards flank the passage and the eastern one drops the key for the locked door out. The room sets no clear-tag of its own, so nothing in the game says so — this step does. `via` makes it mandatory, which is what keeps the guide on the enemies rather than skipping to the room's exit below; it retires itself once no counting enemy remains." },
  { tx = 10, ty = 452, room = 0x70, level = 0,
    note = "Into room 0x70, once it is quiet." },
  { room = 0x80, kind = "clear", via = true,
    note = "Zelda's cell room, and the enemy in the far east holds the big key, so the whole room is one fight. ROOMS[0x80].giant is what makes its enemies count from across the room rather than just on screen, so the guide leads east to that one instead of going quiet when the nearer guards fall." },
  { tx = 44, ty = 518, room = 0x80,
    note = "Her cell, down the stairs from 0x70 — the rescue." },
},

SANCTUARY = {
  note = "The escort back out: once Zelda is freed the return trip leads up through the castle to the hidden north passage and out to the Sanctuary. Authored room by room by playing it, because the room-graph heuristic just heads for the nearest exit and does not know about the hidden passage. Wired to the \"sanct\" goal.",
  { tx = 10, ty = 516, room = 0x80, level = 0,
    note = "Up out of Zelda's cell room, the start of the climb." },
  { tx = 20, ty = 452, room = 0x70, level = 0,
    note = "Back up in 0x70, starting the climb out." },
  { tx = 79, ty = 503, room = 0x71, level = 1,
    note = "Up into 0x71, the boomerang chest room, lower floor." },
  { tx = 124, ty = 524, room = 0x81, level = 0,
    note = "Up into 0x81, the guardroom above 0x71." },
  { tx = 134, ty = 512, room = 0x82, level = 0,
    note = "Up into 0x82, upper floor." },
  { tx = 159, ty = 455, room = 0x72, level = 0,
    note = "Up into 0x72, upper floor." },
  { tx = 119, ty = 15, room = 0x01, level = 1,
    note = "Up into 0x01, lower floor." },
  { tx = 151, ty = 369, room = 0x52, level = 0, via = true,
    note = "UP over the right-side ledge, and mandatory (via): the escape climbs the stairs and drops back down here to dodge the soldiers on the lower-floor line." },
  { tx = 143, ty = 375, room = 0x52, level = 1,
    note = "Down the stair to 0x52's lower floor, continuing the escape." },
  { tx = 136, ty = 415, room = 0x62, level = 1,
    note = "South into 0x62, lower floor, on the open floor east of the wall." },
  { tx = 95, ty = 389, room = 0x61, level = 0,
    note = "West into 0x61, upper floor." },
  { tx = 91, ty = 326, room = 0x51, level = 0, kind = "push", push = 6, track = 0xEE, track_dx = -2, track_dy = 2,
    note = "The throne-room Movable Mantle (sprite 0xEE), a push waypoint. Tracks the mantle's live sprite, offset (-2,+2) onto its left/push side; push = 6 faces east to shove it. done: the mantle latches sprite_G to 0x90 at its end stop (zelda3 Sprite_EE_MovableMantle), so the tone stops and the chain advances once it is fully pushed. tx/ty are the fallback until the sprite loads. Zelda's dialogue narrates the push." },
  { tx = 117, ty = 260, room = 0x41, level = 0,
    note = "North through the opened passage into 0x41, upper floor." },
  { tx = 159, ty = 261, room = 0x42, level = 0,
    note = "East into 0x42, upper floor." },
  { tx = 176, ty = 218, room = 0x32, level = 0, kind = "chest",
    note = "North into 0x32 to the chest (a 2x2 at 176-177,218-219). done: the chest's own tile stops reading as a chest tile once opened — a direct signal, unlike the $7EF000 chest-opened bit, which is not set for this room." },
  { tx = 159, ty = 197, room = 0x32, level = 0, kind = "gate", gate = {"any", {"tile_outside", 0xF0, 0xFF}, {"keys"}},
    note = "The locked door north (a 2x2 at 159-160,197-198). gate: an OPEN door is always a target, so the guide leads Link through it; a still-locked door is a target only once he holds a small key, otherwise the guide stays on the chest, which holds that key, rather than a door he cannot open." },
  { tx = 131, ty = 175, room = 0x22, level = 0,
    note = "North into 0x22, upper floor." },
  { tx = 111, ty = 133, room = 0x21, level = 0, kind = "gate", gate = {"any", {"tile_outside", 0xF0, 0xFF}, {"keys"}},
    note = "West into 0x21, then the locked door north (a 2x2 at 111-112,133-134). Same open-or-keyed gate as 0x32; here the key-holder rat drops the key." },
  { tx = 111, ty = 76, room = 0x11, level = 0, kind = "push", push = 0,
    note = "North into 0x11 to the dungeon push-block (tile 0x76 at 111-112,76-77). A TILE push obstacle with no sprite to track, so push = 0 to face north is all that drives the alignment tone. done: once shoved, its tile stops reading as a push-block, so the tone goes silent when the block can move no further." },
  { tx = 111, ty = 68, room = 0x11, level = 0, gate = {"at", 111, 76, 0, {"tile_outside", 0x70, 0x7F}},
    note = "North through where the block was, further into 0x11. Gated on the push-block at (111,76) being SHOVED: the way north is a dead end until then, and the pathfinder can slip around the block on the open floor beside it, so a collision block alone would not hold the guide back — the block's pushed state is the real gate." },
},

}

ROOMS = {

[0x71] = {
  chambers = {{n = 491, e = 90, s = 506, w = 69}, {n = 487, e = 122, s = 506, w = 101}},
  note = "Not a forced kill-room — it carries its own clear-tag. Two guard pits side by side, every edge on a green ledge wall, walled off from each other: an enemy in one pit cannot be reached from the other, which is why an objective has to be checked for reachability before the guide commits to it. Its chambers bound the enemy tally: a guard in the far pit does not count from the near one, and cannot be targeted through the wall between them.",
},

[0x72] = {
  chambers = {{n = 458, e = 166, s = 474, w = 153}},
  note = "Its fight is a `clear` step in the COURTYARD chain, gated on the chest being shut. What stays here is the chamber: one walled fighting floor, regular walls rather than green ledges, so the box hugs the dark floor inside (cols 153-166, rows 458-474).",
},

[0x80] = {
  chambers = {{n = 512, e = 63, s = 575, w = 0}},
  note = "The jail-cell room, and its fight is a `clear` waypoint in the COURTYARD chain. Its chamber is the whole room — room 0x80 sits at tile column 0, row 8 — which is how the far-east big-key holder still counts from anywhere in it. That used to be a `giant` flag widening a radius; saying the chamber is the room says the same thing without a second mechanism.",
},

}
