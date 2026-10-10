//! Method dispatch: maps JSON-RPC methods onto session commands, queries and rendering.

use base64::Engine;
use schemars::schema_for;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use session::{
    BatchError, Camera, ChangeEvent, Command, CommandError, Document, Outcome, Query, QueryResult,
};

use crate::protocol::{RpcError, codes, failure, parse, success};

/// Largest image side `render_view` accepts, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 4096;

/// What the API needs from the program it runs in (the window app or headless mode).
pub trait Host {
    fn document(&self) -> &Document;
    /// Runs one command the same way the UI would.
    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError>;
    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError>;
    /// Renders the current model from `camera` to tightly packed RGBA8 pixels, or None when no
    /// GPU is available.
    fn render_view(&mut self, camera: &Camera, width: u32, height: u32) -> Option<Vec<u8>>;
}

/// Handles one request line and returns the reply line (None for notifications).
/// `subscribe` is connection-level and handled by `Hub`.
pub fn handle(host: &mut dyn Host, line: &str) -> Option<String> {
    let request = match parse(line) {
        Ok(r) => r,
        Err(reply) => return Some(reply),
    };
    let result = call(host, &request.method, request.params);
    let id = request.id?;
    Some(match result {
        Ok(value) => success(&id, value),
        Err(error) => failure(&id, error),
    })
}

fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "execute" => {
            let command: Command = params_as(params)?;
            let outcome = host.execute(command).map_err(command_failed)?;
            Ok(json!({"changes": outcome.changes, "revision": host.document().revision()}))
        }
        "batch" => {
            #[derive(Deserialize)]
            struct Params {
                commands: Vec<Command>,
                #[serde(default)]
                atomic: bool,
            }
            let p: Params = params_as(params)?;
            let outcomes = host.execute_batch(p.commands, p.atomic).map_err(|e| {
                RpcError::new(
                    codes::BATCH_FAILED,
                    format!("batch stopped at command {}", e.index),
                )
                .with_data(json!(e))
            })?;
            Ok(json!({"outcomes": outcomes, "revision": host.document().revision()}))
        }
        "query" => {
            let query: Query = params_as(params)?;
            let result = host.document().query(&query).map_err(command_failed)?;
            Ok(json!(result))
        }
        "render_view" => render_view(host, params),
        "describe" => Ok(describe()),
        "subscribe" => Err(RpcError::new(
            codes::INVALID_REQUEST,
            "subscribe needs a connection (use the socket or stdio transport)",
        )),
        other => Err(RpcError::new(
            codes::METHOD_NOT_FOUND,
            format!("unknown method {other:?}; call \"describe\" for the list"),
        )),
    }
}

fn render_view(host: &mut dyn Host, params: Value) -> Result<Value, RpcError> {
    #[derive(Deserialize)]
    struct View {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: Option<f64>,
    }
    #[derive(Deserialize)]
    struct Params {
        width: u32,
        height: u32,
        camera: Option<View>,
    }
    let p: Params = params_as(params)?;
    let sides = 1..=MAX_IMAGE_SIDE;
    if !sides.contains(&p.width) || !sides.contains(&p.height) {
        return Err(RpcError::new(
            codes::INVALID_PARAMS,
            format!("width and height must be between 1 and {MAX_IMAGE_SIDE}"),
        ));
    }
    let doc = host.document();
    let camera = match p.camera {
        Some(v) => doc
            .camera_for(v.eye, v.target, v.fov_y_deg)
            .map_err(command_failed)?,
        None => doc.camera.clone(),
    };
    let rgba = host
        .render_view(&camera, p.width, p.height)
        .ok_or_else(|| {
            RpcError::new(codes::RENDER_UNAVAILABLE, "no GPU available for rendering")
        })?;
    let png = encode_png(&rgba, p.width, p.height);
    Ok(json!({
        "width": p.width,
        "height": p.height,
        "png_base64": base64::engine::general_purpose::STANDARD.encode(png),
    }))
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header to memory");
    writer.write_image_data(rgba).expect("png data to memory");
    drop(writer);
    out
}

/// The machine-readable API description: methods, notifications and JSON Schemas.
pub fn describe() -> Value {
    json!({
        "schema_version": 1,
        "protocol": "JSON-RPC 2.0, one JSON object per line (UTF-8, '\\n' terminated)",
        "units": "meters; Z is up; angles in degrees",
        "methods": {
            "execute": {"params": "Command", "result": "{changes: ChangeSummary, revision}"},
            "batch": {
                "params": "{commands: [Command], atomic?: bool}",
                "result": "{outcomes: [Outcome], revision}",
                "notes": "atomic: one undo step, all-or-nothing; undo/redo not allowed"
            },
            "query": {"params": "Query", "result": "QueryResult"},
            "render_view": {
                "params": "{width, height, camera?: {eye, target, fov_y_deg?}}",
                "result": "{width, height, png_base64}",
                "notes": "does not move the document camera"
            },
            "subscribe": {"params": null, "result": "{revision}", "notes": "connection only"},
            "describe": {"params": null, "result": "this document"}
        },
        "notifications": {
            "changed": "{revision, changes: ChangeSummary} after every model change, in order",
            "resync": "{revision}: changes were missed; re-read the model with query entities"
        },
        "errors": {
            "-32700": "parse error", "-32600": "invalid request", "-32601": "method not found",
            "-32602": "invalid params", "-32000": "command failed (data: CommandError)",
            "-32001": "batch failed (data: BatchError)", "-32002": "rendering unavailable"
        },
        "schemas": {
            "Command": schema_for!(Command),
            "Query": schema_for!(Query),
            "QueryResult": schema_for!(QueryResult),
            "Outcome": schema_for!(Outcome),
            "CommandError": schema_for!(CommandError),
            "BatchError": schema_for!(BatchError),
            "ChangeEvent": schema_for!(ChangeEvent),
        }
    })
}

fn params_as<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|e| RpcError::new(codes::INVALID_PARAMS, format!("invalid params: {e}")))
}

fn command_failed(e: CommandError) -> RpcError {
    RpcError::new(codes::COMMAND_FAILED, e.message.clone()).with_data(json!(e))
}
