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

    // Guard gone and the room quiet long enough to believe it: the chest sub-goal announces
    // and takes over. It does not pounce on the first quiet frame, because a blinking sprite
    // slot produces those in the middle of a fight.
    let clear = frame(false);
    let texts2 = settle_quiet(&mut plugin, &clear, 3);
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
fn alttp_room_0x80_counts_the_far_key_carrier_and_ignores_hp0_bystanders() {
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
        // It is the big-key carrier, which is what room 0x80's authored step names:
        // die_action non-zero marks a guard that still drops a key on death.
        set(0x7E0CBA + 1, if enemy_alive { 0x02 } else { 0x00 });
        ram
    };

    // Trooper alive in the far east: room 0x80's step is the whole room, bounded only by
    // what Link can reach, so it catches the trooper even though Zelda (hp 0) sits nearer.
    let live = frame(true);
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    plugin.on_frame(&live, 0);
    plugin.on_frame(&live, 1);
    plugin.command("advance", &live);
    let out = plugin.on_frame(&live, 2);
    let texts: Vec<String> = out.iter().map(|i| i.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Defeat the enemy holding the key")),
        "the far eastern key carrier is the step's target: {texts:?}"
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
        !texts2.iter().any(|t| t.contains("Defeat")),
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
    // This room's kill tag never clears while its far pit is uncleared, so the chest becomes
    // available on the room being QUIET rather than on the tag — after QUIET_FRAMES of nothing
    // reachable to fight.
    let texts = settle_quiet(&mut plugin, &near, 2);
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
/// Frames a room must go without anything to fight before an objective may treat it as quiet —
/// QUIET.NEEDED in the plugin. A sprite slot blinks, so no single frame is evidence either way,
/// and the chest objective waits this long rather than pouncing on one quiet frame.
const QUIET_FRAMES: u64 = 20;

/// Runs `ram` until the room has been quiet long enough for that to count, returning what was
/// said along the way.
fn settle_quiet(p: &mut LuaPlugin, ram: &[u8], from: u64) -> Vec<String> {
    let mut out = Vec::new();
    for f in 0..=QUIET_FRAMES {
        out.extend(p.on_frame(ram, from + f).iter().map(|i| i.text.clone()));
    }
    out
}

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
                -- Non-empty rather than exact counts: the counts churn every time a
                -- step is authored, and what this test is about is that the chains
                -- arrive whole with their prose and their clauses compiled.
                tostring(#WAYPOINTS.UNCLE_APPROACH > 0 and #c > 0 and #sanct > 0),
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
        "true|string|string|function|gate|push|nil|function|nil",
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

/// The `i`th tile the pit scan probes ahead of Link, by the plugin's own arithmetic:
/// x + 8 is his centre column and y + 12 his feet, then i steps of 8 pixels in the
/// facing direction. Computed rather than guessed, because the row it walks is not the
/// row Link's tile is on.
fn scanned_tile(link: (u16, u16), dir: u8, i: i32) -> (u16, u16) {
    let (x, y) = ((link.0 * 8 + 4) as i32, (link.1 * 8 + 4) as i32);
    let dx = if dir == 4 {
        -8
    } else if dir == 6 {
        8
    } else {
        0
    };
    let dy = if dir == 0 {
        -8
    } else if dir == 2 {
        8
    } else {
        0
    };
    (
        ((x + 8 + dx * i) >> 3) as u16,
        ((y + 12 + dy * i) >> 3) as u16,
    )
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
fn alttp_facing_a_pit_blips_once_and_says_nothing() {
    // A sharp quick notification, not a word and not a drone: it fires on turning onto
    // the pit, pans to say which way the edge is, and stops, so it does not bury the
    // enemy and guide tones while Link edges along a ledge.
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
        hazard.ping,
        "a sharp attack-decay strike, not a swell: {hazard:?}"
    );
    assert!(hazard.tremolo >= 6.0, "and a quick one: {hazard:?}");
    // Above every object tone (0.7 to 2.0) and above the guide's sonar (2.4 to 3.5), so
    // there is nothing in the vocabulary it can be mistaken for.
    assert!(
        hazard.pitch > 3.5,
        "pitched clear of everything else: {hazard:?}"
    );
    assert!(
        hazard.dy > 0.0,
        "panned south, where the pit is: {hazard:?}"
    );

    // And it stops on its own while Link is still facing the pit — the property a
    // sustained tone could not have, and the reason it does not bury the other tones
    // while he edges along a ledge.
    for f in 2..12 {
        p.on_frame(&ram, f);
    }
    assert!(
        !p.beacons().iter().any(|b| b.id == "hazard"),
        "the blip is brief: it does not hold while he keeps facing it"
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

#[test]
fn alttp_a_pit_is_flagged_from_several_tiles_off_but_not_through_a_wall() {
    // One tile of look-ahead gave about a frame of warning at walking speed. The scan
    // reaches further now, so the blip lands a couple of steps before the edge — and
    // stops at anything solid, since a pit behind a wall is not a step Link can take.
    let r = Registry::builtin();
    let sounds = |pit_tiles_ahead: u16, wall_at: Option<u16>| -> bool {
        // Facing east from (20,20), painting on the tiles the scan actually probes.
        let pit = scanned_tile((20, 20), 6, pit_tiles_ahead as i32);
        let mut tiles = vec![(pit.0, pit.1, 0x20u8)];
        if let Some(w) = wall_at {
            let wall = scanned_tile((20, 20), 6, w as i32);
            tiles.push((wall.0, wall.1, 0x01));
        }
        let ram = facing_frame((20, 20), 6, &tiles);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.beacons().iter().any(|b| b.id == "hazard")
    };

    assert!(sounds(1, None), "a pit one tile ahead sounds");
    assert!(sounds(4, None), "and one four tiles ahead does too");
    assert!(!sounds(20, None), "one far across the room does not");
    // A wall between Link and the pit stops the scan.
    assert!(
        !sounds(4, Some(2)),
        "a pit behind a wall is not a step he can take, so it is not flagged"
    );
    assert!(sounds(4, Some(6)), "a wall beyond the pit does not hide it");
}

#[test]
fn alttp_the_blip_pans_further_the_further_off_the_pit_is() {
    // It is positioned on the pit rather than on Link, so how far ahead the edge lies is
    // audible in the offset instead of needing to be described.
    let r = Registry::builtin();
    let offset = |ahead: u16| -> f32 {
        let pit = scanned_tile((20, 20), 6, ahead as i32);
        let ram = facing_frame((20, 20), 6, &[(pit.0, pit.1, 0x20)]);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.beacons()
            .iter()
            .find(|b| b.id == "hazard")
            .expect("sounding")
            .dx
    };
    assert!(offset(4) > offset(1), "further pit, wider offset");
}

#[test]
fn alttp_a_room_objective_is_stated_once_however_much_its_target_flickers() {
    // Reported as a room spamming "defeat all enemies". The spoken latch and the
    // "an objective is active" trace were one field, so a single frame with no
    // countable target cleared it and the next frame said the cue again. A moving enemy
    // crossing the reachable boundary, or a sprite slot blinking as it dies, is enough.
    let r = Registry::builtin();
    let frame = |enemy: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x55);
            set(0x7E00AE, 0x0A); // room-wide kill tag
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3C5, 2);
        }
        if enemy {
            sprite_slot(&mut ram, 0, 66, (26, 20), 4);
        }
        ram
    };

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let live = frame(true);
    let gone = frame(false);
    p.on_frame(&live, 0);
    p.on_frame(&live, 1);
    p.command("advance", &live);

    let mut said = 0;
    for f in 2..20 {
        // Alternate: the enemy is countable on even frames and not on odd ones.
        let ram = if f % 2 == 0 { &live } else { &gone };
        said += p
            .on_frame(ram, f)
            .iter()
            .filter(|i| i.text.contains("Defeat all enemies"))
            .count();
    }
    assert_eq!(
        said, 1,
        "stated once across nine appearances, not nine times"
    );
}

#[test]
fn alttp_turning_navigation_on_says_so() {
    // It always said "Navigation off." on the way off, and nothing on the way on, so the
    // key gave no answer about which it had just done.
    let r = Registry::builtin();
    let ram = dungeon_frame((20, 20), (20, 6), &[]);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);

    let on: Vec<String> = p
        .command("advance", &ram)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        on.iter().any(|t| t.contains("Navigation on")),
        "turning it on says so: {on:?}"
    );
    let off: Vec<String> = p
        .command("advance", &ram)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        off.iter().any(|t| t.contains("Navigation off")),
        "and off still does: {off:?}"
    );
}

#[test]
fn alttp_the_auto_start_in_the_house_says_so_and_still_cues_the_beat() {
    // Navigation turns itself on at the opening, and that was silent — the player had no
    // way to know it was on. Saying so must not cost the beat cue that follows it: the
    // first attempt called nav_say from above its own declaration, which errored and took
    // the whole frame's speech with it.
    let r = Registry::builtin();
    let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x04); // room 0x0104: Link's house
        set(0x7E00A1, 0x01);
        set(0x7EF34A, 0); // Lamp not taken yet
    }
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    let texts: Vec<String> = p.on_frame(&ram, 1).iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("Navigation on")),
        "the auto-start announces itself: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.to_lowercase().contains("lantern")),
        "and the beat cue still arrives: {texts:?}"
    );
}

#[test]
fn alttp_a_via_step_holds_the_scan_even_after_it_briefly_read_as_done() {
    // Reported in room 0x80: the guide went straight to Zelda's cell past a
    // Ball-and-Chain Trooper carrying the key. Entering a room, its enemies have not
    // spawned yet, so a `via` fight step reads done for a frame or two — and the done
    // branch used to mark it ARRIVED, which is permanent. Once latched, `via` could never
    // hold the scan again and the later same-room waypoint won every time.
    let r = Registry::builtin();
    let frame = |enemy: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((12, 529), (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x80);
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3CC, 0);
            set(0x7EF3C5, 0); // COURTYARD armed
        }
        if enemy {
            // A key-carrying trooper, as the room really holds.
            sprite_slot(&mut ram, 2, 106, (52, 530), 16);
            ram[wram_offset(0x7E0CBA + 2).unwrap()] = 0x02; // die_action: drops a key
        }
        ram
    };

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    // Arrive with nothing spawned: the fight step reads done for these frames.
    let empty = frame(false);
    p.on_frame(&empty, 0);
    p.on_frame(&empty, 1);
    p.command("advance", &empty);
    p.on_frame(&empty, 2);

    // Now the trooper loads. The fight step must take the route back — after the
    // driver's re-probe throttle expires, since it keeps following a still-valid route
    // rather than re-picking every frame.
    let live = frame(true);
    for f in 3..20 {
        p.on_frame(&live, f);
    }
    assert_eq!(
        p.eval(PICKED, &live).unwrap(),
        "80,nil,nil",
        "the fight step holds the scan; it must not have latched as arrived"
    );
    assert_eq!(
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &live
        )
        .unwrap(),
        "52,530",
        "and it leads to the enemy carrying the key, not on to the cell"
    );
}

#[test]
fn alttp_room_0x21_leads_to_the_rat_carrying_the_key() {
    // The escape's locked door north out of 0x21 needs a key, and the key is on one rat
    // among nine sprites — eight other rats and three Keese share the room. `carries =
    // "key"` picks it out by die_action, which is the only thing that distinguishes it.
    //
    // Authoring the step also widens the area it is judged over: with no fight step in
    // the room there is no kill tag either, so the tally fell back to a 144-pixel radius
    // and the rat 264 pixels away did not count at all.
    let r = Registry::builtin();
    let mut ram = dungeon_frame((107, 175), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x21);
        set(0x7E00EE, 0);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3CC, 1); // Zelda following: the escape, so SANCTUARY is armed
        set(0x7EF3C5, 1);
    }
    // The key carrier, and a decoy rat nearer to Link that drops nothing.
    sprite_slot(&mut ram, 0, 109, (74, 140), 2);
    ram[wram_offset(0x7E0CBA).unwrap()] = 0x01;
    sprite_slot(&mut ram, 3, 109, (98, 172), 2); // die_action stays 0

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    for f in 2..20 {
        p.on_frame(&ram, f);
    }
    assert_eq!(
        p.eval(
            "return pathfind_goal and (pathfind_goal[1]..','..pathfind_goal[2]) or 'nil'",
            &ram
        )
        .unwrap(),
        "74,140",
        "the guide leads to the rat with the key, not the nearer one without it"
    );
}

// ── What Link is facing ─────────────────────────────────────────────────────
// The collision tells the router whether to go round something; it does not tell the
// player what to do with it. A bush needs slashing, a block shoving, a pot lifting.

#[test]
fn alttp_facing_a_block_says_so_and_a_pot_is_told_apart_from_it() {
    // Both are manipulable tiles in 0x70-0x7F, so the tile alone cannot say which. The
    // room's replacement-state slot does: 0 or 1 for a pushable block, 0x1111 for a pot,
    // 0x4040 for a hammer peg.
    let r = Registry::builtin();
    let said = |state: u16| -> Vec<String> {
        let e = faced_tile((20, 20), 6);
        let mut ram = facing_frame((20, 20), 6, &[(e.0, e.1, 0x71)]); // slot 1
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E0500 + 2, (state & 0xFF) as u8);
            set(0x7E0500 + 3, (state >> 8) as u8);
        }
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1).iter().map(|i| i.text.clone()).collect()
    };

    assert!(
        said(0).iter().any(|t| t == "Block."),
        "unshoved block: {:?}",
        said(0)
    );
    assert!(
        said(1).iter().any(|t| t == "Block."),
        "already shoved: still a block"
    );
    assert!(said(0x1111).iter().any(|t| t == "Pot."), "a pot is a pot");
    let peg = said(0x4040);
    assert!(
        !peg.iter().any(|t| t == "Block." || t == "Pot."),
        "a hammer peg is neither: {peg:?}"
    );
}

