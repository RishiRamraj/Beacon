//! Integration tests for the built-in A Link to the Past reference plugin.
//!
//! These drive the real `plugins/alttp/alttp.lua` (through `Registry::builtin`)
//! with synthetic RAM and assert its game-specific behaviour — sprite tables,
//! quest progress, dungeon routing, and so on. They live here, as an integration
//! test against the crate's public API, rather than inside the generic
//! `beacon-plugin` runtime crate's own unit tests, so that crate carries no
//! A-Link-to-the-Past specifics.

use beacon_plugin::{wram_offset, BeaconState, Intent, LuaPlugin, Plugin, Registry};

#[test]
fn alttp_scan_describes_a_nearby_sprite() {
    // Drives the real built-in alttp plugin with synthetic sprite RAM, so the
    // scan logic (sprite table, direction, distance) is exercised as shipped.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x09); // module: overworld
    set(0x7E0011, 0x00); // submodule 0: in play
    set(0x7EF36C, 24); // max health
    set(0x7EF36D, 24); // health
    set(0x7E0022, 0x00);
    set(0x7E0023, 0x01); // Link X = 0x0100
    set(0x7E0020, 0x00);
    set(0x7E0021, 0x01); // Link Y = 0x0100
                         // One active sprite, 0x40 pixels east of Link, no health -> "object".
    set(0x7E0DD0, 0x09); // slot 0 state: active
    set(0x7E0E20, 3); // kind 3: an unnamed sprite id, so it reads as "object"
    set(0x7E0D10, 0x40);
    set(0x7E0D30, 0x01); // sprite X = 0x0140
    set(0x7E0D00, 0x00);
    set(0x7E0D20, 0x01); // sprite Y = 0x0100
    set(0x7E0E50, 0x00); // no health

    // First frame primes `prev`; the second gives scan a state to read.
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    let out = plugin.command("scan", &ram);

    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("1 nearby")), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|t| t.contains("object") && t.contains("east")),
        "{texts:?}"
    );
}

#[test]
fn alttp_enemy_is_tracked_by_beacon_and_never_spoken() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // An in-play frame with a Green Soldier (type 65) `dx` pixels east of Link.
    // On screen is |dx| <= 128; dx 200 is off screen, dx 60 is on.
    let frame = |dx: u16| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09);
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
        let ex = 0x0100u16 + dx;
        set(0x7E0DD0, 0x09); // slot 0 active
        set(0x7E0E20, 65); // type: Green Soldier
        set(0x7E0D10, (ex & 0xFF) as u8);
        set(0x7E0D30, (ex >> 8) as u8);
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01); // enemy Y = 0x0100
        ram
    };
    let soldier = |out: &[Intent]| {
        out.iter()
            .any(|i| i.text.contains("Green Soldier") || i.text.contains("enemy"))
    };

    plugin.on_frame(&frame(200), 0); // prime prev; enemy off screen
                                     // Enemies are never announced by name — the spatial beacon alone tracks them.
    assert!(
        !soldier(&plugin.on_frame(&frame(60), 1)),
        "the enemy is not spoken as it enters the screen"
    );
    // The nearest enemy still gets a spatial-audio beacon, panned toward it,
    // louder the nearer it is.
    let b = plugin.beacons();
    let enemy = b
        .iter()
        .find(|b| b.id == "enemy")
        .expect("a beacon on the enemy");
    assert!(enemy.dx > 0.0, "panned east");
    assert!(
        enemy.volume > 0.0 && enemy.volume <= 1.0,
        "audible volume, got {}",
        enemy.volume
    );
}

#[test]
fn alttp_ball_and_chain_flail_sounds_a_weapon_beacon_while_swinging() {
    // The Ball-and-Chain Trooper (type 0x6A) swings a ball on a chain that is drawn
    // as OAM, not a sprite slot. Mid-swing (ai_state >= 2) the plugin recomputes the
    // ball's position from the trooper's swing aux fields and places a distinct
    // "weapon" beacon on it, separate from the enemy-body beacon. Tucked in, no tone.
    let r = Registry::builtin();

    let frame = |ai_state: u8| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // in play
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
        set(0x7E0DD0, 0x09); // slot 0 active
        set(0x7E0E20, 0x6A); // Ball and Chain Trooper
        set(0x7E0D10, 0x30);
        set(0x7E0D30, 0x01); // sprite X = 0x0130 (48px east of Link)
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01); // sprite Y = 0x0100
        set(0x7E0E50, 16); // hp -> a live enemy
        set(0x7E0D80, ai_state); // sprite_ai_state (>= 2 = swinging)
        set(0x7E0D90, 0x40); // sprite_A: swing angle low byte
        set(0x7E0DA0, 0x00); // sprite_B: swing angle high bit
        set(0x7E0DE0, 1); // sprite_D: facing
        set(0x7E0E10, 8); // sprite_delay_aux2: mid-extension radius
        ram
    };

    // Mid-swing: a weapon beacon tracks the ball, alongside the enemy-body beacon.
    let swinging = frame(2);
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&swinging, 0);
    plugin.on_frame(&swinging, 1);
    let b = plugin.beacons();
    assert!(
        b.iter().any(|x| x.id == "weapon"),
        "the swinging flail sounds its own weapon beacon: {:?}",
        b.iter().map(|x| &x.id).collect::<Vec<_>>()
    );
    assert!(
        b.iter().any(|x| x.id == "enemy"),
        "the trooper body still sounds the enemy beacon"
    );

    // Flail tucked in (ai_state 0): no weapon beacon, only the enemy body.
    let idle = frame(0);
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin2.on_frame(&idle, 0);
    plugin2.on_frame(&idle, 1);
    assert!(
        !plugin2.beacons().iter().any(|x| x.id == "weapon"),
        "a tucked-in flail sounds no weapon beacon"
    );
}

#[test]
fn alttp_zelda_is_not_chirped_as_an_ambient_npc() {
    // Princess Zelda (type 118) is the rescue objective the guide leads to, so she is
    // kept off the ambient "npc" beacon — two cues on one target is confusing. A plain
    // NPC (Sahasrahla, type 22) still gets the tone.
    let r = Registry::builtin();

    let frame = |kind: u8| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09);
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
        set(0x7E0DD0, 0x09); // slot 0 active
        set(0x7E0E20, kind); // NPC type
        set(0x7E0D10, 0x30);
        set(0x7E0D30, 0x01); // sprite X = 0x0130 (near)
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01); // sprite Y = 0x0100
        ram
    };

    // A plain NPC gets the npc beacon (control).
    let plain = frame(22); // Sahasrahla
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&plain, 0);
    plugin.on_frame(&plain, 1);
    assert!(
        plugin.beacons().iter().any(|b| b.id == "npc"),
        "an ordinary NPC still sounds the npc beacon"
    );

    // Zelda does not — the guide leads to her instead.
    let zelda = frame(118);
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin2.on_frame(&zelda, 0);
    plugin2.on_frame(&zelda, 1);
    assert!(
        !plugin2.beacons().iter().any(|b| b.id == "npc"),
        "Zelda the objective is not chirped as an ambient npc"
    );
}

#[test]
fn alttp_a_carried_sprite_overhead_is_not_beaconed() {
    // Sprite state 0x0A is "carried" — an object Link is holding over his head (a
    // lifted pot/bush/rock). It rides on Link, so it is not a world object to track:
    // no beacon (nor scan/map entry). On the ground (state 0x09) it sounds normally.
    let r = Registry::builtin();

    let frame = |state: u8| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09);
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
        set(0x7E0DD0, state); // slot 0 state
        set(0x7E0E20, 3); // an unnamed minor sprite
        set(0x7E0D10, 0x08);
        set(0x7E0D30, 0x01); // sprite X = 0x0108 (8px east, within minor range)
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01); // sprite Y = 0x0100
        ram
    };

    // On the ground: a minor beacon sounds (control).
    let ground = frame(0x09);
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&ground, 0);
    plugin.on_frame(&ground, 1);
    assert!(
        plugin.beacons().iter().any(|b| b.id == "minor"),
        "a minor sprite on the ground sounds its beacon"
    );

    // Carried overhead: no beacon.
    let carried = frame(0x0A);
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin2.on_frame(&carried, 0);
    plugin2.on_frame(&carried, 1);
    assert!(
        !plugin2.beacons().iter().any(|b| b.id == "minor"),
        "a carried sprite overhead is not beaconed"
    );
}

#[test]
fn alttp_detects_a_damageable_sprite_the_type_table_does_not_name() {
    // A sprite whose type is not in REF.enemy_types (75) but which has health is
    // still a threat: detected via health and given an enemy beacon.
    // This is the case the type-only classification missed.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09);
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
        let ex = 0x0100u16 + 60;
        set(0x7E0DD0, 0x09); // active
        set(0x7E0E20, 75); // a type not in REF.enemy_types
        set(0x7E0D10, (ex & 0xFF) as u8);
        set(0x7E0D30, (ex >> 8) as u8);
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01);
        set(0x7E0E50, 4); // has health -> a threat
    }
    plugin.on_frame(&ram, 0); // prime prev (enemy already present)
    plugin.on_frame(&ram, 1);
    let b = plugin.beacons();
    assert!(
        b.iter().any(|b| b.id == "enemy"),
        "a beacon is placed on it"
    );
}

// A frame with a single sprite of `kind` (no health) `dx` pixels east of Link,
// in play. Shared by the category tests below.
fn frame_with_sprite(kind: u8, dx: u16) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x09);
    set(0x7E0011, 0x00);
    set(0x7EF36C, 24);
    set(0x7EF36D, 24);
    set(0x7E0022, 0x00);
    set(0x7E0023, 0x01); // Link X = 0x0100
    set(0x7E0020, 0x00);
    set(0x7E0021, 0x01); // Link Y = 0x0100
    let ex = 0x0100u16 + dx;
    set(0x7E0DD0, 0x09); // slot 0 active
    set(0x7E0E20, kind);
    set(0x7E0D10, (ex & 0xFF) as u8);
    set(0x7E0D30, (ex >> 8) as u8);
    set(0x7E0D00, 0x00);
    set(0x7E0D20, 0x01); // Y = 0x0100
    ram
}

#[test]
fn alttp_an_item_gets_its_own_tone_and_carries_across_the_screen() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // A Heart (type 216) 90 pixels east — an item, not an enemy.
    let ram = frame_with_sprite(216, 90);
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);

    let b = plugin.beacons();
    let item = b.iter().find(|b| b.id == "item").expect("an item beacon");
    assert_eq!(item.pitch, 2.0, "items sound at their own pitch");
    assert!(item.dx > 0.0, "panned east toward the item");
    assert!(
        !b.iter().any(|b| b.id == "enemy"),
        "an item is not an enemy tone"
    );
}

#[test]
fn alttp_scenery_only_sounds_within_a_block() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // A Weathervane (type 42) — non-interactable scenery, so a "minor" tone
    // with a one-block reach. On screen but 60 px off (beyond a block): silent.
    let far = frame_with_sprite(42, 60);
    plugin.on_frame(&far, 0);
    plugin.on_frame(&far, 1);
    assert!(
        !plugin.beacons().iter().any(|b| b.id == "minor"),
        "scenery a block away stays silent"
    );

    // Right beside Link (10 px, within a block): now it chirps, low.
    let near = frame_with_sprite(42, 10);
    plugin.on_frame(&near, 2);
    plugin.on_frame(&near, 3);
    let minor = plugin
        .beacons()
        .into_iter()
        .find(|b| b.id == "minor")
        .expect("scenery within a block sounds");
    assert_eq!(minor.pitch, 0.5, "scenery sounds at its own low pitch");
}

#[test]
fn alttp_a_wall_muffles_an_enemys_beacon() {
    let r = Registry::builtin();

    // A dungeon frame: Green Soldier (type 65) 60 px east of Link on the same
    // row. `wall` drops a wall tile (attr 0x01) into the dungeon collision grid
    // between them, on the straight line Link->enemy.
    let frame = |wall: bool| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x07); // dungeon module (uses the $7F2000 tile grid)
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100 -> tile 32
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100 -> tile 32
        let ex = 0x0100u16 + 60; // enemy X tile 39, same row
        set(0x7E0DD0, 0x09);
        set(0x7E0E20, 65); // Green Soldier
        set(0x7E0D10, (ex & 0xFF) as u8);
        set(0x7E0D30, (ex >> 8) as u8);
        set(0x7E0D00, 0x00);
        set(0x7E0D20, 0x01); // enemy Y = 0x0100
        if wall {
            // Wall tile at (tx=35, ty=32), between Link and the enemy.
            set(0x7F2000 + 32 * 64 + 35, 0x01);
        }
        ram
    };
    // Clear line of sight: beaconed at full strength.
    let mut open = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    open.on_frame(&frame(false), 0);
    open.on_frame(&frame(false), 1);
    let open_vol = open
        .beacons()
        .iter()
        .find(|b| b.id == "enemy")
        .expect("a beacon with a clear line")
        .volume;

    // Wall between: the beacon is muffled — present but much quieter — rather
    // than silenced.
    let mut walled = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    walled.on_frame(&frame(true), 0);
    walled.on_frame(&frame(true), 1);
    let hidden = walled
        .beacons()
        .into_iter()
        .find(|b| b.id == "enemy")
        .expect("occluded beacon is muffled, not removed");
    assert!(
        hidden.volume < open_vol * 0.5,
        "occluded beacon is muffled: {} vs open {}",
        hidden.volume,
        open_vol
    );
}

#[test]
fn alttp_an_unopened_chest_sounds_like_an_item_until_it_is_opened() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // A dungeon frame with a chest tile 56 px east of Link and no sprites.
    // `opened` swaps the chest tile-type (0x58) for plain floor (0x00), the way
    // the game rewrites the tile once the chest has been looted.
    let frame = |opened: bool| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x07); // dungeon module (uses the $7F2000 tile grid)
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100 -> tile 32
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100 -> tile 32
                             // Chest tile at (tx=39, ty=32), 7 tiles (56 px) east on the same row.
        set(0x7F2000 + 32 * 64 + 39, if opened { 0x00 } else { 0x58 });
        ram
    };

    // Unopened: a "chest" beacon, panned east, at the item pitch, audible.
    plugin.on_frame(&frame(false), 0);
    plugin.on_frame(&frame(false), 1);
    let b = plugin.beacons();
    let chest = b
        .iter()
        .find(|b| b.id == "chest")
        .expect("a beacon on the unopened chest");
    assert!(chest.dx > 0.0, "panned east toward the chest");
    assert!(
        chest.volume > 0.0 && chest.volume <= 1.0,
        "audible volume, got {}",
        chest.volume
    );
    assert_eq!(chest.pitch, 2.0, "sounds at the item pitch");

    // Opened: the tile-type changed, so the chest no longer matches and its
    // beacon goes quiet.
    plugin.on_frame(&frame(true), 2);
    plugin.on_frame(&frame(true), 3);
    assert!(
        !plugin.beacons().iter().any(|b| b.id == "chest"),
        "no chest beacon once it is opened"
    );
}

// A dungeon frame: Link at (link_tx, link_ty), a door tile at (door_tx,
// door_ty), and wall tiles, all in the $7F2000 collision grid. No sprites.
/// Which waypoint the guide is leading to, as "room,tx,ty" — its identity rather than
/// its index. Chains get steps inserted and removed; a test that hardcodes a number
/// breaks on every edit and says nothing about what went wrong, so these name the
/// waypoint instead.
const PICKED: &str = r#"
  local wp = nav_chain and nav_chain[nav_chain_i]
  if wp == nil then return "none" end
  return string.format("%02X,%s,%s", wp.room or 0, tostring(wp.tx), tostring(wp.ty))
"#;

