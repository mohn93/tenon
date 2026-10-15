//! Method dispatch: maps JSON-RPC methods onto session commands, queries and rendering.

use base64::Engine;
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use session::{
    BatchError, Camera, ChangeEvent, Command, CommandError, Document, ErrorKind, Lighting, Outcome,
    Query, QueryResult,
};

use crate::editor::{EDITOR_COMMANDS, EditorCommand, EditorState};
use crate::probe::{InferQuery, Inferred, PickHit, PickQuery, View, infer_query, pick_query};
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
    /// Renders the current model from `camera`, lit by `lighting`, to tightly packed RGBA8
    /// pixels, or None when no GPU is available.
    fn render_view(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>>;
    /// The editor's state, or None for a program without an editor (a bare document).
    fn editor(&self) -> Option<EditorState> {
        None
    }
    /// Runs an editor command.
    fn editor_command(&mut self, _command: EditorCommand) -> Result<(), CommandError> {
        Err(no_editor())
    }
    /// Ends the program once the replies waiting are sent. Refuses with `unsaved_changes`
    /// when the model changed since it was opened, saved or started, unless `discard`.
    fn quit(&mut self, _discard: bool) -> Result<(), CommandError> {
        Err(CommandError::new(
            ErrorKind::InvalidInput,
            "this program cannot be quit through the API",
        ))
    }
}

/// The error for editor commands and queries in a program without an editor.
fn no_editor() -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, "this program has no editor")
}

/// Queries the API answers from the view around the document: what lies under a pixel or
/// along a ray, and what the tools' inference would snap to there (see `probe`). Other
/// queries are `session::Query`s.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditorQuery {
    /// Every face a pixel's ray (or a given ray) meets, front to back, and the edges and
    /// guides passing near it, each with its instance path from the root.
    Pick(PickQuery),
    /// What a tool's click at a pixel (or at a model point where the view shows it) would
    /// snap to: kind, point and the entities involved.
    Infer(InferQuery),
    /// The editor's state (see `EditorState`).
    Editor,
}

/// The `type` of a query or command, if it has one.
fn type_of(params: &Value) -> Option<&str> {
    params.get("type").and_then(Value::as_str)
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
        "execute" if type_of(&params).is_some_and(|t| EDITOR_COMMANDS.contains(&t)) => {
            let command: EditorCommand = params_as(params)?;
            host.editor_command(command).map_err(command_failed)?;
            Ok(json!({"editor": host.editor(), "revision": host.document().revision()}))
        }
        "execute" => {
            let command: Command = params_as(params)?;
            let outcome = host.execute(command).map_err(command_failed)?;
            let mut result = json!(outcome);
            result["revision"] = json!(host.document().revision());
            Ok(result)
        }
        "batch" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
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
        "query" if matches!(type_of(&params), Some("pick" | "infer" | "editor")) => {
            let query: EditorQuery = params_as(params)?;
            editor_query(host, query)
        }
        "query" => {
            let query: Query = params_as(params)?;
            let result = host.document().query(&query).map_err(command_failed)?;
            Ok(json!(result))
        }
        "render_view" => render_view(host, params),
        "quit" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Params {
                #[serde(default)]
                discard: bool,
            }
            let p: Params = if params.is_null() {
                Params { discard: false }
            } else {
                params_as(params)?
            };
            host.quit(p.discard).map_err(command_failed)?;
            Ok(json!({}))
        }
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

fn editor_query(host: &mut dyn Host, query: EditorQuery) -> Result<Value, RpcError> {
    let doc = host.document();
    match query {
        EditorQuery::Pick(q) => {
            let hits = pick_query(doc, &q).map_err(command_failed)?;
            Ok(json!({"type": "pick", "hits": hits}))
        }
        EditorQuery::Infer(q) => {
            let mut result = json!(infer_query(doc, &q).map_err(command_failed)?);
            result["type"] = json!("infer");
            Ok(result)
        }
        EditorQuery::Editor => {
            let state = host
                .editor()
                .ok_or_else(no_editor)
                .map_err(command_failed)?;
            let mut result = json!(state);
            result["type"] = json!("editor");
            Ok(result)
        }
    }
}

fn render_view(host: &mut dyn Host, params: Value) -> Result<Value, RpcError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        width: u32,
        height: u32,
        camera: Option<View>,
        lighting: Option<Lighting>,
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
        Some(v) => v.camera(doc).map_err(command_failed)?,
        None => doc.camera.clone(),
    };
    let lighting = p.lighting.unwrap_or(doc.lighting);
    let rgba = host
        .render_view(&camera, lighting, p.width, p.height)
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
            "execute": {
                "params": "Command or EditorCommand (EditorCommand result: {editor: EditorState, revision})",
                "result": "{changes: ChangeSummary, import?: ImportReport, export?: ExportReport, soften?: SoftenReport, positioned?: integer, revision}: the Outcome plus revision"
            },
            "batch": {
                "params": "{commands: [Command], atomic?: bool}",
                "result": "{outcomes: [Outcome], revision}",
                "notes": "atomic: one undo step, all-or-nothing; undo/redo not allowed"
            },
            "query": {
                "params": "Query or EditorQuery",
                "result": "QueryResult, or for EditorQuery: pick {type, hits: [PickHit]}, infer {type, ...Inferred}, editor {type, ...EditorState}"
            },
            "render_view": {
                "params": "{width, height, camera?: {eye, target, fov_y_deg?, up?, projection?}, lighting?: \"day\" | \"night\"}",
                "result": "{width, height, png_base64}",
                "notes": "does not move the document camera or change its lighting"
            },
            "subscribe": {"params": null, "result": "{revision}", "notes": "connection only"},
            "describe": {"params": null, "result": "this document"},
            "quit": {
                "params": "{discard?: bool}",
                "result": "{}",
                "notes": "ends the program after replying; fails with unsaved_changes unless discard"
            }
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
            "EditorQuery": schema_for!(EditorQuery),
            "PickHit": schema_for!(PickHit),
            "Inferred": schema_for!(Inferred),
            "EditorCommand": schema_for!(EditorCommand),
            "EditorState": schema_for!(EditorState),
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