#[test]
fn alttp_a_faced_thing_is_named_once_and_renamed_when_it_changes() {
    let r = Registry::builtin();
    let e = faced_tile((20, 20), 6);
    let block = facing_frame((20, 20), 6, &[(e.0, e.1, 0x71)]); // state 0 -> a block
    let mut pot = facing_frame((20, 20), 6, &[(e.0, e.1, 0x71)]);
    {
        let mut set = |addr: u32, v: u8| pot[wram_offset(addr).unwrap()] = v;
        set(0x7E0500 + 2, 0x11);
        set(0x7E0500 + 3, 0x11);
    }
    let open = facing_frame((20, 20), 6, &[(e.0, e.1, 0x00)]);

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&block, 0);
    let first: Vec<String> = p
        .on_frame(&block, 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        first.iter().any(|t| t == "Block."),
        "named on facing it: {first:?}"
    );
    let again: Vec<String> = p
        .on_frame(&block, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        !again.iter().any(|t| t == "Block."),
        "not repeated: {again:?}"
    );

    // Facing a different thing names that instead, without needing open ground between.
    let switched: Vec<String> = p.on_frame(&pot, 3).iter().map(|i| i.text.clone()).collect();
    assert!(
        switched.iter().any(|t| t == "Pot."),
        "renamed on change: {switched:?}"
    );

    // Open ground re-arms it.
    p.on_frame(&open, 4);
    let back: Vec<String> = p
        .on_frame(&block, 5)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        back.iter().any(|t| t == "Block."),
        "named again after open ground: {back:?}"
    );
}

#[test]
fn alttp_a_shoved_block_is_still_called_a_block() {
    // Pushing a block clears its manipulable tile and leaves 0x27 — hookshottable — at
    // the new position. The push machinery rightly goes quiet, but the thing is still
    // standing there, and a player who cannot see it still wants to know.
    let r = Registry::builtin();
    let e = faced_tile((20, 20), 6);
    let ram = facing_frame((20, 20), 6, &[(e.0, e.1, 0x27)]);
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    let texts: Vec<String> = p.on_frame(&ram, 1).iter().map(|i| i.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t == "Block."),
        "a spent block still announces: {texts:?}"
    );
}

#[test]
fn alttp_the_escape_lever_waypoint_rides_the_good_switch_not_the_bad_one() {
    // Room 0x02 holds two levers: a Good Switch (sprite kind 4) and a Bad Switch (kind
    // 6). The waypoint names the sprite type rather than a tile, which is the only thing
    // that tells them apart, and offsets one tile south to where Link stands to pull it.
    let r = Registry::builtin();
    let mut ram = dungeon_frame((170, 52), (0, 0), &[]);
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x02);
        set(0x7E00EE, 1);
        set(0x7E040C, 0x02);
        set(0x7EF34A, 1);
        set(0x7EF359, 1);
        set(0x7EF3CC, 1); // Zelda following: SANCTUARY armed
        set(0x7EF3C5, 1);
        // The door the lever opens is still blocked, so the step is still wanted. A zero
        // here means the room has already recorded the lever pulled, and the step retires.
        set(0x7E0468, 1);
    }
    sprite_slot(&mut ram, 5, 6, (148, 46), 3); // Bad Switch, the decoy
    sprite_slot(&mut ram, 6, 4, (170, 46), 3); // Good Switch

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    for f in 2..20 {
        p.on_frame(&ram, f);
    }
    assert_eq!(
        p.eval(
            r#"local w = WAYPOINTS.SANCTUARY[22]
               return w.slot .. "@" .. w.tx .. "," .. w.ty"#,
            &ram
        )
        .unwrap(),
        "6@170,47",
        "it rides slot 6, the Good Switch, offset one tile south of it"
    );
    assert_eq!(
        p.eval(PICKED, &ram).unwrap(),
        "02,170,47",
        "and the guide is leading to it"
    );
}

#[test]
fn alttp_the_lever_step_retires_when_the_room_records_it_pulled() {
    // The lever's own sprite says nothing durable about having been pulled. The game
    // records it as a property of the ROOM: pulling a Good Switch sets a state-change
    // flag, and RoomTag_RoomTrigger_BlockDoor consumes it by lowering the door it was
    // blocking, clearing dung_flag_trapdoors_down. That flag is what the step reads.
    let r = Registry::builtin();
    let frame = |trapdoors: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((170, 52), (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x02);
            set(0x7E00AE, 0x14); // the block-door tag this room carries
            set(0x7E00EE, 1);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3CC, 1);
            set(0x7EF3C5, 1);
            set(0x7E0468, trapdoors); // 1 = door still blocked, 0 = lever pulled
        }
        sprite_slot(&mut ram, 6, 4, (170, 46), 3); // the Good Switch
        ram
    };
    let done = |trapdoors: u8| -> String {
        let ram = frame(trapdoors);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.eval(
            r#"local w = WAYPOINTS.SANCTUARY[22]
               return tostring(KIND.done({ module = 0x07, dungeon_room = 0x02 }, w))"#,
            &ram,
        )
        .unwrap()
    };

    assert_eq!(
        done(1),
        "false",
        "door still blocked: the lever wants pulling"
    );
    assert_eq!(done(0), "true", "room records it pulled: the step retires");
}

#[test]
fn alttp_the_doorway_the_lever_opens_is_gated_on_the_lever() {
    // The doorway south out of 0x02 only exists once the lever has been pulled, so aiming
    // at it before that would send Link into a wall. Same shape as a locked door's gate,
    // keyed on the room flag rather than on a key.
    let r = Registry::builtin();
    let gate = |trapdoors: u8| -> String {
        let mut ram = dungeon_frame((159, 52), (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x02);
            set(0x7E00EE, 1);
            set(0x7E0468, trapdoors);
        }
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.eval(
            r#"local w = WAYPOINTS.SANCTUARY[23]
               return tostring(w.gate({ module = 0x07, dungeon_room = 0x02 }, w))"#,
            &ram,
        )
        .unwrap()
    };
    assert_eq!(gate(1), "false", "door still blocked: not a target");
    assert_eq!(gate(0), "true", "lever pulled: the doorway opens up");

    // And the doorway must be walkable, or the route could not reach it. 0x86 is
    // TileHandlerIndoor_80, the indoor door collision, and is deliberately not in
    // IMPASSABLE — asked through REACH, which floods over passable ground only.
    let mut ram = dungeon_frame((159, 52), (0, 0), &[]);
    for ty in 56..59u32 {
        for tx in 159..161u32 {
            ram[wram_offset(0x7F3000 + (ty & 63) * 64 + (tx & 63)).unwrap()] = 0x86;
        }
    }
    ram[wram_offset(0x7E00EE).unwrap()] = 1;
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    assert_eq!(
        p.eval(
            r#"local s = { x = 159*8+4, y = 52*8+4, module = 0x07, dungeon_room = 0x02 }
               return tostring(REACH.can(s, 159*8+4, 57*8+4))"#,
            &ram
        )
        .unwrap(),
        "true",
        "a 0x86 doorway is walkable, so the route can reach a waypoint standing in it"
    );
}

#[test]
fn alttp_the_sanctuary_chest_speaks_its_arrival_line_on_reaching_it() {
    // The escort's parting line sits on the chest rather than the door, because the chest
    // has a `done` and so is errand-indexed: it stays reachable after the escort chain
    // retires at progress 2, where a plain position cannot be reached at all.
    //
    // Link has to ARRIVE for an arrival line. chain_start seeds nav_chain.arrived from
    // where he is standing, so a step he is already on when the chain arms counts as
    // reached and says nothing — which is why reloading the plugin while stood on a
    // waypoint never speaks it.
    let r = Registry::builtin();
    let frame = |link: (u16, u16)| -> Vec<u8> {
        let mut ram = dungeon_frame(link, (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x12); // the Sanctuary
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3CC, 1);
            set(0x7EF3C5, 1);
        }
        // Chest shut, or the step is done and never a target.
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                ram[wram_offset(0x7F2000 + ((74 + dy) & 63) * 64 + ((156 + dx) & 63)).unwrap()] =
                    0x58;
            }
        }
        ram
    };

    // Arm the chain well clear of the chest, so nothing is seeded as already reached.
    let away = frame((156, 84));
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&away, 0);
    p.on_frame(&away, 1);
    let mut said: Vec<String> = p
        .command("advance", &away)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    for f in 2..6 {
        said.extend(p.on_frame(&away, f).iter().map(|i| i.text.clone()));
    }
    assert!(
        !said.iter().any(|t| t.contains("excuuuuuse me")),
        "nothing yet: he has not reached it — {said:?}"
    );

    // Now walk him onto it.
    let on = frame((156, 74));
    for f in 6..14 {
        said.extend(p.on_frame(&on, f).iter().map(|i| i.text.clone()));
    }
    assert!(
        said.iter().any(|t| t.contains("excuuuuuse me")),
        "reaching the chest speaks its arrival line: {said:?}"
    );
    assert_eq!(
        said.iter().filter(|t| t.contains("excuuuuuse me")).count(),
        1,
        "and only once: {said:?}"
    );
}

#[test]
fn alttp_the_sanctuary_chest_comes_before_the_door() {
    // Ordering, not just presence: with the chest unopened the guide must be on the chest,
    // and once opened it retires and the door step takes over.
    let r = Registry::builtin();
    let frame = |chest_open: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((155, 78), (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x12);
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3CC, 1); // Zelda following: the escort chain is armed
            set(0x7EF3C5, 1);
        }
        if !chest_open {
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    ram[wram_offset(0x7F2000 + ((74 + dy) & 63) * 64 + ((156 + dx) & 63))
                        .unwrap()] = 0x58;
                }
            }
        }
        ram
    };
    let picked = |chest_open: bool| -> String {
        let ram = frame(chest_open);
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        p.on_frame(&ram, 0);
        p.on_frame(&ram, 1);
        p.command("advance", &ram);
        for f in 2..20 {
            p.on_frame(&ram, f);
        }
        p.eval(PICKED, &ram).unwrap()
    };

    assert_eq!(
        picked(false),
        "12,156,74",
        "chest shut: the guide is on the chest"
    );
    assert_eq!(
        picked(true),
        "12,159,120",
        "chest taken: the door step takes over"
    );
}

#[test]
fn alttp_a_quiet_step_keeps_its_kind_but_drops_its_cue() {
    // The Sanctuary chest is led to as part of the escort, and "Open the chest." there is
    // stating the obvious over the top of it. `quiet` drops the kind's cue while keeping
    // everything else the kind does — its done predicate, and so its place in the errand
    // index. Its own arrival line still speaks.
    let r = Registry::builtin();
    let frame = |link: (u16, u16)| -> Vec<u8> {
        let mut ram = dungeon_frame(link, (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x12);
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3CC, 1);
            set(0x7EF3C5, 1);
        }
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                ram[wram_offset(0x7F2000 + ((74 + dy) & 63) * 64 + ((156 + dx) & 63)).unwrap()] =
                    0x58;
            }
        }
        ram
    };

    let away = frame((156, 84));
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&away, 0);
    p.on_frame(&away, 1);
    let mut said: Vec<String> = p
        .command("advance", &away)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    for f in 2..8 {
        said.extend(p.on_frame(&away, f).iter().map(|i| i.text.clone()));
    }
    assert!(
        !said.iter().any(|t| t.contains("Open the chest")),
        "setting off toward it says nothing about chests: {said:?}"
    );
    // The kind is still a chest: it retires when the tile stops reading as one.
    assert_eq!(
        p.eval(
            r#"local w = WAYPOINTS.SANCTUARY[24]
               return tostring(w.kind) .. "," .. tostring(KIND.of(w).done ~= nil)"#,
            &away
        )
        .unwrap(),
        "chest,true",
        "quiet drops the cue, not the kind"
    );
}

