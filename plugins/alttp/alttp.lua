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

local function facing(direction)
  if direction == 0 then return "north"
  elseif direction == 2 then return "south"
  elseif direction == 4 then return "west"
  else return "east" end
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
-- Whether each sprite slot's enemy has already been announced since it entered
-- the visible screen, so each entrance speaks once. Reset when it leaves.
local announced = {}

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

-- Sprite type id -> name, and which type ids are enemies. Generated from the
-- alttp-navi tables (SPRITE_TYPE_NAMES + ENEMY_NAMES); do not hand-edit.
local SPRITE_NAMES = { [1]="Raven", [2]="Vulture", [8]="Octorok", [9]="Octorok", [10]="Cucco", [12]="Buzzblob", [13]="Snapdragon", [14]="Octoballoon", [15]="Octoballoon baby", [16]="Hinox", [17]="Moblin", [18]="Mini Helmasaur", [19]="Thieves' Town Grate", [21]="Antifairy", [22]="Elder", [23]="Hylian villager", [24]="Mini Moldorm", [25]="Poe", [26]="Leever", [27]="Arrow target", [28]="Statue pullable", [30]="Crystal switch", [31]="Sick Kid", [32]="Sluggula", [33]="Water switch", [34]="Ropa", [35]="Red Bari", [36]="Blue Bari", [37]="Talking tree", [38]="Hardhat Beetle", [39]="Deadrock", [40]="Storyteller", [41]="Zora", [42]="Weathervane", [43]="Pikit", [44]="Maiden at sanctuary", [45]="Apple tree", [47]="Master Sword", [48]="Devalant (non-shooter)", [49]="Devalant (shooter)", [51]="Rupee crab", [53]="Toppo", [55]="Popo", [56]="Popo (2)", [57]="Cane of Byrna spark", [59]="Hylian guard", [61]="Bush hoarder", [62]="Bombable guard", [63]="Whirlpool", [64]="open chest", [65]="Green Soldier", [66]="Blue Soldier", [67]="Red Soldier", [68]="Red Soldier", [69]="Blue Archer", [70]="Green Archer", [71]="Blue Soldier", [72]="Red Soldier", [73]="Red Bomb Soldier", [74]="Green Bomb Soldier", [75]="lantern", [83]="Armos", [84]="Armos Knight", [85]="Lanmola", [86]="Fireball Zora", [87]="Walking Zora", [88]="Crab", [89]="Lost Woods Bird", [91]="Spark (clockwise)", [92]="Spark (counterclockwise)", [93]="Roller (vertical)", [94]="Roller (horizontal)", [96]="Roller (diagonal)", [97]="Beamos", [99]="Debirando", [100]="Debirando (falling)", [102]="Wall cannon (vertical)", [103]="Wall cannon (horizontal)", [104]="Ball and Chain Trooper", [105]="Cannon Soldier", [106]="Ball and Chain Trooper", [107]="Rat", [108]="Rope", [109]="Keese", [110]="Helmasaur King Fireball", [111]="Leever", [112]="Fairy activation", [113]="Uncle / Priest", [114]="Running Man", [115]="Bottle Vendor", [116]="Princess Zelda", [118]="Zelda", [119]="Pipe Down", [120]="Pipe Up", [121]="Pipe Right", [122]="Pipe Left", [123]="Good Bee", [124]="Hylian inscription", [125]="Thief hoarder", [126]="Bug-catching Kid", [128]="Moldorm (Eye)", [129]="Moldorm", [130]="Telepathic tile", [131]="Green Eyegore", [132]="Red Eyegore", [133]="Stalfos", [134]="Kodongo", [135]="Kodongo fire", [136]="Mothula", [137]="Mothula beam", [138]="Spike Trap", [139]="Spike Trap", [140]="Arrghus", [141]="Arrghus spawn", [142]="Terrorpin", [143]="Blob", [144]="Wallmaster", [145]="Stalfos Knight", [146]="Helmasaur King", [147]="Bumper", [149]="Laser Eye (right)", [150]="Laser Eye (left)", [151]="Laser Eye (down)", [152]="Laser Eye (up)", [153]="Pengator", [154]="Kyameron", [155]="Wizzrobe", [160]="Babasu", [161]="Babusu", [162]="Haunted grove hopper", [163]="Lumberjack tree pull", [164]="Teleport bug", [165]="Firesnake", [166]="Hover", [167]="Water Tektite", [168]="Antifairy Circle", [169]="Green Eyegore (mimic)", [170]="Red Eyegore (mimic)", [171]="Yellow Stalfos", [172]="Kodongo", [173]="Flames", [174]="Mothula platform", [177]="Four-way fireball", [178]="Guruguru Bar (clockwise)", [179]="Guruguru Bar (counterclockwise)", [180]="Winder", [181]="Draw bridge", [182]="Rupee pull", [185]="Red Rupee Crab", [186]="Red Bari", [187]="Blue Bari", [188]="Tektite", [200]="Blind", [201]="Blind laser", [203]="Blind", [204]="Kholdstare shell", [206]="Vitreous", [207]="Vitreous (small)", [208]="Viterous lightning", [209]="Catfish", [210]="Agahnim teleport", [211]="Bully / Pink Ball", [212]="Whirlpool", [214]="Ganon", [215]="Agahnim", [216]="Heart", [217]="Green Rupee", [218]="Blue Rupee", [219]="Red Rupee", [220]="Bombs (1)", [221]="Bombs (4)", [222]="Bombs (8)", [223]="Small Magic Jar", [224]="Large Magic Jar", [225]="Arrows (5)", [226]="Arrows (10)", [227]="Fairy", [228]="Small Key", [229]="Big Key", [232]="Mushroom", [233]="Fake Master Sword", [235]="Shopkeeper", [237]="Maiden", [242]="Chest game guy", [244]="Sahasrahla", [245]="Old Man on mountain", [247]="Witch", [249]="Waterfall fairy" }
local ENEMY_TYPES = { [1]=true, [2]=true, [8]=true, [9]=true, [12]=true, [13]=true, [14]=true, [15]=true, [16]=true, [17]=true, [18]=true, [21]=true, [24]=true, [25]=true, [26]=true, [32]=true, [34]=true, [35]=true, [36]=true, [38]=true, [39]=true, [41]=true, [43]=true, [48]=true, [49]=true, [51]=true, [53]=true, [55]=true, [56]=true, [61]=true, [65]=true, [66]=true, [67]=true, [68]=true, [69]=true, [70]=true, [71]=true, [72]=true, [73]=true, [74]=true, [83]=true, [84]=true, [85]=true, [86]=true, [87]=true, [88]=true, [89]=true, [99]=true, [100]=true, [104]=true, [105]=true, [106]=true, [107]=true, [108]=true, [109]=true, [111]=true, [131]=true, [132]=true, [133]=true, [134]=true, [136]=true, [139]=true, [142]=true, [143]=true, [144]=true, [145]=true, [146]=true, [153]=true, [154]=true, [155]=true, [160]=true, [162]=true, [165]=true, [167]=true, [169]=true, [170]=true, [171]=true, [172]=true, [180]=true, [185]=true, [186]=true, [187]=true, [188]=true, [203]=true, [206]=true, [211]=true, [214]=true, [215]=true }

