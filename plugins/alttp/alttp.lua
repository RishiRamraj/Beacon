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
-- room/area callout covers location), and the non-interactive title screens.
local MODULE_SILENT = {
  [0x00] = true, -- intro
  [0x07] = true, -- dungeon (in play)
  [0x09] = true, -- overworld (in play)
  [0x0e] = true, -- text box
  [0x14] = true, -- attract mode
}

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
    dungeon_id = mem.u8(A.dungeon_id.addr),
  }
end

-- The game's critical path, as an ordered spine of objectives. The local guide
-- (#28) navigates tactically within a room but has no idea where the player
-- *should* be heading; this is the strategic layer that gives it a destination.
-- Order and gating are cross-checked against a thorough walkthrough; each step's
-- `done` predicate reads the quest-progress bytes so the current objective is
-- inferred from the save, not tracked separately. `done` is evaluated in order
-- and the first unfinished step is the current one.
--
-- The three pendants collectively gate the Master Sword and individually mark
-- their dungeons (Courage=Eastern, Power=Desert, Wisdom=Hera); the seven
-- crystals likewise mark the Dark World dungeons. The intro spine (grab the
-- Lamp, reach Uncle, escort Zelda, beat Agahnim) rides on the $3C5 progress
-- byte, which the game advances 0->1->2->3 at exactly those beats.
local MILESTONES = {
  { goal = "Reach your uncle for the sword",
    hint = "Leave the house and head north into Hyrule Castle. Grab the Lamp on the way, find your dying uncle for the sword and shield, then free Princess Zelda in the cell below.",
    done = function(v) return v.progress >= 1 end },
  { goal = "Escort Zelda to the Sanctuary",
    hint = "Take Zelda up through the castle and out the hidden north passage to the Sanctuary. Then seek out Sahasrahla to begin the hunt for the three Pendants.",
    done = function(v) return v.progress >= 2 end },
  { goal = "Eastern Palace, the first pendant",
    hint = "The big green palace at the far east edge of the Light World. Clear it for the Bow and the Pendant of Courage; Sahasrahla then gives you the Pegasus Boots.",
    done = function(v) return v.pendants & 0x01 ~= 0 end },
  { goal = "Desert Palace, the second pendant",
    hint = "The Desert of Mystery in the southwest. Read the stone tablet there with the Book of Mudora to open the way in. Clear it for the Power Glove and the Pendant of Power.",
    done = function(v) return v.pendants & 0x04 ~= 0 end },
  { goal = "Tower of Hera, the third pendant",
    hint = "The summit of Death Mountain, to the north. Take the Magic Mirror from the old man on the climb and the Moon Pearl inside. Clear it for the Pendant of Wisdom.",
    done = function(v) return v.pendants & 0x02 ~= 0 end },
  { goal = "Claim the Master Sword",
    hint = "Deep in the Lost Woods, northwest. With all three Pendants, pull the Master Sword from its pedestal in the grove.",
    done = function(v) return v.sword >= 2 end },
  { goal = "Hyrule Castle Tower, defeat Agahnim",
    hint = "The Master Sword breaks the barrier around the castle's front tower. Climb to the top and defeat Agahnim; the fight casts you into the Dark World.",
    done = function(v) return v.progress >= 3 end },
  { goal = "Palace of Darkness, the first crystal",
    hint = "Northeast Dark World, near the Pyramid. You need the Moon Pearl to stay human and the Bow. Clear it for the Magic Hammer.",
    done = function(v) return v.crystals & 0x02 ~= 0 end },
  { goal = "Swamp Palace, the second crystal",
    hint = "The southern Dark World swamp. First open the dam in the Light World swamp to lower the water, then Mirror across. Clear it for the Hookshot.",
    done = function(v) return v.crystals & 0x10 ~= 0 end },
  { goal = "Skull Woods, the third crystal",
    hint = "The northwest Dark World woods, the counterpart of the Lost Woods. Clear it for the Fire Rod.",
    done = function(v) return v.crystals & 0x40 ~= 0 end },
  { goal = "Thieves' Town, the fourth crystal",
    hint = "The Village of Outcasts in the west Dark World. Clear it for the Titan's Mitt, which lifts the heavy dark rocks gating the last three dungeons.",
    done = function(v) return v.crystals & 0x20 ~= 0 end },
  { goal = "Ice Palace, the fifth crystal",
    hint = "The island in the far southeast Dark World. Clear it for the Blue Mail.",
    done = function(v) return v.crystals & 0x04 ~= 0 end },
  { goal = "Misery Mire, the sixth crystal",
    hint = "The southwest Dark World. Stand at the entrance and use the Ether Medallion to open it. Clear it for the Cane of Somaria.",
    done = function(v) return v.crystals & 0x01 ~= 0 end },
  { goal = "Turtle Rock, the seventh crystal",
    hint = "The summit of the Dark World Death Mountain, east. Use the Quake Medallion at the Light World Lake of Ill Omen to open it. Clear it for the Mirror Shield.",
    done = function(v) return v.crystals & 0x08 ~= 0 end },
  { goal = "Ganon's Tower, then Ganon",
    hint = "With all seven Crystals the seal on Ganon's Tower, atop the Dark World Death Mountain, lifts. Beat Agahnim again at the top, then finish Ganon at the Pyramid with the Silver Arrows.",
    done = function(_) return false end },
}

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

-- The first unfinished milestone: the player's current objective. Returns its
-- index and record; the last is terminal (never "done") so this always yields.
local function current_milestone(v)
  for i, m in ipairs(MILESTONES) do
    if not m.done(v) then return i, m end
  end
  return #MILESTONES, MILESTONES[#MILESTONES]
end

-- Whether the dungeon Link is standing in has already been cleared, keyed by the
-- $040C dungeon id. "Cleared" means its prize is in hand: the pendant for a Light
-- World dungeon, the crystal for a Dark World one (same bitfields the milestone
-- logic reads). Dungeons without a collectible prize (the castle, the sewer, the
-- towers) are never "done" here — the milestone spine covers those. If the guide
-- finds the current dungeon cleared, there is nothing left to fetch and it heads
-- for the exit.
local DUNGEON_DONE = {
  [0x04] = function(v) return v.pendants & 0x01 ~= 0 end, -- Eastern  -> Courage
  [0x06] = function(v) return v.pendants & 0x04 ~= 0 end, -- Desert   -> Power
  [0x14] = function(v) return v.pendants & 0x02 ~= 0 end, -- Hera     -> Wisdom
  [0x0C] = function(v) return v.crystals & 0x02 ~= 0 end, -- Dark Palace
  [0x0A] = function(v) return v.crystals & 0x10 ~= 0 end, -- Swamp
  [0x10] = function(v) return v.crystals & 0x40 ~= 0 end, -- Skull Woods
  [0x16] = function(v) return v.crystals & 0x20 ~= 0 end, -- Thieves' (Gargoyle)
  [0x12] = function(v) return v.crystals & 0x04 ~= 0 end, -- Ice
  [0x0E] = function(v) return v.crystals & 0x01 ~= 0 end, -- Misery Mire
  [0x18] = function(v) return v.crystals & 0x08 ~= 0 end, -- Turtle Rock
}

-- The SRAM room-data table: one 16-bit word per dungeon room at $7EF000 + room*2
-- (room is the $00A0 value). Bit layout, high byte `dddd b k ck cr`, low byte
-- `cccc qqqq`: bits 4-7 chests opened, bit 10 key/item taken, bit 11 boss beaten.
-- Lets the guide tell which of a dungeon's chests and its boss are already done.
local ROOM_DATA = 0x7EF000
local function room_word(room)
  return mem.u8(ROOM_DATA + room * 2) + mem.u8(ROOM_DATA + room * 2 + 1) * 256
end
local function room_chests_opened(room) return (room_word(room) >> 4) & 0x0F end
local function room_item_taken(room) return room_word(room) & 0x0400 ~= 0 end
local function room_boss_beaten(room) return room_word(room) & 0x0800 ~= 0 end

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
-- the visible screen, so each entrance speaks once. Reset only once it has left
-- the screen — not merely ducked out of line of sight, so a patrolling enemy
-- weaving behind cover is not re-announced as a fresh enemy each time it steps
-- back into view (which sounds like a whole sequence of enemies).
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

-- Sprite type id -> name. Regenerated from the ALttP disassembly's sprite-prep
-- jump table (walkingeyerobot/alttp-disassembly, sprite_prep.asm), which is the
-- game's own per-type dispatch — the authoritative meaning of the $7E0E20 type
-- Beacon reads. (The earlier alttp-navi-derived table was wrong for most
-- non-enemy sprites.) ITEM_TYPES/NPC_TYPES follow those names; ENEMY_TYPES is the
-- prior set minus the ids the corrected names show to be NPCs or objects.
local SPRITE_NAMES = { [0]="Raven", [1]="Vulture", [2]="Flying Stalfos Head", [4]="Good Switch", [5]="Switch", [6]="Bad Switch", [7]="Switch again, facing up", [8]="Octorock", [9]="Giant Moldorm", [10]="Four Shooter Octorock", [11]="Chicken", [12]="Octorock projectile", [13]="Buzzblob", [14]="Plants with big mouths", [15]="Octoballoon", [16]="Octospawn", [17]="Hinox", [18]="Moblin", [19]="Helmasaur", [20]="Gargoyle Grate", [21]="Bubble", [22]="Sahasrahla", [23]="Rupee Crab under bush", [24]="Moldorm", [25]="Poe", [26]="Dwarves and helper sprites", [27]="Arrow in Wall", [28]="Movable Statue", [29]="Weathervane", [30]="Crystal Switch", [31]="Bug Net Kid", [32]="Sluggula", [33]="Push Switch", [34]="Ropa", [35]="Bari (Blue)", [36]="Bari (Red)", [37]="Conversational Tree", [38]="Hardhat Beetle", [39]="Deadrock", [40]="Story Teller Set 1", [41]="Human NPC Set 1", [42]="Sweeping lady", [43]="Hobo under bridge", [44]="Lumberjack Bros", [45]="Telepathic Stones", [46]="Flute Boy's Notes", [47]="Race Game Couple", [48]="Person", [49]="Fortune Teller", [50]="Quarrel Bros", [51]="Pull For Rupees", [52]="Young Snitch Girl", [53]="Inn Keeper", [54]="Witch", [55]="Waterfall", [56]="Arrow Target", [57]="Middle-aged desert guy", [58]="Mad Batter", [59]="Dash item", [60]="Kid in village near trough", [61]="Old Snitch Lady", [62]="Rupee Crab under rock", [63]="Tutorial Soldier", [64]="Barrier", [65]="Green Soldier", [66]="Blue Soldier", [67]="Red Spear Soldier", [68]="Psycho Trooper", [69]="Psycho Spear Soldier", [70]="Blue Archer Soldier", [71]="Green Archer Bush Soldier", [72]="Red Javelin Trooper", [73]="Red Javelin Bush Soldier", [74]="Green Enemy Bombs", [75]="Green Soldier (weak version)", [76]="Gerudo Man", [77]="Toppo", [78]="Popo", [79]="Bot", [80]="Metal Ball", [81]="Armos", [82]="Zora King", [83]="Armos Knight", [84]="Lanmola", [85]="Zora and Fireball", [86]="Walking Zora", [87]="Desert Palace barriers", [88]="Crab", [89]="Lost Woods Bird", [90]="Lost Woods Squirrel", [91]="Spark (clockwise)", [92]="Spark (counter-clockwise)", [93]="Roller (down then up)", [94]="Roller (up then down)", [95]="Roller", [96]="Roller", [97]="Beamos", [98]="Master Sword", [99]="Debirando Pit", [100]="Debirando", [101]="Archery Game Guy", [102]="Wall Cannon", [103]="Wall Cannon", [104]="Wall Cannon", [105]="Wall Cannon", [106]="Ball And Chain Trooper", [107]="Cannon Trooper", [108]="Warp Vortex", [109]="Rat", [110]="Rope", [111]="Keese", [112]="Helmasaur King Fireball", [113]="Leever", [114]="Pond Activator", [115]="Link's Uncle", [116]="Red Hat Wussy", [117]="Bottle Vendor", [118]="Princess Zelda", [119]="Alternate Bubble", [120]="Elder's Wife", [121]="Good Bee stuck in Ice Cavern", [122]="Agahnim", [123]="Agahnim energy", [124]="Green Stalfos", [125]="Spike Trap", [126]="Guruguru Bar", [127]="Guruguru Bar", [128]="Wandering Fireball Chains", [129]="Hover", [130]="Bubble Group", [131]="Eyegore", [132]="Eyegore 2", [133]="Yellow Stalfos", [134]="Kodondo", [135]="Flames", [136]="Mothula", [137]="Mothula Beam", [138]="Spike Block", [139]="Gibdo", [140]="Arrghus", [141]="Arrgi", [142]="Chair Turtles (kill with hammer)", [143]="Terrorpin", [144]="Grabber Things", [145]="Stalfos Knight", [146]="Helmasaur King", [147]="Bumper", [148]="Pirogusu", [149]="Laser Eye (right)", [150]="Laser Eye (left)", [151]="Laser Eye (down)", [152]="Laser Eye (up)", [153]="Attack Penguin", [154]="Kyameron", [155]="Wizzrobe", [156]="Zoro", [157]="Babusu", [158]="Ostrich seen with Flute Boy", [159]="Rabbit seen with Flute Boy", [160]="Bird seen with Flute Boy", [161]="Freezor", [162]="Kholdstare", [163]="Kholdstare part 2", [164]="Kholdstare Ice balls", [165]="Blue Zazak", [166]="Red Zazak", [167]="Stalfos", [168]="Green Bomber", [169]="Blue Bomber", [170]="Pikit", [171]="Crystal Maiden", [172]="Apple(s) in tree", [173]="Old Mountain Man", [174]="Down Pipe", [175]="Up Pipe", [176]="Right Pipe", [177]="Left Pipe", [178]="Good Bee", [179]="Hylian Inscription", [180]="Thief Chest", [181]="Bomb Shop Guy and company", [183]="Blind disguised as a Maiden", [184]="Dialogue Testing Sprite", [185]="Bully and Ball Guy", [186]="Whirlpool", [187]="Shopkeeper", [188]="Drunk in the Inn", [189]="Vitreous", [190]="Smaller Vitreous Eyeballs", [191]="Vitreous Lightning Blast", [192]="Giant Cranky Catfish", [193]="Agahnim Teleporting Zelda", [194]="Boulder", [195]="Gibo", [196]="Thief", [197]="Evil Fireball Spitters", [198]="Fourway Fireball Spitters", [199]="Hokbok", [200]="Big Faerie", [201]="Ganon Helpers + Tektite", [202]="Chain Chomp", [203]="Agahnim", [204]="Trinexx Part 2", [205]="Trinexx Part 3", [206]="Blind", [207]="Swamola", [208]="Lynel", [209]="Yellow Transform", [210]="Flopping Fish", [211]="Stal", [212]="Landmine", [213]="Digging Game Guy", [214]="Ganon", [215]="InvinceoGanon", [216]="Heart Refill", [217]="Green Rupee", [218]="Blue Rupee", [219]="Red Rupee", [220]="1 Bomb Refill", [221]="4 Bomb Refill", [222]="8 Bomb Refill", [223]="Small Magic Refill", [224]="Full Magic Refill", [225]="5 Arrow Refill", [226]="10 Arrow Refill", [227]="Faerie", [228]="Key", [229]="Big Key", [230]="Shield Pickup", [231]="Mushroom", [232]="Fake Master Sword", [233]="Magic Shop Dude", [234]="Heart Container", [235]="Heart Piece", [236]="Bush", [237]="Cane of Somaria Platform", [238]="Movable Mantle", [239]="Cane of Somaria Platform", [240]="Cane of Somaria Platform", [241]="Cane of Somaria Platform", [242]="Medallion Tablet" }
local ENEMY_TYPES = { [1]=true, [2]=true, [8]=true, [9]=true, [12]=true, [13]=true, [14]=true, [15]=true, [16]=true, [17]=true, [18]=true, [21]=true, [24]=true, [25]=true, [32]=true, [34]=true, [35]=true, [36]=true, [38]=true, [39]=true, [65]=true, [66]=true, [67]=true, [68]=true, [69]=true, [70]=true, [71]=true, [72]=true, [73]=true, [74]=true, [83]=true, [84]=true, [85]=true, [86]=true, [88]=true, [99]=true, [100]=true, [104]=true, [105]=true, [106]=true, [107]=true, [109]=true, [111]=true, [131]=true, [132]=true, [133]=true, [134]=true, [136]=true, [139]=true, [142]=true, [143]=true, [144]=true, [145]=true, [146]=true, [153]=true, [154]=true, [155]=true, [162]=true, [165]=true, [167]=true, [169]=true, [170]=true, [185]=true, [203]=true, [206]=true, [211]=true, [214]=true, [215]=true }

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
local ITEM_TYPES = { [98]=true, [178]=true, [216]=true, [217]=true, [218]=true, [219]=true, [220]=true, [221]=true, [222]=true, [223]=true, [224]=true, [225]=true, [226]=true, [227]=true, [228]=true, [229]=true, [230]=true, [231]=true, [234]=true, [235]=true }
-- People to talk to and switches to act on — interactable, but not picked up.
local NPC_TYPES = { [22]=true, [30]=true, [31]=true, [33]=true, [47]=true, [49]=true, [53]=true, [54]=true, [60]=true, [76]=true, [82]=true, [115]=true, [117]=true, [118]=true, [120]=true, [171]=true, [173]=true, [187]=true, [233]=true }

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
  return ow_tile_attr(ow_map16(m[n], cn), px >> 3, py)
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

local PATH_PITCH = 3.0         -- a high, distinct navigation tone
local PATH_ALIGNED_PITCH = 3.4 -- brighter when Link faces the way to go
local PATH_VOLUME = 0.30        -- kept well under the object beacons so threats read over the guide
local PATH_PING_HZ = 0.5        -- sonar: a ping every 2 seconds over a soft steady tone
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
    tremolo = PATH_PING_HZ, ping = true, -- sonar ping over a soft steady tone
  })
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
      if attr and attr >= 0x30 and attr <= 0x37 then -- a door / passage tile
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
      if attr and ((attr >= 0x58 and attr <= 0x5D) or attr == 0x63) then
        local d = math.abs(x - ltx) + math.abs(y - lty)
        if best_d == nil or d < best_d then
          best_d, best = d, { (ox + x) * 8 + 4, (oy + y) * 8 + 4 }
        end
      end
    end
  end
  return best