fn dungeon_frame(link: (u16, u16), door: (u16, u16), walls: &[(u16, u16)]) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x07); // dungeon
        set(0x7E0011, 0x00);
        set(0x7E001B, 0x01); // indoors
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        let lx = link.0 * 8 + 4;
        let ly = link.1 * 8 + 4;
        // The camera, centred on Link as the game keeps it. Sprite_CheckIfScreenIsClear
        // measures from these, so a frame that leaves them at zero puts the 256-pixel
        // kill screen nowhere near the room and nothing counts.
        let (sx, sy) = (lx.saturating_sub(128), ly.saturating_sub(128));
        set(0x7E00E2, (sx & 0xFF) as u8);
        set(0x7E00E3, (sx >> 8) as u8);
        set(0x7E00E8, (sy & 0xFF) as u8);
        set(0x7E00E9, (sy >> 8) as u8);
        set(0x7E0022, (lx & 0xFF) as u8);
        set(0x7E0023, (lx >> 8) as u8);
        set(0x7E0020, (ly & 0xFF) as u8);
        set(0x7E0021, (ly >> 8) as u8);
        let tile = |set: &mut dyn FnMut(u32, u8), tx: u16, ty: u16, attr: u8| {
            set(0x7F2000 + (ty as u32 & 63) * 64 + (tx as u32 & 63), attr);
        };
        tile(&mut set, door.0, door.1, 0x30); // a door tile
        for &(wx, wy) in walls {
            tile(&mut set, wx, wy, 0x01); // wall
        }
    }
    ram
}

fn path_beacon(plugin: &LuaPlugin) -> Option<BeaconState> {
    plugin.beacons().into_iter().find(|b| b.id == "path")
}

#[test]
fn alttp_in_combat_the_guide_ducks_and_only_the_enemy_sounds() {
    // With an enemy within striking distance the navigation guide DUCKS (drops well
    // under the enemy tone but keeps guiding) and the pickup/person tones drop out,
    // so a fight is not cluttered by a nearby item and the threat reads clearly over
    // the guide — without losing the route entirely.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // A room with a door to route to, and an item (Green Rupee, type 217) on screen
    // but no enemy: the guide sounds and the item beacon plays.
    let with_enemy = |enemy: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((10, 10), (20, 10), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        let (lx, ly) = (10u16 * 8 + 4, 10u16 * 8 + 4);
        // slot 1: an item, ~80px east (in beacon range, well outside combat range).
        set(0x7E0DD0 + 1, 0x09);
        set(0x7E0E20 + 1, 217); // Green Rupee (an ITEM_TYPE)
        let ix = lx + 80;
        set(0x7E0D10 + 1, (ix & 0xFF) as u8);
        set(0x7E0D30 + 1, (ix >> 8) as u8);
        set(0x7E0D00 + 1, (ly & 0xFF) as u8);
        set(0x7E0D20 + 1, (ly >> 8) as u8);
        if enemy {
            // slot 0: a Green Soldier 24px east — within COMBAT_RANGE.
            set(0x7E0DD0, 0x09);
            set(0x7E0E20, 65);
            let ex = lx + 24;
            set(0x7E0D10, (ex & 0xFF) as u8);
            set(0x7E0D30, (ex >> 8) as u8);
            set(0x7E0D00, (ly & 0xFF) as u8);
            set(0x7E0D20, (ly >> 8) as u8);
        }
        ram
    };

    // Clear of enemies: start the guide, and confirm guide + item both sound.
    let calm = with_enemy(false);
    plugin.on_frame(&calm, 0);
    plugin.command("pathfind", &calm);
    plugin.on_frame(&calm, 1);
    let calm_vol = path_beacon(&plugin)
        .expect("the guide sounds when clear")
        .volume;
    assert!(
        plugin.beacons().iter().any(|b| b.id == "item"),
        "the item sounds when clear"
    );

    // Enemy steps into striking range: the guide ducks (still present, quieter), the
    // item goes silent, and the enemy sounds.
    plugin.on_frame(&with_enemy(true), 2);
    let ids: Vec<String> = plugin.beacons().iter().map(|b| b.id.clone()).collect();
    let ducked = path_beacon(&plugin).expect("the guide keeps guiding, ducked, in combat");
    assert!(
        ducked.volume < calm_vol,
        "the guide ducks below its clear volume in combat: {} vs {calm_vol}",
        ducked.volume
    );
    assert!(
        !ids.contains(&"item".to_string()),
        "item hushes in combat: {ids:?}"
    );
    assert!(
        ids.contains(&"enemy".to_string()),
        "the enemy still sounds: {ids:?}"
    );
}

#[test]
fn alttp_pathfinder_routes_around_a_wall_to_a_door() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Door is due south of Link, but a wall spans the whole west side of row 13
    // (tx 0..11), so the only way through is the gap at tx>=12 to the east.
    let walls: Vec<(u16, u16)> = (0..=11).map(|x| (x, 13)).collect();
    let ram = dungeon_frame((10, 10), (10, 16), &walls);

    plugin.on_frame(&ram, 0); // prime
    plugin.command("pathfind", &ram); // plan a route to the nearest door
    plugin.on_frame(&ram, 1); // follower places the guide beacon

    let guide = path_beacon(&plugin).expect("a guide beacon toward the route");
    // A straight shot would point due south (dx≈0); routing around the wall
    // means the first corner is to the east.
    assert!(
        guide.dx > 0.0,
        "guide points east around the wall, not straight south (dx={}, dy={})",
        guide.dx,
        guide.dy
    );
}

#[test]
fn alttp_pathfinder_clears_the_guide_on_arrival() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Open room, door five tiles south of Link.
    let start = dungeon_frame((10, 10), (10, 15), &[]);
    plugin.on_frame(&start, 0);
    plugin.command("pathfind", &start);
    plugin.on_frame(&start, 1);
    let guide = path_beacon(&plugin).expect("guide beacon while en route");
    assert!(
        guide.dy > 0.0,
        "points south toward the door, dy={}",
        guide.dy
    );

    // Walk Link onto the door tile: the guide falls silent (no generic arrival
    // chatter) and the beacon clears.
    let at_door = dungeon_frame((10, 15), (10, 15), &[]);
    let out = plugin.on_frame(&at_door, 2);
    assert!(
        !out.iter().any(|i| i.text.contains("arrived")),
        "no generic arrival prompt: {:?}",
        out.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
    assert!(
        path_beacon(&plugin).is_none(),
        "guide beacon cleared on arrival"
    );
}

#[test]
fn alttp_scan_groups_and_counts_same_enemies() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09);
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01); // Link X = 0x0100
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01); // Link Y = 0x0100
                             // Two Green Soldiers (type 65), both east of Link.
        for (slot, dx) in [(0u32, 60u16), (1u32, 80u16)] {
            let ex = 0x0100u16 + dx;
            set(0x7E0DD0 + slot, 0x09);
            set(0x7E0E20 + slot, 65);
            set(0x7E0D10 + slot, (ex & 0xFF) as u8);
            set(0x7E0D30 + slot, (ex >> 8) as u8);
            set(0x7E0D00 + slot, 0x00);
            set(0x7E0D20 + slot, 0x01); // Y = 0x0100
        }
    }
    plugin.on_frame(&ram, 0); // scan reads `prev`, set by a frame first
    let out = plugin.command("scan", &ram);
    let texts: Vec<&String> = out.iter().map(|i| &i.text).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Two Green Soldiers") && t.contains("east")),
        "two of a kind are grouped and counted: {texts:?}"
    );
}

#[test]
fn alttp_marker_guides_back_to_where_it_was_dropped() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Drop a marker at (10,10), then walk east to (20,10) and ask to go back.
    let start = dungeon_frame((10, 10), (63, 63), &[]);
    plugin.on_frame(&start, 0);
    plugin.command("mark", &start);

    let moved = dungeon_frame((20, 10), (63, 63), &[]);
    plugin.on_frame(&moved, 1);
    plugin.command("guide_to_mark", &moved);
    plugin.on_frame(&moved, 2);

    let guide = path_beacon(&plugin).expect("a guide beacon back to the marker");
    assert!(
        guide.dx < 0.0,
        "guide points west, back toward the marker (dx={})",
        guide.dx
    );
}

#[test]
fn alttp_explore_routes_toward_unwalked_ground() {
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // An open room. After standing at (10,10) — which marks the tiles around
    // Link explored — "explore" should route to nearer unwalked ground.
    let room = dungeon_frame((10, 10), (63, 63), &[]);
    plugin.on_frame(&room, 0);
    plugin.on_frame(&room, 1); // marks the 3x3 around Link explored
    let out = plugin.command("explore", &room);
    plugin.on_frame(&room, 2);

    assert!(
        !out.iter().any(|i| i.text.contains("explored")),
        "there is still unexplored ground to route to"
    );
    assert!(
        path_beacon(&plugin).is_some(),
        "explore starts guiding toward unexplored ground"
    );
}

#[test]
fn alttp_advance_on_the_overworld_heads_toward_the_story_objective() {
    // On the overworld, "advance" starts a route toward the current goal's area when
    // it is in the same world, and flags the other world otherwise. Past the intro
    // (progress 2, Zelda delivered), the current quest goal is the first pendant
    // dungeon, in the same (Light) world, so it announces routing there.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld module
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, 0x18); // current area: Kakariko (row 3, col 0)
        set(0x7EF3C5, 2); // progress: intro over, so a dungeon goal is current
    }
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    let out = plugin.command("advance", &ram);
    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Routing")),
        "starts routing toward the objective: {texts:?}"
    );

    // Post-Agahnim: all three pendants, the Master Sword, Agahnim beaten, no
    // crystals yet. The next goal is Palace of Darkness (area 0x5E, Dark
    // World). The assist is already on, so as the objective changes the next frame
    // re-aims on its own and flags the other world — no second key press.
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF374, 0x07); // all three pendants
        set(0x7EF359, 2); // Master Sword
        set(0x7EF3C5, 3); // Agahnim beaten
        set(0x7E008A, 0x1B); // in the Light World castle area
    }
    let out = plugin.on_frame(&ram, 2);
    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Dark World")),
        "flags a Dark World destination: {texts:?}"
    );
}

#[test]
fn alttp_entering_the_dungeon_or_overworld_is_not_narrated() {
    // Crossing into the dungeon or overworld module should not speak
    // "dungeon" / "overworld" — the room/area callout already says where.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let frame = |module: u8, area: u8| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, module);
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, area);
        ram
    };

    // Booting into the intro (module 0x00) is not narrated either.
    plugin.on_frame(&frame(0x01, 0), 0); // file select (primes prev)
    let intro = plugin.on_frame(&frame(0x00, 0), 1);
    assert!(
        !intro.iter().any(|i| i.text.to_lowercase() == "intro"),
        "the intro is not narrated: {:?}",
        intro.iter().map(|i| &i.text).collect::<Vec<_>>()
    );

    // Prime in a menu, then cross into the overworld, then into a dungeon.
    plugin.on_frame(&frame(0x01, 0), 2); // file select
    let ow = plugin.on_frame(&frame(0x09, 0x1B), 3);
    assert!(
        !ow.iter().any(|i| i.text.to_lowercase() == "overworld"),
        "entering the overworld is not narrated: {:?}",
        ow.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
    let dg = plugin.on_frame(&frame(0x07, 0), 4);
    assert!(
        !dg.iter().any(|i| i.text.to_lowercase() == "dungeon"),
        "entering the dungeon is not narrated: {:?}",
        dg.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
}

#[test]
fn alttp_advance_inside_an_unchained_dungeon_stays_quiet() {
    // The old item -> Big Key -> boss room-graph spine is retired; a dungeon is
    // guided by an authored waypoint chain instead. Until a given dungeon has one,
    // the guide must NOT fall back to naming canonical items or routing a spine
    // (which fought the chain navigator) — inside the dungeon it stays quiet rather
    // than mis-route.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut room = dungeon_frame((10, 10), (20, 10), &[]);
    {
        let mut set = |addr: u32, v: u8| room[wram_offset(addr).unwrap()] = v;
        set(0x7E040C, 0x04); // Eastern Palace (no chain authored yet)
        set(0x7E00A0, 0x00);
        set(0x7EF340, 0x00); // Bow not held
        set(0x7EF3C5, 2); // intro done -> a post-intro dungeon goal is current
    }
    plugin.on_frame(&room, 0);
    plugin.on_frame(&room, 1);
    let out = plugin.command("advance", &room);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("Bow")
            || t.contains("Big Key")
            || t.to_lowercase().contains("boss")
            || t.to_lowercase().contains("exit")),
        "no retired spine chatter inside an un-chained dungeon: {texts:?}"
    );
    plugin.on_frame(&room, 2);
    assert!(
        path_beacon(&plugin).is_none(),
        "and it does not drive a route there"
    );
}

#[test]
fn alttp_objective_tracks_the_quest_from_the_progress_bytes() {
    // The strategic "objective" command reads the quest-progress save bytes
    // (progress $7EF3C5, pendants $7EF374, crystals $7EF37A, sword $7EF359)
    // and reports the current critical-path goal. A fresh save points at
    // the very first step; partway through the pendant hunt it advances to the
    // next unfinished dungeon.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Fresh save: every progress byte zero -> the scripted intro's first beat,
    // grabbing the Lamp, spoken as a "Getting started" step rather than a
    // bare goal (the intro chain refines the first two goals into fine steps).
    let fresh = vec![0u8; 128 * 1024];
    let out = plugin.command("objective", &fresh);
    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Getting started, step 1 of") && t.contains("Lamp")),
        "{texts:?}"
    );

    // Sanctuary reached (progress 2) and the Pendant of Courage taken from
    // Eastern Palace (pendants bit 0): the next objective is Desert Palace.
    let mut mid = vec![0u8; 128 * 1024];
    mid[wram_offset(0x7EF3C5).unwrap()] = 2; // progress: Zelda at Sanctuary
    mid[wram_offset(0x7EF374).unwrap()] = 0x01; // pendants: Courage
    let out = plugin.command("objective", &mid);
    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Desert Palace")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("Eastern Palace")),
        "the finished pendant dungeon is not re-suggested: {texts:?}"
    );
}

#[test]
fn alttp_intro_chain_walks_the_opening_beat_by_beat() {
    // The scripted intro refines the coarse "reach uncle" / "escort Zelda"
    // goals into fine beats, each unlocked by a save byte: the Lamp
    // ($7EF34A), the sword from Uncle ($7EF359), Zelda following ($7EF3CC == 1),
    // and Zelda delivered (progress $7EF3C5 >= 2). The "objective" readout should
    // advance through them in order, then hand off to the post-intro dungeon goals.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let objective = |plugin: &mut LuaPlugin, ram: &[u8]| -> String {
        plugin
            .command("objective", ram)
            .iter()
            .map(|i| i.text.clone())
            .collect::<Vec<_>>()
            .join(" ")
    };

    // Fresh save -> beat 1, the Lamp.
    let fresh = vec![0u8; 128 * 1024];
    let t = objective(&mut plugin, &fresh);
    assert!(t.contains("step 1 of") && t.contains("Lamp"), "{t}");

    // Lamp in hand -> beat 2, reaching Uncle for the sword.
    let mut lamp = vec![0u8; 128 * 1024];
    lamp[wram_offset(0x7EF34A).unwrap()] = 1; // Lamp
    let t = objective(&mut plugin, &lamp);
    assert!(
        t.contains("step 2 of") && t.to_lowercase().contains("uncle"),
        "{t}"
    );

    // Sword taken from Uncle -> beat 3, freeing Zelda.
    let mut sword = lamp.clone();
    sword[wram_offset(0x7EF359).unwrap()] = 1; // Fighter's Sword
    let t = objective(&mut plugin, &sword);
    assert!(t.contains("step 3 of") && t.contains("Zelda"), "{t}");

    // Zelda following (follower indicator == 1) -> beat 4, the Sanctuary.
    let mut following = sword.clone();
    following[wram_offset(0x7EF3CC).unwrap()] = 1; // Zelda tagalong
    let t = objective(&mut plugin, &following);
    assert!(t.contains("step 4 of") && t.contains("Sanctuary"), "{t}");

    // Zelda delivered (progress 2): the intro is over, so the current goal is the
    // first pendant dungeon.
    let mut delivered = vec![0u8; 128 * 1024];
    delivered[wram_offset(0x7EF3C5).unwrap()] = 2;
    let t = objective(&mut plugin, &delivered);
    assert!(
        t.contains("Objective") && t.contains("Eastern Palace"),
        "{t}"
    );
}