local function sprite_name(kind)
  return SPRITE_NAMES[kind] or (ENEMY_TYPES[kind] and "enemy" or "object")
end

-- Whether a sprite is a threat. Damageable (has health) OR a known enemy type:
-- the type table is not exhaustive, so health is what catches the rest.
local function is_enemy(sp)
  return (sp.hp ~= nil and sp.hp > 0) or ENEMY_TYPES[sp.kind] == true
end

-- What to call an enemy: its type name only when the type is a classified enemy,
-- otherwise just "enemy" — a damageable sprite the table does not name is still a
-- threat, and a wrong name would be worse than none.
local function enemy_name(sp)
  if ENEMY_TYPES[sp.kind] then
    return SPRITE_NAMES[sp.kind] or "enemy"
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
    if st ~= nil and st ~= 0 then
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
-- Types you collect or open. A bright, high tone.
local ITEM_TYPES = { [47]=true, [64]=true, [123]=true, [216]=true, [217]=true, [218]=true, [219]=true, [220]=true, [221]=true, [222]=true, [223]=true, [224]=true, [225]=true, [226]=true, [227]=true, [228]=true, [229]=true, [232]=true, [233]=true }
-- People to talk to and switches to act on — interactable, but not picked up.
local NPC_TYPES = { [22]=true, [23]=true, [27]=true, [30]=true, [31]=true, [33]=true, [37]=true, [40]=true, [44]=true, [112]=true, [113]=true, [114]=true, [115]=true, [116]=true, [118]=true, [126]=true, [235]=true, [237]=true, [242]=true, [244]=true, [245]=true, [247]=true, [249]=true }