#[test]
fn alttp_an_errand_speaks_its_arrival_line_with_no_chain_armed() {
    // The whole point of putting the line on the chest was that a step with a `done` is
    // errand-indexed and so reachable once its chain has retired. But the errand driver
    // had no arrival handling, so it led Link there and said nothing — reachable for
    // routing, silent on arrival.
    let r = Registry::builtin();
    let frame = |link: (u16, u16)| -> Vec<u8> {
        let mut ram = dungeon_frame(link, (0, 0), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7E00A0, 0x12);
            set(0x7E00EE, 0);
            set(0x7E040C, 0x02);
            set(0x7EF34A, 1);
            set(0x7EF359, 1);
            set(0x7EF3CC, 0); // Zelda delivered...
            set(0x7EF3C5, 2); // ...so the escort chain has retired
        }
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                ram[wram_offset(0x7F2000 + ((74 + dy) & 63) * 64 + ((156 + dx) & 63)).unwrap()] =
                    0x58; // chest still shut
            }
        }
        ram
    };

    let away = frame((156, 84));
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&away, 0);
    p.on_frame(&away, 1);
    let mut said: Vec<String> = p
        .command("advance", &away)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    for f in 2..8 {
        said.extend(p.on_frame(&away, f).iter().map(|i| i.text.clone()));
    }
    assert_eq!(
        p.eval("return tostring(nav_chain)", &away).unwrap(),
        "nil",
        "no chain is armed at this point in the quest"
    );
    assert!(
        !said.iter().any(|t| t.contains("excuuuuuse me")),
        "not yet: he has not reached it — {said:?}"
    );
    // And `quiet` holds on this path too, not just on the chain leg.
    assert!(
        !said.iter().any(|t| t.contains("Open the chest")),
        "a quiet step stays quiet whichever driver leads to it: {said:?}"
    );

    let on = frame((156, 74));
    for f in 8..16 {
        said.extend(p.on_frame(&on, f).iter().map(|i| i.text.clone()));
    }
    assert!(
        said.iter().any(|t| t.contains("excuuuuuse me")),
        "reaching an errand speaks its arrival line, chain or no chain: {said:?}"
    );
    assert_eq!(
        said.iter().filter(|t| t.contains("excuuuuuse me")).count(),
        1,
        "and once: {said:?}"
    );
}

#[test]
fn alttp_overworld_chain_waypoints_are_numbered_too() {
    // The dungeon overlay labels each step with its chain index; the overworld branch drew
    // the markers without numbers, so UNCLE_APPROACH gave position and nothing to look up
    // in the editor. Same numbers, same teal, so a number on either map means the same.
    let r = Registry::builtin();
    let mut ram = vec![0u8; 128 * 1024];
    {
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E0010, 0x09); // overworld
        set(0x7E001B, 0x00); // outdoors
        set(0x7EF36C, 24);
        set(0x7EF36D, 24);
        set(0x7E008A, 0x1B); // the castle area, where the uncle beat drives the chain
                             // Lamp taken (that beat done), sword NOT yet — which is the uncle beat, the one
                             // UNCLE_APPROACH is wired to. With the sword in hand the goal is complete and no
                             // chain arms at all.
        set(0x7EF34A, 1);
        set(0x7EF359, 0);
        // Link near the chain's first waypoint (280,316).
        let (lx, ly) = (282u16 * 8 + 4, 316u16 * 8 + 4);
        set(0x7E0022, (lx & 0xFF) as u8);
        set(0x7E0023, (lx >> 8) as u8);
        set(0x7E0020, (ly & 0xFF) as u8);
        set(0x7E0021, (ly >> 8) as u8);
    }

    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    p.on_frame(&ram, 0);
    p.on_frame(&ram, 1);
    p.command("advance", &ram);
    p.on_frame(&ram, 2);

    // Draw through a stub canvas so the labels can be inspected.
    let drawn = p
        .eval(
            r#"
              local seen = {}
              local stub = {
                width = 256, height = 256,
                clear = function() end, rect = function() end, line = function() end,
                text = function(self, x, y, s, color)
                  seen[#seen + 1] = s .. ":" .. string.format("%06X", color)
                end,
              }
              local ok, err = pcall(on_draw, stub, 0)
              if not ok then return "ERROR " .. tostring(err) end
              return table.concat(seen, " ")
            "#,
            &ram,
        )
        .unwrap();

    assert!(
        !drawn.starts_with("ERROR"),
        "the overworld map draws without error: {drawn}"
    );
    // At least one authored index, in the teal that means "look this up in the editor".
    assert!(
        drawn.split_whitespace().any(|t| t.ends_with(":20B0A0")),
        "an overworld chain waypoint is numbered in teal: {drawn}"
    );
    // And it drew with no computed route at all — this frame has no ROM, so the
    // overworld A* has nothing to decode. The chain used to be nested inside the
    // route block, so an unreachable target took the whole chain off the map.
    assert_eq!(
        p.eval("return tostring(ow_route_path)", &ram).unwrap(),
        "nil",
        "no route was computed, and the chain was drawn anyway"
    );
}

// ── Menus ───────────────────────────────────────────────────────────────────
// A menu is unusable unheard: the whole content of the screen is which option the
// cursor is on, and no tone can stand in for that.

/// A file-select frame: module 0x01 submodule 0x05, cursor at `at`, and `exists`
/// saying which of the three save files the screen found a signature for.
fn file_select_frame(at: u8, exists: [bool; 3]) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x01); // file select
    set(0x7E0011, 0x05); // its main submodule
    set(0x7E00C8, at); // the cursor
    for (k, &e) in exists.iter().enumerate() {
        set(0x7E00BF + k as u32 * 2, if e { 1 } else { 0 });
    }
    ram
}

#[test]
fn alttp_a_file_just_named_is_not_announced_as_empty() {
    // Finishing the name picker calls ReturnToFileSelect, which sets submodule 1 —
    // FileSelect_ReInitSaveFlagsAndEraseTriforce, which memsets the file-exists flags to
    // zero. Submodules 2-4 run, and only submodule 5's own handler puts them back from
    // SRAM. So the first submodule-5 frame the plugin sees still reads all files as empty,
    // and a reading latched there can never correct itself: the file the player has just
    // named is announced as empty until they move the cursor off it and back.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Naming file 1, then finishing.
    p.on_frame(&name_entry_at_slot(26, 0, 4), 0);
    p.on_frame(&name_entry_at_slot(26, 0, 4), 1);

    // Back at the file select, cursor on the file just named, flags not yet restored.
    let stale = name_in_save([false, false, false], [0x0B, 0x5F, 0x0D, 0x0A, 0x59, 0x59]);
    let mut said: Vec<String> = p
        .on_frame(&stale, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();

    // FileSelect_Main has now run, so the flags are back and the name is there.
    let settled = name_in_save([true, false, false], [0x0B, 0x5F, 0x0D, 0x0A, 0x59, 0x59]);
    for f in 3..7 {
        said.extend(p.on_frame(&settled, f).iter().map(|i| i.text.clone()));
    }

    assert!(
        !said.iter().any(|t| t == "File 1, empty"),
        "the file was just named, so it is never empty: {said:?}"
    );
    assert!(
        said.iter().any(|t| t == "File 1, LINK"),
        "and its name is what gets read: {said:?}"
    );
}

/// Everything said over `n` frames of one unchanging RAM image, from a fresh plugin.
///
/// Needs more than two: the first on_frame returns early for want of a previous state, and
/// the file select then spends MENU.SETTLE frames waiting for FileSelect_Main to put the
/// file-exists flags and names in place before it trusts what it reads.
fn speaks_over(r: &Registry, ram: &[u8], n: u64) -> Vec<String> {
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let mut out = Vec::new();
    for f in 0..n {
        out.extend(p.on_frame(ram, f).iter().map(|i| i.text.clone()));
    }
    out
}

#[test]
fn alttp_the_file_select_reads_the_option_under_the_cursor() {
    let r = Registry::builtin();
    let read = |at: u8, exists: [bool; 3]| speaks_over(&r, &file_select_frame(at, exists), 4);

    assert!(read(0, [false; 3]).iter().any(|t| t == "File 1, empty"));
    assert!(read(1, [false; 3]).iter().any(|t| t == "File 2, empty"));
    assert!(read(2, [false; 3]).iter().any(|t| t == "File 3, empty"));
    // Cursor 3 enters the copy module and 4 the erase module (zelda3 FileSelect_Main).
    assert!(read(3, [false; 3]).iter().any(|t| t == "Copy"));
    assert!(read(4, [false; 3]).iter().any(|t| t == "Erase"));
    // A file the screen found a signature for is not "empty", even with no name decoded.
    let occupied = read(0, [true, false, false]);
    assert!(
        occupied.iter().any(|t| t == "File 1"),
        "an existing file drops the empty qualifier: {occupied:?}"
    );
}

#[test]
fn alttp_a_menu_option_is_read_once_and_again_when_the_cursor_moves() {
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let first = file_select_frame(0, [false; 3]);
    let second = file_select_frame(1, [false; 3]);
    // Frame 0 returns early for want of a previous state, frame 1 is the settle frame the
    // file select spends waiting for FileSelect_Main, so frame 2 is the first that speaks.
    p.on_frame(&first, 0);
    p.on_frame(&first, 1);
    let said: Vec<String> = p
        .on_frame(&first, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(said.iter().any(|t| t == "File 1, empty"), "{said:?}");

    // Held still, it does not repeat.
    let again: Vec<String> = p
        .on_frame(&first, 3)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        !again.iter().any(|t| t.starts_with("File")),
        "not repeated: {again:?}"
    );

    // Moved, it reads the new option.
    let moved: Vec<String> = p
        .on_frame(&second, 4)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(moved.iter().any(|t| t == "File 2, empty"), "{moved:?}");

    // Critical, so a menu is never lost to the verbosity gate.
    let one = p.on_frame(&file_select_frame(3, [false; 3]), 4);
    assert!(
        one.iter()
            .any(|i| i.text == "Copy" && format!("{:?}", i.priority) == "Critical"),
        "menus outrank the verbosity gate: {:?}",
        one.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
}

/// A name-entry frame: module 0x04 submodule 0x03, picker cursor at (col, row).
fn name_entry_frame(col: u8, row: u8) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x04);
    set(0x7E0011, 0x03);
    set(0x7E0B10, col); // selectfile_var3
    set(0x7E0B15, row); // selectfile_var5
    ram
}

/// Everything said while `ram` is shown for two frames from `frame`.
///
/// Two, because a page is spoken only once its text has held still for a frame: the message
/// buffer can be caught half written, and a half-written one differs from one frame to the next
/// where a finished one does not. Tests that walk pages by hand have to give each page both.
fn read_page(p: &mut LuaPlugin, ram: &[u8], frame: u64) -> Vec<String> {
    let mut out = Vec::new();
    for f in 0..2 {
        out.extend(p.on_frame(ram, frame + f).iter().map(|i| i.text.clone()));
    }
    out
}

/// Frames a fresh plugin needs before a screen's first reading is spoken.
///
/// Three, for two unrelated reasons. The first on_frame returns early for want of a previous
/// state, so the reader never runs. The next is the frame MENU.SETTLE spends waiting for the
/// screen's own submodule handler to have run — until it has, the file-exists flags and the
/// staged names still describe whatever was on screen before. The third is the one that
/// speaks — plus one more, because a page waits for its text to hold still. Any test that walks
/// frames by hand has to start after these.
const WARM: u64 = 4;

/// Run a plugin up to and including its first reading of `ram`, returning what it said.
fn warm(p: &mut LuaPlugin, ram: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for f in 0..WARM {
        out.extend(p.on_frame(ram, f).iter().map(|i| i.text.clone()));
    }
    out
}

/// The same screen with the name cursor (selectfile_var4) at a given slot. The game advances
/// that slot on every commit, which is how a selection is detected at all.
fn name_entry_at_slot(col: u8, row: u8, slot: u8) -> Vec<u8> {
    let mut ram = name_entry_frame(col, row);
    ram[wram_offset(0x7E0B12).unwrap()] = slot;
    ram
}

#[test]
fn alttp_selecting_a_character_speaks_it() {
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    // Sit on capital A (col 26, row 0) long enough for the cursor line to be said and latched.
    let holding = name_entry_at_slot(26, 0, 0);
    let arrived = warm(&mut p, &holding);
    assert!(
        arrived.iter().any(|t| t == "A"),
        "the cursor line is said: {arrived:?}"
    );
    let quiet: Vec<String> = p
        .on_frame(&holding, WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(quiet.is_empty(), "holding still is silent: {quiet:?}");

    // Pressing A types it and advances the name cursor 0 -> 1, without moving the grid.
    let typed: Vec<String> = p
        .on_frame(&name_entry_at_slot(26, 0, 1), WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        typed.iter().any(|t| t == "A"),
        "the typed character is spoken: {typed:?}"
    );

    // The grid cursor never moved, so its latch must be intact and moving off it still speaks.
    let moved: Vec<String> = p
        .on_frame(&name_entry_at_slot(27, 0, 1), WARM + 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        moved.iter().any(|t| t == "B"),
        "the cursor still speaks after a commit: {moved:?}"
    );
}

#[test]
fn alttp_the_name_cursor_controls_type_nothing() {
    // `back` and `forward` move the same slot a commit does, so a naive slot watch would
    // announce whatever letter the grid cursor happened to be resting on.
    let r = Registry::builtin();
    for (col, row, label) in [(0u8, 3u8, "forward"), (11, 3, "back")] {
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        let holding = name_entry_at_slot(col, row, 2);
        warm(&mut p, &holding);
        // Slot moves as the control does its job; nothing was typed, so nothing is said.
        let after: Vec<String> = p
            .on_frame(&name_entry_at_slot(col, row, 3), WARM)
            .iter()
            .map(|i| i.text.clone())
            .collect();
        assert!(after.is_empty(), "{label} types nothing: {after:?}");
    }
}

#[test]
fn alttp_re_entering_the_picker_does_not_speak_a_phantom_character() {
    // NameFile zeroes the slot on entry. Leaving with the cursor at slot 3 and coming back
    // reads as 3 -> 0, which is a change, and would announce a character nobody typed.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let inside = name_entry_at_slot(26, 0, 3);
    warm(&mut p, &inside);
    // Out to the file select, then back in with the slot reset and the cursor on a letter.
    // Each screen change costs its own settle frame, hence the frame walked past each way.
    let files = file_select_frame(0, [false, false, false]);
    p.on_frame(&files, WARM);
    p.on_frame(&files, WARM + 1);
    let inside = name_entry_at_slot(26, 0, 0);
    p.on_frame(&inside, WARM + 2);
    let back: Vec<String> = p
        .on_frame(&inside, WARM + 3)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        back.iter().any(|t| t == "A"),
        "re-entry announces the cursor: {back:?}"
    );

    // Both readings say "A", so the frame after is what tells them apart: the cursor line
    // sets the latch and falls quiet, while a commit returns before setting it and repeats.
    let held: Vec<String> = p
        .on_frame(&inside, WARM + 4)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        held.is_empty(),
        "that was the cursor, so holding still is silent: {held:?}"
    );
}

