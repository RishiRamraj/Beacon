-- The Legend of Zelda: A Link to the Past — Beacon plugin.
--
-- Reads Link's state each frame and proposes what might be worth saying. It
-- proposes only: the host arbiter decides what is actually spoken, so this is
-- free to be generous. Being wrong about relevance here is cheaper than every
-- plugin reimplementing suppression badly.
--
-- This is a port of the native alttp.rs stand-in, which was itself written from
-- the alttp-navi proof of concept. The addresses come from the manifest's
-- [watch] table so the numbers live in one place.

local A = watch

-- Health is stored in eighths of a heart.
local function hearts(eighths)
  return eighths / 8.0
end

local function module_name(m)
  local names = {
    [0x00] = "intro",
    [0x01] = "file select",
    [0x02] = "copy file",
    [0x03] = "erase file",
    [0x04] = "name file",
    [0x05] = "loading game",
    [0x06] = "entering dungeon",
    [0x07] = "dungeon",
    [0x08] = "entering overworld",
    [0x09] = "overworld",
    [0x0e] = "menu",
    [0x12] = "death",
    [0x14] = "attract mode",
    [0x19] = "triforce room",
  }
  return names[m] or "unknown"
end

-- Modules whose entry is not announced by the generic module-change callout:
-- the text box (spoken separately), the two in-play modules (obvious, and the
-- room/area callout covers location), the loading/transition modules between them
-- (the destination speaks for itself), and the non-interactive title screens.
local MODULE_SILENT = {
  [0x00] = true, -- intro
  [0x06] = true, -- entering dungeon (transition)
  [0x07] = true, -- dungeon (in play)
  [0x08] = true, -- entering overworld (transition)
  [0x09] = true, -- overworld (in play)
  [0x0e] = true, -- text box
  [0x14] = true, -- attract mode
}

-- The four facings (Link's $7E002F direction byte): compass word and unit vector,
-- in one table so direction->word and direction->vector are not re-encoded as inline
-- ladders (used by facing(), the map arrow, and the route follower's alignment).
-- `dpad` is this facing's bit in the held-controller register $7E00F0 (low nibble
-- U 0x08 / D 0x04 / L 0x02 / R 0x01), so "is the player pressing this way" is one AND.
local DIRS = {
  [0] = { word = "north", dx = 0, dy = -1, dpad = 0x08 },
  [2] = { word = "south", dx = 0, dy = 1, dpad = 0x04 },
  [4] = { word = "west", dx = -1, dy = 0, dpad = 0x02 },
  [6] = { word = "east", dx = 1, dy = 0, dpad = 0x01 },
}

local function facing(direction)
  local d = DIRS[direction]
  return d and d.word or "east"
end

-- One frame's reading of the game.
local function read_state()
  local module = mem.u8(A.module.addr)
  if module == nil then return nil end
  return {
    module = module,
    -- Non-zero while a transition or animation is in progress.
    submodule = mem.u8(A.submodule.addr),
    health = mem.u8(A.health.addr),
    max_health = mem.u8(A.max_health.addr),
    rupees = mem.u16(A.rupees.addr),
    x = mem.u16(A.link_x.addr),
    y = mem.u16(A.link_y.addr),
    direction = mem.u8(A.direction.addr),
    indoors = mem.u8(A.indoors.addr),
    dungeon_room = mem.u16(A.dungeon_room.addr),
    ow_screen = mem.u16(A.ow_screen.addr),
    world = mem.u8(A.world.addr),
    dungeon_id = mem.u8(A.dungeon_id.addr),
  }
end

-- The quest-progress bytes the objective logic reads. Kept separate from
-- read_state's moment-to-moment fields since it is only consulted on demand.
local function read_progress()
  local p = mem.u8(A.progress.addr)
  if p == nil then return nil end
  return {
    progress = p,
    pendants = mem.u8(A.pendants.addr),
    crystals = mem.u8(A.crystals.addr),
    sword = mem.u8(A.sword.addr),
  }
end

-- Whether the player is actually controlling Link, as opposed to sitting in a
-- menu, a transition, or the intro.
local function in_play(s)
  return (s.module == 0x07 or s.module == 0x09) and s.submodule == 0
end

-- Fraction of maximum health below which the low-health warning fires.
local LOW_HEALTH_FRACTION = 0.3

-- State kept between frames. The Lua state persists for the life of the plugin,
-- so upvalues are the natural home for it.
local prev = nil
-- Latched so the warning fires on crossing the threshold, not every frame below.
local low_health_warned = false
-- Latched so navigation auto-starts once when Link gains control at the very start
-- of the quest, not every frame he is in his house. Cleared when he leaves that
-- opening context, so it re-arms on a fresh start and a manual toggle-off stays off.
local intro_nav_armed = false

-- Sprite table: 16 slots of active objects and enemies. Addresses from the
-- well-documented ALttP RAM map, verified against the running game. Each slot's
-- fields are 16 consecutive bytes indexed by slot number.
local SPRITE = {
  state = 0x7E0DD0, -- 0 = inactive
  kind  = 0x7E0E20, -- sprite type id
  x_lo  = 0x7E0D10,
  x_hi  = 0x7E0D30,
  y_lo  = 0x7E0D00,
  y_hi  = 0x7E0D20,
  hp    = 0x7E0E50,
}

-- Sprite classification tables (names, enemy/item/npc sets) live in data.lua under
-- the shared REF namespace — bulky reference data kept out of this chunk's local
-- budget. Read them as REF.sprite_names / REF.enemy_types / REF.item_types /
-- REF.npc_types. Names are the ALttP disassembly's sprite-prep dispatch, the
-- authoritative meaning of the $7E0E20 type.

local function sprite_name(kind)
  return REF.sprite_names[kind] or (REF.enemy_types[kind] and "enemy" or "object")
end

-- Whether a sprite is a threat. Damageable (has health) OR a known enemy type:
-- the type table is not exhaustive, so health is what catches the rest.
local function is_enemy(sp)
  return (sp.hp ~= nil and sp.hp > 0) or REF.enemy_types[sp.kind] == true
end

-- Whether a sprite is live enough to draw on the map: it has health, or it is a
-- pickup / person that carries no health by design (items, NPCs, switches — things
-- to walk to). A spent sprite that is hp 0 and neither is dead or inert (a defeated
-- enemy, a projectile, incidental scenery) and just clutters the map. Princess Zelda
-- and other story NPCs are hp 0 but classified as NPCs, so they stay.
local function is_live(sp)
  return (sp.hp ~= nil and sp.hp > 0) or REF.item_types[sp.kind] or REF.npc_types[sp.kind]
end

-- What to call an enemy: its type name only when the type is a classified enemy,
-- otherwise just "enemy" — a damageable sprite the table does not name is still a
-- threat, and a wrong name would be worse than none.
local function enemy_name(sp)
  if REF.enemy_types[sp.kind] then
    return REF.sprite_names[sp.kind] or "enemy"
  end
  return "enemy"
end

-- Reads the active sprites, nearest first, each with slot, position, offset from
-- Link, Manhattan distance, type, and health.
local function sprites()
  local s = prev
  if s == nil or not in_play(s) then return {} end
  local out = {}
  for i = 0, 15 do
    local st = mem.u8(SPRITE.state + i)
    -- Skip state 0x0A ("carried"): an object Link is holding over his head — a
    -- lifted pot, bush, or rock — rides on him, so it is not a thing in the world to
    -- beacon, map, or scan. (Sprite state jump table: 0x0A = SpriteModule_Carried.)
    if st ~= nil and st ~= 0 and st ~= 0x0A then
      local sx = mem.u8(SPRITE.x_lo + i) + mem.u8(SPRITE.x_hi + i) * 256
      local sy = mem.u8(SPRITE.y_lo + i) + mem.u8(SPRITE.y_hi + i) * 256
      local dx, dy = sx - s.x, sy - s.y
      out[#out + 1] = {
        slot = i,
        x = sx, y = sy, dx = dx, dy = dy,
        dist = math.abs(dx) + math.abs(dy),
        kind = mem.u8(SPRITE.kind + i),
        hp = mem.u8(SPRITE.hp + i),
      }
    end
  end
  table.sort(out, function(a, b) return a.dist < b.dist end)
  return out
end

-- Whether an offset falls within the visible screen (256x224, Link near centre).
local function on_screen(dx, dy)
  return math.abs(dx) <= 128 and math.abs(dy) <= 116
end

-- ── Kill-rooms ──────────────────────────────────────────────────────────────
-- Some dungeon rooms lock progress until their enemies are defeated. The room's
-- two header tag bytes ($7E00AE/$AF) say how: derived from the ALttP room-tag
-- routines (zelda3src dungeon.c), tags 0x01-0x0A open doors or drop trapdoors on
-- clear, 0x26 removes a blocking statue, and 0x29-0x32 reveal a chest. When one is
-- set and enemies remain, the guide leads to the next enemy instead of a (locked)
-- door and states the requirement.
local KILL_HDR_TAG = 0x7E00AE       -- two bytes: tag[0], tag[1]
local SPRITE_FLAGS4 = 0x7E0F60      -- bit 0x40 set = ignored by the room-clear check
local OVERLORD_TYPE = 0x7E0B00      -- spawner overlords; 0x14/0x18 hold the room open
local KILL_TAGS = {}
for _, t in ipairs({ 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A,
                     0x26, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32 }) do
  KILL_TAGS[t] = true
end

-- Per-room permanent progress bits live at $7EF000 + room*2 (one u16 per dungeon
-- room). Bit 0x8000 flips when the room's chest is opened and never clears.
local function room_chest_opened(room)
  return (mem.u16(0x7EF000 + room * 2) & 0x8000) ~= 0
end

-- Rooms to treat as kill-rooms even though the game sets no clear-tag on them: a
-- guard there drops a small key for the locked exit, so the guide should lead Link
-- to defeat it first. Keyed by dungeon room id; the value is a predicate `(s) ->
-- bool` for whether the room is a kill-room right now, so each room states its own
-- rule as data rather than a shared type/field ladder. Only matters while enemies
-- are live — the objective also requires a pending enemy.
local FORCE_KILL_ROOMS = {
  -- 0x72: forced-kill only until its chest is opened. That bit is permanent, so a
  -- backtrack (which respawns the guard, since the room has no clear-tag) never
  -- re-arms the sub-goal, and the room's reuse for the lower area is never a
  -- kill-room.
  [0x72] = function(s) return not room_chest_opened(s.dungeon_room) end,
  -- 0x70: two guards flank the way through; the eastern one drops the key for the
  -- locked door out, so the first objective is to defeat them. No clear-tag, so force
  -- it. Always on: the kill objective self-clears once no counting enemy remains, and
  -- the escape runs one-way forward, so a backtrack re-arm never arises in practice.
  [0x70] = function(_) return true end,
  -- 0x80: the jail-cell room. The enemy in the far east holds the big key, so the
  -- whole room is one kill objective — a "giant" kill-room (see GIANT_KILL_ROOMS)
  -- whose enemies count across the whole screen, not just the near ones, so the
  -- guide leads Link east to that enemy instead of dropping the objective once the
  -- nearer guards fall. No clear-tag, so force it; the escape is one-way forward.
  [0x80] = function(_) return true end,
}

-- Kill-rooms whose fighting area is the whole room, not just what is on screen.
-- In an ordinary kill-room the enemy tally only counts sprites within ~one screen
-- of Link (ENEMY_ONSCREEN), so a sprite loaded from an adjacent room can't hold the
-- room "uncleared" forever. A giant kill-room deliberately spans the whole 512-pixel
-- room: a key-holder waiting at the far side must still count from across it, so the
-- tally uses a room-sized reach (ENEMY_INROOM) here. Keyed by dungeon room id.
local GIANT_KILL_ROOMS = { [0x80] = true }

-- Debug map only: the tile bounds of a kill-room's fighting pits, in world tiles,
-- each drawn as a 1px rectangle. A dungeon room is one 64-tile block, but the parts
-- that actually gate progress are smaller chambers walled off by green ledge tiles;
-- a room can hold several such pits, so each room maps to a LIST of boxes and the
-- overlay outlines them all. Edges (n/e/s/w = north/east/south/west) are read off
-- the green walls so an outline hugs its pit rather than framing the whole screen.
--   0x71: two guard pits side by side, every edge on a green ledge wall.
--   0x72: one walled fighting chamber (regular walls, not green ledges); the outline
--         hugs the dark floor inside — cols 153-166, rows 458-474.
local KILL_REGION = {
  [0x71] = {
    { n = 491, e = 90,  s = 506, w = 69 },  -- west pit
    { n = 487, e = 122, s = 506, w = 101 }, -- east pit
  },
  [0x72] = {
    { n = 458, e = 166, s = 474, w = 153 }, -- the central chamber floor
  },
}

-- Is Link in a dungeon room gated on defeating enemies?
local function kill_room(s)
  if s.module ~= 0x07 then return false end
  local fk = FORCE_KILL_ROOMS[s.dungeon_room]
  if fk and fk(s) then return true end
  return KILL_TAGS[mem.u8(KILL_HDR_TAG)] == true or KILL_TAGS[mem.u8(KILL_HDR_TAG + 1)] == true
end

-- The nearest live enemy still counting toward the room clear (state set, and not
-- flagged out of the tally, matching Sprite_CheckIfRoomIsClear), or nil. Overlord
-- spawners (0x14/0x18) also hold a room; reported separately since they have no
-- position to walk to.
-- Half-screen reach for the room-clear check: a sprite loaded from an adjacent
-- room, or walled off in another part of the room, sits well outside the visible
-- screen and must not hold the room "uncleared" forever. Kept generous (a bit
-- past the 256x224 screen) so an on-screen enemy near a room edge still counts.
local ENEMY_ONSCREEN = 144
-- A giant kill-room counts enemies across the whole 512-pixel room, so a key-holder
-- waiting at the far side still registers from anywhere in the room.
local ENEMY_INROOM = 512

local function nearest_pending_enemy(s)
  local reach = GIANT_KILL_ROOMS[s.dungeon_room] and ENEMY_INROOM or ENEMY_ONSCREEN
  local best, bd
  for i = 0, 15 do
    local st = mem.u8(SPRITE.state + i)
    -- hp 0 is dead or inert (or a bystander NPC like caged Zelda) — never a pending
    -- enemy, so it can't hold a room "uncleared", especially a giant kill-room whose
    -- wide reach would otherwise sweep such a sprite in from across the room.
    if st ~= nil and st ~= 0 and (mem.u8(SPRITE_FLAGS4 + i) & 0x40) == 0
        and (mem.u8(SPRITE.hp + i) or 0) > 0 then
      local sx = mem.u8(SPRITE.x_lo + i) + mem.u8(SPRITE.x_hi + i) * 256
      local sy = mem.u8(SPRITE.y_lo + i) + mem.u8(SPRITE.y_hi + i) * 256
      if math.abs(sx - s.x) <= reach and math.abs(sy - s.y) <= reach then
        local d = math.abs(sx - s.x) + math.abs(sy - s.y)
        if bd == nil or d < bd then best, bd = { sx, sy }, d end
      end
    end
  end
  return best
end

local function overlords_pending()
  for i = 0, 7 do
    local t = mem.u8(OVERLORD_TYPE + i)
    if t == 0x14 or t == 0x18 then return true end
  end
  return false
end

-- Link is "in combat" while an enemy is this close. Then the guide hushes and only
-- the nearest enemy sounds, so a fight is not cluttered by navigation or pickups.
local COMBAT_RANGE = 48
-- Set each frame: whether an enemy is within COMBAT_RANGE. Global so the guide
-- followers can fall silent while it holds, and for MCP inspection.
combat_engaged = false

-- A compass direction from an offset. y decreases upward on the SNES.
local function direction(dx, dy)
  local ax, ay = math.abs(dx), math.abs(dy)
  local ns = dy < 0 and "north" or "south"
  local ew = dx < 0 and "west" or "east"
  if ax > 2 * ay then return ew
  elseif ay > 2 * ax then return ns
  else return ns .. "-" .. ew end
end

-- A rough distance word. Roughly 16 pixels to a tile.
local function proximity(dist)
  if dist < 24 then return "right beside you"
  elseif dist < 64 then return "close"
  elseif dist < 160 then return "nearby"
  else return "in the distance" end
end

-- Beacon categories. Every visible object falls into one class, and the nearest
-- of each class gets a spatial-audio tone — so the soundscape stays legible: one
-- distinct pitch per class rather than a wall of sound. What matters carries
-- further: enemies and things worth walking to (items, chests, people, switches)
-- call from across the screen; incidental scenery only chirps when Link is right
-- on top of it.
--
-- Types you collect or open (REF.item_types, a bright high tone) and people to
-- talk to or switches to act on (REF.npc_types, interactable but not picked up) —
-- both in data.lua.

-- Per-class tone, reach, and pulse. `pitch` scales the 330 Hz base tone (higher
-- is brighter); enemies keep the original 1.0. `range` is Manhattan pixels —
-- about 16 to a tile, so 24 is "within a block", the near-only reach for scenery.
-- `tremolo` is the amplitude-swell rate in Hz: a rhythmic signature that tells the
-- classes apart by ear even when they overlap. Danger swells fast, reward slow:
-- enemies pulse at 2 Hz (120 BPM), the things you collect — items and chests — at
-- 1 Hz (60 BPM), and incidental scenery sits steady. The guide tone carries no
-- swell at all (see the path beacon), so the thing you actively steer by is a
-- solid tone, never mistaken for a threat or a pickup.
-- `gain` scales the class's loudness on top of the distance falloff (clamped to
-- 1.0). Enemies carry a boost so a threat is heard over the quieter guide tone;
-- the calmer classes sit at unity.
local BEACON_KINDS = {
  enemy = { pitch = 1.0, range = 224, tremolo = 2.0, gain = 1.6 }, -- 120 BPM: danger
  item  = { pitch = 2.0, range = 224, tremolo = 1.0, gain = 1.0 }, -- 60 BPM: a pickup
  npc   = { pitch = 1.5, range = 224, tremolo = 1.0, gain = 1.0 }, -- 60 BPM: safe to approach
  minor = { pitch = 0.5, range = 24,  tremolo = 0.0, gain = 1.0 }, -- steady, incidental
}

-- A treasure chest is a fixed landmark, not a loose pickup you chase down, so it
-- sounds the item tone at a lower volume — present but not competing with the guide
-- and the pickups. Same pitch/pulse as an item so it still reads as "a good thing".
local CHEST_BEACON = { pitch = 2.0, range = 224, tremolo = 1.0, gain = 0.5 }

-- NPC types the guide leads Link to as a quest objective rather than ambient people
-- to pass by. The navigation guide already homes on them, so also sounding the "safe
-- to approach" NPC tone puts two cues on one target and muddies which to follow —
-- these are kept off the NPC beacon (they still show on the map and in a scan).
local BEACON_SKIP_NPC = { [118] = true } -- Princess Zelda (the rescue objective)

-- How much a wall between the player and a source dims its beacon: muffled, not
-- silenced, so an occluded threat still registers.
local BEACON_OCCLUDED_SCALE = 0.35

-- Which beacon class a sprite belongs to. Enemies first (a damageable sprite is a
-- threat whatever the type table calls it), then the interactable classes, and
-- everything else is incidental scenery.
local function category(sp)
  if is_enemy(sp) then return "enemy"
  elseif REF.item_types[sp.kind] then return "item"
  elseif REF.npc_types[sp.kind] then return "npc"
  else return "minor" end
end

-- Game text: decode ALttP's compressed dialogue table from the ROM once at load,
-- then read the current message by id at runtime. Ported from the alttp-navi
-- proof of concept. The data tables below are generated from its decoder and
-- must not be hand-edited.
local ALPHABET = { "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "!", "?", "-", ".", ",", "...", ">", "(", ")", "", "", "", "", "", "\"", "", "", "", "", "'", "", "", "", "", "", "", "", " ", "<", "", "", "", "" }
local DICTIONARY = { "    ", "   ", "  ", "'s ", "and ", "are ", "all ", "ain", "and", "at ", "ast", "an", "at", "ble", "ba", "be", "bo", "can ", "che", "com", "ck", "des", "di", "do", "en ", "er ", "ear", "ent", "ed ", "en", "er", "ev", "for", "fro", "give ", "get", "go", "have", "has", "her", "hi", "ha", "ight ", "ing ", "in", "is", "it", "just", "know", "ly ", "la", "lo", "man", "ma", "me", "mu", "n't ", "non", "not", "open", "ound", "out ", "of", "on", "or", "per", "ple", "pow", "pro", "re ", "re", "some", "se", "sh", "so", "st", "ter ", "thin", "ter", "tha", "the", "thi", "to", "tr", "up", "ver", "with", "wa", "we", "wh", "wi", "you", "Her", "Tha", "The", "Thi", "You" }
local CMD_LENGTHS = { 1, 1, 1, 1, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 1, 1, 1, 1, 1 }
local CMD_NAMES = { "NextPic", "Choose", "Item", "Name", "Window", "Number", "Position", "ScrollSpd", "Selchg", "Unused_Crash", "Choose3", "Choose2", "Scroll", "1", "2", "3", "Color", "Wait", "Sound", "Speed", "Unused_Mark", "Unused_Mark2", "Unused_Clear", "Waitkey" }
local ROM_ADDRS = { 0x9C8000, 0x8EDF40 }

-- WRAM $7E1CF0 holds the id of the message currently displayed.
local DIALOG_ID = 0x7E1CF0

local function snes_to_rom(snes)
  local bank = (snes >> 16) & 0x7F
  local off = snes & 0xFFFF
  return bank * 0x8000 + (off - 0x8000)
end

local function normalize(s)
  s = s:gsub("%s+", " ")
  return s:match("^%s*(.-)%s*$")
end

