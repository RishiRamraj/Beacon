//! A Link to the Past instrumentation.
//!
//! This is a native stand-in for what will become a Lua plugin. It is written
//! against the same shape the plugin API will have (read memory, emit intents,
//! never speak) so that porting it later is a translation rather than a
//! redesign, and so the plugin API is designed against something real.
//!
//! Memory addresses come from the alttp-navi proof of concept, which is the
//! surviving product of the reverse engineering effort and the most valuable
//! thing it produced.

use std::time::Duration;

use beacon_output::{Intent, Priority};

/// SNES bank $7E is the first 64 KiB of work RAM, bank $7F the second.
fn wram_offset(addr: u32) -> Option<usize> {
    match addr >> 16 {
        0x7E => Some((addr & 0xFFFF) as usize),
        0x7F => Some(0x10000 + (addr & 0xFFFF) as usize),
        _ => None,
    }
}

fn u8_at(ram: &[u8], addr: u32) -> Option<u8> {
    ram.get(wram_offset(addr)?).copied()
}

fn u16_at(ram: &[u8], addr: u32) -> Option<u16> {
    let o = wram_offset(addr)?;
    Some(*ram.get(o)? as u16 | ((*ram.get(o + 1)? as u16) << 8))
}

/// One frame's reading of the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub module: u8,
    /// Non-zero while a transition or animation is in progress.
    pub submodule: u8,
    pub health: u8,
    pub max_health: u8,
    pub rupees: u16,
    pub x: u16,
    pub y: u16,
    /// 0 = north, 2 = south, 4 = west, 6 = east.
    pub direction: u8,
    pub indoors: u8,
    pub dungeon_room: u16,
    pub ow_screen: u16,
    pub world: u8,
}

impl State {
    pub fn read(ram: &[u8]) -> Option<Self> {
        Some(State {
            module: u8_at(ram, 0x7E0010)?,
            submodule: u8_at(ram, 0x7E0011)?,
            health: u8_at(ram, 0x7EF36D)?,
            max_health: u8_at(ram, 0x7EF36C)?,
            rupees: u16_at(ram, 0x7EF360)?,
            x: u16_at(ram, 0x7E0022)?,
            y: u16_at(ram, 0x7E0020)?,
            direction: u8_at(ram, 0x7E002F)?,
            indoors: u8_at(ram, 0x7E001B)?,
            dungeon_room: u16_at(ram, 0x7E00A0)?,
            ow_screen: u16_at(ram, 0x7E008A)?,
            world: u8_at(ram, 0x7E007B)?,
        })
    }

    /// Health is stored in eighths of a heart.
    pub fn hearts(&self) -> f32 {
        self.health as f32 / 8.0
    }

    pub fn max_hearts(&self) -> f32 {
        self.max_health as f32 / 8.0
    }

    /// Whether the player is actually controlling Link, as opposed to sitting
    /// in a menu, a transition, or the intro.
    pub fn in_play(&self) -> bool {
        matches!(self.module, 0x07 | 0x09) && self.submodule == 0
    }

    pub fn facing(&self) -> &'static str {
        match self.direction {
            0 => "north",
            2 => "south",
            4 => "west",
            _ => "east",
        }
    }
}

pub fn module_name(m: u8) -> &'static str {
    match m {
        0x00 => "intro",
        0x01 => "file select",
        0x02 => "copy file",
        0x03 => "erase file",
        0x04 => "name file",
        0x05 => "loading game",
        0x06 => "entering dungeon",
        0x07 => "dungeon",
        0x08 => "entering overworld",
        0x09 => "overworld",
        0x0e => "menu",
        0x12 => "death",
        0x14 => "attract mode",
        0x19 => "triforce room",
        _ => "unknown",
    }
}

/// Fraction of maximum health below which the low-health warning fires.
const LOW_HEALTH_FRACTION: f32 = 0.3;

pub struct Alttp {
    prev: Option<State>,
    /// Latched so the warning fires on crossing the threshold, not on every
    /// frame spent below it.
    low_health_warned: bool,
}

impl Alttp {
    pub fn new() -> Self {
        Alttp {
            prev: None,
            low_health_warned: false,
        }
    }

