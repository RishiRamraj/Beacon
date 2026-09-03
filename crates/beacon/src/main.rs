//! Beacon: a SNES emulator with accessibility as a first class feature.

mod access;
mod action;
mod app;
mod audio;
mod beacons;
mod config_modal;
mod image;
mod input;
mod mcp;
mod menu;
// Windows and macOS have a real menu widget; Linux has none reachable from a winit window.
#[cfg(any(windows, target_os = "macos"))]
mod native;
mod rom;
mod session;
mod state;

use std::path::PathBuf;
use std::rc::Rc;

use beacon_config::Settings;
use beacon_emu::Emulator;
use beacon_output::sink::{Fanout, JsonSink, SpeechSink};
use beacon_output::{Arbiter, Config};
use beacon_plugin::Plugin;

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
  --map                 start with the plugin's map beside the game (toggle: m)
  --map-only            show only the plugin's map, no game picture
  --control             serve the MCP control protocol on a local socket while
                        the window runs, so an agent can assist a live session
  --connect             no ROM; bridge stdio to a running --control session's
                        socket, so a stdio MCP client can drive that live window

game controls (fixed):
  arrows                d-pad            enter    start
  z x a s               B A Y X          rshift   select
  q w                   L R

action keys (default, all rebindable):
  c   scan              e   where am I      h   status
  t   save state        g   load state      n/b next/prev slot
  p   pause             f   frame advance   v   cycle verbosity
  j   mute
  r   repeat last       k   input config    esc quit
  tab menu

The menu (tab, or the guide button on a pad) reaches File (open a ROM, exit), Save
and Load by slot, and Input. Up and down move, right or enter chooses, left goes
back, escape closes. Every entry says where it is in its list and whether it
leads to a submenu, so the shape of the menu is audible.

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
    /// `None` starts with no game, so a ROM can be found from the menu. The usual way in on a
    /// desktop, where nobody types a path.
    rom: Option<PathBuf>,
    headless: Option<u64>,
    json: bool,
    quiet: bool,
    rate: Option<i8>,
    mcp: bool,
    map: bool,
    map_only: bool,
    control: bool,
    connect: bool,
}

