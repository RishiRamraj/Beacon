//! The MCP control surface: Beacon driven by an agent.
//!
//! `beacon <rom> --mcp` runs the session with no window, serving the Model
//! Context Protocol on stdio. An agent connected to it can do everything a player
//! can — press buttons, run commands, save and load, rebind keys, walk the input
//! configuration — and read what a player would have heard, plus inspect memory
//! and step frames. That is the point: an end user can hand their whole setup and
//! play to an agent. Audio and speech still run, so the human hears the game; only
//! the (unneeded) video window is absent.
//!
//! Threading is deliberately small. A reader thread runs the protocol
//! ([`beacon_mcp::serve`]) and forwards each tool call down a channel; the main
//! thread owns the [`Session`] and is the only thing that touches it, running
//! frames when nothing is pending. So there is no shared mutable state and no lock
//! — the emulator is single-threaded, as it must be.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use beacon_mcp::{Handler, ServerInfo, ToolDef};
use serde_json::{json, Value};

use crate::session::Session;

/// One tool call in flight: the tool name, its arguments, and where to send the
/// result. The main thread runs it against the session and replies.
type Request = (String, Value, Sender<Result<Value, String>>);

/// The protocol-side handler. It owns no session state; it just forwards calls to
/// the thread that does, and waits for the answer.
struct ChannelHandler {
    tx: Sender<Request>,
}

impl Handler for ChannelHandler {
    fn tools(&self) -> Vec<ToolDef> {
        tool_defs()
    }

    fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
        let (rtx, rrx) = mpsc::channel();
        self.tx
            .send((name.to_string(), args.clone(), rtx))
            .map_err(|_| "session has shut down".to_string())?;
        rrx.recv()
            .map_err(|_| "no response from session".to_string())?
    }
}

