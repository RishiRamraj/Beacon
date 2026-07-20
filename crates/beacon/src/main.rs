//! Beacon: a SNES emulator with accessibility as a first class feature.

mod alttp;
mod app;
mod audio;
mod input;

use std::path::PathBuf;

use beacon_config::Settings;
use beacon_emu::Emulator;
use beacon_output::sink::{Fanout, JsonSink, SpeechSink};
use beacon_output::{Arbiter, Config};

fn usage() -> ! {
    eprintln!(
        "\
Beacon - an accessible SNES emulator

usage: beacon <rom.sfc> [options]

options:
  --headless <frames>   run without a window, for testing and benchmarking
  --json                emit line delimited JSON events on stdout
  --quiet               no speech, useful with --json
  --rate <-100..100>    speech rate; overrides the saved setting

controls:
  arrows                d-pad            enter    start
  z x a s               B A Y X          rshift   select
  q w                   L R

  c   scan              e   where am I
  h   status            v   cycle verbosity
  r   repeat last       esc quit

Settings live at {}, and every value can also be changed while playing.",
        Settings::default_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "the user config directory".into())
    );
    std::process::exit(2)
}

struct Args {
    rom: PathBuf,
    headless: Option<u64>,
    json: bool,
    quiet: bool,
    rate: Option<i8>,
}

fn parse_args() -> Args {
    let mut rom = None;
    let mut args = Args {
        rom: PathBuf::new(),
        headless: None,
        json: false,
        quiet: false,
        rate: None,
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--headless" => {
                args.headless = Some(
                    it.next()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| usage()),
                );
            }
            "--json" => args.json = true,
            "--quiet" => args.quiet = true,
            "--rate" => {
                args.rate = Some(
                    it.next()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| usage()),
                );
            }
            "-h" | "--help" => usage(),
            other if other.starts_with('-') => usage(),
            other => rom = Some(PathBuf::from(other)),
        }
    }

    args.rom = rom.unwrap_or_else(|| usage());
    args
}

/// Builds the speech sinks, tolerating the absence of any of them.
///
/// A missing screen reader must never prevent the emulator from starting. It
/// degrades what Beacon can tell you; it does not stop you playing.
fn build_speech(settings: &Settings, args: &Args) -> Fanout {
    let mut fanout = Fanout::new();

    if args.json || settings.speech.json_events {
        fanout.push(Box::new(JsonSink::new(std::io::stdout())));
    }

    if args.quiet || !settings.speech.enabled {
        return fanout;
    }

    #[cfg(unix)]
    {
        use beacon_output::sink::SpeechDispatcherSink;
        match SpeechDispatcherSink::connect() {
            Ok(mut sink) => {
                let rate = args.rate.unwrap_or(settings.speech.rate);
                if let Err(e) = sink.set_rate(rate) {
                    eprintln!("could not set speech rate: {e}");
                }
                if !settings.speech.module.is_empty() {
                    if let Err(e) = sink.set_module(&settings.speech.module) {
                        eprintln!("could not set speech module: {e}");
                    }
                }
                fanout.push(Box::new(sink));
            }
            Err(e) => eprintln!("speech unavailable: {e}\n  (is speech-dispatcher running?)"),
        }
    }

    fanout
}

/// Runs without a window. Used for benchmarking and for replay testing, both of
/// which want the frame loop without the presentation.
fn run_headless(
    mut emu: Emulator,
    mut arbiter: Arbiter,
    mut speech: Fanout,
    frames: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Instant;

    let mut game = alttp::Alttp::new();
    let mut audio = Vec::new();
    let start = Instant::now();

    for n in 0..frames {
        // Tap start so the game walks out of the title screen unattended.
        let buttons = if n > 120 && (n / 20) % 2 == 0 {
            beacon_emu::button::START
        } else {
            0
        };
        emu.set_buttons(0, buttons);
        emu.run_frame();

        audio.clear();
        emu.drain_audio(&mut audio);

        let intents = game.on_frame(emu.main_ram()?);
        if !intents.is_empty() {
            // Time from the frame counter, not the clock, so a replay of the
            // same inputs arbitrates identically.
            let now = std::time::Duration::from_secs_f64(n as f64 / 60.098);
            for utterance in arbiter.resolve(intents, now) {
                // Human readable progress goes to stderr; stdout is reserved
                // for the JSON event stream so it stays machine parseable.
                eprintln!("frame {n:>6}  {}", utterance.text);
                let _ = speech.speak(&utterance);
            }
        }
    }

    let elapsed = start.elapsed();
    let fps = frames as f64 / elapsed.as_secs_f64();
    eprintln!(
        "\n{frames} frames in {:.2}s = {fps:.0} fps ({:.1}x realtime)",
        elapsed.as_secs_f64(),
        fps / 60.098
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();

    let settings = match Settings::default_path() {
        Some(path) => Settings::load(&path).unwrap_or_else(|e| {
            eprintln!("{e}; using defaults");
            Settings::default()
        }),
        None => Settings::default(),
    };

    let emu = Emulator::load(&args.rom)?;
    let arbiter = Arbiter::new(Config::from(&settings.arbiter));
    let speech = build_speech(&settings, &args);

    if let Some(frames) = args.headless {
        return run_headless(emu, arbiter, speech, frames);
    }

    let audio = audio::Audio::new(beacon_emu::AUDIO_SAMPLE_RATE)?;
    let mut app = app::App::new(emu, audio, arbiter, speech, settings);

    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app)?;
    Ok(())
}