#[test]
fn alttp_intro_guide_auto_advances_when_a_beat_completes() {
    // Engaging the intro guide (advance) arms an auto-follow: once the Lamp is
    // picked up, the guide re-aims at the next beat on its own on the next frame,
    // with no further key press. In the starting house — an interior the dungeon
    // graph does not reach — that next beat sends Link to the door out.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // House interior: module 0x07, room 0x0104, with a chest tile and a door.
    let house = |lamp: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((32, 40), (32, 6), &[]); // door tile at (32,6)
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x04); // room 0x0104, low byte
        set(0x7E00A1, 0x01); // room 0x0104, high byte
        set(0x7EF34A, lamp); // Lamp ($7EF34A)
        set(0x7F2000 + 20 * 64 + 20, 0x58); // a chest tile
        ram
    };

    // Prime; nav auto-starts in the house on the next frame and cues the Lamp beat —
    // no key press (the player is up out of bed).
    plugin.on_frame(&house(0), 0);
    let out = plugin.on_frame(&house(0), 1);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.to_lowercase().contains("lantern")),
        "auto-starts on the Lamp beat, cueing the lantern: {texts:?}"
    );

    // Lamp now in hand: the next frame auto-advances to the leave-the-house beat,
    // no second key press.
    let out = plugin.on_frame(&house(1), 2);
    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.to_lowercase().contains("house")),
        "auto-advances to the next beat once the Lamp is taken: {texts:?}"
    );
}

#[test]
fn alttp_intro_chest_speaks_a_custom_arrival_cue() {
    // The Lamp beat routes to the chest with a per-waypoint arrival cue: reaching it
    // says "Open the chest." in place of the generic "You have arrived."
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let house = |link: (u16, u16)| -> Vec<u8> {
        let mut ram = dungeon_frame(link, (32, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x04); // room 0x0104
        set(0x7E00A1, 0x01);
        set(0x7EF34A, 0); // Lamp not yet taken -> still the Lamp beat
        set(0x7F2000 + 20 * 64 + 20, 0x58); // chest tile at (20,20)
        ram
    };

    plugin.on_frame(&house((32, 40)), 0);
    plugin.on_frame(&house((32, 40)), 1); // nav auto-starts in the house -> routes to the chest

    // Reach the chest: the custom arrival cue replaces the generic line.
    let out = plugin.on_frame(&house((20, 20)), 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Open the chest")),
        "custom arrival cue at the chest: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("You have arrived")),
        "generic arrival is replaced: {texts:?}"
    );
}

#[test]
fn alttp_zelda_beat_arms_the_courtyard_chain_and_advances_by_proximity() {
    // On the overworld during the "free Zelda" beat, engaging the guide arms a
    // two-waypoint chain (the bushes, then the castle door). The chain drives the
    // white/pink map rendering; here we check the data model: it starts at
    // waypoint 1 and steps to 2 once Link reaches the first spot.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Overworld, Zelda beat: Lamp and sword in hand, Zelda not yet following,
    // progress below 2 — so intro_step() lands on "zelda". Link parked away from
    // the first waypoint (world tile 282,225) so the chain does not pre-advance.
    let frame = |x: u16, y: u16| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld module
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24); // health, so in_play holds
        set(0x7EF36D, 24);
        set(0x7E008A, 0x1B); // Hyrule Castle area
        set(0x7EF34A, 1); // Lamp held -> lamp beat done
        set(0x7EF359, 1); // sword held -> uncle beat done
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // progress < 2
        set(0x7E0022, (x & 0xFF) as u8);
        set(0x7E0023, (x >> 8) as u8);
        set(0x7E0020, (y & 0xFF) as u8);
        set(0x7E0021, (y >> 8) as u8);
        ram
    };

    let away = frame(100 * 8, 100 * 8);
    plugin.on_frame(&away, 0);
    plugin.on_frame(&away, 1);
    plugin.command("advance", &away); // engage the guide -> arms the chain

    let armed = plugin
        .eval("return #nav_chain .. ',' .. nav_chain_i", &away)
        .unwrap();
    assert_eq!(
        armed, "19,1",
        "Zelda beat arms the courtyard chain at index 1: {armed}"
    );

    // Drive Link onto the first waypoint (282,225); a frame there advances to 2.
    let at_wp1 = frame(282 * 8, 225 * 8);
    plugin.on_frame(&at_wp1, 2);
    let advanced = plugin.eval(PICKED, &at_wp1).unwrap();
    assert_eq!(
        advanced, "00,256,225",
        "reaching waypoint 1 advances the chain to 2: {advanced}"
    );
}

#[test]
fn alttp_courtyard_chain_resumes_at_the_door_after_a_dungeon_trip() {
    // The bug: after Link went through the castle door (into the dungeon) and
    // then backtracked out to the courtyard, re-arming the chain sent him all the
    // way back to the first waypoint — the already-cut bushes. It must resume at
    // the door he came back through.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Overworld, Zelda beat (same quest state as the courtyard-chain test).
    let ow = |x: u16, y: u16| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld module
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, 0x1B); // Hyrule Castle area
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // progress < 2
        set(0x7E0022, (x & 0xFF) as u8);
        set(0x7E0023, (x >> 8) as u8);
        set(0x7E0020, (y & 0xFF) as u8);
        set(0x7E0021, (y >> 8) as u8);
        ram
    };
    // A castle-interior frame (module 0x07) with the same quest state, in some
    // room that is not Zelda's cell — the dip through the door.
    let inside = || -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x07); // dungeon module
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3CC, 0);
        set(0x7EF3C5, 0);
        set(0x7E00A0, 0x00); // some room, not Zelda's cell (0x80)
        set(0x7E0022, 0x00);
        set(0x7E0023, 0x01);
        set(0x7E0020, 0x00);
        set(0x7E0021, 0x01);
        ram
    };

    // Arm the chain, then drive Link onto the bushes so it advances to the door.
    let away = ow(100 * 8, 100 * 8);
    plugin.on_frame(&away, 0);
    plugin.on_frame(&away, 1);
    plugin.command("advance", &away); // engage -> arms chain at 1
    plugin.on_frame(&ow(282 * 8, 225 * 8), 2); // reach wp1 -> advance to 2
    assert_eq!(
        plugin.eval(PICKED, &away).unwrap(),
        "00,256,225",
        "reaching the bushes advances to the door"
    );

    // Dip into the castle (module change re-aims and drops the chain), then come
    // back out to the courtyard (module change re-arms it).
    plugin.on_frame(&inside(), 3);
    plugin.on_frame(&away, 4);

    let resumed = plugin
        .eval("return #nav_chain .. ',' .. nav_chain_i", &away)
        .unwrap();
    assert_eq!(
        resumed, "19,2",
        "the chain resumes at the door, not back at the bushes: {resumed}"
    );
}

#[test]
fn alttp_courtyard_chain_arms_at_the_door_when_link_is_already_beside_it() {
    // The player reaches the castle by the door directly, never touching the
    // bushes, then backtracks out to the courtyard just south of the door. Arming
    // the chain there must aim at the door (waypoint 2) — the one Link is standing
    // next to — not send him back northeast to the bushes (waypoint 1).
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let frame = |x: u16, y: u16| -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, 0x1B); // Hyrule Castle area
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // progress < 2 -> Zelda beat
        set(0x7E0022, (x & 0xFF) as u8);
        set(0x7E0023, (x >> 8) as u8);
        set(0x7E0020, (y & 0xFF) as u8);
        set(0x7E0021, (y >> 8) as u8);
        ram
    };

    // Link just south of the castle door (tile 256,225); the bushes are 26 tiles
    // east — far outside the resume reach.
    let at_door = frame(256 * 8, 232 * 8);
    plugin.on_frame(&at_door, 0);
    plugin.on_frame(&at_door, 1);
    plugin.command("advance", &at_door); // engage -> arm the chain here

    let armed = plugin
        .eval("return #nav_chain .. ',' .. nav_chain_i", &at_door)
        .unwrap();
    assert_eq!(
        armed, "19,2",
        "arms at the door Link is beside, not back at the bushes: {armed}"
    );
}

#[test]
fn alttp_zelda_chain_leads_through_the_castle_rooms() {
    // The courtyard chain continues past the door into the castle as dungeon
    // waypoints, room by room: one in room 0x61, then one in room 0x60. The chain
    // stays armed across rooms; reaching one advances to the next without a
    // signature change. The waypoints are silent — the guide tone leads, and the
    // chain no longer narrates where it is setting off to.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Inside the castle, Zelda beat. `room` and world-tile `ltx,lty` vary.
    let frame = |room: u8, ltx: u16, lty: u16| -> Vec<u8> {
        let mut ram = dungeon_frame((ltx, lty), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, room); // dungeon room
        set(0x7E00EE, if room == 0x60 { 1 } else { 0 }); // room 0x60 is on the lower floor
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // progress < 2 -> Zelda beat
        ram
    };

    // In room 0x61, a few tiles west of that room's waypoint (72,415): engaging arms
    // the chain and the driver leads to it.
    let approach = frame(0x61, 65, 415);
    plugin.on_frame(&approach, 0);
    plugin.on_frame(&approach, 1);
    plugin.command("advance", &approach); // engage -> chain armed
    plugin.on_frame(&approach, 2); // driver leads to the waypoint
    assert_eq!(
        plugin
            .eval("return #nav_chain .. ',' .. nav_chain_i", &approach)
            .unwrap(),
        "19,4",
        "the dungeon leg targets room 0x61's waypoint (index 4)"
    );

    // Reaching it records it — no room-graph hop. The chain stays armed; only when
    // Link crosses into room 0x60 (its lower floor) does its waypoint (index 5) take
    // over and lead to the point there.
    plugin.on_frame(&frame(0x61, 72, 415), 3); // arrived -> recorded
    assert!(
        plugin
            .eval("return tostring(nav_chain)", &frame(0x61, 72, 415))
            .unwrap()
            .contains("table"),
        "the chain stays armed after arriving"
    );
    plugin.on_frame(&frame(0x60, 48, 415), 4); // enter room 0x60
    plugin.on_frame(&frame(0x60, 48, 415), 5);
    assert_eq!(
        plugin.eval(PICKED, &frame(0x60, 48, 415)).unwrap(),
        "60,47,392",
        "in room 0x60 the chain leads to that room's waypoint (index 5)"
    );
}

#[test]
fn alttp_routing_crosses_floors_through_the_layer_swap_stairs() {
    // A two-level room is ONE contiguous space to the pathfinder: it crosses the
    // layer-swap stairs on its own, so from the upper floor the guide routes straight
    // to a LOWER-floor waypoint — no hand-placed stair waypoint needed. Room 0x72's
    // chain ends at a lower-floor point (149,507, level 1). And the crossing is
    // DIRECTIONAL: a down-stair (0x3E) lets Link descend to it, but an up-stair (0x1E)
    // at the same spot does not — descending it would be climbing a one-way drop
    // backwards — so the lower point stays unreachable and the guide holds upstairs.
    let r = Registry::builtin();

    // Link upstairs (level 0) at (156,485). `stair` is the tile attribute placed at
    // (151,499) on the upper floor, with a passable landing beneath it on the lower.
    let frame = |stair: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((156, 485), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x72);
        set(0x7E00EE, 0); // upper floor
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat -> courtyard chain armed
        set(0x7EF0E5, 0x80); // room 0x72 chest opened -> no kill sub-goal in the way
                             // (151,499): (499 & 63) = 51, (151 & 63) = 23.
        set(0x7F2000 + 51 * 64 + 23, stair); // upper floor: the stair
        set(0x7F3000 + 51 * 64 + 23, 0x00); // lower floor: passable landing
        ram
    };

    // Down-stair (0x3E): the guide crosses down and targets the lower-floor waypoint.
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let down = frame(0x3E);
    plugin.on_frame(&down, 0);
    plugin.on_frame(&down, 1);
    plugin.command("advance", &down);
    plugin.on_frame(&down, 2);
    assert_eq!(
        plugin.eval(PICKED, &down).unwrap(),
        "72,149,507",
        "a down-stair lets the guide cross to the lower-floor waypoint (index 10)"
    );

    // Up-stair (0x1E) at the same spot: not a downward path, so the lower point is
    // unreachable and the guide holds at the last reachable upper waypoint (index 9).
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let up = frame(0x1E);
    plugin2.on_frame(&up, 0);
    plugin2.on_frame(&up, 1);
    plugin2.command("advance", &up);
    plugin2.on_frame(&up, 2);
    assert_ne!(
        plugin2.eval(PICKED, &up).unwrap(),
        "72,149,507",
        "an up-stair is not a down path: the one-way drop is not routed through backwards"
    );
}

#[test]
fn alttp_a_down_staircase_is_walked_across_not_treated_as_a_wall() {
    // A down-STAIRCASE (0x3D-0x3F) is an in-room stair Link simply walks down, whose far
    // end may be on the SAME floor (0x55's exit pocket, say) — so unlike a swap-layer
    // stair the pathfinder MAY walk across it, exactly as it would plain floor. A solid
    // wall in the same gap is not crossed.
    //
    // Painted from scratch rather than aimed at an authored waypoint: this used to route
    // to COURTYARD's layer-swap-stairs waypoint, and when that waypoint was deleted the
    // only 0x72 point past the wall was on the other floor, so both gap types behaved
    // the same and the test quietly stopped distinguishing them. A door of its own
    // cannot be renumbered or removed out from under it.
    let r = Registry::builtin();

    let frame = |gap: u8| -> Vec<u8> {
        // Link north of a wall band; the door he is guided to is south of it, so the
        // only way through is the gap.
        let mut ram = dungeon_frame((159, 470), (159, 500), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x72);
        set(0x7E00EE, 0); // upper floor
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        // A wall band across the upper floor at grid row 33, with one gap at col 31.
        for tx in 0..64u32 {
            set(0x7F2000 + 33 * 64 + tx, 0x01);
        }
        set(0x7F2000 + 33 * 64 + 31, gap);
        ram
    };

    let crosses = |gap: u8| -> bool {
        let ram = frame(gap);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("pathfind", &ram); // guide to the nearest door
        p.on_frame(&ram, 2);
        p.eval("return tostring(pathfind_active)", &ram).unwrap() == "true"
    };

    assert!(
        crosses(0x3D),
        "a down-staircase in the gap is walked across"
    );
    assert!(!crosses(0x01), "a solid wall in the gap is not");
}

#[test]
fn alttp_an_overlay_mask_hole_is_a_one_way_drop_to_the_lower_floor() {
    // 0x1C is the upper layer's overlay mask: a hole where the raised platform is
    // absent. Link cannot stand on it or walk across it as flat ground (doing so was the
    // layer-swap bug), but he CAN step off the ledge into it and fall to the lower floor
    // — a one-way down portal, like a down-stair reached by stepping toward it. Room
    // 0x72, Link upstairs; its chain ends at a lower-floor waypoint (index 10) reachable
    // only by a floor crossing. With every tile plain floor the two levels are
    // disconnected and that point is unreachable (the guide holds upstairs at index 9);
    // punching a 0x1C hole (passable landing beneath) drops Link through to it (index
    // 10). The drop is upper-only, so it never doubles as a way back up.
    let r = Registry::builtin();

    // `tile` sits at (151,499) on the upper floor, with a passable lower-floor landing.
    let frame = |tile: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((156, 485), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x72);
        set(0x7E00EE, 0); // upper floor
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat -> courtyard chain armed
        set(0x7EF0E5, 0x80); // room 0x72 chest opened -> no kill sub-goal in the way
                             // (151,499): (499 & 63) = 51, (151 & 63) = 23.
        set(0x7F2000 + 51 * 64 + 23, tile); // upper floor: the hole (or plain floor)
        set(0x7F3000 + 51 * 64 + 23, 0x00); // lower floor: passable landing
        ram
    };

    // A 0x1C hole: Link drops off the ledge into it and reaches the lower waypoint (10).
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let hole = frame(0x1C);
    plugin.on_frame(&hole, 0);
    plugin.on_frame(&hole, 1);
    plugin.command("advance", &hole);
    plugin.on_frame(&hole, 2);
    assert_eq!(
        plugin.eval(PICKED, &hole).unwrap(),
        "72,149,507",
        "an overlay-mask hole drops Link to the lower-floor waypoint (index 10)"
    );

    // Plain floor there: no floor crossing anywhere, so the two levels stay separate and
    // the lower waypoint is unreachable — the guide holds at the last upper one (9).
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let solid = frame(0x00);
    plugin2.on_frame(&solid, 0);
    plugin2.on_frame(&solid, 1);
    plugin2.command("advance", &solid);
    plugin2.on_frame(&solid, 2);
    assert_ne!(
        plugin2.eval(PICKED, &solid).unwrap(),
        "72,149,507",
        "with no floor crossing, the lower-floor waypoint stays unreachable"
    );
}

