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
  die   = 0x7E0CBA, -- sprite_die_action: non-zero = drops a key/big-key on death
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

-- Per-room authored knowledge — which rooms gate progress on a fight, which count
-- their enemies room-wide, and where their fighting pits are — lives in
-- waypoints.lua under ROOMS, beside the chains, for the same reason: it is mapped
-- by playing, not derived, so it belongs in the file a person (and the editor) can
-- change without touching this script. Read here, never written.
--
--   kill   true, or a WP clause: is this room a kill-room right now? Some rooms set
--          no clear-tag of their own and still gate on a fight, because a guard
--          drops the key for the locked way out.
--   chambers the room's fighting chambers, as world-tile boxes. A dungeon "room" is
--          one 64-tile block, but the parts that gate progress are smaller chambers
--          walled off inside it, so a room can hold several. Bounds the enemy tally
--          when one covers Link, and outlines the pit on the debug map.


-- Is Link in a dungeon room gated on defeating enemies, and over what area?
--
-- Returns "room" or "screen", or nil. The tag says which, and the distinction is the
-- game's own: RoomTag_RoomTrigger (0x0A, 0x32) waits on Sprite_CheckIfRoomIsClear,
-- which walks every sprite slot with no bounds at all, while every other kill tag goes
-- through RoomTag_QuadrantTrigger or RoomTag_KillRoomBlock and waits on
-- Sprite_CheckIfScreenIsClear, which counts only sprites within 256x256 of the scroll
-- origin. Both then require Sprite_CheckIfOverlordsClear, so a spawner holding the room
-- open needs no separate test here.
--
-- Worth having exactly right, because the tag is what opens the door: agreeing with the
-- game about "clear" is the difference between going quiet when the room does and going
-- quiet a fight too early.
-- Globals, not locals: the main chunk is at its 200-local ceiling, so anything new
-- here hangs off the global namespace instead (the same reason SWEEP and WP do).
KILL_ROOMWIDE = { [0x0A] = true, [0x32] = true }
local function kill_room(s)
  if s.module ~= 0x07 then return nil end
  for i = 0, 1 do
    local t = mem.u8(KILL_HDR_TAG + i)
    if KILL_TAGS[t] then return KILL_ROOMWIDE[t] and "room" or "screen" end
  end
  return nil
end

-- The 256x256 window the game's own screen-clear check uses, as world pixels: the
-- scroll origin is BG2HOFS_copy2/BG2VOFS_copy2 ($7E00E2 / $7E00E8).
function on_kill_screen(sx, sy)
  local ox, oy = mem.u16(0x7E00E2), mem.u16(0x7E00E8)
  if ox == nil or oy == nil then return false end
  return (sx - ox) >= 0 and (sx - ox) < 256 and (sy - oy) >= 0 and (sy - oy) < 256
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

-- The mapped fighting chamber Link is standing in, or nil if the room maps none or
-- none covers him (a corridor between two pits, say).


-- Where a chamber is mapped and Link is in it, that chamber IS the fighting area:
-- an enemy counts only if it shares the chamber. A radius round Link is the fallback,
-- and it is a crude stand-in for the same idea — room 0x71's two guard pits are walled
-- off from each other, yet at 144 pixels a guard in the far pit lands inside the radius
-- and holds the room uncleared, or gets picked as the nearest enemy from behind a wall
-- Link cannot cross. The chamber has the wall in it; the radius does not.
-- Does a sprite at (sx, sy) count toward clearing the room Link is in? An authored
-- chamber first, since it can be finer than anything the game states; then the bound
-- the room's own kill tag implies; then a plain radius for a room with no tag at all.
-- Global so every caller shares one answer.
-- Sprites carry the floor they are on ($7E0F20, set from link_is_on_lower_level when
-- they spawn), and a two-floor room has two collision grids sharing one set of tile
-- coordinates. Without this an enemy directly above or below Link reads as standing
-- next to him: REACH is built for HIS floor, and the comparison was purely positional.
--
-- The game's own clear checks ignore the floor, but they are answering a different
-- question — whether the room is finished — and the tag answers that for us now. This
-- one only decides what Link can fight from where he stands, and he cannot fight
-- through a floor. (Value 2 is the transient the explosion path sets; treat it as
-- present on either floor rather than on neither.)
local SPRITE_FLOOR = 0x7E0F20
function enemy_floor_matches(s, slot)
  local f = mem.u8(SPRITE_FLOOR + slot)
  -- $7E00EE literal, not LOWER_LEVEL: that local is declared further down the file, and
  -- naming it here would silently read a nil global instead.
  return f == nil or f == 2 or f == mem.u8(0x7E00EE)
end

function enemy_counts(s, sx, sy, slot)
  if slot ~= nil and not enemy_floor_matches(s, slot) then return false end
  local scope = kill_room(s)
  if scope == nil then
    -- No tag to take an area from. But an authored `clear` step for this room is itself
    -- the statement that the ROOM is the fight — rooms 0x70, 0x72 and 0x80 set no tag,
    -- which is exactly why someone had to write their fights down — so the area is the
    -- room and the fill removes its walls. That is what a whole-room chamber box used to
    -- say by hand, and before that what the `giant` flag said by widening a radius.
    if WP.fights[s.dungeon_room] then return REACH.can(s, sx, sy) end
    -- Nothing says this room gates on a fight at all, so a plain radius stands.
    return math.abs(sx - s.x) <= ENEMY_ONSCREEN and math.abs(sy - s.y) <= ENEMY_ONSCREEN
  end
  -- Two independent questions, and the answer needs both. The tag says over what AREA
  -- the game checks — the whole room, or the 256x256 screen — and reachability says
  -- which of that area Link can actually get to. Area alone counted enemies through
  -- walls; reachability alone would count one the game does not require killing.
  if scope == "screen" and not on_kill_screen(sx, sy) then return false end
  return REACH.can(s, sx, sy)
end

local function nearest_pending_enemy(s)
  local best, bd
  for i = 0, 15 do
    local st = mem.u8(SPRITE.state + i)
    -- hp 0 is dead or inert (or a bystander NPC like caged Zelda) — never a pending
    -- enemy, so it can't hold a room "uncleared", especially a whole-room chamber whose
    -- reach would otherwise sweep such a sprite in from across the room.
    if st ~= nil and st ~= 0 and (mem.u8(SPRITE_FLAGS4 + i) & 0x40) == 0
        and (mem.u8(SPRITE.hp + i) or 0) > 0 then
      local sx = mem.u8(SPRITE.x_lo + i) + mem.u8(SPRITE.x_hi + i) * 256
      local sy = mem.u8(SPRITE.y_lo + i) + mem.u8(SPRITE.y_hi + i) * 256
      if enemy_counts(s, sx, sy, i) then
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

-- Nothing reads `dialog` to speak with any more — TEXT below does that from the buffer the
-- game is actually drawing from, so that only what has been shown gets read. It stays as the
-- whole-message reference: every message by id, readable without one being on screen, which
-- is what makes `dialog[0x1F]` answerable from eval_lua when working out what a scene says.

-- ── Paginated game text ─────────────────────────────────────────────────────
-- What the game has put on screen, page by page, rather than the whole message.
--
-- Reading the ROM's copy of a message tells the player things they have not been shown yet:
-- Zelda's telepathic plea is five pages that turn on a button press, and it was all being
-- read out at once the moment the box opened.
--
-- The game itself has the answer. Text_LoadCharacterBuffer expands the message it is about
-- to show into messaging_text_buffer, and RenderText_Draw_MessageCharacters walks that
-- buffer leaving dialogue_msg_read_pos at the next byte to consume — so the bytes before
-- that position are exactly what has been drawn. The buffer is pre-expanded: dictionary
-- words, the player's name and preloaded numbers are already substituted, so unlike the ROM
-- decoder there is no dictionary byte to look up here.
TEXT = {
  BUFFER = 0x7F1200, -- messaging_text_buffer (WRAM offset 0x11200, so bank $7F)
  POS = 0x7E1CD9, -- dialogue_msg_read_pos
  ID = 0x7E1CF0, -- dialogue_message_index: which message is being shown
  MAX = 0x400, -- a bound, so a corrupt buffer cannot spin; messages are far shorter
  END = 0x7F,
}

-- The commands the renderer stops on to wait for the player, which is what makes a page a
-- page. Waitkey (0x7E) is the ordinary page turn; the Choose family halts on a prompt; 0x7F
-- ends the message. RenderText_Draw_MessageCharacters leaves read_pos sitting ON each of
-- these instead of stepping past it, so the resting position means "this much is drawn".
-- Verified live: mid-message read_pos rested at 28, the byte there 0x7E.
TEXT.BREAKS = {
  [0x68] = true, -- Choose
  [0x69] = true, -- Item
  [0x6F] = true, -- Selchg
  [0x71] = true, -- Choose3
  [0x72] = true, -- Choose2
  [0x7E] = true, -- Waitkey
  [0x7F] = true, -- end of message
}