-- Decodes every message into a table keyed by message id (0-based), matching the
-- dialog id read from WRAM at runtime.
local function decode_dialog()
  if rom.size == 0 then return {} end
  local data = rom.slice(0, rom.size) -- whole ROM as a byte string, read once
  local function byte(pos) return string.byte(data, pos + 1) end -- 0-based

  local messages = {}
  local id = 0
  local addr_idx = 1
  local pos = snes_to_rom(ROM_ADDRS[addr_idx])
  local current = {}

  while pos < rom.size do
    local b = byte(pos)
    pos = pos + 1
    if b == nil then break end

    if b == 0xFF then -- end of all dialogue
      if #current > 0 then messages[id] = normalize(table.concat(current)) end
      break
    elseif b == 0x7F then -- end of one message
      messages[id] = normalize(table.concat(current))
      id = id + 1
      current = {}
    elseif b == 0x80 then -- switch ROM bank
      addr_idx = addr_idx + 1
      if addr_idx <= #ROM_ADDRS then
        pos = snes_to_rom(ROM_ADDRS[addr_idx])
      else
        break
      end
    elseif b <= 0x5E then -- alphabet character
      current[#current + 1] = ALPHABET[b + 1] or ""
    elseif b >= 0x67 and b <= 0x7E then -- command byte
      local cmd_idx = b - 0x67
      local name = CMD_NAMES[cmd_idx + 1]
      if name == "Name" then
        current[#current + 1] = "Link"
      elseif name == "1" or name == "2" or name == "3" or name == "Scroll" then
        current[#current + 1] = " "
      end
      if CMD_LENGTHS[cmd_idx + 1] == 2 then pos = pos + 1 end -- skip parameter
    elseif b >= 0x88 then -- dictionary entry
      current[#current + 1] = DICTIONARY[b - 0x88 + 1] or ""
    end
    -- bytes 0x5F-0x66 and 0x81-0x87 are unused; skip
  end
  return messages
end

-- Global (in this plugin's own Lua state) so it can be inspected with eval_lua
-- when developing or debugging.
dialog = decode_dialog()

-- The message currently displayed, or nil if none / not decoded.
local function current_dialog_text()
  local did = mem.u16(DIALOG_ID)
  if did == nil then return nil end
  local text = dialog[did]
  if text and text ~= "" then return text end
  return nil
end

-- The map's collision colours. A tile attribute describes what a tile *is* for
-- collision; only a few classes are worth drawing, and the rest is open floor,
-- left as background. Ported from the tile classes in alttp-navi's map_renderer.
local TILE_COLOR = {}
do
  local function fill(color, ids)
    for _, a in ipairs(ids) do TILE_COLOR[a] = color end
  end
  fill(0x5A6478, { 0x01, 0x02, 0x03, 0x0B, 0x26, 0x43, 0x6C, 0x6D, 0x6E, 0x6F }) -- wall / cliff
  fill(0x2C6AC0, { 0x08, 0x09, 0x4B })                                           -- water
  fill(0x0A0E16, { 0x20 })                                                       -- hole / pit
  fill(0x50A070, { 0x1C, 0x1D, 0x1E, 0x1F, 0x22, 0x28, 0x29, 0x2A, 0x2B })       -- ledge / stairs
  fill(0xE0C040, { 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37 })             -- door / passage
  fill(0x9C6B3C, { 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56 })                   -- solid object
end
-- Indoors, attribute 0x04 (and the rest of the indoor-wall set, already walls
-- above) is a wall; outdoors the same value is diggable ground. So it is folded
-- in per-context, not into the shared table.
local INDOOR_WALL_04 = 0x5A6478

-- Walls and cliffs block line of sight; water, pits, doors, and floors do not.
-- (Indoors, 0x04 is also a wall — handled per-context in sight_blocked.)
local SIGHT_BLOCKERS = {}
for _, a in ipairs({ 0x01, 0x02, 0x03, 0x0B, 0x26, 0x43, 0x6C, 0x6D, 0x6E, 0x6F }) do
  SIGHT_BLOCKERS[a] = true
end

-- Dungeon collision map. $7F2000 holds a 64x64 grid, one byte per 8-pixel tile —
-- live WRAM, so the room's real shape. The lower level of a two-level room lives
-- 0x1000 further on.
local DUNGEON_TILE_TABLE = 0x7F2000
local LOWER_LEVEL = 0x7E00EE
local OW_TILE_TABLE = 0x7E2000 -- overworld map16 indices, live WRAM

-- Overworld collision map. The visible tiles are map16 indices in the $7E2000
-- WRAM table; each index resolves through two ROM tables to a collision
-- attribute. Loaded once here (the ROM does not change); ported from alttp-navi's
-- rom parser. The whole-ROM `snes_to_rom` mapping is the one already used above
-- for dialogue.
local OW_MAP16_TO_MAP8 = rom.slice(snes_to_rom(0x8F8000), 3752 * 4 * 2) -- uint16 LE
local OW_MAP8_TO_ATTR = rom.slice(snes_to_rom(0x8E9459), 512)           -- uint8

-- Resolve a map16 tile index to its collision attribute. `x` is in 8-pixel tile
-- units and `y` in pixels; their low bits pick which of the map16's four 8x8
-- sub-tiles applies. Global so it can be checked with eval_lua against the
-- reference decoder, like `dialog`.
function ow_tile_attr(map16_index, x, y)
  if #OW_MAP16_TO_MAP8 == 0 or #OW_MAP8_TO_ATTR == 0 then return 0 end
  local t = (map16_index * 4) | ((y & 8) >> 2) | (x & 1)
  local i = t * 2
  if i < 0 or i + 2 > #OW_MAP16_TO_MAP8 then return 0 end
  local map8 = string.byte(OW_MAP16_TO_MAP8, i + 1) | (string.byte(OW_MAP16_TO_MAP8, i + 2) << 8)
  local idx = map8 & 0x1FF
  if idx + 1 > #OW_MAP8_TO_ATTR then return 0 end
  local rv = string.byte(OW_MAP8_TO_ATTR, idx + 1)
  if rv >= 0x10 and rv < 0x1C then
    rv = rv | ((map8 >> 14) & 1)
  end
  return rv
end

-- The collision attribute of the tile containing world pixel (px, py) in the
-- Overworld bushes read as a solid collision attribute (0x50, shared with walls),
-- but Link's sword cuts them, so the router should pass straight through. They are
-- identifiable only by their map16 tile id — the same one the game's own bush
-- check keys on — so tile_attr_at reports them as a distinct passable BUSH_TILE,
-- above the real 0x00-0xFF attribute range, that the pathfinder crosses and the
-- guide can flag ("slash the bush").
local BUSH_MAP16 = { [0x036] = true, [0x72A] = true }
local BUSH_TILE = 0x1B0

-- current area, or nil if it cannot be read. Dungeons index the WRAM grid
-- directly; the overworld goes through the same scroll-offset + ROM decode the
-- map render uses.
local function tile_attr_at(s, px, py, level)
  if s.module == 0x07 then
    -- `level` overrides Link's current floor ($7E00EE) so a route can be planned on
    -- the other floor of a two-level room (its grid lives 0x1000 further on).
    local lvl = level ~= nil and level or mem.u8(LOWER_LEVEL)
    local base = DUNGEON_TILE_TABLE + (lvl == 1 and 0x1000 or 0)
    return mem.u8(base + ((py >> 3) & 63) * 64 + ((px >> 3) & 63))
  elseif s.module == 0x09 and #OW_MAP16_TO_MAP8 > 0 then
    local mask_y, mask_x = mem.u16(0x7E070A), mem.u16(0x7E070E)
    if mask_x == 0 or mask_y == 0 then return nil end
    local ow_tx = px >> 3
    local t = (((py - mem.u16(0x7E0708)) & mask_y) * 8) | ((ow_tx - mem.u16(0x7E070C)) & mask_x)
    local byte_off = (t >> 1) * 2
    local lo, hi = mem.u8(OW_TILE_TABLE + byte_off), mem.u8(OW_TILE_TABLE + byte_off + 1)
    if lo == nil or hi == nil then return nil end
    local m16 = lo | (hi << 8)
    if BUSH_MAP16[m16] then return BUSH_TILE end
    return ow_tile_attr(m16, ow_tx, py)
  end
  return nil
end

-- Cue the player to slash a bush the guide is leading them into. The router treats
-- bushes as passable (the sword cuts them), but the player still has to swing to
-- get through, so when Link faces a bush tile while the assist is on, say so once,
-- resetting when he faces open ground again. Global `nav_active` gates it so the
-- cue only sounds while actually being guided.
local bush_cued = false
local function bush_cue(s)
  if s == nil or s.module ~= 0x09 or not nav_active then bush_cued = false; return end
  local dir = s.direction
  local ax = s.x + 8 + (dir == 4 and -12 or dir == 6 and 12 or 0)
  local ay = s.y + 12 + (dir == 0 and -12 or dir == 2 and 12 or 0)
  if tile_attr_at(s, ax, ay) == BUSH_TILE then
    if not bush_cued then
      say("Bush.", { priority = "navigation", category = "on-demand" })
      bush_cued = true
    end
  else
    bush_cued = false
  end
end

-- ===========================================================================
-- Full-overworld collision from ROM. The live $7E2000 table only holds the
-- loaded screens, so to route to a distant objective we decode any area's map16
-- layout straight from the cartridge: two LZ2-compressed blobs per area -> 256
-- map32 indices -> map16 via the corner tables -> the same map16->map8->attr
-- decode ow_tile_attr already does. Verified byte-for-byte against the live
-- $7E2000 table (1022/1024 cells; the 2 diffs were a runtime door overlay).
-- Areas are decoded on first use and cached; the whole ROM is sliced lazily,
-- since a player who never routes on the overworld need not pay for it.
-- ===========================================================================
local OW_ROM = nil                              -- whole cart (compressed blobs are scattered)
local OW_PTR_HI, OW_PTR_LO = 0x1794D, 0x17B2D   -- map32 blob pointer tables (PC), 3-byte SNES ptrs
local OW_CORNER = { 0x18000, 0x1B400, 0x20000, 0x23400 } -- map32->map16 corner tables TL TR BL BR (PC)
local ow_area_cache = {}

local function ow_rb(o) return string.byte(OW_ROM, o + 1) end

-- The overworld LZ2 variant (command 4's back-reference is a big-endian absolute
-- index into the output). Terminator 0xFF; header top 3 bits command, low 5 bits
-- length-1, with the 111 escape carrying a 10-bit length.
local function ow_lz2(p)
  local out = {}
  while true do
    local h = ow_rb(p); if h == 0xFF then break end
    local c = h >> 5; local l = h & 0x1F
    if c == 7 then c = (h >> 2) & 7; l = ((h & 3) << 8) | ow_rb(p + 1); p = p + 1 end
    l = l + 1
    if c == 0 then for j = 0, l - 1 do out[#out + 1] = ow_rb(p + 1 + j) end; p = p + l + 1
    elseif c == 1 then local v = ow_rb(p + 1); for j = 0, l - 1 do out[#out + 1] = v end; p = p + 2
    elseif c == 2 then local a, b2 = ow_rb(p + 1), ow_rb(p + 2); for j = 0, l - 1 do out[#out + 1] = (j % 2 == 0) and a or b2 end; p = p + 3
    elseif c == 3 then local v = ow_rb(p + 1); for j = 0, l - 1 do out[#out + 1] = (v + j) & 0xFF end; p = p + 2
    elseif c == 4 then local f = (ow_rb(p + 1) << 8) | ow_rb(p + 2); for j = 0, l - 1 do out[#out + 1] = out[f + 1 + j] end; p = p + 3
    else return nil end
    if #out > 4096 then return nil end
  end
  return out
end

-- Expand a map32 index into one of its four map16 corners (cn: 1 TL, 2 TR, 3 BL,
-- 4 BR). Six ROM bytes encode four map32s: four low bytes then two packed nibbles.
local function ow_map16(t, cn)
  local C = OW_CORNER[cn]; local g = t >> 2; local k = t & 3; local bs = C + g * 6
  local lo = ow_rb(bs + k); local hib = ow_rb(bs + 4 + (k >> 1))
  local hn = ((k & 1) == 0) and ((hib >> 4) & 0xF) or (hib & 0xF)
  return lo | (hn << 8)
end

-- The 256 map32 indices (a 16x16 grid) of one overworld area, decoded and cached.
local function ow_area(area)
  local cached = ow_area_cache[area]; if cached then return cached end
  if OW_ROM == nil then OW_ROM = rom.slice(0, 0x100000) end
  local function r24(o) return ow_rb(o) | (ow_rb(o + 1) << 8) | (ow_rb(o + 2) << 16) end
  local hi = ow_lz2(snes_to_rom(r24(OW_PTR_HI + 3 * area)))
  local lo = ow_lz2(snes_to_rom(r24(OW_PTR_LO + 3 * area)))
  if hi == nil or lo == nil then return nil end
  local m = {}
  for n = 0, 255 do m[n] = ((hi[n + 1] or 0) << 8) | (lo[n + 1] or 0) end
  ow_area_cache[area] = m
  return m
end

-- Collision attribute at absolute overworld pixel (px, py) in world w (0 light, 1
-- dark), decoded from ROM — the cross-screen counterpart of tile_attr_at.
local function ow_rom_attr(w, px, py)
  local area = w * 0x40 + (py >> 9) * 8 + (px >> 9)
  local m = ow_area(area); if m == nil then return nil end
  local lx, ly = (px & 0x1FF) >> 4, (py & 0x1FF) >> 4
  local n = (ly >> 1) * 16 + (lx >> 1); local cn = 1 + (lx & 1) + ((ly & 1) << 1)
  local m16 = ow_map16(m[n], cn)
  if BUSH_MAP16[m16] then return BUSH_TILE end -- Link cuts bushes; route through
  return ow_tile_attr(m16, px >> 3, py)
end

-- Whether a wall lies on the straight line between two world points, so a sprite
-- behind it is out of sight. Walks the 8-pixel tiles the segment crosses
-- (Bresenham), skipping the two endpoint tiles — Link's own tile and the
-- sprite's never count as occluders. Unknown tiles do not block.
local function sight_blocked(s, x0, y0, x1, y1)
  local indoors = (s.module == 0x07)
  local tx0, ty0, tx1, ty1 = x0 >> 3, y0 >> 3, x1 >> 3, y1 >> 3
  local dx, dy = math.abs(tx1 - tx0), -math.abs(ty1 - ty0)
  local sx = tx0 < tx1 and 1 or -1
  local sy = ty0 < ty1 and 1 or -1
  local err = dx + dy
  local cx, cy = tx0, ty0
  while not (cx == tx1 and cy == ty1) do
    local e2 = 2 * err
    if e2 >= dy then err = err + dy; cx = cx + sx end
    if e2 <= dx then err = err + dx; cy = cy + sy end
    if not (cx == tx1 and cy == ty1) then
      local attr = tile_attr_at(s, cx << 3, cy << 3)
      if attr and (SIGHT_BLOCKERS[attr] or (indoors and attr == 0x04)) then
        return true
      end
    end
  end
  return false
end

-- ===========================================================================
-- Pathfinding. A* over the passable tiles of Link's current 512-pixel window (a
-- whole dungeon room, or the loaded overworld screen), then a follow-the-beacon
-- guide: a tone is placed at the next corner of the route and pans toward it, so
-- the player walks toward the sound and it hops forward as they close on each
-- corner. Inspired by the Toby Accessibility Mod's pathfinder; Beacon reads a
-- real tile grid, so the graph is the grid itself rather than an inferred one.
-- The follower state is global so it can be driven and inspected over MCP.
-- ===========================================================================

-- Tiles a route may not cross: walls/cliffs, pits, water, and solid objects.
local IMPASSABLE = {}
for _, a in ipairs({
  0x01, 0x02, 0x03, 0x0B, 0x26, 0x43, 0x6C, 0x6D, 0x6E, 0x6F, -- wall / cliff
  0x20,                                                       -- pit / hole
  0x08, 0x4B,                                                 -- deep water (0x09 shallow is wadeable)
  0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56,                   -- solid object
}) do IMPASSABLE[a] = true end
-- Flaggable doors (0xF0-0xFF): locked doors and doors held shut by a room flag.
-- The game's own tile classifier makes every value in this range a solid
-- collision (zelda3 tile_detect.c TileBehavior_FlaggableDoor); opening one
-- rewrites its attribute in the $7F2000 table to a passable value (a passage or
-- open-door tile). So a tile still reading 0xF0-0xFF is a CLOSED door the route
-- must not cross — this is what stops the guide leading Link through a locked
-- door before he holds its key. Once the door is opened the tile no longer reads
-- in this range, so the same route opens up with no special-casing here.
for a = 0xF0, 0xFF do IMPASSABLE[a] = true end

-- In-room layer-swap staircases (BG1<->BG2): a tile Link steps onto to change floor
-- within one room, split by direction. Up-stairs (lower floor -> upper) read
-- 0x1D-0x1F, down-stairs (upper -> lower) 0x3D-0x3F (zelda3 tile_detect.c). The
-- cross-floor pathfinder treats each as a ONE-WAY portal in its own direction: some
-- drops let Link fall to the floor below with no way back up the same tile, so up
-- and down are not interchangeable. (The same two sets also feed the cross-ROOM
-- stair finder further down, which is why they are split up/down rather than pooled.)
local STAIR_UP   = { [0x1D] = true, [0x1E] = true, [0x1F] = true }
local STAIR_DOWN = { [0x3D] = true, [0x3E] = true, [0x3F] = true }
-- 0x1C is the upper layer's overlay mask (zelda3 TileBehavior_OverlayMask_1C): the
-- raised platform is absent, so the square is a hole down to the lower level. It is not
-- standable upper-floor ground (walking across it as flat floor is the layer-swap bug),
-- but Link CAN walk off the ledge into it and fall — a one-way drop, upper only. Like a
-- down-stair, but reached by stepping toward it rather than onto it, and the whole
-- masked region is one big drop rather than a single stair tile. Matched inline as the
-- literal 0x1C in plan_path (a named local would blow the 200-per-chunk local budget).

local function tile_passable(s, wtx, wty, level)
  local attr = tile_attr_at(s, wtx * 8, wty * 8, level)
  if attr == nil then return false end
  -- A flaggable/locked door (0xF0-0xFF) is solid, EXCEPT the route may lead through one
  -- while Link holds a small key ($7EF36F) to open it — otherwise, with the door still a
  -- hard wall, the pathfinder detours the long way round (up a stair, across the wall on
  -- the map) instead of the door it means to take. The waypoint's own past_locked_door
  -- gate still decides whether to aim beyond the door; this just lets the path cross it.
  if attr >= 0xF0 and attr <= 0xFF then return mem.u8(0x7EF36F) > 0 end
  if IMPASSABLE[attr] then return false end
  if s.module == 0x07 and attr == 0x04 then return false end -- indoor wall
  -- 0x1C is the upper layer's overlay mask (zelda3 TileBehavior_OverlayMask_1C):
  -- the raised platform is absent here, so this square is really the level below,
  -- a one-way drop, not standable upper-floor ground. Treating it as flat floor let
  -- A* walk "across" it on the upper level — in effect routing the lower level up to
  -- the upper without a stair. Block it so a two-level room's drop is respected: the
  -- route reaches the far point by the real upper-floor path (or a stair), not the
  -- drop it cannot climb back up.
  if s.module == 0x07 and attr == 0x1C then return false end
  return true
end

-- Every tile attribute Link physically collides with, taken from the game's own
-- classifier (zelda3src tile_detect.c: the cases flagging solid collision, plus
-- deep water and pits he cannot walk onto). Used to paint the map so the player
-- feels every obstacle, not just a hand-picked few. Some tiles are solid only
-- indoors (walls, shutter doors) and open ground outdoors, so the check takes the
-- context. Rendered in a neutral obstacle grey where no feature colour applies.
local COLLIDE_COLOR = 0x7C7C88
local COLLIDE_ALWAYS, COLLIDE_INDOOR = {}, {}
do
  local function set(t, ids) for _, a in ipairs(ids) do t[a] = true end end
  local function range(t, lo, hi) for a = lo, hi do t[a] = true end end
  set(COLLIDE_ALWAYS, {
    0x01, 0x02, 0x03, 0x26, 0x43, -- walls / standard collision
    0x27,                          -- hookshot posts: logs, pegs
    0x42, 0x44, 0x46,              -- gravestone, spike, hylian plaque
    0x57, 0x63, 0x67,              -- bonk rocks, minigame chest, crystal peg
    0x08, 0x0b, 0x4b,              -- deep water
    0x20,                          -- pit / hole
  })
  range(COLLIDE_ALWAYS, 0x50, 0x5D) -- liftables (rocks/pots), chests
  range(COLLIDE_ALWAYS, 0x70, 0x7F) -- manipulable pots / skulls
  range(COLLIDE_ALWAYS, 0xB0, 0xBD) -- pit variants
  range(COLLIDE_ALWAYS, 0xC0, 0xCF) -- torches
  range(COLLIDE_ALWAYS, 0xF0, 0xFF) -- flaggable doors
  set(COLLIDE_INDOOR, { 0x04 })     -- indoor wall
  range(COLLIDE_INDOOR, 0x6C, 0x6F) -- solid indoors (open grass outdoors)
  range(COLLIDE_INDOOR, 0x80, 0x8D) -- indoor collision
  range(COLLIDE_INDOOR, 0x90, 0xAF) -- shutter / toggle doors
end

local function is_collidable(attr, indoors)
  if attr == nil then return false end
  if COLLIDE_ALWAYS[attr] then return true end
  return indoors and COLLIDE_INDOOR[attr] == true
end

-- A* from one world tile to another, both inside Link's current 512-pixel (64
-- tile) window. Returns a list of world tiles {tx, ty, level} from start to goal, or
-- nil if unreachable / out of the window. 4-connected, Manhattan heuristic, binary
-- heap — a few thousand nodes at most, run on demand, not per frame.
--
-- A two-level dungeon room is ONE contiguous space: the two floors are searched
-- together, joined at the layer-swap staircases, so a route flows up and down floors
-- wherever the stairs allow and stops only at real blockers (walls, a locked door).
-- The search starts on Link's live floor ($7E00EE) and ends on `goal_level` (nil =
-- Link's floor). A node is level*4096 + ty*64 + tx; besides the four in-plane
-- neighbours on its own floor, a stair tile offers a ONE-WAY hop to the same tile on
-- the other floor — up-stairs only from the lower floor, down-stairs only from the
-- upper — so a drop Link cannot climb back up is never routed through backwards.
local function plan_path(s, s_tx, s_ty, g_tx, g_ty, goal_level)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3 -- window origin
  local slx, sly, glx, gly = s_tx - ox, s_ty - oy, g_tx - ox, g_ty - oy
  if slx < 0 or slx > 63 or sly < 0 or sly > 63 then return nil end
  if glx < 0 or glx > 63 or gly < 0 or gly > 63 then return nil end
  local two_floor = s.module == 0x07
  local slv = two_floor and mem.u8(LOWER_LEVEL) or 0
  local glv = (two_floor and goal_level ~= nil) and goal_level or slv
  if not tile_passable(s, g_tx, g_ty, glv) then return nil end

  local FLOOR = 4096 -- 64*64 nodes per floor; the level is the high part of the key
  local function h(x, y) return math.abs(x - glx) + math.abs(y - gly) end
  local start, goal = slv * FLOOR + sly * 64 + slx, glv * FLOOR + gly * 64 + glx
  local g, came, closed, heap = {}, {}, {}, {}
  local function push(n, f)
    heap[#heap + 1] = { n = n, f = f }
    local i = #heap
    while i > 1 and heap[i >> 1].f > heap[i].f do
      heap[i], heap[i >> 1] = heap[i >> 1], heap[i]; i = i >> 1
    end
  end
  local function pop()
    local top = heap[1].n
    heap[1] = heap[#heap]; heap[#heap] = nil
    local i, n = 1, #heap
    while true do
      local l, r, m = i * 2, i * 2 + 1, i
      if l <= n and heap[l].f < heap[m].f then m = l end
      if r <= n and heap[r].f < heap[m].f then m = r end
      if m == i then break end
      heap[i], heap[m] = heap[m], heap[i]; i = m
    end
    return top
  end

  g[start] = 0
  push(start, h(slx, sly))
  local dirs = { { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }
  -- Cap the search. A real route in a 64x64 room settles in a few hundred expansions;
  -- only an UNREACHABLE goal makes A* sweep the whole two-floor grid (~8192 nodes), and
  -- doing that every re-plan is what makes an unreachable-waypoint room lag. Bail out as
  -- "no path" well before then — far above any genuine route, far below a full sweep.
  local budget = 3000
  while #heap > 0 do
    budget = budget - 1
    if budget <= 0 then return nil end
    local n = pop()
    if n == goal then
      local rev, cur = {}, n
      while cur do
        local lv, rem = cur // FLOOR, cur % FLOOR
        rev[#rev + 1] = { ox + rem % 64, oy + rem // 64, lv }; cur = came[cur]
      end
      local path = {}
      for i = #rev, 1, -1 do path[#path + 1] = rev[i] end
      return path
    end
    if not closed[n] then
      closed[n] = true
      local lv, rem = n // FLOOR, n % FLOOR
      local nx, ny = rem % 64, rem // 64
      local function relax(c)
        local t = g[n] + 1
        if not closed[c] and (g[c] == nil or t < g[c]) then
          g[c] = t; came[c] = n; push(c, t + h(c % FLOOR % 64, c % FLOOR // 64))
        end
      end
      -- A swap-layer UP-stair (0x1E/0x1F) on its entry floor (the lower) is not flat
      -- ground: stepping onto it forces the swap up, so it is a portal-ONLY hop and
      -- cannot be walked across. A down-STAIRCASE (0x3D-0x3F) is an in-room stair Link
      -- simply walks down — its far end may be the SAME floor (e.g. 0x55's exit pocket,
      -- reached by walking across it) OR the floor below — so it allows BOTH the normal
      -- in-plane neighbours AND the one-way hop down; the A* takes whichever reaches.
      local up_to, down_to
      if two_floor then
        local attr = tile_attr_at(s, (ox + nx) * 8, (oy + ny) * 8, lv)
        if attr then
          if lv == 1 and STAIR_UP[attr] then up_to = 0        -- up-stairs: lower -> upper (portal only)
          elseif lv == 0 and STAIR_DOWN[attr] then down_to = 1 end -- down-staircase: walk across and/or hop down
        end
      end
      if up_to == nil then
        for _, d in ipairs(dirs) do
          local cx, cy = nx + d[1], ny + d[2]
          if cx >= 0 and cx <= 63 and cy >= 0 and cy <= 63 then
            if tile_passable(s, ox + cx, oy + cy, lv) then
              relax(lv * FLOOR + cy * 64 + cx)
            elseif two_floor and lv == 0
                and tile_attr_at(s, (ox + cx) * 8, (oy + cy) * 8, 0) == 0x1C then
              -- Stepping off the upper platform into the overlay-mask hole: Link falls to
              -- the lower floor. If the tile straight below is walled off, the fall keeps
              -- going the same way ACROSS the hole to the first open lower-floor tile —
              -- the ledge-hop landing (0x52's drop clears a two-tile walled barrier).
              -- Scan the contiguous hole in the step direction for that landing, but only
              -- a few tiles: a real ledge hop is short, so a bounded scan both keeps
              -- plan_path cheap over big hole regions and refuses to "fall" across a wide
              -- wall band to the far side. One-way (no relax from lv == 1) and portal-only
              -- (the hole is impassable in-plane, so Link can never cross it flat).
              local fx, fy = cx, cy
              for _ = 1, 4 do
                if fx < 0 or fx > 63 or fy < 0 or fy > 63
                    or tile_attr_at(s, (ox + fx) * 8, (oy + fy) * 8, 0) ~= 0x1C then break end
                if tile_passable(s, ox + fx, oy + fy, 1) then relax(1 * FLOOR + fy * 64 + fx); break end
                fx, fy = fx + d[1], fy + d[2]
              end
            end
          end
        end
        -- A down-staircase also offers the one-way hop to the floor below (same tile).
        if down_to and tile_passable(s, ox + nx, oy + ny, down_to) then
          relax(down_to * FLOOR + ny * 64 + nx)
        end
      elseif tile_passable(s, ox + nx, oy + ny, up_to) then
        relax(up_to * FLOOR + ny * 64 + nx)
      end
    end
  end
  return nil
end

-- Whether every tile on the straight line between two world tiles is passable.
local function line_passable(s, ax, ay, bx, by, level)
  local dx, dy = math.abs(bx - ax), -math.abs(by - ay)
  local sx = ax < bx and 1 or -1
  local sy = ay < by and 1 or -1
  local err = dx + dy
  local cx, cy = ax, ay
  while true do
    if not tile_passable(s, cx, cy, level) then return false end
    if cx == bx and cy == by then return true end
    local e2 = 2 * err
    if e2 >= dy then err = err + dy; cx = cx + sx end
    if e2 <= dx then err = err + dx; cy = cy + sy end
  end
end

-- String-pulling: drop interior waypoints Link can walk straight past, so the
-- guide beacon points at real corners rather than every tile. Each path tile carries
-- its floor; a run is pulled only within one floor (on that floor's grid), and the
-- pull always breaks at a layer-swap — the floor change is a genuine corner, and a
-- straight line across floors would read the wrong grid.
local function simplify(s, tiles)
  if #tiles <= 2 then return tiles end
  local out, anchor = { tiles[1] }, 1
  for i = 2, #tiles - 1 do
    local alv = tiles[anchor][3]
    if tiles[i + 1][3] ~= alv
        or not line_passable(s, tiles[anchor][1], tiles[anchor][2], tiles[i + 1][1], tiles[i + 1][2], alv) then
      out[#out + 1] = tiles[i]; anchor = i
    end
  end
  out[#out + 1] = tiles[#tiles]
  return out
end

-- A string-pulled A* route between two world tiles, ending on `goal_level` (nil =
-- Link's current floor), or nil if unreachable. The single routine both the live
-- guide and the map renderer go through, so a drawn route is always exactly the one
-- Link would walk — across floors and all.
local function planned_route(s, s_tx, s_ty, g_tx, g_ty, goal_level)
  local tiles = plan_path(s, s_tx, s_ty, g_tx, g_ty, goal_level)
  return tiles and simplify(s, tiles)
end

-- ===========================================================================
-- Overworld cross-screen routing: A* over the ROM-decoded collision of the whole
-- world, not just the loaded window, so a route can span screens to a distant
-- objective. Same 8-pixel grid the local planner uses; the collision comes from
-- ow_rom_attr instead of the live table. Measured a few hundred nodes for a
-- cross-field route — fast enough to replan while walking.
-- ===========================================================================
local function ow_walk(w, tx, ty)
  if tx < 0 or tx > 511 or ty < 0 or ty > 511 then return false end
  local a = ow_rom_attr(w, tx * 8 + 4, ty * 8 + 4)
  return a ~= nil and not IMPASSABLE[a]
end

-- The nearest walkable tile to (tx,ty), spiralling out — Link's $0020/$0022 is his
-- head, often inside a wall attribute, so a route must seed from real footing.
local function ow_nearest_walk(w, tx, ty)
  for r = 0, 12 do
    for dy = -r, r do
      for dx = -r, r do
        if math.max(math.abs(dx), math.abs(dy)) == r and ow_walk(w, tx + dx, ty + dy) then
          return tx + dx, ty + dy
        end
      end
    end
  end
  return nil
end

-- Whether the straight line between two world tiles stays walkable (string-pull).
local function ow_line(w, x0, y0, x1, y1)
  local dx, dy = math.abs(x1 - x0), -math.abs(y1 - y0)
  local sx = x0 < x1 and 1 or -1
  local sy = y0 < y1 and 1 or -1
  local err = dx + dy
  local x, y = x0, y0
  while true do
    if not ow_walk(w, x, y) then return false end
    if x == x1 and y == y1 then return true end
    local e2 = 2 * err
    if e2 >= dy then err = err + dy; x = x + sx end
    if e2 <= dx then err = err + dx; y = y + sy end
  end
end

-- Large overworld areas (Hyrule Castle, Kakariko, ...) span a 2x2 block of cells
-- that all share one "parent" id — the id the game reports in $008A. Routing must
-- treat any cell of a large area as the same destination, else it drags Link to
-- the parent's top-left cell when he is already on the screen. The parent table
-- is indexed by the within-world cell (0-0x3F); the value is the parent cell.
local OW_PARENT = nil
local function ow_parent(cell)
  if OW_PARENT == nil then OW_PARENT = rom.slice(0x125EC, 0x40) end
  return (string.byte(OW_PARENT, (cell & 0x3F) + 1) or (cell & 0x3F)) & 0x3F
end

local OW_ASTAR_CAP = 40000 -- node budget; bounds a worst-case mazey route

-- A* over 8-pixel world tiles from Link's footing toward (gx,gy). If `goal_area`
-- (a 0..0x3F within-world area index) is given, the goal is any reachable tile in
-- that area — so it stops at the screen boundary rather than a possibly-walled
-- centre — and (gx,gy) is only the heuristic aim point. Returns a string-pulled
-- list of world tiles {tx,ty}, or nil if unreachable / off the map.
local function ow_plan_path(s, gx, gy, goal_area)
  local w = (s.ow_screen & 0x40) ~= 0 and 1 or 0
  local sx, sy = ow_nearest_walk(w, s.x >> 3, s.y >> 3)
  if goal_area == nil then gx, gy = ow_nearest_walk(w, gx, gy) end
  if sx == nil or gx == nil then return nil end
  local function key(x, y) return y * 512 + x end
  local function heur(x, y) return math.abs(x - gx) + math.abs(y - gy) end
  local goal_parent = ow_parent(goal_area or 0)
  local function is_goal(n)
    if goal_area == nil then return n == key(gx, gy) end
    local nx, ny = n % 512, n // 512
    return ow_parent((ny >> 6) * 8 + (nx >> 6)) == goal_parent
  end
  local g, came, closed, heap = { [key(sx, sy)] = 0 }, {}, {}, {}
  local function push(n, f)
    heap[#heap + 1] = { n, f }
    local i = #heap
    while i > 1 and heap[i >> 1][2] > heap[i][2] do heap[i], heap[i >> 1] = heap[i >> 1], heap[i]; i = i >> 1 end
  end
  local function pop()
    local top = heap[1][1]
    heap[1] = heap[#heap]; heap[#heap] = nil
    local i, n = 1, #heap
    while true do
      local l, r, m = i * 2, i * 2 + 1, i
      if l <= n and heap[l][2] < heap[m][2] then m = l end
      if r <= n and heap[r][2] < heap[m][2] then m = r end
      if m == i then break end
      heap[i], heap[m] = heap[m], heap[i]; i = m
    end
    return top
  end
  push(key(sx, sy), heur(sx, sy))
  local reached, expanded = nil, 0
  while #heap > 0 and expanded < OW_ASTAR_CAP do
    local n = pop()
    if is_goal(n) then reached = n; break end
    if not closed[n] then
      closed[n] = true; expanded = expanded + 1
      local nx, ny = n % 512, n // 512
      for _, d in ipairs({ { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }) do
        local cx, cy = nx + d[1], ny + d[2]
        if ow_walk(w, cx, cy) then
          local c = key(cx, cy); local ng = g[n] + 1
          if g[c] == nil or ng < g[c] then g[c] = ng; came[c] = n; push(c, ng + heur(cx, cy)) end
        end
      end
    end
  end
  if reached == nil then return nil end
  local rev, c = {}, reached
  while c do rev[#rev + 1] = { c % 512, c // 512 }; c = came[c] end
  local pts = {}
  for i = #rev, 1, -1 do pts[#pts + 1] = rev[i] end
  if #pts <= 2 then return pts end
  local out, anchor = { pts[1] }, 1
  for i = 2, #pts - 1 do
    if not ow_line(w, pts[anchor][1], pts[anchor][2], pts[i + 1][1], pts[i + 1][2]) then
      out[#out + 1] = pts[i]; anchor = i
    end
  end
  out[#out + 1] = pts[#pts]
  return out
end

-- Follower state. Global so an agent can inspect/drive it over MCP.
pathfind_active = false
pathfind_path = nil   -- string-pulled list of world-tile waypoints {tx, ty}
pathfind_goal = nil   -- {tx, ty}
local pathfind_wp = 1
local pathfind_area = nil
local pathfind_replan_in = 0
local pathfind_arrival = nil -- what to say on reaching the goal, else a generic line

local PATH_PITCH = 3.0         -- a high, distinct navigation tone
local PATH_ALIGNED_PITCH = 3.4 -- brighter when Link faces the way to go
local PATH_VOLUME = 0.30        -- kept well under the object beacons so threats read over the guide
local PATH_DUCK = 0.35          -- nav volume scale while an enemy is engaged: ducked, not silenced
local PATH_PING_HZ = 0.5        -- sonar: a ping every 2 seconds over a soft steady tone
local WAYPOINT_REACHED = 12    -- px, ~1.5 tiles
local REPLAN_INTERVAL = 45     -- frames; also self-heals straying off the route

-- Aim the sonar path beacon from Link toward waypoint `w` (a {tile_x, tile_y}).
-- Shared by both route followers (local pathfinder and cross-screen overworld), so
-- the heading maths and tone live in one place. The pitch brightens when Link is
-- already facing along the dominant axis toward the corner, so walking toward the
-- sound is walking the route. `duck` drops the volume while an enemy is engaged, so
-- the threat tone reads over the guide without the guide dropping out entirely.
local function aim_path_beacon(s, w, duck)
  local dx, dy = (w[1] * 8 + 4) - s.x, (w[2] * 8 + 4) - s.y
  local d = DIRS[s.direction]
  local on_course = d ~= nil and (
    (math.abs(dx) > math.abs(dy)) and (d.dx ~= 0 and (dx > 0) == (d.dx > 0))
    or (math.abs(dx) <= math.abs(dy)) and (d.dy ~= 0 and (dy > 0) == (d.dy > 0)))
  beacon.set("path", {
    x = dx, y = dy,
    pitch = on_course and PATH_ALIGNED_PITCH or PATH_PITCH,
    volume = duck and PATH_VOLUME * PATH_DUCK or PATH_VOLUME,
    tremolo = PATH_PING_HZ, ping = true, -- sonar ping over a soft steady tone
  })
end

local function area_id(s)
  return (s.indoors == 1) and ("d" .. s.dungeon_room) or ("o" .. s.ow_screen)
end

local function pathfind_replan(s)
  if pathfind_goal == nil then return false end
  local tiles = planned_route(s, s.x >> 3, s.y >> 3, pathfind_goal[1], pathfind_goal[2], pathfind_goal[3])
  if tiles == nil then return false end
  pathfind_path = tiles
  pathfind_wp = math.min(2, #pathfind_path)
  pathfind_area = area_id(s)
  pathfind_replan_in = REPLAN_INTERVAL
  return true
end

-- Begin guiding Link to a world-pixel destination. `arrival`, if given, is spoken
-- on reaching it in place of the generic line. `level` is the destination's floor in
-- a two-level room (nil = Link's current floor); the route crosses floors to reach
-- it. Global for MCP / other cues.
function pathfind_to(wx, wy, arrival, level)
  local s = prev
  if s == nil or not in_play(s) then
    say("Cannot navigate now.", { priority = "navigation", category = "on-demand" })
    return false
  end
  pathfind_arrival = arrival
  pathfind_goal = { wx >> 3, wy >> 3, level }
  if pathfind_replan(s) then
    pathfind_active = true
    return true
  end
  pathfind_goal = nil
  pathfind_active = false
  beacon.clear("path")
  say("No path there.", { priority = "navigation", category = "on-demand" })
  return false
end

function pathfind_stop()
  pathfind_active = false
  pathfind_path = nil
  pathfind_goal = nil
  pathfind_arrival = nil
  beacon.clear("path")
end

-- Advance the follower one frame and place or clear the guide beacon.
local function pathfind_update(s)
  if not pathfind_active then return end
  if not in_play(s) then beacon.clear("path"); return end

  pathfind_replan_in = pathfind_replan_in - 1
  if pathfind_area ~= area_id(s) or pathfind_replan_in <= 0 then
    if not pathfind_replan(s) then
      -- Can't re-plan (e.g. Link stepped into a new room the chain hands off): stop
      -- quietly and let the guide re-aim next frame, rather than announcing a loss.
      pathfind_stop()
      return
    end
  end

  local path = pathfind_path
  while pathfind_wp <= #path do
    local w = path[pathfind_wp]
    if math.abs(w[1] * 8 + 4 - s.x) + math.abs(w[2] * 8 + 4 - s.y) <= WAYPOINT_REACHED then
      pathfind_wp = pathfind_wp + 1
    else
      break
    end
  end
  if pathfind_wp > #path then
    if pathfind_arrival then
      say(pathfind_arrival, { priority = "navigation", category = "on-demand" })
    end
    pathfind_stop()
    return
  end

  -- Duck the guide while an enemy is engaged, so the threat tone reads clearly over
  -- it, but keep guiding — full volume returns once the enemy backs off.
  aim_path_beacon(s, path[pathfind_wp], combat_engaged)
end

-- Tile-attribute classes, named like the collision sets (IMPASSABLE, COLLIDE_*),
-- so the same magic range is not re-tested inline at each call site. STAIR_UP/DOWN
-- (defined up with the pathfinder, which needs them for cross-floor portals) also
-- feed the stair finder; ENTRANCE_TILES is the set of non-door tiles a route may
-- leave a room through (walk-through entrances and the layer-swap stairs).
local DOOR_TILES, CHEST_TILES = {}, {}
local ENTRANCE_TILES = { [0x8E] = true, [0x8F] = true }
do
  for a = 0x30, 0x37 do DOOR_TILES[a] = true end         -- door / passage tiles
  for a = 0x58, 0x5D do CHEST_TILES[a] = true end
  CHEST_TILES[0x63] = true                                -- minigame chest
  for a in pairs(STAIR_UP) do ENTRANCE_TILES[a] = true end
  for a in pairs(STAIR_DOWN) do ENTRANCE_TILES[a] = true end
end

-- The nearest door / passage tile in the current 64x64 window, as world pixel
-- coordinates (centre of the tile), or nil if none is in view. Shared by the
-- door guide and the dungeon exit-finder.
local function nearest_door_tile(s)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local best, best_d
  for y = 0, 63 do
    for x = 0, 63 do
      local attr = tile_attr_at(s, (ox + x) * 8, (oy + y) * 8)
      if attr and DOOR_TILES[attr] then
        local d = math.abs(x - ltx) + math.abs(y - lty)
        if best_d == nil or d < best_d then
          best_d, best = d, { (ox + x) * 8 + 4, (oy + y) * 8 + 4 }
        end
      end
    end
  end
  return best
end

-- The nearest treasure-chest tile in the current window, as world pixel
-- coordinates, or nil. Chests read as tile-types 0x58-0x5D (and 0x63, a minigame
-- chest) in the game's tile detection. Used to lead to the Lamp chest in the intro.
local function nearest_chest_tile(s)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local best, best_d
  for y = 0, 63 do
    for x = 0, 63 do
      local attr = tile_attr_at(s, (ox + x) * 8, (oy + y) * 8)
      if attr and CHEST_TILES[attr] then
        local d = math.abs(x - ltx) + math.abs(y - lty)
        if best_d == nil or d < best_d then
          best_d, best = d, { (ox + x) * 8 + 4, (oy + y) * 8 + 4 }
        end
      end
    end
  end
  return best
end

-- The nearest on-screen item pickup (a sprite in REF.item_types), as world pixel
-- coordinates, or nil. sprites() is sorted nearest-first, so the first match is
-- the closest. Used by the dungeon guide to fetch a loose item in the room.
local function nearest_item_sprite(s)
  for _, sp in ipairs(sprites()) do
    if REF.item_types[sp.kind] then return { sp.x, sp.y } end
  end
  return nil
end

-- The nearest on-screen sprite of a specific type, as world pixel coordinates, or
-- nil. sprites() is sorted nearest-first. Used to home the intro guide on a story
-- character — Link's Uncle (115), Princess Zelda (118) — rather than a door.
local function nearest_sprite_kind(s, kind)
  for _, sp in ipairs(sprites()) do
    if sp.kind == kind then return { sp.x, sp.y } end
  end
  return nil
end

-- The nearest walkable tile to a world-pixel point, spiralling out, as a
-- world-pixel spot. A sprite to guide to — a dying uncle slumped against a wall,
-- a caged Zelda — often sits on an impassable tile, so aiming the pathfinder at
-- the sprite itself yields "no path"; snap to a tile beside it instead.
local function walkable_near(s, wx, wy, level)
  local tx, ty = wx >> 3, wy >> 3
  for r = 0, 8 do
    for dy = -r, r do
      for dx = -r, r do
        if math.max(math.abs(dx), math.abs(dy)) == r and tile_passable(s, tx + dx, ty + dy, level) then
          return (tx + dx) * 8 + 4, (ty + dy) * 8 + 4
        end
      end
    end
  end
  return wx, wy
end

-- ===========================================================================
-- Cross-room dungeon routing: a room-to-room guide layered over the local
-- pathfinder, which only reaches within the current room. Two graphs feed it. A
-- baked STATIC graph knows every room's connections up front (which side each
-- doorway or staircase leaves by), so a route can lead through rooms Link has
-- never walked. A LEARNED graph on top records the exact spot each transition
-- fired at, refining a hop the moment Link has walked it. A breadth-first search
-- over the union gives the next room to head for; the local pathfinder is aimed
-- at that hop's exit — the learned spot if known, else the door (or edge, or
-- staircase) on the static side — and re-aimed at every room boundary.
-- ===========================================================================

-- Static room adjacency, baked from the door/stair connectivity dataset (the
-- ALttP Door Randomizer's room tables, cross-checked against the disassembly's
-- underworld-room list). Packed three bytes per directed edge: from-room,
-- to-room, and the side you leave `from` by. Sides: 0 N, 1 S, 2 E, 3 W, and 4/5
-- Up/Dn for the spiral staircases that change floor. Room ids are the $00A0
-- value, globally unique, so one table spans every dungeon.
local STATIC_ADJ_PACKED =
    "\x01\x50\x03\x01\x52\x02\x01\x72\x05\x02\x11\x05\x04\x14\x01\x04\xB5\x05\x07\x17\x05\x09\x4A\x04\x0A\x3A\x04\x0B\x1B\x00"
  .."\x0C\x6B\x04\x0C\x8C\x05\x0E\x1E\x05\x11\x02\x04\x11\x21\x01\x13\x14\x02\x14\x04\x00\x14\x13\x03\x14\x15\x02\x14\x24\x01"
  .."\x15\x14\x03\x15\xB6\x04\x16\x66\x05\x17\x07\x04\x17\x27\x05\x19\x1A\x02\x1A\x19\x03\x1A\x2A\x01\x1A\x6A\x05\x1B\x0B\x01"
  .."\x1B\x2B\x01\x1C\x8C\x04\x1D\x4C\x05\x1E\x0E\x04\x1E\x1F\x02\x1E\x2E\x01\x1F\x1E\x03\x1F\x3F\x01\x21\x11\x00\x21\x22\x02"
  .."\x22\x21\x03\x22\x32\x01\x23\x24\x02\x24\x14\x00\x24\x23\x03\x26\x36\x01\x26\x76\x05\x27\x17\x04\x27\x31\x05\x28\x38\x05"
  .."\x2A\x1A\x00\x2A\x2B\x02\x2A\x3A\x01\x2B\x1B\x00\x2B\x2A\x03\x2B\x3B\x01\x2E\x1E\x00\x30\x40\x01\x31\x27\x04\x31\x77\x05"
  .."\x32\x22\x00\x32\x42\x01\x34\x35\x02\x34\x54\x04\x35\x34\x03\x35\x36\x02\x36\x26\x00\x36\x35\x03\x36\x37\x02\x36\x46\x01"
  .."\x37\x36\x03\x37\x38\x02\x38\x28\x04\x38\x37\x03\x39\x49\x01\x3A\x0A\x05\x3A\x2A\x00\x3A\x4A\x01\x3B\x2B\x00\x3B\x4B\x01"
  .."\x3D\x4D\x01\x3D\x96\x01\x3E\x4E\x01\x3F\x1F\x00\x3F\x5F\x05\x40\x30\x00\x40\xB0\x05\x41\x42\x05\x41\x51\x01\x42\x32\x00"
  .."\x42\x41\x04\x43\x53\x01\x44\x45\x02\x45\x44\x03\x45\xBC\x04\x46\x36\x00\x49\x39\x00\x49\x59\x01\x4A\x09\x05\x4A\x3A\x00"
  .."\x4B\x3B\x00\x4C\x1D\x04\x4D\x3D\x00\x4D\xA6\x05\x4E\x3E\x00\x4E\x6E\x05\x50\x01\x02\x50\x60\x01\x51\x41\x00\x51\x61\x01"
  .."\x52\x01\x03\x52\x62\x01\x53\x43\x00\x53\x63\x05\x54\x34\x05\x56\x57\x02\x57\x56\x03\x57\x58\x02\x57\x67\x01\x58\x57\x03"
  .."\x58\x68\x01\x59\x49\x00\x5B\x5C\x02\x5B\x6B\x01\x5C\x5B\x03\x5C\x5D\x04\x5D\x5C\x05\x5D\x6D\x01\x5E\x5F\x02\x5E\x6E\x01"
  .."\x5E\x7E\x01\x5F\x3F\x04\x5F\x5E\x03\x5F\x7F\x05\x60\x50\x00\x60\x61\x02\x61\x51\x00\x61\x60\x03\x61\x62\x02\x62\x52\x00"
  .."\x62\x61\x03\x63\x53\x04\x64\x65\x02\x64\xAB\x05\x65\x64\x03\x66\x16\x04\x66\x76\x01\x67\x57\x00\x67\x68\x02\x68\x58\x00"
  .."\x68\x67\x03\x6A\x1A\x04\x6B\x0C\x05\x6B\x5B\x00\x6C\x6D\x02\x6C\xA5\x04\x6D\x5D\x00\x6D\x6C\x03\x6E\x4E\x04\x6E\x5E\x00"
  .."\x70\x71\x04\x70\x80\x05\x71\x70\x05\x71\x81\x01\x72\x01\x04\x72\x82\x01\x73\x74\x02\x73\x83\x01\x74\x73\x03\x74\x75\x02"
  .."\x74\x84\x01\x75\x74\x03\x75\x85\x01\x76\x26\x04\x76\x66\x00\x77\x31\x04\x77\x87\x05\x7B\x7C\x02\x7B\x8B\x01\x7C\x7B\x03"
  .."\x7C\x7D\x02\x7D\x7C\x03\x7D\x8D\x01\x7E\x5E\x00\x7E\x7F\x02\x7E\x8E\x01\x7F\x5F\x04\x7F\x7E\x03\x80\x70\x04\x81\x71\x00"
  .."\x81\x82\x02\x82\x72\x00\x82\x81\x03\x83\x73\x00\x84\x74\x00\x84\x85\x02\x85\x75\x00\x85\x84\x03\x87\x77\x04\x8B\x7B\x00"
  .."\x8B\x8C\x02\x8B\x9B\x01\x8C\x0C\x04\x8C\x1C\x05\x8C\x8B\x03\x8C\x8D\x02\x8C\x9C\x01\x8D\x7D\x00\x8D\x8C\x03\x8D\x9D\x01"
  .."\x8E\x7E\x00\x8E\xAE\x05\x91\x92\x02\x91\xA0\x04\x92\x91\x03\x92\x93\x02\x93\x92\x03\x93\xA2\x04\x95\x96\x02\x95\xA5\x01"
  .."\x96\x3D\x00\x96\x95\x03\x97\xD1\x05\x98\xD2\x05\x99\xA9\x01\x99\xDA\x04\x9B\x8B\x00\x9B\x9C\x02\x9C\x8C\x00\x9C\x9B\x03"
  .."\x9D\x8D\x00\x9E\x9F\x02\x9E\xBE\x05\x9F\x9E\x03\x9F\xAF\x01\xA0\x91\x05\xA1\xA2\x02\xA1\xB1\x01\xA2\x93\x05\xA2\xA1\x03"
  .."\xA2\xA3\x02\xA2\xB2\x01\xA3\xA2\x03\xA3\xB3\x01\xA5\x6C\x05\xA5\x95\x00\xA6\x4D\x04\xA8\xA9\x02\xA8\xB8\x01\xA9\x99\x00"
  .."\xA9\xA8\x03\xA9\xAA\x02\xA9\xB9\x01\xAA\xA9\x03\xAA\xBA\x01\xAB\x64\x04\xAB\xBB\x01\xAE\x8E\x04\xAE\xAF\x02\xAF\x9F\x00"
  .."\xAF\xAE\x03\xB0\x40\x04\xB0\xC0\x05\xB1\xA1\x00\xB1\xC1\x01\xB2\xA2\x00\xB2\xB3\x02\xB2\xC2\x01\xB3\xA3\x00\xB3\xB2\x03"
  .."\xB3\xC3\x01\xB4\xC4\x01\xB5\x04\x04\xB5\xC5\x01\xB6\x15\x05\xB6\xC6\x01\xB7\xC7\x01\xB8\xA8\x00\xB8\xB9\x02\xB9\xA9\x00"
  .."\xB9\xB8\x03\xB9\xBA\x02\xB9\xC9\x01\xBA\xAA\x00\xBA\xB9\x03\xBB\xAB\x00\xBB\xBC\x02\xBC\x45\x05\xBC\xBB\x03\xBC\xCC\x01"
  .."\xBE\x9E\x04\xBE\xBF\x02\xBE\xCE\x01\xBF\xBE\x03\xC0\xB0\x04\xC0\xD0\x05\xC1\xB1\x00\xC1\xC2\x02\xC1\xD1\x01\xC2\xB2\x00"
  .."\xC2\xC1\x03\xC2\xC3\x02\xC2\xD2\x01\xC3\xB3\x00\xC3\xC2\x03\xC4\xB4\x00\xC4\xC5\x02\xC5\xB5\x00\xC5\xC4\x03\xC5\xD5\x01"
  .."\xC6\xB6\x00\xC6\xC7\x02\xC6\xD6\x01\xC7\xB7\x00\xC7\xC6\x03\xC9\xB9\x00\xCB\xCC\x02\xCB\xDB\x01\xCC\xBC\x00\xCC\xCB\x03"
  .."\xCC\xDC\x01\xCE\xBE\x00\xD0\xC0\x04\xD0\xE0\x05\xD1\x97\x04\xD1\xC1\x00\xD2\x98\x04\xD2\xC2\x00\xD5\xC5\x00\xD6\xC6\x00"
  .."\xD8\xD9\x02\xD9\xD8\x03\xD9\xDA\x02\xDA\x99\x05\xDA\xD9\x03\xDB\xCB\x00\xDB\xDC\x02\xDC\xCC\x00\xDC\xDB\x03\xE0\xD0\x04"

-- Side codes and their in-room-grid heading. Up/Dn (spiral stairs) change floor,
-- so they have no cardinal heading and are found by their staircase tile instead.
local SIDE_UP, SIDE_DN = 4, 5
local SIDE_DIR  = { [0] = { 0, -1 }, [1] = { 0, 1 }, [2] = { 1, 0 }, [3] = { -1, 0 } }
local SIDE_WORD = { [0] = "north", [1] = "south", [2] = "east", [3] = "west",
                    [SIDE_UP] = "up the stairs", [SIDE_DN] = "down the stairs" }

-- from_room -> { to_room -> side }, decoded from the packed table above.
local STATIC_ADJ = {}
for i = 1, #STATIC_ADJ_PACKED, 3 do
  local frm  = string.byte(STATIC_ADJ_PACKED, i)
  local to   = string.byte(STATIC_ADJ_PACKED, i + 1)
  local side = string.byte(STATIC_ADJ_PACKED, i + 2)
  local g = STATIC_ADJ[frm]; if g == nil then g = {}; STATIC_ADJ[frm] = g end
  g[to] = side
end

-- Forward declaration: hop_goal turns a route hop into a spot to aim at, but it
-- needs door_toward (defined lower); room_route_update and route_to_room, defined
-- above door_toward, reference it here and it is assigned once door_toward exists.
local hop_goal

-- Forward declaration: the goal engine (GOALS, current_goal, INTRO_GOALS, and the
-- scripted-intro helper intro_step) is defined with the advance guide far below,
-- but the objective readout above it consults these too; all reference these
-- upvalues, assigned once the goal table is defined.
local intro_step
local current_goal
local INTRO_GOALS

-- Forward declaration: nav_update re-aims the navigation assist each frame while
-- it is toggled on; on_frame (defined above the chain) drives it.
local nav_update

-- Learned graph: from_room -> { to_room -> {x, y} }, the absolute pixel spot in
-- from_room where the walk into to_room happened (so aiming Link back at it
-- re-triggers the same transition — works for doors, stairs, and holes alike).
-- Room ids are globally unique, so one graph spans every dungeon. Global for MCP.
room_graph = {}
local rg_last_room = nil -- last stable dungeon room
local rg_last_pos = nil  -- Link's pixel spot on the previous in-play frame

-- Grow the graph by observing room transitions. Runs every frame.
local function record_room_transition(s)
  if s.module ~= 0x07 or not in_play(s) then rg_last_room = nil; return end
  local room = s.dungeon_room
  if rg_last_room ~= nil and rg_last_room ~= room and rg_last_pos ~= nil then
    local g = room_graph[rg_last_room]
    if g == nil then g = {}; room_graph[rg_last_room] = g end
    g[room] = { rg_last_pos[1], rg_last_pos[2] }
  end
  rg_last_room = room
  rg_last_pos = { s.x, s.y }
end

-- The set of rooms reachable in one hop from `r`, across both graphs. A learned
-- edge and a static edge to the same room collapse to one entry, since the search
-- only needs the neighbour ids; hop_goal decides where in the room to aim.
local function room_neighbors(r)
  local out = {}
  for nr in pairs(STATIC_ADJ[r] or {}) do out[nr] = true end
  for nr in pairs(room_graph[r] or {}) do out[nr] = true end
  return out
end

-- Breadth-first search over the static+learned edges: the ordered list of rooms
-- after `from`, ending at `to`, or nil if neither graph connects them.
local function room_path(from, to)
  if from == to then return {} end
  local prev, queue, head = { [from] = false }, { from }, 1
  while head <= #queue do
    local r = queue[head]; head = head + 1
    for nr in pairs(room_neighbors(r)) do
      if prev[nr] == nil then
        prev[nr] = r
        if nr == to then
          local path, c = { to }, r
          while c ~= from do table.insert(path, 1, c); c = prev[c] end
          return path
        end
        queue[#queue + 1] = nr
      end
    end
  end
  return nil
end

-- Aim the local pathfinder at a world-pixel spot, quietly (no per-room chatter).
local function route_set_goal(s, wx, wy, level)
  pathfind_goal = { wx >> 3, wy >> 3, level }
  if pathfind_replan(s) then pathfind_active = true; return true end
  return false
end

-- The active cross-room target room, and the room we last re-aimed from. Global
-- target for MCP inspection.
route_room = nil
local rr_last_room = nil

local function room_route_stop() route_room = nil; rr_last_room = nil end

-- Re-aim the local pathfinder at each room boundary toward the target room. Only
-- acts when the room actually changes, so the local follower runs undisturbed
-- between rooms.
local function room_route_update(s)
  if route_room == nil then return end
  if s.module ~= 0x07 or not in_play(s) then return end
  if s.dungeon_room == rr_last_room then return end
  rr_last_room = s.dungeon_room
  if s.dungeon_room == route_room then
    -- Arrived at the target room: hand off to local guidance (a loose item here,
    -- else a door) and end the cross-room route.
    room_route_stop()
    local it = nearest_item_sprite(s)
    local d = it or nearest_door_tile(s)
    if d then route_set_goal(s, d[1], d[2]) end
    return
  end
  local path = room_path(s.dungeon_room, route_room)
  local hop = path and path[1]
  local exit = hop and hop_goal(s, s.dungeon_room, hop)
  if exit then route_set_goal(s, exit[1], exit[2]) end
  -- else: the graph has no next hop from here yet; leave the local goal in place.
end

-- ===========================================================================
-- Exploration memory and user markers, built on the pathfinder above.
-- ===========================================================================

-- Tiles Link has been near, so "explore" can route somewhere he has not. Keyed
-- by absolute world tile (unique per room / overworld area), so it persists
-- correctly across areas. Global for inspection over MCP.
explored = {}
local function tile_key(wtx, wty) return wty * 4096 + wtx end

local function mark_explored(s)
  local tx, ty = s.x >> 3, s.y >> 3
  for dy = -1, 1 do
    for dx = -1, 1 do
      explored[tile_key(tx + dx, ty + dy)] = true
    end
  end
end

-- Nearest passable tile in the current window Link has not yet been near, found
-- by breadth-first search over passable tiles (so it is reachable), or nil if the
-- whole reachable area has been explored.
local function nearest_unexplored(s)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local slx, sly = (s.x >> 3) - ox, (s.y >> 3) - oy
  local q, head = { { slx, sly } }, 1
  local seen = { [sly * 64 + slx] = true }
  while head <= #q do
    local c = q[head]; head = head + 1
    local wtx, wty = ox + c[1], oy + c[2]
    if not explored[tile_key(wtx, wty)] then return wtx, wty end
    for _, d in ipairs({ { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }) do
      local nx, ny = c[1] + d[1], c[2] + d[2]
      if nx >= 0 and nx <= 63 and ny >= 0 and ny <= 63 then
        local k = ny * 64 + nx
        if not seen[k] and tile_passable(s, ox + nx, oy + ny) then
          seen[k] = true; q[#q + 1] = { nx, ny }
        end
      end
    end
  end
  return nil
end

-- User waypoint markers: drop one at Link's tile, get guided back later. Keyed by
-- slot; each records the area so guidance only offers markers in the current one
-- (routing is within the loaded window). Global for MCP / multi-slot use.
markers = {}

function mark_set(slot)
  local s = prev
  if s == nil or not in_play(s) then return false end
  markers[slot] = { area = area_id(s), tx = s.x >> 3, ty = s.y >> 3 }
  return true
end

function mark_goto(slot)
  local s = prev
  if s == nil or not in_play(s) then
    say("Cannot navigate now.", { priority = "navigation", category = "on-demand" })
    return false
  end
  local m = markers[slot]
  if m == nil then
    say("No marker there.", { priority = "navigation", category = "on-demand" })
    return false
  end
  if m.area ~= area_id(s) then
    say("That marker is in another area.", { priority = "navigation", category = "on-demand" })
    return false
  end
  return pathfind_to(m.tx * 8 + 4, m.ty * 8 + 4)
end

function mark_clear(slot) markers[slot] = nil end

-- ===========================================================================
-- Overworld route follower: drives the guide beacon along a cross-screen path
-- from ow_plan_path, replanning as Link walks, in the same style as the local
-- pathfind follower. Only one router owns the "path" beacon at a time — the
-- local pathfinder takes priority, and ow_route_to stops the others.
-- ===========================================================================
ow_route_goal = nil -- {tx, ty, area?} target; global so an agent can inspect it
ow_route_path = nil -- string-pulled world-tile waypoints; global for inspection
local ow_route_wp = 1
local ow_replan_in = 0

local function ow_route_stop()
  ow_route_goal = nil
  ow_route_path = nil
end

-- Begin a cross-screen route to a world pixel destination.
function ow_route_to(wx, wy)
  pathfind_stop() -- one router owns the beacon
  room_route_stop()
  ow_route_goal = { wx >> 3, wy >> 3 }
  ow_route_path = nil
  ow_replan_in = 0
end

-- Begin a cross-screen route to an overworld AREA (0..0x3F within the world):
-- route to the nearest reachable tile on that screen, aiming at its centre. Best
-- for a destination whose exact tile isn't known or is walled off (a building).
function ow_route_to_area(area)
  pathfind_stop()
  room_route_stop()
  local col, row = area & 7, (area >> 3) & 7
  ow_route_goal = { col * 64 + 32, row * 64 + 32, area = area & 0x3F }
  ow_route_path = nil
  ow_replan_in = 0
end

local function ow_route_update(s)
  if ow_route_goal == nil or pathfind_active then return end
  if s.module ~= 0x09 or not in_play(s) then
    beacon.clear("path"); return
  end
  ow_replan_in = ow_replan_in - 1
  if ow_route_path == nil or ow_replan_in <= 0 then
    -- ow_route_goal holds tile coordinates, which ow_plan_path expects directly.
    ow_route_path = ow_plan_path(s, ow_route_goal[1], ow_route_goal[2], ow_route_goal.area)
    ow_route_wp = 1
    ow_replan_in = REPLAN_INTERVAL
  end
  local path = ow_route_path
  if path == nil then beacon.clear("path"); return end
  while ow_route_wp <= #path do
    local w = path[ow_route_wp]
    if math.abs(w[1] * 8 + 4 - s.x) + math.abs(w[2] * 8 + 4 - s.y) <= WAYPOINT_REACHED then
      ow_route_wp = ow_route_wp + 1
    else
      break
    end
  end
  if ow_route_wp > #path then
    ow_route_stop(); beacon.clear("path"); return
  end
  aim_path_beacon(s, path[ow_route_wp], combat_engaged) -- duck the guide in a fight
end

-- The opening context navigation auto-starts in: Link up and controllable in his
-- house (room 0x104) with the Lamp ($7EF34A) not yet taken — just out of bed, before
-- the first errand. Named so the frame loop reads intent, not bare constants.
local function at_quest_opening(s)
  return in_play(s) and s.dungeon_room == 0x104 and mem.u8(0x7EF34A) == 0
end

-- Place one beacon `id` at offset (dx, dy), `dist` away, using class `kind`'s reach,
-- gain, pitch and tremolo: quadratic falloff, muffled behind a wall, cleared when
-- out of reach or `hushed`. One body for every beacon source — the per-class sprite
-- tones and the tile-sourced chest tone alike, so they cannot drift apart.
local function sound_beacon(now, id, dx, dy, dist, kind, hushed)
  if hushed or dist >= kind.range then beacon.clear(id); return end
  local t = 1 - dist / kind.range
  local vol = math.min(1, t * t * (kind.gain or 1))
  if sight_blocked(now, now.x, now.y, now.x + dx, now.y + dy) then
    vol = vol * BEACON_OCCLUDED_SCALE
  end
  beacon.set(id, { x = dx, y = dy, pitch = kind.pitch, volume = vol, tremolo = kind.tremolo })
end

-- ── Enemy weapons ───────────────────────────────────────────────────────────
-- Some enemies attack with a weapon that is a threat in its own right, apart from
-- the enemy's body — most vividly the Ball-and-Chain Trooper's flail, a heavy ball
-- swung on a chain through a wide arc. Enemy weapons get their own beacon type: a
-- distinct, urgent tone (low and fast-pulsing) so a blind player can hear the arc
-- itself and dodge it, not just the enemy standing behind it. Never hushed in
-- combat — the weapon IS the danger. New weapon sources fold into WEAPON.nearest so
-- they all share the one tone. Everything hangs off one table to spare the main
-- chunk's local budget (Lua caps a function at 200 locals).
local WEAPON = {}
WEAPON.beacon = { pitch = 0.7, range = 224, tremolo = 5.0, gain = 1.8 }
-- Ball-and-Chain flail geometry, ported from zelda3 SpriteDraw_BNCFlail: a 9-bit
-- swing angle (sprite_A/B), a radius that grows through the attack (`radius`, indexed
-- by the attack timer sprite_delay_aux2), and a per-facing pivot (px/py). The aux
-- arrays sit alongside the ones SPRITE already names.
WEAPON.flail = {
  kind = 0x6A, aistate = 0x7E0D80, a = 0x7E0D90, b = 0x7E0DA0, d = 0x7E0DE0, delay2 = 0x7E0E10,
  radius = {
    [0] = 0x10, 0x12, 0x14, 0x16, 0x18, 0x1a, 0x1c, 0x1e, 0x20, 0x22, 0x24, 0x26, 0x28, 0x2a, 0x2c, 0x2e,
    0x30, 0x2e, 0x2c, 0x2a, 0x28, 0x26, 0x24, 0x22, 0x20, 0x1e, 0x1c, 0x1a, 0x18, 0x16, 0x14, 0x12,
  },
  px = { [0] = 4, 4, 12, -5 }, py = { [0] = -2, -2, -6, -4 },
}

-- One swing-axis component: the sine sample scaled by the radius, rounded to the
-- nearest 1/256th (kSinusLookupTable is a half-sine 0..256; the sign comes from the
-- angle's 0x100 bit). math.sin lands within a pixel of the ROM table — exact enough
-- for a directional beacon.
function WEAPON.component(angle, qq)
  local a = math.floor(256 * math.sin((angle & 0xff) * math.pi / 256) + 0.5)
  local m = a >= 256 and qq or (((a * qq) >> 8) + (((a * qq) >> 7) & 1))
  return (angle & 0x100) ~= 0 and -m or m
end

-- The swinging ball's world position for the trooper in slot `i`, or nil when its
-- flail is tucked in (ai_state < 2): only the extended, arcing ball is worth a tone.
function WEAPON.ball_pos(i)
  local f = WEAPON.flail
  if mem.u8(SPRITE.kind + i) ~= f.kind or mem.u8(f.aistate + i) < 2 then return nil end
  local r0 = mem.u8(f.a + i) | (mem.u8(f.b + i) << 8)
  local qq = f.radius[mem.u8(f.delay2 + i) & 31]
  local d = mem.u8(f.d + i) & 3
  local ox = WEAPON.component(r0, qq) - 4 + f.px[d]
  local oy = WEAPON.component((r0 + 0x80) & 0x1ff, qq) - 4 + f.py[d]
  local sx = mem.u8(SPRITE.x_lo + i) + mem.u8(SPRITE.x_hi + i) * 256
  local sy = mem.u8(SPRITE.y_lo + i) + mem.u8(SPRITE.y_hi + i) * 256
  return sx + ox, sy + oy
end

-- The nearest enemy weapon to Link as { dx, dy, dist } in world pixels, or nil.
-- Today that is the Ball-and-Chain flail; other served/thrown weapons can be added
-- here so they all sound through the one weapon beacon.
function WEAPON.nearest(s)
  local best
  for i = 0, 15 do
    local st = mem.u8(SPRITE.state + i)
    if st ~= nil and st ~= 0 then
      local bx, by = WEAPON.ball_pos(i)
      if bx then
        local dx, dy = bx - s.x, by - s.y
        local dist = math.abs(dx) + math.abs(dy)
        if best == nil or dist < best.dist then best = { dx = dx, dy = dy, dist = dist } end
      end
    end
  end
  return best
end

-- ── Movable objects: push waypoints ─────────────────────────────────────────
-- A general strategy for quest steps that need Link to shove a movable object a set
-- way (the throne-room Movable Mantle today; more to come). A chain waypoint declares
-- one, no hand-placed coordinates required beyond a fallback:
--   track = <sprite kind>    the waypoint FOLLOWS that object's live sprite, so the
--                            guide keeps pointing at it as it slides
--   push  = <facing 0/2/4/6> the direction Link must face to push it (see DIRS)
--   track_dx / track_dy      optional tile offset onto the standing/push side
-- The navigation guide leads Link to the object as usual. Then, while he is on it, a
-- steady tone DISTINCT from the sonar guide sounds only when he faces the push
-- direction, so a blind player rocks the stick until the "aligned" tone confirms and
-- pushes. Global (not local) to stay under the chunk's 200-local cap and to let MCP
-- inspect it, mirroring how the nav chain and pathfinder state are exposed.
PUSH = {
  -- Low and slow, a heavy grind — the sound of heaving something big — and distinct
  -- from the soft sonar "path" ping. Sounds while aligned: "push here". Pitch 0.8: as
  -- low/heavy as stays audible here — 0.5 and lower rendered too deep to hear.
  beacon = { pitch = 0.8, range = 48, tremolo = 3.0, gain = 1.0 },
  reach = 3, -- tiles from the object that count as being "on" it
}
-- Sit every tracking waypoint on its object while that object's sprite is in the room,
-- and remember the sprite's slot so a `done` predicate can read its state; out of the
-- room the waypoint keeps its last position (its authored fallback) and clears the slot.
-- Call once per frame before the nav re-aim and the beacon pass so both see the live spot.
function PUSH.track(s)
  if not nav_chain then return end
  for _, wp in ipairs(nav_chain) do
    if wp.track and wp.room == s.dungeon_room then
      wp.slot = nil
      for i = 0, 15 do
        if mem.u8(SPRITE.state + i) ~= 0 and mem.u8(SPRITE.kind + i) == wp.track then
          wp.slot = i
          local x = mem.u8(SPRITE.x_lo + i) + mem.u8(SPRITE.x_hi + i) * 256
          local y = mem.u8(SPRITE.y_lo + i) + mem.u8(SPRITE.y_hi + i) * 256
          wp.tx = (x >> 3) + (wp.track_dx or 0)
          wp.ty = (y >> 3) + (wp.track_dy or 0)
          break
        end
      end
    end
  end
end
-- The push waypoint Link is currently on (has `push`, in this room/floor, within
-- reach), else nil.
function PUSH.active(s)
  if not nav_chain then return nil end
  local ltx, lty = s.x >> 3, s.y >> 3
  local level = mem.u8(LOWER_LEVEL)
  for _, wp in ipairs(nav_chain) do
    if wp.push and wp.room == s.dungeon_room and (wp.level or 0) == level
        and math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= PUSH.reach then
      return wp
    end
  end
  return nil
end
-- Sound the alignment tone when Link is on a push object, facing its push direction,
-- and NOT currently pressing that way — it is a "you're lined up, now push" orientation
-- cue, so it drops the instant the player holds the push direction (the shove itself has
-- the game's own push sound effect). Pressing is read from the held-controller register
-- rather than from movement, since a block inches so slowly Link's position barely
-- changes frame to frame. It also goes silent once the object is fully pushed: a
-- waypoint's optional `done(slot)` predicate reads the tracked sprite's own state (the
-- Movable Mantle latches sprite_G to 0x90 at its end stop). Kept off the "path" id so it
-- never fights the guide.
function PUSH.tone(s)
  local wp = PUSH.active(s)
  local d = wp and DIRS[wp.push]
  local pushing = d and (mem.u8(0x7E00F0) & d.dpad) ~= 0
  local done = wp and wp.done and wp.done(s, wp)
  if wp and s.direction == wp.push and not pushing and not done then
    local dx, dy = wp.tx * 8 + 4 - s.x, wp.ty * 8 + 4 - s.y
    sound_beacon(s, "push", dx, dy, math.abs(dx) + math.abs(dy), PUSH.beacon, false)
  else
    beacon.clear("push")
  end
end

function on_frame(frame)
  local now = read_state()
  if now == nil then return end
  if prev == nil then
    prev = now
    return -- First frame has nothing to compare against.
  end

  local was = prev
  prev = now

  -- Turn navigation on by itself at the very start of the quest — once Link is up
  -- out of bed and controllable in his house — so the opening guidance leads without
  -- the player first pressing the key. Setting nav_active is enough: nav_update,
  -- later this frame, does the re-aim. Edge-triggered, and cleared once he leaves the
  -- opening, so it re-arms on a fresh start but a deliberate toggle-off stays off.
  if at_quest_opening(now) then
    if not intro_nav_armed then
      intro_nav_armed = true
      nav_active = true
    end
  else
    intro_nav_armed = false
  end

  -- Death outranks everything else that could be happening.
  if now.module == 0x12 and was.module ~= 0x12 then
    say("You died.", { priority = "critical", category = "combat" })
    low_health_warned = false
    return
  end

  -- Low health, latched on the crossing.
  if now.max_health > 0 and in_play(now) then
    local fraction = now.health / now.max_health
    if fraction <= LOW_HEALTH_FRACTION and now.health > 0 then
      if not low_health_warned then
        say(
          string.format("Low health. %.1f hearts.", hearts(now.health)),
          { priority = "critical", category = "combat" }
        )
        low_health_warned = true
      end
    else
      low_health_warned = false
    end
  end

  -- Healing is worth knowing about too, quietly.
  if in_play(now) and now.health > was.health then
    say(
      string.format("%.1f hearts.", hearts(now.health)),
      { priority = "interaction", category = "status", rate_limit = "800ms" }
    )
  end

  -- Game text: when a text or menu box opens (module 0x0E), read it aloud. Marked
  -- `always` so the game's own story and menu text is spoken at any verbosity — a
  -- low chatter setting trims the guide's routine callouts, never the plot.
  if now.module == 0x0E and was.module ~= 0x0E then
    local text = current_dialog_text()
    if text then
      say(text, { priority = "navigation", category = "dialog", always = true })
    end
  end

  -- Top level state changes: file select, entering a dungeon, and so on. Some
  -- modules are deliberately silent: the text module (0x0E, handled just above),
  -- the dungeon (0x07) and overworld (0x09) — being in one is obvious and the
  -- room / area callout below already says where — and the non-interactive title
  -- screens, intro (0x00) and attract mode (0x14), which the player never chose
  -- to enter. Announcing any of these is just noise.
  if now.module ~= was.module and not MODULE_SILENT[now.module] then
    -- Only announce named modules; the unlisted transition modules Link passes
    -- through (e.g. leaving a house) would otherwise be read out as "unknown".
    local nm = module_name(now.module)
    if nm ~= "unknown" then
      say(nm, { priority = "navigation", category = "area" })
    end
  end

  -- Light and dark world.
  if now.world ~= was.world and in_play(now) then
    local which = "Light world."
    if now.world ~= 0 then which = "Dark world." end
    say(which, { priority = "navigation", category = "area" })
  end

  -- Moving between rooms or overworld screens. Collapsed under one key so a
  -- transition that changes both only announces once.
  if in_play(now) and in_play(was) then
    if now.indoors == 1 and now.dungeon_room ~= was.dungeon_room then
      say(
        string.format("Room %d.", now.dungeon_room),
        { priority = "navigation", category = "area", collapse_key = "area-change", distance = 0 }
      )
    elseif now.indoors == 0 and now.ow_screen ~= was.ow_screen then
      say(
        string.format("Area %d.", now.ow_screen),
        { priority = "navigation", category = "area", collapse_key = "area-change", distance = 0 }
      )
    end
  end

  -- Enemies are never announced by name — only the spatial-audio beacon tracks
  -- them (the tone below), so a fight is signalled purely by sound and direction.
  if in_play(now) then
    local list = sprites()

    -- Keep any tracking waypoint sitting on its movable object before nav and beacons
    -- read its position this frame.
    PUSH.track(now)

    -- Spatial-audio beacons: one tone per class, on the nearest sprite of that
    -- class within its reach. `list` is sorted nearest-first, so the first sprite
    -- seen for a class is its closest one.
    local nearest = {}
    for _, sp in ipairs(list) do
      local c = category(sp)
      -- A quest-objective NPC (Zelda) is led to by the guide, not chirped at as an
      -- ambient person; skip it here so a farther real NPC can still take the tone.
      if not (c == "npc" and BEACON_SKIP_NPC[sp.kind]) and nearest[c] == nil then
        nearest[c] = sp
      end
    end

    -- In combat — an enemy within COMBAT_RANGE — only that nearest enemy sounds;
    -- the guide and the pickup/person tones fall silent so the fight is clear.
    local ne = nearest.enemy
    combat_engaged = ne ~= nil and ne.dist < COMBAT_RANGE

    -- One tone per class, on its nearest sprite within reach; in combat only the
    -- enemy sounds (the rest hushed). Each class swells at its own rate and gain.
    for name, kind in pairs(BEACON_KINDS) do
      local sp = nearest[name]
      if sp then
        sound_beacon(now, name, sp.dx, sp.dy, sp.dist, kind, combat_engaged and name ~= "enemy")
      else
        beacon.clear(name)
      end
    end

    -- Treasure chests are tiles, not sprites, but they are just another beacon
    -- source: sound the nearest unopened chest with the item tone through the same
    -- body. An opened chest changes tile-type and no longer matches, so the tone
    -- drops on its own the moment it is looted; hushed in combat like the pickups.
    local chest = nearest_chest_tile(now)
    if chest then
      local dx, dy = chest[1] - now.x, chest[2] - now.y
      sound_beacon(now, "chest", dx, dy, math.abs(dx) + math.abs(dy), CHEST_BEACON, combat_engaged)
    else
      beacon.clear("chest")
    end

    -- The enemy-weapon tone, on the nearest swinging weapon (the flail ball today).
    -- Never hushed — it is the acute threat, and its own arc is what has to be dodged.
    local weapon = WEAPON.nearest(now)
    if weapon then
      sound_beacon(now, "weapon", weapon.dx, weapon.dy, weapon.dist, WEAPON.beacon, false)
    else
      beacon.clear("weapon")
    end

    -- The alignment tone for a movable object: sounds only while Link is on a push
    -- waypoint, facing its push direction, and not pressing into it (not mid-shove).
    PUSH.tone(now)
  else
    combat_engaged = false
    for name in pairs(BEACON_KINDS) do -- no tone in menus or transitions
      beacon.clear(name)
    end
    beacon.clear("chest")
    beacon.clear("weapon")
    beacon.clear("push")
  end

  -- Remember where Link has been, for the explore command.
  if in_play(now) then mark_explored(now) end

  -- Learn the dungeon's room connectivity as Link walks, and re-aim any active
  -- cross-room route at each room boundary, before the local follower runs.
  record_room_transition(now)
  -- Keep the navigation assist aimed at the objective as beats complete and Link
  -- crosses screens, before the followers it drives so a fresh target takes effect
  -- this frame.
  nav_update(now)
  bush_cue(now)
  room_route_update(now)

  -- Route guidance runs last, so its beacon coexists with the object beacons.
  ow_route_update(now)
  pathfind_update(now)
end

-- "Where am I?"
on_command("where", function()
  if prev == nil then
    say("No game state yet.", { priority = "navigation", category = "on-demand" })
    return
  end
  local s = prev
  if in_play(s) then
    local place
    if s.indoors == 1 then
      place = string.format("Room %d", s.dungeon_room)
    else
      place = string.format("Area %d", s.ow_screen)
    end
    say(
      string.format("%s, facing %s, position %d %d.", place, facing(s.direction), s.x, s.y),
      { priority = "navigation", category = "on-demand" }
    )
  else
    say(
      string.format("%s. Not in play.", module_name(s.module)),
      { priority = "navigation", category = "on-demand" }
    )
  end
end)

-- Small natural-language helpers so a scan can say "Two Green Soldiers" rather
-- than listing each one. Counts one to ten read as words; more as digits.
local NUMBER_WORDS = {
  "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten",
}
local function count_word(n) return NUMBER_WORDS[n] or tostring(n) end

local function article(name)
  return name:sub(1, 1):match("[AEIOUaeiou]") and "An" or "A"
end

-- Plural of a name: "enemy" -> "enemies", otherwise just add s. Good enough for
-- the sprite names in play; a full rule set is not worth it.
local function pluralize(name)
  if name:sub(-1) == "y" and not name:sub(-2, -2):match("[AEIOUaeiou]") then
    return name:sub(1, -2) .. "ies"
  end
  return name .. "s"
end

-- "A Green Soldier to the north, close." / "Two Green Soldiers to the east, nearby."
local function group_phrase(count, name, dir, prox)
  if count == 1 then
    return string.format("%s %s to the %s, %s.", article(name), name, dir, prox)
  end
  return string.format("%s %s to the %s, %s.", count_word(count), pluralize(name), dir, prox)
end

-- "Scan" — describe the objects and enemies around Link, grouped so a busy room
-- reads as "Two Green Soldiers to the east" instead of one line per sprite. The
-- host binds it to a key (c by default).
on_command("scan", function()
  if not (prev ~= nil and in_play(prev)) then
    say("Not in play.", { priority = "navigation", category = "on-demand" })
    return
  end
  local list = sprites()
  if #list == 0 then
    say("Nothing nearby.", { priority = "navigation", category = "on-demand" })
    return
  end

  -- Group by name and direction; `list` is nearest-first, so a group's first
  -- sighting is its nearest member, which fixes both the ordering and the
  -- distance word. Enemies are named as enemies (a damageable sprite is a threat
  -- even if the type table would call it something else); the rest by object name.
  local groups, order = {}, {}
  for _, sp in ipairs(list) do
    local nm = is_enemy(sp) and enemy_name(sp) or (REF.sprite_names[sp.kind] or "object")
    local dir = direction(sp.dx, sp.dy)
    local key = nm .. "\0" .. dir
    local g = groups[key]
    if g == nil then
      g = { name = nm, dir = dir, count = 0, dist = sp.dist }
      groups[key] = g
      order[#order + 1] = key
    end
    g.count = g.count + 1
  end

  say(string.format("%d nearby.", #list), { priority = "navigation", category = "on-demand" })
  -- Up to four groups, nearest first, so it stays a summary rather than a list.
  for i = 1, math.min(4, #order) do
    local g = groups[order[i]]
    say(
      group_phrase(g.count, g.name, g.dir, proximity(g.dist)),
      { priority = "navigation", category = "on-demand" }
    )
  end
end)

-- "Read text" — re-read the message currently on screen, a custom command.
on_command("read_text", function()
  local text = current_dialog_text()
  if text then
    say(text, { priority = "navigation", category = "on-demand" })
  else
    say("No text on screen.", { priority = "navigation", category = "on-demand" })
  end
end)

-- "Coordinates" — a custom command declared in the manifest. The exact tile
-- position, finer than "where" gives, useful for precise navigation and for
-- debugging the plugin itself.
on_command("coordinates", function()
  if prev ~= nil and in_play(prev) then
    say(string.format("X %d, Y %d.", prev.x, prev.y),
        { priority = "navigation", category = "on-demand" })
  else
    say("Not in play.", { priority = "navigation", category = "on-demand" })
  end
end)

-- "What should I be doing?" The strategic counterpart to the local guide: it
-- reads the quest-progress bytes, finds the current critical-path milestone, and
-- speaks the objective and where to head. Says how far along the spine the player
-- is so the goal has a sense of scale.
on_command("objective", function()
  local v = read_progress()
  if v == nil then
    say("No game state yet.", { priority = "navigation", category = "on-demand" })
    return
  end
  local si, step, sn = intro_step(v)
  if step then
    say(
      string.format("Getting started, step %d of %d: %s. %s", si, sn, step.goal, step.hint),
      { priority = "navigation", category = "on-demand" }
    )
    return
  end
  -- Past the intro: number the post-intro goals from 1 (the pendant hunt onward).
  local idx, g, total = current_goal(v)
  say(
    string.format("Objective %d of %d: %s. %s", idx - INTRO_GOALS, total - INTRO_GOALS, g.goal, g.hint),
    { priority = "navigation", category = "on-demand" }
  )
end)

-- "Guide me to the nearest door." A concrete use of the pathfinder: it routes to
-- the nearest door tile. Other targets (markers, frontier) drive pathfind_to too.
on_command("pathfind", function()
  local s = prev
  if s == nil or not in_play(s) then
    say("Not in play.", { priority = "navigation", category = "on-demand" })
    return
  end
  local d = nearest_door_tile(s)
  if d == nil then
    say("No door nearby.", { priority = "navigation", category = "on-demand" })
  else
    pathfind_to(d[1], d[2])
  end
end)

-- ===========================================================================
-- "Advance the quest" — the context-aware guide bound to the L key. It knows the
-- game, not just the room: on the overworld it heads for the next place the main
-- story wants you (each GOAL's researched `area`); inside a dungeon it follows that
-- goal's authored waypoint chain, or stays quiet where no chain exists yet (the old
-- item -> Big Key -> boss room-graph spine has been retired — see
-- plugins/alttp/dungeon-rooms.md for the room data to build each chain against).
-- The pathfinder navigates precisely within the current room; a chain threads it
-- room to room.
-- ===========================================================================
local nav_say = function(text)
  say(text, { priority = "navigation", category = "on-demand" })
end

-- Compass heading from one overworld area to another on the 8-wide area grid. An
-- area byte's low three bits are the column, the next three the row; the 0x40 bit
-- is Light vs Dark world. Returns a direction word (nil if already in the target
-- cell) and whether the destination lies in the other world.
local function area_heading(from_area, to_area)
  local fc, fr = from_area & 7, (from_area >> 3) & 7
  local tc, tr = to_area & 7, (to_area >> 3) & 7
  local other_world = (from_area & 0x40) ~= (to_area & 0x40)
  if fc == tc and fr == tr then return nil, other_world end
  return direction(tc - fc, tr - fr), other_world
end

-- The door/passage tile in the current window best aligned with a room-grid
-- heading (ddx east, ddy south), tie-broken toward the nearer one. With no
-- heading it is just the nearest door. Used to leave a room in roughly the right
-- direction when the goal is in another room the local pathfinder cannot reach.
local function door_toward(s, ddx, ddy)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local best, best_score
  for y = 0, 63 do
    for x = 0, 63 do
      local attr = tile_attr_at(s, (ox + x) * 8, (oy + y) * 8)
      if attr and DOOR_TILES[attr] then
        local rx, ry = x - ltx, y - lty
        local dist = math.abs(rx) + math.abs(ry)
        local score = (ddx == 0 and ddy == 0) and -dist or (rx * ddx + ry * ddy - dist * 0.01)
        if best_score == nil or score > best_score then
          best_score, best = score, { (ox + x) * 8 + 4, (oy + y) * 8 + 4 }
        end
      end
    end
  end
  return best
end

-- Any room-leaving tile — an in-plane door (0x30-0x37), a room entrance/exit
-- (0x8E-0x8F, the game's TileBehavior_Entrance), or a staircase (0x1D-0x1F up,
-- 0x3D-0x3F down) — best aligned with a room-grid heading. Used to leave a room
-- the graph does not connect: a room can have several exits (the uncle room has
-- both a down stair and a south entrance passage), so the heading toward the
-- target room picks the right one rather than the nearest.
local function is_exit_attr(a)
  return a ~= nil and (DOOR_TILES[a] or ENTRANCE_TILES[a])
end
local function exit_toward(s, ddx, ddy)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local best, best_score
  for y = 0, 63 do
    for x = 0, 63 do
      if is_exit_attr(tile_attr_at(s, (ox + x) * 8, (oy + y) * 8)) then
        local rx, ry = x - ltx, y - lty
        local dist = math.abs(rx) + math.abs(ry)
        local score = (ddx == 0 and ddy == 0) and -dist or (rx * ddx + ry * ddy - dist * 0.01)
        if best_score == nil or score > best_score then
          best_score, best = score, { (ox + x) * 8 + 4, (oy + y) * 8 + 4 }
        end
      end
    end
  end
  return best
end

-- STAIR_UP / STAIR_DOWN are defined with the other tile-attribute classes above.
-- Provenance (zelda3 tile_detect.c, TileDetect_ExecuteInner): north/up stairs read
-- as 0x1D-0x1F, down stairs as 0x3D-0x3F. (The 0x30-0x37 the door finder keys on
-- also count as stair tiles there, but they are the in-plane doorways; 0x38-0x3C
-- are ordinary floor.) An Up hop wants the up set, a Down hop the down set.

-- The nearest staircase tile in the current window matching the wanted direction,
-- as a world-pixel spot, or nil. Falls back to the other direction's set so a hop
-- still lands on a staircase even if a room labels its stairs unexpectedly.
local function nearest_stair_tile(s, want_up)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local function scan(set)
    local best, best_d
    for y = 0, 63 do
      for x = 0, 63 do
        if set[tile_attr_at(s, (ox + x) * 8, (oy + y) * 8)] then
          local d = math.abs(x - ltx) + math.abs(y - lty)
          if best_d == nil or d < best_d then
            best_d, best = d, { (ox + x) * 8 + 4, (oy + y) * 8 + 4 }
          end
        end
      end
    end
    return best
  end
  return scan(want_up and STAIR_UP or STAIR_DOWN) or scan(want_up and STAIR_DOWN or STAIR_UP)
end

-- The middle of the current room's edge in a heading, as a world-pixel spot. Used
-- when a hop leaves by an open edge or a ladder rather than a door tile: there is
-- nothing to key on, so aim at the edge and let the local A* get as close as the
-- layout allows, which is enough to cross into the next room.
local function room_edge_goal(s, ddx, ddy)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local tx = (ddx == 0) and ltx or (ddx > 0 and 62 or 1)
  local ty = (ddy == 0) and lty or (ddy > 0 and 62 or 1)
  return { (ox + tx) * 8 + 4, (oy + ty) * 8 + 4 }
end

-- Where in `from` to aim to cross into `to` — the concrete spot behind a route
-- hop. Prefer the exact place the learned graph saw the transition; failing that,
-- take the static graph's side and find it live: a matching door on that side,
-- else (open edge / ladder) that edge; a spiral staircase for an Up/Dn hop. Also
-- returns the side, so the caller can name the direction. nil if neither graph
-- knows the hop.
hop_goal = function(s, from, to)
  local learned = room_graph[from] and room_graph[from][to]
  if learned then return learned, nil end
  local side = STATIC_ADJ[from] and STATIC_ADJ[from][to]
  if side == nil then return nil, nil end
  local dir = SIDE_DIR[side]
  if dir == nil then -- Up/Dn: a spiral staircase
    return nearest_stair_tile(s, side == SIDE_UP) or nearest_door_tile(s), side
  end
  return door_toward(s, dir[1], dir[2]) or room_edge_goal(s, dir[1], dir[2]), side
end

-- Route toward a target room, given a spoken label for what is there. Already in
-- the room: guide to a loose item if visible, else a door, and say it is here.
-- Elsewhere: start a cross-room route. If either graph connects the rooms, aim at
-- the first hop's exit (the door, edge or staircase leaving toward the target) and
-- name the direction, following the chain room by room. Only if the rooms are
-- unconnected in both graphs does it fall back to a rough compass heading (dungeon
-- rooms are a 16-wide grid, id low nibble = column, high nibble = row) and let the
-- route lock on as exploration fills the learned graph in.
local function route_to_room(s, target_room, label)
  if target_room == nil then return false end
  if s.dungeon_room == target_room then
    room_route_stop()
    local it = nearest_item_sprite(s)
    local d = it or nearest_door_tile(s)
    if d then pathfind_to(d[1], d[2]) end
    nav_say(label .. " It's in this room.")
    return true
  end
  route_room = target_room
  rr_last_room = s.dungeon_room
  local path = room_path(s.dungeon_room, target_room)
  local hop = path and path[1]
  local exit, side = nil, nil
  if hop then exit, side = hop_goal(s, s.dungeon_room, hop) end
  if exit then
    route_set_goal(s, exit[1], exit[2])
    if side then
      nav_say(string.format("%s Head %s.", label, SIDE_WORD[side]))
    else
      nav_say(label .. " Following the route.")
    end
  else
    local ddx = (target_room & 0x0F) - (s.dungeon_room & 0x0F)
    local ddy = (target_room >> 4) - (s.dungeon_room >> 4)
    local d = door_toward(s, ddx, ddy)
    if d then route_set_goal(s, d[1], d[2]) end
    nav_say(string.format("%s Head roughly %s; I'll route you once the way is known.", label, direction(ddx, ddy)))
  end
  return true
end

-- Guide toward an overworld goal by its area (a field on the goal record): a
-- compass heading and a note when it lies in the other world, a cross-screen route
-- when it is elsewhere in this one, or "look for the entrance" once Link stands in
-- it. Inside a dungeon or interior, head for the way out first. Shared by every
-- overworld or dungeon-entrance goal via route_to.
local function route_to_area(s, g)
  local area = g.area
  if s.module ~= 0x09 then
    room_route_stop()
    local d = nearest_door_tile(s)
    if d then pathfind_to(d[1], d[2]) end
    nav_say(string.format("Leave here, then on to %s.", g.goal))
    return
  end
  room_route_stop() -- drop any stale dungeon route when out on the overworld
  if area == nil then nav_say(string.format("Next: %s. %s", g.goal, g.hint)); return end
  if (s.ow_screen & 0x40) ~= (area & 0x40) then
    -- The destination is in the other world; we cannot draw a path across the
    -- mirror, so name it and give a heading.
    local dir = area_heading(s.ow_screen, area)
    local which = (area & 0x40) ~= 0 and "Dark World" or "Light World"
    nav_say(string.format("Next: %s. Head %s, then cross to the %s.", g.goal, dir or "toward it", which))
    return
  end
  if ow_parent(s.ow_screen & 0x3F) == ow_parent(area & 0x3F) then
    -- Already on the destination screen (parents compared, so a large 2x2 area
    -- counts wherever Link stands in it): hand off — the exact entrance is a
    -- per-goal waypoint, still to come.
    ow_route_stop()
    nav_say(string.format("You're at %s. Look for the entrance.", g.goal))
    return
  end
  -- Same world, elsewhere: route onto the destination screen.
  ow_route_to_area(area)
  nav_say(string.format("Routing to %s.", g.goal))
end

-- ===========================================================================
-- The scripted intro. Milestones 1 and 2 ("reach your uncle", "escort Zelda to
-- the Sanctuary") are each a single progress bump, but the opening is really a
-- chain of small beats: grab the Lamp, drop into the secret entrance to your
-- dying uncle for the sword, descend to Zelda's cell, then lead her up and out to
-- the Sanctuary. This refines those two milestones into fine steps that both the
-- objective readout and the advance guide drive, so a first-time blind player is
-- led beat by beat rather than just pointed at the castle. Each beat's completion
-- is read from the save exactly like the milestone spine: Lamp $F34A, sword
-- $F359, Zelda-following $F3CC == 1, Zelda-delivered progress $3C5 >= 2 (all
-- verified against the game's own variables). The chain is active only until
-- progress reaches 2; the milestone spine (Eastern Palace on) takes over after.
-- Rooms are the verified intro path: secret entrance / uncle 0x55, Zelda's cell
-- 0x80, Sanctuary 0x12 (overworld area 0x13); Hyrule Castle is area 0x1B.
-- ===========================================================================
local HOUSE_AREA = 0x2C -- Link's house sits on this overworld area (its interior is room 0x104)
local SANCTUARY_AREA, SANCTUARY_ROOM = 0x13, 0x12

-- The overworld approach from Link's house to the castle entrance, as the player's
-- own authored cues (mapped live by playing). The uncle beat drives this chain once
-- Link is in the castle overworld area, so the guide speaks these cues rather than
-- restating the objective. Each cue is an `arrival` line — spoken when Link reaches
-- that spot (in place of the generic "you have arrived"), the guide leading there
-- silently over the sonar path. The last waypoint is the castle-entrance bush.
local UNCLE_APPROACH = {
  { tx = 280, ty = 316, arrival = "South of the castle.", cue = true },
  { tx = 304, ty = 213, arrival = "Pick up the bush." },
  { tx = 304, ty = 212, arrival = "Enter the tunnel.", after_lift = true },
}

-- The courtyard crossing after Uncle, as an ordered waypoint sequence: with the
-- sword in hand Link leaves the uncle room back out to the courtyard, cuts through
-- the two bushes, then enters the castle proper by the door just south of him. The
-- guide leads to each in turn (advancing by proximity), rather than straight at the
-- door — the intended path goes through the bushes, and routing direct would skip
-- them. Coords are world tiles read live from the game; the rooms beyond the door
-- are in the dungeon graph, which routes on to Zelda's cell.
-- The final waypoint carries a `room`, marking it a dungeon point: once through
-- the door Link is inside the castle (room 0x61), and the in-room pathfinder leads
-- him to it. Reaching it retires the chain and hands off to the room-graph route,
-- which carries on through the castle to Zelda's cell.
-- A gate on a waypoint that lies past a locked door: the driver treats the
-- waypoint (and, being later in the chain, the ones after it) as not-yet-eligible
-- until Link can actually pass the door — either he holds a small key to open it,
-- or it is already open. Room 0x71 is open enough on the lower floor that the
-- pathfinder can reach the far waypoint without ever crossing the door, so a pure
-- collision block is not enough; the guide must be told the dependency. The door
-- tile at (dtx,dty,dlvl) reads 0xF0-0xFF while shut and is rewritten out of that
-- range once opened (see IMPASSABLE), so its live value reports the door's state;
-- the key count covers the window after Link has the key but before he spends it
-- at the door. Keyed by a real game signal, not a hand-set flag, so it re-opens on
-- its own. Small keys for the current dungeon live at $7EF36F.
local CUR_DUNGEON_KEYS = 0x7EF36F
local function past_locked_door(dtx, dty, dlvl)
  return function(s)
    if mem.u8(CUR_DUNGEON_KEYS) > 0 then return true end
    local a = tile_attr_at(s, dtx * 8, dty * 8, dlvl)
    return a == nil or a < 0xF0 or a > 0xFF
  end
end

local COURTYARD = {
  { tx = 282, ty = 225, say = "Head to the bushes and slash through." },
  { tx = 256, ty = 225, say = "Go to the castle." },
  { tx = 335, ty = 379, room = 0x55, level = 0 }, -- sewer room where the uncle is met
  { tx = 72, ty = 415, room = 0x61, say = "Find Zelda." },
  { tx = 47, ty = 392, room = 0x60, level = 1 },
  { tx = 57, ty = 335, room = 0x50, level = 1 },
  { tx = 95, ty = 11, room = 0x01, level = 1 },
  { tx = 159, ty = 472, room = 0x72, level = 0 }, -- south-door exit (upper floor, after the guard/key/chest)
  { tx = 153, ty = 491, room = 0x72, level = 0 }, -- mouth of the layer-swap stairs
  { tx = 149, ty = 507, room = 0x72, level = 1 }, -- lower floor, reached down those stairs
  { tx = 129, ty = 560, room = 0x82, level = 1 },
  { tx = 79, ty = 518, room = 0x81, level = 1 },
  { tx = 88, ty = 495, room = 0x71, level = 1 }, -- lower-floor anchor by the chest, where the route to the next room (0x70) and its key-soldier begins
  { tx = 79, ty = 486, room = 0x71, level = 0, gate = function(s) return mem.u8(0x7EF36F) > 0 end, done = function(s, wp) local a = tile_attr_at(s, wp.tx * 8, wp.ty * 8, wp.level or 0); return a == nil or a < 0xF0 or a > 0xFF end }, -- the locked door itself, up on the UPPER floor (2x2 at 79-80,485-486). Reached by a clean straight climb up the swap stair and north — no floor-flip back to L1, so no wall-cross. gate: only a target once Link holds a key to open it; done: clears once the door's tile stops reading as locked (0xF0-0xFF).
  { tx = 84, ty = 455, room = 0x71, level = 1, gate = function(s) local a = tile_attr_at(s, 79 * 8, 486 * 8, 0); return a == nil or a < 0xF0 or a > 0xFF end }, -- floor-1 door out of 0x71. gate: not a target until the locked door above (79,486) is actually OPEN — so Link is led to unlock that door first rather than aimed here early (which forces the pathfinder up-and-back through the wall). Once the door is open the way to here is clear.
  { tx = 10, ty = 452, room = 0x70, level = 0 }, -- into room 0x70
  { tx = 44, ty = 518, room = 0x80, say = "Free Princess Zelda." }, -- her cell, down the stairs from 0x70 — the rescue
}

-- The escort back out: once Zelda is freed the return trip leads up through the
-- castle to the hidden north passage and out to the Sanctuary. Authored room by
-- room by playing it (the room-graph heuristic just heads for the nearest exit,
-- which does not know the hidden passage). Wired to the "sanct" goal; grows as the
-- route is walked.
local SANCTUARY = {
  { tx = 10, ty = 516, room = 0x80, level = 0 }, -- up out of Zelda's cell room, the start of the climb
  { tx = 20, ty = 452, room = 0x70, level = 0 }, -- back up in 0x70, starting the climb out
  { tx = 79, ty = 503, room = 0x71, level = 1 }, -- up into 0x71 (the boomerang chest room), lower floor
  { tx = 124, ty = 524, room = 0x81, level = 0 }, -- up into 0x81 (the guardroom above 0x71)
  { tx = 134, ty = 512, room = 0x82, level = 0 }, -- up into 0x82, upper floor
  { tx = 159, ty = 455, room = 0x72, level = 0 }, -- up into 0x72, upper floor
  { tx = 119, ty = 15, room = 0x01, level = 1 }, -- up into 0x01, lower floor
  { tx = 151, ty = 369, room = 0x52, level = 0, via = true }, -- UP over the right-side ledge (via = mandatory): the escape climbs the stairs and drops back down here to dodge the soldiers on the lower-floor line.
  { tx = 143, ty = 375, room = 0x52, level = 1 }, -- down the stair to 0x52's lower floor, continuing the escape
  { tx = 136, ty = 415, room = 0x62, level = 1 }, -- south into 0x62, lower floor (on the open floor east of the wall)
  { tx = 95, ty = 389, room = 0x61, level = 0 }, -- west into 0x61, upper floor
  { tx = 91, ty = 326, room = 0x51, level = 0, track = 0xEE, track_dx = -2, track_dy = 2, push = 6,
    done = function(s, wp) return wp.slot ~= nil and mem.u8(0x7E0ED0 + wp.slot) == 0x90 end }, -- the throne-room Movable Mantle (sprite 0xEE): a push waypoint. Tracks the mantle's live sprite, offset (-2,+2) onto its left/push side; push = 6 (face east) to shove it. done: the mantle latches sprite_G ($7E0ED0+slot) to 0x90 at its end stop (zelda3 Sprite_EE_MovableMantle), so the tone stops and the chain advances once fully pushed. tx/ty are the fallback until the sprite loads. Zelda's dialogue narrates the push.
  { tx = 117, ty = 260, room = 0x41, level = 0 }, -- north through the opened passage into 0x41, upper floor
  { tx = 159, ty = 261, room = 0x42, level = 0 }, -- east into 0x42, upper floor
  { tx = 176, ty = 218, room = 0x32, level = 0, done = function(s, wp) local a = mem.u8((wp.level == 1 and 0x7F3000 or 0x7F2000) + (wp.ty & 63) * 64 + (wp.tx & 63)); return not (a >= 0x58 and a <= 0x5D) end }, -- north into 0x32 to the chest (2x2 at 176-177,218-219). done: the chest's own tile stops reading as a chest tile (0x58-0x5D) once opened — a direct signal, unlike the $7EF000 chest-opened bit which is not set for this room.
  { tx = 159, ty = 197, room = 0x32, level = 0, gate = function(s, wp) local a = mem.u8((wp.level == 1 and 0x7F3000 or 0x7F2000) + (wp.ty & 63) * 64 + (wp.tx & 63)); return a < 0xF0 or a > 0xFF or mem.u8(0x7EF36F) > 0 end }, -- the locked door north (2x2 at 159-160,197-198). gate: an OPEN door (tile no longer 0xF0-0xFF) is always a target so the guide leads Link through it; a still-locked door is a target only once he holds a small key ($7EF36F), else the guide stays on the chest (its key) rather than a door he cannot open.
  { tx = 131, ty = 175, room = 0x22, level = 0 }, -- north into 0x22, upper floor
}

-- A visual waypoint chain for the current map: an ordered list of {tx, ty, say}
-- world-tile waypoints. The overworld guide homes on the active one
-- (nav_chain[nav_chain_i]) and the map renderer draws the whole remaining chain —
-- the active waypoint white, the rest pink, with straight segments linking them —
-- so the player sees where the route continues past the immediate target (the
-- bushes, then on to the castle door). Cleared to nil when a plain single target
-- is in play. Globals so eval_lua and the renderer can inspect them.
-- A waypoint chain: an ordered list of {tx, ty, ...} world-tile waypoints. Two
-- kinds:
--   * hard targets — the guide routes to each in turn (bushes, a door, the
--     entrance); nav_chain_i tracks the active one and the map draws the route
--     white/pink to it.
--   * cues (`cue = true`) — spoken when Link passes near, but never routed to, so
--     a contextual "south of the castle" line doesn't drag the route off toward a
--     spot Link is already beside. The route always aims at the next hard target.
-- A waypoint's `arrival` line is spoken on reaching it (in place of a generic
-- arrival); a `say` line, if present, is spoken as the guide sets off toward it.
nav_chain = nil
nav_chain_i = 1
-- Debug map overlay toggle. When on, the map renderer adds developer aids on top
-- of the normal schematic: every waypoint of the active chain that belongs to the
-- current room, tagged with its 1-based order in the chain, and a coloured outline
-- around any room that is currently a kill-room. Always drawn in a dungeon (there is
-- no toggle) — these developer aids are wanted on at all times during authoring.
local chain_cued = {} -- cue index -> announced, so each cue speaks once per chain
-- Furthest hard index reached per chain (keyed by the chain table). Persists
-- across a dungeon excursion — chain_stop drops the live chain but not this — so
-- re-entering the map resumes at the waypoint Link had got to (the castle door he
-- came back through), never restarting him at the first (the already-cut bushes).
local chain_reached = {}
-- Per-index latch for a waypoint's `say` line (spoken once as the guide sets off
-- toward it), and the dungeon room the chain last aimed from (so it re-plans the
-- hop when Link crosses into the next room).
local chain_said = {}
local chain_last_room = nil
local chain_last_level = nil -- Link's floor ($7E00EE) at the last dungeon re-aim
-- Frames until the dungeon leg re-checks which of a room's waypoints Link can
-- reach (a waypoint behind a locked door is unreachable until it opens). Throttled
-- so the reachability probe — an A* per candidate — does not run every frame.
local chain_probe_in = 0
local CHAIN_REACH = 2 -- tiles; within this of a hard target, count it reached
local CUE_REACH = 10 -- tiles; within this of a cue, speak it
-- On re-arm, a hard waypoint within this many tiles of Link counts as already
-- reached: standing beside the castle door he came back through resumes there
-- rather than routing back to an earlier waypoint. Kept well under the spacing
-- between waypoints so it never skips one Link is only passing near.
local RESUME_REACH = 12

-- The first hard (non-cue) waypoint at or after index i; #chain+1 if none remain.
local function chain_next_hard(chain, i)
  while i <= #chain and chain[i].cue do i = i + 1 end
  return i
end

-- Whether waypoint `wp` is routable from where Link is now. A waypoint with a
-- `room` is a dungeon point — reachable only while Link is in that room, routed by
-- the in-room pathfinder; without one it is an overworld point, reachable only on
-- the overworld, routed by the cross-screen router. This lets one chain span the
-- courtyard and the rooms beyond its door without the overworld leg trying to aim
-- at a dungeon tile (or vice versa).
local function chain_here(wp, s)
  if wp.room ~= nil then return s.module == 0x07 and s.dungeon_room == wp.room end
  return s.module == 0x09
end

-- Whether the chain has any dungeon waypoint. The dungeon leg stays armed the
-- whole time such a chain's goal is active, so it can re-lead room by room (even
-- on a backtrack); the driver decides guidance from Link's current room.
local function chain_has_dungeon(chain)
  for _, wp in ipairs(chain) do
    if wp.room ~= nil then return true end
  end
  return false
end

-- Point the active follower at the current hard waypoint, silently: the overworld
-- cross-screen router on the overworld, the in-room pathfinder in a dungeon (via
-- the quiet route_set_goal, so a re-lead never re-announces or says "no path").
local function chain_aim(s)
  local wp = nav_chain and nav_chain[nav_chain_i]
  if not wp then return end
  if s.module == 0x07 then
    route_set_goal(s, wp.tx * 8 + 4, wp.ty * 8 + 4)
  else
    ow_route_to(wp.tx * 8 + 4, wp.ty * 8 + 4)
  end
end

-- Where a (re)armed chain should resume, among the hard waypoints routable from
-- where Link is now (see chain_here): the first such, bumped forward to the
-- furthest one already reached and to any Link is standing next to. The latter
-- covers coming back onto the map beside a mid-chain waypoint (the door) without
-- having tripped the earlier ones; scoping to reachable-here waypoints keeps the
-- overworld leg off the dungeon point and vice versa.
local function chain_resume_index(chain, s)
  local ltx, lty = s.x >> 3, s.y >> 3
  local pick
  for i = 1, #chain do
    local wp = chain[i]
    if not wp.cue and chain_here(wp, s) then
      if pick == nil then pick = i end
      if (chain_reached[chain] or 0) >= i then pick = i end
      if math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= RESUME_REACH then pick = i end
    end
  end
  return pick or chain_next_hard(chain, 1)
end

-- Aim at the active hard waypoint, record it as the furthest reached (so a later
-- re-arm resumes here), and speak its `say` line as the guide sets off.
local function chain_route(s)
  local wp = nav_chain and nav_chain[nav_chain_i]
  if not wp then return end
  chain_reached[nav_chain] = math.max(chain_reached[nav_chain] or 0, nav_chain_i)
  chain_aim(s)
  if wp.say then nav_say(wp.say) end
end

-- Begin a chain. On the overworld, aim at the first hard target at or after `i`,
-- or resume where Link left off (chain_resume_index) so re-entering the map picks
-- up mid-approach. In a dungeon the driver leads to the first unreached dungeon
-- waypoint, room by room, re-aiming each frame — so there is nothing to aim here.
local function chain_start(s, chain, i)
  nav_chain = chain
  chain.arrived = 0 -- reset the `via`-gate high-water mark for this run of the chain
  -- Seed it from Link's position so a mid-room re-arm (toggling the guide, a reload)
  -- doesn't send him back through a `via` detour he has already taken. A via waypoint
  -- counts as reached if Link is standing on it, or is already near the waypoint that
  -- follows it — its landing, where the detour rejoins the route (the drop scatters a
  -- few tiles, so this reach is looser than CHAIN_REACH). At the room's entrance he is
  -- far from that landing, so the gate still fires and leads the detour as intended.
  if s.module == 0x07 then
    local ltx, lty = s.x >> 3, s.y >> 3
    local level = mem.u8(LOWER_LEVEL)
    for j, wp in ipairs(chain) do
      local here = wp.room == s.dungeon_room and (wp.level or 0) == level
        and math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= CHAIN_REACH
      local nxt = wp.via and chain[j + 1]
      local landed = nxt and nxt.room == s.dungeon_room and (nxt.level or 0) == level
        and math.abs(ltx - nxt.tx) + math.abs(lty - nxt.ty) <= 12
      if here or landed then chain.arrived = math.max(chain.arrived, j) end
    end
  end
  chain_cued = {}
  chain_said = {}
  chain_last_room = nil
  if s.module == 0x09 then
    nav_chain_i = i and chain_next_hard(chain, i) or chain_resume_index(chain, s)
    chain_route(s)
  else
    nav_chain_i = chain_next_hard(chain, 1)
  end
end

-- Drop the chain (a plain single-target route replaces it).
local function chain_stop()
  nav_chain = nil
  nav_chain_i = 1
  chain_cued = {}
  chain_said = {}
  chain_last_room = nil
end

-- ── The main-quest goal engine ──────────────────────────────────────────────
-- Guidance is data, not per-goal code. GOALS lists the quest's checkpoints in
-- order; each is a plain record — where it lives, what it says, and the save byte
-- that marks it done. A single dispatcher (route_to) puts any goal onto Link's
-- current map, so the same handful of lines drive every step: take the first unmet
-- goal and route to it, letting the map transitions and approach chains carry Link
-- there. A goal approached from the wrong side (the Lamp, once Link has wandered
-- off) routes him back the way he came — skipping it turns him around rather than
-- stranding the guide.

-- A goal's completion test, as data: mem[addr] >= n.
-- A completion condition on a save byte: {addr, n} means mem[addr] >= n, or
-- {addr, bit = mask} means the flag is set. A `done` may be one condition, or a
-- list of them satisfied if ANY holds — used where the direct byte is not monotonic
-- (Zelda's follower flag clears once she is delivered, so progress covers it).
local function cond_met(c)
  local val = mem.u8(c[1]) or 0
  if c.bit then return (val & c.bit) ~= 0 end
  return val >= c[2]
end
local function met(done)
  if type(done[1]) == "table" then
    for _, c in ipairs(done) do if cond_met(c) then return true end end
    return false
  end
  return cond_met(done)
end

-- Resolve a goal to a world-pixel target on Link's current map: a dynamic lookup
-- (the chest, a named sprite) or a fixed tile, else nil.
local function goal_point(s, g)
  if g.find == "chest" then return nearest_chest_tile(s) end
  if g.find == "sprite" then
    local sp = nearest_sprite_kind(s, g.kind)
    if sp then local wx, wy = walkable_near(s, sp[1], sp[2]); return { wx, wy } end
    return nil
  end
  if g.tx then return { g.tx * 8 + 4, g.ty * 8 + 4 } end
  return nil
end

-- Put goal `g` onto Link's current map — the only place that branches on context,
-- shared by every goal. A room goal (the intro): in its room, pathfind to the
-- target; elsewhere in the same dungeon, walk the room graph; not in the dungeon
-- yet, take its authored overworld approach chain, double back to a home area if it
-- names one (the Lamp), or — inside another interior — head for the way out. A
-- dungeon goal (the palaces): inside, run the dungeon's item→Big-Key→boss spine;
-- outside, route to its overworld area. Any other goal is a plain overworld spot.
local function route_to(s, g, v)
  -- An authored waypoint chain leads the goal: its overworld waypoints outside,
  -- its dungeon waypoints room by room once inside (the driver advances through
  -- them). While the chain is still leading, nothing else routes — a chain-only
  -- goal (Zelda) relies on it end to end, with no room-graph "waypoint to Zelda".
  if g.chain
     and (s.module == 0x09 or (s.module == 0x07 and chain_has_dungeon(g.chain))) then
    if nav_chain ~= g.chain then chain_start(s, g.chain) end
    return
  end
  -- The chain is consumed (or Link is inside with it fully walked). A chain-only
  -- goal with no room/area/dungeon of its own has nothing left to do — the final
  -- waypoint was the destination.
  if g.chain and not (g.room or g.area or g.dungeon) then return end
  if g.room then
    -- In the target room: home on the sprite/tile there.
    if s.module == 0x07 and s.dungeon_room == g.room then
      local p = goal_point(s, g)
      if p then pathfind_to(p[1], p[2], g.arrival) end
      if g.lead then nav_say(g.lead) end
      return
    end
    -- Inside a dungeon with a known path to the goal room: route room-to-room.
    if s.module == 0x07 and room_path(s.dungeon_room, g.room) then
      route_to_room(s, g.room, g.lead or g.arrival or "Continue on.")
      return
    end
    -- Overworld with a known entrance area, else head for the way out.
    if g.entrance_area and s.module == 0x09 then
      ow_route_to_area(g.entrance_area)
      if g.recover then nav_say(g.recover) end
    else
      local d = exit_toward(s, 0, 1) or nearest_door_tile(s)
      if d then pathfind_to(d[1], d[2]) end
      if g.leave then nav_say(g.leave) end
    end
    return
  end
  if g.dungeon and s.module == 0x07 then
    -- Inside the target dungeon. The old item -> Big Key -> boss room-graph spine is
    -- retired in favour of authored waypoint chains (see plugins/alttp/dungeon-rooms.md
    -- for the room data to build each chain against). Until this dungeon has a chain,
    -- stay quiet rather than mis-route — no spine to fight the chain navigator.
    room_route_stop()
    pathfind_stop()
    return
  end
  route_to_area(s, g) -- head toward the goal's overworld area (or pedestal)
end

-- The quest's opening beats, in order. Pure data; route_to interprets each.
local GOALS = {
  { id = "lamp", goal = "Grab the Lamp from the chest",
    hint = "There is a treasure chest in your house holding the Lamp — take it for the dark passages ahead.",
    -- Held, or the intro is over (progress >= 2): the wider intro is bounded by
    -- that byte, so the Lamp only nags — and backtracks — until then.
    done = { { 0x7EF34A, 1 }, { 0x7EF3C5, 2 } }, room = 0x104, find = "chest",
    lead = "Get the lantern.", arrival = "Open the chest.",
    entrance_area = HOUSE_AREA, recover = "Go back for the lantern." },
  { id = "uncle", goal = "Reach your uncle for the sword",
    hint = "Enter Hyrule Castle by the hidden passage — the bush against the wall drops you in — and reach your dying uncle for the sword and shield.",
    done = { { 0x7EF359, 1 }, { 0x7EF3C5, 2 } }, room = 0x55, find = "sprite", kind = 115,
    chain = UNCLE_APPROACH, leave = "Leave the house.", arrival = "Find your uncle." },
  { id = "zelda", goal = "Free Princess Zelda",
    hint = "Descend through the castle to the dungeon below and free Princess Zelda from her cell.",
    -- Her follower flag clears once she is delivered, so progress >= 2 also counts.
    -- Chain-only: the authored waypoints lead all the way to her cell, so there is
    -- no room-graph fallback (which was misrouting). done stays a data predicate.
    done = { { 0x7EF3CC, 1 }, { 0x7EF3C5, 2 } }, chain = COURTYARD },
  { id = "sanct", goal = "Escort Zelda to the Sanctuary",
    hint = "Lead Zelda back up through the castle and out the hidden north passage to the Sanctuary.",
    done = { 0x7EF3C5, 2 }, chain = SANCTUARY, room = SANCTUARY_ROOM, entrance_area = SANCTUARY_AREA,
    leave = "Head for the Sanctuary." },
  -- Post-intro: the pendant hunt, the Master Sword, Agahnim, the seven crystals,
  -- then Ganon. Each `dungeon` goal routes to its overworld area; inside the dungeon
  -- it follows that dungeon's authored waypoint chain (none yet — the guide stays
  -- quiet there for now), the retired spine's room data kept in dungeon-rooms.md.
  -- The Master Sword is a plain overworld spot. Areas from the researched entrance
  -- table (the old MILESTONE_AREA); done bits are the pendant/crystal bitfields.
  { id = "eastern", goal = "Eastern Palace, the first pendant",
    hint = "The big green palace at the far east edge of the Light World. Clear it for the Bow and the Pendant of Courage; Sahasrahla then gives you the Pegasus Boots.",
    done = { 0x7EF374, bit = 0x01 }, area = 0x1E, dungeon = true },
  { id = "desert", goal = "Desert Palace, the second pendant",
    hint = "The Desert of Mystery in the southwest. Read the stone tablet there with the Book of Mudora to open the way in. Clear it for the Power Glove and the Pendant of Power.",
    done = { 0x7EF374, bit = 0x04 }, area = 0x30, dungeon = true },
  { id = "hera", goal = "Tower of Hera, the third pendant",
    hint = "The summit of Death Mountain, to the north. Take the Magic Mirror from the old man on the climb and the Moon Pearl inside. Clear it for the Pendant of Wisdom.",
    done = { 0x7EF374, bit = 0x02 }, area = 0x03, dungeon = true },
  { id = "mastersword", goal = "Claim the Master Sword",
    hint = "Deep in the Lost Woods, northwest. With all three Pendants, pull the Master Sword from its pedestal in the grove.",
    done = { 0x7EF359, 2 }, area = 0x00 },
  { id = "agahnim", goal = "Hyrule Castle Tower, defeat Agahnim",
    hint = "The Master Sword breaks the barrier around the castle's front tower. Climb to the top and defeat Agahnim; the fight casts you into the Dark World.",
    done = { 0x7EF3C5, 3 }, area = 0x1B, dungeon = true },
  { id = "pod", goal = "Palace of Darkness, the first crystal",
    hint = "Northeast Dark World, near the Pyramid. You need the Moon Pearl to stay human and the Bow. Clear it for the Magic Hammer.",
    done = { 0x7EF37A, bit = 0x02 }, area = 0x5E, dungeon = true },
  { id = "swamp", goal = "Swamp Palace, the second crystal",
    hint = "The southern Dark World swamp. First open the dam in the Light World swamp to lower the water, then Mirror across. Clear it for the Hookshot.",
    done = { 0x7EF37A, bit = 0x10 }, area = 0x7B, dungeon = true },
  { id = "skull", goal = "Skull Woods, the third crystal",
    hint = "The northwest Dark World woods, the counterpart of the Lost Woods. Clear it for the Fire Rod.",
    done = { 0x7EF37A, bit = 0x40 }, area = 0x40, dungeon = true },
  { id = "thieves", goal = "Thieves' Town, the fourth crystal",
    hint = "The Village of Outcasts in the west Dark World. Clear it for the Titan's Mitt, which lifts the heavy dark rocks gating the last three dungeons.",
    done = { 0x7EF37A, bit = 0x20 }, area = 0x58, dungeon = true },
  { id = "ice", goal = "Ice Palace, the fifth crystal",
    hint = "The island in the far southeast Dark World. Clear it for the Blue Mail.",
    done = { 0x7EF37A, bit = 0x04 }, area = 0x75, dungeon = true },
  { id = "mire", goal = "Misery Mire, the sixth crystal",
    hint = "The southwest Dark World. Stand at the entrance and use the Ether Medallion to open it. Clear it for the Cane of Somaria.",
    done = { 0x7EF37A, bit = 0x01 }, area = 0x70, dungeon = true },
  { id = "turtle", goal = "Turtle Rock, the seventh crystal",
    hint = "The summit of the Dark World Death Mountain, east. Use the Quake Medallion at the Light World Lake of Ill Omen to open it. Clear it for the Mirror Shield.",
    done = { 0x7EF37A, bit = 0x08 }, area = 0x47, dungeon = true },
  { id = "ganon", goal = "Ganon's Tower, then Ganon",
    hint = "With all seven Crystals the seal on Ganon's Tower, atop the Dark World Death Mountain, lifts. Beat Agahnim again at the top, then finish Ganon at the Pyramid with the Silver Arrows.",
    done = { 0x7EF3C5, 99 }, area = 0x43, dungeon = true }, -- terminal: never met
}

INTRO_GOALS = 4 -- the first four goals are the scripted intro

-- The current quest goal: the first not yet met (Ganon is terminal, never met, so
-- this always yields). Returns its index, record and the goal count.
current_goal = function(v)
  for i, g in ipairs(GOALS) do
    if not met(g.done) then return i, g, #GOALS end
  end
  return #GOALS, GOALS[#GOALS], #GOALS
end

-- The scripted-intro beat, for the objective readout only: the current goal while
-- it is one of the opening four and Zelda is not yet delivered, else nil.
intro_step = function(v)
  if v == nil or v.progress >= 2 then return nil end
  local i, g = current_goal(v)
  if i <= INTRO_GOALS then return i, g, INTRO_GOALS end
  return nil
end

-- The navigation assist is a global on/off toggle, bound to L (advance). While it
-- is on it re-aims itself at the current objective across every screen, room and
-- module change, so the route stays alive map to map instead of dying at each
-- transition — the player flips it on once and it leads the whole way. `nav_sig`
-- is the context it last aimed from; it re-aims only when the module or the
-- objective changes, since the room-to-room and screen-to-screen followers handle
-- movement within a module quietly. Global for MCP inspection.
nav_active = false
local nav_sig = nil
local nav_idle_sig = nil -- signature at which an "on but idle" re-aim was last forced
-- The "<room>:<objective>" we last announced, so a short-term room objective is
-- stated once on entry rather than every frame while it is unmet.
local room_obj_announced = nil

-- Aim the guide at the current quest goal from wherever Link stands. One line now:
-- the goal engine picks the first unmet goal and route_to puts it on Link's map,
-- whether that is an intro room, a dungeon to clear, or an overworld destination.
local function nav_reaim(s, v)
  chain_stop() -- each goal rebuilds its own approach chain if it has one
  local _, g = current_goal(v)
  route_to(s, g, v)
end

-- What the guide should be heading toward, plus which module Link is in — so it
-- re-aims exactly when either changes and stays put otherwise. The objective is just
-- the current goal's id; a chain, once armed, drives its own per-room advance inside
-- a dungeon without a signature change.
local function nav_signature(s, v)
  local _, g = current_goal(v)
  return s.module .. ":" .. (g and g.id or "done")
end

-- ── Short-term room objectives ──────────────────────────────────────────────
-- Some rooms gate progress on a task done in place — clearing the enemies to open
-- the doors, and later the likes of pushing a block or lighting torches. Each is a
-- data record: a detector for "present and unmet", the line to state on entering,
-- and where to lead (a world pixel, or nothing). While one is active it overrides
-- the quest goal; when it clears, the goal resumes. Kill-rooms are the first,
-- built on the room-tag and live-enemy checks defined earlier.

-- Rooms whose chest is worth a detour — its contents matter to the route (e.g. the
-- room-0x72 map, the room-0x71 chest on the escape route). Most chests are optional
-- and only get the item beacon, not a routing objective, so the guide does not drag
-- Link off the linear route to every chest it passes. Keyed by dungeon room id.
local CHEST_ROOMS = { [0x72] = true, [0x71] = true }

local ROOM_OBJECTIVES = {
  { id = "kill",
    cue = "Defeat all enemies.",
    active = function(s)
      return kill_room(s) and (nearest_pending_enemy(s) ~= nil or overlords_pending())
    end,
    target = function(s)
      local e = nearest_pending_enemy(s)
      if e then return walkable_near(s, e[1], e[2]) end
    end },
  -- A loose key or big key dropped in the room (e.g. by a slain guard): fetch it
  -- before moving on. Listed above the chest so, once both are out, the key first.
  { id = "key",
    cue = "Grab the key.",
    -- Only while heading INTO the dungeon, not while escorting Zelda back out. On the
    -- castle-escape backtrack a respawned guard drops a key Link no longer needs (a
    -- slain guard becomes its own key sprite), and routing to it just pulls the guide
    -- off the escort. Zelda's follow flag ($7EF3CC) is set only during that escort and
    -- clear in every other dungeon, so this gates exactly the escort, nothing else.
    active = function(s)
      return mem.u8(0x7EF3CC) == 0
        and (nearest_sprite_kind(s, 228) ~= nil or nearest_sprite_kind(s, 229) ~= nil)
    end,
    target = function(s)
      local k = nearest_sprite_kind(s, 228) or nearest_sprite_kind(s, 229)
      if k then return walkable_near(s, k[1], k[2]) end
    end },
  -- An unopened chest in the room — but only once the room is quiet, so the guide
  -- does not send Link to the chest with a guard still on him (clear enemies, then
  -- the key drop, then the chest). Gated on the chest being on-screen: a big dungeon
  -- room can hold several chambers behind doors within one room id, and a chest in a
  -- far chamber must not pull the guide across the room ahead of the door and the
  -- fight between here and there — it takes over only once Link reaches it.
  { id = "chest",
    cue = "Open the chest.",
    active = function(s)
      local c = nearest_chest_tile(s)
      return CHEST_ROOMS[s.dungeon_room] and c ~= nil and on_screen(c[1] - s.x, c[2] - s.y)
        and nearest_pending_enemy(s) == nil and not overlords_pending()
    end,
    target = function(s)
      local c = nearest_chest_tile(s)
      if c then return walkable_near(s, c[1], c[2]) end
    end },
}

-- The active short-term objective in Link's current room, or nil. Dungeon-only.
local function room_objective(s)
  if s.module ~= 0x07 then return nil end
  for _, o in ipairs(ROOM_OBJECTIVES) do
    if o.active(s) then return o end
  end
  return nil
end

-- Turn the assist off and drop every route it was driving.
local function nav_stop()
  nav_active = false
  nav_sig = nil
  room_obj_announced = nil
  chain_stop()
  room_route_stop()
  ow_route_stop()
  pathfind_stop()
end

-- Per-frame while the assist is on: re-aim when the module or objective changes.
-- Runs before the followers it feeds so a fresh target takes effect this frame.
nav_update = function(s)
  if not nav_active or not in_play(s) then return end
  local v = read_progress()
  local sig = nav_signature(s, v)
  -- Re-aim on a context change. Also re-aim when nav is on but nothing is armed at
  -- an unchanged signature: loading a savestate (or re-entering play) can leave nav
  -- on with its followers cleared, and the signature-only gate would then keep it
  -- silently idle — drawing no route and giving no guidance — until it was toggled
  -- off and on. The nav_idle_sig latch limits this to a single re-aim per signature,
  -- so a genuinely unrouteable spot does not re-aim every frame.
  local idle = nav_chain == nil and not pathfind_active
    and ow_route_goal == nil and route_room == nil
  if sig ~= nav_sig then
    nav_sig = sig
    nav_idle_sig = nil
    nav_reaim(s, v)
  elseif idle and nav_idle_sig ~= sig then
    nav_idle_sig = sig
    nav_reaim(s, v)
  end
  -- Short-term room objective: while this room gates progress on a task done in
  -- place (clearing its enemies, ...), it overrides the quest goal — stated once on
  -- entry, leading to whatever satisfies it, which retargets only when it moves a
  -- couple of tiles (once close, the combat beacon takes the final approach). When
  -- it clears, re-aim at the quest goal (now, e.g., with the doors open).
  local ro = room_objective(s)
  if ro then
    local key = s.dungeon_room .. ":" .. ro.id
    if room_obj_announced ~= key then
      nav_say(ro.cue)
      room_obj_announced = key
    end
    local tx, ty = ro.target(s)
    if tx and (pathfind_goal == nil
      or math.abs(pathfind_goal[1] - (tx >> 3)) + math.abs(pathfind_goal[2] - (ty >> 3)) >= 2) then
      route_set_goal(s, tx, ty)
    end
    return
  end
  if room_obj_announced ~= nil then
    room_obj_announced = nil
    nav_reaim(s, v) -- objective cleared: resume the quest goal
  end
  -- Drive the waypoint chain. Two legs, by module:
  if nav_chain and in_play(s) then
    local ltx, lty = s.x >> 3, s.y >> 3
    -- Soft cues (overworld): speak once as Link passes near, never routed to.
    for c, wp in ipairs(nav_chain) do
      if wp.cue and chain_here(wp, s) and not chain_cued[c]
        and math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= CUE_REACH then
        if wp.arrival then nav_say(wp.arrival) end
        chain_cued[c] = true
      end
    end
    if s.module == 0x09 then
      -- Overworld leg: advance by proximity, staying alive on the last overworld
      -- waypoint (re-leading if Link strays) so the guide is never idle. Dungeon
      -- waypoints belong to the inside — they are not advanced to out here.
      local wp = nav_chain[nav_chain_i]
      if wp and not wp.room and math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= CHAIN_REACH
        and (not wp.after_lift or mem.u8(0x7E0309) ~= 0) then
        if not chain_cued[nav_chain_i] then
          if wp.arrival then nav_say(wp.arrival) end
          chain_cued[nav_chain_i] = true
        end
        chain_reached[nav_chain] = math.max(chain_reached[nav_chain] or 0, nav_chain_i)
        local nxt = chain_next_hard(nav_chain, nav_chain_i + 1)
        if nxt <= #nav_chain and not nav_chain[nxt].room then
          nav_chain_i = nxt
          chain_route(s)
        end
      end
      wp = nav_chain[nav_chain_i]
      if wp and not wp.room and ow_route_goal == nil
        and math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) > CHAIN_REACH then
        ow_route_to(wp.tx * 8 + 4, wp.ty * 8 + 4)
      end
    else
      -- Dungeon leg. The quest is one long linear route; in whatever room Link is in,
      -- head to the LAST chain waypoint of that room he can currently REACH. A
      -- waypoint behind a locked door is unreachable, so the guide leads to the door
      -- (an earlier waypoint) until it is opened, then advances to the one beyond.
      -- No progress bookkeeping and backtrack-proof: any room re-aims at its last
      -- reachable waypoint. A room's sub-goal already took precedence above.
      -- A two-level room is ONE contiguous space to the pathfinder: it crosses the
      -- layer-swap stairs on its own, so a waypoint is a candidate whatever floor it
      -- is on — reachability (plan_path across floors, one-way drops respected) is the
      -- only test. Each candidate is snapped and planned on its own floor (wp.level).
      -- Re-aim on a floor change too, since the flip opens or closes cross-floor
      -- routes (a one-way drop is gone once taken).
      local level = mem.u8(LOWER_LEVEL)
      local reaimed = chain_last_room ~= s.dungeon_room or chain_last_level ~= level
      chain_probe_in = chain_probe_in - 1
      -- Keep following a still-valid route; re-pick the target only when not
      -- following one, when Link changed rooms or floors, or on the throttled
      -- re-probe (which catches a door that just opened, making a further waypoint
      -- reachable).
      local cur = nav_chain[nav_chain_i]
      local following = pathfind_active and cur and cur.room == s.dungeon_room and not reaimed
      if not following or chain_probe_in <= 0 then
        chain_probe_in = 12
        local pick, pgx, pgy, plevel
        for i, wp in ipairs(nav_chain) do
          if wp.room == s.dungeon_room and (wp.gate == nil or wp.gate(s, wp)) then
            if wp.done and wp.done(s, wp) then
              -- Its objective is already met (a chest looted, a block fully shoved):
              -- count it reached and never target it again, so the guide advances past it.
              nav_chain.arrived = math.max(nav_chain.arrived or 0, i)
            else
              local gx, gy = walkable_near(s, wp.tx * 8 + 4, wp.ty * 8 + 4, wp.level)
              if plan_path(s, ltx, lty, gx >> 3, gy >> 3, wp.level) then pick, pgx, pgy, plevel = i, gx, gy, wp.level end
            end
          end
          -- A `via` waypoint is a mandatory intermediate: once it is the reachable pick,
          -- hold the "furthest reachable" scan here until Link has actually arrived at it
          -- (nav_chain.arrived), so the chain can't shortcut past it to a later same-room
          -- waypoint. 0x52's escape climbs the ledge and drops back down to dodge the
          -- soldiers standing on the shorter lower-floor line to the next waypoint.
          if wp.via and pick == i and (nav_chain.arrived or 0) < i then break end
        end
        if pick then
          nav_chain_i = pick
          local wp = nav_chain[pick]
          if math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= CHAIN_REACH and (wp.level or 0) == level then
            nav_chain.arrived = math.max(nav_chain.arrived or 0, pick) -- clears any `via` gate at/behind here
            if wp.arrival and not chain_cued[pick] then nav_say(wp.arrival); chain_cued[pick] = true end
            pathfind_stop() -- arrived; go quiet until the next waypoint opens up
          else
            if wp.say and not chain_said[pick] then nav_say(wp.say); chain_said[pick] = true end
            route_set_goal(s, pgx, pgy, plevel)
          end
        end
      end
      chain_last_room = s.dungeon_room
      chain_last_level = level
    end
  end
end

-- ── Waypoint recording (mapping phase) ──────────────────────────────────────
-- Tooling for building waypoint chains by playing: the user drives Link to a spot
-- and Claude captures it with rec_here("what to say there") over eval_lua. REC
-- accumulates across calls; rec_dump prints the run as a pasteable chain literal.
-- These are globals so eval_lua reaches them, and their bodies close over `prev`
-- (the latest frame) and `mem` from the file scope. State resets on reload_plugin
-- — dump before reloading. Recording never runs during normal play.
REC = REC or {}

-- Capture Link's current spot as a waypoint. Tile coords use the same >>3 world
-- tile the chain proximity test compares against, so a recorded spot re-triggers
-- where it was taken. `say` is the line the guide speaks on reaching it.
function rec_here(say)
  local s = prev
  if s == nil then return "not in play" end
  local e = {
    module = s.module,
    ow_area = mem.u8(0x7E008A) & 0x3F, -- overworld screen id (LW/DW share the low 6 bits)
    room = s.dungeon_room,
    tx = s.x >> 3, ty = s.y >> 3,
    dir = s.direction,
    say = say or "",
  }
  REC[#REC + 1] = e
  return string.format("#%d  module=%02X ow_area=%02X room=%04X  tx=%d ty=%d dir=%d  | %s",
    #REC, e.module, e.ow_area, e.room or 0xFFFF, e.tx, e.ty, e.dir, e.say)
end

function rec_undo()
  if #REC == 0 then return "nothing recorded" end
  local e = REC[#REC]; REC[#REC] = nil
  return string.format("removed (tx=%d ty=%d) — %d left", e.tx, e.ty, #REC)
end

function rec_clear() REC = {}; return "cleared" end

function rec_list()
  if #REC == 0 then return "(empty)" end
  local out = {}
  for i, e in ipairs(REC) do
    out[i] = string.format("#%d module=%02X ow_area=%02X room=%04X tx=%d ty=%d dir=%d | %s",
      i, e.module, e.ow_area, e.room or 0xFFFF, e.tx, e.ty, e.dir, e.say)
  end
  return table.concat(out, "\n")
end

-- The recorded run as a pasteable chain literal (world tiles + the say lines).
function rec_dump()
  if #REC == 0 then return "-- (no waypoints recorded)" end
  local out = { "{" }
  for _, e in ipairs(REC) do
    out[#out + 1] = string.format(
      "  { tx = %d, ty = %d, say = %q },  -- module %02X ow_area %02X room %04X dir %d",
      e.tx, e.ty, e.say, e.module, e.ow_area, e.room or 0xFFFF, e.dir)
  end
  out[#out + 1] = "}"
  return table.concat(out, "\n")
end

on_command("advance", function()
  local s = prev
  if s == nil or not in_play(s) then
    nav_say("Not in play.")
    return
  end
  if nav_active then
    nav_stop()
    nav_say("Navigation off.")
    return
  end
  nav_active = true
  local v = read_progress()
  nav_sig = nav_signature(s, v)
  nav_reaim(s, v)
end)

on_command("pathfind_stop", function()
  nav_stop()
  say("Navigation stopped.", { priority = "navigation", category = "on-demand" })
end)

-- "Guide me somewhere I haven't been." Routes toward the nearest reachable tile
-- in this area that Link has not yet walked near.
on_command("explore", function()
  local s = prev
  if s == nil or not in_play(s) then
    say("Not in play.", { priority = "navigation", category = "on-demand" })
    return
  end
  local tx, ty = nearest_unexplored(s)
  if tx == nil then
    say("This area is explored.", { priority = "navigation", category = "on-demand" })
  else
    pathfind_to(tx * 8 + 4, ty * 8 + 4)
  end
end)

-- Drop a waypoint at Link's spot, and guide back to it later. Slot 1 from the
-- keyboard; mark_set/mark_goto cover more slots over MCP.
on_command("mark", function()
  if mark_set(1) then
    say("Marker set.", { priority = "navigation", category = "on-demand" })
  else
    say("Not in play.", { priority = "navigation", category = "on-demand" })
  end
end)

on_command("guide_to_mark", function()
  mark_goto(1) -- speaks its own outcome
end)

-- Map mode: a schematic of what the plugin reads, for debugging and for sighted
-- assistance. In a dungeon or on the overworld it draws the area's actual shape
-- from the collision map; elsewhere it is just the position/sprite overlay.
-- Integer math throughout (// is floor division) so coordinates stay whole for
-- the canvas.
function on_draw(canvas)
  local w, h = canvas.width, canvas.height
  canvas:clear(0x101828)

  local s = prev
  if s == nil then
    canvas:text(8, 8, "NO STATE YET", 0x808890)
    return
  end

  -- Header: where we are.
  local place = "TITLE"
  if in_play(s) then
    if s.indoors == 1 then
      place = string.format("ROOM %d", s.dungeon_room)
    else
      place = string.format("AREA %d", s.ow_screen)
    end
  else
    place = module_name(s.module):upper()
  end
  canvas:text(8, 8, place, 0xE0E0E0)

  -- Health hearts along the top, filled for current, outlined for the rest.
  local max_hearts = s.max_health // 8
  local cur_eighths = s.health
  for i = 0, max_hearts - 1 do
    local x = 8 + i * 9
    local filled = (i + 1) * 8 <= cur_eighths
    canvas:rect(x, 20, 7, 7, filled and 0xE03030 or 0x402028)
  end

  -- The playfield: Link's position within the current 512-pixel screen.
  local fx, fy, fw = 28, 40, 200
  canvas:rect(fx, fy, fw, fw, 0x1C2438)
  canvas:line(fx, fy, fx + fw, fy, 0x304058)
  canvas:line(fx, fy + fw, fx + fw, fy + fw, 0x304058)
  canvas:line(fx, fy, fx, fy + fw, 0x304058)
  canvas:line(fx + fw, fy, fx + fw, fy + fw, 0x304058)

  if in_play(s) then
    -- The 512-pixel window's origin, in world pixels. On the overworld it is
    -- centred on Link (tile-aligned) so he stays in the middle of the map as he
    -- walks, instead of drifting to the edge of a fixed 512-pixel block. In a
    -- dungeon it stays anchored to the room's block, which the WRAM collision grid
    -- is indexed against, so the walls keep lining up.
    local winx, winy
    if s.module == 0x09 then
      winx = (((s.x + 8) >> 3) - 32) * 8
      winy = (((s.y + 8) >> 3) - 32) * 8
    else
      winx = s.x - s.x % 512
      winy = s.y - s.y % 512
    end
    -- World pixel -> playfield screen coords, and whether a point is in the window.
    local function plot(wpx, wpy)
      return fx + (wpx - winx) * fw // 512, fy + (wpy - winy) * fw // 512
    end
    local function inwin(wpx, wpy)
      return wpx >= winx and wpx < winx + 512 and wpy >= winy and wpy < winy + 512
    end

    -- The area's real shape first, under everything else. A 64x64 tile grid maps
    -- exactly onto the 512-pixel playfield the sprites are plotted in (64 tiles x
    -- 8 px = 512), so walls and doors line up with the objects standing on them.
    local function cell(tx, ty, color)
      local x0 = fx + tx * fw // 64
      local y0 = fy + ty * fw // 64
      canvas:rect(x0, y0, (fx + (tx + 1) * fw // 64) - x0,
                  (fy + (ty + 1) * fw // 64) - y0, color)
    end

    if s.module == 0x07 then
      -- Dungeon: the 64x64 collision grid is read straight from WRAM.
      local base = DUNGEON_TILE_TABLE + (mem.u8(LOWER_LEVEL) == 1 and 0x1000 or 0)
      local data = mem.slice(base, 4096)
      if #data == 4096 then
        for ty = 0, 63 do
          for tx = 0, 63 do
            local attr = string.byte(data, ty * 64 + tx + 1)
            local color = TILE_COLOR[attr] or (attr == 0x04 and INDOOR_WALL_04)
              or (is_collidable(attr, true) and COLLIDE_COLOR) or nil
            if color then cell(tx, ty, color) end
          end
        end
      end
    elseif s.module == 0x09 and #OW_MAP16_TO_MAP8 > 0 then
      -- Overworld: each visible tile is a map16 index from the $7E2000 table,
      -- addressed through the game's live scroll offsets, then resolved to a
      -- collision attribute via the ROM tables. Drawn for the 512-pixel window
      -- around Link, aligned to the same mod-512 grid the sprites use.
      local mask_y = mem.u16(0x7E070A)
      local mask_x = mem.u16(0x7E070E)
      local ow = mem.slice(0x7E2000, 8192)
      if mask_x ~= 0 and mask_y ~= 0 and #ow == 8192 then
        local base_y = mem.u16(0x7E0708)
        local base_x = mem.u16(0x7E070C)
        local block_x = winx
        local block_y = winy
        for ty = 0, 63 do
          for tx = 0, 63 do
            local px = block_x + tx * 8
            local py = block_y + ty * 8
            local ow_tx = px >> 3
            local t = (((py - base_y) & mask_y) * 8) | ((ow_tx - base_x) & mask_x)
            local byte_off = (t >> 1) * 2
            if byte_off >= 0 and byte_off + 2 <= 8192 then
              local map16 = string.byte(ow, byte_off + 1) | (string.byte(ow, byte_off + 2) << 8)
              local attr = ow_tile_attr(map16, ow_tx, py)
              local color = TILE_COLOR[attr] or (is_collidable(attr, false) and COLLIDE_COLOR) or nil
              if color then cell(tx, ty, color) end
            end
          end
        end
      end
    end

    -- Sprites next, so Link's marker sits on top of them. Coloured by beacon
    -- class: enemies red, items yellow, people/switches green, scenery dim cyan.
    local class_col = {
      enemy = 0xF04040,
      item  = 0xF0D040,
      npc   = 0x40E060,
      minor = 0x40C0F0,
    }
    for _, sp in ipairs(sprites()) do
      if is_live(sp) and inwin(sp.x, sp.y) then
        local px, py = plot(sp.x, sp.y)
        canvas:rect(px - 1, py - 1, 3, 3, class_col[category(sp)])
      end
    end

    -- The active guidance route: the same corners the audio beacon leads through,
    -- drawn as a magenta line with a dot at each corner and the current target
    -- brightened — so the guide is legible on the map too.
    if pathfind_active and pathfind_path then
      for i = 1, #pathfind_path - 1 do
        local ax, ay = plot(pathfind_path[i][1] * 8 + 4, pathfind_path[i][2] * 8 + 4)
        local bx, by = plot(pathfind_path[i + 1][1] * 8 + 4, pathfind_path[i + 1][2] * 8 + 4)
        canvas:line(ax, ay, bx, by, 0xFF60D0)
      end
      for i, wt in ipairs(pathfind_path) do
        local px, py = plot(wt[1] * 8 + 4, wt[2] * 8 + 4)
        canvas:rect(px - 1, py - 1, 3, 3, (i == pathfind_wp) and 0xFFFFFF or 0xFF60D0)
      end
    end

    -- Only the active target — the next waypoint in the sequence — is marked (white);
    -- the route to it, across floors and all, is the pink A* line drawn above. The
    -- rest of the chain is deliberately not drawn: those are points Link is not headed
    -- to yet, and painting them just clutters the map (and, on a two-level room, drops
    -- phantom markers where a point's tile coincides with the other floor's overlay).
    -- Its own tile coordinates plot the same on either floor, so a target on the floor
    -- above/below still shows where to head. Hidden while a room sub-goal is active
    -- (clear the guard, grab the key, open the chest).
    if s.module == 0x07 and nav_chain and room_objective(s) == nil then
      local wp = nav_chain[nav_chain_i]
      if wp and wp.room == s.dungeon_room then
        local px, py = plot(wp.tx * 8 + 4, wp.ty * 8 + 4)
        canvas:rect(px - 1, py - 1, 3, 3, 0xFFFFFF)
      end
    end

    -- Debug overlay: developer aids the normal map hides, always on. Only in a
    -- dungeon, where the room and its waypoints live.
    if s.module == 0x07 then
      -- A kill-room's boundary, in a distinct red (never the pink of the nav route).
      -- When the room's fighting pit is mapped (KILL_REGION), draw a 1px rectangle on
      -- its tile bounds so the border hugs the real pit instead of framing the whole
      -- screen. Rooms with no mapped pit fall back to a frame outside the playfield.
      if kill_room(s) then
        local kc = 0xE83838
        local reg = KILL_REGION[s.dungeon_room]
        if reg then
          for _, b in ipairs(reg) do
            local x0, y0 = plot(b.w * 8, b.n * 8)             -- NW corner
            local x1, y1 = plot((b.e + 1) * 8, (b.s + 1) * 8) -- SE corner (tile far edge)
            canvas:line(x0, y0, x1, y0, kc) -- north
            canvas:line(x0, y1, x1, y1, kc) -- south
            canvas:line(x0, y0, x0, y1, kc) -- west
            canvas:line(x1, y0, x1, y1, kc) -- east
          end
        else
          for t = 1, 2 do
            canvas:line(fx - t, fy - t, fx + fw + t, fy - t, kc)             -- top
            canvas:line(fx - t, fy + fw + t, fx + fw + t, fy + fw + t, kc)   -- bottom
            canvas:line(fx - t, fy - t, fx - t, fy + fw + t, kc)             -- left
            canvas:line(fx + fw + t, fy - t, fx + fw + t, fy + fw + t, kc)   -- right
          end
        end
      end
      -- Every waypoint of the active chain that belongs to this room, each tagged
      -- with its 1-based order in the chain so the routing sequence is legible.
      -- Cyan for the ordinary points, white for the active target, so the debug
      -- markers never read as the pink route or the orange dropped markers.
      if nav_chain then
        for i, wp in ipairs(nav_chain) do
          if wp.room == s.dungeon_room and inwin(wp.tx * 8 + 4, wp.ty * 8 + 4) then
            local px, py = plot(wp.tx * 8 + 4, wp.ty * 8 + 4)
            local wc = (i == nav_chain_i) and 0xFFFFFF or 0x50D0F0
            canvas:rect(px - 1, py - 1, 3, 3, wc)
            canvas:text(px + 3, py - 3, tostring(i), wc)
          end
        end
      end

      -- Direct-pathfind phases (e.g. escorting Zelda) have no chain, but the guide
      -- still leads along an A* route drawn above as pink corners. Number those
      -- corners too — in the route's own pink, the active one white — so the
      -- immediate target always carries a number, not just chain waypoints.
      if pathfind_active and pathfind_path then
        for i, wt in ipairs(pathfind_path) do
          if inwin(wt[1] * 8 + 4, wt[2] * 8 + 4) then
            local px, py = plot(wt[1] * 8 + 4, wt[2] * 8 + 4)
            canvas:text(px + 3, py - 3, tostring(i), (i == pathfind_wp) and 0xFFFFFF or 0xFF60D0)
          end
        end
      end
    end

    -- The cross-screen overworld route, drawn through the current 512-pixel
    -- window; the segment leaving the screen edge points on toward the next area.
    -- World tiles are placed relative to Link's block, so off-window corners clip.
    if s.module == 0x09 and ow_route_goal and ow_route_path then
      local function oplot(tx, ty)
        return plot(tx * 8 + 4, ty * 8 + 4)
      end
      -- The active route to the immediate target, string-pulled, in pink.
      for i = 1, #ow_route_path - 1 do
        local ax, ay = oplot(ow_route_path[i][1], ow_route_path[i][2])
        local cx2, cy2 = oplot(ow_route_path[i + 1][1], ow_route_path[i + 1][2])
        canvas:line(ax, ay, cx2, cy2, 0xFF60D0)
      end
      if nav_chain then
        -- Past the active target, draw the rest of the chain: straight pink
        -- segments linking the remaining waypoints, then a marker on each — the
        -- next waypoint white, the rest pink — so the route reads ahead (bushes →
        -- castle door) and the immediate goal stands out. Only the overworld
        -- waypoints are drawn here; a dungeon point (one with a `room`) belongs to
        -- the map inside, not out on this screen.
        for i = nav_chain_i, #nav_chain - 1 do
          if nav_chain[i].room == nil and nav_chain[i + 1].room == nil then
            local ax, ay = oplot(nav_chain[i].tx, nav_chain[i].ty)
            local cx2, cy2 = oplot(nav_chain[i + 1].tx, nav_chain[i + 1].ty)
            canvas:line(ax, ay, cx2, cy2, 0xFF60D0)
          end
        end
        for i = nav_chain_i, #nav_chain do
          if nav_chain[i].room == nil then
            local px, py = oplot(nav_chain[i].tx, nav_chain[i].ty)
            canvas:rect(px - 1, py - 1, 3, 3, (i == nav_chain_i) and 0xFFFFFF or 0xFF60D0)
          end
        end
      else
        -- A plain single target: mark its destination white.
        local px, py = oplot(ow_route_goal[1], ow_route_goal[2])
        canvas:rect(px - 1, py - 1, 3, 3, 0xFFFFFF)
      end
    end

    -- Dropped waypoint markers in this area, as small orange squares.
    local here = area_id(s)
    for _, m in pairs(markers) do
      if m.area == here and inwin(m.tx * 8 + 4, m.ty * 8 + 4) then
        local px, py = plot(m.tx * 8 + 4, m.ty * 8 + 4)
        canvas:rect(px - 1, py - 1, 3, 3, 0xFF9020)
      end
    end

    -- Link's marker at his sprite CENTRE, not the raw $0020/$0022 which is the
    -- 16x16 sprite's top-left corner — often up in a wall tile a row or two above
    -- where he visibly stands, so the raw point reads a tile off from the ground
    -- (and the bush/entrance) beneath his feet.
    local lx, ly = plot(s.x + 8, s.y + 8)
    canvas:rect(lx - 2, ly - 2, 5, 5, 0x40FF60) -- Link

    -- A short line in the direction he faces.
    local d = DIRS[s.direction] or DIRS[6]
    canvas:line(lx, ly, lx + d.dx * 12, ly + d.dy * 12, 0xFFF060)

    canvas:text(8, h - 14, string.format("X %d Y %d", s.x, s.y), 0x9098A0)
  else
    canvas:text(fx + 8, fy + fw // 2, "NOT IN PLAY", 0x707880)
  end
end

-- "Status."
on_command("status", function()
  if prev ~= nil and prev.max_health > 0 then
    local s = prev
    say(
      string.format("%.1f of %.1f hearts. %d rupees.", hearts(s.health), hearts(s.max_health), s.rupees),
      { priority = "navigation", category = "on-demand" }
    )
  else
    say("No game state yet.", { priority = "navigation", category = "on-demand" })
  end
end)
