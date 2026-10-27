//! The API as a Model Context Protocol server: MCP's JSON-RPC lifecycle and seven tools over a
//! [`Backend`] that answers the API's own methods (a host in this process, or the app over its
//! socket). Transport-free: [`serve_mcp`] reads and writes newline-delimited JSON.

#[cfg(not(target_arch = "wasm32"))]
mod socket;
mod tools;

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::dispatch::{Host, call};
use crate::protocol::{RpcError, codes, failure, parse_value, success};

#[cfg(not(target_arch = "wasm32"))]
pub use socket::SocketBackend;

/// The MCP versions this server speaks, newest first. A client asking for another gets the
/// newest.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The first version whose tool results carry `structuredContent`.
const STRUCTURED_SINCE: &str = "2025-06-18";

const INSTRUCTIONS: &str = "A SketchUp-style 3D modeler. Units are meters, Z is up, angles are in degrees. Change the model with transact (several commands run as one atomic undo step by default), read it with query, see it with render_view, and drive the drawing tools like a person with tool_input. Call describe for the full command and query schemas.";

/// What answers the API's methods (`execute`, `batch`, `query`, ...) for the MCP tools.
pub trait Backend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError>;
}

/// A backend over a host in this process.
pub struct HostBackend<'a>(pub &'a mut dyn Host);

impl Backend for HostBackend<'_> {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        call(self.0, method, params)
    }
}

/// Serves MCP over `input` and `output`, one JSON-RPC message per line, until `input` ends.
/// Nothing but protocol messages is written to `output`.
pub fn serve_mcp(
    backend: &mut dyn Backend,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut server = Server { version: None };
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if input.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        // Invalid UTF-8 is replaced, so the line fails to parse with a reply instead of ending
        // the session.
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(reply) = server.handle(backend, line) {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

struct Server {
    /// The negotiated protocol version, once `initialize` was answered.
    version: Option<&'static str>,
}

impl Server {
    /// The reply line to one message, or None for a notification or a client's reply.
    fn handle(&mut self, backend: &mut dyn Backend, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(failure(
                    &Value::Null,
                    RpcError::new(codes::PARSE_ERROR, format!("parse error: {e}")),
                ));
            }
        };
        if let Some(m) = message.as_object()
            && !m.contains_key("method")
            && (m.contains_key("result") || m.contains_key("error"))
        {
            // A reply to a request of ours; this server sends none, so it is dropped.
            return None;
        }
        let request = match parse_value(&message) {
            Ok(r) => r,
            Err(reply) => return Some(reply),
        };
        let result = self.method(backend, &request.method, request.params);
        let id = request.id?;
        Some(match result {
            Ok(value) => success(&id, value),
            Err(error) => failure(&id, error),
        })
    }

    fn method(
        &mut self,
        backend: &mut dyn Backend,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = PROTOCOL_VERSIONS
                    .into_iter()
                    .find(|v| Some(*v) == asked)
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                self.version = Some(version);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "tenon", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            _ => {
                let Some(version) = self.version else {
                    return Err(RpcError::new(
                        codes::INVALID_REQUEST,
                        "call initialize first",
                    ));
                };
                match method {
                    "tools/list" => Ok(json!({"tools": tools::list()})),
                    "tools/call" => tools::call(backend, &params, version >= STRUCTURED_SINCE),
                    other => Err(RpcError::new(
                        codes::METHOD_NOT_FOUND,
                        format!("unknown method {other:?}"),
                    )),
                }
            }
        }
    }
}