/// Runs the session headless under MCP until stdin closes or a quit is requested.
pub fn run(mut session: Session) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx): (Sender<Request>, Receiver<Request>) = mpsc::channel();

    // The protocol reader runs on its own thread: it blocks on stdin, and must
    // not hold up the frame loop.
    let handler = ChannelHandler { tx };
    let reader = std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let info = ServerInfo {
            name: "beacon".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        // Ends when stdin closes (the agent disconnects), which drops the sender
        // and unblocks the main loop below.
        let _ = beacon_mcp::serve(stdin.lock(), stdout.lock(), info, &handler);
    });

    eprintln!(
        "beacon: MCP server ready on stdio ({} tools)",
        tool_defs().len()
    );

    loop {
        // Serve everything already queued before running any frames, so a burst
        // of calls is handled promptly.
        while let Ok((name, args, resp)) = rx.try_recv() {
            let _ = resp.send(dispatch(&mut session, &name, &args));
        }
        if session.quit_requested() {
            break;
        }

        // Run frames while playing, then wait briefly for the next call. While
        // paused or configuring, just wait — nothing advances on its own.
        let idle = session.paused() || session.in_config();
        if !idle {
            session.run_frames();
        }
        let wait = if idle {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(2)
        };
        match rx.recv_timeout(wait) {
            Ok((name, args, resp)) => {
                let _ = resp.send(dispatch(&mut session, &name, &args));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    drop(reader); // detached; the process exits on return
    Ok(())
}

/// Runs a tool against the session. `Ok` is a structured result; `Err` is a tool
/// error the agent sees as a failed call.
fn dispatch(s: &mut Session, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "get_state" => Ok(json!({
            "frame": s.frame_count(),
            "paused": s.paused(),
            "in_config": s.in_config(),
            "active_slot": s.active_slot_index(),
            "plugin": s.plugin_name(),
        })),

        "recent_speech" => Ok(json!({ "spoken": s.take_speech() })),

        "get_map" => match s.render_map() {
            Some((w, h)) => {
                let png = crate::image::encode_png(w, h, s.map_pixels());
                // An image content block, so the agent sees the plugin's map.
                Ok(json!({
                    "type": "image",
                    "data": crate::image::base64(&png),
                    "mimeType": "image/png",
                }))
            }
            None => Err("this game's plugin draws no map".to_string()),
        },

        "read_memory" => {
            let addr = arg_u32(args, "address")?;
            let len = arg_u64(args, "length").unwrap_or(1) as usize;
            match s.read_wram(addr, len) {
                Some(bytes) => Ok(json!({ "address": addr, "bytes": bytes })),
                None => Err(format!("0x{addr:06X}..+{len} is outside mapped work RAM")),
            }
        }

        "step" => {
            let n = arg_u64(args, "count").unwrap_or(1) as u32;
            s.step_frames(n);
            Ok(json!({ "frame": s.frame_count(), "spoken": s.take_speech() }))
        }

        "pause" => {
            s.set_paused(true);
            Ok(json!({ "paused": true }))
        }
        "resume" => {
            s.set_paused(false);
            Ok(json!({ "paused": false }))
        }

        "set_buttons" => {
            let names = arg_str_array(args, "buttons")?;
            let mut mask = 0u16;
            for n in &names {
                match crate::input::snes_button_from_name(n) {
                    Some(bit) => mask |= bit,
                    None => return Err(format!("unknown SNES button '{n}'")),
                }
            }
            s.set_held_buttons(mask);
            Ok(json!({ "held": names }))
        }

        "run_command" => {
            let id = arg_string(args, "id")?;
            s.run_command(&id);
            Ok(json!({ "spoken": s.take_speech() }))
        }

        "save_state" => {
            s.save_state();
            Ok(json!({ "spoken": s.take_speech() }))
        }
        "load_state" => {
            s.load_state();
            Ok(json!({ "spoken": s.take_speech() }))
        }
        "set_slot" => {
            let slot = arg_u64(args, "slot").ok_or("missing 'slot'")? as u8;
            s.set_active_slot(slot);
            Ok(json!({ "active_slot": s.active_slot_index() }))
        }

        "list_actions" => {
            let actions: Vec<Value> = s
                .bindable_actions()
                .into_iter()
                .map(|b| {
                    let keys = s.keys_for_action(&b.id);
                    json!({ "id": b.id, "label": b.label, "keys": keys })
                })
                .collect();
            Ok(json!({ "actions": actions }))
        }
        "get_bindings" => {
            let bindings: Vec<Value> = s
                .bindings()
                .into_iter()
                .map(|(input, action)| json!({ "input": input, "action": action }))
                .collect();
            Ok(json!({ "bindings": bindings }))
        }
        "bind" => {
            let input = arg_string(args, "input")?;
            let action = arg_string(args, "action")?;
            s.bind(&input, &action)?;
            Ok(json!({ "input": input, "action": action }))
        }
        "unbind" => {
            let input = arg_string(args, "input")?;
            s.unbind(&input);
            Ok(json!({ "input": input }))
        }

        "get_setting" => {
            let key = arg_string(args, "key")?;
            Ok(json!({ "key": key, "value": s.get_setting(&key)? }))
        }
        "set_setting" => {
            let key = arg_string(args, "key")?;
            let value = arg_string(args, "value")?;
            s.set_setting(&key, &value)?;
            Ok(json!({ "key": key, "value": value }))
        }

        // Driving the configuration modal headlessly, exactly as a player would:
        // each call speaks, and the agent reads the announcement back.
        "open_config" => {
            s.open_input_config();
            Ok(json!({ "spoken": s.take_speech() }))
        }
        "config_navigate" => {
            let delta = arg_i64(args, "delta").unwrap_or(1) as i32;
            s.config_navigate(delta);
            Ok(json!({ "spoken": s.take_speech() }))
        }
        "config_bind" => {
            let input = arg_string(args, "input")?;
            s.config_bind(&input);
            Ok(json!({ "spoken": s.take_speech() }))
        }
        "config_clear" => {
            s.config_clear();
            Ok(json!({ "spoken": s.take_speech() }))
        }
        "config_close" => {
            s.config_close();
            Ok(json!({ "spoken": s.take_speech() }))
        }

        other => Err(format!("unknown tool: {other}")),
    }
}

// --- Argument helpers ------------------------------------------------------

/// A u32 argument, accepting a JSON number or a hex/decimal string so an agent
/// can pass `"0x7EF36D"` or `8319341`.
fn arg_u32(args: &Value, key: &str) -> Result<u32, String> {
    let v = args.get(key).ok_or_else(|| format!("missing '{key}'"))?;
    if let Some(n) = v.as_u64() {
        return Ok(n as u32);
    }
    if let Some(s) = v.as_str() {
        let s = s.trim();
        let parsed = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16)
        } else {
            s.parse::<u32>()
        };
        return parsed.map_err(|_| format!("'{key}' is not a valid address"));
    }
    Err(format!("'{key}' must be a number or a hex string"))
}