fn parse_args() -> Args {
    let mut rom = None;
    let mut args = Args {
        rom: None,
        headless: None,
        json: false,
        quiet: false,
        rate: None,
        mcp: false,
        map: false,
        map_only: false,
        control: false,
        connect: false,
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
            "--map-only" => args.map_only = true,
            "--control" => args.control = true,
            "--connect" => args.connect = true,
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

    // Only the modes that run frames unattended need one up front: with a window there is a
    // menu to open a ROM from, but nothing is going to pick one for a benchmark.
    if rom.is_none() && args.headless.is_some() {
        eprintln!("--headless needs a ROM to run");
        usage()
    }
    args.rom = rom;
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

    // Windows has no speech-dispatcher. The equivalent is the player's screen reader, which
    // is where their voice, rate and punctuation settings live — so Beacon speaks through it
    // rather than choosing a voice of its own, the same principle as inheriting Orca's.
    //
    // Both libraries are DLLs that ship beside the executable, so a missing one is ordinary
    // rather than an error: Beacon says so and carries on with the JSON stream, which is
    // exactly what that stream is the insurance policy for.
    #[cfg(windows)]
    {
        use beacon_output::sink::ScreenReaderSink;
        match ScreenReaderSink::connect() {
            Ok(sink) => {
                eprintln!("speech: {}", sink.name());
                fanout.push(Box::new(sink));
            }
            Err(e) => eprintln!(
                "speech unavailable: {e}\n  (put Tolk.dll, or nvdaControllerClient64.dll, \
                 beside beacon.exe)"
            ),
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

    // The stdio<->socket bridge needs nothing else — no ROM, emulator, or plugin.
    if args.connect {
        #[cfg(unix)]
        return mcp::connect_bridge().map_err(Into::into);
        // Refused rather than ignored: an agent harness that asked to bridge and got a
        // silently ordinary startup would wait for a protocol that is never coming.
        #[cfg(not(unix))]
        {
            eprintln!("--connect needs a Unix socket and is not available on this platform");
            std::process::exit(2);
        }
    }

    let settings = match Settings::default_path() {
        Some(path) => Settings::load(&path).unwrap_or_else(|e| {
            eprintln!("{e}; using defaults");
            Settings::default()
        }),
        None => Settings::default(),
    };

    let arbiter = Arbiter::new(Config::from(&settings.arbiter));
    let speech = build_speech(&settings, &args);
    let rom = match args.rom.as_ref() {
        Some(path) => rom::read(path),
        None => Rc::new(Vec::new()),
    };
    let sha1 = (!rom.is_empty()).then(|| beacon_plugin::rom_sha1(&rom));
    let (plugin, reload_spec) = rom::select_plugin(sha1.as_deref(), &rom);

    if let Some(frames) = args.headless {
        // parse_args refuses --headless without a ROM, so this is always Some. No device to
        // consult without a window, so the default rate stands.
        let path = args.rom.as_ref().expect("--headless requires a ROM");
        return run_headless(Emulator::load(path)?, arbiter, speech, plugin, frames);
    }

    // The device opens BEFORE the emulator, because the rate it settles on is the rate the
    // emulator has to produce. Asking a device to run at the emulator's rate instead is what
    // failed on Windows, where WASAPI shared mode serves only its own mix format.
    let audio = audio::Audio::new(beacon_emu::AUDIO_SAMPLE_RATE)?;
    let rate = audio.sample_rate();

    // No ROM is a legitimate start: an empty machine with a menu on it.
    let emu = match args.rom.as_ref() {
        Some(path) => Some(Emulator::load_at(path, rate)?),
        None => None,
    };
    let mut session = session::Session::new(
        emu,
        audio,
        arbiter,
        speech,
        plugin,
        reload_spec,
        rom.clone(),
        args.rom.clone(),
        settings,
        sha1.as_deref(),
    );
    // Fill any keys a built-in action or the plugin suggests, without overriding the
    // user's own bindings. Built-ins first, so a plugin command cannot take the key a
    // new host action was about to get.
    session.apply_builtin_default_keys();
    session.apply_plugin_default_keys();
    if args.map || args.map_only {
        session.show_map_at_start();
    }

    // MCP mode runs the same session with no window, driven by an agent over
    // stdio. Audio and speech still play, so a blind player hears the game while
    // the agent handles setup and assistance.
    if args.mcp {
        return mcp::run(session);
    }

    // With --control, an agent can attach to this live windowed session over a
    // local socket and drive it while the player keeps playing.
    #[cfg(unix)]
    let control_rx = if args.control {
        let path = mcp::control_socket_path();
        match mcp::serve_socket(&path) {
            Ok(rx) => {
                eprintln!("beacon: control socket ready at {}", path.display());
                Some(rx)
            }
            Err(e) => {
                eprintln!("could not open control socket: {e}");
                None
            }
        }
    } else {
        None
    };
    // Said out loud, because a player following instructions that mention --control should
    // hear why it did nothing rather than wonder whether it worked.
    #[cfg(not(unix))]
    let control_rx = {
        if args.control {
            eprintln!("--control needs a Unix socket and is not available on this platform");
        }
        None
    };

    // A user-event loop, because the accessibility adapter wakes it: assistive technology
    // asks for the tree from its own thread, and the reply has to come from this one.
    let event_loop =
        winit::event_loop::EventLoop::<accesskit_winit::Event>::with_user_event().build()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = app::App::new(
        session,
        input::Input::new(),
        args.map_only,
        control_rx,
        event_loop.create_proxy(),
    );
    event_loop.run_app(&mut app)?;
    Ok(())
}
