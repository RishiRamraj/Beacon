-- Beacon reference data for The Legend of Zelda: A Link to the Past.
--
-- A data-only module. The host loads it (declared in the manifest's `modules`) into
-- the same Lua state BEFORE alttp.lua, so it is a separate chunk with its own
-- local-variable budget — the plugin's large reference tables live here rather than
-- eating the main script's budget (Lua caps locals at 200 per chunk). It hands its
-- tables to the script through the shared global `REF` namespace, which alttp.lua
-- reads. Pure data: no game logic, no host-API calls, no per-frame work.
--
-- As the plugin grows, move more reference data (tile classes, per-dungeon room
-- data, dialogue tables, …) here the same way: add `REF.<name> = { … }` and read it
-- from alttp.lua as `REF.<name>`.
REF = REF or {}

-- Sprite classification, keyed by the $7E0E20 sprite type. Names are from the ALttP
-- disassembly's sprite list. enemy/item/npc are the subsets the guide sorts sprites
-- into — an enemy reads as a threat, an item as a pickup, an npc as a person; a type
-- in none of them is inert scenery.
REF.sprite_names = { [0]="Raven", [1]="Vulture", [2]="Flying Stalfos Head", [4]="Good Switch", [5]="Switch", [6]="Bad Switch", [7]="Switch again, facing up", [8]="Octorock", [9]="Giant Moldorm", [10]="Four Shooter Octorock", [11]="Chicken", [12]="Octorock projectile", [13]="Buzzblob", [14]="Plants with big mouths", [15]="Octoballoon", [16]="Octospawn", [17]="Hinox", [18]="Moblin", [19]="Helmasaur", [20]="Gargoyle Grate", [21]="Bubble", [22]="Sahasrahla", [23]="Rupee Crab under bush", [24]="Moldorm", [25]="Poe", [26]="Dwarves and helper sprites", [27]="Arrow in Wall", [28]="Movable Statue", [29]="Weathervane", [30]="Crystal Switch", [31]="Bug Net Kid", [32]="Sluggula", [33]="Push Switch", [34]="Ropa", [35]="Bari (Blue)", [36]="Bari (Red)", [37]="Conversational Tree", [38]="Hardhat Beetle", [39]="Deadrock", [40]="Story Teller Set 1", [41]="Human NPC Set 1", [42]="Sweeping lady", [43]="Hobo under bridge", [44]="Lumberjack Bros", [45]="Telepathic Stones", [46]="Flute Boy's Notes", [47]="Race Game Couple", [48]="Person", [49]="Fortune Teller", [50]="Quarrel Bros", [51]="Pull For Rupees", [52]="Young Snitch Girl", [53]="Inn Keeper", [54]="Witch", [55]="Waterfall", [56]="Arrow Target", [57]="Middle-aged desert guy", [58]="Mad Batter", [59]="Dash item", [60]="Kid in village near trough", [61]="Old Snitch Lady", [62]="Rupee Crab under rock", [63]="Tutorial Soldier", [64]="Barrier", [65]="Green Soldier", [66]="Blue Soldier", [67]="Red Spear Soldier", [68]="Psycho Trooper", [69]="Psycho Spear Soldier", [70]="Blue Archer Soldier", [71]="Green Archer Bush Soldier", [72]="Red Javelin Trooper", [73]="Red Javelin Bush Soldier", [74]="Green Enemy Bombs", [75]="Green Soldier (weak version)", [76]="Gerudo Man", [77]="Toppo", [78]="Popo", [79]="Bot", [80]="Metal Ball", [81]="Armos", [82]="Zora King", [83]="Armos Knight", [84]="Lanmola", [85]="Zora and Fireball", [86]="Walking Zora", [87]="Desert Palace barriers", [88]="Crab", [89]="Lost Woods Bird", [90]="Lost Woods Squirrel", [91]="Spark (clockwise)", [92]="Spark (counter-clockwise)", [93]="Roller (down then up)", [94]="Roller (up then down)", [95]="Roller", [96]="Roller", [97]="Beamos", [98]="Master Sword", [99]="Debirando Pit", [100]="Debirando", [101]="Archery Game Guy", [102]="Wall Cannon", [103]="Wall Cannon", [104]="Wall Cannon", [105]="Wall Cannon", [106]="Ball And Chain Trooper", [107]="Cannon Trooper", [108]="Warp Vortex", [109]="Rat", [110]="Rope", [111]="Keese", [112]="Helmasaur King Fireball", [113]="Leever", [114]="Pond Activator", [115]="Link's Uncle", [116]="Red Hat Wussy", [117]="Bottle Vendor", [118]="Princess Zelda", [119]="Alternate Bubble", [120]="Elder's Wife", [121]="Good Bee stuck in Ice Cavern", [122]="Agahnim", [123]="Agahnim energy", [124]="Green Stalfos", [125]="Spike Trap", [126]="Guruguru Bar", [127]="Guruguru Bar", [128]="Wandering Fireball Chains", [129]="Hover", [130]="Bubble Group", [131]="Eyegore", [132]="Eyegore 2", [133]="Yellow Stalfos", [134]="Kodondo", [135]="Flames", [136]="Mothula", [137]="Mothula Beam", [138]="Spike Block", [139]="Gibdo", [140]="Arrghus", [141]="Arrgi", [142]="Chair Turtles (kill with hammer)", [143]="Terrorpin", [144]="Grabber Things", [145]="Stalfos Knight", [146]="Helmasaur King", [147]="Bumper", [148]="Pirogusu", [149]="Laser Eye (right)", [150]="Laser Eye (left)", [151]="Laser Eye (down)", [152]="Laser Eye (up)", [153]="Attack Penguin", [154]="Kyameron", [155]="Wizzrobe", [156]="Zoro", [157]="Babusu", [158]="Ostrich seen with Flute Boy", [159]="Rabbit seen with Flute Boy", [160]="Bird seen with Flute Boy", [161]="Freezor", [162]="Kholdstare", [163]="Kholdstare part 2", [164]="Kholdstare Ice balls", [165]="Blue Zazak", [166]="Red Zazak", [167]="Stalfos", [168]="Green Bomber", [169]="Blue Bomber", [170]="Pikit", [171]="Crystal Maiden", [172]="Apple(s) in tree", [173]="Old Mountain Man", [174]="Down Pipe", [175]="Up Pipe", [176]="Right Pipe", [177]="Left Pipe", [178]="Good Bee", [179]="Hylian Inscription", [180]="Thief Chest", [181]="Bomb Shop Guy and company", [183]="Blind disguised as a Maiden", [184]="Dialogue Testing Sprite", [185]="Bully and Ball Guy", [186]="Whirlpool", [187]="Shopkeeper", [188]="Drunk in the Inn", [189]="Vitreous", [190]="Smaller Vitreous Eyeballs", [191]="Vitreous Lightning Blast", [192]="Giant Cranky Catfish", [193]="Agahnim Teleporting Zelda", [194]="Boulder", [195]="Gibo", [196]="Thief", [197]="Evil Fireball Spitters", [198]="Fourway Fireball Spitters", [199]="Hokbok", [200]="Big Faerie", [201]="Ganon Helpers + Tektite", [202]="Chain Chomp", [203]="Agahnim", [204]="Trinexx Part 2", [205]="Trinexx Part 3", [206]="Blind", [207]="Swamola", [208]="Lynel", [209]="Yellow Transform", [210]="Flopping Fish", [211]="Stal", [212]="Landmine", [213]="Digging Game Guy", [214]="Ganon", [215]="InvinceoGanon", [216]="Heart Refill", [217]="Green Rupee", [218]="Blue Rupee", [219]="Red Rupee", [220]="1 Bomb Refill", [221]="4 Bomb Refill", [222]="8 Bomb Refill", [223]="Small Magic Refill", [224]="Full Magic Refill", [225]="5 Arrow Refill", [226]="10 Arrow Refill", [227]="Faerie", [228]="Key", [229]="Big Key", [230]="Shield Pickup", [231]="Mushroom", [232]="Fake Master Sword", [233]="Magic Shop Dude", [234]="Heart Container", [235]="Heart Piece", [236]="Bush", [237]="Cane of Somaria Platform", [238]="Movable Mantle", [239]="Cane of Somaria Platform", [240]="Cane of Somaria Platform", [241]="Cane of Somaria Platform", [242]="Medallion Tablet" }
REF.enemy_types = { [1]=true, [2]=true, [8]=true, [9]=true, [12]=true, [13]=true, [14]=true, [15]=true, [16]=true, [17]=true, [18]=true, [21]=true, [24]=true, [25]=true, [32]=true, [34]=true, [35]=true, [36]=true, [38]=true, [39]=true, [65]=true, [66]=true, [67]=true, [68]=true, [69]=true, [70]=true, [71]=true, [72]=true, [73]=true, [74]=true, [83]=true, [84]=true, [85]=true, [86]=true, [88]=true, [99]=true, [100]=true, [104]=true, [105]=true, [106]=true, [107]=true, [109]=true, [111]=true, [131]=true, [132]=true, [133]=true, [134]=true, [136]=true, [139]=true, [142]=true, [143]=true, [144]=true, [145]=true, [146]=true, [153]=true, [154]=true, [155]=true, [162]=true, [165]=true, [167]=true, [169]=true, [170]=true, [185]=true, [203]=true, [206]=true, [211]=true, [214]=true, [215]=true }
REF.item_types = { [98]=true, [178]=true, [216]=true, [217]=true, [218]=true, [219]=true, [220]=true, [221]=true, [222]=true, [223]=true, [224]=true, [225]=true, [226]=true, [227]=true, [228]=true, [229]=true, [230]=true, [231]=true, [234]=true, [235]=true }
REF.npc_types = { [22]=true, [30]=true, [31]=true, [33]=true, [47]=true, [49]=true, [53]=true, [54]=true, [60]=true, [76]=true, [82]=true, [115]=true, [117]=true, [118]=true, [120]=true, [171]=true, [173]=true, [187]=true, [233]=true }

-- The name-entry picker grid (zelda3 select_file.c kNamePlayer_Tab3): 128 cells, 32 per
-- row, indexed by the live cursor (selectfile_var3 + selectfile_var5 * 0x20). Each cell
-- holds a glyph code; the game stores a picked one as (t & 0xFFF0) * 2 + (t & 0xF).
REF.name_grid = {
  0x06, 0x07, 0x5F, 0x09, 0x59, 0x59, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x60, 0x23,
  0x59, 0x59, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x59, 0x59, 0x59, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05,
  0x10, 0x11, 0x12, 0x13, 0x59, 0x59, 0x24, 0x5F, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D,
  0x59, 0x59, 0x7B, 0x7C, 0x7D, 0x7E, 0x7F, 0x59, 0x59, 0x59, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
  0x40, 0x41, 0x42, 0x59, 0x59, 0x59, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x40, 0x41, 0x42, 0x59,
  0x59, 0x59, 0x61, 0x3F, 0x45, 0x46, 0x59, 0x59, 0x59, 0x59, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
  0x44, 0x59, 0x6F, 0x6F, 0x59, 0x59, 0x59, 0x59, 0x59, 0x59, 0x59, 0x5A, 0x44, 0x59, 0x6F, 0x6F,
  0x59, 0x59, 0x5A, 0x44, 0x59, 0x6F, 0x6F, 0x59, 0x59, 0x59, 0x59, 0x59, 0x59, 0x59, 0x59, 0x5A,
}

-- What each cell of the name-entry picker says, keyed by grid index (32 per row, four
-- rows), read out of the game by walking the grid: Rishi filled in every cell against
-- what was on screen, and the capitals came out of the arithmetic beforehand and matched.
--
-- Keyed by POSITION, not by glyph code, because one code is two characters: 0x5F draws
-- both capital I and lowercase l — the same glyph doing double duty, as SNES fonts often
-- do — so only where it sits says which is meant. Two other codes appear twice (0x40-0x42,
-- the punctuation block) but agree with themselves, and the lowercase run a-h then this
-- cell then j is what fixes grid index 14 as lowercase i.
REF.name_cells = {
  "G", "H", "I", "J", "space", "space", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "space", "space", "0", "1", "2", "3", "4", "space", "space", "space", "A", "B", "C", "D", "E", "F",
  "Q", "R", "S", "T", "space", "space", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "space", "space", "5", "6", "7", "8", "9", "space", "space", "space", "K", "L", "M", "N", "O", "P",
  "-", ".", ",", "space", "space", "space", "u", "v", "w", "x", "y", "z", "-", ".", ",", "space", "space", "space", "!", "?", "(", ")", "space", "space", "space", "space", "U", "V", "W", "X", "Y", "Z",
  "forward", "space", "end", "end", "space", "space", "space", "space", "space", "space", "space", "back", "forward", "space", "end", "end", "space", "space", "back", "forward", "space", "end", "end", "space", "space", "space", "space", "space", "space", "space", "space", "back",
}

-- Character -> how to SAY it, for the cells a speech engine renders as silence.
--
-- Punctuation is markup to a synthesiser, not a word: handed "." or "-" it pauses or says
-- nothing at all, so those cells were mute while every letter spoke. Naming them is the
-- only way the cursor being on one is audible. Letters, digits and the three controls need
-- no entry — they already speak as themselves.
REF.name_spoken = {
  ["-"] = "dash",
  ["."] = "period",
  [","] = "comma",
  ["!"] = "exclamation mark",
  ["?"] = "question mark",
  ["("] = "open bracket",
  [")"] = "close bracket",
}

-- STORED code -> character, for reading a name back out of a save.
--
-- Keyed by what the save actually holds, which is not the grid's glyph code: picking a
-- cell stores (t & 0xFFF0) * 2 + (t & 0xF), so the grid's 0x00-0x0F pass through unchanged
-- while everything above them moves. That is why a first attempt keyed on grid codes read
-- "LINK" back as "LNK" — L, N and K are all in the identity range and I is not, and Q-Z
-- would have gone the same way. The blank confirms the formula: 0x59 stores as 0xA9, which
-- is what an empty save slot's six characters read as.
--
-- 0x5F stores as 0xAF and draws both capital I and lowercase l, so a saved name cannot
-- distinguish them and this gives capital I; the game has lost the difference too. The
-- three controls are left out, being things to press rather than characters.
REF.name_chars = { [0x00] = "A", [0x01] = "B", [0x02] = "C", [0x03] = "D", [0x04] = "E", [0x05] = "F", [0x06] = "G", [0x07] = "H", [0x09] = "J", [0x0A] = "K", [0x0B] = "L", [0x0C] = "M", [0x0D] = "N", [0x0E] = "O", [0x0F] = "P", [0x20] = "Q", [0x21] = "R", [0x22] = "S", [0x23] = "T", [0x24] = "U", [0x25] = "V", [0x26] = "W", [0x27] = "X", [0x28] = "Y", [0x29] = "Z", [0x2A] = "a", [0x2B] = "b", [0x2C] = "c", [0x2D] = "d", [0x2E] = "e", [0x2F] = "f", [0x40] = "g", [0x41] = "h", [0x43] = "j", [0x44] = "k", [0x46] = "m", [0x47] = "n", [0x48] = "o", [0x49] = "p", [0x4A] = "q", [0x4B] = "r", [0x4C] = "s", [0x4D] = "t", [0x4E] = "u", [0x4F] = "v", [0x60] = "w", [0x61] = "x", [0x62] = "y", [0x63] = "z", [0x6F] = "?", [0x80] = "-", [0x81] = ".", [0x82] = ",", [0x85] = "(", [0x86] = ")", [0xA9] = " ", [0xAF] = "I", [0xC0] = "i", [0xC1] = "!", [0xE6] = "0", [0xE7] = "1", [0xE8] = "2", [0xE9] = "3", [0xEA] = "4", [0xEB] = "5", [0xEC] = "6", [0xED] = "7", [0xEE] = "8", [0xEF] = "9" }