fn arg_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

fn arg_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing string '{key}'"))
}

fn arg_str_array(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing array '{key}'"))?;
    arr.iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("'{key}' must be an array of strings"))
        })
        .collect()
}

// --- Tool catalogue --------------------------------------------------------

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

/// The tools advertised to the agent. Descriptions are written for the agent to
/// read, since they are how it learns what Beacon can do.
fn tool_defs() -> Vec<ToolDef> {
    let none = || json!({ "type": "object", "properties": {} });
    vec![
        ToolDef::new(
            "get_state",
            "Current frame count, whether paused or configuring, the active save slot, and the loaded plugin.",
            none(),
        ),
        ToolDef::new(
            "recent_speech",
            "Drain and return the lines Beacon has spoken since the last read — what the player would have heard.",
            none(),
        ),
        ToolDef::new(
            "get_map",
            "Render the plugin's map — its visual interpretation of memory — and return it as a PNG image. Errors if the plugin draws no map.",
            none(),
        ),
        ToolDef::new(
            "read_memory",
            "Read work RAM by SNES address (e.g. 0x7EF36D). Returns the bytes, or an error if the range is unmapped.",
            obj(
                json!({
                    "address": { "type": ["integer", "string"], "description": "SNES address, number or hex string" },
                    "length": { "type": "integer", "description": "bytes to read (default 1)" }
                }),
                &["address"],
            ),
        ),
        ToolDef::new(
            "step",
            "Pause and advance N frames (default 1), running the plugin over each. Returns the new frame and anything spoken.",
            obj(json!({ "count": { "type": "integer" } }), &[]),
        ),
        ToolDef::new("pause", "Halt emulation.", none()),
        ToolDef::new("resume", "Resume emulation.", none()),
        ToolDef::new(
            "set_buttons",
            "Set the SNES buttons held from now on (A B X Y L R Start Select Up Down Left Right). Empty releases all.",
            obj(
                json!({ "buttons": { "type": "array", "items": { "type": "string" } } }),
                &["buttons"],
            ),
        ),
        ToolDef::new(
            "run_command",
            "Run a plugin command (scan, where, status, or a custom one) and return what it says.",
            obj(json!({ "id": { "type": "string" } }), &["id"]),
        ),
        ToolDef::new("save_state", "Save to the active slot.", none()),
        ToolDef::new("load_state", "Load the active slot.", none()),
        ToolDef::new(
            "set_slot",
            "Choose the active save slot (0-9).",
            obj(json!({ "slot": { "type": "integer" } }), &["slot"]),
        ),
        ToolDef::new(
            "list_actions",
            "Every bindable action, with its label and the keys currently bound to it.",
            none(),
        ),
        ToolDef::new("get_bindings", "Every current input-to-action binding.", none()),
        ToolDef::new(
            "bind",
            "Bind an input (e.g. KeyD, F5, Pad:C) to an action id (e.g. save_state, command:scan). Refuses game controls.",
            obj(
                json!({
                    "input": { "type": "string" },
                    "action": { "type": "string" }
                }),
                &["input", "action"],
            ),
        ),
        ToolDef::new(
            "unbind",
            "Remove any binding for an input.",
            obj(json!({ "input": { "type": "string" } }), &["input"]),
        ),
        ToolDef::new(
            "get_setting",
            "Read a setting by name (e.g. speech.rate, arbiter.verbosity).",
            obj(json!({ "key": { "type": "string" } }), &["key"]),
        ),
        ToolDef::new(
            "set_setting",
            "Set a setting by name; persisted immediately.",
            obj(
                json!({ "key": { "type": "string" }, "value": { "type": "string" } }),
                &["key", "value"],
            ),
        ),
        ToolDef::new(
            "open_config",
            "Open the input configuration, as the player would. Returns the spoken prompt and first item.",
            none(),
        ),
        ToolDef::new(
            "config_navigate",
            "Move the configuration selection by delta (default 1); returns the newly selected action and its binding.",
            obj(json!({ "delta": { "type": "integer" } }), &[]),
        ),
        ToolDef::new(
            "config_bind",
            "Assign an input to the selected action in the open configuration.",
            obj(json!({ "input": { "type": "string" } }), &["input"]),
        ),
        ToolDef::new("config_clear", "Clear the selected action's bindings.", none()),
        ToolDef::new("config_close", "Close the input configuration and resume.", none()),
    ]
}