#[test]
fn alttp_a_ledge_drop_lands_across_a_walled_barrier() {
    // Off a raised platform Link falls to the lower floor; if the tile straight below is
    // walled off, the fall keeps going the same way across the hole to the first open
    // lower-floor tile — a ledge-hop landing (in 0x52's escape he drops ~7 tiles west,
    // clear of a walled barrier). Room 0x72, Link upstairs due north of a one-tile-wide
    // hole column; the lower-floor waypoint (index 10, at 149,507) is below it. The lower
    // floor straight under the hole's upper half is walled, so only a fall that scans on
    // to the open floor below reaches the waypoint.
    let r = Registry::builtin();

    let frame = |open_landing: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((149, 488), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x72);
        set(0x7E00EE, 0); // upper floor
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat -> courtyard chain armed
        set(0x7EF0E5, 0x80); // room 0x72 chest opened -> no kill sub-goal in the way
                             // A one-tile hole column at tx=149 on the upper floor, rows 495..505. Grid is
                             // indexed (ty & 63) * 64 + (tx & 63).
        let cell = |tx: u32, ty: u32| (ty & 63) * 64 + (tx & 63);
        for ty in 495..=505u32 {
            set(0x7F2000 + cell(149, ty), 0x1C);
            // Lower floor beneath the hole's upper half is walled (01); its lower half is
            // the open landing (00) that reaches the waypoint — only when open_landing.
            // The barrier is 3 tiles (495-497) so the fall lands within the bounded scan.
            let low = if ty >= 498 && open_landing {
                0x00
            } else {
                0x01
            };
            set(0x7F3000 + cell(149, ty), low);
        }
        ram
    };

    // Open floor below the walled part: the fall scans down to it and reaches waypoint 10.
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let f = frame(true);
    plugin.on_frame(&f, 0);
    plugin.on_frame(&f, 1);
    plugin.command("advance", &f);
    plugin.on_frame(&f, 2);
    assert_eq!(
        plugin.eval(PICKED, &f).unwrap(),
        "72,149,507",
        "the fall scans across the hole to the open floor and reaches the lower waypoint (10)"
    );

    // Every lower tile under the hole walled: the drop cannot land, so the lower waypoint
    // stays unreachable and the guide holds at the last upper one (index 9).
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let f2 = frame(false);
    plugin2.on_frame(&f2, 0);
    plugin2.on_frame(&f2, 1);
    plugin2.command("advance", &f2);
    plugin2.on_frame(&f2, 2);
    assert_ne!(
        plugin2.eval(PICKED, &f2).unwrap(),
        "72,149,507",
        "with every lower tile under the hole walled, the drop cannot land"
    );
}

#[test]
fn alttp_a_push_waypoint_tracks_its_object_and_aligns_only_when_facing_it() {
    // General movable-object strategy: a chain waypoint with `track = <sprite kind>`
    // follows the object's live sprite (offset onto the push side), and `push =
    // <facing>` names the direction Link must face to shove it. The alignment is true
    // only while Link is ON the object AND facing that way — the cue that drives the
    // distinct "push" tone. Movable Mantle (0xEE) at (97,324); a one-waypoint push chain
    // offset (-6,+2) should ride to (91,326).
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = dungeon_frame((91, 326), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x51);
        set(0x7E00EE, 0);
        set(0x7E0DD0, 0x09); // slot 0 active
        set(0x7E0E20, 0xEE); // Movable Mantle
        let mx = 97u16 * 8 + 4;
        let my = 324u16 * 8 + 4;
        set(0x7E0D10, (mx & 0xFF) as u8);
        set(0x7E0D30, (mx >> 8) as u8);
        set(0x7E0D00, (my & 0xFF) as u8);
        set(0x7E0D20, (my >> 8) as u8);
        set(0x7E0ED0, 0x90); // sprite_G for slot 0: the mantle's fully-pushed latch
    }

    // One eval: track the object, then probe alignment facing the push way, the wrong
    // way, and standing off the object entirely.
    // Drive on_frame so the plugin's `prev` is populated from this frame; sprites()
    // (which PUSH.track walks) reads that file-local, so it must be set for real.
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);

    // One eval: inject a one-waypoint push chain, track the object, then probe alignment
    // facing the push way, the wrong way, and standing off the object entirely. `s` is a
    // hand-built Link state (PUSH.active only reads x/y/dungeon_room/direction).
    let script = r#"
        nav_chain = { { room = 0x51, level = 0, tx = 0, ty = 0,
                        track = 0xEE, track_dx = -6, track_dy = 2, push = 6,
                        done = function(s, wp) return wp.slot ~= nil and mem.u8(0x7E0ED0 + wp.slot) == 0x90 end } }
        PUSH.track({ dungeon_room = 0x51 })
        local tracked = nav_chain[1].tx .. "," .. nav_chain[1].ty
        local s = { x = 91*8+4, y = 326*8+4, dungeon_room = 0x51, direction = 6 }
        local on_east = (PUSH.active(s) ~= nil) and (s.direction == nav_chain[1].push)
        s.direction = 0
        local on_north = (PUSH.active(s) ~= nil) and (s.direction == nav_chain[1].push)
        s.x = (91 + 6) * 8 + 4 -- step off, out of reach
        local off = PUSH.active(s) ~= nil
        -- PUSH.track recorded the sprite slot; done() reads its fully-pushed latch.
        local done = nav_chain[1].done(s, nav_chain[1])
        return tracked .. "|" .. tostring(on_east) .. "|" .. tostring(on_north)
            .. "|" .. tostring(off) .. "|slot" .. tostring(nav_chain[1].slot) .. "|" .. tostring(done)
    "#;
    assert_eq!(
        plugin.eval(script, &ram).unwrap(),
        "91,326|true|false|false|slot0|true",
        "waypoint tracks the mantle; aligned only facing the push way; slot recorded and done() reads the fully-pushed latch"
    );
}

#[test]
fn alttp_a_closed_locked_door_blocks_the_route() {
    // A closed flaggable door (0xF0-0xFF) is a solid tile to the game's own
    // classifier and must be one to the pathfinder too, so the guide never leads
    // Link through a locked door he cannot yet open. Same room-0x72 setup as the
    // cross-floor test: a down-stair (0x3E) at (151,499) whose lower-floor landing
    // is the ONLY way to the lower-floor waypoint (index 10). With the landing
    // passable the guide crosses to it; with the landing a closed door it cannot,
    // and holds at the last reachable upper waypoint (index 9) — the door blocks
    // exactly as a wall would, and opening it (which rewrites the tile out of the
    // 0xF0 range) would restore the crossing with no special-casing.
    let r = Registry::builtin();

    let frame = |landing: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((156, 485), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x72);
        set(0x7E00EE, 0); // upper floor
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat -> courtyard chain armed
        set(0x7EF0E5, 0x80); // room 0x72 chest opened -> no kill sub-goal in the way
        set(0x7F2000 + 51 * 64 + 23, 0x3E); // upper floor: a down-stair
        set(0x7F3000 + 51 * 64 + 23, landing); // lower floor: the landing under it
        ram
    };

    // Passable landing (0x00): the guide crosses down to the lower waypoint (10).
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let open = frame(0x00);
    plugin.on_frame(&open, 0);
    plugin.on_frame(&open, 1);
    plugin.command("advance", &open);
    plugin.on_frame(&open, 2);
    assert_eq!(
        plugin.eval(PICKED, &open).unwrap(),
        "72,149,507",
        "an open landing lets the guide cross to the lower-floor waypoint"
    );

    // Closed locked door (0xF0) on the landing: the crossing is blocked, so the
    // lower waypoint is unreachable and the guide holds at the upper one (9).
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let locked = frame(0xF0);
    plugin2.on_frame(&locked, 0);
    plugin2.on_frame(&locked, 1);
    plugin2.command("advance", &locked);
    plugin2.on_frame(&locked, 2);
    assert_ne!(
        plugin2.eval(PICKED, &locked).unwrap(),
        "72,149,507",
        "a closed locked door blocks the route: the guide does not lead through it"
    );
}

#[test]
fn alttp_locked_door_gate_holds_the_route_until_the_door_is_open() {
    // Room 0x71 (Boomerang Chest Room) is open on the lower floor, so a collision block
    // alone can't keep the guide from the far exit (index 15). The exit is gated on the
    // upper-floor locked door (79,486) being OPEN — not merely on holding a key. So the
    // guide holds at the chest anchor (index 13) both keyless and while the door is still
    // shut (a key alone no longer opens the gate — Link is meant to be led to the door,
    // index 14, to unlock it first); only once the door reads open does the exit open up.
    let r = Registry::builtin();

    let frame = |keys: u8, door_open: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((85, 495), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x71); // Boomerang Chest Room
        set(0x7E00EE, 1); // lower floor (Link's grid at 0x7F3000 is open)
        set(0x7E040C, 0x02); // Hyrule Castle
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat -> courtyard chain armed
        set(0x7EF36F, keys); // current-dungeon small keys
                             // upper-floor door (79,486): 0xF0 shut, plain floor once opened
        set(
            0x7F2000 + (486 & 63) * 64 + (79 & 63),
            if door_open { 0x00 } else { 0xF0 },
        );
        ram
    };
    let arm = |ram: &Vec<u8>| -> String {
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(ram, 0);
        p.on_frame(ram, 1);
        p.command("advance", ram);
        p.on_frame(ram, 2);
        p.eval(PICKED, ram).unwrap()
    };

    // Keyless, door shut: holds at the chest anchor (13).
    assert_eq!(
        arm(&frame(0, false)),
        "71,104,495",
        "keyless, the guide holds at the chest anchor"
    );
    // Keyed but the door still shut: still holds — a key alone no longer opens the exit.
    assert_eq!(
        arm(&frame(1, false)),
        "71,104,495",
        "with a key but the door still shut, the guide still holds at the anchor"
    );
    // Door open: the exit past the door opens up (15).
    assert_eq!(
        arm(&frame(1, true)),
        "71,84,455",
        "with the door open the route continues to the exit past it"
    );
}

#[test]
fn alttp_nav_left_on_but_idle_re_arms_itself() {
    // A savestate load can leave nav on while its followers have been cleared, at an
    // unchanged navigation signature. Without a self-heal the guide stays silently
    // idle — no chain, no route, nothing drawn on the map — until toggled off and on.
    // nav_update must notice "on but idle" and re-aim on its own.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Zelda beat, in room 0x61 by the Find-Zelda waypoint: engaging arms the chain.
    let mut ram = dungeon_frame((65, 415), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x61);
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat
    }
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    plugin.command("advance", &ram); // engage -> chain armed
    plugin.on_frame(&ram, 2);
    assert_eq!(
        plugin
            .eval("return tostring(nav_chain ~= nil)", &ram)
            .unwrap(),
        "true",
        "engaging arms the chain"
    );

    // Simulate a savestate load: followers cleared, but nav still on and the
    // signature unchanged (same room/goal). The old signature-only gate would leave
    // this idle forever.
    plugin
        .eval(
            "nav_chain = nil; pathfind_active = false; ow_route_goal = nil; route_room = nil; return 1",
            &ram,
        )
        .unwrap();

    // One more frame at the same position: the self-heal must re-arm the chain.
    plugin.on_frame(&ram, 3);
    assert_eq!(
        plugin
            .eval("return tostring(nav_chain ~= nil)", &ram)
            .unwrap(),
        "true",
        "nav left on but idle re-aims itself without a toggle"
    );
}

#[test]
fn alttp_navigation_starts_itself_when_link_gets_up_in_his_house() {
    // At the very start of the quest, once Link is up out of bed and controllable in
    // his house (in play, room 0x104, no Lamp yet), navigation turns itself on so the
    // opening guidance leads without the player first pressing the key.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = dungeon_frame((296, 1066), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x04);
        set(0x7E00A1, 0x01); // dungeon_room 0x0104 = Link's house
                             // Lamp ($7EF34A) and progress ($7EF3C5) left at 0.
    }

    assert_eq!(
        plugin.eval("return tostring(nav_active)", &ram).unwrap(),
        "false",
        "nav starts off"
    );
    plugin.on_frame(&ram, 0); // caches state
    plugin.on_frame(&ram, 1); // auto-start fires
    assert_eq!(
        plugin.eval("return tostring(nav_active)", &ram).unwrap(),
        "true",
        "nav turns itself on when Link is up in his house at the start"
    );
    // The host reads this to bring the map up on its own when guidance starts.
    assert!(
        plugin.navigation_active(),
        "navigation_active() reports the guidance is on, so the host shows the map"
    );

    // It does not fight a manual toggle-off: turned off in the house, it stays off.
    plugin.command("advance", &ram); // toggle off
    plugin.on_frame(&ram, 2);
    assert_eq!(
        plugin.eval("return tostring(nav_active)", &ram).unwrap(),
        "false",
        "a deliberate toggle-off in the house is respected"
    );
    assert!(
        !plugin.navigation_active(),
        "navigation_active() reflects the toggle-off"
    );
}

#[test]
fn alttp_kill_room_states_the_requirement_and_leads_to_the_enemy() {
    // A dungeon room whose header carries a kill tag (0x0A: clear the room to open
    // the doors) with an enemy still alive: the guide should state the requirement
    // and aim the pathfinder at the enemy, not the (locked) door. Once the enemy is
    // gone, it stops nagging and resumes normal routing.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Link at tile (20,20), a door up at (20,6), and a Green Soldier out at tile
    // (30,20) — on screen but past combat range, so the guide leads toward it.
    let frame = |enemy_state: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF3C5, 2); // progress past the intro, so a post-intro goal is current
        set(0x7E040C, 0x02); // a dungeon id
        set(0x7E00AE, 0x0A); // room-clear kill tag
        set(0x7E0DD0, enemy_state); // slot 0 state
        set(0x7E0E20, 65); // Green Soldier
        set(0x7E0D10, 0xF0); // x lo -> 240
        set(0x7E0D30, 0x00); // x hi
        set(0x7E0D00, 0xA0); // y lo -> 160
        set(0x7E0D20, 0x00); // y hi
        set(0x7E0E50, 4); // health
        ram
    };

    let live = frame(0x09);
    plugin.on_frame(&live, 0);
    plugin.on_frame(&live, 1);
    plugin.command("advance", &live); // engage the guide
    let out = plugin.on_frame(&live, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Defeat all enemies")),
        "states the kill requirement: {texts:?}"
    );

    // The pathfinder is aimed at the enemy's tile (30,20), not the door at (20,6).
    let goal = plugin
        .eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &live,
        )
        .unwrap();
    assert_eq!(
        goal, "30,20",
        "leads to the enemy, not the locked door: {goal}"
    );

    // Enemy defeated (slot inactive): the requirement is not repeated.
    let cleared = frame(0x00);
    let out2 = plugin.on_frame(&cleared, 3);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts2.iter().any(|t| t.contains("Defeat all enemies")),
        "stops nagging once the room is clear: {texts2:?}"
    );
}

