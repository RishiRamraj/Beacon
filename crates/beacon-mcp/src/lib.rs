//! A small, synchronous MCP server over stdio.
//!
//! Enough of the [Model Context Protocol](https://modelcontextprotocol.io) to let
//! an agent list and call tools: the `initialize` handshake, `tools/list`, and
//! `tools/call`, as newline-delimited JSON-RPC on a reader/writer pair. It is
//! deliberately synchronous and dependency-light — no async runtime — because the
//! thing it fronts (an emulator frame loop) is synchronous, and one blocking
//! reader thread is the whole concurrency story.
//!
//! The protocol is kept entirely separate from what the tools *do*: a [`Handler`]
//! supplies the tool list and executes calls. Beacon implements one that talks to
//! a running session; this crate knows nothing about emulation.

use std::io::{BufRead, Write};

use serde::Serialize;
use serde_json::{json, Value};

/// The protocol version this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// One tool an agent can call.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl ToolDef {
    pub fn new(name: &str, description: &str, input_schema: Value) -> Self {
        ToolDef {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
        }
    }
}

/// What the server delegates to: the tools, and how to run them.
pub trait Handler {
    /// The tools to advertise in `tools/list`.
    fn tools(&self) -> Vec<ToolDef>;

    /// Runs a tool. `Ok` is reported as a successful result, `Err` as a tool
    /// error (`isError: true`) — a tool that failed, not a broken protocol.
    fn call(&self, name: &str, args: &Value) -> Result<Value, String>;
}

/// Server metadata reported in the `initialize` response.
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// Runs the server loop until the input closes.
///
/// Reads one JSON-RPC message per line, dispatches it, and writes one response
/// line for each request (notifications get none). Malformed input is answered
/// with a parse error rather than closing the connection, so one bad line does
/// not end the session.
pub fn serve<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    info: ServerInfo,
    handler: &dyn Handler,
) -> std::io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_message(&mut writer, &parse_error(&e.to_string()))?;
                continue;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");

        // A message without an id is a notification: act on it, answer nothing.
        let Some(id) = id else {
            continue;
        };

        let response = match method {
            "initialize" => success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": info.name, "version": info.version },
                }),
            ),
            "ping" => success(id, json!({})),
            "tools/list" => success(id, json!({ "tools": handler.tools() })),
            "tools/call" => {
                let params = msg.get("params");
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = params
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or(Value::Null);
                match handler.call(name, &args) {
                    Ok(value) => success(id, tool_content(&value, false)),
                    Err(message) => success(id, tool_content(&Value::String(message), true)),
                }
            }
            other => error(id, -32601, &format!("method not found: {other}")),
        };

        write_message(&mut writer, &response)?;
    }
    Ok(())
}

/// Wraps a tool result as MCP content. The value is serialized to a text block,
/// so a structured result reaches the agent as JSON it can parse.
fn tool_content(value: &Value, is_error: bool) -> Value {
    let text = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn parse_error(message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": { "code": -32700, "message": format!("parse error: {message}") },
    })
}

fn write_message<W: Write>(writer: &mut W, message: &Value) -> std::io::Result<()> {
    writeln!(writer, "{}", serde_json::to_string(message)?)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Echo;
    impl Handler for Echo {
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef::new(
                "echo",
                "echoes its argument",
                json!({ "type": "object", "properties": { "value": { "type": "string" } } }),
            )]
        }
        fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
            match name {
                "echo" => Ok(json!({ "echoed": args.get("value") })),
                "fail" => Err("no such widget".into()),
                other => Err(format!("unknown tool {other}")),
            }
        }
    }

    fn run(input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        serve(
            Cursor::new(input.as_bytes().to_vec()),
            &mut out,
            ServerInfo {
                name: "test".into(),
                version: "0".into(),
            },
            &Echo,
        )
        .unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn initialize_reports_tool_capability() {
        let out = run(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        assert_eq!(out[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(out[0]["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_the_handlers_tools() {
        let out = run(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert_eq!(out[0]["result"]["tools"][0]["name"], "echo");
    }

    #[test]
    fn tools_call_wraps_success_and_error() {
        let ok = run(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"value":"hi"}}}"#,
        );
        assert_eq!(ok[0]["result"]["isError"], false);
        // The text block is the JSON-encoded structured result.
        let text = ok[0]["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["echoed"], "hi");

        let err = run(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fail","arguments":{}}}"#,
        );
        assert_eq!(err[0]["result"]["isError"], true);
        assert_eq!(err[0]["result"]["content"][0]["text"], "no such widget");
    }

    #[test]
    fn notifications_get_no_response() {
        // No id => notification => no output line.
        let out = run(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert!(out.is_empty());
    }

    #[test]
    fn unknown_method_is_a_jsonrpc_error() {
        let out = run(r#"{"jsonrpc":"2.0","id":5,"method":"frobnicate"}"#);
        assert_eq!(out[0]["error"]["code"], -32601);
    }
}