#[test]
fn alttp_the_name_picker_says_the_letter_under_the_cursor() {
    // The cell is REF.name_cells[col + row * 0x20], the same lookup the game does to
    // decide what a button press types. Keyed by position rather than glyph code, because
    // code 0x5F draws both capital I and lowercase l and only its place says which.
    let r = Registry::builtin();
    let says = |col: u8, row: u8| -> Vec<String> {
        let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
        warm(&mut p, &name_entry_frame(col, row))
    };

    // The three six-wide capital blocks, which the grid's own arithmetic predicted.
    assert!(says(26, 0).iter().any(|t| t == "A"), "{:?}", says(26, 0));
    assert!(says(31, 0).iter().any(|t| t == "F"));
    assert!(says(26, 1).iter().any(|t| t == "K"));
    assert!(says(31, 2).iter().any(|t| t == "Z"));
    // The gap-fillers at the left edge.
    assert!(says(0, 0).iter().any(|t| t == "G"));
    assert!(says(3, 1).iter().any(|t| t == "T"));
    // Lowercase and digits speak as themselves; punctuation is named instead, since a
    // synthesiser handed "!" says nothing — see alttp_punctuation_cells_are_spoken_by_name.
    assert!(says(6, 0).iter().any(|t| t == "a"));
    assert!(says(11, 2).iter().any(|t| t == "z"));
    assert!(says(18, 1).iter().any(|t| t == "5"));
    assert!(says(18, 2).iter().any(|t| t == "exclamation mark"));
    // The same glyph, read by position: capital I on row 0, lowercase l on row 1.
    assert!(says(2, 0).iter().any(|t| t == "I"), "{:?}", says(2, 0));
    assert!(says(7, 1).iter().any(|t| t == "l"), "{:?}", says(7, 1));
    // And the controls, named for what they do to the name cursor.
    assert!(says(0, 3).iter().any(|t| t == "forward"));
    assert!(says(11, 3).iter().any(|t| t == "back"));
    assert!(says(4, 0).iter().any(|t| t == "space"));
}

/// Picking grid cell `t` does not store `t`: the game stores this, so the grid's 0x00-0x0F
/// pass through unchanged while everything above them moves. Spelling a name through the
/// formula rather than with literals is the point of these tests — an earlier pair hard-coded
/// grid codes on both sides, so they agreed with a table keyed the same wrong way and passed
/// while the real game read "LINK" back as "LNK".
fn stored_code(grid: u16) -> u16 {
    (grid & 0xFFF0) * 2 + (grid & 0xF)
}

fn name_in_save(exists: [bool; 3], grid: [u16; 6]) -> Vec<u8> {
    let mut ram = file_select_frame(0, exists);
    for (i, code) in grid.iter().enumerate() {
        let t = stored_code(*code) + 0x1800;
        let at = 0x7E1002 + 8 + i as u32 * 2;
        ram[wram_offset(at).unwrap()] = (t & 0xFF) as u8;
        ram[wram_offset(at + 1).unwrap()] = (t >> 8) as u8;
    }
    ram
}

#[test]
fn alttp_a_stored_name_decodes_through_the_same_table() {
    // "LINK": grid L 0x0B, I 0x5F, N 0x0D, K 0x0A, then two blanks (0x59). Only I leaves the
    // identity range, which is exactly why the bug hid here — L, N and K decoded either way.
    let r = Registry::builtin();
    let ram = name_in_save([true, false, false], [0x0B, 0x5F, 0x0D, 0x0A, 0x59, 0x59]);
    let said = speaks_over(&r, &ram, 4);
    assert!(
        said.iter().any(|t| t == "File 1, LINK"),
        "the file's name is read with the option: {said:?}"
    );
}

#[test]
fn alttp_a_name_from_the_far_end_of_the_grid_decodes() {
    // Q-Z sit past the identity range too, so they went the same way as I. "ZELDA" spends
    // three of its five characters out there: Z 0x19, E 0x04, L 0x0B, D 0x03, A 0x00.
    let r = Registry::builtin();
    let ram = name_in_save([true, false, false], [0x19, 0x04, 0x0B, 0x03, 0x00, 0x59]);
    let said = speaks_over(&r, &ram, 4);
    assert!(
        said.iter().any(|t| t == "File 1, ZELDA"),
        "the file's name is read with the option: {said:?}"
    );
}

#[test]
fn alttp_punctuation_cells_are_spoken_by_name() {
    // A synthesiser renders "." or "-" as a pause or as nothing, so these cells were mute
    // while every letter spoke. Row 2 holds the whole punctuation block.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    for (col, row, word) in [
        (0u8, 2u8, "dash"),
        (1, 2, "period"),
        (2, 2, "comma"),
        (18, 2, "exclamation mark"),
        (19, 2, "question mark"),
        (20, 2, "open bracket"),
        (21, 2, "close bracket"),
    ] {
        let said = warm(&mut p, &name_entry_frame(col, row));
        assert!(
            said.iter().any(|t| t == word),
            "({col},{row}) -> {word}: {said:?}"
        );
    }
}

#[test]
fn alttp_moving_between_cells_that_read_alike_still_speaks() {
    // The picker has two `end` cells side by side and long runs of blanks. Latching on the
    // spoken text meant moving between them said nothing, which reads as the reader having
    // stopped working. The announcement is how the player learns the cursor moved, so it
    // follows the cursor.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    // Row 3 columns 2 and 3 are both "end"; columns 4 and 5 are both blank.
    let pairs = [((2u8, 3u8), (3u8, 3u8), "end"), ((4, 3), (5, 3), "space")];
    for (a, b, word) in pairs {
        let first = name_entry_frame(a.0, a.1);
        let second = name_entry_frame(b.0, b.1);
        let there = warm(&mut p, &first);
        assert!(there.iter().any(|t| t == word), "{word}: {there:?}");
        // Same word, different cell: it speaks again.
        let moved: Vec<String> = p
            .on_frame(&second, WARM)
            .iter()
            .map(|i| i.text.clone())
            .collect();
        assert!(
            moved.iter().any(|t| t == word),
            "moving to the neighbouring {word} cell speaks again: {moved:?}"
        );
        // Held still on that cell, it does not.
        let held: Vec<String> = p
            .on_frame(&second, WARM + 1)
            .iter()
            .map(|i| i.text.clone())
            .collect();
        assert!(
            !held.iter().any(|t| t == word),
            "holding still does not repeat: {held:?}"
        );
    }
}

/// The copy-file screen: module 0x02, one cursor at $7E00C8 shared by its three submodules.
///
/// `names` are staged into the VRAM upload buffer at the offsets that submodule uses, keyed
/// by row rather than by file, since the target screen shows only the two non-source files.
fn copy_frame(sub: u8, at: u8, exists: [bool; 3], names: &[(u32, [u16; 6])]) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x02);
    set(0x7E0011, sub);
    set(0x7E00C8, at); // selectfile_R16
    for (k, &e) in exists.iter().enumerate() {
        set(0x7E00BF + k as u32 * 2, if e { 1 } else { 0 });
    }
    for (off, name) in names {
        for (i, code) in name.iter().enumerate() {
            let t = stored_code(*code) + 0x1800;
            let at = 0x7E1002 + off + i as u32 * 2;
            set(at, (t & 0xFF) as u8);
            set(at + 1, (t >> 8) as u8);
        }
    }
    ram
}

/// LINK, ZELDA: six grid codes each, blanks padding to the fixed six characters.
const LINK: [u16; 6] = [0x0B, 0x5F, 0x0D, 0x0A, 0x59, 0x59];
const ZELDA: [u16; 6] = [0x19, 0x04, 0x0B, 0x03, 0x00, 0x59];

#[test]
fn alttp_the_copy_screen_reads_the_source_and_quit() {
    // Submodule 3 picks a source: cursor 0-2 are the files, 3 is Quit. Names are staged at
    // 0x3C/0x64/0x8C (CopyFile_SelectionAndBlinker's own Dst, not the file select's).
    let r = Registry::builtin();
    let names = [(0x3C, LINK), (0x64, ZELDA)];
    let read = |at: u8| speaks_over(&r, &copy_frame(0x03, at, [true, true, false], &names), 4);

    let first = read(0);
    assert!(first.iter().any(|t| t == "File 1, LINK"), "{first:?}");
    let second = read(1);
    assert!(second.iter().any(|t| t == "File 2, ZELDA"), "{second:?}");
    let quit = read(3);
    assert!(
        quit.iter().any(|t| t == "Quit"),
        "cursor 3 is Quit: {quit:?}"
    );
}

#[test]
fn alttp_the_copy_screen_reads_the_target_rows_and_quit() {
    // Submodule 4 picks a target: cursor 0-1 are rows, 2 is Quit. A row is not a file — the
    // two candidates live in selectfile_arr2 ($7E00CA) as file * 2, so with file 1 as the
    // source the rows are files 2 and 3. Row names are staged at 0x38+4 and 0x60+4.
    let r = Registry::builtin();
    let names = [(0x38 + 4, ZELDA)];
    let frame = |at: u8| {
        let mut ram = copy_frame(0x04, at, [true, true, false], &names);
        ram[wram_offset(0x7E00CA).unwrap()] = 2; // row 0 -> file 2
        ram[wram_offset(0x7E00CB).unwrap()] = 4; // row 1 -> file 3
        ram
    };

    // Row 0 is file 2, which exists and is named — so the row index is not what gets said.
    let row0 = speaks_over(&r, &frame(0), 4);
    assert!(row0.iter().any(|t| t == "File 2, ZELDA"), "{row0:?}");
    // Row 1 is file 3, which does not exist. Copying into an empty slot is the normal case.
    let row1 = speaks_over(&r, &frame(1), 4);
    assert!(row1.iter().any(|t| t == "File 3, empty"), "{row1:?}");
    let quit = speaks_over(&r, &frame(2), 4);
    assert!(
        quit.iter().any(|t| t == "Quit"),
        "cursor 2 is Quit: {quit:?}"
    );
}

#[test]
fn alttp_the_copy_screen_reads_its_confirmation() {
    // Submodule 5: cursor 0 does the copy, 1 backs out. "COPY OK" is the game's own wording,
    // decoded from the tiles CopyFile_TargetSelectionAndBlink stages for that line.
    let r = Registry::builtin();
    let ok = speaks_over(&r, &copy_frame(0x05, 0, [true, false, false], &[]), 4);
    assert!(ok.iter().any(|t| t == "Copy OK"), "{ok:?}");
    let quit = speaks_over(&r, &copy_frame(0x05, 1, [true, false, false], &[]), 4);
    assert!(quit.iter().any(|t| t == "Quit"), "{quit:?}");
}

