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
-- The ring the nearest enemy was last in, so proximity speaks on approach rather
-- than every frame. nil when no enemy is near.
local nearest_enemy_ring = nil

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

-- Reads the active sprites, nearest first, each as a table of absolute position,
-- offset from Link, Manhattan distance, type, and health.
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

-- A compass direction from an offset. y decreases upward on the SNES.
local function direction(dx, dy)
  local ax, ay = math.abs(dx), math.abs(dy)
  local ns = dy < 0 and "north" or "south"
  local ew = dx < 0 and "west" or "east"
  if ax > 2 * ay then return ew
  elseif ay > 2 * ax then return ns
  else return ns .. ew end
end

-- A rough distance word. Roughly 16 pixels to a tile.
local function proximity(dist)
  if dist < 24 then return "right beside you"
  elseif dist < 64 then return "close"
  elseif dist < 160 then return "nearby"
  else return "in the distance" end
end

-- The proximity ring an enemy is in, smaller being nearer, or nil past "nearby".
-- Used to speak only when an enemy crosses into a closer ring.
local function enemy_ring(dist)
  if dist < 24 then return 0
  elseif dist < 64 then return 1
  elseif dist < 160 then return 2
  else return nil end
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

  -- Automatic enemy proximity. Hysteresis by ring, so it speaks when the nearest
  -- enemy crosses into a closer ring, not every frame; it resets once none is
  -- near, so a fresh approach speaks again. The thresholds are a starting point
  -- for players to tune, and the arbiter rate-limits the "proximity" category
  -- regardless. Interaction priority, so a low-verbosity player can silence it.
  if in_play(now) then
    local nearest = nil
    for _, sp in ipairs(sprites()) do
      if sp.hp ~= nil and sp.hp > 0 then
        nearest = sp
        break
      end
    end
    local ring = nearest and enemy_ring(nearest.dist) or nil
    if ring ~= nil and (nearest_enemy_ring == nil or ring < nearest_enemy_ring) then
      say(
        string.format("Enemy %s, %s.", direction(nearest.dx, nearest.dy), proximity(nearest.dist)),
        { priority = "interaction", category = "proximity", rate_limit = "1500ms" }
      )
    end
    nearest_enemy_ring = ring
  end
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

-- "Scan" — describe the objects and enemies around Link, nearest first. This is
-- the standard scan command; the host binds it to a key (c by default).
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

  say(
    string.format("%d nearby.", #list),
    { priority = "navigation", category = "on-demand" }
  )
  -- Describe up to the three nearest, so a busy room does not become a monologue.
  for i = 1, math.min(3, #list) do
    local sp = list[i]
    local kind = (sp.hp ~= nil and sp.hp > 0) and "enemy" or "object"
    say(
      string.format("%s, %s, %s.", kind, direction(sp.dx, sp.dy), proximity(sp.dist)),
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

-- Map mode: a schematic of what the plugin reads, for debugging and for sighted
-- assistance. Not the game's own map — a picture of Link's state as this plugin
-- understands it. Integer math throughout (// is floor division) so coordinates
-- stay whole for the canvas.
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
    -- Sprites first, so Link's marker sits on top of them. Enemies (those with
    -- health) in red, other objects in cyan.
    for _, sp in ipairs(sprites()) do
      local px = fx + (sp.x % 512) * fw // 512
      local py = fy + (sp.y % 512) * fw // 512
      local col = (sp.hp ~= nil and sp.hp > 0) and 0xF04040 or 0x40C0F0
      canvas:rect(px - 1, py - 1, 3, 3, col)
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