-- Per-class tone and reach. `pitch` scales the 330 Hz base tone (higher is
-- brighter); enemies keep the original 1.0. `range` is Manhattan pixels — about
-- 16 to a tile, so 24 is "within a block", the near-only reach for scenery.
local BEACON_KINDS = {
  enemy = { pitch = 1.0, range = 224 },
  item  = { pitch = 2.0, range = 224 },
  npc   = { pitch = 1.5, range = 224 },
  minor = { pitch = 0.5, range = 24 },
}

-- How much a wall between the player and a source dims its beacon: muffled, not
-- silenced, so an occluded threat still registers.
local BEACON_OCCLUDED_SCALE = 0.35

-- Which beacon class a sprite belongs to. Enemies first (a damageable sprite is a
-- threat whatever the type table calls it), then the interactable classes, and
-- everything else is incidental scenery.
local function category(sp)
  if is_enemy(sp) then return "enemy"
  elseif ITEM_TYPES[sp.kind] then return "item"
  elseif NPC_TYPES[sp.kind] then return "npc"
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
-- current area, or nil if it cannot be read. Dungeons index the WRAM grid
-- directly; the overworld goes through the same scroll-offset + ROM decode the
-- map render uses.
local function tile_attr_at(s, px, py)
  if s.module == 0x07 then
    local base = DUNGEON_TILE_TABLE + (mem.u8(LOWER_LEVEL) == 1 and 0x1000 or 0)
    return mem.u8(base + ((py >> 3) & 63) * 64 + ((px >> 3) & 63))
  elseif s.module == 0x09 and #OW_MAP16_TO_MAP8 > 0 then
    local mask_y, mask_x = mem.u16(0x7E070A), mem.u16(0x7E070E)
    if mask_x == 0 or mask_y == 0 then return nil end
    local ow_tx = px >> 3
    local t = (((py - mem.u16(0x7E0708)) & mask_y) * 8) | ((ow_tx - mem.u16(0x7E070C)) & mask_x)
    local byte_off = (t >> 1) * 2
    local lo, hi = mem.u8(OW_TILE_TABLE + byte_off), mem.u8(OW_TILE_TABLE + byte_off + 1)
    if lo == nil or hi == nil then return nil end
    return ow_tile_attr(lo | (hi << 8), ow_tx, py)
  end
  return nil
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
  0x08, 0x09, 0x4B,                                           -- water
  0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56,                   -- solid object
}) do IMPASSABLE[a] = true end

local function tile_passable(s, wtx, wty)
  local attr = tile_attr_at(s, wtx * 8, wty * 8)
  if attr == nil or IMPASSABLE[attr] then return false end
  if s.module == 0x07 and attr == 0x04 then return false end -- indoor wall
  return true
end