#[test]
fn alttp_forced_kill_room_leads_to_the_guard_then_the_chest() {
    // Room 0x72 carries no clear-tag but is force-treated as a kill-room (a guard
    // there drops the key for the locked exit). The guide states the kill
    // requirement and leads to the guard; once the room is quiet, the chest
    // sub-goal takes over and points at the unopened chest.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Dungeon room 0x72, Link at (20,20), an unopened chest tile at (34,20), no
    // kill tag. `alive` toggles the guard (Green Soldier at (30,20)).
    let frame = |alive: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF3C5, 2);
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x72); // room 0x72 -> force kill room
        set(0x7F2000 + 20 * 64 + 34, 0x58); // an unopened chest tile at (34,20)
        set(0x7E0DD0, if alive { 0x09 } else { 0x00 }); // slot 0 state
        set(0x7E0E20, 65); // Green Soldier
        set(0x7E0D10, 0xF4); // x -> 244 -> tile 30
        set(0x7E0D30, 0x00);
        set(0x7E0D00, 0xA4); // y -> 164 -> tile 20
        set(0x7E0D20, 0x00);
        set(0x7E0E50, 4); // health
        ram
    };

    // Guard alive: states the kill requirement and leads to it (tile 30,20), even
    // though the room has no clear-tag.
    let live = frame(true);
    plugin.on_frame(&live, 0);
    plugin.on_frame(&live, 1);
    plugin.command("advance", &live);
    let out = plugin.on_frame(&live, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Defeat all enemies")),
        "force kill-room states the requirement: {texts:?}"
    );
    assert_eq!(
        plugin
            .eval(
                "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
                &live
            )
            .unwrap(),
        "30,20",
        "leads to the guard"
    );

    // Guard gone, room quiet: the chest sub-goal announces (on the first such
    // frame) and takes over.
    let clear = frame(false);
    let out2 = plugin.on_frame(&clear, 3);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts2.iter().any(|t| t.contains("Open the chest")),
        "then the chest sub-goal takes over: {texts2:?}"
    );
}

#[test]
fn alttp_forced_kill_room_stays_cleared_after_its_chest_is_opened() {
    // The forced kill-room (0x72) exists only to fight the guard for the key and
    // open the chest. Once that chest is opened the sub-goal is done for GOOD:
    // backtracking respawns the guard (the room has no clear-tag), but the guide
    // must not re-arm the kill objective — it stays on the linear route.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Room 0x72 with a live (respawned) guard. `chest_open` sets the room's
    // permanent chest-opened bit ($7EF000 + 0x72*2 = $7EF0E4, bit 0x8000).
    let frame = |chest_open: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF3C5, 2);
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x72);
        if chest_open {
            set(0x7EF0E5, 0x80); // high byte of $7EF0E4 -> bit 0x8000
        }
        set(0x7E0DD0, 0x09); // guard respawned, alive
        set(0x7E0E20, 65); // Green Soldier
        set(0x7E0D10, 0xF4); // x -> tile 30
        set(0x7E0D00, 0xA4); // y -> tile 20
        set(0x7E0E50, 4);
        ram
    };

    // Chest already opened: the kill objective must NOT re-arm, even with the
    // guard alive again after a backtrack.
    let opened = frame(true);
    plugin.on_frame(&opened, 0);
    plugin.on_frame(&opened, 1);
    plugin.command("advance", &opened);
    let out = plugin.on_frame(&opened, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("Defeat all enemies")),
        "chest open: kill objective must not re-arm on backtrack: {texts:?}"
    );

    // Contrast: with the chest bit cleared the same live guard DOES arm the kill
    // objective — proving the chest bit is what gates it.
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let unopened = frame(false);
    plugin2.on_frame(&unopened, 0);
    plugin2.on_frame(&unopened, 1);
    plugin2.command("advance", &unopened);
    let out2 = plugin2.on_frame(&unopened, 2);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts2.iter().any(|t| t.contains("Defeat all enemies")),
        "chest not yet open: the kill objective is armed: {texts2:?}"
    );
}

#[test]
fn alttp_room_0x70_is_forced_to_a_kill_room() {
    // Room 0x70 on the escape has no clear-tag, but two guards block the way and the
    // eastern one drops the key for the locked door out. It is forced to a kill-room:
    // the first objective is to defeat them, and it self-clears once none remain.
    let r = Registry::builtin();

    // Room 0x70, Link at (20,20); `alive` toggles a Blue Soldier (type 66) at (30,20).
    let frame = |alive: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF3C5, 2);
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x70); // room 0x70 -> forced kill-room
        set(0x7E0DD0, if alive { 0x09 } else { 0x00 }); // slot 0 state
        set(0x7E0E20, 66); // Blue Soldier
        set(0x7E0D10, 0xF4); // x -> 244 -> tile 30
        set(0x7E0D00, 0xA4); // y -> 164 -> tile 20
        set(0x7E0E50, 4); // health
        ram
    };

    // Guards alive: the room states the kill requirement, though it carries no tag.
    let live = frame(true);
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&live, 0);
    plugin.on_frame(&live, 1);
    plugin.command("advance", &live);
    let out = plugin.on_frame(&live, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Defeat all enemies")),
        "0x70 is forced to a kill-room: {texts:?}"
    );

    // Guards down: no counting enemy, so the kill objective drops and the route resumes.
    let clear = frame(false);
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin2.on_frame(&clear, 0);
    plugin2.on_frame(&clear, 1);
    plugin2.command("advance", &clear);
    let out2 = plugin2.on_frame(&clear, 2);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts2.iter().any(|t| t.contains("Defeat all enemies")),
        "with the guards down the kill objective drops: {texts2:?}"
    );
}

#[test]
fn alttp_giant_kill_room_counts_the_far_enemy_and_ignores_hp0_bystanders() {
    // Room 0x80 is a giant kill-room: the enemy tally reaches across the whole room
    // so the big-key holder waiting in the far east still counts (an ordinary room's
    // ~144px window would miss it). And hp-0 sprites never count, so the caged Zelda
    // NPC (type 118, hp 0) sitting in the room can't hold it "uncleared" forever.
    let r = Registry::builtin();

    // Slot 0 is always Zelda (hp 0, a bystander). Slot 1 is the Ball-and-Chain
    // trooper far to the east; `enemy_alive` toggles it.
    let frame = |enemy_alive: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((10, 518), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x80); // Jail Cell Room -> giant kill-room
                             // Slot 0: Princess Zelda (type 118), hp 0 — must never count.
        set(0x7E0DD0, 0x09);
        set(0x7E0E20, 118);
        set(0x7E0D10, 0x64); // x = 0x0164 -> tile 44 (near Link)
        set(0x7E0D30, 0x01);
        set(0x7E0D00, 0x34); // y = 0x1034 -> tile 518
        set(0x7E0D20, 0x10);
        set(0x7E0E50, 0); // Zelda hp 0
                          // Slot 1: Ball-and-Chain Trooper (type 106), ~328px east — beyond the ~144px
                          // on-screen window, so it only registers under the giant room-wide reach.
        set(0x7E0DD1, if enemy_alive { 0x09 } else { 0x00 });
        set(0x7E0E21, 106);
        set(0x7E0D11, 0x9C); // x = 0x019C -> tile 51
        set(0x7E0D31, 0x01);
        set(0x7E0D01, 0x34); // y -> tile 518
        set(0x7E0D21, 0x10);
        set(0x7E0E51, if enemy_alive { 16 } else { 0 });
        ram
    };

    // Trooper alive in the far east: the giant reach catches it, so the room states
    // its kill requirement even though Zelda (hp 0) sits nearer.
    let live = frame(true);
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&live, 0);
    plugin.on_frame(&live, 1);
    plugin.command("advance", &live);
    let out = plugin.on_frame(&live, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Defeat all enemies")),
        "the far eastern enemy counts under the room-wide reach: {texts:?}"
    );

    // Trooper gone: only Zelda (hp 0) remains, and hp 0 never counts, so the kill
    // objective drops rather than sending Link at the caged princess.
    let clear = frame(false);
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin2.on_frame(&clear, 0);
    plugin2.on_frame(&clear, 1);
    plugin2.command("advance", &clear);
    let out2 = plugin2.on_frame(&clear, 2);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts2.iter().any(|t| t.contains("Defeat all enemies")),
        "the hp-0 Zelda bystander does not hold the room uncleared: {texts2:?}"
    );
}

#[test]
fn alttp_grab_the_key_is_suppressed_while_escorting_zelda() {
    // A slain key-guard becomes its own Key sprite (type 228), and the guide normally
    // says "Grab the key." But while escorting Zelda out of the castle — her follow
    // flag $7EF3CC is set — a respawned guard's key is not needed, so the cue is
    // suppressed rather than pulling the guide off the escort.
    let r = Registry::builtin();

    let frame = |following: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x72);
        set(0x7EF3C5, 1); // has sword, Zelda not yet delivered
        set(0x7EF3CC, following); // Zelda follow flag
                                  // A dropped Key sprite (type 228) near Link.
        set(0x7E0DD0, 0x09); // slot 0 active
        set(0x7E0E20, 228); // Key
        set(0x7E0D10, 0xF4); // x -> 244 -> tile 30
        set(0x7E0D30, 0x00);
        set(0x7E0D00, 0xA4); // y -> 164 -> tile 20
        set(0x7E0D20, 0x00);
        ram
    };

    // Heading in (not following): the guide says to grab the dropped key.
    let inbound = frame(0);
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&inbound, 0);
    plugin.on_frame(&inbound, 1);
    plugin.command("advance", &inbound);
    let out = plugin.on_frame(&inbound, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Grab the key")),
        "a dropped key is called out when heading into the dungeon: {texts:?}"
    );

    // Escorting Zelda out (follow flag set): the key cue is suppressed.
    let escort = frame(1);
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin2.on_frame(&escort, 0);
    plugin2.on_frame(&escort, 1);
    plugin2.command("advance", &escort);
    let out2 = plugin2.on_frame(&escort, 2);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts2.iter().any(|t| t.contains("Grab the key")),
        "the key cue is suppressed while escorting Zelda out: {texts2:?}"
    );
}

#[test]
fn alttp_escape_room_0x71_chest_is_a_routing_objective() {
    // Room 0x71 on the castle escape is a genuine kill-room (header tag 0x08) that
    // also holds a chest worth the detour, so it is listed in CHEST_ROOMS. Once its
    // enemies are down and the key grabbed, the chest sub-goal takes over and points
    // Link at the chest — the same machinery as the 0x72 map chest. But the chest is
    // in a far chamber of the room (past a door and a second fight), so it only takes
    // over once it is on-screen; a distant chest must not pull the guide across early.
    let r = Registry::builtin();

    // `chest_tile` places the (unopened) chest; Link is always at tile (20,20).
    let frame = |chest_x: u32, chest_y: u32| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF3C5, 2);
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x71); // room 0x71
                             // Room 0x71 is a genuine kill-room (header tag 0x08), but this frame is the
                             // state AFTER its fight: the game zeroes a satisfied kill tag itself, and the
                             // guide now reads the tag rather than counting corpses, so a cleared room is a
                             // room whose tag is zero. Leaving 0x08 set here would model a room that is
                             // still gating, and the fight would rightly outrank the chest.
        set(0x7E00AE, 0x00);
        set(0x7F2000 + chest_y * 64 + chest_x, 0x58); // an unopened chest tile
        ram
    };

    // Chest on-screen (14 tiles east): the sub-goal takes over and leads to it.
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let near = frame(34, 20);
    plugin.on_frame(&near, 0);
    plugin.on_frame(&near, 1);
    plugin.command("advance", &near);
    let out = plugin.on_frame(&near, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Open the chest")),
        "the room-0x71 chest is a routing objective when reached: {texts:?}"
    );
    assert_eq!(
        plugin
            .eval(
                "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
                &near
            )
            .unwrap(),
        "34,20",
        "leads to the chest"
    );

    // Chest in a far chamber (off-screen, 40 tiles east): stays quiet, so the guide
    // is free to route through the door and the second fight first.
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let far = frame(60, 20);
    plugin2.on_frame(&far, 0);
    plugin2.on_frame(&far, 1);
    plugin2.command("advance", &far);
    let out2 = plugin2.on_frame(&far, 2);
    let texts2: Vec<String> = out2.iter().map(|i| i.text.clone()).collect();
    assert!(
        !texts2.iter().any(|t| t.contains("Open the chest")),
        "a far, off-screen chest does not take over early: {texts2:?}"
    );
}

#[test]
fn alttp_nav_assist_toggles_on_and_off_with_advance() {
    // The navigation assist (L / advance) is a global on/off toggle: the first
    // press turns it on and aims at the objective, the second turns it off. It is
    // not a one-shot that has to be re-pressed.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, 0x18); // Kakariko
    }
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);

    // First press: on, and it speaks some guidance (here, with no Lamp, the intro
    // turns Link back for the lantern).
    let on: Vec<String> = plugin
        .command("advance", &ram)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        !on.is_empty(),
        "first L turns the assist on and guides: {on:?}"
    );

    // Second press: off.
    let off: Vec<String> = plugin
        .command("advance", &ram)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        off.iter().any(|t| t.contains("Navigation off")),
        "second L turns the assist off: {off:?}"
    );
}

#[test]
fn alttp_intro_without_the_lamp_backtracks_to_the_house() {
    // The Lamp goal completes only when the Lamp is actually held. Skipping it and
    // wandering out onto the overworld should turn Link around toward his house for
    // the lantern, not let him press on to the castle.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld
        set(0x7E0011, 0x00);
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, 0x18); // wandered off to Kakariko, still no Lamp
    }
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    let out = plugin.command("advance", &ram);
    let texts: Vec<&str> = out.iter().map(|i| i.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.to_lowercase().contains("lantern")),
        "backtracks for the lantern instead of pressing on: {texts:?}"
    );
}

// ── Room sweeps ─────────────────────────────────────────────────────────────
// "Show me everything in this room": a generated waypoint chain over whatever the
// room still holds, rather than an authored route through it. Loot and enemies are
// the same mechanism with different collectors, so these test the mechanism once
// through each mode.

/// A dungeon frame with Link at `link`, in room `room` on the upper floor, and no
/// walls — a bare room for a sweep to find things in.
fn sweep_room(link: (u16, u16), room: u8) -> Vec<u8> {
    let mut ram = dungeon_frame(link, (0, 0), &[]);
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E00A0, room);
    set(0x7E00EE, 0); // upper floor
    ram
}

/// Writes sprite slot `slot`: state, kind, world position, health.
fn sprite_slot(ram: &mut [u8], slot: u32, kind: u8, tile: (u16, u16), hp: u8) {
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    let (x, y) = (tile.0 * 8 + 4, tile.1 * 8 + 4);
    set(0x7E0DD0 + slot, 0x09); // active
    set(0x7E0E20 + slot, kind);
    set(0x7E0D10 + slot, (x & 0xFF) as u8);
    set(0x7E0D30 + slot, (x >> 8) as u8);
    set(0x7E0D00 + slot, (y & 0xFF) as u8);
    set(0x7E0D20 + slot, (y >> 8) as u8);
    set(0x7E0E50 + slot, hp);
    // A sprite spawns on whichever floor Link is on, and the plugin reads that to decide
    // whether he can fight it. Frames that left it at zero put every sprite on the upper
    // floor regardless.
    let floor = ram[wram_offset(0x7E00EE).unwrap()];
    ram[wram_offset(0x7E0F20 + slot).unwrap()] = floor;
}

/// Paints a 2x2 chest with its top-left tile at `tile`, as the game lays one out.
fn chest_tiles(ram: &mut [u8], tile: (u16, u16)) {
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    for dy in 0..2u32 {
        for dx in 0..2u32 {
            let (tx, ty) = (tile.0 as u32 + dx, tile.1 as u32 + dy);
            set(0x7F2000 + (ty & 63) * 64 + (tx & 63), 0x58);
        }
    }
}

