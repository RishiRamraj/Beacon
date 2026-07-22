//! Beacon: a SNES emulator with accessibility as a first class feature.

mod action;
mod app;
mod audio;
mod config_modal;
mod image;
mod input;
mod mcp;
mod session;
mod state;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use beacon_config::Settings;
use beacon_emu::Emulator;
use beacon_output::sink::{Fanout, JsonSink, SpeechSink};
use beacon_output::{Arbiter, Config};
use beacon_plugin::{LuaPlugin, NullPlugin, Plugin, PluginSpec, Registry};

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
  --mcp                 no window; serve the MCP control protocol on stdio,
                        so an agent can drive setup and play (audio still runs)
  --map                 start with the plugin's map view shown (toggle with m)

game controls (fixed):
  arrows                d-pad            enter    start
  z x a s               B A Y X          rshift   select
  q w                   L R

action keys (default, all rebindable):
  c   scan              e   where am I      h   status
  t   save state        g   load state      n/b next/prev slot
  p   pause             f   frame advance   v   cycle verbosity
  r   repeat last       k   input config    esc quit

Press the input-config key (k, or the left stick button on a pad) to rebind
anything, including from a controller alone. Settings live at {}, and every
value can also be changed while playing.",
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
    mcp: bool,
    map: bool,
}

fn parse_args() -> Args {
    let mut rom = None;
    let mut args = Args {
        rom: PathBuf::new(),
        headless: None,
        json: false,
        quiet: false,
        rate: None,
        mcp: false,
        map: false,
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
            "--mcp" => args.mcp = true,
            "--map" => args.map = true,
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

    // In MCP mode stdout carries the protocol, so the JSON event sink must stay
    // off it; the agent gets speech through the recent_speech tool instead.
    if !args.mcp && (args.json || settings.speech.json_events) {
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

/// Directories searched for drop-in plugins, in addition to the built-ins.
///
/// A `plugins/` directory beside the executable is the shipped layout; one in
/// the working directory is the convenience during development. Both are
/// optional: a missing directory is not an error.
fn plugin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("plugins"));
        }
    }
    dirs.push(PathBuf::from("plugins"));
    dirs
}

/// Reads the ROM, stripped of any copier header, for hashing and for plugins to
/// decode static game data. Empty (with a message) if the file cannot be read.
fn read_rom(rom_path: &Path) -> Rc<Vec<u8>> {
    match std::fs::read(rom_path) {
        Ok(bytes) => Rc::new(beacon_emu::strip_copier_header(&bytes).to_vec()),
        Err(e) => {
            eprintln!("could not read ROM: {e}");
            Rc::new(Vec::new())
        }
    }
}

/// Picks the plugin matching a ROM hash, falling back to no instrumentation.
///
/// The user never chooses: identification is by headerless SHA-1. A ROM with no
/// matching plugin still plays, just silently, and a plugin that fails to load
/// is reported rather than fatal. The plugin is handed the ROM so it can decode
/// static game data at load.
fn select_plugin(sha1: Option<&str>, rom: &Rc<Vec<u8>>) -> (Box<dyn Plugin>, Option<PluginSpec>) {
    let Some(sha1) = sha1 else {
        return (Box::new(NullPlugin), None);
    };

    let mut registry = Registry::builtin();
    for dir in plugin_dirs() {
        registry.load_dir(&dir);
    }

    match registry.select(sha1) {
        Some(spec) => match LuaPlugin::load(spec, rom.clone()) {
            Ok(plugin) => {
                eprintln!("plugin: {}", plugin.name());
                // Keep the spec so the session can reload the plugin later.
                (Box::new(plugin), Some(spec.clone()))
            }
            Err(e) => {
                eprintln!("plugin failed to load, running without instrumentation: {e}");
                (Box::new(NullPlugin), None)
            }
        },
        None => {
            eprintln!("no plugin matches this ROM (sha1 {sha1}); running without instrumentation");
            (Box::new(NullPlugin), None)
        }
    }
}

/// Runs without a window. Used for benchmarking and for replay testing, both of
/// which want the frame loop without the presentation.
fn run_headless(
    mut emu: Emulator,
    mut arbiter: Arbiter,
    mut speech: Fanout,
    mut plugin: Box<dyn Plugin>,
    frames: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Instant;

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

        let intents = plugin.on_frame(emu.main_ram()?, n);
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
    let rom = read_rom(&args.rom);
    let sha1 = (!rom.is_empty()).then(|| beacon_plugin::rom_sha1(&rom));
    let (plugin, reload_spec) = select_plugin(sha1.as_deref(), &rom);

    if let Some(frames) = args.headless {
        return run_headless(emu, arbiter, speech, plugin, frames);
    }

    let audio = audio::Audio::new(beacon_emu::AUDIO_SAMPLE_RATE)?;
    let mut session = session::Session::new(
        emu,
        audio,
        arbiter,
        speech,
        plugin,
        reload_spec,
        rom.clone(),
        settings,
        sha1.as_deref().unwrap_or("unknown"),
    );
    if args.map {
        session.show_map_at_start();
    }

    // MCP mode runs the same session with no window, driven by an agent over
    // stdio. Audio and speech still play, so a blind player hears the game while
    // the agent handles setup and assistance.
    if args.mcp {
        return mcp::run(session);
    }

    let mut app = app::App::new(session, input::Input::new());

    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app)?;
    Ok(())
}