-- A* from one world tile to another, both inside Link's current 512-pixel (64
-- tile) window. Returns a list of world tiles {tx, ty} from start to goal, or nil
-- if unreachable / out of the window. 4-connected, Manhattan heuristic, binary
-- heap — a few thousand nodes at most, run on demand, not per frame.
local function plan_path(s, s_tx, s_ty, g_tx, g_ty)
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3 -- window origin
  local slx, sly, glx, gly = s_tx - ox, s_ty - oy, g_tx - ox, g_ty - oy
  if slx < 0 or slx > 63 or sly < 0 or sly > 63 then return nil end
  if glx < 0 or glx > 63 or gly < 0 or gly > 63 then return nil end
  if not tile_passable(s, g_tx, g_ty) then return nil end

  local function h(x, y) return math.abs(x - glx) + math.abs(y - gly) end
  local start, goal = sly * 64 + slx, gly * 64 + glx
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
  while #heap > 0 do
    local n = pop()
    if n == goal then
      local rev, cur = {}, n
      while cur do rev[#rev + 1] = { ox + cur % 64, oy + cur // 64 }; cur = came[cur] end
      local path = {}
      for i = #rev, 1, -1 do path[#path + 1] = rev[i] end
      return path
    end
    if not closed[n] then
      closed[n] = true
      local nx, ny = n % 64, n // 64
      for _, d in ipairs(dirs) do
        local cx, cy = nx + d[1], ny + d[2]
        if cx >= 0 and cx <= 63 and cy >= 0 and cy <= 63
            and tile_passable(s, ox + cx, oy + cy) then
          local c = cy * 64 + cx
          local t = g[n] + 1
          if not closed[c] and (g[c] == nil or t < g[c]) then
            g[c] = t; came[c] = n
            push(c, t + h(cx, cy))
          end
        end
      end
    end
  end
  return nil
end

-- Whether every tile on the straight line between two world tiles is passable.
local function line_passable(s, ax, ay, bx, by)
  local dx, dy = math.abs(bx - ax), -math.abs(by - ay)
  local sx = ax < bx and 1 or -1
  local sy = ay < by and 1 or -1
  local err = dx + dy
  local cx, cy = ax, ay
  while true do
    if not tile_passable(s, cx, cy) then return false end
    if cx == bx and cy == by then return true end
    local e2 = 2 * err
    if e2 >= dy then err = err + dy; cx = cx + sx end
    if e2 <= dx then err = err + dx; cy = cy + sy end
  end
end

-- String-pulling: drop interior waypoints Link can walk straight past, so the
-- guide beacon points at real corners rather than every tile.
local function simplify(s, tiles)
  if #tiles <= 2 then return tiles end
  local out, anchor = { tiles[1] }, 1
  for i = 2, #tiles - 1 do
    if not line_passable(s, tiles[anchor][1], tiles[anchor][2], tiles[i + 1][1], tiles[i + 1][2]) then
      out[#out + 1] = tiles[i]; anchor = i
    end
  end
  out[#out + 1] = tiles[#tiles]
  return out
end

-- Follower state. Global so an agent can inspect/drive it over MCP.
pathfind_active = false
pathfind_path = nil   -- string-pulled list of world-tile waypoints {tx, ty}
pathfind_goal = nil   -- {tx, ty}
local pathfind_wp = 1
local pathfind_area = nil
local pathfind_replan_in = 0

local PATH_PITCH = 3.0         -- a high, distinct navigation tone
local PATH_ALIGNED_PITCH = 3.4 -- brighter when Link faces the way to go
local PATH_VOLUME = 0.7
local WAYPOINT_REACHED = 12    -- px, ~1.5 tiles
local REPLAN_INTERVAL = 45     -- frames; also self-heals straying off the route

local function area_id(s)
  return (s.indoors == 1) and ("d" .. s.dungeon_room) or ("o" .. s.ow_screen)
end

local function pathfind_replan(s)
  if pathfind_goal == nil then return false end
  local tiles = plan_path(s, s.x >> 3, s.y >> 3, pathfind_goal[1], pathfind_goal[2])
  if tiles == nil then return false end
  pathfind_path = simplify(s, tiles)
  pathfind_wp = math.min(2, #pathfind_path)
  pathfind_area = area_id(s)
  pathfind_replan_in = REPLAN_INTERVAL
  return true
end

-- Begin guiding Link to a world-pixel destination. Global for MCP / other cues.
function pathfind_to(wx, wy)
  local s = prev
  if s == nil or not in_play(s) then
    say("Cannot navigate now.", { priority = "navigation", category = "on-demand" })
    return false
  end
  pathfind_goal = { wx >> 3, wy >> 3 }
  if pathfind_replan(s) then
    pathfind_active = true
    say("Following the guide.", { priority = "navigation", category = "on-demand" })
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
  beacon.clear("path")
end

-- Advance the follower one frame and place or clear the guide beacon.
local function pathfind_update(s)
  if not pathfind_active then return end
  if not in_play(s) then beacon.clear("path"); return end

  pathfind_replan_in = pathfind_replan_in - 1
  if pathfind_area ~= area_id(s) or pathfind_replan_in <= 0 then
    if not pathfind_replan(s) then
      say("Lost the path.", { priority = "navigation", category = "on-demand" })
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
    say("You have arrived.", { priority = "navigation", category = "on-demand" })
    pathfind_stop()
    return
  end

  local w = path[pathfind_wp]
  local dx, dy = (w[1] * 8 + 4) - s.x, (w[2] * 8 + 4) - s.y
  local on_course
  if math.abs(dx) > math.abs(dy) then
    on_course = (dx > 0 and s.direction == 6) or (dx < 0 and s.direction == 4)
  else
    on_course = (dy > 0 and s.direction == 2) or (dy < 0 and s.direction == 0)
  end
  beacon.set("path", {
    x = dx, y = dy,
    pitch = on_course and PATH_ALIGNED_PITCH or PATH_PITCH,
    volume = PATH_VOLUME,
  })
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

function on_frame(frame)
  local now = read_state()
  if now == nil then return end
  if prev == nil then
    prev = now
    return -- First frame has nothing to compare against.
  end

  local was = prev
  prev = now

  -- Death outranks everything else that could be happening.
  if now.module == 0x12 and was.module ~= 0x12 then
    say("You died.", { priority = "critical", category = "combat" })
    low_health_warned = false
    return
  end

  -- Damage. Only while actually in play: menu transitions that zero health
  -- would otherwise register as being hit.
  if in_play(now) and now.health < was.health and was.max_health > 0 then
    local lost = (was.health - now.health) / 8.0
    say(
      string.format("Hit. %.1f hearts lost, %.1f left.", lost, hearts(now.health)),
      { priority = "critical", category = "combat", rate_limit = "400ms" }
    )
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

  -- Game text: when a text or menu box opens (module 0x0E), read it aloud.
  if now.module == 0x0E and was.module ~= 0x0E then
    local text = current_dialog_text()
    if text then
      say(text, { priority = "navigation", category = "dialog" })
    end
  end

  -- Top level state changes: file select, entering a dungeon, and so on. The
  -- text module is handled just above, so it is not also announced generically.
  if now.module ~= was.module and now.module ~= 0x0E then
    say(module_name(now.module), { priority = "navigation", category = "area" })
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

  -- Enemies. Announce each by name and direction as it enters the visible screen
  -- ("Green Soldier, north-east."), once per entrance — the spatial-audio beacon
  -- gives the continuous sense of where the nearest one is.
  if in_play(now) then
    local list = sprites()

    -- Speak an enemy as it appears on screen; reset the latch once it leaves, so
    -- a re-entrance speaks again. The arbiter rate-limits a busy room.
    local active = {}
    for _, sp in ipairs(list) do
      active[sp.slot] = sp
    end
    for i = 0, 15 do
      local sp = active[i]
      -- On screen, a threat, and not hidden behind a wall: a sprite the player
      -- could actually see. Occluded enemies stay unannounced until they clear
      -- the wall, so the callout matches what is really visible.
      local visible = sp ~= nil and is_enemy(sp) and on_screen(sp.dx, sp.dy)
        and not sight_blocked(now, now.x, now.y, sp.x, sp.y)
      if visible and not announced[i] then
        say(
          string.format("%s, %s.", enemy_name(sp), direction(sp.dx, sp.dy)),
          { priority = "interaction", category = "enemy" }
        )
        announced[i] = true
      elseif not visible then
        announced[i] = false
      end
    end

    -- Spatial-audio beacons: one tone per class, on the nearest sprite of that
    -- class within its reach. It pans toward the source and grows louder as it
    -- closes. `list` is sorted nearest-first, so the first sprite seen for a
    -- class is its closest one.
    local nearest = {}
    for _, sp in ipairs(list) do
      local c = category(sp)
      if nearest[c] == nil then nearest[c] = sp end
    end
    for name, kind in pairs(BEACON_KINDS) do
      local sp = nearest[name]
      if sp and sp.dist < kind.range then
        -- Quadratic falloff: quieter at a distance, ramping up steeply as the
        -- source closes, rather than a flat linear fade.
        local t = 1 - sp.dist / kind.range
        local vol = t * t
        -- Behind a wall: muffled rather than silent, so a close but occluded
        -- source still registers without sounding like it is out in the open.
        if sight_blocked(now, now.x, now.y, sp.x, sp.y) then
          vol = vol * BEACON_OCCLUDED_SCALE
        end
        beacon.set(name, { x = sp.dx, y = sp.dy, pitch = kind.pitch, volume = vol })
      else
        beacon.clear(name)
      end
    end
  else
    for name in pairs(BEACON_KINDS) do -- no tone in menus or transitions
      beacon.clear(name)
    end
    for i = 0, 15 do
      announced[i] = false
    end
  end

  -- Remember where Link has been, for the explore command.
  if in_play(now) then mark_explored(now) end

  -- Route guidance runs last, so its beacon coexists with the object beacons.
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
    local nm = is_enemy(sp) and enemy_name(sp) or (SPRITE_NAMES[sp.kind] or "object")
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

-- "Guide me to the nearest door." A concrete use of the pathfinder: it scans the
-- current window for door tiles and routes to the nearest one. Other targets
-- (markers, unexplored frontier) can drive pathfind_to the same way.
on_command("pathfind", function()
  local s = prev
  if s == nil or not in_play(s) then
    say("Not in play.", { priority = "navigation", category = "on-demand" })
    return
  end
  local ox, oy = (s.x - s.x % 512) >> 3, (s.y - s.y % 512) >> 3
  local ltx, lty = (s.x >> 3) - ox, (s.y >> 3) - oy
  local best, best_d
  for y = 0, 63 do
    for x = 0, 63 do
      local attr = tile_attr_at(s, (ox + x) * 8, (oy + y) * 8)
      if attr and attr >= 0x30 and attr <= 0x37 then -- a door / passage tile
        local d = math.abs(x - ltx) + math.abs(y - lty)
        if best_d == nil or d < best_d then best_d, best = d, { ox + x, oy + y } end
      end
    end
  end
  if best == nil then
    say("No door nearby.", { priority = "navigation", category = "on-demand" })
  else
    pathfind_to(best[1] * 8 + 4, best[2] * 8 + 4)
  end
end)

on_command("pathfind_stop", function()
  pathfind_stop()
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
            local color = TILE_COLOR[attr] or (attr == 0x04 and INDOOR_WALL_04 or nil)
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
        local block_x = s.x - (s.x % 512)
        local block_y = s.y - (s.y % 512)
        for ty = 0, 63 do
          for tx = 0, 63 do
            local px = block_x + tx * 8
            local py = block_y + ty * 8
            local ow_tx = px >> 3
            local t = (((py - base_y) & mask_y) * 8) | ((ow_tx - base_x) & mask_x)
            local byte_off = (t >> 1) * 2
            if byte_off >= 0 and byte_off + 2 <= 8192 then
              local map16 = string.byte(ow, byte_off + 1) | (string.byte(ow, byte_off + 2) << 8)
              local color = TILE_COLOR[ow_tile_attr(map16, ow_tx, py)]
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
      local px = fx + (sp.x % 512) * fw // 512
      local py = fy + (sp.y % 512) * fw // 512
      canvas:rect(px - 1, py - 1, 3, 3, class_col[category(sp)])
    end

    -- The active guidance route: the same corners the audio beacon leads through,
    -- drawn as a magenta line with a dot at each corner and the current target
    -- brightened — so the guide is legible on the map too.
    if pathfind_active and pathfind_path then
      local function plot(wt)
        return fx + ((wt[1] * 8 + 4) % 512) * fw // 512,
               fy + ((wt[2] * 8 + 4) % 512) * fw // 512
      end
      for i = 1, #pathfind_path - 1 do
        local ax, ay = plot(pathfind_path[i])
        local bx, by = plot(pathfind_path[i + 1])
        canvas:line(ax, ay, bx, by, 0xFF60D0)
      end
      for i, wt in ipairs(pathfind_path) do
        local px, py = plot(wt)
        canvas:rect(px - 1, py - 1, 3, 3, (i == pathfind_wp) and 0xFFFFFF or 0xFF60D0)
      end
    end

    -- Dropped waypoint markers in this area, as small orange squares.
    local here = area_id(s)
    for _, m in pairs(markers) do
      if m.area == here then
        local px = fx + ((m.tx * 8 + 4) % 512) * fw // 512
        local py = fy + ((m.ty * 8 + 4) % 512) * fw // 512
        canvas:rect(px - 1, py - 1, 3, 3, 0xFF9020)
      end
    end

    local lx = fx + (s.x % 512) * fw // 512
    local ly = fy + (s.y % 512) * fw // 512
    canvas:rect(lx - 2, ly - 2, 5, 5, 0x40FF60) -- Link

    -- A short line in the direction he faces.
    local dx, dy = 0, 0
    if s.direction == 0 then dy = -12
    elseif s.direction == 2 then dy = 12
    elseif s.direction == 4 then dx = -12
    else dx = 12 end
    canvas:line(lx, ly, lx + dx, ly + dy, 0xFFF060)

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