#[test]
fn alttp_a_loot_sweep_waypoints_every_chest_and_pickup_nearest_first() {
    // The loot collector turns each unopened chest and each loose pickup in the room
    // into one waypoint. A chest occupies a 2x2 tile block, and must yield ONE
    // waypoint (at its top-left), not four — the dedupe is what keeps a chest room
    // from reading as a dozen errands. The chain is sorted nearest-first from Link,
    // which is what makes the sweep a greedy tour, and is flagged `sweep` so the
    // dungeon leg takes the nearest reachable errand rather than the furthest.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = sweep_room((10, 10), 0x51);
    chest_tiles(&mut ram, (20, 20)); // far: 160px away
    sprite_slot(&mut ram, 0, 217, (14, 10), 0); // Green Rupee, near: 32px away

    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    let on: Vec<String> = plugin
        .command("sweep", &ram)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        on.iter().any(|t| t.contains("Loot sweep on")),
        "the first press arms the loot sweep: {on:?}"
    );

    // The next frame collects the room and builds the chain.
    let spoken: Vec<String> = plugin
        .on_frame(&ram, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        spoken.iter().any(|t| t.contains("Two items to collect")),
        "the count is announced: {spoken:?}"
    );

    let script = r#"
        return #nav_chain .. "|" .. tostring(nav_chain.sweep)
            .. "|" .. tostring(nav_chain[1].slot)
            .. "|" .. nav_chain[2].name .. "@" .. nav_chain[2].tx .. "," .. nav_chain[2].ty
    "#;
    assert_eq!(
        plugin.eval(script, &ram).unwrap(),
        "2|true|0|chest@20,20",
        "one waypoint per pickup and per chest (its top-left tile), pickup first as the nearer"
    );
}

#[test]
fn alttp_sweep_waypoints_clear_as_their_loot_is_taken_and_hand_the_guide_back() {
    // Each sweep waypoint carries its own completion test: a chest's tile stops
    // reading as a chest once opened, and a pickup's sprite slot goes free once
    // collected. When the last one clears, the sweep says so and stands down — which
    // is what lets the quest route resume in a room that has been picked clean.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = sweep_room((10, 10), 0x51);
    chest_tiles(&mut ram, (20, 20));
    sprite_slot(&mut ram, 0, 217, (14, 10), 0);

    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    plugin.command("sweep", &ram);
    plugin.on_frame(&ram, 2);

    // Neither errand is done yet.
    // `prev` is a file-local the plugin keeps to itself; a done predicate only reads
    // the module off the state, so a hand-built one is enough here.
    let probe = r#"
        local s = { module = 0x07 }
        local c, i
        for _, wp in ipairs(nav_chain) do
          if wp.name == "chest" then c = wp else i = wp end
        end
        return tostring(c.done(s, c)) .. "|" .. tostring(i.done(s, i))
    "#;
    assert_eq!(
        plugin.eval(probe, &ram).unwrap(),
        "false|false",
        "an unopened chest and an uncollected pickup are both outstanding"
    );

    // Open the chest (its tiles are rewritten out of the chest range) and take the
    // pickup (its slot goes inactive).
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                set(0x7F2000 + ((20 + dy) & 63) * 64 + ((20 + dx) & 63), 0x00);
            }
        }
        set(0x7E0DD0, 0x00); // slot 0 free
    }
    assert_eq!(
        plugin.eval(probe, &ram).unwrap(),
        "true|true",
        "both clear from the game's own signals, with no bookkeeping of our own"
    );

    // Run frames until the throttled re-collect notices the room is empty.
    let mut cleared = false;
    for f in 3..24 {
        if plugin
            .on_frame(&ram, f)
            .iter()
            .any(|i| i.text.contains("Room swept"))
        {
            cleared = true;
            break;
        }
    }
    assert!(cleared, "the swept room is announced");
    assert_eq!(
        plugin.eval("return tostring(nav_chain)", &ram).unwrap(),
        "nil",
        "the sweep drops its chain and hands the guide back"
    );
}

#[test]
fn alttp_a_kill_sweep_waypoints_every_live_enemy_and_follows_it() {
    // The second press cycles to the enemy sweep. Every live enemy in the room gets a
    // waypoint — room-wide, not just on-screen, since the point is finding the one
    // skulking in a corner — and each waypoint rides its sprite, so the guide leads
    // to where the enemy is now rather than where it was when collected. A struck-out
    // enemy's waypoint clears itself.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let mut ram = sweep_room((10, 10), 0x51);
    sprite_slot(&mut ram, 0, 65, (40, 40), 4); // Green Soldier, far corner
    sprite_slot(&mut ram, 1, 65, (16, 10), 4); // Green Soldier, near

    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    plugin.command("sweep", &ram); // loot
    let on: Vec<String> = plugin
        .command("sweep", &ram) // enemies
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        on.iter().any(|t| t.contains("Enemy sweep on")),
        "the second press cycles to the enemy sweep: {on:?}"
    );

    let spoken: Vec<String> = plugin
        .on_frame(&ram, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        spoken.iter().any(|t| t.contains("Two enemies to defeat")),
        "the count is announced: {spoken:?}"
    );
    assert_eq!(
        plugin
            .eval(
                r#"return #nav_chain .. "|" .. tostring(nav_chain[1].slot) .. "@" .. nav_chain[1].tx"#,
                &ram
            )
            .unwrap(),
        "2|1@16",
        "both enemies, the nearer one first"
    );

    // The near soldier walks four tiles east; its waypoint goes with it.
    sprite_slot(&mut ram, 1, 65, (20, 10), 4);
    plugin.on_frame(&ram, 3);
    assert_eq!(
        plugin
            .eval(r#"return tostring(nav_chain[1].tx)"#, &ram)
            .unwrap(),
        "20",
        "the waypoint follows the enemy that carries it"
    );

    // Struck out: hp 0 clears its waypoint, and the sweep drops to one target.
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0E50 + 1, 0);
    }
    let mut down_to_one = false;
    for f in 4..40 {
        plugin.on_frame(&ram, f);
        if plugin.eval("return tostring(#nav_chain)", &ram).unwrap() == "1" {
            down_to_one = true;
            break;
        }
    }
    assert!(
        down_to_one,
        "the defeated enemy leaves the sweep, the far one remains"
    );
}

#[test]
fn alttp_a_sweep_of_an_empty_room_says_so_and_leaves_the_guide_alone() {
    // Arming a sweep in a room with nothing to do must not seize the guide: it
    // reports the room finished and stands down, so the quest route keeps leading.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let ram = sweep_room((10, 10), 0x51);
    plugin.on_frame(&ram, 0);
    plugin.on_frame(&ram, 1);
    plugin.command("sweep", &ram);
    let spoken: Vec<String> = plugin
        .on_frame(&ram, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        spoken.iter().any(|t| t.contains("Room swept")),
        "an empty room reports itself swept: {spoken:?}"
    );
    assert_eq!(
        plugin
            .eval(
                "return tostring(SWEEP.chain) .. \"|\" .. tostring(nav_chain)",
                &ram
            )
            .unwrap(),
        "nil|nil",
        "no chain is installed, so the quest guide is untouched"
    );
}

// ── Waypoint predicates ─────────────────────────────────────────────────────
// The authored chains are pure data in waypoints.lua, so their gates and dones
// arrive as declarative clauses that WP compiles into the closures the driver
// calls. What each chain does with its gates is covered by the route tests above;
// these cover the clause vocabulary itself, which is the part the editor writes.

/// A bare upper-floor dungeon frame with the given tiles painted (tx, ty, attr).
fn clause_frame(tiles: &[(u16, u16, u8)]) -> Vec<u8> {
    let mut ram = dungeon_frame((10, 10), (0, 0), &[]);
    for &(tx, ty, attr) in tiles {
        let off = 0x7F2000 + (ty as u32 & 63) * 64 + (tx as u32 & 63);
        ram[wram_offset(off).unwrap()] = attr;
    }
    ram
}

/// Every clause, against one frame. `s` need only carry the module for a tile
/// read, so the probe builds a literal rather than reaching for the file-local
/// state the real drivers pass in.
const CLAUSE_PROBE: &str = r#"
  local s = { module = 0x07 }
  local wp = { tx = 20, ty = 20, level = 0 }   -- the locked door
  local out = {}
  local function t(c) out[#out + 1] = tostring(WP.test(s, wp, c)) end
  t({"tile_inside", 0xF0, 0xFF})                                  -- the door is shut
  t({"tile_outside", 0xF0, 0xFF})
  t({"at", 21, 21, 0, {"tile_outside", 0xF0, 0xFF}})              -- the open tile beside it
  t({"not", {"tile_inside", 0xF0, 0xFF}})
  t({"any", {"tile_outside", 0xF0, 0xFF}, {"keys"}})              -- open-or-keyed
  t({"all", {"tile_inside", 0xF0, 0xFF}, {"byte", 0x7EF3C5, 2}})
  t({"keys"})
  t({"byte", 0x7EF3C5, 2})
  t({"byte", 0x7EF3C5, 3})
  t({"bit", 0x7EF374, 0x02})                                      -- pendant of Wisdom
  t({"bit", 0x7EF374, 0x04})
  return table.concat(out, "|")
"#;

#[test]
fn alttp_waypoint_clauses_report_the_game_state_they_name() {
    let r = Registry::builtin();
    let read = |keys: u8| -> String {
        // A locked door at (20,20) and open floor at (21,21).
        let mut ram = clause_frame(&[(20, 20, 0xF0), (21, 21, 0x00)]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF36F, keys);
        set(0x7EF3C5, 2); // quest progress: Zelda delivered
        set(0x7EF374, 0x02); // pendants: Wisdom only
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.eval(CLAUSE_PROBE, &ram).unwrap()
    };

    assert_eq!(
        read(0),
        "true|false|true|false|false|true|false|true|false|true|false",
        "keyless, a shut door reads shut and the open-or-keyed gate stays closed"
    );
    // The one clause the key moves: open-or-keyed opens on the key alone.
    assert_eq!(
        read(1),
        "true|false|true|false|true|true|true|true|false|true|false",
        "holding a key opens the open-or-keyed gate, and nothing else changes"
    );
}

#[test]
fn alttp_a_push_clause_reads_the_tracked_sprites_end_stop() {
    // `pushed` is the one clause about a sprite rather than a tile: the Movable
    // Mantle latches sprite_G to 0x90 when it can go no further. Before the sprite
    // is even found (no slot) the errand cannot be done.
    let r = Registry::builtin();
    let read = |slot: Option<u8>, latch: u8| -> String {
        let mut ram = clause_frame(&[]);
        ram[wram_offset(0x7E0ED0 + 3).unwrap()] = latch;
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        let wp = match slot {
            Some(n) => format!("{{ tx = 0, ty = 0, slot = {n} }}"),
            None => "{ tx = 0, ty = 0 }".to_string(),
        };
        p.eval(
            &format!("return tostring(WP.test({{ module = 0x07 }}, {wp}, {{\"pushed\"}}))"),
            &ram,
        )
        .unwrap()
    };

    assert_eq!(
        read(None, 0x90),
        "false",
        "no tracked sprite yet, so not pushed"
    );
    assert_eq!(
        read(Some(3), 0x00),
        "false",
        "the mantle has not reached its stop"
    );
    assert_eq!(read(Some(3), 0x90), "true", "latched at the end stop");
}

#[test]
fn alttp_an_unknown_clause_never_gates_a_waypoint_shut() {
    // A typo in waypoints.lua is an editing mistake, not a reason to wedge the
    // guide: an unrecognised or absent clause degrades to an ungated waypoint.
    let r = Registry::builtin();
    let ram = clause_frame(&[]);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    assert_eq!(
        p.eval(
            r#"
              local s, wp = { module = 0x07 }, { tx = 0, ty = 0 }
              return tostring(WP.test(s, wp, {"frobnicate", 1}))
                .. "|" .. tostring(WP.test(s, wp, nil))
                .. "|" .. tostring(WP.test(s, wp, {"any", {"nope"}}))
            "#,
            &ram
        )
        .unwrap(),
        "true|true|true",
        "unknown and absent clauses read as true, nested ones too"
    );
}

#[test]
fn alttp_the_authored_chains_are_data_the_plugin_compiles() {
    // waypoints.lua is the editor's file: chains, their prose, and clauses. The
    // plugin compiles the clauses into closures in place at load, so the chain the
    // driver walks is the same table the file declared.
    let r = Registry::builtin();
    let ram = clause_frame(&[]);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    assert_eq!(
        p.eval(
            r#"
              local c, sanct = WAYPOINTS.COURTYARD, WAYPOINTS.SANCTUARY
              return table.concat({
                #WAYPOINTS.UNCLE_APPROACH, #c, #sanct,
                type(c.note),                 -- the chain's own prose
                type(c[14].note),             -- the locked door's rationale
                type(c[14].gate),             -- ...compiled from a clause
                tostring(c[14].kind),         -- and its kind says what satisfies it
                tostring(sanct[12].kind),     -- the Movable Mantle's push
                type(c[1].gate),              -- an ungated waypoint stays ungated
                -- A kind supplies the done its waypoint no longer spells out.
                type(KIND.of(c[14]).done),
                tostring(KIND.of(c[1]).done), -- a plain place has nothing to satisfy
              }, "|")
            "#,
            &ram
        )
        .unwrap(),
        "3|19|20|string|string|function|gate|push|nil|function|nil",
        "chains arrive whole, prose intact, clauses compiled to closures"
    );
}

/// Frames between sweep re-collections (SWEEP.PROBE), plus one to land past it.
const SWEEP_PROBE_FRAMES: u64 = 16;

/// Paints a 2x2 manipulable object at `tile` using object slot `slot`, and sets
/// that slot's replacement state — which is what says whether it is a pot.
fn manip_object(ram: &mut [u8], tile: (u16, u16), slot: u8, state: u16) {
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    for dy in 0..2u32 {
        for dx in 0..2u32 {
            let (tx, ty) = (tile.0 as u32 + dx, tile.1 as u32 + dy);
            set(0x7F2000 + (ty & 63) * 64 + (tx & 63), 0x70 | slot);
        }
    }
    set(0x7E0500 + slot as u32 * 2, (state & 0xFF) as u8);
    set(0x7E0500 + slot as u32 * 2 + 1, (state >> 8) as u8);
}

#[test]
fn alttp_a_pot_sweep_finds_pots_and_leaves_the_pushable_block_alone() {
    // Both are attr 0x70-0x7F: the tile cannot tell them apart, only the room's
    // replacement-state slot can. RoomDraw_SinglePot writes 0x1111 for a pot and
    // DrawObjects_PushableBlock writes 0 for a block, and the game lifts only what
    // masks to 0x1010 — so a sweep that read the tile alone would send the player
    // to heave at a block that does not lift.
    let r = Registry::builtin();
    let mut ram = sweep_room((10, 10), 0x55);
    manip_object(&mut ram, (20, 20), 0x1, 0x1111); // a pot
    manip_object(&mut ram, (30, 30), 0x2, 0x0000); // a pushable block
    manip_object(&mut ram, (40, 40), 0x3, 0x4040); // a hammer peg
    manip_object(&mut ram, (16, 16), 0x4, 0x1111); // a nearer pot

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    // off -> loot -> kill -> pots
    p.command("sweep", &ram);
    p.command("sweep", &ram);
    let spoken = p.command("sweep", &ram);
    assert!(
        spoken.iter().any(|i| i.text.contains("Pot sweep on")),
        "the third press reaches the pot sweep: {spoken:?}"
    );
    p.on_frame(&ram, 2);

    assert_eq!(
        p.eval(
            r#"
              local out = {}
              for _, wp in ipairs(SWEEP.chain or {}) do
                out[#out + 1] = wp.name .. "@" .. wp.tx .. "," .. wp.ty
              end
              return #out .. "|" .. table.concat(out, " ")
            "#,
            &ram
        )
        .unwrap(),
        "2|pot@16,16 pot@20,20",
        "both pots, one waypoint each despite the 2x2, nearest first; no block, no peg"
    );

    // Lifting a pot clears its slot's pot state, and its waypoint retires with it.
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0500 + 0x4 * 2, 0x00);
        set(0x7E0500 + 0x4 * 2 + 1, 0x00);
    }
    // Targets are re-collected on a timer, not every frame, so run past it.
    for f in 3..3 + SWEEP_PROBE_FRAMES {
        p.on_frame(&ram, f);
    }
    assert_eq!(
        p.eval("return tostring(SWEEP.chain and #SWEEP.chain)", &ram)
            .unwrap(),
        "1",
        "the lifted pot's waypoint is dropped, the other stays"
    );
}

