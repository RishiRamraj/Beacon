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
    let calm_vol = path_beacon(&plugin).expect("the guide sounds when clear").volume;
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
        !texts.iter().any(|t| t.contains("Bow") || t.contains("Big Key")
            || t.to_lowercase().contains("boss") || t.to_lowercase().contains("exit")),
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
        armed, "16,1",
        "Zelda beat arms the courtyard chain at index 1: {armed}"
    );

    // Drive Link onto the first waypoint (282,225); a frame there advances to 2.
    let at_wp1 = frame(282 * 8, 225 * 8);
    plugin.on_frame(&at_wp1, 2);
    let advanced = plugin
        .eval("return tostring(nav_chain_i)", &at_wp1)
        .unwrap();
    assert_eq!(
        advanced, "2",
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
        plugin.eval("return tostring(nav_chain_i)", &away).unwrap(),
        "2",
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
        resumed, "16,2",
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
        armed, "16,2",
        "arms at the door Link is beside, not back at the bushes: {armed}"
    );
}

#[test]
fn alttp_zelda_chain_leads_through_the_castle_rooms() {
    // The courtyard chain continues past the door into the castle as dungeon
    // waypoints, room by room: Find Zelda in room 0x61, then a silent point in
    // room 0x60. The chain stays armed across rooms; reaching one advances to the
    // next without a signature change.
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

    // In room 0x61, a few tiles west of the Find Zelda waypoint (72,415): engaging
    // arms the chain, and the driver leads to that waypoint, announcing "Find
    // Zelda" as it sets off.
    let approach = frame(0x61, 65, 415);
    plugin.on_frame(&approach, 0);
    plugin.on_frame(&approach, 1);
    plugin.command("advance", &approach); // engage -> chain armed
    let out = plugin.on_frame(&approach, 2); // driver leads to the waypoint
    assert_eq!(
        plugin
            .eval("return #nav_chain .. ',' .. nav_chain_i", &approach)
            .unwrap(),
        "16,4",
        "the dungeon leg targets the Find Zelda waypoint (index 4)"
    );
    assert!(
        out.iter().any(|i| i.text.contains("Find Zelda")),
        "announces the waypoint phrase: {:?}",
        out.iter().map(|i| &i.text).collect::<Vec<_>>()
    );

    // Reaching Find Zelda records it and goes quiet — no room-graph hop. The chain
    // stays armed; only when Link crosses into room 0x60 (its lower floor) does its
    // waypoint (index 5) take over and lead to the silent point there.
    plugin.on_frame(&frame(0x61, 72, 415), 3); // on Find Zelda -> recorded
    assert!(
        plugin
            .eval("return tostring(nav_chain)", &frame(0x61, 72, 415))
            .unwrap()
            .contains("table"),
        "the chain stays armed after Find Zelda"
    );
    plugin.on_frame(&frame(0x60, 48, 415), 4); // enter room 0x60
    plugin.on_frame(&frame(0x60, 48, 415), 5);
    assert_eq!(
        plugin
            .eval("return tostring(nav_chain_i)", &frame(0x60, 48, 415))
            .unwrap(),
        "5",
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
        plugin.eval("return tostring(nav_chain_i)", &down).unwrap(),
        "10",
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
    assert_eq!(
        plugin2.eval("return tostring(nav_chain_i)", &up).unwrap(),
        "9",
        "an up-stair is not a down path: the one-way drop is not routed through backwards"
    );
}

#[test]
fn alttp_a_layer_swap_stair_cannot_be_walked_across_on_its_entry_floor() {
    // A layer-swap stair is not flat ground: stepping onto it forces the floor change,
    // so the pathfinder must not walk across it to the tile beyond on the same floor.
    // Room 0x72, Link on the upper floor, split by a wall band with a single gap; the
    // upper waypoint 9 sits past the gap. With plain floor in the gap Link walks
    // through to it (index 9); with a down-stair in the gap — whose portal is blocked
    // (impassable landing) — he cannot walk across, so waypoint 9 is unreachable and
    // the guide holds at waypoint 8.
    let r = Registry::builtin();

    let frame = |gap: u8| -> Vec<u8> {
        let mut ram = dungeon_frame((159, 470), (0, 0), &[]);
        let mut set = |addr: u32, v: u8| ram[wram_offset(addr).unwrap()] = v;
        set(0x7E00A0, 0x72);
        set(0x7E00EE, 0); // upper floor
        set(0x7EF34A, 1); // Lamp
        set(0x7EF359, 1); // sword
        set(0x7EF3CC, 0); // Zelda not following
        set(0x7EF3C5, 0); // Zelda beat -> courtyard chain armed
        set(0x7EF0E5, 0x80); // room 0x72 chest opened -> no kill sub-goal in the way
        // A wall band across the upper floor at grid row 33, one gap at col 31.
        for tx in 0..64u32 {
            set(0x7F2000 + 33 * 64 + tx, 0x01);
        }
        set(0x7F2000 + 33 * 64 + 31, gap); // the gap: plain floor, or a down-stair
        set(0x7F3000 + 33 * 64 + 31, 0x01); // block the stair's landing (portal leads nowhere)
        ram
    };

    // Plain floor in the gap: Link walks through to the far upper waypoint (9).
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let open = frame(0x00);
    plugin.on_frame(&open, 0);
    plugin.on_frame(&open, 1);
    plugin.command("advance", &open);
    plugin.on_frame(&open, 2);
    assert_eq!(
        plugin.eval("return tostring(nav_chain_i)", &open).unwrap(),
        "9",
        "plain floor in the gap is walked through to the far waypoint (index 9)"
    );

    // A down-stair in the gap (portal blocked): it cannot be walked across, so the far
    // waypoint is unreachable and the guide holds at the near one (index 8).
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let stair = frame(0x3E);
    plugin2.on_frame(&stair, 0);
    plugin2.on_frame(&stair, 1);
    plugin2.command("advance", &stair);
    plugin2.on_frame(&stair, 2);
    assert_eq!(
        plugin2.eval("return tostring(nav_chain_i)", &stair).unwrap(),
        "8",
        "a stair in the gap is not flat ground: it is not walked across to the far waypoint"
    );
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
        plugin.eval("return tostring(nav_chain_i)", &hole).unwrap(),
        "10",
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
    assert_eq!(
        plugin2.eval("return tostring(nav_chain_i)", &solid).unwrap(),
        "9",
        "with no floor crossing the lower-floor waypoint stays unreachable (index 9)"
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
        plugin.eval("return tostring(nav_chain_i)", &open).unwrap(),
        "10",
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
    assert_eq!(
        plugin2.eval("return tostring(nav_chain_i)", &locked).unwrap(),
        "9",
        "a closed locked door blocks the route: the guide does not lead through it"
    );
}

#[test]
fn alttp_locked_door_gate_holds_the_route_at_the_chest_until_keyed() {
    // Room 0x71 (Boomerang Chest Room) is open enough on the lower floor that the
    // pathfinder reaches the far waypoint (index 15, past the locked door) without
    // ever crossing the door, so a collision block alone cannot keep the guide out.
    // The waypoints past the door carry a `gate` on the small-key / door state:
    // keyless, the guide holds at the chest anchor (index 13, where the route out to
    // the key in 0x70 begins); once Link holds a key the far waypoint opens up.
    let r = Registry::builtin();

    let frame = |keys: u8| -> Vec<u8> {
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
        set(0x7F2000 + (486 & 63) * 64 + (79 & 63), 0xF0); // upper-floor door still shut
        ram
    };

    // Keyless: the far waypoint is gated off, so the guide holds at the anchor (13).
    let mut plugin = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let keyless = frame(0);
    plugin.on_frame(&keyless, 0);
    plugin.on_frame(&keyless, 1);
    plugin.command("advance", &keyless);
    plugin.on_frame(&keyless, 2);
    assert_eq!(
        plugin.eval("return tostring(nav_chain_i)", &keyless).unwrap(),
        "13",
        "keyless, the guide holds at the chest anchor and does not lead past the locked door"
    );

    // With a small key the gate opens and the guide advances past the door (14).
    let mut plugin2 = LuaPlugin::load(&r.specs()[0], std::rc::Rc::new(Vec::new())).unwrap();
    let keyed = frame(1);
    plugin2.on_frame(&keyed, 0);
    plugin2.on_frame(&keyed, 1);
    plugin2.command("advance", &keyed);
    plugin2.on_frame(&keyed, 2);
    assert_eq!(
        plugin2.eval("return tostring(nav_chain_i)", &keyed).unwrap(),
        "14",
        "with the key the gate opens and the route continues past the door"
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
        plugin.eval("return tostring(nav_chain ~= nil)", &ram).unwrap(),
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
        plugin.eval("return tostring(nav_chain ~= nil)", &ram).unwrap(),
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
        set(0x7E00AE, 0x08); // header kill tag (a real kill-room)
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