#[test]
fn alttp_the_copy_screen_speaks_again_when_the_cursor_moves() {
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let names = [(0x3C, LINK), (0x64, ZELDA)];
    let first = copy_frame(0x03, 0, [true, true, false], &names);
    let arrived = warm(&mut p, &first);
    assert!(arrived.iter().any(|t| t == "File 1, LINK"), "{arrived:?}");

    let held: Vec<String> = p
        .on_frame(&first, WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(held.is_empty(), "holding still is silent: {held:?}");

    // Down to Quit. The cursor key carries the submodule too, so the same cursor value on
    // the source and target screens cannot be mistaken for holding still.
    let moved: Vec<String> = p
        .on_frame(&copy_frame(0x03, 3, [true, true, false], &names), WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(moved.iter().any(|t| t == "Quit"), "{moved:?}");
}

/// The erase screen: module 0x03, cursor at $7E00C8, names staged where the file select
/// stages them because it is the same routine (SelectFile_Func17) that puts them there.
fn erase_frame(sub: u8, at: u8, exists: [bool; 3], names: &[(u32, [u16; 6])]) -> Vec<u8> {
    let mut ram = copy_frame(sub, at, exists, names);
    ram[wram_offset(0x7E0010).unwrap()] = 0x03;
    ram
}

#[test]
fn alttp_the_erase_screen_reads_the_files_and_quit() {
    let r = Registry::builtin();
    // Offsets 8 and 0x5C are the file select's, which this screen shares.
    let names = [(8, LINK), (0x5C, ZELDA)];
    let read = |at: u8| speaks_over(&r, &erase_frame(0x03, at, [true, true, false], &names), 4);

    let first = read(0);
    assert!(first.iter().any(|t| t == "File 1, LINK"), "{first:?}");
    let second = read(1);
    assert!(second.iter().any(|t| t == "File 2, ZELDA"), "{second:?}");
    let third = read(2);
    assert!(third.iter().any(|t| t == "File 3, empty"), "{third:?}");
    let quit = read(3);
    assert!(
        quit.iter().any(|t| t == "Quit"),
        "cursor 3 is Quit: {quit:?}"
    );
}

#[test]
fn alttp_the_erase_confirmation_names_the_file_it_will_destroy() {
    // The confirmation replaces the upload buffer with its own prompt and never re-stages a
    // name, so the name has to be remembered from the screen before. Which file it is comes
    // from subsubmodule_index ($7E00B0), not from the cursor, which is now the yes/no.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let names = [(8, LINK), (0x5C, ZELDA)];

    // Pick file 2 on the selection screen, so its name is read once while it is readable.
    let picking = erase_frame(0x03, 1, [true, true, false], &names);
    let seen = warm(&mut p, &picking);
    assert!(seen.iter().any(|t| t == "File 2, ZELDA"), "{seen:?}");

    // Now the confirmation, with no name staged anywhere and the chosen file in $7E00B0.
    let mut confirm = erase_frame(0x04, 0, [true, true, false], &[]);
    confirm[wram_offset(0x7E00B0).unwrap()] = 1;
    p.on_frame(&confirm, WARM);
    let asked: Vec<String> = p
        .on_frame(&confirm, WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        asked
            .iter()
            .any(|t| t == "Erase this player: File 2, ZELDA"),
        "the confirmation says which save: {asked:?}"
    );

    // The other option backs out.
    let mut quit_frame = erase_frame(0x04, 1, [true, true, false], &[]);
    quit_frame[wram_offset(0x7E00B0).unwrap()] = 1;
    let quit: Vec<String> = p
        .on_frame(&quit_frame, WARM + 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(quit.iter().any(|t| t == "Quit"), "{quit:?}");
}

#[test]
fn alttp_the_erase_confirmation_still_asks_when_no_name_was_seen() {
    // Reloading the plugin on the confirmation screen means nothing was ever remembered.
    // Better to ask without the name than to say nothing on an irreversible prompt.
    let r = Registry::builtin();
    let mut confirm = erase_frame(0x04, 0, [true, false, false], &[]);
    confirm[wram_offset(0x7E00B0).unwrap()] = 0;
    let asked = speaks_over(&r, &confirm, 4);
    assert!(
        asked.iter().any(|t| t == "Erase this player: File 1"),
        "{asked:?}"
    );
}

// ── Paginated game text ─────────────────────────────────────────────────────

const TEXT_WAITKEY: u8 = 0x7E;
const TEXT_SCROLL: u8 = 0x73;
const TEXT_END: u8 = 0x7F;

/// One character in the message encoding, which is not the name-picker's encoding.
fn text_byte(c: char) -> u8 {
    match c {
        'A'..='Z' => c as u8 - b'A',
        'a'..='z' => c as u8 - b'a' + 0x1A,
        '0'..='9' => c as u8 - b'0' + 0x34,
        '!' => 0x3E,
        '.' => 0x41,
        ',' => 0x42,
        '?' => 0x3F,
        '>' => 0x44,
        '(' => 0x45,
        ')' => 0x46,
        '\'' => 0x50,
        ' ' => 0x59,
        _ => panic!("no encoding for {c:?}"),
    }
}

/// A message box mid-render: the pre-expanded buffer the game decodes from, and how far
/// RenderText_Draw_MessageCharacters has got through it.
fn dialog_frame(buf: &[u8], read_pos: u16) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x0E); // the text module
    set(0x7E1CF0, 0x1F); // dialogue_message_index
    set(0x7E1CD9, (read_pos & 0xFF) as u8); // dialogue_msg_read_pos
    set(0x7E1CDA, (read_pos >> 8) as u8);
    for (i, b) in buf.iter().enumerate() {
        set(0x7F1200 + i as u32, *b);
    }
    ram
}

/// Builds a buffer from pieces, returning it with the START offset of each piece — which is
/// what a test wants, since a break is a piece and read_pos rests exactly on one.
fn dialog_buffer(pieces: &[&str]) -> (Vec<u8>, Vec<u16>) {
    let mut buf = Vec::new();
    let mut starts = Vec::new();
    for piece in pieces {
        starts.push(buf.len() as u16);
        match *piece {
            "<wait>" => buf.push(TEXT_WAITKEY),
            "<scroll>" => buf.push(TEXT_SCROLL),
            "<end>" => buf.push(TEXT_END),
            s => buf.extend(s.chars().map(text_byte)),
        }
    }
    (buf, starts)
}

#[test]
fn alttp_a_paginated_message_is_read_one_page_at_a_time() {
    // Zelda's telepathic plea: five pages that turn on a button press. The whole message was
    // being read out the moment the box opened, which gives away pages not yet turned to.
    let (buf, starts) = dialog_buffer(&[
        "Help me!",
        "<wait>",
        "<scroll>",
        "I am a prisoner.",
        "<wait>",
        "<scroll>",
        "My name is Zelda.",
        "<end>",
    ]);
    let (first_wait, second_wait, end) = (starts[1], starts[4], starts[7]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // The renderer rests ON the Waitkey while it waits, so that position means page complete.
    let page1 = warm(&mut p, &dialog_frame(&buf, first_wait));
    assert!(page1.iter().any(|t| t == "Help me!"), "{page1:?}");
    assert!(
        !page1
            .iter()
            .any(|t| t.contains("prisoner") || t.contains("Zelda")),
        "no page the player has not turned to: {page1:?}"
    );

    // Held at the same break, it does not repeat.
    let held = read_page(&mut p, &dialog_frame(&buf, first_wait), WARM);
    assert!(held.is_empty(), "one announcement per page: {held:?}");

    let page2 = read_page(&mut p, &dialog_frame(&buf, second_wait), WARM + 1);
    assert!(page2.iter().any(|t| t == "I am a prisoner."), "{page2:?}");
    assert!(
        !page2
            .iter()
            .any(|t| t.contains("Help") || t.contains("Zelda")),
        "the page turned to, not the ones before or after: {page2:?}"
    );

    // The last page has no Waitkey after it; the terminator is its break.
    let page3 = read_page(&mut p, &dialog_frame(&buf, end), WARM + 2);
    assert!(page3.iter().any(|t| t == "My name is Zelda."), "{page3:?}");
}

#[test]
fn alttp_a_word_split_across_a_page_break_is_read_whole() {
    // A page can end mid-word. Reading only as far as the break would say half of it, so the
    // rest is taken across the boundary — and the next page then starts past it, rather than
    // beginning with a fragment already spoken.
    let (buf, starts) =
        dialog_buffer(&["I am in the dunge", "<wait>", "on of the castle.", "<end>"]);
    let (wait, end) = (starts[1], starts[3]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let page1 = warm(&mut p, &dialog_frame(&buf, wait));
    assert!(
        page1.iter().any(|t| t == "I am in the dungeon"),
        "the word is finished across the break: {page1:?}"
    );

    let page2 = read_page(&mut p, &dialog_frame(&buf, end), WARM);
    assert!(
        page2.iter().any(|t| t == "of the castle."),
        "and is not said again: {page2:?}"
    );
}

#[test]
fn alttp_a_break_on_a_word_boundary_reads_no_further() {
    // The other half of finishing a split word: when the break falls between words there is
    // nothing to finish, and reading on would give away the next page's first word.
    let (buf, starts) = dialog_buffer(&["Go ", "<wait>", "now!", "<end>"]);
    let wait = starts[1];

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let page1 = warm(&mut p, &dialog_frame(&buf, wait));
    assert!(page1.iter().any(|t| t == "Go"), "{page1:?}");
    assert!(
        !page1.iter().any(|t| t.contains("now")),
        "the next page's word is not given away: {page1:?}"
    );
}

#[test]
fn alttp_a_page_is_read_as_soon_as_it_starts_appearing() {
    // Not once it has finished: waiting out the typewriter means waiting to hear a word of a
    // page that is sitting in the buffer whole already. read_pos here is five bytes into an
    // eight-byte page, and the whole page is spoken.
    let (buf, _) = dialog_buffer(&["Help me!", "<wait>", "<end>"]);
    let r = Registry::builtin();
    let drawing = speaks_over(&r, &dialog_frame(&buf, 5), 4);
    assert!(
        drawing.iter().any(|t| t == "Help me!"),
        "the page is read while it is still drawing: {drawing:?}"
    );
}

#[test]
fn alttp_nothing_is_read_before_a_page_has_begun() {
    // read_pos at 0 is the box opening with nothing drawn yet. The page is spoken once the
    // renderer has moved into it, not before.
    let (buf, _) = dialog_buffer(&["Help me!", "<wait>", "<end>"]);
    let r = Registry::builtin();
    let opening = speaks_over(&r, &dialog_frame(&buf, 0), 4);
    assert!(
        !opening.iter().any(|t| t.contains("Help")),
        "nothing before the page starts: {opening:?}"
    );
}

#[test]
fn alttp_reopening_a_box_starts_from_its_first_page() {
    // Text_LoadCharacterBuffer zeroes read_pos for every message, including the same message
    // shown twice, so a position behind where we had got to means a fresh message.
    let (buf, starts) = dialog_buffer(&["Help me!", "<wait>", "<scroll>", "Please!", "<end>"]);
    let (first_wait, end) = (starts[1], starts[4]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    warm(&mut p, &dialog_frame(&buf, first_wait));
    p.on_frame(&dialog_frame(&buf, end), WARM);

    // The box closes. Built from the same frame, because leaving a box does not clear the
    // message index or read_pos — only the module changes.
    let mut closed = dialog_frame(&buf, end);
    closed[wram_offset(0x7E0010).unwrap()] = 0x07;
    p.on_frame(&closed, WARM + 1);

    // And is opened again, which runs Text_Initialize and so Text_LoadCharacterBuffer: read_pos
    // back to 0 with the box up. That load is what earns the reader the right to read it.
    p.on_frame(&dialog_frame(&buf, 0), WARM + 2);
    let again = read_page(&mut p, &dialog_frame(&buf, first_wait), WARM + 3);
    assert!(
        again.iter().any(|t| t == "Help me!"),
        "the first page is read again: {again:?}"
    );
}

#[test]
fn alttp_a_page_ending_in_punctuation_does_not_swallow_the_next_word() {
    // The word-completion's limit. A page that ends on punctuation ended on a finished word,
    // so there is nothing to carry across — and carrying on regardless would say the first
    // word of a page the player has not turned to. Distinct from the space case: here there
    // is no line break after the Waitkey to stop the reader by accident.
    let (buf, starts) = dialog_buffer(&["Help me!", "<wait>", "Now go.", "<end>"]);
    let (wait, end) = (starts[1], starts[3]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let page1 = warm(&mut p, &dialog_frame(&buf, wait));
    assert!(
        page1.iter().any(|t| t == "Help me!"),
        "the page ends where its punctuation does: {page1:?}"
    );
    assert!(
        !page1.iter().any(|t| t.contains("Now")),
        "the next page's word is not swallowed: {page1:?}"
    );

    // And that word is still there to be read when the page is turned.
    let page2 = read_page(&mut p, &dialog_frame(&buf, end), WARM);
    assert!(page2.iter().any(|t| t == "Now go."), "{page2:?}");
}

#[test]
fn alttp_a_message_is_not_re_read_when_the_screen_comes_back() {
    // Reported: the opening text repeated after the lights came on. Leaving the text module
    // forgot how far we had got, so coming back with read_pos still at the end started again
    // from page one — and since read_pos was already past every break, each page satisfied
    // its trigger on the very next frame and the whole message replayed at once.
    let (buf, starts) = dialog_buffer(&[
        "Help me!",
        "<wait>",
        "<scroll>",
        "I am a prisoner.",
        "<wait>",
        "<scroll>",
        "My name is Zelda.",
        "<end>",
    ]);
    let (first_wait, second_wait, end) = (starts[1], starts[4], starts[7]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    // Every page genuinely read before the lights come on — a page needs its settle frame, and
    // giving it one frame and discarding the result would leave it unread while looking read.
    warm(&mut p, &dialog_frame(&buf, first_wait));
    let two = read_page(&mut p, &dialog_frame(&buf, second_wait), WARM);
    assert!(two.iter().any(|t| t == "I am a prisoner."), "{two:?}");
    let three = read_page(&mut p, &dialog_frame(&buf, end), WARM + 2);
    assert!(three.iter().any(|t| t == "My name is Zelda."), "{three:?}");

    // The lights go on: out of the text module and back. Built from the same frame so the
    // message index and read_pos persist across it, which is what the game does — only the
    // module changes.
    let mut lit = dialog_frame(&buf, end);
    lit[wram_offset(0x7E0010).unwrap()] = 0x07;
    p.on_frame(&lit, WARM + 4);
    let mut after = Vec::new();
    for f in 0..6 {
        after.extend(
            p.on_frame(&dialog_frame(&buf, end), WARM + 5 + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    // Nothing at all, not merely none of the earlier pages: the last page coming round again
    // is just as much a repeat, and checking only for the earlier ones let the whole fix rest
    // on knowing which page read_pos was in.
    assert!(
        after.is_empty(),
        "a message already read is not read again: {after:?}"
    );
}

#[test]
fn alttp_joining_a_message_part_way_reads_only_the_current_page() {
    // Reloading the plugin mid-scene, or arriving with the renderer already past earlier
    // pages: the page read_pos is inside is the one to read, not the message from the top.
    let (buf, starts) = dialog_buffer(&[
        "Help me!",
        "<wait>",
        "<scroll>",
        "I am a prisoner.",
        "<wait>",
        "<scroll>",
        "My name is Zelda.",
        "<end>",
    ]);
    let r = Registry::builtin();
    // read_pos inside the third page, the first two long since drawn.
    let joined = speaks_over(&r, &dialog_frame(&buf, starts[6] + 4), 4);
    assert!(
        joined.iter().any(|t| t == "My name is Zelda."),
        "the page it is on: {joined:?}"
    );
    assert!(
        !joined
            .iter()
            .any(|t| t.contains("Help") || t.contains("prisoner")),
        "not the pages before it: {joined:?}"
    );
}

/// The same frame with a different message index, for the window where the game has changed
/// which message it means but has not yet refilled the buffer.
fn with_msg_id(mut ram: Vec<u8>, id: u16) -> Vec<u8> {
    ram[wram_offset(0x7E1CF0).unwrap()] = (id & 0xFF) as u8;
    ram[wram_offset(0x7E1CF1).unwrap()] = (id >> 8) as u8;
    ram
}

#[test]
fn alttp_a_new_message_is_not_read_out_of_the_old_buffer() {
    // Reported: "Please help me" played before the uncle's line. dialogue_message_index changes
    // BEFORE Text_LoadCharacterBuffer refills the buffer and zeroes read_pos, so for a frame or
    // two the new id still describes the text just finished — and adopting the new id there read
    // the tail of the old message under it.
    let (zelda, zstarts) =
        dialog_buffer(&["Help me!", "<wait>", "<scroll>", "Please help me!", "<end>"]);
    let (uncle, _) = dialog_buffer(&["Link, I am going out.", "<end>"]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    warm(&mut p, &dialog_frame(&zelda, zstarts[1]));
    let ended = read_page(&mut p, &dialog_frame(&zelda, zstarts[4]), WARM);
    assert!(ended.iter().any(|t| t == "Please help me!"), "{ended:?}");

    // The uncle's message is named, but the buffer and read_pos are still Zelda's.
    let mut stale = Vec::new();
    for f in 0..3 {
        let frame = with_msg_id(dialog_frame(&zelda, zstarts[4]), 0x2A);
        stale.extend(
            p.on_frame(&frame, WARM + 1 + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        stale.is_empty(),
        "nothing is read until the buffer holds the message named: {stale:?}"
    );

    // Now it is loaded: read_pos back near the start, the buffer the uncle's.
    let loaded = read_page(
        &mut p,
        &with_msg_id(dialog_frame(&uncle, 4), 0x2A),
        WARM + 5,
    );
    assert!(
        loaded.iter().any(|t| t == "Link, I am going out."),
        "and then it is read: {loaded:?}"
    );
    assert!(
        !loaded.iter().any(|t| t.contains("help")),
        "without the old message's tail: {loaded:?}"
    );
}

#[test]
fn alttp_a_box_opening_on_a_leftover_buffer_reads_nothing() {
    // With no box up, the buffer still holds the last message shown, and the plugin has no
    // rewind to tell it otherwise. So a plugin that starts outside a box must not read what it
    // finds there when one opens — it waits for the load it can actually see.
    let (old, _) = dialog_buffer(&["Link, I am going out.", "<end>"]);
    let (fresh, _) = dialog_buffer(&["It is dangerous to go alone.", "<end>"]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // In play, no box, the previous message still sitting in the buffer at its end.
    let mut idle = dialog_frame(&old, 21);
    idle[wram_offset(0x7E0010).unwrap()] = 0x07;
    for f in 0..3 {
        p.on_frame(&idle, f);
    }

    // A box opens for a new message: the index has changed, the buffer has not caught up.
    let mut opening = Vec::new();
    for f in 0..3 {
        let frame = with_msg_id(dialog_frame(&old, 21), 0x2A);
        opening.extend(p.on_frame(&frame, 3 + f).iter().map(|i| i.text.clone()));
    }
    assert!(
        opening.is_empty(),
        "the leftover buffer is not read: {opening:?}"
    );

    // Once the load is visible, the new message is read.
    let loaded = read_page(&mut p, &with_msg_id(dialog_frame(&fresh, 4), 0x2A), 7);
    assert!(
        loaded.iter().any(|t| t == "It is dangerous to go alone."),
        "{loaded:?}"
    );
}

#[test]
fn alttp_a_swapped_buffer_is_read_from_its_first_page_not_its_last() {
    // Reported from play: the map, the boomerang and the big key were all read BACK TO FRONT,
    // last page first, some with a page of nonsense ahead of them.
    //
    // Text_LoadCharacterBuffer fills the buffer BEFORE it zeroes read_pos, so there is a window
    // holding the new text at the old message's position. Reading from that position picks
    // whichever page it happens to land in — the last one — and if it lands beyond the new
    // message's terminator, a page of leftover bytes from the longer message before it.
    //
    // So a swap detected that way is not a load. There is nothing to read from it, and the reader
    // waits for read_pos to come back before deciding where it is.
    let (before, bstarts) = dialog_buffer(&["Help me!", "<wait>", "<scroll>", "Please!", "<end>"]);
    let (after, _) = dialog_buffer(&[
        "You got the Big Key! It can open many",
        "<wait>",
        "<scroll>",
        "locks that small keys cannot.",
        "<end>",
    ]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let said = warm(&mut p, &dialog_frame(&before, bstarts[1]));
    assert!(said.iter().any(|t| t == "Help me!"), "{said:?}");

    // The swap: a different buffer with read_pos still where the last message left it. Past the
    // recorded break, which is the case that bites — a position at it merely waits, whereas a
    // position beyond it looks like the page having been turned, so the reader would advance into
    // the new buffer at an offset that means nothing in it.
    let stale = bstarts[1] + 12;
    let mut during = Vec::new();
    for f in 0..3 {
        during.extend(
            p.on_frame(&dialog_frame(&after, stale), WARM + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        during.is_empty(),
        "a half-loaded buffer is not read at all: {during:?}"
    );

    // read_pos comes back, and the message is read from the top, in order.
    p.on_frame(&dialog_frame(&after, 0), WARM + 4);
    let mut heard = Vec::new();
    for f in 5..9 {
        heard.extend(
            p.on_frame(&dialog_frame(&after, 4), WARM + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        heard
            .iter()
            .any(|t| t == "You got the Big Key! It can open many"),
        "the first page is what gets read: {heard:?}"
    );
    assert!(
        !heard.iter().any(|t| t.contains("locks that small keys")),
        "and not the last page ahead of it: {heard:?}"
    );
}

#[test]
fn alttp_a_page_is_not_read_twice_when_the_box_reloads_under_it() {
    // Module0E_Interface runs Sprite_Main while a box is up, and sprite code resets
    // messaging_module, which sends the game back through Text_Initialize and so through
    // Text_LoadCharacterBuffer — the same message loaded again, read_pos back to 0, with the
    // box never having closed. Read as a fresh message that plays the text a second time,
    // which is what "You got the Lamp" doing it twice was.
    let (buf, _) = dialog_buffer(&["You got the Lamp!", "<end>"]);
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let first = warm(&mut p, &dialog_frame(&buf, 8));
    assert!(first.iter().any(|t| t == "You got the Lamp!"), "{first:?}");

    // Reloaded under the same open box: read_pos to 0, then drawing again.
    let mut again = Vec::new();
    p.on_frame(&dialog_frame(&buf, 0), WARM);
    for f in 1..5 {
        again.extend(
            p.on_frame(&dialog_frame(&buf, 2 + f as u16), WARM + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        again.is_empty(),
        "the same page is not read again: {again:?}"
    );
}

#[test]
fn alttp_a_box_shown_again_after_closing_is_read_again() {
    // The limit of that: once the box has closed, showing the message again is a new showing
    // and has to be read. Reading a sign twice should say it twice.
    let (buf, _) = dialog_buffer(&["You got the Lamp!", "<end>"]);
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    warm(&mut p, &dialog_frame(&buf, 8));

    // The box closes.
    let mut closed = dialog_frame(&buf, 8);
    closed[wram_offset(0x7E0010).unwrap()] = 0x07;
    p.on_frame(&closed, WARM);

    // And is opened again on the same message: a load, then drawing.
    p.on_frame(&dialog_frame(&buf, 0), WARM + 1);
    let mut reopened = Vec::new();
    for f in 2..6 {
        reopened.extend(
            p.on_frame(&dialog_frame(&buf, 2 + f as u16), WARM + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        reopened.iter().any(|t| t == "You got the Lamp!"),
        "a second showing is read: {reopened:?}"
    );
}

// ── The pause screen ────────────────────────────────────────────────────────

/// The item grid up and navigable: module 0x0E submodule 1 (Hud_Module_Run) with the HUD's own
/// state machine at 4 (Hud_NormalMenu). `owned` are the slot bytes from $7EF340 on.
fn pause_frame(slot: u8, dirs: u8, owned: &[(u8, u8)]) -> Vec<u8> {
    let mut ram = vec![0u8; 128 * 1024];
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7E0010, 0x0E);
    set(0x7E0011, 0x01);
    set(0x7E0200, 4); // overworld_map_state: Hud_NormalMenu
    set(0x7E0202, slot); // hud_cur_item
    set(0x7E00F4, dirs); // filtered_joypad_H
    for (n, v) in owned {
        set(0x7EF340 + *n as u32 - 1, *v);
    }
    ram
}

const UP: u8 = 0x08;
const DOWN: u8 = 0x04;
const LEFT: u8 = 0x02;
const RIGHT: u8 = 0x01;

#[test]
fn alttp_the_pause_screen_names_the_item_on_every_press() {
    // The case that prompted this: only the Lamp is owned, so no direction moves the cursor.
    // A reader that spoke on movement would say nothing at all, which is why this follows the
    // button rather than the cursor.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let lamp = &[(11u8, 1u8)][..];

    // Opening the menu says where the cursor already is, without a press.
    let opened = warm(&mut p, &pause_frame(11, 0, lamp));
    assert!(opened.iter().any(|t| t == "Lamp"), "{opened:?}");

    // Held still, it does not repeat.
    let still: Vec<String> = p
        .on_frame(&pause_frame(11, 0, lamp), WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(still.is_empty(), "holding still is silent: {still:?}");

    // Every direction says it again, though the cursor cannot move.
    let mut frame = WARM + 1;
    for (dir, label) in [(UP, "up"), (DOWN, "down"), (LEFT, "left"), (RIGHT, "right")] {
        let said: Vec<String> = p
            .on_frame(&pause_frame(11, dir, lamp), frame)
            .iter()
            .map(|i| i.text.clone())
            .collect();
        assert!(
            said.iter().any(|t| t == "Lamp"),
            "{label} re-reads it: {said:?}"
        );
        // The press ends; the direction bits clear.
        p.on_frame(&pause_frame(11, 0, lamp), frame + 1);
        frame += 2;
    }
}

#[test]
fn alttp_a_press_standing_for_two_frames_is_one_announcement() {
    // Edge-triggered on the direction bits appearing, so a press cannot be counted twice if
    // filtered_joypad_H is still standing on the next frame.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let lamp = &[(11u8, 1u8)][..];
    warm(&mut p, &pause_frame(11, 0, lamp));

    let first: Vec<String> = p
        .on_frame(&pause_frame(11, UP, lamp), WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(first.iter().any(|t| t == "Lamp"), "{first:?}");
    let second: Vec<String> = p
        .on_frame(&pause_frame(11, UP, lamp), WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(second.is_empty(), "one press, one announcement: {second:?}");
}

#[test]
fn alttp_the_pause_screen_names_each_slot_it_lands_on() {
    // Slot n is the byte at $7EF340 + n - 1, which is what Hud_DoWeHaveThisItem tests, so the
    // grid order is pinned rather than assumed. A few spread across it, including the corners.
    let r = Registry::builtin();
    let owned: Vec<(u8, u8)> = (1..=20).map(|n| (n, 1u8)).collect();
    let read = |slot: u8| speaks_over(&r, &pause_frame(slot, 0, &owned), 4);

    for (slot, name) in [
        (1u8, "Bow"),
        (4, "Bombs"),
        (11, "Lamp"),
        (12, "Hammer"),
        (15, "Book of Mudora"),
        (20, "Magic Mirror"),
    ] {
        let said = read(slot);
        assert!(
            said.iter().any(|t| t == name),
            "slot {slot} is {name}: {said:?}"
        );
    }
}

#[test]
fn alttp_a_shared_slot_is_named_by_what_is_in_it() {
    // Slot 13 holds the Shovel or the Flute, two unrelated items, and Hud_GetIconForItem tells
    // them apart by the slot's own value. Naming the slot instead of the item would be wrong
    // half the time here — and slot 5 is Mushroom before it is Magic Powder.
    let r = Registry::builtin();
    for (slot, value, name) in [
        (13u8, 1u8, "Shovel"),
        (13, 2, "Flute"),
        (5, 1, "Mushroom"),
        (5, 2, "Magic Powder"),
        (2, 1, "Boomerang"),
        (2, 2, "Red Boomerang"),
        (1, 3, "Silver Bow"),
    ] {
        let said = speaks_over(&r, &pause_frame(slot, 0, &[(slot, value)]), 4);
        assert!(
            said.iter().any(|t| t == name),
            "slot {slot} value {value} is {name}: {said:?}"
        );
    }
}

#[test]
fn alttp_the_pause_screen_is_not_read_while_it_is_being_built() {
    // hud_cur_item is not settled until Hud_Init has chosen a starting item, and the HUD spends
    // its earlier states building the grid. Only state 4 is the grid being navigated.
    let r = Registry::builtin();
    for state in [0u8, 1, 2, 3] {
        let mut ram = pause_frame(11, 0, &[(11, 1)]);
        ram[wram_offset(0x7E0200).unwrap()] = state;
        let said = speaks_over(&r, &ram, 4);
        assert!(
            !said.iter().any(|t| t == "Lamp"),
            "state {state} is not the grid: {said:?}"
        );
    }
}

#[test]
fn alttp_the_pause_screen_does_not_read_leftover_dialogue() {
    // The pause screen shares module 0x0E with the text box — kMessagingSubmodules dispatches on
    // submodule_index, 1 being the HUD and 2 the text. The message buffer still holds whatever
    // the last box left in it, so without excluding the HUD by name, opening the menu reads the
    // previous message aloud.
    let (buf, _) = dialog_buffer(&["You got the Lamp!", "<end>"]);
    let r = Registry::builtin();
    let mut ram = pause_frame(11, 0, &[(11, 1)]);
    for (i, b) in buf.iter().enumerate() {
        ram[wram_offset(0x7F1200 + i as u32).unwrap()] = *b;
    }
    // read_pos part way in, as it would be left after that message was drawn.
    ram[wram_offset(0x7E1CD9).unwrap()] = 8;

    let said = speaks_over(&r, &ram, 4);
    assert!(
        !said.iter().any(|t| t.contains("Lamp!")),
        "the message buffer is not read here: {said:?}"
    );
    assert!(said.iter().any(|t| t == "Lamp"), "the item is: {said:?}");
}

// ── Pickups ─────────────────────────────────────────────────────────────────

/// An in-play frame with the three pickup accumulators set: link_rupees_goal ($7EF360, u16),
/// link_hearts_filler ($7EF372) and link_magic_filler ($7EF373).
fn pickup_frame(rupees: u16, hearts_filler: u8, magic_filler: u8) -> Vec<u8> {
    let mut ram = dungeon_frame((100, 100), (120, 100), &[]);
    let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
    set(0x7EF360, (rupees & 0xFF) as u8);
    set(0x7EF361, (rupees >> 8) as u8);
    set(0x7EF372, hearts_filler);
    set(0x7EF373, magic_filler);
    ram
}

/// What a plugin already settled in play says when these three values change.
fn after_pickup(r: &Registry, before: (u16, u8, u8), after: (u16, u8, u8)) -> Vec<String> {
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let start = pickup_frame(before.0, before.1, before.2);
    for f in 0..3 {
        p.on_frame(&start, f);
    }
    p.on_frame(&pickup_frame(after.0, after.1, after.2), 3)
        .iter()
        .map(|i| i.text.clone())
        .collect()
}

#[test]
fn alttp_a_rupee_says_how_much_it_was_worth() {
    // kRupeesAbsorption is {1, 5, 20} for green, blue and red, and chests give more. The amount
    // comes free in the delta, so there is no reason to say only "rupee".
    let r = Registry::builtin();
    for (amount, line) in [
        (1u16, "1 rupee."),
        (5, "5 rupees."),
        (20, "20 rupees."),
        (300, "300 rupees."),
    ] {
        let said = after_pickup(&r, (0, 0, 0), (amount, 0, 0));
        assert!(said.iter().any(|t| t == line), "{amount}: {said:?}");
    }
}

#[test]
fn alttp_a_heart_and_a_fairy_are_told_apart() {
    // Both credit link_hearts_filler — a heart adds 8, a fairy 56 — so naming it "heart" either
    // way would be wrong for one of them.
    let r = Registry::builtin();
    let heart = after_pickup(&r, (0, 0, 0), (0, 8, 0));
    assert!(heart.iter().any(|t| t == "Heart."), "{heart:?}");
    let fairy = after_pickup(&r, (0, 0, 0), (0, 56, 0));
    assert!(fairy.iter().any(|t| t == "Fairy."), "{fairy:?}");
    assert!(
        !fairy.iter().any(|t| t == "Heart."),
        "a fairy is not a heart: {fairy:?}"
    );
}

#[test]
fn alttp_a_magic_jar_says_whether_it_was_a_full_one() {
    // A small jar adds 0x10; a full one sets the filler to 0x80.
    let r = Registry::builtin();
    let small = after_pickup(&r, (0, 0, 0), (0, 0, 0x10));
    assert!(small.iter().any(|t| t == "Magic."), "{small:?}");
    let full = after_pickup(&r, (0, 0, 0), (0, 0, 0x80));
    assert!(full.iter().any(|t| t == "Full magic."), "{full:?}");
}

#[test]
fn alttp_a_draining_filler_is_not_a_pickup() {
    // This is why the fillers are the right thing to watch and also the trap in watching them:
    // they fall every frame as they drain into health and magic, and rupees fall when spent.
    // None of that is something Link picked up.
    let r = Registry::builtin();
    let drained = after_pickup(&r, (100, 40, 0x40), (60, 16, 0x10));
    assert!(
        !drained
            .iter()
            .any(|t| t.contains("rupee") || t.contains("Heart") || t.contains("Magic")),
        "spending and draining say nothing: {drained:?}"
    );
}

#[test]
fn alttp_several_pickups_on_one_frame_are_all_named() {
    // A dropped heart and a rupee can be absorbed on the same frame, and each is worth saying.
    let r = Registry::builtin();
    let said = after_pickup(&r, (0, 0, 0), (5, 8, 0x10));
    for line in ["5 rupees.", "Heart.", "Magic."] {
        assert!(said.iter().any(|t| t == line), "{line} among {said:?}");
    }
}

#[test]
fn alttp_arriving_in_play_is_not_a_pickup() {
    // A game loading restores all three at once. The baseline is dropped whenever Link is not in
    // play, so the first in-play frame has nothing to compare against and says nothing.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Not in play: the file select, all three reading zero as they do before a save is loaded.
    let menu = file_select_frame(0, [true, false, false]);
    for f in 0..3 {
        p.on_frame(&menu, f);
    }

    // Then in play, the save's purse and fillers appearing all at once. Every one of them is
    // higher than it was a frame ago, and none of it was picked up — which is why the baseline
    // has to be dropped rather than carried across the boundary.
    let arrived: Vec<String> = p
        .on_frame(&pickup_frame(0x2C, 40, 0x80), 3)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        !arrived
            .iter()
            .any(|t| t.contains("rupee") || t.contains("Heart") || t.contains("magic")),
        "arriving in play announces no pickups: {arrived:?}"
    );
}

#[test]
fn alttp_a_different_save_loaded_mid_session_is_not_a_pickup() {
    // The baseline has to be DROPPED on leaving play, not merely left unset. Being in play
    // records it; going to the file select and loading a different save comes back with a bigger
    // purse and fuller bottles, and the difference is a different Link, not an acquisition.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // In play, with a modest purse, long enough for the baseline to be taken.
    let poor = pickup_frame(10, 0, 0);
    for f in 0..3 {
        p.on_frame(&poor, f);
    }

    // Out to the file select and back in on a save that is further along.
    p.on_frame(&file_select_frame(0, [true, false, false]), 3);
    let rich: Vec<String> = p
        .on_frame(&pickup_frame(900, 40, 0x80), 4)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        !rich
            .iter()
            .any(|t| t.contains("rupee") || t.contains("Heart") || t.contains("magic")),
        "a different save is not a pickup: {rich:?}"
    );
}

#[test]
fn alttp_a_blinking_enemy_slot_does_not_hand_the_room_to_the_chest() {
    // From a real log: "Defeat all enemies." / "Open the chest." / "Defeat all enemies." while
    // the room's actual objective was the fight and the key it drops.
    //
    // The two objectives disagreed about what makes a room busy. `kill` needs a countable enemy,
    // `chest` needs there to be none — so in any frame where the enemy is momentarily NOT
    // countable, and a sprite slot blinks for all sorts of reasons, kill went inactive and chest
    // went active in the same frame. Worse, the spoken latch held only the last objective, so
    // every wobble was a fresh announcement.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Room 0x72 with a chest and a guard, as the log's room was.
    let frame = |alive: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7EF3C5, 2);
        set(0x7E040C, 0x02);
        set(0x7E00A0, 0x72);
        set(0x7F2000 + 20 * 64 + 34, 0x58); // an unopened chest
        set(0x7E0DD0, if alive { 0x09 } else { 0x00 });
        set(0x7E0E20, 65); // Green Soldier
        set(0x7E0D10, 0xF4);
        set(0x7E0D00, 0xA4);
        set(0x7E0E50, 4);
        ram
    };

    let live = frame(true);
    p.on_frame(&live, 0);
    p.on_frame(&live, 1);
    p.command("advance", &live);
    let started: Vec<String> = p
        .on_frame(&live, 2)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        started.iter().any(|t| t.contains("Defeat all enemies")),
        "the fight is the objective: {started:?}"
    );

    // One frame with the slot blinked out, then back — the guard has not been killed.
    let mut heard = Vec::new();
    for f in 3..12 {
        let blink = f % 3 == 0; // a slot that flickers, not one that dies
        heard.extend(p.on_frame(&frame(!blink), f).iter().map(|i| i.text.clone()));
    }
    assert!(
        !heard.iter().any(|t| t.contains("Open the chest")),
        "a blink is not the room going quiet: {heard:?}"
    );
    assert!(
        !heard.iter().any(|t| t.contains("Defeat all enemies")),
        "and the fight is not re-announced: {heard:?}"
    );
}

#[test]
fn alttp_an_objective_that_wobbles_is_still_announced_only_once() {
    // QUIET stops the fight-versus-chest wobble at source, but it does not cover every pair. A
    // dropped key is a sprite too, and a blinking slot flips the room between "grab the key" and
    // "open the chest". A latch holding only the LAST objective announced calls each flip new,
    // because each is always different from the one before it — so it remembers all of them.
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Room 0x72, no enemies, an unopened chest, and a dropped small key that blinks.
    let frame = |key_there: bool| -> Vec<u8> {
        let mut ram = dungeon_frame((20, 20), (20, 6), &[]);
        {
            let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
            set(0x7EF3C5, 2);
            set(0x7E040C, 0x02);
            set(0x7E00A0, 0x72);
            set(0x7E00AE, 0x00); // no kill tag
            set(0x7F2000 + 20 * 64 + 34, 0x58); // an unopened chest
        }
        if key_there {
            sprite_slot(&mut ram, 0, 228, (26, 20), 0); // a dropped small key
        }
        ram
    };

    let present = frame(true);
    p.on_frame(&present, 0);
    p.on_frame(&present, 1);
    p.command("advance", &present);

    // Let the room settle, then flip the key slot on and off for a while.
    let mut heard = settle_quiet(&mut p, &present, 2);
    let base = 2 + QUIET_FRAMES + 1;
    for f in 0..12u64 {
        heard.extend(
            p.on_frame(&frame(f % 2 == 0), base + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }

    let keys = heard.iter().filter(|t| t.contains("Grab the key")).count();
    let chests = heard
        .iter()
        .filter(|t| t.contains("Open the chest"))
        .count();
    assert!(
        keys <= 1,
        "the key is announced once, not per flip: {heard:?}"
    );
    assert!(chests <= 1, "and so is the chest: {heard:?}");
    assert!(keys + chests >= 1, "something was announced: {heard:?}");
}

#[test]
fn alttp_a_box_opening_before_its_load_reads_nothing() {
    // From a log of real play: three utterances of nonsense — "V", "o", "pw2ZfLPDHAAA?" — landing
    // immediately before each real message, and identical both times, so deterministic buffer
    // content rather than noise.
    //
    // Trust outlives the moment it was earned. A read_pos that falls while no box is up — which
    // is what loading a save state looks like, and this session does that a lot — was taken as a
    // load and granted trust. Trust then survived every frame of walking around, and the frame a
    // box OPENS still shows the previous buffer, because Text_Initialize has not run yet. So the
    // reader worked its way through whatever was in there, page by page, until the real load
    // finally arrived.
    let (old, _) = dialog_buffer(&["Take my sword and shield.", "<end>"]);
    let (fresh, _) = dialog_buffer(&["I will give 100 Rupees.", "<end>"]);

    let in_play = |buf: &[u8], pos: u16| -> Vec<u8> {
        let mut ram = dialog_frame(buf, pos);
        ram[wram_offset(0x7E0010).unwrap()] = 0x07; // no box
        ram
    };

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // In play, the previous message still in the buffer with read_pos left at its end.
    for f in 0..3 {
        p.on_frame(&in_play(&old, 25), f);
    }
    // A save state loads: read_pos lands lower, with no box in sight. Not a message starting.
    p.on_frame(&in_play(&old, 4), 3);

    // Now a box opens. Text_Initialize has not run, so this frame still shows the old buffer.
    let mut opening = Vec::new();
    for f in 0..3 {
        opening.extend(
            p.on_frame(&dialog_frame(&old, 6 + f as u16), 4 + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        opening.is_empty(),
        "nothing is read until the box's own load: {opening:?}"
    );

    // The load arrives, and the message that actually opened the box is read.
    p.on_frame(&dialog_frame(&fresh, 0), 8);
    let said = read_page(&mut p, &dialog_frame(&fresh, 4), 9);
    assert!(
        said.iter().any(|t| t == "I will give 100 Rupees."),
        "and then the real message: {said:?}"
    );
}

#[test]
fn alttp_a_stale_read_pos_does_not_pick_the_last_page_first() {
    // The map came out as "of the dungeon (Press )." and only then "You got the Map! ... the
    // rest" — its two pages, backwards. Text_LoadCharacterBuffer fills the buffer before it
    // zeroes read_pos, so there is a window with the new text at the old message's position;
    // locating a page from there lands in the LAST one.
    //
    // read_pos has to be inside the page being read, the renderer being what draws it. Beyond
    // the page's own break, the position is not about this page, and the page is passed over
    // rather than spoken.
    let (old, ostarts) = dialog_buffer(&["Take my sword and shield and listen to me.", "<end>"]);
    let (map, _) = dialog_buffer(&[
        "You got the Map! You can use it to see the rest",
        "<wait>",
        "<scroll>",
        "of the dungeon (Press ).",
        "<end>",
    ]);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let said = warm(&mut p, &dialog_frame(&old, ostarts[1]));
    assert!(said.iter().any(|t| t.contains("Take my sword")), "{said:?}");

    // The map's text arrives while read_pos still sits where the message before it left off.
    // Deliberately INSIDE the map's second page rather than past the end of it: a position past
    // the end finds no break at all and stops for that reason, which would prove nothing about
    // locating a page from a position that does not belong to it.
    let stale = 52;
    let mut window = Vec::new();
    for f in 0..4 {
        window.extend(
            p.on_frame(&dialog_frame(&map, stale), WARM + f)
                .iter()
                .map(|i| i.text.clone()),
        );
    }
    assert!(
        !window.iter().any(|t| t.contains("of the dungeon")),
        "the last page is not spoken first: {window:?}"
    );

    // read_pos catches up, and the map reads from its first page.
    p.on_frame(&dialog_frame(&map, 0), WARM + 5);
    let first = read_page(&mut p, &dialog_frame(&map, 4), WARM + 6);
    assert!(
        first
            .iter()
            .any(|t| t == "You got the Map! You can use it to see the rest"),
        "the first page leads: {first:?}"
    );
}

#[test]
fn alttp_a_half_written_buffer_is_not_read() {
    // The uncle's line came out as "Link, I'm going out for a while. I'll be back by morning.
    // Don't leave tle. My name is Zelda." — his message as far as it had been written, and then
    // the tail of the message before it, because the terminator is written LAST and the decode
    // ran straight past where it should have been.
    //
    // A half-written buffer differs from one frame to the next; a finished one does not. So a
    // page is spoken only once its text has held still for a frame.
    let (whole, _) = dialog_buffer(&["Do not leave the house.", "<end>"]);

    // The same message with its terminator not yet written, so the decode runs on into the
    // leftovers of a longer message behind it.
    let mut half = whole.clone();
    let cut = whole.len() - 8;
    half.truncate(cut);
    half.extend("tle. My name is Zelda.".chars().map(text_byte));
    half.push(TEXT_END);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // The load lands, and the very next frame catches the buffer mid-write.
    p.on_frame(&dialog_frame(&half, 0), 0);
    let caught: Vec<String> = p
        .on_frame(&dialog_frame(&half, 4), 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        !caught.iter().any(|t| t.contains("Zelda")),
        "a half-written page is not spoken: {caught:?}"
    );

    // The write completes, and what is spoken is the finished message.
    let done = read_page(&mut p, &dialog_frame(&whole, 4), 2);
    assert!(
        done.iter().any(|t| t == "Do not leave the house."),
        "the finished message is: {done:?}"
    );
    assert!(
        !done.iter().any(|t| t.contains("Zelda")),
        "without the leftovers: {done:?}"
    );
}

// ── Choices ─────────────────────────────────────────────────────────────────

const TEXT_CHOOSE: u8 = 0x68;
const TEXT_LINE2: u8 = 0x75;
const TEXT_LINE3: u8 = 0x76;

/// The sanctuary priest's closing page, as the message itself is laid out: the question, then the
/// options on their own lines with the cursor glyph against the first, then a Choose command.
fn choice_buffer() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend("Do you understand?".chars().map(text_byte));
    buf.push(TEXT_LINE2);
    buf.extend("  > Yes".chars().map(text_byte));
    buf.push(TEXT_LINE3);
    buf.extend("  Not at all".chars().map(text_byte));
    buf.push(TEXT_CHOOSE);
    buf.push(TEXT_END);
    buf
}

/// The cursor-only message the game loads to move the marker, byte for byte as observed live:
/// a Speed command, line 3, six blanks, line 2, four blanks, the cursor, then the Choose.
fn cursor_only_message() -> Vec<u8> {
    let mut buf = vec![0x7A, 0x00, TEXT_LINE3];
    buf.extend([0x59u8; 6]);
    buf.push(TEXT_LINE2);
    buf.extend([0x59u8; 4]);
    buf.push(0x44); // the cursor glyph
    buf.push(TEXT_CHOOSE);
    buf.push(TEXT_END);
    buf
}

/// The choice frame with a given selection in choice_in_multiselect_box.
fn choice_frame(buf: &[u8], pos: u16, choice: u8) -> Vec<u8> {
    let mut ram = dialog_frame(buf, pos);
    ram[wram_offset(0x7E1CE8).unwrap()] = choice;
    ram
}

#[test]
fn alttp_a_choice_reads_the_question_and_the_option_under_the_cursor() {
    // Reading every option gives away a choice the player has not made. The question and the
    // option they are on is what the screen is telling them.
    let buf = choice_buffer();
    let brk = (buf.len() - 2) as u16; // the Choose command
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    let said = warm(&mut p, &choice_frame(&buf, brk, 0));
    assert!(
        said.iter().any(|t| t == "Do you understand? Yes"),
        "the question and the selected option: {said:?}"
    );
    assert!(
        !said.iter().any(|t| t.contains("Not at all")),
        "and not the one not selected: {said:?}"
    );
}

#[test]
fn alttp_moving_between_options_reads_the_one_moved_to() {
    let buf = choice_buffer();
    let brk = (buf.len() - 2) as u16;
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    warm(&mut p, &choice_frame(&buf, brk, 0));

    // Down to the second option. The game reloads a cursor-only message to move the marker, so the
    // options are no longer in the buffer — they have to have been remembered.
    let cursor_only = cursor_only_message();

    let moved: Vec<String> = p
        .on_frame(&choice_frame(&cursor_only, 0, 1), WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        moved.iter().any(|t| t == "Not at all"),
        "the option moved to is read: {moved:?}"
    );
    assert!(
        !moved.iter().any(|t| t.contains("understand")),
        "without asking the question again: {moved:?}"
    );

    // And back up again.
    let back: Vec<String> = p
        .on_frame(&choice_frame(&cursor_only, 0, 0), WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(back.iter().any(|t| t == "Yes"), "and back: {back:?}");
}

#[test]
fn alttp_a_cursor_only_message_says_nothing_of_its_own() {
    // Blanks and a ">" is not something to read out, and it must not be allowed to replace the
    // options it was drawn on top of — otherwise the next move has nothing to name.
    let buf = choice_buffer();
    let brk = (buf.len() - 2) as u16;
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    warm(&mut p, &choice_frame(&buf, brk, 0));

    let cursor_only = cursor_only_message();

    // Loaded with the selection unchanged: nothing to announce at all.
    let quiet = read_page(&mut p, &choice_frame(&cursor_only, 4, 0), WARM);
    assert!(
        !quiet.iter().any(|t| t.contains(">")),
        "the marker is not spoken: {quiet:?}"
    );

    // The options survived it, so moving still names one.
    let moved: Vec<String> = p
        .on_frame(&choice_frame(&cursor_only, 4, 1), WARM + 3)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        moved.iter().any(|t| t == "Not at all"),
        "the options were not lost: {moved:?}"
    );
}

#[test]
fn alttp_moving_without_the_options_still_says_which_one() {
    // Arriving part way through a choice — reloading the plugin while one is on screen — means
    // never having seen the page that carries the options, and they are not in the buffer by then.
    // The cursor still moves, and answering a keypress with silence reads as the reader having
    // stopped working. So it says which option, even when it cannot say what it is called.
    let cursor = cursor_only_message();
    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Straight into the cursor-only message: no page with options has ever been read.
    let quiet = warm(&mut p, &choice_frame(&cursor, 4, 0));
    assert!(
        !quiet.iter().any(|t| t.contains(">")),
        "the marker itself is still not spoken: {quiet:?}"
    );

    let moved: Vec<String> = p
        .on_frame(&choice_frame(&cursor, 4, 1), WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        moved.iter().any(|t| t == "Option 2"),
        "the move is answered: {moved:?}"
    );

    let back: Vec<String> = p
        .on_frame(&choice_frame(&cursor, 4, 0), WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        back.iter().any(|t| t == "Option 1"),
        "and so is moving back: {back:?}"
    );
}

#[test]
fn alttp_options_are_recovered_from_the_message_underneath() {
    // Arriving part way through a choice means never having read the page with the options on it.
    // They are still in the buffer though: the cursor-only message is seventeen bytes and
    // Text_LoadCharacterBuffer writes only as far as a message needs, so the prompt it replaced is
    // sitting right behind it. Observed live — the priest's "Do you understand? > Yes / Not at
    // all" was still readable from byte 17 on, while byte 0 held the cursor message.
    let mut buf = cursor_only_message();
    // The prompt underneath, as the buffer really lays it out: a page break, the question, the
    // options, the Choose, and its own terminator.
    buf.push(TEXT_WAITKEY);
    buf.push(TEXT_SCROLL);
    buf.extend("Do you understand?".chars().map(text_byte));
    buf.push(TEXT_SCROLL);
    buf.extend("    > Yes".chars().map(text_byte));
    buf.push(TEXT_SCROLL);
    buf.extend("       Not at all".chars().map(text_byte));
    buf.push(TEXT_CHOOSE);
    buf.push(TEXT_END);

    let r = Registry::builtin();
    let mut p = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();

    // Straight into the cursor message, exactly as a reload mid-choice does.
    let quiet = warm(&mut p, &choice_frame(&buf, 4, 0));
    assert!(
        !quiet.iter().any(|t| t.contains("understand")),
        "the question is not re-asked: {quiet:?}"
    );

    // Moving now names the option, rather than falling back to "Option 2".
    let moved: Vec<String> = p
        .on_frame(&choice_frame(&buf, 4, 1), WARM)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(
        moved.iter().any(|t| t == "Not at all"),
        "the name is recovered: {moved:?}"
    );
    let back: Vec<String> = p
        .on_frame(&choice_frame(&buf, 4, 0), WARM + 1)
        .iter()
        .map(|i| i.text.clone())
        .collect();
    assert!(back.iter().any(|t| t == "Yes"), "and the other: {back:?}");
}

#[test]
fn alttp_the_guide_tone_says_whether_link_is_facing_the_way_to_walk() {
    // The whole of steering, in one rule: turn until the tone goes up, then walk. High is
    // facing the way to go, mid is the route off to one side, low is facing away from it.
    // Three pitches rather than a stereo image the player has to translate into a heading.
    let r = Registry::builtin();
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let ram = vec![0u8; 128 * 1024];

    // Facing codes are the game's own: 0 north, 2 south, 4 west, 6 east.
    // The corner is well to the east, so east is the way to walk.
    let probe = r#"
        local east, west = 64, -64
        local ahead  = path_tone(6, east, 0)
        local behind = path_tone(4, east, 0)
        local side_n = path_tone(0, east, 0)
        local side_s = path_tone(2, east, 0)
        -- The way to walk is the axis with the most ground left, so a corner mostly north
        -- with a little east in it is a northward tone, not an eastward one.
        local dominant = path_tone(0, 8, -64)
        local unknown  = path_tone(99, east, 0)
        local standing = path_tone(6, 0, 0)
        return table.concat({ahead, behind, side_n, side_s, dominant, unknown, standing}, ",")
    "#;
    let tones: Vec<f32> = plugin
        .eval(probe, &ram)
        .unwrap()
        .split(',')
        .map(|v| v.parse().expect("a number per tone"))
        .collect();
    let (ahead, behind, side_n, side_s) = (tones[0], tones[1], tones[2], tones[3]);

    assert!(
        ahead > side_n && side_n > behind,
        "high facing the way to go, mid to the side, low facing away: {tones:?}"
    );
    assert_eq!(side_n, side_s, "both sideways facings are the one tone");
    assert_eq!(tones[4], ahead, "the dominant axis is the way to walk");
    // An unknown facing and standing on the corner both say the neutral middle rather
    // than claiming a direction they do not have.
    assert_eq!(tones[5], side_n);
    assert_eq!(tones[6], side_n);

    // And the vocabulary still holds: clear of the item tone below and the pit warning
    // above, so the guide cannot be mistaken for either.
    assert!(behind > 2.0, "above the item tone: {tones:?}");
    assert!(ahead < 4.0, "below the pit warning: {tones:?}");
    // Equal ratios, so turning through the three sounds like even steps.
    let step_up = ahead / side_n;
    let step_from_low = side_n / behind;
    assert!(
        (step_up - step_from_low).abs() < 0.02,
        "even steps: {step_up} vs {step_from_low}"
    );
}