// ── Map labels ──────────────────────────────────────────────────────────────
// A 64-tile room is drawn 200 pixels across, so a tile is barely three pixels and
// a two-digit label is eleven wide — labels crowd each other badly, and the route
// numbers every tile of the path. LABELS keeps them apart or drops them.

/// Places labels through LABELS against a stub canvas and reports what landed
/// where, as "text@x,y" in draw order.
const LABEL_PROBE: &str = r#"
  local drawn = {}
  local canvas = { text = function(self, x, y, s) drawn[#drawn + 1] = s .. "@" .. x .. "," .. y end }
  LABELS.reset()
  LABELS.number(canvas, POINTS)
  return table.concat(drawn, " ")
"#;

fn place(points: &str) -> String {
    let r = Registry::builtin();
    let ram = vec![0u8; 128 * 1024];
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.eval(&format!("POINTS = {points}"), &ram).unwrap();
    p.eval(LABEL_PROBE, &ram).unwrap()
}

#[test]
fn alttp_a_crowded_map_label_moves_aside_or_is_dropped() {
    // One label alone takes the spot a reader looks first: up and to the right.
    assert_eq!(
        place(r#"{ { 50, 50, text = "7", color = 0 } }"#),
        "7@53,47",
        "a lone label sits above and right of its marker"
    );

    // A second marker three pixels away — one tile — cannot use that spot, so it
    // goes to the left of its own marker rather than printing over the first.
    let two = place(r#"{ { 50, 50, text = "7", color = 0 }, { 53, 50, text = "8", color = 0 } }"#);
    assert_eq!(two, "7@53,47 8@45,47", "the crowded label steps aside");

    // Six markers on the same pixel exhaust the offsets: the ones that fit are
    // drawn, the rest are dropped rather than smeared on top of each other.
    let pile: Vec<String> = (0..8)
        .map(|i| format!(r#"{{ 60, 60, text = "{i}", color = 0 }}"#))
        .collect();
    let drawn = place(&format!("{{ {} }}", pile.join(", ")));
    let count = drawn.split_whitespace().count();
    assert!(
        count < 8,
        "labels stacked on one point cannot all be drawn: {drawn}"
    );
    // Every label that was drawn is at a distinct position.
    let mut spots: Vec<&str> = drawn
        .split_whitespace()
        .map(|d| d.split('@').nth(1).unwrap())
        .collect();
    let before = spots.len();
    spots.sort_unstable();
    spots.dedup();
    assert_eq!(
        spots.len(),
        before,
        "no two labels share a position: {drawn}"
    );
}

#[test]
fn alttp_the_active_label_wins_the_spot_it_wants() {
    // The immediate goal is the one number that has to be readable, so it is placed
    // before its neighbours even when it comes later in the list.
    assert_eq!(
        place(
            r#"{ { 50, 50, text = "7", color = 0 },
                 { 53, 50, text = "8", color = 0, first = true } }"#
        ),
        "8@56,47 7@42,47",
        "the active label takes its preferred spot and the other steps aside"
    );
}

// ── Hazards underfoot ───────────────────────────────────────────────────────
// A pit does not stop you, it punishes you for walking in, so the router treating
// it as impassable is not enough — the edge has to be announced before the step.

/// The tile Link faces, by the same arithmetic the plugin uses (x + 8 is his centre
/// column, y + 12 his feet, then one tile on in the facing direction). Computed
/// rather than hardcoded so these tests state the geometry instead of guessing it.
fn faced_tile(link: (u16, u16), dir: u8) -> (u16, u16) {
    let (x, y) = ((link.0 * 8 + 4) as i32, (link.1 * 8 + 4) as i32);
    let ax = x
        + 8
        + if dir == 4 {
            -12
        } else if dir == 6 {
            12
        } else {
            0
        };
    let ay = y
        + 12
        + if dir == 0 {
            -12
        } else if dir == 2 {
            12
        } else {
            0
        };
    ((ax >> 3) as u16, (ay >> 3) as u16)
}

/// A dungeon frame with Link at `link` facing `dir` (0 north, 2 south, 4 west,
/// 6 east) and the given tiles painted on the upper floor.
fn facing_frame(link: (u16, u16), dir: u8, tiles: &[(u16, u16, u8)]) -> Vec<u8> {
    let mut ram = dungeon_frame(link, (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x55);
        set(0x7E00EE, 0);
        set(0x7E002F, dir);
    }
    for &(tx, ty, attr) in tiles {
        ram[wram_offset(0x7F2000 + (ty as u32 & 63) * 64 + (tx as u32 & 63)).unwrap()] = attr;
    }
    ram
}

#[test]
fn alttp_facing_a_pit_sounds_a_danger_tone_and_says_nothing() {
    // A tone, not a word. Speech is the channel everything else competes for, and "Pit."
    // arrives slower than the danger and is gone once spoken; a tone lasts as long as the
    // edge does and pans to say where it is.
    let r = Registry::builtin();
    let pit_at = faced_tile((20, 20), 2);
    let ram = facing_frame((20, 20), 2, &[(pit_at.0, pit_at.1, 0x20)]);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    let out = p.on_frame(&ram, 1);

    assert!(
        !out.iter().any(|i| i.text.to_lowercase().contains("pit")),
        "nothing is spoken: {:?}",
        out.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
    let b = p.beacons();
    let hazard = b
        .iter()
        .find(|b| b.id == "hazard")
        .expect("a hazard tone sounds");
    assert!(
        hazard.pitch < 1.0 && hazard.tremolo >= 4.0,
        "low and urgent: {hazard:?}"
    );
    assert!(
        hazard.dy > 0.0,
        "panned south, where the pit is: {hazard:?}"
    );

    // It persists rather than firing once: the edge is still there next frame.
    p.on_frame(&ram, 2);
    assert!(
        p.beacons().iter().any(|b| b.id == "hazard"),
        "the tone holds while Link keeps facing it"
    );
}

#[test]
fn alttp_turning_away_from_a_pit_stops_the_tone_and_facing_back_starts_it() {
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let pit_at = faced_tile((20, 20), 2);
    let pit = facing_frame((20, 20), 2, &[(pit_at.0, pit_at.1, 0x20)]);
    // Same pit tile, but facing north — what he would step into is open floor.
    let away = facing_frame((20, 20), 0, &[(pit_at.0, pit_at.1, 0x20)]);
    p.on_frame(&pit, 0);
    p.on_frame(&pit, 1);
    assert!(
        p.beacons().iter().any(|b| b.id == "hazard"),
        "facing it: sounding"
    );

    p.on_frame(&away, 2);
    assert!(
        !p.beacons().iter().any(|b| b.id == "hazard"),
        "facing open ground, the tone stops"
    );
    p.on_frame(&pit, 3);
    assert!(
        p.beacons().iter().any(|b| b.id == "hazard"),
        "turning back onto it sounds again: it is about the step he is about to take"
    );
}

#[test]
fn alttp_the_pit_tone_covers_the_dungeon_hole_variants_but_not_the_overworld() {
    // TileBehavior_Pit is 0x20 plus the 0xB0-0xBD holes-with-a-destination.
    let r = Registry::builtin();
    let sounds = |attr: u8, overworld: bool| -> bool {
        let e = faced_tile((20, 20), 6);
        let mut ram = facing_frame((20, 20), 6, &[(e.0, e.1, attr)]);
        if overworld {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E0010, 0x09); // overworld
            set(0x7E001B, 0x00); // outdoors
        }
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.beacons().iter().any(|b| b.id == "hazard")
    };

    for attr in [0x20u8, 0xB0, 0xBD] {
        assert!(sounds(attr, false), "attr 0x{attr:02X} is a pit");
    }
    assert!(!sounds(0xBE, false), "0xBE is past the class and is not");
    // The overworld gives entrance holes the same attribute and they are meant to be
    // fallen into, so the tone is dungeon-only.
    assert!(
        !sounds(0x20, true),
        "no pit tone on the overworld, where the same tile is a doorway"
    );
}
#[test]
fn alttp_an_unreachable_objective_falls_through_to_one_the_guide_can_reach() {
    // Room 0x71's two guard pits are walled off from each other. With the key-holder
    // in the far pit and Link in the near one, `keyholder` outranks `kill` but cannot
    // be routed to; committing to it left the guide silent with an enemy five tiles
    // away. It must fall through to the reachable one.
    let r = Registry::builtin();
    // Link at 77,499 puts the room window at tiles 64-127 by 460-523, so the wall has
    // to span the full window height or the router simply walks around its end.
    let wall: Vec<(u16, u16)> = (460..524).map(|y| (100u16, y as u16)).collect();
    let mut ram = dungeon_frame((77, 499), (0, 0), &wall);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x71); // Boomerang Chest Room: kill-tagged, two pits
        set(0x7E00AE, 0x08); // a clear-tag in KILL_TAGS
        set(0x7E00EE, 0); // upper floor, the grid dungeon_frame paints the wall on
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 0);
    }
    // Near enemy, this side of the wall. Far enemy beyond it, carrying the key.
    sprite_slot(&mut ram, 0, 66, (81, 496), 4); // Blue Soldier, reachable
    sprite_slot(&mut ram, 1, 66, (116, 496), 6); // Green Soldier, walled off
                                                 // die_action ($7E0CBA) non-zero marks a guard that still drops its key. Only the
                                                 // far one, so key_holder can pick nothing but the unreachable enemy.
    ram[wram_offset(0x7E0CBA + 1).unwrap()] = 0x0B;

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    p.on_frame(&ram, 2);

    let goal = p
        .eval(
            "return pathfind_goal and (pathfind_goal[1] .. ',' .. pathfind_goal[2]) or 'nil'",
            &ram,
        )
        .unwrap();
    assert_eq!(
        p.eval("return tostring(pathfind_active)", &ram).unwrap(),
        "true",
        "the guide is routing somewhere rather than stalling (goal {goal})"
    );
    assert_eq!(
        goal, "81,496",
        "it aims at the reachable enemy, not the walled-off key-holder at 116,496"
    );
}

// ── Room rules are gone ─────────────────────────────────────────────────────
// There is no ROOMS table any more. What a room is like is either read from the game
// (its kill tag) or worked out from its collision (which of it Link can reach), and
// what to DO in a room is a waypoint. This asserts the table stays gone, because it
// grew back twice: first holding kill rules, then chamber boxes.

#[test]
fn alttp_there_are_no_hand_authored_room_rules_left() {
    let r = Registry::builtin();
    let ram = clause_frame(&[]);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    assert_eq!(
        p.eval(
            r#"
              return table.concat({
                tostring(ROOMS),              -- no room table at all
                type(REACH.can),              -- reachability answers the wall question
                tostring(WP.fights[0x70]),    -- and an authored fight says the area is
                tostring(WP.fights[0x72]),    -- the room, for the rooms that set no tag
                tostring(WP.fights[0x80]),
                tostring(WP.fights[0x71]),    -- 0x71 sets its own tag, so it needs none
              }, "|")
            "#,
            &ram
        )
        .unwrap(),
        "nil|function|true|true|true|nil",
        "rooms are read, not written down"
    );
}
#[test]
fn alttp_the_chest_opened_clause_reads_the_rooms_permanent_bit() {
    // $7EF000 + room*2, bit 0x8000, set for good once the chest is opened. Room 0x72's
    // kill rule is the negation, which is what stops a backtrack re-arming the fight.
    let r = Registry::builtin();
    let probe = r#"
        local s = { module = 0x07, dungeon_room = 0x72 }
        return tostring(WP.test(s, { room = 0x72 }, {"chest_opened"}))
          -- 0x72's fight is a chain step gated on the chest being shut, so the clause
          -- is read through that waypoint's own gate rather than a room rule.
          .. "|" .. tostring(WP.test(s, { room = 0x72 }, {"not", {"chest_opened"}}))
          .. "|" .. tostring(WP.test(s, { room = 0x71 }, {"chest_opened"}))
    "#;
    let read = |opened: bool| -> String {
        let mut ram = clause_frame(&[]);
        if opened {
            ram[wram_offset(0x7EF000 + 0x72 * 2 + 1).unwrap()] = 0x80;
        }
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.eval(probe, &ram).unwrap()
    };
    assert_eq!(
        read(false),
        "false|true|false",
        "chest shut: 0x72 is a kill-room, and 0x71's own bit is untouched"
    );
    assert_eq!(
        read(true),
        "true|false|false",
        "chest opened: the kill rule goes quiet, and only 0x72's bit moved"
    );
}

// ── The map renders ─────────────────────────────────────────────────────────
// on_draw had no test, and a rename of a file-local it called turned every map
// frame into a Lua error: the host logs it to stderr and returns no map, so the
// window simply stopped updating and nothing else noticed. A draw pass touches the
// whole read side of the plugin, so just running it without error is worth having.

/// Draws one frame and returns the error, or None if it rendered.
fn draw_error(plugin: &mut LuaPlugin, ram: &[u8]) -> Option<String> {
    let probe = r#"
        local ok, err = pcall(on_draw, __beacon_canvas, 0)
        return ok and "ok" or tostring(err)
    "#;
    match plugin.eval(probe, ram) {
        Ok(s) if s == "ok" => None,
        Ok(s) => Some(s),
        Err(e) => Some(e),
    }
}

#[test]
fn alttp_the_map_draws_without_error_in_every_context() {
    let r = Registry::builtin();

    // A dungeon room, with the guide armed and a chain to draw.
    let mut ram = dungeon_frame((77, 499), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x71);
        set(0x7E00AE, 0x08); // kill-tagged, so the objective overlay is exercised
        set(0x7E00EE, 1);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 0);
    }
    sprite_slot(&mut ram, 0, 66, (81, 496), 4);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram); // arm the guide: chain, route and labels all draw
    p.on_frame(&ram, 2);
    assert_eq!(
        draw_error(&mut p, &ram),
        None,
        "a guided dungeon room draws"
    );

    // A sweep replaces the chain with a generated one; its waypoints draw too.
    p.command("sweep", &ram);
    p.on_frame(&ram, 3);
    assert_eq!(draw_error(&mut p, &ram), None, "a room being swept draws");

    // A room whose active step has no place of its own. Room 0x70's step is a
    // room-clear, whose target is whichever enemy is nearest, so there is no tile to
    // plot — and the drawing loops used to reach straight for wp.tx. This is the case
    // that killed the map, and the 0x71 frame above cannot catch it because no
    // authored clear step lives there.
    let mut placeless = dungeon_frame((20, 452), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| placeless[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x70);
        set(0x7E00EE, 0);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 0); // COURTYARD armed, so its clear step for 0x70 is in play
    }
    sprite_slot(&mut placeless, 0, 66, (30, 456), 4);
    let mut p3 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p3.on_frame(&placeless, 0);
    p3.on_frame(&placeless, 1);
    p3.command("advance", &placeless);
    p3.on_frame(&placeless, 2);
    assert_eq!(
        p3.eval(PICKED, &placeless).unwrap(),
        "70,nil,nil",
        "the active step really is the placeless one"
    );
    assert_eq!(
        draw_error(&mut p3, &placeless),
        None,
        "a step with no tile of its own still draws"
    );

    // Before any state has been read, and outside play: the map says so rather than
    // reaching into a nil state.
    let mut fresh = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    assert_eq!(
        draw_error(&mut fresh, &ram),
        None,
        "no state yet still draws"
    );
    let mut title = vec![0u8; 128 * 1024]; // module 0x00: not in play
    title[wram_offset(0x7EF36C).unwrap()] = 24;
    let mut p2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p2.on_frame(&title, 0);
    assert_eq!(draw_error(&mut p2, &title), None, "the title screen draws");

    // And the host agrees it produced a map, not just that no error was raised.
    let mut out = Vec::new();
    assert!(
        p.draw(&ram, 4, &mut out).is_some(),
        "the host gets pixels back, so get_map has something to return"
    );
}

