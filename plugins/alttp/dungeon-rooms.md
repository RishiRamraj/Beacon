# ALttP per-dungeon room reference

Room ids and progress bits for each dungeon's canonical spine — signature item →
Big Key → boss. This was the data behind the old `DUNGEON_NAV` table + `advance_dungeon`
spine, which has been **retired** in favour of authored waypoint chains (see
[ADR 0022](../../docs/decisions/0022-navigation-and-goal-engine.md)). It is kept here
as the reference to build each dungeon's chain against as we play through them.

Keyed by `$7E040C` dungeon id. Room ids are the `$7E00A0` value; boss/entrance/chest
room ids were confirmed against the randomizer chest table and the disassembly's
underworld-room list. Small keys, map and compass were deliberately not tracked —
not what a player is steered toward, and pot/enemy-drop keys have no stable room id.

`have` is the inventory read that means the signature item is in hand. `bk` is the
Big Key bit in the `$7EF366`/`$7EF367` big-key bitfields.

| id | Dungeon | Item | `have` read | item room | Big Key (byte & bit → room) | boss room | entrance room |
|---|---|---|---|---|---|---|---|
| 0x04 | Eastern Palace | the Bow | `$7EF340 ≥ 1` | 0xA9 | `$7EF367 & 0x20` → 0xB8 | 0xC8 | 0xC9 |
| 0x06 | Desert Palace | the Power Glove | `$7EF354 ≥ 1` | 0x73 | `$7EF367 & 0x10` → 0x75 | 0x33 | 0x84 |
| 0x14 | Tower of Hera | the Moon Pearl | `$7EF357 ≥ 1` | 0x27 | `$7EF366 & 0x20` → 0x87 | 0x07 | 0x77 |
| 0x0C | Palace of Darkness | the Magic Hammer | `$7EF34B ≥ 1` | 0x1A | `$7EF367 & 0x02` → 0x3A | 0x5A | 0x4A |
| 0x0A | Swamp Palace | the Hookshot | `$7EF342 ≥ 1` | 0x36 | `$7EF367 & 0x04` → 0x35 | 0x06 | 0x28 |
| 0x10 | Skull Woods | the Fire Rod | `$7EF345 ≥ 1` | 0x58 | `$7EF366 & 0x80` → 0x57 | 0x29 | — (3 overworld entrances) |
| 0x16 | Thieves' Town | the Titan's Mitt | `$7EF354 ≥ 2` | 0x44 | `$7EF366 & 0x10` → 0xDB | 0xAC | 0xDB |
| 0x12 | Ice Palace | the Blue Mail | `$7EF35B ≥ 1` | 0x9E | `$7EF366 & 0x40` → 0x1F | 0xDE | 0x0E |
| 0x0E | Misery Mire | the Cane of Somaria | `$7EF350 ≥ 1` | 0xC3 | `$7EF367 & 0x01` → 0xD1 | 0x90 | 0x98 |
| 0x18 | Turtle Rock | the Mirror Shield | `$7EF35A ≥ 3` | 0x24 | `$7EF366 & 0x08` → 0x14 | 0xA4 | 0xD6 |

`room_boss_beaten(room)` = the dungeon room-data word for `room` (`$7EF000 + room*2`)
has bit `0x0800` set.

The overworld **entrance area** each dungeon goal sends you to (from the researched
entrance table) lives on the `GOALS` records in `alttp.lua` as each dungeon goal's
`area` field.