    /// Reads a frame and proposes what might be worth saying.
    ///
    /// Proposes only. The arbiter decides what survives, so this is free to be
    /// generous: being wrong about relevance is cheaper here than a plugin
    /// implementing its own suppression badly.
    pub fn on_frame(&mut self, ram: &[u8]) -> Vec<Intent> {
        let Some(now) = State::read(ram) else {
            return Vec::new();
        };
        let Some(prev) = self.prev.replace(now) else {
            return Vec::new(); // First frame has nothing to compare against.
        };

        let mut out = Vec::new();

        // Death outranks everything else that could be happening.
        if now.module == 0x12 && prev.module != 0x12 {
            out.push(Intent::new("You died.", Priority::Critical, "combat"));
            self.low_health_warned = false;
            return out;
        }

        // Damage. Only while actually in play, or menu transitions that zero
        // health register as being hit.
        if now.in_play() && now.health < prev.health && prev.max_health > 0 {
            let lost = (prev.health - now.health) as f32 / 8.0;
            out.push(
                Intent::new(
                    format!("Hit. {lost:.1} hearts lost, {:.1} left.", now.hearts()),
                    Priority::Critical,
                    "combat",
                )
                .dedup_for(Duration::from_millis(400)),
            );
        }

        // Low health, latched on the crossing.
        if now.max_health > 0 && now.in_play() {
            let fraction = now.health as f32 / now.max_health as f32;
            if fraction <= LOW_HEALTH_FRACTION && now.health > 0 {
                if !self.low_health_warned {
                    out.push(Intent::new(
                        format!("Low health. {:.1} hearts.", now.hearts()),
                        Priority::Critical,
                        "combat",
                    ));
                    self.low_health_warned = true;
                }
            } else {
                self.low_health_warned = false;
            }
        }

        // Healing is worth knowing about too, quietly.
        if now.in_play() && now.health > prev.health {
            out.push(
                Intent::new(
                    format!("{:.1} hearts.", now.hearts()),
                    Priority::Interaction,
                    "status",
                )
                .dedup_for(Duration::from_millis(800)),
            );
        }

        // Top level state changes: file select, entering a dungeon, and so on.
        if now.module != prev.module {
            out.push(Intent::new(
                module_name(now.module).to_string(),
                Priority::Navigation,
                "area",
            ));
        }

        // Light and dark world.
        if now.world != prev.world && now.in_play() {
            out.push(Intent::new(
                if now.world == 0 {
                    "Light world."
                } else {
                    "Dark world."
                },
                Priority::Navigation,
                "area",
            ));
        }

        // Moving between rooms or overworld screens. Collapsed under one key so
        // a transition that changes both only announces once.
        if now.in_play() && prev.in_play() {
            if now.indoors == 1 && now.dungeon_room != prev.dungeon_room {
                out.push(
                    Intent::new(
                        format!("Room {}.", now.dungeon_room),
                        Priority::Navigation,
                        "area",
                    )
                    .collapse("area-change", 0.0),
                );
            } else if now.indoors == 0 && now.ow_screen != prev.ow_screen {
                out.push(
                    Intent::new(
                        format!("Area {}.", now.ow_screen),
                        Priority::Navigation,
                        "area",
                    )
                    .collapse("area-change", 0.0),
                );
            }
        }

        out
    }

    /// Answers the "where am I?" command.
    pub fn describe_position(&self) -> Intent {
        let text = match self.prev {
            Some(s) if s.in_play() => format!(
                "{}, facing {}, position {} {}.",
                if s.indoors == 1 {
                    format!("Room {}", s.dungeon_room)
                } else {
                    format!("Area {}", s.ow_screen)
                },
                s.facing(),
                s.x,
                s.y
            ),
            Some(s) => format!("{}. Not in play.", module_name(s.module)),
            None => "No game state yet.".to_string(),
        };
        Intent::new(text, Priority::Navigation, "on-demand")
    }

    /// Answers the "status" command.
    pub fn describe_status(&self) -> Intent {
        let text = match self.prev {
            Some(s) if s.max_health > 0 => format!(
                "{:.1} of {:.1} hearts. {} rupees.",
                s.hearts(),
                s.max_hearts(),
                s.rupees
            ),
            _ => "No game state yet.".to_string(),
        };
        Intent::new(text, Priority::Navigation, "on-demand")
    }
}

impl Default for Alttp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram_with(values: &[(u32, u8)]) -> Vec<u8> {
        let mut ram = vec![0u8; 128 * 1024];
        for (addr, v) in values {
            ram[wram_offset(*addr).unwrap()] = *v;
        }
        ram
    }

    /// In-play overworld state with full health.
    fn playing(health: u8) -> Vec<u8> {
        ram_with(&[
            (0x7E0010, 0x09),
            (0x7E0011, 0x00),
            (0x7EF36C, 24),
            (0x7EF36D, health),
        ])
    }

    #[test]
    fn first_frame_says_nothing() {
        let mut a = Alttp::new();
        assert!(a.on_frame(&playing(24)).is_empty());
    }

    #[test]
    fn reports_damage_with_hearts_remaining() {
        let mut a = Alttp::new();
        a.on_frame(&playing(24));
        let out = a.on_frame(&playing(16));

        let hit = out
            .iter()
            .find(|i| i.text.starts_with("Hit."))
            .expect("expected a damage intent");
        assert_eq!(hit.priority, Priority::Critical);
        assert!(hit.text.contains("2.0 left"), "got {:?}", hit.text);
    }

    #[test]
    fn low_health_warns_once_per_crossing() {
        let mut a = Alttp::new();
        a.on_frame(&playing(24));

        let out = a.on_frame(&playing(4));
        assert!(out.iter().any(|i| i.text.starts_with("Low health")));

        // Still low, but already warned.
        let out = a.on_frame(&playing(4));
        assert!(!out.iter().any(|i| i.text.starts_with("Low health")));

        // Heal above the threshold, then drop again: warns afresh.
        a.on_frame(&playing(24));
        let out = a.on_frame(&playing(4));
        assert!(out.iter().any(|i| i.text.starts_with("Low health")));
    }

    #[test]
    fn death_reports_and_suppresses_the_rest() {
        let mut a = Alttp::new();
        a.on_frame(&playing(8));
        let out = a.on_frame(&ram_with(&[(0x7E0010, 0x12)]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "You died.");
    }

    #[test]
    fn menu_transitions_are_not_reported_as_damage() {
        // Health reads as zero outside play; that must not sound like a hit.
        let mut a = Alttp::new();
        a.on_frame(&playing(24));
        let out = a.on_frame(&ram_with(&[(0x7E0010, 0x01)])); // file select
        assert!(!out.iter().any(|i| i.text.starts_with("Hit.")));
    }

    #[test]
    fn transitions_do_not_count_as_in_play() {
        let mut s = State::read(&playing(24)).unwrap();
        assert!(s.in_play());
        s.submodule = 3;
        assert!(!s.in_play(), "mid-transition is not in play");
    }
}
