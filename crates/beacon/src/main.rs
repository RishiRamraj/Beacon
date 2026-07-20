//! Beacon - phase 0 harness.
//!
//! This is not the emulator yet. It loads a ROM, runs frames, and reads work
//! RAM between them, which is the one property the whole design rests on: the
//! instrumentation runs in-process against real memory rather than sampling an
//! emulator over a socket.
//!
//! The ALttP readings below are a throwaway plugin standing in for the real
//! plugin runtime. They exist to prove the frame hook, not to be kept.

use std::path::PathBuf;
use std::time::Instant;

use beacon_emu::Emulator;

/// SNES bank $7E is the first 64 KiB of work RAM, bank $7F the second.
/// Converts an A-bus address such as `$7EF36D` into a work RAM offset.
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
    let lo = *ram.get(o)? as u16;
    let hi = *ram.get(o + 1)? as u16;
    Some(lo | (hi << 8))
}

/// A Link to the Past state, read straight out of work RAM.
struct Alttp {
    module: u8,
    health: u8,
    max_health: u8,
    x: u16,
    y: u16,
}

impl Alttp {
    fn read(ram: &[u8]) -> Option<Self> {
        Some(Alttp {
            module: u8_at(ram, 0x7E0010)?,
            health: u8_at(ram, 0x7EF36D)?,
            max_health: u8_at(ram, 0x7EF36C)?,
            x: u16_at(ram, 0x7E0022)?,
            y: u16_at(ram, 0x7E0020)?,
        })
    }

    /// Health is stored in eighths of a heart.
    fn hearts(&self) -> f32 {
        self.health as f32 / 8.0
    }

    fn max_hearts(&self) -> f32 {
        self.max_health as f32 / 8.0
    }
}

/// ALttP's main module, the closest thing the game has to a top-level state.
fn module_name(m: u8) -> &'static str {
    match m {
        0x00 => "intro",
        0x01 => "file select",
        0x02 => "copy file",
        0x03 => "erase file",
        0x04 => "name file",
        0x05 => "loading game",
        0x06 => "pre-dungeon",
        0x07 => "dungeon",
        0x08 => "pre-overworld",
        0x09 => "overworld",
        0x0e => "text / menu",
        0x12 => "death",
        0x14 => "attract mode",
        0x19 => "triforce room",
        _ => "?",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let rom: PathBuf = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: beacon <rom.sfc> [frames]");
            std::process::exit(2);
        }
    };
    let frames: u64 = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
        .unwrap_or(600);

    let mut emu = Emulator::load(&rom)?;
    let ram = emu.main_ram()?;
    println!(
        "loaded {}\n  region {}  work RAM {} KiB",
        rom.display(),
        emu.region(),
        ram.len() / 1024
    );

    // Advance frames, reading state between each one.
    //
    // Start is tapped periodically to walk the title screen through to the
    // file select, so the readings below show a game that is actually running
    // rather than a boot screen. Real input arrives with the host.
    let start = Instant::now();
    let mut prev: Option<Alttp> = None;
    for _ in 0..frames {
        let n = emu.frame_count();
        let tapping_start = n > 120 && (n / 20) % 2 == 0;
        emu.set_buttons(
            0,
            if tapping_start {
                beacon_emu::button::START
            } else {
                0
            },
        );

        let n = emu.run_frame();

        // Frame-to-frame diffing, which is what the real event detector does:
        // compare this frame's state to the last and report what changed.
        if let Some(s) = Alttp::read(emu.main_ram()?) {
            if prev.as_ref().map(|p: &Alttp| p.module) != Some(s.module) {
                println!(
                    "frame {n:>5}  module {:#04x} -> {:#04x}  {}",
                    prev.as_ref().map(|p| p.module).unwrap_or(0),
                    s.module,
                    module_name(s.module),
                );
                if s.module == 0x07 || s.module == 0x09 {
                    println!("             position ({}, {})", s.x, s.y);
                }
            }
            if prev.as_ref().map(|p| p.health) != Some(s.health) && s.max_health > 0 {
                println!(
                    "frame {n:>5}  health {:.1}/{:.1} hearts",
                    s.hearts(),
                    s.max_hearts()
                );
            }
            prev = Some(s);
        }
    }
    let elapsed = start.elapsed();

    // Emulation speed relative to the SNES's ~60.098 Hz NTSC refresh. This is
    // the number that decides whether accuracy is affordable on a given
    // machine, so phase 0 measures it rather than assuming.
    let fps = frames as f64 / elapsed.as_secs_f64();
    println!(
        "\nran {frames} frames in {:.2}s  =  {fps:.0} fps  ({:.1}x realtime)",
        elapsed.as_secs_f64(),
        fps / 60.098
    );

    // Savestates are what make replay-based regression testing possible later.
    let state = emu.save_state()?;
    println!("savestate: {} bytes", state.len());

    Ok(())
}