#[test]
fn alttp_an_authored_waypoint_number_is_teal_and_a_generated_one_is_not() {
    // Two numberings share the map. A teal number is an index into waypoints.lua, so
    // it can be looked up and moved in the editor; a sweep's numbers are generated
    // and correspond to nothing in the file, so they must not read as editable.
    let r = Registry::builtin();
    let probe = r#"
        local seen = {}
        local canvas = {
          text = function(self, x, y, s, color) seen[#seen + 1] = string.format("%06X", color) end,
          rect = function() end, line = function() end, clear = function() end,
        }
        LABELS.reset()
        local labels = {}
        local nc = nav_chain.sweep and 0x50D0F0 or 0x20B0A0
        for i, wp in ipairs(nav_chain) do
          labels[#labels + 1] = { i * 20, i * 20, text = tostring(i), color = nc }
        end
        LABELS.number(canvas, labels)
        return table.concat(seen, ",")
    "#;

    let mut ram = dungeon_frame((77, 499), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x71);
        // Upper floor: the floor chest_tiles paints on, so the loot sweep finds its
        // chest and keeps its generated chain instead of standing down again.
        set(0x7E00EE, 0);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 0);
    }
    chest_tiles(&mut ram, (80, 500)); // something for a loot sweep to find

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram); // the authored chain
    p.on_frame(&ram, 2);
    let authored = p.eval(probe, &ram).unwrap();
    assert!(
        authored.split(',').all(|c| c == "20B0A0"),
        "every authored number is teal: {authored}"
    );

    p.command("sweep", &ram); // a generated chain replaces it
    p.on_frame(&ram, 3);
    let swept = p.eval(probe, &ram).unwrap();
    assert!(
        !swept.is_empty() && swept.split(',').all(|c| c == "50D0F0"),
        "a generated number is not teal: {swept}"
    );
}

#[test]
fn alttp_a_clear_waypoint_leads_to_the_enemies_and_holds_the_route_until_they_are_down() {
    // COURTYARD 15 is `clear` for room 0x70 and 16 is the room's exit. The clear step
    // is `via`, so the guide must stay on the enemies rather than skipping to the exit
    // — and must hand on to the exit the moment the room is quiet.
    let r = Registry::builtin();
    // Whether the room is still occupied is decided by the caller adding a sprite.
    let frame = || -> Vec<u8> {
        let mut ram = dungeon_frame((20, 452), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x70);
        set(0x7E00EE, 0);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3CC, 0);
        set(0x7EF3C5, 0); // Zelda beat -> COURTYARD armed
        ram
    };
    let arm = |ram: &Vec<u8>| -> (String, String) {
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(ram, 0);
        p.on_frame(ram, 1);
        p.command("advance", ram);
        p.on_frame(ram, 2);
        (
            p.eval(PICKED, ram).unwrap(),
            p.eval(
                "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
                ram,
            )
            .unwrap(),
        )
    };

    let mut live = frame();
    sprite_slot(&mut live, 0, 66, (30, 456), 4); // a Blue Soldier in the room
    let (i, goal) = arm(&live);
    assert_eq!(
        i, "70,nil,nil",
        "the clear step is the target, not the room's exit"
    );
    assert_eq!(
        goal, "30,456",
        "and it leads to the enemy, wherever it stands"
    );

    // With the room quiet the clear step retires itself and the exit takes over.
    let quiet = frame();
    let (i2, _) = arm(&quiet);
    assert_eq!(
        i2, "70,10,452",
        "a cleared room hands on to the exit waypoint"
    );
}

#[test]
fn alttp_a_clear_waypoint_takes_over_from_the_room_objective() {
    // The room-scoped objectives are the fallback for rooms no chain covers. Where a
    // chain has a clear step, that step drives — otherwise the same fight would be
    // run twice, once in route order and once as an override.
    let r = Registry::builtin();
    let mut ram = dungeon_frame((20, 452), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x70);
        set(0x7E00EE, 0);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 0);
    }
    sprite_slot(&mut ram, 0, 66, (30, 456), 4);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    p.on_frame(&ram, 2);
    assert_eq!(
        p.eval(
            "return tostring(chain_clears_room and 'x' or nav_chain_i)",
            &ram
        )
        .unwrap(),
        "16",
        "the chain's clear step is what the guide committed to"
    );
    // The objective did not also fire: room_obj_announced stays clear, so nothing
    // overrode the chain.
    assert_eq!(
        p.eval("return tostring(room_obj_announced)", &ram).unwrap(),
        "nil",
        "no room objective overrode the chain's own step"
    );
}

#[test]
fn alttp_an_authored_fight_still_speaks_when_no_chain_covers_the_room() {
    // The hole this closes. Room 0x70's fight is a step in COURTYARD, but COURTYARD's
    // goal completes at progress 2, so a later backtrack into the room had no chain to
    // consult and the guards blocking the passage went unmentioned. The errand index
    // makes the step visible from any quest state, which is why the room needs no
    // separate rule of its own.
    let r = Registry::builtin();
    let mut ram = dungeon_frame((20, 452), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x70);
        set(0x7E00EE, 0);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 2); // Zelda delivered: neither castle chain is armed any more
    }
    sprite_slot(&mut ram, 0, 66, (30, 456), 4); // a guard is back

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    let out = p.on_frame(&ram, 2);

    assert_eq!(
        p.eval("return tostring(nav_chain)", &ram).unwrap(),
        "nil",
        "no chain is armed at this point in the quest"
    );
    assert_eq!(
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram
        )
        .unwrap(),
        "30,456",
        "and the guide still leads to the guard, from the authored step alone"
    );
    assert!(
        out.iter().any(|i| i.text.contains("Defeat all enemies")),
        "stating the requirement: {:?}",
        out.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
}

#[test]
fn alttp_a_chamber_stops_an_enemy_counting_from_behind_a_wall() {
    // Room 0x71's two guard pits are walled off from each other, and the west pit's
    // mapped chamber ends at tile 90. A guard in the east pit is well inside the old
    // 144-pixel radius, so it used to hold the room uncleared and could be picked as
    // "nearest" from behind a wall Link cannot cross. The chamber has the wall in it.
    let r = Registry::builtin();
    let frame = || -> Vec<u8> {
        let mut ram = dungeon_frame((77, 499), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x71);
        set(0x7E00AE, 0x08); // clear-tagged, so the enemy objective is what speaks
        set(0x7E00EE, 1);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 2); // past the castle chains: no authored fight here
        ram
    };
    let goal = |enemy: (u16, u16)| -> String {
        let mut ram = frame();
        sprite_slot(&mut ram, 0, 66, enemy, 4);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("advance", &ram);
        p.on_frame(&ram, 2);
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram,
        )
        .unwrap()
    };

    // Link stands at 77,499 — inside the west pit (69..90 by 491..506).
    assert_eq!(
        goal((85, 499)),
        "85,499",
        "a guard sharing Link's chamber is the target"
    );
    // The east pit starts at tile 101. Within the old radius, outside the chamber.
    assert_eq!(
        goal((105, 499)),
        "nil",
        "a guard in the other pit is not counted, so nothing is aimed at it"
    );
}

// ── The unauthored fallback ─────────────────────────────────────────────────
// Rooms nobody has mapped still work, from the room's own header tag. The tag says
// the room gates on a fight, says over what area, and says when it is satisfied —
// the game zeroes it — so the fallback asks the game rather than guessing.

#[test]
fn alttp_the_fallback_bounds_the_fight_the_way_the_tag_does() {
    // Tag 0x0A goes through RoomTag_RoomTrigger, which waits on
    // Sprite_CheckIfRoomIsClear: every sprite slot, no bounds. Tag 0x08 goes through
    // RoomTag_QuadrantTrigger, which waits on Sprite_CheckIfScreenIsClear: only what
    // is within 256x256 of the scroll origin. A far enemy therefore counts under one
    // tag and not the other, and neither is a radius.
    let r = Registry::builtin();
    let goal = |tag: u8, enemy: (u16, u16)| -> String {
        // Scroll origin at (0,0), so the kill screen is world pixels 0..255.
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x55); // a room with no authored chambers
            set(0x7E00AE, tag);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3C5, 2);
        }
        sprite_slot(&mut ram, 0, 66, enemy, 4);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("advance", &ram);
        p.on_frame(&ram, 2);
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram,
        )
        .unwrap()
    };

    // With the scroll origin at 0, the kill screen is world pixels 0..255 — tiles 0..31.
    // Tile 28 is pixel 228, inside it; tile 40 is pixel 324, outside.
    assert_eq!(
        goal(0x08, (28, 20)),
        "28,20",
        "on-screen: counted under a screen tag"
    );
    assert_eq!(
        goal(0x08, (40, 20)),
        "nil",
        "off-screen: not counted under a screen tag"
    );
    assert_eq!(
        goal(0x0A, (40, 20)),
        "40,20",
        "the same enemy counts under a room tag"
    );
}

#[test]
fn alttp_the_fallback_needs_both_a_tag_and_something_to_fight() {
    // Every kill tag reaches RoomTag_OperateChestReveal or Dung_TagRoutine_TrapdoorsUp,
    // both of which zero it, so the tag is the room's own "still gating" flag. Reading
    // it rather than counting sprites means the guide cannot go quiet while a room's
    // enemies are mid-spawn, nor re-arm on a respawn after the room is done with.
    let r = Registry::builtin();
    let says_fight = |tag: u8, enemy: bool| -> bool {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x55);
            set(0x7E00AE, tag);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3C5, 2);
        }
        if enemy {
            // Tile 28 is world pixel 228 — inside the 256-pixel kill screen.
            sprite_slot(&mut ram, 0, 66, (28, 20), 4);
        }
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("advance", &ram);
        p.on_frame(&ram, 2)
            .iter()
            .any(|i| i.text.contains("Defeat all enemies"))
    };

    assert!(
        says_fight(0x08, true),
        "tag set with an enemy: the room gates"
    );
    // The tag alone is not enough to CLAIM a fight. A room can be gating with nothing
    // Link can reach from where he stands — its enemies in another chamber, or the game
    // yet to run the tag routine for his quadrant — and announcing a fight with no
    // target left the guide repeating it forever with no way to advance. The tag governs
    // when a fight is over; a countable enemy governs whether there is one to point at.
    assert!(
        !says_fight(0x08, false),
        "tag set but nothing countable here: no fight is claimed"
    );
    assert!(
        !says_fight(0x00, true),
        "no tag: not a kill room, whatever is standing there"
    );
}

#[test]
fn alttp_a_key_holder_across_a_wall_is_not_targeted_and_no_fight_is_claimed() {
    // Reported live in room 0x71: the last guard is in the east pit, Link is in the
    // west one, and the room's tag stays set because the room is not finished. The
    // guide announced "Defeat all enemies", aimed at the guard through the wall, and
    // then never moved — the goal was unreachable so nothing refreshed it, and the
    // objective re-claimed itself every frame.
    let r = Registry::builtin();
    let mut ram = dungeon_frame((90, 495), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x71);
        set(0x7E00AE, 0x08); // still gating: its other pit is not clear
        set(0x7E00EE, 1);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3C5, 0);
    }
    // The only live enemy is in the east pit (chamber 101..122), carrying the key.
    sprite_slot(&mut ram, 1, 66, (116, 496), 6);
    ram[wram_offset(0x7E0CBA + 1).unwrap()] = 0x0B; // die_action: drops a key

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    let out = p.on_frame(&ram, 2);

    assert_eq!(
        p.eval("return tostring(key_holder({ x = 726, y = 3960, module = 0x07, dungeon_room = 0x71 }))", &ram)
            .unwrap(),
        "nil",
        "the key-holder in the other pit is out of Link's chamber, so not a target"
    );
    assert!(
        !out.iter().any(|i| i.text.contains("Defeat")),
        "no fight is claimed when there is nothing here to fight: {:?}",
        out.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
    let goal = p
        .eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram,
        )
        .unwrap();
    assert_ne!(goal, "116,496", "and nothing is aimed through the wall");
}

// ── Reachability instead of authored chambers ───────────────────────────────
// A chamber box was an approximation of "can Link walk there". A flood fill from his
// tile answers it exactly, follows walls that are not rectangles, and needs nothing
// written down — the walls are already in the collision data.

#[test]
fn alttp_an_enemy_behind_a_wall_does_not_count_without_any_authored_chamber() {
    // Room 0x55, which has no ROOMS entry at all, split by a wall band with no gap.
    // Room-scoped kill tag (0x0A), so the tag itself imposes no bound: only the fill
    // separates the two halves.
    let r = Registry::builtin();
    let goal = |enemy: (u16, u16), gap: bool| -> String {
        let mut ram = dungeon_frame((20, 20), (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x55);
            set(0x7E00AE, 0x0A); // room-wide kill tag
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3C5, 2);
            // A wall right across the room at grid row 30, optionally with one gap.
            for tx in 0..64u32 {
                set(0x7F2000 + 30 * 64 + tx, 0x01);
            }
            if gap {
                set(0x7F2000 + 30 * 64 + 31, 0x00);
            }
        }
        sprite_slot(&mut ram, 0, 66, enemy, 4);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("advance", &ram);
        p.on_frame(&ram, 2);
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram,
        )
        .unwrap()
    };

    // Link is at tile 20; the wall is at row 30. An enemy on his side counts.
    assert_eq!(
        goal((26, 20), false),
        "26,20",
        "same side of the wall: counted"
    );
    // One on the far side does not, even though the tag is room-wide.
    assert_eq!(
        goal((26, 40), false),
        "nil",
        "walled off: not counted, with no chamber authored"
    );
    // Open a single gap and it becomes reachable, so it counts again — a rectangle
    // could not express that, and a switch-operated wall gets it for free.
    assert_eq!(
        goal((26, 40), true),
        "26,40",
        "one gap in the wall is enough"
    );
}

#[test]
fn alttp_an_enemy_on_the_other_floor_is_not_something_link_can_fight() {
    // A two-floor room has two collision grids sharing one set of tile coordinates, so
    // an enemy directly above or below Link sits at the same tx,ty he does. The
    // reachability fill is built for HIS floor, and the comparison was purely positional,
    // so the enemy read as standing next to him. Sprites carry their floor at $7E0F20.
    let r = Registry::builtin();
    let goal = |enemy_floor: u8| -> String {
        let mut ram = dungeon_frame((20, 20), (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x55);
            set(0x7E00AE, 0x0A); // room-wide kill tag, so only the floor can exclude it
            set(0x7E00EE, 0); // Link on the upper floor
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3C5, 2);
            set(0x7E0F20, enemy_floor); // slot 0's floor
        }
        sprite_slot(&mut ram, 0, 66, (26, 20), 4);
        // sprite_slot must not clobber the floor byte, so set it after.
        ram[wram_offset(0x7E0F20).unwrap()] = enemy_floor;
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("advance", &ram);
        p.on_frame(&ram, 2);
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram,
        )
        .unwrap()
    };

    assert_eq!(goal(0), "26,20", "same floor as Link: a target");
    assert_eq!(
        goal(1),
        "nil",
        "the floor below: not something he can fight from here"
    );
    assert_eq!(
        goal(2),
        "26,20",
        "floor 2 is the transient the explosion path sets: counted"
    );
}