end

-- The nearest on-screen item pickup (a sprite in ITEM_TYPES), as world pixel
-- coordinates, or nil. sprites() is sorted nearest-first, so the first match is
-- the closest. Used by the dungeon guide to fetch a loose item in the room.
local function nearest_item_sprite(s)
  for _, sp in ipairs(sprites()) do
    if ITEM_TYPES[sp.kind] then return { sp.x, sp.y } end
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
local function walkable_near(s, wx, wy)
  local tx, ty = wx >> 3, wy >> 3
  for r = 0, 8 do
    for dy = -r, r do
      for dx = -r, r do
        if math.max(math.abs(dx), math.abs(dy)) == r and tile_passable(s, tx + dx, ty + dy) then
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

-- Forward declaration: intro_step (the current scripted-intro beat) is defined
-- with the advance guide far below, but the objective readout above it consults
-- it too; both reference this upvalue, assigned once the chain is defined.
local intro_step

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
local function route_set_goal(s, wx, wy)
  pathfind_goal = { wx >> 3, wy >> 3 }
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
    say("You've reached the room.", { priority = "navigation", category = "on-demand" })
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
    say("You have arrived.", { priority = "navigation", category = "on-demand" })
    ow_route_stop(); beacon.clear("path"); return
  end
  local w = path[ow_route_wp]
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
    tremolo = PATH_PING_HZ, ping = true, -- sonar ping over a soft steady tone
  })
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

  -- Top level state changes: file select, entering a dungeon, and so on. Some
  -- modules are deliberately silent: the text module (0x0E, handled just above),
  -- the dungeon (0x07) and overworld (0x09) — being in one is obvious and the
  -- room / area callout below already says where — and the non-interactive title
  -- screens, intro (0x00) and attract mode (0x14), which the player never chose
  -- to enter. Announcing any of these is just noise.
  if now.module ~= was.module and not MODULE_SILENT[now.module] then
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
      -- "Present" = a threat that is on the visible screen, whether or not a wall
      -- currently hides it. An occluded enemy is still present, so it stays
      -- latched; only leaving the screen re-arms it.
      local present = sp ~= nil and is_enemy(sp) and on_screen(sp.dx, sp.dy)
      if not present then
        announced[i] = false -- off screen: a genuine re-entrance may speak again
      elseif not announced[i]
          and not sight_blocked(now, now.x, now.y, sp.x, sp.y) then
        -- Announce once, when it is actually in the clear. If it first appears
        -- occluded it waits, but it will not re-announce merely for stepping back
        -- into line of sight after ducking behind cover.
        say(
          string.format("%s, %s.", enemy_name(sp), direction(sp.dx, sp.dy)),
          { priority = "interaction", category = "enemy" }
        )
        announced[i] = true
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
        -- source closes, rather than a flat linear fade. The class gain scales it
        -- (enemies boosted so they carry over the guide), clamped to full volume.
        local t = 1 - sp.dist / kind.range
        local vol = math.min(1, t * t * (kind.gain or 1))
        -- Behind a wall: muffled rather than silent, so a close but occluded
        -- source still registers without sounding like it is out in the open.
        if sight_blocked(now, now.x, now.y, sp.x, sp.y) then
          vol = vol * BEACON_OCCLUDED_SCALE
        end
        -- Each class swells at its own fixed rate (enemies 120 BPM, pickups 60).
        beacon.set(name, { x = sp.dx, y = sp.dy, pitch = kind.pitch, volume = vol, tremolo = kind.tremolo })
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

  -- Learn the dungeon's room connectivity as Link walks, and re-aim any active
  -- cross-room route at each room boundary, before the local follower runs.
  record_room_transition(now)
  -- Keep the navigation assist aimed at the objective as beats complete and Link
  -- crosses screens, before the followers it drives so a fresh target takes effect
  -- this frame.
  nav_update(now)
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
  local idx, m = current_milestone(v)
  say(
    string.format("Objective %d of %d: %s. %s", idx, #MILESTONES, m.goal, m.hint),
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
-- game, not just the room: in a dungeon it heads for the next thing worth taking
-- (or the exit, once the dungeon is cleared or you lack what it takes to finish);
-- on the overworld it heads for the next place the main story wants you.
--
-- The destinations come from researched data, not guesses:
--   * DUNGEON_NAV[dungeon_id] — each dungeon's signature item, Big Key and boss
--     rooms, from a thorough walkthrough cross-checked against the randomizer's
--     room table and the disassembly's underworld-room list.
--   * MILESTONE_AREA[milestone] — the overworld area each story step sends you to.
-- The pathfinder only navigates within the current room, and dungeon room ids
-- stack floors in one grid, so a cross-room heading is a rough hint, not a path:
-- the guide names the goal and points you the right way, guiding precisely only
-- once you are in the room the target sits in.
-- ===========================================================================
local nav_say = function(text)
  say(text, { priority = "navigation", category = "on-demand" })
end

-- Milestone index -> the overworld area its destination sits in. The area byte
-- carries the +0x40 Dark World offset, so it doubles as the world marker. From
-- the Archipelago randomizer entrance table, cross-checked against the
-- disassembly's overworld-area names (both agree on every value).
local MILESTONE_AREA = {
  [1]  = 0x1B, -- Hyrule Castle (reach uncle, free Zelda)
  [2]  = 0x13, -- Sanctuary
  [3]  = 0x1E, -- Eastern Palace
  [4]  = 0x30, -- Desert Palace
  [5]  = 0x03, -- Tower of Hera (west Death Mountain)
  [6]  = 0x00, -- Master Sword pedestal (Lost Woods)
  [7]  = 0x1B, -- Hyrule Castle Tower (Agahnim)
  [8]  = 0x5E, -- Palace of Darkness
  [9]  = 0x7B, -- Swamp Palace
  [10] = 0x40, -- Skull Woods
  [11] = 0x58, -- Thieves' Town
  [12] = 0x75, -- Ice Palace
  [13] = 0x70, -- Misery Mire
  [14] = 0x47, -- Turtle Rock
  [15] = 0x43, -- Ganon's Tower
}

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

-- Per-dungeon navigation data, keyed by $040C dungeon id. The canonical spine of
-- a dungeon is: fetch its signature item, then the Big Key, then beat the boss.
-- Each entry gives how to tell the signature item is in hand (an inventory read),
-- the Big Key bit for this dungeon (in the $366/$367 big-key bitfields), and the
-- room ids of the item, the Big Key, the boss, and the entrance. Room ids are the
-- $00A0 value; all boss/entrance/chest room ids are confirmed against the
-- randomizer chest table and the disassembly's underworld-room list. Small keys,
-- map and compass are deliberately not tracked — they are not what a player is
-- steered toward, and pot/enemy-drop keys have no stable room id.
local DUNGEON_NAV = {
  [0x04] = { name = "Eastern Palace",     item = "the Bow",
             have = function() return mem.u8(0x7EF340) >= 1 end,
             item_room = 0xA9, bk_byte = 0x7EF367, bk_bit = 0x20, bk_room = 0xB8,
             boss_room = 0xC8, entrance_room = 0xC9 },
  [0x06] = { name = "Desert Palace",      item = "the Power Glove",
             have = function() return mem.u8(0x7EF354) >= 1 end,
             item_room = 0x73, bk_byte = 0x7EF367, bk_bit = 0x10, bk_room = 0x75,
             boss_room = 0x33, entrance_room = 0x84 },
  [0x14] = { name = "Tower of Hera",      item = "the Moon Pearl",
             have = function() return mem.u8(0x7EF357) >= 1 end,
             item_room = 0x27, bk_byte = 0x7EF366, bk_bit = 0x20, bk_room = 0x87,
             boss_room = 0x07, entrance_room = 0x77 },
  [0x0C] = { name = "Palace of Darkness", item = "the Magic Hammer",
             have = function() return mem.u8(0x7EF34B) >= 1 end,
             item_room = 0x1A, bk_byte = 0x7EF367, bk_bit = 0x02, bk_room = 0x3A,
             boss_room = 0x5A, entrance_room = 0x4A },
  [0x0A] = { name = "Swamp Palace",       item = "the Hookshot",
             have = function() return mem.u8(0x7EF342) >= 1 end,
             item_room = 0x36, bk_byte = 0x7EF367, bk_bit = 0x04, bk_room = 0x35,
             boss_room = 0x06, entrance_room = 0x28 },
  [0x10] = { name = "Skull Woods",        item = "the Fire Rod",
             have = function() return mem.u8(0x7EF345) >= 1 end,
             item_room = 0x58, bk_byte = 0x7EF366, bk_bit = 0x80, bk_room = 0x57,
             boss_room = 0x29, entrance_room = nil }, -- three overworld entrances
  [0x16] = { name = "Thieves' Town",      item = "the Titan's Mitt",
             have = function() return mem.u8(0x7EF354) >= 2 end,
             item_room = 0x44, bk_byte = 0x7EF366, bk_bit = 0x10, bk_room = 0xDB,
             boss_room = 0xAC, entrance_room = 0xDB },
  [0x12] = { name = "Ice Palace",         item = "the Blue Mail",
             have = function() return mem.u8(0x7EF35B) >= 1 end,
             item_room = 0x9E, bk_byte = 0x7EF366, bk_bit = 0x40, bk_room = 0x1F,
             boss_room = 0xDE, entrance_room = 0x0E },
  [0x0E] = { name = "Misery Mire",        item = "the Cane of Somaria",
             have = function() return mem.u8(0x7EF350) >= 1 end,
             item_room = 0xC3, bk_byte = 0x7EF367, bk_bit = 0x01, bk_room = 0xD1,
             boss_room = 0x90, entrance_room = 0x98 },
  [0x18] = { name = "Turtle Rock",        item = "the Mirror Shield",
             have = function() return mem.u8(0x7EF35A) >= 3 end,
             item_room = 0x24, bk_byte = 0x7EF366, bk_bit = 0x08, bk_room = 0x14,
             boss_room = 0xA4, entrance_room = 0xD6 },
}

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
      if attr and attr >= 0x30 and attr <= 0x37 then
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
  return a ~= nil and ((a >= 0x30 and a <= 0x37) or a == 0x8E or a == 0x8F
    or (a >= 0x1D and a <= 0x1F) or (a >= 0x3D and a <= 0x3F))
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

-- The tile-types of a floor-changing spiral staircase, from the game's own tile
-- detection (zelda3 tile_detect.c, TileDetect_ExecuteInner): north/up stairs read
-- as 0x1D-0x1F, down stairs as 0x3D-0x3F. (The 0x30-0x37 the door finder keys on
-- also count as stair tiles there, but they are the in-plane doorways; 0x38-0x3C
-- are ordinary floor.) An Up hop wants the up set, a Down hop the down set.
local STAIR_UP   = { [0x1D] = true, [0x1E] = true, [0x1F] = true }
local STAIR_DOWN = { [0x3D] = true, [0x3E] = true, [0x3F] = true }

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

-- In a dungeon: cleared -> the exit; otherwise the next canonical target — the
-- signature item, then the Big Key, then the boss.
local function advance_dungeon(s, v)
  ow_route_stop() -- drop any overworld route once inside a dungeon
  local nav = DUNGEON_NAV[s.dungeon_id]
  local done = DUNGEON_DONE[s.dungeon_id]
  local cleared = (done and v and done(v)) or (nav and room_boss_beaten(nav.boss_room))
  if cleared then
    if not route_to_room(s, nav and nav.entrance_room, "Dungeon cleared. Heading for the exit.") then
      local d = nearest_door_tile(s)
      if d then pathfind_to(d[1], d[2]) end
      nav_say("Dungeon cleared. Find the stairs out.")
    end
    return
  end
  if nav == nil then
    -- No canonical spine for this place (sewer, castle, the towers): keep the
    -- player moving — a loose item in the room, else into unexplored ground.
    local it = nearest_item_sprite(s)
    if it then pathfind_to(it[1], it[2]); nav_say("Guiding to an item in this room."); return end
    local tx, ty = nearest_unexplored(s)
    if tx then pathfind_to(tx * 8 + 4, ty * 8 + 4); nav_say("Guiding you deeper into the dungeon.")
    else
      local d = nearest_door_tile(s)
      if d then pathfind_to(d[1], d[2]) end
      nav_say("Heading for the next door.")
    end
    return
  end
  -- The canonical spine: signature item, then Big Key, then boss.
  if not nav.have() then
    route_to_room(s, nav.item_room, "Next: " .. nav.item .. ".")
  elseif (mem.u8(nav.bk_byte) & nav.bk_bit) == 0 then
    route_to_room(s, nav.bk_room, "Next: the Big Key.")
  else
    route_to_room(s, nav.boss_room, "Head for the boss.")
  end
end

-- On the overworld: head for the current story milestone's destination — a
-- compass heading across the area grid toward it, and a note when it is in the
-- other world. If already in the target area, point at finding the entrance.
local function advance_overworld(s, v)
  room_route_stop() -- drop any stale dungeon route when out on the overworld
  if v == nil then nav_say("No game state yet."); return end
  local idx, m = current_milestone(v)
  local area = MILESTONE_AREA[idx]
  if area == nil then
    nav_say(string.format("Next: %s. %s", m.goal, m.hint))
    return
  end
  -- If the destination is in the other world, we can't draw a path across the
  -- mirror — name it and give a heading instead.
  local other_world = (s.ow_screen & 0x40) ~= (area & 0x40)
  if other_world then
    local dir = area_heading(s.ow_screen, area)
    local which = (area & 0x40) ~= 0 and "Dark World" or "Light World"
    nav_say(string.format("Next: %s. Head %s, then cross to the %s.", m.goal, dir or "toward it", which))
    return
  end
  -- Already on the destination screen (comparing parents, so a large 2x2 area
  -- counts wherever Link stands in it): the objective is here, so stop routing
  -- and hand off — the exact entrance is a per-objective waypoint, still to come.
  if ow_parent(s.ow_screen & 0x3F) == ow_parent(area & 0x3F) then
    ow_route_stop()
    nav_say(string.format("You're at %s. Look for the entrance.", m.goal))
    return
  end
  -- Same world, elsewhere: draw a path onto the destination screen (nearest
  -- reachable tile there), rather than a possibly-walled centre.
  ow_route_to_area(area)
  nav_say(string.format("Routing to %s.", m.goal))
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
local CASTLE_AREA = 0x1B
local SANCTUARY_AREA, SANCTUARY_ROOM = 0x13, 0x12

-- The castle entrance the intro drops into to reach Uncle — a hole (tile type
-- 0x20) you fall into, so no sword or bush-cutting is needed here (the bush-hidden
-- entrance is a later, sword-gated route). The hole itself reads as impassable, so
-- the pathfinder cannot route onto it; aim instead at the walkable tile just south
-- (world tile 304, 214 — read live from the game) and tell the player to step
-- north in. The overworld area is Hyrule Castle 0x1B.
local CASTLE_ENTRANCE = {
  tx = 304, ty = 214,
  say = "Step north into the castle entrance.",
}

-- Route toward an intro beat from wherever Link is. In a dungeon room the graph
-- connects to the target, door-to-door route there (stage 2). In an indoor room
-- the graph does not reach yet — Link's house at the very start — head for the
-- door out. On the overworld, take the stage-1 cross-screen path to the area;
-- once standing in it, aim at a known entrance waypoint if the beat gives one,
-- else hand off to look for the way in.
local function head_for(s, area, room, label, entrance)
  if s.module == 0x07 then
    if room and (room == s.dungeon_room or room_path(s.dungeon_room, room)) then
      route_to_room(s, room, label)
      return
    end
    -- An interior the graph does not reach (Link's house at the start, or the
    -- castle secret-entrance room whose onward passage the rando omits). Head for
    -- the exit — door, entrance passage or staircase — that points toward the
    -- target room on the dungeon-room grid (low nibble = column, high = row), so a
    -- room with several exits picks the right one. Fall back to that heading's
    -- edge. The local A* gets Link there; the auto-follow re-aims once he crosses.
    local ddx, ddy = 0, 1 -- default heading: south (a house exits at its bottom)
    if room and s.dungeon_room <= 0xFF then
      -- Both are standard dungeon rooms on the 16-wide grid; head toward the target.
      ddx = (room & 0x0F) - (s.dungeon_room & 0x0F)
      ddy = (room >> 4) - (s.dungeon_room >> 4)
    end
    local d = exit_toward(s, ddx, ddy)
      or room_edge_goal(s, ddx > 0 and 1 or ddx < 0 and -1 or 0, ddy >= 0 and 1 or -1)
    if d then pathfind_to(d[1], d[2]) end
    nav_say(label .. " Head for the way out.")
  elseif ow_parent(s.ow_screen & 0x3F) == ow_parent(area & 0x3F) then
    if entrance then
      pathfind_to(entrance.tx * 8 + 4, entrance.ty * 8 + 4)
      nav_say(label .. " " .. entrance.say)
    else
      ow_route_stop()
      nav_say(label .. " Look for the entrance.")
    end
  else
    ow_route_to_area(area)
    nav_say(label .. " Routing there.")
  end
end

local INTRO = {
  { key = "lamp",
    goal = "Grab the Lamp from the chest",
    hint = "There is a treasure chest in your house holding the Lamp — take it for the dark passages ahead.",
    -- Met once the Lamp is held, or once you have moved past the start (sword in
    -- hand / progress bumped), so skipping it never leaves the guide nagging.
    done = function(v) return mem.u8(0x7EF34A) >= 1 or v.sword >= 1 or v.progress >= 1 end,
    act = function(s, v)
      if s.module == 0x07 then
        local c = nearest_chest_tile(s)
        if c then pathfind_to(c[1], c[2]); nav_say("Open the chest for the Lamp."); return end
      end
      head_for(s, CASTLE_AREA, 0x55, "Head into Hyrule Castle's hidden entrance for your uncle.", CASTLE_ENTRANCE)
    end },
  { key = "uncle",
    goal = "Reach your uncle for the sword",
    hint = "Enter Hyrule Castle by the hidden passage — the bush against the wall drops you in — and reach your dying uncle for the sword and shield.",
    done = function(v) return v.sword >= 1 or v.progress >= 1 end,
    act = function(s, v)
      if s.module == 0x07 and s.dungeon_room == 0x55 then
        local u = nearest_sprite_kind(s, 115) -- Link's Uncle
        if u then pathfind_to(walkable_near(s, u[1], u[2])) end
        nav_say("Your uncle is in this room. Reach him for the sword.")
        return
      end
      head_for(s, CASTLE_AREA, 0x55, "Reach your uncle for the sword.", CASTLE_ENTRANCE)
    end },
  { key = "zelda",
    goal = "Free Princess Zelda",
    hint = "Descend through the castle to the dungeon below and free Princess Zelda from her cell.",
    done = function(v) return mem.u8(0x7EF3CC) == 1 or v.progress >= 2 end,
    act = function(s, v)
      if s.module == 0x07 and s.dungeon_room == 0x80 then
        local z = nearest_sprite_kind(s, 118) -- Princess Zelda
        if z then pathfind_to(walkable_near(s, z[1], z[2])) end
        nav_say("Zelda is in this cell. Reach her.")
        return
      end
      head_for(s, CASTLE_AREA, 0x80, "Free Princess Zelda from her cell.")
    end },
  { key = "sanctuary",
    goal = "Escort Zelda to the Sanctuary",
    hint = "Lead Zelda back up through the castle and out the hidden north passage to the Sanctuary.",
    done = function(v) return v.progress >= 2 end,
    act = function(s, v)
      head_for(s, SANCTUARY_AREA, SANCTUARY_ROOM, "Escort Zelda to the Sanctuary.")
    end },
}

-- The current intro beat: the first step not yet met, or nil once the intro is
-- over (Zelda delivered, progress >= 2) so the milestone spine takes over.
intro_step = function(v)
  if v == nil or v.progress >= 2 then return nil end
  for i, step in ipairs(INTRO) do
    if not step.done(v) then return i, step, #INTRO end
  end
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

-- Aim the guide at the current objective from wherever Link stands: the scripted
-- intro beat while the intro runs, else the dungeon spine, else the overworld
-- milestone. Each announces what it is heading for.
local function nav_reaim(s, v)
  local _, step = intro_step(v)
  if step then
    step.act(s, v)
  elseif s.module == 0x07 then
    advance_dungeon(s, v)
  else
    advance_overworld(s, v)
  end
end

-- What the guide should be heading toward, plus which module Link is in — so it
-- re-aims exactly when either changes and stays put otherwise. The objective is
-- the intro beat while the intro runs; in a dungeon, the spine phase (whether the
-- signature item, Big Key and boss are done, so grabbing the item re-aims at the
-- Big Key); on the overworld, the current milestone.
local function nav_signature(s, v)
  local _, step = intro_step(v)
  local obj
  if step then
    obj = "in:" .. step.key
  elseif s.module == 0x07 then
    local nav = DUNGEON_NAV[s.dungeon_id]
    if nav then
      local a = nav.have() and 1 or 0
      local b = (mem.u8(nav.bk_byte) & nav.bk_bit) ~= 0 and 1 or 0
      local c = room_boss_beaten(nav.boss_room) and 1 or 0
      obj = string.format("dg%d.%d%d%d", s.dungeon_id, a, b, c)
    else
      obj = "dg" .. s.dungeon_id
    end
  else
    obj = "ms" .. (current_milestone(v))
  end
  return s.module .. ":" .. obj
end

-- Turn the assist off and drop every route it was driving.
local function nav_stop()
  nav_active = false
  nav_sig = nil
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
  if sig ~= nav_sig then
    nav_sig = sig
    nav_reaim(s, v)
  end
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

    -- The cross-screen overworld route, drawn through the current 512-pixel
    -- window; the segment leaving the screen edge points on toward the next area.
    -- World tiles are placed relative to Link's block, so off-window corners clip.
    if s.module == 0x09 and ow_route_goal and ow_route_path then
      local bx, by = s.x - s.x % 512, s.y - s.y % 512
      local function oplot(wt)
        return fx + (wt[1] * 8 + 4 - bx) * fw // 512, fy + (wt[2] * 8 + 4 - by) * fw // 512
      end
      for i = 1, #ow_route_path - 1 do
        local ax, ay = oplot(ow_route_path[i])
        local cx2, cy2 = oplot(ow_route_path[i + 1])
        canvas:line(ax, ay, cx2, cy2, 0xFF60D0)
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

    -- Link's marker at his sprite CENTRE, not the raw $0020/$0022 which is the
    -- 16x16 sprite's top-left corner — often up in a wall tile a row or two above
    -- where he visibly stands, so the raw point reads a tile off from the ground
    -- (and the bush/entrance) beneath his feet.
    local lx = fx + ((s.x + 8) % 512) * fw // 512
    local ly = fy + ((s.y + 8) % 512) * fw // 512
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
