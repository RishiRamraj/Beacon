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
-- `tremolo` is the amplitude-pulse rate in Hz: a rhythmic signature that tells
-- the classes apart by ear even when they overlap. The rate rises with danger —
-- scenery and pickups are calm or steady, enemies throb fast (and faster still
-- the tougher they are, see the emission loop). The guide tone stays steady (no
-- tremolo) so the thing you actively steer by is never mistaken for a threat.
local BEACON_KINDS = {
  enemy = { pitch = 1.0, range = 224, tremolo = 6.0 }, -- base; scaled by HP below
  item  = { pitch = 2.0, range = 224, tremolo = 2.0 }, -- a calm "come and get me"
  npc   = { pitch = 1.5, range = 224, tremolo = 3.5 }, -- gently active, but safe
  minor = { pitch = 0.5, range = 24,  tremolo = 0.0 }, -- steady, incidental
}

-- Enemy pulse scales with toughness: the base rate plus a term in the sprite's
-- health, so a stronger foe throbs faster and more urgently. Health is capped
-- before scaling so a boss's huge HP does not run the rate off into a buzz.
local ENEMY_TREMOLO_BASE = 6.0    -- Hz, a plain enemy with little or no health
local ENEMY_TREMOLO_PER_HP = 0.06 -- added Hz per point of (capped) health
local ENEMY_TREMOLO_HP_CAP = 160  -- health past this does not pulse any faster

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

-- The nearest on-screen item pickup (a sprite in ITEM_TYPES), as world pixel
-- coordinates, or nil. sprites() is sorted nearest-first, so the first match is
-- the closest. Used by the dungeon guide to fetch a loose item in the room.
local function nearest_item_sprite(s)
  for _, sp in ipairs(sprites()) do
    if ITEM_TYPES[sp.kind] then return { sp.x, sp.y } end
  end
  return nil
end

-- ===========================================================================
-- Cross-room dungeon routing: a room-to-room guide layered over the local
-- pathfinder, which only reaches within the current room. Rather than decode the
-- ROM's door tables, the plugin *learns* a dungeon's connectivity as Link walks
-- it: each room transition records a directed edge and the spot in the room he
-- left from. A breadth-first search over that learned graph gives the next room
-- to head for, and the local pathfinder is aimed at the door that leaves toward
-- it, re-aimed at every room boundary. Rooms not yet walked simply are not in the
-- graph; guidance falls back to a compass heading until exploration connects them.
-- ===========================================================================

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

-- Breadth-first search over learned edges: the ordered list of rooms after
-- `from`, ending at `to`, or nil if the graph does not yet connect them.
local function room_path(from, to)
  if from == to then return {} end
  local prev, queue, head = { [from] = false }, { from }, 1
  while head <= #queue do
    local r = queue[head]; head = head + 1
    for nr in pairs(room_graph[r] or {}) do
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
  local exit = hop and room_graph[s.dungeon_room] and room_graph[s.dungeon_room][hop]
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
        -- source closes, rather than a flat linear fade.
        local t = 1 - sp.dist / kind.range
        local vol = t * t
        -- Behind a wall: muffled rather than silent, so a close but occluded
        -- source still registers without sounding like it is out in the open.
        if sight_blocked(now, now.x, now.y, sp.x, sp.y) then
          vol = vol * BEACON_OCCLUDED_SCALE
        end
        -- Enemies pulse faster the tougher they are; other classes use the
        -- class's fixed rate.
        local trem = kind.tremolo
        if name == "enemy" then
          local hp = sp.hp or 0
          if hp > ENEMY_TREMOLO_HP_CAP then hp = ENEMY_TREMOLO_HP_CAP end
          trem = ENEMY_TREMOLO_BASE + hp * ENEMY_TREMOLO_PER_HP
        end
        beacon.set(name, { x = sp.dx, y = sp.dy, pitch = kind.pitch, volume = vol, tremolo = trem })
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
  room_route_update(now)

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

-- Route toward a target room, given a spoken label for what is there. Already in
-- the room: guide to a loose item if visible, else a door, and say it is here.
-- Elsewhere: start a cross-room route. If the learned graph already connects the
-- rooms, aim precisely at the door that leaves toward the target and follow the
-- chain room by room; if not, set a rough heading (dungeon rooms are a 16-wide
-- grid, id low nibble = column, high nibble = row) and let the route lock on as
-- exploration fills the graph in. Cross-floor headings are approximate, so the
-- fallback wording stays soft.
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
  local exit = hop and room_graph[s.dungeon_room] and room_graph[s.dungeon_room][hop]
  if exit then
    route_set_goal(s, exit[1], exit[2])
    nav_say(label .. " Following the route.")
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
  local dir, other_world = area_heading(s.ow_screen, area)
  local world_note = ""
  if other_world then
    world_note = (area & 0x40) ~= 0 and " It's in the Dark World." or " It's in the Light World."
  end
  if dir == nil then
    nav_say(string.format("You're in the right area for %s. Find the entrance.%s", m.goal, world_note))
  else
    nav_say(string.format("Head %s toward %s.%s", dir, m.goal, world_note))
  end
end

on_command("advance", function()
  local s = prev
  if s == nil or not in_play(s) then
    nav_say("Not in play.")
    return
  end
  local v = read_progress()
  if s.module == 0x07 then
    advance_dungeon(s, v)
  else
    advance_overworld(s, v)
  end
end)

on_command("pathfind_stop", function()
  room_route_stop()
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