-- Decode the buffer from `from` until `upto`, and then to the end of a word the boundary
-- splits. Returns the text and where it actually stopped, so a caller reading page by page
-- can start the next one past a word it has already spoken.
--
-- The word completion is what stops a half-drawn line reading as half a word — asking for
-- the text while "Help me" is still typing should not say "Help m". It only extends when the
-- boundary falls INSIDE a word: landing just after a space reads no further, or every page
-- would give away the first word of the next one.
function TEXT.decode(from, upto)
  local out, i, in_word = {}, from, false
  local function take()
    local b = mem.u8(TEXT.BUFFER + i)
    if b == nil or b == TEXT.END then return nil end
    local ch, step = "", 1
    if b <= 0x5E then
      ch = ALPHABET[b + 1] or ""
    elseif b >= 0x67 and b <= 0x7E then
      local name = CMD_NAMES[b - 0x67 + 1]
      -- The line and scroll commands are where one line ends and the next begins, which is
      -- a word gap even though the message spells no space there.
      if name == "Scroll" or name == "1" or name == "2" or name == "3" then ch = " " end
      step = CMD_LENGTHS[b - 0x67 + 1] or 1
    end
    i = i + step
    return ch
  end

  while i < upto and i < TEXT.MAX do
    local ch = take()
    if ch == nil then return normalize(table.concat(out)), i end
    out[#out + 1] = ch
    -- Only a letter or digit leaves us mid-word. Ending on punctuation means the word
    -- finished, and extending there would swallow the next page's first word: a page ending
    -- "Help me!" must not go on to say the "Now" that starts the page after it.
    if ch ~= "" then in_word = ch ~= " " and ch:match("%w") ~= nil end
  end

  while in_word and i < TEXT.MAX do
    local at = i
    local ch = take()
    if ch == nil or ch == " " then
      i = at -- leave the boundary unconsumed, so the next page starts on it
      break
    end
    -- Step over a zero-width command rather than stopping at it: the break itself sits
    -- between the halves of a word split across a page, so this is how the rest is reached.
    if ch ~= "" then
      out[#out + 1] = ch
      if ch:match("%w") == nil then break end -- trailing punctuation closes the word
    end
  end

  return normalize(table.concat(out)), i
end

-- Where the page starting at `from` ends: the next command the renderer will stop on.
--
-- Steps by command length rather than byte by byte, so a command's parameter can never be
-- mistaken for a break — a Speed or Sound parameter is free to be 0x7E.
function TEXT.next_break(from)
  local i = from
  while i < TEXT.MAX do
    local b = mem.u8(TEXT.BUFFER + i)
    if b == nil then return nil end
    if TEXT.BREAKS[b] then return i end
    i = i + ((b >= 0x67 and b <= 0x7E) and (CMD_LENGTHS[b - 0x67 + 1] or 1) or 1)
  end
  return nil
end

-- The start of the page `pos` falls inside: just past the last break before it, or 0 if
-- there is none. Steps by command length for the same reason next_break does.
function TEXT.page_at(pos)
  local from, i = 0, 0
  while i < pos and i < TEXT.MAX do
    local b = mem.u8(TEXT.BUFFER + i)
    if b == nil then break end
    if TEXT.BREAKS[b] then from = i + 1 end
    i = i + ((b >= 0x67 and b <= 0x7E) and (CMD_LENGTHS[b - 0x67 + 1] or 1) or 1)
  end
  return from
end

-- The whole of the page being shown, for reading on demand. The whole page rather than the
-- part drawn so far: it is all in the buffer, and the player asking wants the page, not a
-- race with the typewriter.
function TEXT.shown()
  local from = TEXT.from or 0
  local brk = TEXT.next_break(from)
  if brk == nil then return TEXT.last end
  local text = TEXT.decode(from, brk)
  if text ~= "" then return text end
  return TEXT.last
end

-- Read each page as it BEGINS to appear, not once it has finished.
--
-- Waiting for a page to finish drawing means waiting out the typewriter before hearing a word
-- of it, and the whole page is sitting in the buffer already — so the moment the renderer
-- starts on a page, the page can be spoken in full. What the renderer is still doing is
-- putting it on screen for anyone reading it with their eyes.
--
-- So the trigger is the page having started rather than the renderer having come to rest:
-- speak the page at `from` as soon as read_pos has moved into it, then wait for read_pos to
-- pass that page's break, which is the player turning it.
function TEXT.update(s)
  local id, pos = mem.u16(TEXT.ID), mem.u16(TEXT.POS)
  if id == nil or pos == nil then return end

  -- A new message, or the same one shown again — Text_LoadCharacterBuffer zeroes read_pos
  -- either way, so a position behind where we had got to means a fresh message.
  --
  -- Progress is deliberately NOT forgotten when the module stops being 0x0E. The screen can
  -- leave and come back mid-message — the opening brings the lights up part way through
  -- Zelda's plea — and starting over there replayed the whole message: read_pos was already
  -- past every break, so page after page met its trigger on consecutive frames.
  if id ~= TEXT.msg or pos < (TEXT.from or 0) then
    TEXT.msg, TEXT.spoke, TEXT.next_from, TEXT.last = id, nil, nil, nil
    -- Not necessarily the top of the message: arriving with the renderer already part way in,
    -- as a plugin reload mid-scene does, should read the page it is on and not everything
    -- before it. For a genuinely new message read_pos is 0 and this is 0 too.
    TEXT.from = TEXT.page_at(pos)
  end

  -- Track wherever the message is, but only speak while the box is actually up.
  if s.module ~= 0x0E then return end

  if TEXT.spoke ~= nil then
    -- Said already. Nothing more until read_pos passes its break, which only happens when
    -- the player turns the page.
    if pos <= TEXT.spoke then return end
    TEXT.from, TEXT.spoke, TEXT.next_from = TEXT.next_from, nil, nil
  end

  -- Wait for the page to have actually begun, so nothing is said before the box is up.
  if pos <= TEXT.from then return end
  local brk = TEXT.next_break(TEXT.from)
  if brk == nil then return end

  local text, stop = TEXT.decode(TEXT.from, brk)
  TEXT.spoke = brk
  -- The next page starts after this one's break — or after the word that ran past it, when a
  -- word was split across it, so the half already spoken is not said again.
  TEXT.next_from = (stop > brk + 1) and stop or (brk + 1)
  if text ~= "" then
    TEXT.last = text
    -- `always`: the game's own story is spoken at any verbosity. A low chatter setting trims
    -- the guide's routine callouts, never the plot.
    say(text, { priority = "navigation", category = "dialog", always = true })
  end
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
  -- the map) instead of the door it means to take. The waypoint's own locked-door gate
  -- still decides whether to aim beyond the door; this just lets the path cross it.
  if attr >= 0xF0 and attr <= 0xFF then return mem.u8(0x7EF36F) > 0 end
  if IMPASSABLE[attr] then return false end
  if s.module == 0x07 and attr == 0x04 then return false end -- indoor wall
  -- 0x1C is the upper layer's overlay mask (zelda3 TileBehavior_OverlayMask_1C): the
  -- raised platform is absent here. On the UPPER floor (level 0) that means this square
  -- is a one-way drop to the level below, not standable — block it so A* can't walk
  -- "across" the hole as flat ground. On the LOWER floor (level 1) the same mask means
  -- Link is standing UNDER that upper platform: solid, walkable ground — so it must NOT
  -- be blocked, or a lower-floor route past a stair's masked approach (e.g. 0x71) breaks.
  if s.module == 0x07 and attr == 0x1C and level == 0 then return false end
  return true
end

-- ===========================================================================
-- Reachability: which of this room can Link actually walk to from where he stands?
--
-- A flood fill from his tile over passable ground, once, answering for all sixteen
-- sprite slots at a stroke. It replaces the hand-authored chamber boxes that used to
-- bound the enemy tally, and it is strictly better than a rectangle: it follows the
-- room's real walls, so a chamber that is not rectangular works, and a wall that
-- opens when a switch is hit stops being a wall on the next rebuild. Room 0x71's two
-- guard pits are separated by green ledge tiles that are right there in the collision
-- data — there was never a need to write their corners down.
--
-- Rebuilt only when it can have changed: a different room or floor, a different
-- 512-pixel window, Link standing somewhere the current fill does not cover (which is
-- exactly the moment he crosses into a new region), or the throttle expiring so a
-- door that just opened is picked up. Walking about inside one region reuses it.
--
-- One caveat inherited from tile_passable: it counts a locked door as crossable while
-- Link holds a key, because the router needs to plan through it. So holding a key can
-- make the next chamber reachable, and its enemies start counting. That is arguable
-- either way — he CAN get there — and it is the game's own notion of "can reach"
-- rather than a wrong answer.
-- ===========================================================================
REACH = { set = nil, room = nil, level = nil, ox = nil, oy = nil, probe = 0 }
REACH.PROBE = 15
REACH.DIRS = { { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }

function REACH.build(s, level, ox, oy, seed)
  local set, queue = { [seed] = true }, { seed }
  local head = 1
  while head <= #queue do
    local i = queue[head]
    head = head + 1
    local lx, ly = i % 64, i // 64
    for _, d in ipairs(REACH.DIRS) do
      local nx, ny = lx + d[1], ly + d[2]
      if nx >= 0 and nx < 64 and ny >= 0 and ny < 64 then
        local j = ny * 64 + nx
        if not set[j] and tile_passable(s, ox + nx, oy + ny, level) then
          set[j] = true
          queue[#queue + 1] = j
        end
      end
    end
  end
  REACH.set, REACH.room, REACH.level, REACH.ox, REACH.oy = set, s.dungeon_room, level, ox, oy
end

-- Can Link reach the tile containing world pixel (sx, sy)?
function REACH.can(s, sx, sy)
  local level = mem.u8(LOWER_LEVEL)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local seed = ((s.y >> 3) - oy) * 64 + ((s.x >> 3) - ox)
  REACH.probe = REACH.probe - 1
  if REACH.set == nil or REACH.room ~= s.dungeon_room or REACH.level ~= level
    or REACH.ox ~= ox or REACH.oy ~= oy or not REACH.set[seed] or REACH.probe <= 0
  then
    REACH.probe = REACH.PROBE
    REACH.build(s, level, ox, oy, seed)
  end
  local tx, ty = sx >> 3, sy >> 3
  if tx < ox or tx >= ox + 64 or ty < oy or ty >= oy + 64 then return false end
  return REACH.set[(ty - oy) * 64 + (tx - ox)] == true
end

-- ── Hazards underfoot ───────────────────────────────────────────────────────
-- A pit is the one obstacle that punishes you for walking into it instead of
-- stopping you, so the router treating it as impassable is not enough: a player
-- who cannot see the floor needs to know the edge is there before he steps off it.
-- Same idea as the bush cue — look where Link is heading — but a tone rather than a
-- word, and no words at all. Speech is the one channel everything else competes for,
-- and "Pit." is both slower to arrive than the danger it describes and gone the
-- instant it is spoken. A tone is continuous: it is there for as long as the edge is,
-- it says WHERE by panning as Link turns, and it costs nothing that was going to be
-- said about the room. Low and fast-pulsing, the vocabulary the enemy-weapon beacon
-- already uses for "this will hurt you", positioned on the faced tile so sweeping
-- reads the edge out.
--
-- It is not gated on the guide either. A bush cue is routing advice, useful only while
-- being led somewhere; a pit is a hazard whether or not the guide is on.
--
-- Dungeon only. The overworld gives entrance holes the same pit attribute — the
-- castle intro drop among them — and they are places you are meant to fall into,
-- so warning there would cry wolf at every doorway.
--
-- Tile classes are the game's own (zelda3 tile_detect.c TileBehavior_Pit): 0x20,
-- plus the 0xB0-0xBD variants dungeons use for holes with a set destination.
-- Everything hangs off one table to spare the main chunk's local budget.
HAZARD = { facing = false, ticks = 0 }
HAZARD.PIT = {}
do
  HAZARD.PIT[0x20] = true
  for a = 0xB0, 0xBD do HAZARD.PIT[a] = true end
end
-- A sharp, quick notification rather than a sustained hum: `ping` makes each cycle an
-- attack-decay strike instead of a swell, and BLIP frames at this tremolo is about one
-- strike, so facing a pit produces a single crisp blip. Being brief also separates it
-- from the enemy-weapon tone it used to sit next to in pitch — a one-shot and a
-- continuous drone cannot be confused however close their frequencies are.
-- Pitched above everything else in the vocabulary. The object tones run 0.7 to 2.0 and
-- the guide's sonar sits at 3.0-3.4; putting the hazard at 4.0 leaves it nothing to be
-- confused with, and high-and-sharp reads as a notification demanding attention where
-- low-and-slow reads as a thing sitting in the room.
HAZARD.TONE = { pitch = 4.0, tremolo = 8.0, volume = 0.8 }
HAZARD.BLIP = 8 -- frames it sounds for, about a sixth of a second

-- The nearest pit within REACH tiles in the direction Link faces, as world pixels, or
-- nil. Looking only at the tile immediately ahead gave about one frame of warning at
-- walking speed, which is no warning at all; scanning further turns it into a couple of
-- steps' notice.
--
-- The scan stops at anything solid, because a pit behind a wall is not a step Link can
-- take and warning about it would cry wolf every time he walked along the far side of
-- one. A pit is itself impassable, so it is tested for before the wall check.
HAZARD.REACH = 5 -- tiles ahead, about two steps of warning

function HAZARD.pit_ahead(s)
  local dir = s.direction
  local dx = (dir == 4 and -8) or (dir == 6 and 8) or 0
  local dy = (dir == 0 and -8) or (dir == 2 and 8) or 0
  if dx == 0 and dy == 0 then return nil end
  local px, py = s.x + 8, s.y + 12
  for i = 1, HAZARD.REACH do
    local ax, ay = px + dx * i, py + dy * i
    local a = tile_attr_at(s, ax, ay)
    if a == nil then return nil end
    if HAZARD.PIT[a] then return ax, ay end
    if not tile_passable(s, ax >> 3, ay >> 3) then return nil end
  end
  return nil
end

function HAZARD.clear()
  HAZARD.facing = false
  HAZARD.ticks = 0
  beacon.clear("hazard")
end

function HAZARD.update(s)
  if s == nil or s.module ~= 0x07 then HAZARD.clear(); return end
  local ax, ay = HAZARD.pit_ahead(s)
  if ax == nil then HAZARD.clear(); return end
  -- Fires on turning ONTO the pit, not for as long as he faces it: a warning about a
  -- step he is about to take has said its piece once he has heard it, and holding the
  -- tone would bury the enemy and guide tones under it while he edges along a ledge.
  -- Facing away re-arms it, so every fresh approach gets its own blip.
  if not HAZARD.facing then
    HAZARD.facing = true
    HAZARD.ticks = HAZARD.BLIP
  end
  if HAZARD.ticks > 0 then
    HAZARD.ticks = HAZARD.ticks - 1
    -- Positioned on the pit itself, so the blip pans toward it and its distance is
    -- audible in the stereo offset rather than having to be described.
    beacon.set("hazard", { x = ax - s.x, y = ay - s.y, pitch = HAZARD.TONE.pitch,
      tremolo = HAZARD.TONE.tremolo, ping = true, volume = HAZARD.TONE.volume })
  else
    beacon.clear("hazard")
  end
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

  -- Menus first, and outside every in-play gate: the file select and the title screens
  -- are exactly where Link does not exist yet, and they are unusable unheard.
  MENU.update(now)

  -- Game text likewise: it is read as the game draws each page, so it has to be watched
  -- every frame rather than at a module change, and a text box is not in-play either.
  TEXT.update(now)

  -- Turn navigation on by itself at the very start of the quest — once Link is up
  -- out of bed and controllable in his house — so the opening guidance leads without
  -- the player first pressing the key. Setting nav_active is enough: nav_update,
  -- later this frame, does the re-aim. Edge-triggered, and cleared once he leaves the
  -- opening, so it re-arms on a fresh start but a deliberate toggle-off stays off.
  if at_quest_opening(now) then
    if not intro_nav_armed then
      intro_nav_armed = true
      nav_active = true
      -- say, not nav_say: that wrapper is a file-local declared much further down, so
      -- naming it here calls a nil global and takes the whole frame's speech with it.
      say("Navigation on.", { priority = "navigation", category = "on-demand" })
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

  -- Game text is read page by page as the game draws it, from TEXT.update below, rather than
  -- announced whole when the box opens (module 0x0E). Reading it on the module change meant
  -- reading pages the player had not turned to yet.

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
  FACE.update(now)
  HAZARD.update(now) -- a pit in front of Link, guide or no guide
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

-- "Read text" — re-read the page currently on screen, a custom command. The page, not the
-- message: the rest of it has not been shown yet.
on_command("read_text", function()
  local text = TEXT.shown()
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

-- ===========================================================================
-- Waypoint predicate compiler.
--
-- A waypoint's `gate` (may the guide aim here yet?) and `done` (has this errand
-- already been carried out?) used to be hand-written closures sitting beside the
-- coordinates. They now arrive from waypoints.lua as declarative clauses —
-- {"keys"}, {"tile_outside", 0xF0, 0xFF} — and WP.compile turns each into the
-- closure the chain driver calls. The point is not brevity: it makes a chain pure
-- data, so waypoints.lua stays a file the editor (scripts/waypoints.py) can read,
-- rewrite and hand back without ever parsing Lua code.
--
-- Every clause is evaluated against a focus tile, which defaults to the
-- waypoint's own live tx/ty/level — so "the door I am standing on" needs no
-- coordinates at all, and `at` moves the focus when a waypoint depends on some
-- other square. The clause vocabulary is documented in waypoints.lua, next to the
-- chains that use it.
-- ===========================================================================
WP = {}

WP.PRED = {
  -- The shape nearly every state test takes, because the game rewrites a tile out
  -- of its class once the thing is done: a locked door leaves 0xF0-0xFF when
  -- opened, a chest leaves 0x58-0x5D when looted, a push-block leaves 0x70-0x7F
  -- when shoved. An unreadable tile (nil — not in a dungeon) counts as outside.
  tile_outside = function(s, wp, c, tx, ty, level)
    local a = tile_attr_at(s, tx * 8, ty * 8, level)
    return a == nil or a < c[2] or a > c[3]
  end,
  tile_inside = function(s, wp, c, tx, ty, level)
    local a = tile_attr_at(s, tx * 8, ty * 8, level)
    return a ~= nil and a >= c[2] and a <= c[3]
  end,
  -- {"at", tx, ty, level, clause}: the same clause about a different tile.
  at = function(s, wp, c) return WP.test(s, wp, c[5], c[2], c[3], c[4] or 0) end,
  ["not"] = function(s, wp, c, tx, ty, level) return not WP.test(s, wp, c[2], tx, ty, level) end,
  any = function(s, wp, c, tx, ty, level)
    for i = 2, #c do if WP.test(s, wp, c[i], tx, ty, level) then return true end end
    return false
  end,
  all = function(s, wp, c, tx, ty, level)
    for i = 2, #c do if not WP.test(s, wp, c[i], tx, ty, level) then return false end end
    return true
  end,
  -- Small keys for the current dungeon ($7EF36F). Covers the window after Link has
  -- the key but before he spends it at the door, so a gate keyed on a real game
  -- signal re-arms itself rather than latching on a hand-set flag.
  keys = function(s, wp, c)
    local v = mem.u8(0x7EF36F)
    return v ~= nil and v >= (c[2] or 1)
  end,
  -- A tracked push sprite at its end stop: the Movable Mantle latches sprite_G
  -- ($7E0ED0 + slot) to 0x90 when fully shoved (zelda3 Sprite_EE_MovableMantle).
  pushed = function(s, wp, c)
    return wp.slot ~= nil and mem.u8(0x7E0ED0 + wp.slot) == (c[2] or 0x90)
  end,
  -- Has a room's chest been opened? Per-room permanent progress lives at
  -- $7EF000 + room*2, one u16 each, bit 0x8000 flipping when the chest is opened
  -- and never clearing. Defaults to the subject's own room, so a room rule needs no
  -- argument; pass one to ask about a different room.
  chest_opened = function(s, wp, c)
    local room = c[2] or (wp and wp.room) or s.dungeon_room
    if room == nil then return false end
    local v = mem.u16(0x7EF000 + room * 2)
    return v ~= nil and (v & 0x8000) ~= 0
  end,
  -- Has this room's lever been pulled? The game keeps that as a property of the room
  -- rather than of the lever: pulling a Good Switch sets a state-change flag, and the
  -- room's own tag routine consumes it by lowering the door it was blocking
  -- (RoomTag_RoomTrigger_BlockDoor waits on exactly that pair). So the durable answer is
  -- the blocked-door flag, dung_flag_trapdoors_down at $7E0468, clearing to 0 — read from
  -- the game rather than tracked by us, and true of the room however Link got there.
  lever_pulled = function(s, wp, c)
    local v = mem.u8(0x7E0468)
    return v ~= nil and v == 0
  end,
  -- Save/WRAM bytes, for the progress the tiles cannot report.
  byte = function(s, wp, c)
    local v = mem.u8(c[2])
    return v ~= nil and v >= (c[3] or 1) and v <= (c[4] or 0xFF)
  end,
  bit = function(s, wp, c)
    local v = mem.u8(c[2])
    return v ~= nil and (v & c[3]) ~= 0
  end,
}

-- Evaluates one clause. An absent or unrecognised clause reads as true, so a typo
-- in waypoints.lua can never wedge the guide by gating a waypoint shut forever —
-- it degrades to an ungated waypoint, which is the pre-gate behaviour.
function WP.test(s, wp, c, tx, ty, level)
  if type(c) ~= "table" then return true end
  local f = WP.PRED[c[1]]
  if f == nil then return true end
  -- A room rule's subject is a room, not a tile, so it carries no position; default
  -- to the origin rather than nil so a tile clause misused there reads a tile
  -- instead of erroring.
  if tx == nil then tx, ty, level = wp.tx or 0, wp.ty or 0, wp.level or 0 end
  return f(s, wp, c, tx, ty, level) and true or false
end

function WP.compile(c)
  return function(s, wp) return WP.test(s, wp, c) end
end

-- Compiles a chain's declarative gate/done clauses into the closures the driver
-- calls, in place, and hands the chain back.
function WP.chain(list)
  for _, wp in ipairs(list) do
    if type(wp.gate) == "table" then wp.gate = WP.compile(wp.gate) end
    if type(wp.done) == "table" then wp.done = WP.compile(wp.done) end
  end
  return list
end

-- ===========================================================================
-- Waypoint kinds.
--
-- A waypoint is a step in the route, and its kind says what satisfies it. Most are
-- a place to stand, but a route is not only places: a room whose enemies must be
-- cleared, the one enemy carrying the key, a chest to open, a locked door to
-- unlock, a cabinet to shove — each is a step you take in order, and each used to
-- be modelled somewhere else. Kill rooms were a keyed table consulted per frame,
-- and the errands inside a room were a priority list that OVERRODE the chain. So
-- "clear room 0x70, then leave by its west door" was two unrelated mechanisms with
-- an override between them, when it is plainly two consecutive steps.
--
-- One list now, in route order. A kind supplies up to three things:
--   target(s, wp)  where to lead, as world pixels, or nil if there is nowhere to
--                  walk right now (a room held open by a spawner with no position).
--                  Resolved every re-probe, so a kind can track something that moves.
--   done(s, wp)    is the step satisfied? An authored `done` clause still wins, so
--                  a waypoint can override its kind's default.
--   cue            spoken once on arming, for a step whose requirement is not
--                  obvious from a tone ("Defeat all enemies.").
-- A waypoint with no kind is a place, which is why every existing chain keeps
-- working untouched.
-- ===========================================================================
KIND = {}

KIND.spot = {
  target = function(s, wp) return walkable_near(s, wp.tx * 8 + 4, wp.ty * 8 + 4, wp.level) end,
}

-- The room's enemies, all of them. tx/ty are optional here and only a fallback for
-- the map: the step is about the room, and what to walk to is whichever enemy is
-- nearest right now.
KIND.clear = {
  cue = "Defeat all enemies.",
  target = function(s, wp)
    local e = nearest_pending_enemy(s)
    if e then return walkable_near(s, e[1], e[2]) end
    if wp.tx then return walkable_near(s, wp.tx * 8 + 4, wp.ty * 8 + 4, wp.level) end
  end,
  done = function(s, wp)
    return nearest_pending_enemy(s) == nil and not overlords_pending()
  end,
}

-- One particular enemy. `carries = "key"` picks the one that still drops a key on
-- death, which is the case that matters: a guard flanking a locked door.
KIND.enemy = {
  cue = "Defeat the enemy holding the key.",
  target = function(s, wp)
    local e = wp.carries == "key" and key_holder(s) or nearest_pending_enemy(s)
    if e then return walkable_near(s, e[1], e[2]) end
  end,
  done = function(s, wp)
    if wp.carries == "key" then return key_holder(s) == nil end
    return nearest_pending_enemy(s) == nil
  end,
}

-- A chest, at its own tile. Done when the tile stops reading as a chest — the game
-- rewrites it on opening, so no flag is needed.
KIND.chest = {
  cue = "Open the chest.",
  target = KIND.spot.target,
  done = function(s, wp)
    local a = tile_attr_at(s, wp.tx * 8, wp.ty * 8, wp.level)
    return a == nil or not CHEST_TILES[a]
  end,
}

-- A locked or flaggable door, at its own tile. Done once it stops reading as one.
KIND.gate = {
  target = KIND.spot.target,
  done = function(s, wp)
    local a = tile_attr_at(s, wp.tx * 8, wp.ty * 8, wp.level)
    return a == nil or a < 0xF0 or a > 0xFF
  end,
}

-- Something to shove. A tracked sprite (the Movable Mantle) latches sprite_G at its
-- end stop; a plain tile push-block simply stops reading as one.
KIND.push = {
  target = KIND.spot.target,
  done = function(s, wp)
    if wp.slot ~= nil then return mem.u8(0x7E0ED0 + wp.slot) == (wp.latch or 0x90) end
    local a = tile_attr_at(s, wp.tx * 8, wp.ty * 8, wp.level)
    return a == nil or a < 0x70 or a > 0x7F
  end,
}

-- A waypoint's kind, defaulting to a place to stand. An unknown kind reads as a
-- place too rather than breaking the route, the same forgiveness an unknown clause
-- gets: bad data should degrade, not wedge.
function KIND.of(wp)
  return (wp.kind and KIND[wp.kind]) or KIND.spot
end

-- Where to lead for this waypoint, as world pixels, or nil if nowhere yet.
function KIND.target(s, wp)
  return KIND.of(wp).target(s, wp)
end

-- Is this waypoint's errand carried out? An authored clause wins over the kind's
-- own rule, so a waypoint can say something its kind does not know.
function KIND.done(s, wp)
  if wp.done then return wp.done(s, wp) and true or false end
  local d = KIND.of(wp).done
  return d ~= nil and d(s, wp) and true or false
end

-- The authored chains themselves live in waypoints.lua, loaded ahead of this
-- script by the manifest. Each carries its own prose: what the chain is for, and
-- why each waypoint sits where it does.
local UNCLE_APPROACH = WP.chain(WAYPOINTS.UNCLE_APPROACH)
local COURTYARD = WP.chain(WAYPOINTS.COURTYARD)
local SANCTUARY = WP.chain(WAYPOINTS.SANCTUARY)

-- Every authored errand, indexed by the room it happens in.
--
-- A step that can be CLEARED — a room's enemies, a chest, a locked door, a block to
-- shove — is worth doing whenever Link is standing in that room, whichever chain it
-- was written in and whatever the quest is currently pointed at. Room 0x70 is the
-- case: its fight is a step in COURTYARD, but COURTYARD's goal completes at progress
-- 2, so a later backtrack into the room had no chain to consult and the guards
-- blocking the passage went unmentioned. That was the hole I plugged with a separate
-- room-scoped rule; indexing the errands closes it properly, because the step itself
-- is what says the room gates on a fight.
--
-- A plain place is deliberately NOT indexed. An errand is worth doing on sight; a
-- position is only meaningful inside the route it belongs to, and leading Link to one
-- from a chain that is not running would drag him along a route he is not on. "Has
-- something to satisfy" is exactly the line between the two.
WP.errands = {}
-- Rooms whose authored steps speak for the ENEMIES specifically. Kept apart from the
-- errand index because a chest or a door says nothing about a fight: room 0x71 has an
-- authored locked door and no authored fight, so the enemy objectives must still speak
-- there or its guards go unmentioned.
WP.fights = {}
for _, list in ipairs({ UNCLE_APPROACH, COURTYARD, SANCTUARY }) do
  for _, wp in ipairs(list) do
    if wp.room and KIND.of(wp).done ~= nil then
      WP.errands[wp.room] = WP.errands[wp.room] or {}
      local into = WP.errands[wp.room]
      into[#into + 1] = wp
      if wp.kind == "clear" or wp.kind == "enemy" then WP.fights[wp.room] = true end
    end
  end
end

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
    -- Ask the kind where to lead: a room-clear has no place of its own, so its target
    -- is whichever enemy is nearest, and it may be nowhere at all this instant.
    local gx, gy = KIND.target(s, wp)
    if gx then route_set_goal(s, gx, gy, wp.level) end
  elseif wp.tx then
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

-- Lead one step along the active chain's dungeon leg: pick the waypoint of Link's
-- current room to head for, and aim the in-room pathfinder at it. Two pick policies,
-- because two kinds of chain want opposite ends of the room's waypoints:
--
--   * An authored quest chain is one long linear route, so the target is the LAST
--     waypoint of this room Link can currently REACH. A waypoint behind a locked
--     door is unreachable, so the guide leads to the door (an earlier waypoint)
--     until it opens, then advances past it. No progress bookkeeping and
--     backtrack-proof: any room re-aims at its last reachable waypoint.
--   * A generated sweep chain (`chain.sweep`) is a set of errands in one room with
--     no order forced on it, so the target is the FIRST reachable one — and since
--     the sweep keeps its chain sorted nearest-first, that is the closest one left.
--
-- A two-level room is ONE contiguous space to the pathfinder: it crosses the
-- layer-swap stairs on its own, so a waypoint is a candidate whatever floor it is
-- on — reachability (plan_path across floors, one-way drops respected) is the only
-- test. Each candidate is snapped and planned on its own floor (wp.level). Re-aim
-- on a floor change too, since the flip opens or closes cross-floor routes (a
-- one-way drop is gone once taken). A global so the sweep driver can call it; it
-- closes over the chain bookkeeping locals above either way.
function chain_dungeon_leg(s)
  local ltx, lty = s.x >> 3, s.y >> 3
  local level = mem.u8(LOWER_LEVEL)
  local reaimed = chain_last_room ~= s.dungeon_room or chain_last_level ~= level
  chain_probe_in = chain_probe_in - 1
  -- Keep following a still-valid route; re-pick the target only when not following
  -- one, when Link changed rooms or floors, or on the throttled re-probe (which
  -- catches a door that just opened, making a further waypoint reachable).
  local cur = nav_chain[nav_chain_i]
  local following = pathfind_active and cur and cur.room == s.dungeon_room and not reaimed
  if not following or chain_probe_in <= 0 then
    chain_probe_in = 12
    local pick, pgx, pgy, plevel
    for i, wp in ipairs(nav_chain) do
      if wp.room == s.dungeon_room and (wp.gate == nil or wp.gate(s, wp)) then
        if KIND.done(s, wp) then
          -- Its errand is already carried out (a chest looted, a room cleared, a block
          -- fully shoved), so it is simply not a target this frame — skipping it needs
          -- nothing recorded.
          --
          -- It must NOT be marked arrived. `arrived` means Link physically got there, and
          -- it is what lets a `via` step stop the scan running past it; latching it from a
          -- done reading made that permanent, and a done reading is not. Room 0x80 is the
          -- case: entering it, its enemies have not spawned yet, so its `via` clear step
          -- reads done for a frame, latches, and can never hold the scan again — the guide
          -- goes straight to Zelda's cell past a Ball-and-Chain Trooper carrying the key.
        else
          -- Where to lead depends on the kind: a place resolves to its own tile, a
          -- room-clear to whichever enemy is nearest this instant.
          local gx, gy = KIND.target(s, wp)
          if gx and plan_path(s, ltx, lty, gx >> 3, gy >> 3, wp.level) then
            pick, pgx, pgy, plevel = i, gx, gy, wp.level
            if nav_chain.sweep then break end -- nearest-first: the first reachable errand wins
          end
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
      -- Arrival is by proximity only for a waypoint that HAS a place: standing by a
      -- chest or a door is arriving, and its `done` then retires it once opened. A
      -- room-clear has no place of its own — its target is whichever enemy is nearest
      -- — so nothing but `done` can retire it, and the guide keeps leading until the
      -- room is quiet rather than falling silent next to the first enemy.
      local reached = wp.tx ~= nil
        and math.abs(ltx - wp.tx) + math.abs(lty - wp.ty) <= CHAIN_REACH
        and (wp.level or 0) == level
      if reached then
        nav_chain.arrived = math.max(nav_chain.arrived or 0, pick) -- clears any `via` gate at/behind here
        if wp.arrival and not chain_cued[pick] then nav_say(wp.arrival); chain_cued[pick] = true end
        pathfind_stop() -- arrived; go quiet until the next waypoint opens up
      else
        -- The waypoint's own line if it has one, else the kind's: a step whose
        -- requirement a tone cannot convey says so once ("Defeat all enemies."). `quiet`
        -- opts a step out of its kind's cue, for one whose kind is right but whose
        -- narration is not wanted — the Sanctuary chest is led to as part of the escort,
        -- and "Open the chest." there is stating the obvious over the top of it.
        local line = wp.say or (not wp.quiet and KIND.of(wp).cue) or nil
        if line and not chain_said[pick] then nav_say(line); chain_said[pick] = true end
        route_set_goal(s, pgx, pgy, plevel)
      end
    end
  end
  chain_last_room = s.dungeon_room
  chain_last_level = level
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
-- Bookkeeping for the room objective, global so it costs the main chunk no locals.
-- `said` is the spoken latch (one cue per objective per room), `lapse` counts frames
-- with no target so a flicker is not mistaken for the objective being finished.
ROOMOBJ = { room = nil, said = nil, lapse = 0, arrived = nil }
ROOMOBJ.LAPSE = 30 -- half a second of no target before believing it is over

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

-- A specific enemy made into an objective. Some rooms hang a key on ONE designated
-- enemy (zelda3 sets sprite_die_action, which makes Sprite_DoTheDeath drop a small/big
-- key when it dies) — so the guide should lead Link to THAT enemy, not the nearest
-- random one, when the key is what he is here for. The nearest live match, as {x, y}, or
-- nil. Global (not local) to stay under the chunk's 200-local cap and to let MCP inspect
-- it, like the PUSH movable-object helpers.
-- The enemy still carrying a key, bounded exactly like the room-clear tally is. It
-- used to scan every slot unbounded, which in room 0x71 meant targeting the guard in
-- the far pit through the wall between them: the guide aimed at a soldier Link could
-- not reach and then sat on that goal.
function key_holder(s)
  local best, bd
  for i = 0, 15 do
    local st = mem.u8(SPRITE.state + i)
    if st ~= nil and st ~= 0 and (mem.u8(SPRITE.hp + i) or 0) > 0
        and mem.u8(SPRITE.die + i) ~= 0 then
      local sx = mem.u8(SPRITE.x_lo + i) + mem.u8(SPRITE.x_hi + i) * 256
      local sy = mem.u8(SPRITE.y_lo + i) + mem.u8(SPRITE.y_hi + i) * 256
      if enemy_counts(s, sx, sy, i) then
        local d = math.abs(sx - s.x) + math.abs(sy - s.y)
        if bd == nil or d < bd then best, bd = { sx, sy }, d end
      end
    end
  end
  return best
end

-- ── Room sweeps ─────────────────────────────────────────────────────────────
-- "Show me everything in this room." A sweep is a generated waypoint chain rather
-- than an authored one: a collector lists what the room still holds, each item
-- becomes a waypoint that clears itself when its own errand is done, and the chain
-- is re-collected as the room changes under Link. Everything downstream — the A*
-- route, the guide tone, the map overlay, the arrival handling — is the ordinary
-- chain machinery, so a sweep costs a collector and nothing else.
--
-- The whole mode is deliberately generic: `loot` and `kill` differ only in what
-- their collector returns and how a target reads as finished. A future sweep (every
-- pot to lift, every torch to light) is another entry in SWEEP.MODES.
--
-- One sweep runs at a time, per room, until toggled off. While it has anything left
-- it drives the guide, overriding the quest route; when the room is finished it says
-- so, goes quiet and hands the guide back, then re-collects on entering the next
-- room. Globals throughout (like PUSH) to stay under the chunk's 200-local cap and
-- to let MCP inspect the live sweep.
SWEEP = {
  kind = nil,    -- "loot" | "kill" while a sweep runs, else nil
  chain = nil,   -- the generated chain, while it has targets left
  room = nil,    -- the room it was collected for
  level = nil,   -- and the floor, since tile targets are read off one floor
  sig = nil,     -- identity of the outstanding target set, so the chain is rebuilt only on change
  said = nil,    -- last count announced, so a steady room is not re-announced
  probe = 0,     -- frames until the next re-collect
}
SWEEP.PROBE = 15   -- frames between re-collections; a sprite scan plus a tile scan
SWEEP.KEYS = { [228] = true, [229] = true } -- small key / big key sprite drops

-- A tile target is finished when its tile stops answering to the class that found
-- it — the game rewrites the tile once the errand is done (an opened chest leaves
-- the chest range, a lifted pot stops being a pot), so this needs no flag of its
-- own. The class travels with the waypoint as `is`, since a sweep's targets all
-- came from one.
function SWEEP.tile_done(s, wp)
  return not wp.is(tile_attr_at(s, wp.tx * 8, wp.ty * 8, wp.level))
end

function SWEEP.is_chest(attr)
  return attr ~= nil and CHEST_TILES[attr] ~= nil
end

-- A dungeon's manipulable objects all read as attr 0x70-0x7F, so the tile alone
-- cannot say whether a thing is a pot to lift, a block to push, or a hammer peg.
-- What tells them apart is the room's replacement-state slot that the tile's low
-- nibble indexes: sixteen 16-bit entries at $7E0500, one per manipulable object
-- the room drew. The game sets 0x1111 for a pot (RoomDraw_SinglePot), 0 for a
-- pushable block, 0x4040 for a hammer peg, and lifts only what masks to 0x1010
-- (Dungeon_LiftAndReplaceLiftable) — so that mask is the test, read exactly as the
-- game's own lift check reads it. A lifted pot's slot stops matching, which is
-- what retires its waypoint.
MANIP_STATE = 0x7E0500
function SWEEP.is_block(attr)
  if attr == nil or attr < 0x70 or attr > 0x7F then return false end
  local st = mem.u16(MANIP_STATE + (attr & 0x0F) * 2)
  return st == 0 or st == 1
end

function SWEEP.is_pot(attr)
  if attr == nil or attr < 0x70 or attr > 0x7F then return false end
  local st = mem.u16(MANIP_STATE + (attr & 0x0F) * 2)
  return st ~= nil and (st & 0xF0F0) == 0x1010
end

-- ── Menus ───────────────────────────────────────────────────────────────────
-- A menu is unusable if you cannot see it, and unlike the overworld there is no tone
-- that can stand in: the whole content of the screen is which option the cursor is on.
-- So the cursor is read aloud, and re-read whenever it moves.
--
-- The file select screen (module 0x01, submodule 0x05) is the first one. Its cursor is a
-- single byte at $7E00C8 running 0-4: the three save files, then Copy and Erase. What
-- happens on the button confirms the mapping — cursor 3 enters main_module 2, the copy
-- module, and cursor 4 enters module 3, erase (zelda3 FileSelect_Main).
MENU = { said = nil }

MENU.FILE_SELECT_OPTIONS = { "Copy", "Erase" } -- what follows the three files

-- Does save file `k` (0-based) exist? The screen keeps that as a flag per file, set
-- where it finds the 0x55AA signature in the cartridge save (selectfile_arr1, $7E00BF,
-- one 16-bit entry each). Reading the game's own answer beats re-deriving it, and the
-- save itself is in SRAM, which the plugin cannot reach.
--
-- Only meaningful once FileSelect_Main has run: see MENU.SETTLE.
function MENU.file_exists(k)
  return mem.u16(0x7E00BF + k * 2) == 1
end

-- A save file's name, or nil.
--
-- The name lives in SRAM (offset 0x3D9, six 16-bit characters) and the plugin only reads
-- work RAM. But the screen stages the name's tiles into the VRAM upload buffer to draw
-- it, and THAT is in work RAM: $7E1002, with file k's name at byte offset 8, 0x5C or
-- 0xB0, six 16-bit entries, each the character plus 0x1800 (zelda3 SelectFile_Func17).
-- So the characters are read back off what the game is about to draw. Only staged for
-- files that exist, which is why the caller checks first.
--
-- REF.name_chars turns a code into a character. It is not the dialogue encoding — the
-- text decoder's ALPHABET is a different space entirely — so it was read out of the game
-- by walking the name-entry picker and recording what each cell shows.
-- Where each screen stages its names, as byte offsets into the buffer. Every screen lays
-- its file rows out differently, so this belongs to the screen rather than to the file:
--   files   the file select      (SelectFile_Func17)
--   source  copy, pick a source  (CopyFile_SelectionAndBlinker, Dst)
--   target  copy, pick a target  (CopyFile_TargetSelectionAndBlink, Dst + 4 for the row's
--           own "1"/"2"/"3" glyph, and only two rows because the source is skipped)
MENU.NAME_AT = {
  files = { 8, 0x5C, 0xB0 },
  source = { 0x3C, 0x64, 0x8C },
  target = { 0x38 + 4, 0x60 + 4 },
}
MENU.NAME_LEN = 6

function MENU.name_codes(off)
  if off == nil then return nil end
  local out = {}
  for i = 0, MENU.NAME_LEN - 1 do
    local t = mem.u16(0x7E1002 + off + i * 2)
    if t == nil then return nil end
    out[#out + 1] = (t - 0x1800) & 0xFFFF
  end
  return out
end

function MENU.name_text(off)
  local codes = MENU.name_codes(off)
  if codes == nil then return nil end
  local out = {}
  for _, c in ipairs(codes) do out[#out + 1] = REF.name_chars[c] or "" end
  local name = table.concat(out):gsub("%s+$", "")
  return name ~= "" and name or nil
end

-- Names as they were last read, per file.
--
-- A name is only in the VRAM upload buffer while a screen that draws it is up. The erase
-- confirmation replaces the buffer with its own prompt and never re-stages the names, so
-- remembering is the only way to say WHICH save is about to be destroyed. Deliberately not a
-- fallback inside file_line: a screen that ought to be able to read a name should be seen to
-- fail when its offsets are wrong, rather than quietly serving a remembered one.
MENU.seen_names = {}

-- One file row, wherever it is being shown: "File 2, LINK", or "File 2, empty".
function MENU.file_line(k, off)
  local label = "File " .. (k + 1)
  if not MENU.file_exists(k) then
    MENU.seen_names[k] = nil
    return label .. ", empty"
  end
  local name = MENU.name_text(off)
  if name ~= nil then MENU.seen_names[k] = name end
  return name and (label .. ", " .. name) or label
end

function MENU.file_name(k)
  return MENU.name_text(MENU.NAME_AT.files[k + 1])
end

-- The name-entry picker (module 0x04, submodule 0x03). The highlighted cell is
-- REF.name_cells[var3 + var5 * 0x20], read by POSITION rather than by glyph code, because
-- one code is two characters: 0x5F draws both capital I and lowercase l, so only where it
-- sits says which is meant. Same lookup the game does to decide what a button press types.
function MENU.name_entry_line()
  local at, row = mem.u8(0x7E0B10), mem.u8(0x7E0B15)
  if at == nil or row == nil then return nil end
  local said = REF.name_cells[at + row * 0x20 + 1]
  if said == nil or said == "" then return nil end
  -- Punctuation cells hold the character itself, which a speech engine renders as a pause
  -- or as nothing, so those cells were silent. Say their names instead.
  --
  -- Case is NOT distinguished, though the grid has separate blocks for each and "A" and "a"
  -- are spoken alike. A screen reader would normally say "cap A"; asked, the player said the
  -- letter alone is what they want here, so this is settled rather than overlooked.
  return REF.name_spoken[said] or said
end

-- The copy-file screen (module 0x02), which is three screens sharing one cursor,
-- selectfile_R16 at $7E00C8 — the same byte the file select uses.
--
--   submodule 3  pick a source: cursor 0-2 are the files, 3 is Quit. Navigation skips files
--                that do not exist, so the cursor only ever lands on a real one or on Quit.
--   submodule 4  pick a target: cursor 0-1 are the two files that are not the source, 2 is
--                Quit. Which file a row is is not the cursor — the game keeps the two
--                candidates in selectfile_arr2 ($7E00CA) as file * 2, ascending, and the
--                rows follow that order. A target may well be empty; that is the point.
--   submodule 5  confirm: cursor 0 does the copy, 1 backs out. "COPY OK" is the game's own
--                wording, decoded from the tiles the screen stages for that line.
function MENU.copy_line(sub, at)
  local key = string.format("copy:%s:%s", tostring(sub), tostring(at))
  if sub == 0x03 then
    if at >= 3 then return "Quit", key end
    return MENU.file_line(at, MENU.NAME_AT.source[at + 1]), key
  elseif sub == 0x04 then
    if at >= 2 then return "Quit", key end
    local slot = mem.u8(0x7E00CA + at)
    if slot == nil then return nil end
    return MENU.file_line(slot >> 1, MENU.NAME_AT.target[at + 1]), key
  elseif sub == 0x05 then
    return (at == 0 and "Copy OK" or "Quit"), key
  end
  return nil
end

-- The erase screen (module 0x03), the same shape as copy and sharing its cursor byte.
--
--   submodule 3  pick a file: cursor 0-2, 3 is Quit, and navigation skips files that do not
--                exist. Names are staged by SelectFile_Func17 — the routine the file select
--                itself uses — so this screen shares the file select's offsets rather than
--                having its own, unlike the copy screen.
--   submodule 4  confirm: cursor 0 goes ahead, 1 backs out. "ERASE THIS PLAYER" is the
--                game's own wording for that line, decoded from the tiles it stages.
--
-- The confirmation names the file as well. The screen does say which — it clears the other
-- two rows and leaves the chosen one standing — and the wording alone would not, which for
-- something irreversible is worth the extra words. Which file it is is not the cursor here
-- but subsubmodule_index at $7E00B0, where KILLFile_ChooseTarget parked it.
function MENU.erase_line(sub, at)
  local key = string.format("erase:%s:%s", tostring(sub), tostring(at))
  if sub == 0x03 then
    if at >= 3 then return "Quit", key end
    return MENU.file_line(at, MENU.NAME_AT.files[at + 1]), key
  elseif sub == 0x04 then
    if at ~= 0 then return "Quit", key end
    local k = mem.u8(0x7E00B0)
    if k == nil or k > 2 then return "Erase this player", key end
    local name = MENU.seen_names[k]
    local which = "File " .. (k + 1) .. (name and (", " .. name) or "")
    return "Erase this player: " .. which, key
  end
  return nil
end

-- What the cursor is on, and a key for where it is. The key is what decides whether to
-- speak again, because the text alone cannot: the picker has two `end` cells side by side
-- and a dozen blanks in a row, and moving between them said nothing at all. An
-- announcement is how the player learns the cursor moved, so it follows the cursor rather
-- than the words.
function MENU.line(s)
  if s.module == 0x03 then
    local at = mem.u8(0x7E00C8)
    if at == nil then return nil end
    return MENU.erase_line(mem.u8(0x7E0011), at)
  end
  if s.module == 0x02 then
    local at = mem.u8(0x7E00C8)
    if at == nil then return nil end
    return MENU.copy_line(mem.u8(0x7E0011), at)
  end
  if s.module == 0x04 and mem.u8(0x7E0011) == 0x03 then
    local at, row = mem.u8(0x7E0B10), mem.u8(0x7E0B15)
    return MENU.name_entry_line(), string.format("name:%s,%s", tostring(at), tostring(row))
  end
  if s.module ~= 0x01 or mem.u8(0x7E0011) ~= 0x05 then return nil end
  local at = mem.u8(0x7E00C8)
  if at == nil then return nil end
  local key = "file:" .. at
  if at >= 3 then
    return MENU.FILE_SELECT_OPTIONS[at - 2], key
  end
  local label = "File " .. (at + 1)
  if not MENU.file_exists(at) then return label .. ", empty", key end
  local name = MENU.file_name(at)
  return (name and (label .. ", " .. name) or label), key
end

-- Cells that move the name cursor instead of typing. The game tells them apart by glyph
-- code (0x5A back, 0x44 forward, 0x6F end) before it commits anything; here the label does.
MENU.CONTROLS = { back = true, forward = true, ["end"] = true }

-- The character just committed, or nil if nothing was.
--
-- The typed name itself is unreachable: the game writes it straight to SRAM, and `mem` is
-- WRAM only. What is reachable is the name cursor, selectfile_var4 at $7E0B12 — the slot
-- the next character lands in — and the game advances it on every commit. So a slot that
-- moved means a button was pressed, and the cell under the grid cursor says what was typed.
--
-- `back` and `forward` move the slot too and type nothing, so the label has to be checked;
-- otherwise walking the name cursor would announce whichever letter happened to be under
-- the grid cursor. They stay SILENT rather than announcing the slot they moved to, which the
-- player confirmed is what they want; the alternative was calling out "slot 3" or re-reading
-- the name so far after every keystroke. Reading the grid cursor on this frame is right rather than a frame late,
-- because NameFile returns early once it has scrolled, so no frame both moves and commits.
function MENU.name_selection()
  local slot = mem.u8(0x7E0B12)
  if slot == nil then return nil end
  local was = MENU.slot
  MENU.slot = slot
  if was == nil or was == slot then return nil end
  local at, row = mem.u8(0x7E0B10), mem.u8(0x7E0B15)
  if at == nil or row == nil then return nil end
  local cell = REF.name_cells[at + row * 0x20 + 1]
  if cell == nil or cell == "" or MENU.CONTROLS[cell] then return nil end
  return REF.name_spoken[cell] or cell
end

-- Frames to let a screen settle before reading it.
--
-- Every one of these screens is drawn by its own submodule handler, and everything the
-- reader asks about — which files exist, and their names — is put in place by that handler
-- on each of its runs. So the first frame on a new screen is one where the handler has not
-- run yet and the reader is looking at whatever the last screen left behind.
--
-- The file select is where this bit. Arriving from the name picker goes through submodule 1,
-- FileSelect_ReInitSaveFlagsAndEraseTriforce, which memsets the file-exists flags to zero,
-- and only submodule 5's FileSelect_Main puts them back. The announcement latches on the
-- cursor, so that first reading was also the last word on it: a file just named was
-- announced as empty until the cursor moved off it and back. The copy screen stages its
-- names the same way and would have gone the same way, so the wait belongs to all of them
-- rather than to the one screen that showed the symptom.
--
-- One frame is enough — the handler running once is all it takes — and 16ms is inaudible.
-- Only a change of screen resets it, so moving the cursor within a screen is not delayed.
-- The alternative, latching on the words instead so a corrected reading re-speaks, is worse
-- on both counts: it would still say the wrong thing once, and the names are read out of
-- the VRAM upload buffer, which is transient enough to stutter.
MENU.SETTLE = 1

function MENU.update(s)
  local sub = mem.u8(0x7E0011)

  -- Forget the slot on the way out, or re-entering the picker reads the reset back to 0 as
  -- a commit and speaks a character nobody typed.
  local naming = s.module == 0x04 and sub == 0x03
  if not naming then MENU.slot = nil end

  local screen = sub and (s.module .. ":" .. sub) or nil
  MENU.screen_for = (screen ~= nil and screen == MENU.screen) and (MENU.screen_for + 1) or 0
  MENU.screen = screen
  if MENU.screen_for < MENU.SETTLE then
    -- Nothing trustworthy to say yet. Clear the latch so the settled reading speaks.
    MENU.said = nil
    return
  end

  if naming then
    local picked = MENU.name_selection()
    if picked ~= nil then
      -- Deliberately not touching MENU.said: the grid cursor has not moved, so the latch
      -- still describes where it is and moving off this cell must still speak.
      say(picked, { priority = "critical", category = "menu" })
      return
    end
  end

  local line, key = MENU.line(s)
  if line == nil then MENU.said = nil; return end
  if MENU.said == key then return end
  MENU.said = key
  -- Critical: a menu with nothing spoken is a menu that cannot be used, so it must not
  -- sit behind the verbosity gate, and a fresh selection should cut off the last one
  -- rather than queue behind it.
  say(line, { priority = "critical", category = "menu" })
end

-- ── What Link is facing ─────────────────────────────────────────────────────
-- Name the thing in front of Link, once, when it is something he has to act on rather
-- than walk around. A bush needs slashing, a block needs shoving, a pot needs lifting;
-- the router treats a bush as passable and a block as solid, and in neither case does
-- the collision alone tell the player what to DO with it.
--
-- One announcer rather than a cue per class, because they are the same behaviour: read
-- the faced tile, say what it is if it is worth saying, and stay quiet until he faces
-- something else. `said` holds the last thing named, so turning from a bush to a block
-- announces the block — it is about what is in front of him now, not about novelty.
--
-- Some classes only make sense while being led somewhere. A bush cue is routing advice
-- (the guide has just routed THROUGH that bush, and the player has to swing to follow),
-- so it keeps the `guided` gate it always had. A block or a pot is worth knowing about
-- whenever Link walks into one.
FACE = { said = nil }

-- One tile ahead, by facing: the reach the bush cue has always used.
function FACE.ahead(s)
  local dir = s.direction
  return s.x + 8 + (dir == 4 and -12 or dir == 6 and 12 or 0),
         s.y + 12 + (dir == 0 and -12 or dir == 2 and 12 or 0)
end

FACE.CLASSES = {
  -- Overworld only in practice: BUSH_TILE is what the overworld decode reports for the
  -- two bush map16 ids, and the dungeon grid never yields it.
  { say = "Bush.", guided = true, test = function(a) return a == BUSH_TILE end },
  -- 0x27 is TileBehavior_Hookshottables, and it is what a shoved block becomes at its
  -- new position: the tile stops being manipulable, so the push machinery rightly goes
  -- quiet, but the thing is still standing there and still worth naming. Statues, logs
  -- and hookshot pegs share the class and get called blocks too, which is a wrong word
  -- for a log and the right one for the case that comes up.
  { say = "Block.", test = function(a) return SWEEP.is_block(a) or a == 0x27 end },
  { say = "Pot.", test = function(a) return SWEEP.is_pot(a) end },
}

function FACE.update(s)
  if s == nil or not in_play(s) then FACE.said = nil; return end
  local ax, ay = FACE.ahead(s)
  local a = tile_attr_at(s, ax, ay)
  local hit
  for _, c in ipairs(FACE.CLASSES) do
    if (not c.guided or nav_active) and c.test(a) then hit = c; break end
  end
  if hit == nil then FACE.said = nil; return end
  if FACE.said ~= hit.say then
    say(hit.say, { priority = "navigation", category = "on-demand" })
    FACE.said = hit.say
  end
end

-- Every tile of a class in the room window, as one target per cluster. The things
-- worth sweeping are drawn 2x2 (a chest, a pot), so a tile is taken only when its
-- west and north neighbours are not the same class — one object, one waypoint,
-- rather than four stacked on each other.
function SWEEP.tile_targets(s, is, name)
  local out = {}
  local level = mem.u8(LOWER_LEVEL)
  local ox, oy = SWEEP.window(s)
  local otx, oty = ox >> 3, oy >> 3
  for y = 0, 63 do
    for x = 0, 63 do
      if is(tile_attr_at(s, (otx + x) * 8, (oty + y) * 8, level)) then
        local w = x > 0 and tile_attr_at(s, (otx + x - 1) * 8, (oty + y) * 8, level) or nil
        local n = y > 0 and tile_attr_at(s, (otx + x) * 8, (oty + y - 1) * 8, level) or nil
        if not is(w) and not is(n) then
          out[#out + 1] = { tx = otx + x, ty = oty + y, level = level, name = name,
            is = is, id = name:sub(1, 1) .. (otx + x) .. "." .. (oty + y),
            done = SWEEP.tile_done }
        end
      end
    end
  end
  return out
end

-- A sprite target is gone when its slot is free or has been reused by something
-- else. The kind is part of the identity precisely because slots are recycled.
function SWEEP.sprite_gone(s, wp)
  return wp.slot == nil or mem.u8(SPRITE.state + wp.slot) == 0
    or mem.u8(SPRITE.kind + wp.slot) ~= wp.kind
end

-- An enemy is finished when it is gone or out of health. (A defeated enemy lingers
-- a few frames as its death animation, at hp 0 — done the moment it is struck out.)
function SWEEP.enemy_done(s, wp)
  return SWEEP.sprite_gone(s, wp) or (mem.u8(SPRITE.hp + wp.slot) or 0) == 0
end

-- The 512-pixel room window Link stands in, as world tile coordinates. Sprites
-- outside it belong to a neighbouring room that happens to be loaded.
function SWEEP.window(s)
  local ox, oy = s.x - s.x % 512, s.y - s.y % 512
  return ox, oy
end

-- Everything in the room worth picking up: unopened chests, loose pickups, and key
-- drops. Chests are tiles, not sprites, and each occupies a 2x2 block — only its
-- top-left tile is taken, so one chest is one waypoint rather than four. Tiles are
-- read off the floor Link is on (the collision table only describes one at a time),
-- so a two-floor room is swept a floor at a time; sprites are room-wide. That last
-- part is reasoned from how the table works, not yet watched happening: the sweeps
-- have been verified live in a single-floor room (0x72) only. Whether "Room swept"
-- on the upper floor and a re-arm on the way down reads as helpful or as a bug is a
-- question for a real two-floor room.
function SWEEP.loot(s)
  local out = SWEEP.tile_targets(s, SWEEP.is_chest, "chest")
  local level = mem.u8(LOWER_LEVEL)
  local ox, oy = SWEEP.window(s)
  for _, sp in ipairs(sprites()) do
    if (REF.item_types[sp.kind] or SWEEP.KEYS[sp.kind])
        and sp.x >= ox and sp.x < ox + 512 and sp.y >= oy and sp.y < oy + 512 then
      out[#out + 1] = { tx = sp.x >> 3, ty = sp.y >> 3, level = level, slot = sp.slot,
        kind = sp.kind, name = sprite_name(sp.kind),
        id = "s" .. sp.slot .. "." .. sp.kind, done = SWEEP.sprite_gone }
    end
  end
  return out
end

-- Every live enemy in the room. Mirrors the room-clear tally (health above zero and
-- not flagged out of it, matching Sprite_CheckIfRoomIsClear) so the sweep agrees
-- with the game about what is still standing, but lists them all rather than the
-- nearest — and room-wide rather than on-screen, since the point is to find the one
-- skulking in the far corner.
function SWEEP.kill(s)
  local out = {}
  local level = mem.u8(LOWER_LEVEL)
  local ox, oy = SWEEP.window(s)
  for _, sp in ipairs(sprites()) do
    if (sp.hp or 0) > 0 and is_enemy(sp) and (mem.u8(SPRITE_FLAGS4 + sp.slot) & 0x40) == 0
        and sp.x >= ox and sp.x < ox + 512 and sp.y >= oy and sp.y < oy + 512 then
      out[#out + 1] = { tx = sp.x >> 3, ty = sp.y >> 3, level = level, slot = sp.slot,
        kind = sp.kind, name = enemy_name(sp),
        id = "s" .. sp.slot .. "." .. sp.kind, done = SWEEP.enemy_done }
    end
  end
  return out
end

-- Every pot still standing. Pots hide the room's small change — hearts, rupees,
-- the odd key — but a player who cannot see them has no way to know a room holds
-- any, which is exactly the sort of thing sighted players get for free.
function SWEEP.lift(s)
  return SWEEP.tile_targets(s, SWEEP.is_pot, "pot")
end

SWEEP.MODES = {
  loot = { collect = SWEEP.loot, on = "Loot sweep on.",
    noun = "item", nouns = "items", verb = "to collect.", clear = "Room swept." },
  kill = { collect = SWEEP.kill, on = "Enemy sweep on.",
    noun = "enemy", nouns = "enemies", verb = "to defeat.", clear = "Room clear." },
  lift = { collect = SWEEP.lift, on = "Pot sweep on.",
    noun = "pot", nouns = "pots", verb = "to lift.", clear = "Nothing left to lift." },
}

-- The outstanding target set's identity: what has to change before the chain is
-- worth rebuilding. Positions are deliberately left out — a wandering enemy moves
-- every frame but is still the same errand, and its waypoint follows it live.
function SWEEP.signature(list)
  local ids = {}
  for i, t in ipairs(list) do ids[i] = t.id end
  table.sort(ids)
  return table.concat(ids, "|")
end

-- Order the chain nearest-first from Link. The dungeon leg takes the first
-- reachable waypoint of a sweep chain, so this ordering is what makes the sweep a
-- greedy tour: always the closest errand left, re-sorted as Link works through them.
function SWEEP.sort(s, chain)
  table.sort(chain, function(a, b)
    return math.abs(a.tx * 8 - s.x) + math.abs(a.ty * 8 - s.y)
      < math.abs(b.tx * 8 - s.x) + math.abs(b.ty * 8 - s.y)
  end)
end

-- Keep each sprite-backed waypoint sitting on its sprite, so the guide leads to
-- where the enemy is now rather than where it was when collected.
function SWEEP.follow(s)
  if SWEEP.chain == nil then return end
  for _, wp in ipairs(SWEEP.chain) do
    if wp.slot and not wp.done(s, wp) then
      wp.tx = (mem.u8(SPRITE.x_lo + wp.slot) + mem.u8(SPRITE.x_hi + wp.slot) * 256) >> 3
      wp.ty = (mem.u8(SPRITE.y_lo + wp.slot) + mem.u8(SPRITE.y_hi + wp.slot) * 256) >> 3
    end
  end
end

-- Hand the guide back: drop the sweep's chain and force the quest navigator to
-- re-aim (its signature is unchanged by a sweep, so without this it would sit idle
-- believing it is still routed).
function SWEEP.release()
  if SWEEP.chain and nav_chain == SWEEP.chain then
    nav_chain = nil
    nav_chain_i = 1
  end
  SWEEP.chain = nil
  nav_sig = nil
  nav_idle_sig = nil
end

-- Re-collect the room and rebuild the chain if the set of outstanding targets has
-- changed; otherwise just re-sort what is left. Announces the count on a change,
-- and the mode's "room finished" line once nothing remains.
function SWEEP.refresh(s)
  local m = SWEEP.MODES[SWEEP.kind]
  local list = m.collect(s)
  local sig = SWEEP.signature(list)
  if sig == SWEEP.sig then
    if SWEEP.chain then SWEEP.sort(s, SWEEP.chain) end
    return
  end
  SWEEP.sig = sig
  if #list == 0 then
    SWEEP.release()
    if SWEEP.said ~= "clear" then
      nav_say(m.clear)
      SWEEP.said = "clear"
    end
    return
  end
  -- Copied rather than used directly, because the chain is mutated as it is walked
  -- (a sprite waypoint's position follows its sprite, the driver marks arrivals)
  -- and the collector should be free to hand back the same table twice. The copy is
  -- wholesale: listing the fields by hand meant a collector could add one — the tile
  -- class a `done` predicate needs, say — and have it silently dropped here.
  local chain = { sweep = true }
  for i, t in ipairs(list) do
    local wp = {}
    for k, v in pairs(t) do wp[k] = v end
    wp.room = s.dungeon_room -- what makes it a dungeon waypoint to the driver
    chain[i] = wp
  end
  SWEEP.sort(s, chain)
  SWEEP.chain = chain
  nav_chain = chain
  nav_chain_i = 1
  chain_probe_in = 0 -- re-pick immediately against the new set
  if SWEEP.said ~= #list then
    nav_say(count_word(#list) .. " " .. (#list == 1 and m.noun or m.nouns) .. " " .. m.verb)
    SWEEP.said = #list
  end
end

-- Per-frame while a sweep is armed, ahead of the quest navigator. Returns whether
-- the sweep is driving the guide: true while it has targets left in this room, so
-- the caller stands down; false when it is off, out of a dungeon, or done here, so
-- the quest route resumes.
function SWEEP.update(s)
  if SWEEP.kind == nil or not in_play(s) then return false end
  if s.module ~= 0x07 then
    if SWEEP.chain then SWEEP.release() end
    SWEEP.room = nil
    return false
  end
  local level = mem.u8(LOWER_LEVEL)
  if SWEEP.room ~= s.dungeon_room or SWEEP.level ~= level then
    SWEEP.room, SWEEP.level = s.dungeon_room, level
    SWEEP.sig, SWEEP.said, SWEEP.probe = nil, nil, 0
    if SWEEP.chain then SWEEP.release() end
  end
  SWEEP.probe = SWEEP.probe - 1
  if SWEEP.probe <= 0 then
    SWEEP.probe = SWEEP.PROBE
    SWEEP.refresh(s)
  end
  if SWEEP.chain == nil then return false end
  nav_chain = SWEEP.chain -- reclaim it if the quest navigator re-aimed under us
  SWEEP.follow(s)
  chain_dungeon_leg(s)
  return true
end

-- The order one key cycles through: off, loot, enemies, off again. A single
-- binding reaches both modes, which is what the ten-custom-command budget allows —
-- and SWEEP.set names a mode directly for anything driving the plugin over MCP.
SWEEP.CYCLE = { "loot", "kill", "lift" }

function SWEEP.cycle()
  local at = 0
  for i, k in ipairs(SWEEP.CYCLE) do if SWEEP.kind == k then at = i end end
  local nxt = SWEEP.CYCLE[at + 1]
  SWEEP.kind = nil -- so set() reads as a switch, never as a toggle-off
  return SWEEP.set(nxt)
end

-- Toggle a sweep mode on, or off if it is already the one running. Switching
-- straight from one mode to the other replaces it. Returns the line to speak.
function SWEEP.set(kind)
  if SWEEP.kind == kind then kind = nil end
  SWEEP.release()
  SWEEP.kind = kind
  SWEEP.room, SWEEP.level, SWEEP.sig, SWEEP.said, SWEEP.probe = nil, nil, nil, nil, 0
  if kind == nil then return "Sweep off." end
  room_route_stop()
  ow_route_stop()
  pathfind_stop()
  return SWEEP.MODES[kind].on
end

local ROOM_OBJECTIVES = {
  -- The enemy carrying the key comes first: go for it, then its dropped key, then the
  -- rest. Gated off the Zelda escort out (like the dropped-key objective), since the
  -- escape does not stop to fight for keys.
  { id = "keyholder",
    cue = "Defeat the enemy holding the key.",
    -- Not gated off the Zelda escort (unlike the dropped-key objective): the escape can
    -- still need a key on the way out, and a LIVE key-holder is a real forward target —
    -- a respawned guard in an already-cleared room carries no key (die_action 0), so this
    -- only fires on an enemy that genuinely still drops one.
    active = function(s) return key_holder(s) ~= nil end,
    target = function(s)
      local e = key_holder(s)
      if e then return walkable_near(s, e[1], e[2]) end
    end },
  { id = "kill",
    cue = "Defeat all enemies.",
    -- While the tag is set the room is still gating, full stop: the game zeroes it
    -- itself once its own clear check passes (every kill tag reaches
    -- RoomTag_OperateChestReveal or Dung_TagRoutine_TrapdoorsUp), and that check already
    -- folds in the overlord spawners. Counting enemies here as well only added ways to be
    -- wrong: going quiet in the frames where a room's sprites are mid-spawn, and
    -- re-arming on a respawn after the room was already officially clear.
    -- The tag says the room is still gating; a countable enemy says there is something
    -- to fight FROM HERE. Both are needed. Room 0x71 is why: its tag stays set while its
    -- other pit is uncleared, but from Link's pit there is nothing to reach, and
    -- announcing "defeat all enemies" with no target left the guide saying it forever
    -- and never advancing. Termination is still the tag's job — an enemy that dies while
    -- the tag stands means the room is not finished, it just has nothing here.
    active = function(s)
      return kill_room(s) ~= nil
        and (nearest_pending_enemy(s) ~= nil or overlords_pending())
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

-- Aim at the first room objective that is both active and actually reachable,
-- returning the one committed to, or nil. Dungeon-only.
--
-- Reachability is part of the choice, not an afterthought. Room 0x71 is the case
-- that proved it: two guard pits walled off from each other, the key-holder in one
-- and Link in the other. `keyholder` sits above `kill` in the list and was picked
-- on being live alone, so the guide committed to a soldier the router could not
-- reach — route_set_goal failed, no path was drawn, and the enemy five tiles from
-- Link went unmentioned. An objective whose target cannot be routed to is real but
-- not yet actionable, so it falls through to the next one that can.
--
-- The cheap path is unchanged: an objective already aimed where it wants keeps its
-- route without replanning, so the common frame plans nothing. Only a target that
-- has moved (or failed) costs a search, and a failed search used to buy nothing at
-- all.
-- Is there an authored errand for this room that is not carried out yet, and does the
-- ARMED chain own it? Two questions, because they lead to different places: a step
-- the armed chain owns is driven by the chain leg, in route order and with `via`
-- honoured; one that only the errand index knows about (its chain has since completed)
-- is driven directly, below. Either way an authored step exists, so the room-scoped
-- objectives — the fallback for rooms nobody has mapped — must yield to it.
local function chain_errand_here(s)
  if nav_chain == nil then return false end
  for _, wp in ipairs(nav_chain) do
    if wp.room == s.dungeon_room and KIND.of(wp).done ~= nil and not KIND.done(s, wp) then
      return true
    end
  end
  return false
end

-- The furthest uncleared errand for this room that the guide can actually reach, from
-- any authored chain. Furthest rather than nearest, matching the chain leg's own pick
-- policy: later steps supersede earlier ones, so the most recent uncleared one is the
-- live errand.
local function room_errand(s)
  local list = WP.errands[s.dungeon_room]
  if list == nil then return nil end
  local ltx, lty = s.x >> 3, s.y >> 3
  local pick, pgx, pgy
  for _, wp in ipairs(list) do
    if (wp.gate == nil or wp.gate(s, wp)) and not KIND.done(s, wp) then
      local gx, gy = KIND.target(s, wp)
      if gx and plan_path(s, ltx, lty, gx >> 3, gy >> 3, wp.level) then
        pick, pgx, pgy = wp, gx, gy
      end
    end
  end
  return pick, pgx, pgy
end

local function room_aim(s)
  if s.module ~= 0x07 then return nil end
  -- An authored step for this room says what the room needs; the objectives are only
  -- for rooms nobody has mapped, so they stand aside where one exists.
  local charted = WP.fights[s.dungeon_room] == true
  for _, o in ipairs(ROOM_OBJECTIVES) do
    if charted and (o.id == "kill" or o.id == "keyholder") then goto continue end
    if o.active(s) then
      local tx, ty = o.target(s)
      -- Active with nowhere to walk (overlord spawners hold a room open but have no
      -- position): still the objective, and still worth stating.
      if tx == nil then return o end
      if pathfind_active and pathfind_goal ~= nil
        and math.abs(pathfind_goal[1] - (tx >> 3)) + math.abs(pathfind_goal[2] - (ty >> 3)) < 2
      then
        return o -- already leading there, and the route is live: leave it alone
      end
      if route_set_goal(s, tx, ty) then return o end
    end
    ::continue::
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
  -- A room sweep outranks the quest route while it has errands left in this room,
  -- and runs whether or not the quest guide is on — it is its own mode. Once the
  -- room is finished it stands down and the rest of this runs as usual.
  if SWEEP.update(s) then return end
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
  -- An authored errand in this room that the armed chain does not own — its chain has
  -- since completed, so the chain leg will never look at it. Drive it here: standing in
  -- a room whose fight still gates the way through, the guide should say so whatever
  -- the quest has moved on to.
  if s.module == 0x07 and not chain_errand_here(s) then
    local wp, gx, gy = room_errand(s)
    if wp then
      local key = s.dungeon_room .. ":errand"
      if room_obj_announced ~= key then
        -- Same rule as the chain leg, `quiet` included: two drivers disagreeing about
        -- whether a step narrates itself is worse than either answer.
        local line = wp.say or (not wp.quiet and KIND.of(wp).cue) or nil
        if line then nav_say(line) end
        room_obj_announced = key
      end
      -- Arriving counts here too. The chain leg speaks a step's `arrival` on reaching it;
      -- this driver did not, so a step whose chain had retired could be led to and then
      -- say nothing on arrival — which is not "reachable from any quest state" in any
      -- sense the player would recognise. Latched per step, so it lands once.
      if wp.tx and math.abs((s.x >> 3) - wp.tx) + math.abs((s.y >> 3) - wp.ty) <= CHAIN_REACH
        and (wp.level or 0) == mem.u8(LOWER_LEVEL)
      then
        if wp.arrival and ROOMOBJ.arrived ~= wp then
          nav_say(wp.arrival)
          ROOMOBJ.arrived = wp
        end
        pathfind_stop() -- standing on it; go quiet rather than shuffling on the spot
        return
      end
      if pathfind_goal == nil
        or math.abs(pathfind_goal[1] - (gx >> 3)) + math.abs(pathfind_goal[2] - (gy >> 3)) >= 2
      then
        route_set_goal(s, gx, gy, wp.level)
      end
      return
    end
  end
  -- room_aim both picks the objective and sets its route, so what gets announced is
  -- what the guide actually committed to leading Link toward.
  -- Saying it again is a fresh room's business, not a fresh frame's.
  if ROOMOBJ.room ~= s.dungeon_room then
    ROOMOBJ.room, ROOMOBJ.said, ROOMOBJ.lapse, ROOMOBJ.arrived = s.dungeon_room, nil, 0, nil
  end
  local ro = room_aim(s)
  if ro then
    local key = s.dungeon_room .. ":" .. ro.id
    room_obj_announced = key
    ROOMOBJ.lapse = 0
    -- The spoken latch is separate from the active trace on purpose. They used to be one
    -- field, so a single frame with no target cleared it and the next frame said the cue
    -- again — "defeat all enemies" over and over. ROOMOBJ.said only resets when the room
    -- does, so a lapse cannot buy a second announcement.
    if ROOMOBJ.said ~= key then
      nav_say(ro.cue)
      ROOMOBJ.said = key
    end
    return
  end
  if room_obj_announced ~= nil then
    -- And one frame without a target is not the objective clearing. A moving enemy
    -- crosses the reachable boundary, a sprite slot blinks as it dies or respawns; taking
    -- that for "done" re-aimed the whole quest route each time it happened.
    ROOMOBJ.lapse = ROOMOBJ.lapse + 1
    if ROOMOBJ.lapse < ROOMOBJ.LAPSE then return end
    room_obj_announced = nil
    ROOMOBJ.lapse = 0
    nav_reaim(s, v) -- objective genuinely cleared: resume the quest goal
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
      -- Dungeon leg: lead to the room's last reachable waypoint. A room's sub-goal
      -- already took precedence above.
      chain_dungeon_leg(s)
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
  -- Said before the re-aim so it lands ahead of whatever goal the re-aim announces:
  -- "Navigation on. Rescue Princess Zelda." The off case always said so, and the
  -- silence on the way on left the player pressing the key to find out which it did.
  nav_say("Navigation on.")
  local v = read_progress()
  nav_sig = nav_signature(s, v)
  nav_reaim(s, v)
end)

-- Sweep the room: lead to every piece of loot in it, then (next press) to every
-- enemy in it, one waypoint at a time. One key cycles off -> loot -> enemies -> off.
on_command("sweep", function()
  say(SWEEP.cycle(), { priority = "navigation", category = "on-demand" })
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

-- Map label placement.
--
-- The playfield draws a 64-tile room 200 pixels across, so one tile is barely
-- three pixels, while a two-digit number in the 5x7 font is eleven wide and seven
-- tall — near four tiles by two. A label drawn at a fixed offset from its marker
-- therefore lands on its neighbours', and since the route numbers every tile of
-- the path, three pixels apart, the result is a smear that reads as nothing.
--
-- So every numbered label goes through here. It remembers what it has already put
-- down this frame and walks a few offsets around the marker looking for clear
-- space, drawing nothing rather than a pile. Skipping a label is the right failure:
-- the marker is still there, and a number that cannot be read is worse than a gap,
-- which at least thins the route's numbering into something legible.
LABELS = { taken = {} }

-- Offsets to try, as {dx, dy, anchor}: anchor 1 puts the label's left edge at dx,
-- -1 its right edge, 0 centres it. Right-of-marker first, since that is where a
-- reader looks, then left, then above and below.
LABELS.SPOTS = {
  { 3, -3, 1 }, { 3, 1, 1 }, { -3, -3, -1 }, { -3, 1, -1 }, { 0, -9, 0 }, { 0, 5, 0 },
}

function LABELS.reset()
  LABELS.taken = {}
end

-- Draws `text` near (px, py) in the first free spot, returning true if it fit.
function LABELS.put(canvas, px, py, text, color)
  local w, h = #text * 6 - 1, 7
  for _, o in ipairs(LABELS.SPOTS) do
    local x = px + o[1]
    if o[3] == -1 then x = x - w elseif o[3] == 0 then x = px + o[1] - w // 2 end
    local y = py + o[2]
    local clear = true
    for _, r in ipairs(LABELS.taken) do
      if x < r[1] + r[3] and r[1] < x + w and y < r[2] + r[4] and r[2] < y + h then
        clear = false
        break
      end
    end
    if clear then
      LABELS.taken[#LABELS.taken + 1] = { x, y, w, h }
      canvas:text(x, y, text, color)
      return true
    end
  end
  return false
end

-- Draws a set of labels, those marked `first` ahead of the rest: the earliest
-- caller keeps its spot, so the immediate goal is the number that survives a
-- crowd. Each entry is {x, y, text = ..., color = ..., first = ...}.
function LABELS.number(canvas, points)
  for pass = 1, 2 do
    for _, p in ipairs(points) do
      if (p.first == true) == (pass == 1) then
        LABELS.put(canvas, p[1], p[2], p.text, p.color)
      end
    end
  end
end

-- Map mode: a schematic of what the plugin reads, for debugging and for sighted
-- assistance. In a dungeon or on the overworld it draws the area's actual shape
-- from the collision map; elsewhere it is just the position/sprite overlay.
-- Integer math throughout (// is floor division) so coordinates stay whole for
-- the canvas.
function on_draw(canvas)
  local w, h = canvas.width, canvas.height
  canvas:clear(0x101828)
  LABELS.reset()

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
    -- `room_obj_announced` is the read-only trace of that: nav_update sets it to the
    -- objective it committed to and clears it when none holds. Drawing must not ask
    -- room_aim directly — choosing an objective plans a route, and a draw pass has no
    -- business changing where the guide is pointed.
    -- A step with no place of its own (a room-clear, whose target is whichever enemy is
    -- nearest) has nothing to plot: the enemy it is aiming at carries its own marker and
    -- its own tone. Skip it rather than invent a position for it.
    if s.module == 0x07 and nav_chain and room_obj_announced == nil then
      local wp = nav_chain[nav_chain_i]
      if wp and wp.room == s.dungeon_room and wp.tx then
        local px, py = plot(wp.tx * 8 + 4, wp.ty * 8 + 4)
        canvas:rect(px - 1, py - 1, 3, 3, 0xFFFFFF)
      end
    end

    -- Debug overlay: developer aids the normal map hides, always on. Only in a
    -- dungeon, where the room and its waypoints live.
    if s.module == 0x07 then
      -- A kill-room's boundary, in a distinct red (never the pink of the nav route).
      -- When the room's chambers are mapped (ROOMS[room].chambers), draw a 1px rectangle on
      -- its tile bounds so the border hugs the real pit instead of framing the whole
      -- screen. Rooms with no mapped pit fall back to a frame outside the playfield.
      -- Either the room's own tag says it gates on a fight, or an authored `clear` step
      -- does. The overlay follows whatever bounds the tally, so it is drawn for both —
      -- gating on the tag alone hid it in exactly the rooms someone had to map by hand.
      if kill_room(s) or WP.fights[s.dungeon_room] then
        local kc = 0xE83838
        -- The fighting area, as the tally actually sees it: every tile Link can reach
        -- from where he stands, tinted. This used to outline an authored rectangle; the
        -- rectangles are gone, and the fill is both the truth and a better picture of it
        -- — a chamber that is not rectangular shows up as the shape it really is.
        if REACH.set and REACH.room == s.dungeon_room then
          for i in pairs(REACH.set) do
            local px, py = plot((REACH.ox + i % 64) * 8, (REACH.oy + i // 64) * 8)
            canvas:rect(px, py, 1, 1, kc)
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
      --
      -- The number's colour says which numbering it belongs to, because two are on
      -- screen at once and only one of them can be looked up. TEAL means an index
      -- into an authored chain exactly as waypoints.lua declares it, so a teal 11 is
      -- `show 11` in the editor and `move 11` repoints it. A generated sweep chain is
      -- numbered the same way but corresponds to nothing in the file, so it stays the
      -- neutral cyan; the pink numbers below are route steps, not waypoints at all.
      -- The marker square keeps the white/coloured active distinction, so "which is
      -- the target" and "which numbering is this" are two signals rather than one
      -- overloaded colour.
      if nav_chain then
        local nc = nav_chain.sweep and 0x50D0F0 or 0x20B0A0
        local labels = {}
        for i, wp in ipairs(nav_chain) do
          if wp.room == s.dungeon_room and wp.tx
            and inwin(wp.tx * 8 + 4, wp.ty * 8 + 4) then
            local px, py = plot(wp.tx * 8 + 4, wp.ty * 8 + 4)
            canvas:rect(px - 1, py - 1, 3, 3, (i == nav_chain_i) and 0xFFFFFF or nc)
            labels[#labels + 1] = { px, py, text = tostring(i), color = nc,
              first = i == nav_chain_i }
          end
        end
        LABELS.number(canvas, labels)
      end

      -- Direct-pathfind phases (e.g. escorting Zelda) have no chain, but the guide
      -- still leads along an A* route drawn above as pink corners. Number those
      -- corners too — in the route's own pink, the active one white — so the
      -- immediate target always carries a number, not just chain waypoints.
      if pathfind_active and pathfind_path then
        local labels = {}
        for i, wt in ipairs(pathfind_path) do
          if inwin(wt[1] * 8 + 4, wt[2] * 8 + 4) then
            local px, py = plot(wt[1] * 8 + 4, wt[2] * 8 + 4)
            labels[#labels + 1] = { px, py, text = tostring(i),
              color = (i == pathfind_wp) and 0xFFFFFF or 0xFF60D0,
              first = i == pathfind_wp }
          end
        end
        LABELS.number(canvas, labels)
      end
    end

    -- The overworld chain and the route to its current target, drawn through the
    -- current 512-pixel window; the segment leaving the screen edge points on toward
    -- the next area. World tiles are placed relative to Link's block, so off-window
    -- corners clip.
    --
    -- The chain is drawn on its own terms rather than nested inside the route, which is
    -- how it used to be: the route needs a computed A* path, so a target the router
    -- could not reach took the whole chain off the map with it, exactly when seeing
    -- where the guide meant to go would help most.
    if s.module == 0x09 then
      local function oplot(tx, ty)
        return plot(tx * 8 + 4, ty * 8 + 4)
      end
      -- The active route to the immediate target, string-pulled, in pink.
      if ow_route_path then
        for i = 1, #ow_route_path - 1 do
          local ax, ay = oplot(ow_route_path[i][1], ow_route_path[i][2])
          local cx2, cy2 = oplot(ow_route_path[i + 1][1], ow_route_path[i + 1][2])
          canvas:line(ax, ay, cx2, cy2, 0xFF60D0)
        end
      end
      if nav_chain then
        -- Past the active target, draw the rest of the chain: straight pink segments
        -- linking the remaining waypoints, then a marker on each — the next waypoint
        -- white, the rest pink — so the route reads ahead (bushes -> castle door) and
        -- the immediate goal stands out. Only the overworld waypoints are drawn here; a
        -- dungeon point (one with a `room`) belongs to the map inside, not out here.
        for i = nav_chain_i, #nav_chain - 1 do
          if nav_chain[i].room == nil and nav_chain[i].tx
            and nav_chain[i + 1].room == nil and nav_chain[i + 1].tx then
            local ax, ay = oplot(nav_chain[i].tx, nav_chain[i].ty)
            local cx2, cy2 = oplot(nav_chain[i + 1].tx, nav_chain[i + 1].ty)
            canvas:line(ax, ay, cx2, cy2, 0xFF60D0)
          end
        end
        -- Numbered like the dungeon overlay's, and in the same teal, so a number on
        -- either map means the same thing: an index into an authored chain that
        -- `show N` in the editor will find. These markers used to carry no number,
        -- which left UNCLE_APPROACH showing position and nothing to look up.
        local labels = {}
        for i = nav_chain_i, #nav_chain do
          if nav_chain[i].room == nil and nav_chain[i].tx then
            local px, py = oplot(nav_chain[i].tx, nav_chain[i].ty)
            canvas:rect(px - 1, py - 1, 3, 3, (i == nav_chain_i) and 0xFFFFFF or 0xFF60D0)
            labels[#labels + 1] = { px, py, text = tostring(i),
              color = nav_chain.sweep and 0x50D0F0 or 0x20B0A0,
              first = i == nav_chain_i }
          end
        end
        LABELS.number(canvas, labels)
      elseif ow_route_goal then
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
