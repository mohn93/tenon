//! JSON-RPC 2.0 message helpers.

use serde_json::{Value, json};

/// Longest accepted request line in bytes; longer lines get an invalid-request reply and are
/// skipped.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Error codes: the JSON-RPC standard ones plus application errors in -32000..-32099.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// A command or query failed; `data` is the session `CommandError`.
    pub const COMMAND_FAILED: i64 = -32000;
    /// A batch stopped; `data` is the session `BatchError`.
    pub const BATCH_FAILED: i64 = -32001;
    /// `render_view` was asked for but no GPU is available.
    pub const RENDER_UNAVAILABLE: i64 = -32002;
    /// The MCP adapter lost its connection to the app; every later call fails with it.
    pub const CONNECTION_LOST: i64 = -32003;
}

/// An error to send back: code, message and optional structured data.
#[derive(Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

pub fn success(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

pub fn failure(id: &Value, error: RpcError) -> String {
    let mut e = json!({"code": error.code, "message": error.message});
    if let Some(data) = error.data {
        e["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": e}).to_string()
}

pub fn notification(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// A parsed request: `id` is None for notifications (no reply expected).
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// Parses one line. On failure returns the reply to send (with a null id when unknown).
pub fn parse(line: &str) -> Result<Request, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| {
        failure(
            &Value::Null,
            RpcError::new(codes::PARSE_ERROR, format!("parse error: {e}")),
        )
    })?;
    parse_value(&v)
}

/// [`parse`] for a line already parsed as JSON.
pub fn parse_value(v: &Value) -> Result<Request, String> {
    let invalid = |id: &Value, why: &str| {
        failure(
            id,
            RpcError::new(codes::INVALID_REQUEST, format!("invalid request: {why}")),
        )
    };
    let Some(obj) = v.as_object() else {
        return Err(invalid(&Value::Null, "expected a JSON object"));
    };
    let id = obj.get("id").cloned();
    if matches!(
        &id,
        Some(Value::Bool(_) | Value::Array(_) | Value::Object(_))
    ) {
        return Err(invalid(&Value::Null, "id must be a string, number or null"));
    }
    let reply_id = id.clone().unwrap_or(Value::Null);
    if obj.get("jsonrpc") != Some(&json!("2.0")) {
        return Err(invalid(&reply_id, "jsonrpc must be \"2.0\""));
    }
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Err(invalid(&reply_id, "method must be a string"));
    };
    Ok(Request {
        id,
        method: method.to_string(),
        params: obj.get("params").cloned().unwrap_or(Value::Null),
    })
}
