//! Method dispatch: maps JSON-RPC methods onto session commands, queries and rendering.

use base64::Engine;
use kernel::ChangeSummary;
use render::print::Printed;
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use session::print::{DEFAULT_DPI, Paper, PrintSpec};
use session::{
    BatchCommand, BatchError, Camera, ChangeEvent, Command, CommandError, Document, ErrorKind,
    Lighting, Outcome, Query, QueryResult, check_references,
};

use crate::editor::{EDITOR_COMMANDS, EditorCommand, EditorState, PdfReport};
use crate::input::{
    ToolInput, ToolInputError, ToolInputParams, ToolInputResult, ToolInputStopped, check,
};
use crate::probe::{
    InferQuery, Inferred, PickHit, PickQuery, ScreenCommand, View, box_ids, infer_query, pick_query,
};
use crate::protocol::{RpcError, codes, failure, parse, success};
use crate::roots::ApiRoots;
use crate::schema::{any_of_root, batch_ref_schema, hoist, object_root, with_batch_refs};

/// Largest image side `render_view` accepts, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 4096;

/// The reply to `execute`: the outcome and the document revision after it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExecuteResult {
    #[serde(flatten)]
    pub outcome: Outcome,
    pub revision: u64,
}

/// The reply to `batch`: each command's outcome, the net change of them all, and the revision.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BatchResult {
    pub outcomes: Vec<Outcome>,
    /// The net effect of the whole batch (made then erased is absent).
    pub changes: ChangeSummary,
    pub revision: u64,
}

/// The reply to `execute` of an editor command: the editor after it and the revision.
#[derive(Serialize, JsonSchema)]
struct EditorResult {
    editor: Option<EditorState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf: Option<PdfReport>,
    revision: u64,
}

/// `batch`'s params.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BatchParams {
    /// Commands, read one by one so that ids may be references (`BatchCommand::parse`).
    #[schemars(with = "Vec<Command>")]
    commands: Vec<Value>,
    #[serde(default)]
    atomic: bool,
}

/// `render_view`'s params.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenderParams {
    width: u32,
    height: u32,
    camera: Option<View>,
    lighting: Option<Lighting>,
}

/// What the API needs from the program it runs in (the window app or headless mode).
pub trait Host {
    fn document(&self) -> &Document;
    /// Runs one command the same way the UI would.
    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError>;
    fn execute_batch(
        &mut self,
        commands: Vec<BatchCommand>,
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
    /// Renders the current view as a print (see `render::print`) with the GPU; the default is
    /// a host without one.
    fn render_pdf(&mut self, _spec: &PrintSpec) -> Result<Printed, CommandError> {
        Err(no_gpu_for_printing())
    }
    /// Runs an editor command.
    fn editor_command(&mut self, _command: EditorCommand) -> Result<(), CommandError> {
        Err(no_editor())
    }
    /// Feeds the active tool the events, as the window would, and reports the editor after
    /// them and the commands they ran.
    fn tool_input(&mut self, _input: ToolInput) -> Result<ToolInputResult, ToolInputError> {
        Err(no_editor().into())
    }
    /// Ends the program once the replies waiting are sent. Refuses with `unsaved_changes`
    /// when the model changed since it was opened, saved or started, unless `discard`.
    fn quit(&mut self, _discard: bool) -> Result<(), CommandError> {
        Err(CommandError::new(
            ErrorKind::InvalidInput,
            "this program cannot be quit through the API",
        ))
    }
    /// The folders file paths must lie within (`--api-root`), or None for no limit.
    fn roots(&self) -> Option<&ApiRoots> {
        None
    }
    /// True once a `quit` was accepted: every later request is refused, so no edit is made
    /// that would not be saved.
    fn quitting(&self) -> bool {
        false
    }
}

/// Applies the host's `--api-root` folders to `command`'s paths.
fn within_roots(
    host: &dyn Host,
    command: &mut Command,
    in_batch: bool,
) -> Result<(), CommandError> {
    match host.roots() {
        Some(roots) => roots.apply(command, host.document().path(), in_batch),
        None => Ok(()),
    }
}

/// The reply to a request that arrives after an accepted `quit`.
fn quitting_error() -> RpcError {
    command_failed(CommandError::new(
        ErrorKind::InvalidInput,
        "the program is quitting",
    ))
}

/// The error for editor commands and queries in a program without an editor.
pub fn no_gpu_for_printing() -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, "printing needs a GPU adapter")
}

/// The print an `export_pdf` command describes: its paper, orientation, scale and resolution.
fn print_spec(
    paper: Option<&str>,
    landscape: Option<bool>,
    scale: Option<f64>,
    dpi: Option<u32>,
) -> Result<PrintSpec, CommandError> {
    let paper = match paper {
        None => Paper::A4,
        Some(name) => Paper::from_name(name).ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidInput,
                format!("unknown paper {name:?}: use a4, a3, letter, legal or tabloid"),
            )
        })?,
    };
    Ok(PrintSpec {
        paper,
        landscape: landscape.unwrap_or(true),
        scale,
        dpi: dpi.unwrap_or(DEFAULT_DPI),
    })
}

/// Runs `export_pdf`: checks the path first (so a refusal costs no rendering), prints, writes.
fn export_pdf(
    host: &mut dyn Host,
    path: &str,
    spec: PrintSpec,
    overwrite: Option<bool>,
) -> Result<PdfReport, CommandError> {
    let overwrite = overwrite.unwrap_or(false);
    let (path, refuse_links) = match host.roots() {
        Some(roots) => (roots.write(path)?.display().to_string(), true),
        None => (path.to_string(), false),
    };
    host.document()
        .check_pdf_target(&path, overwrite, refuse_links)?;
    let printed = host.render_pdf(&spec)?;
    host.document()
        .write_pdf(&path, &printed.pdf, overwrite, refuse_links)?;
    Ok(PdfReport {
        sheets: printed.sheets,
        warnings: printed.warnings,
    })
}

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
    Editor {},
}

/// The `type` of a query or command, if it has one.
fn type_of(params: &Value) -> Option<&str> {
    params.get("type").and_then(Value::as_str)
}

/// Handles one request line and returns the reply line (None for notifications).
/// `subscribe` is connection-level and handled by `Hub`.
pub fn handle(host: &mut dyn Host, line: &str) -> Option<String> {
    respond(host, line).map(|(reply, _)| reply)
}

/// [`handle`], with the error message when the reply is an error, so the activity log need
/// not parse the reply again (a `render_view` reply is megabytes).
pub(crate) fn respond(host: &mut dyn Host, line: &str) -> Option<(String, Option<String>)> {
    let request = match parse(line) {
        Ok(r) => r,
        Err(reply) => {
            // A parse or request error: a short line.
            let message = serde_json::from_str::<Value>(&reply)
                .ok()
                .and_then(|r| r["error"]["message"].as_str().map(str::to_string));
            return Some((reply, message));
        }
    };
    let result = if host.quitting() {
        Err(quitting_error())
    } else {
        call(host, &request.method, request.params)
    };
    let id = request.id?;
    Some(match result {
        Ok(value) => (success(&id, value), None),
        Err(error) => {
            let message = error.message.clone();
            (failure(&id, error), Some(message))
        }
    })
}

/// Runs one request's method with its params and returns the result or the error to send.
pub fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "execute" if type_of(&params).is_some_and(|t| EDITOR_COMMANDS.contains(&t)) => {
            let command: EditorCommand = params_as(params)?;
            let pdf = match command {
                EditorCommand::ExportPdf {
                    path,
                    paper,
                    landscape,
                    scale,
                    dpi,
                    overwrite,
                } => {
                    let spec = print_spec(paper.as_deref(), landscape, scale, dpi);
                    Some(
                        spec.and_then(|spec| export_pdf(host, &path, spec, overwrite))
                            .map_err(command_failed)?,
                    )
                }
                EditorCommand::PlaceLibraryItem { path } => {
                    let path = match host.roots() {
                        Some(roots) => roots
                            .read(&path)
                            .map(|p| p.display().to_string())
                            .map_err(command_failed)?,
                        None => path,
                    };
                    host.editor_command(EditorCommand::PlaceLibraryItem { path })
                        .map_err(command_failed)?;
                    None
                }
                command => {
                    host.editor_command(command).map_err(command_failed)?;
                    None
                }
            };
            Ok(json!(EditorResult {
                editor: host.editor(),
                pdf,
                revision: host.document().revision(),
            }))
        }
        "execute" if type_of(&params) == Some("select") && params.get("box").is_some() => {
            let ScreenCommand::Select { screen_box, mode } = params_as(params)?;
            let ids = box_ids(host.document(), &screen_box).map_err(command_failed)?;
            let outcome = host
                .execute(Command::Select {
                    ids,
                    mode,
                    all: false,
                })
                .map_err(command_failed)?;
            Ok(json!(ExecuteResult {
                outcome,
                revision: host.document().revision(),
            }))
        }
        "execute" => {
            let mut command: Command = params_as(params)?;
            within_roots(host, &mut command, false).map_err(command_failed)?;
            let outcome = host.execute(command).map_err(command_failed)?;
            Ok(json!(ExecuteResult {
                outcome,
                revision: host.document().revision(),
            }))
        }
        "batch" => {
            let p: BatchParams = params_as(params)?;
            let mut commands = p
                .commands
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    BatchCommand::parse(value, index).map_err(|reason| {
                        RpcError::new(
                            codes::INVALID_PARAMS,
                            format!("invalid params: commands[{index}]: {reason}"),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            check_references(&commands).map_err(|(index, reason)| {
                RpcError::new(
                    codes::INVALID_PARAMS,
                    format!("invalid params: commands[{index}]: {reason}"),
                )
            })?;
            for (index, command) in commands.iter_mut().enumerate() {
                if let Err(error) = within_roots(host, command.command_mut(), true) {
                    let e = BatchError {
                        index,
                        error,
                        completed: Vec::new(),
                        rolled_back: false,
                    };
                    return Err(RpcError::new(
                        codes::BATCH_FAILED,
                        format!("batch stopped at command {index}"),
                    )
                    .with_data(json!(e)));
                }
            }
            let outcomes = host.execute_batch(commands, p.atomic).map_err(|e| {
                RpcError::new(
                    codes::BATCH_FAILED,
                    format!("batch stopped at command {}", e.index),
                )
                .with_data(json!(e))
            })?;
            let changes = ChangeSummary::join(outcomes.iter().map(|o| &o.changes));
            Ok(json!(BatchResult {
                outcomes,
                changes,
                revision: host.document().revision(),
            }))
        }
        "query" if matches!(type_of(&params), Some("pick" | "infer" | "editor")) => {
            let query: EditorQuery = params_as(params)?;
            editor_query(host, query)
        }
        "query" => {
            let file = type_of(&params) == Some("file");
            let mut query: Query = params_as(params)?;
            if let Some(roots) = host.roots() {
                roots.apply_query(&mut query).map_err(command_failed)?;
            }
            let mut result = json!(host.document().query(&query).map_err(command_failed)?);
            if file {
                result["api_roots"] = json!(host.roots().map(ApiRoots::dirs));
            }
            Ok(result)
        }
        "render_view" => render_view(host, params),
        "tool_input" => {
            let p: ToolInputParams = params_as(params)?;
            let input = check(p).map_err(command_failed)?;
            let result = host.tool_input(input).map_err(|e| match e {
                ToolInputError::Refused(e) => command_failed(e),
                ToolInputError::Stopped(stopped) => {
                    RpcError::new(codes::COMMAND_FAILED, stopped.error.message.clone())
                        .with_data(json!(stopped))
                }
            })?;
            Ok(json!(result))
        }
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
        EditorQuery::Editor {} => {
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
    let p: RenderParams = params_as(params)?;
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
        "schema_version": 2,
        "protocol": "JSON-RPC 2.0, one JSON object per line (UTF-8, '\\n' terminated)",
        "units": "meters; Z is up; angles in degrees",
        "paths": "file paths are absolute or relative to the app's working directory; with --api-root, relative to the first root and kept within the roots after symlinks (query file lists them as api_roots); save and export refuse to replace a file of another format unless overwrite: true",
        "discard": "in the window, new, open and quit with discard: true first write the unsaved model to a recovery file in ~/.sketchup-alt/recovery (the status bar and View > Agent Activity name it); if that fails the command fails with file and nothing is discarded; headless writes nothing",
        "methods": {
            "execute": {
                "params": "Command, ScreenCommand (select by a box on screen) or EditorCommand (EditorCommand result: {editor: EditorState, pdf?: {sheets, warnings} (export_pdf only), revision})",
                "result": "{changes: ChangeSummary, import?: ImportReport, export?: ExportReport, soften?: SoftenReport, intersect?: IntersectReport, changed?: [EntityId] (hide, unhide, unhide_last, unhide_all, reverse_faces, flip), orient?: {flipped: [EntityId], conflicts: integer} (orient_faces), positioned?: integer, purged?: integer, definitions?: [DefinitionId] (make_unique), text_3d?: Text3dReport, material?: MaterialId (insert_library_item of a material), units?: Units, revision}: the Outcome plus revision",
                "params_schema": any_of_root(vec![
                    schema::<Command>(),
                    schema::<ScreenCommand>(),
                    schema::<EditorCommand>(),
                ]),
                "result_schema": any_of_root(vec![schema::<ExecuteResult>(), schema::<EditorResult>()]),
            },
            "batch": {
                "params": "{commands: [Command], atomic?: bool}",
                "result": "{outcomes: [Outcome], changes: ChangeSummary, revision}: changes is the net effect of the whole batch",
                "notes": "atomic: one undo step, all-or-nothing; undo/redo not allowed; an id may be a BatchRef to ids an earlier command of the batch made (\"$0:f0\", \"$0:d0\" for a definition make_unique made, \"$0:y0\" for a style add_style made; a bad one is -32602 before anything runs, one past what was made fails its command with invalid_input)",
                "params_schema": batch_params_schema(),
                "result_schema": schema::<BatchResult>(),
            },
            "query": {
                "params": "Query or EditorQuery",
                "result": "QueryResult, or for EditorQuery: pick {type, hits: [PickHit]}, infer {type, ...Inferred}, editor {type, ...EditorState}",
                "params_schema": any_of_root(vec![schema::<Query>(), schema::<EditorQuery>()]),
                "result_schema": query_result_schema(),
            },
            "render_view": {
                "params": "{width, height, camera?: {eye, target, fov_y_deg?, up?, projection?}, lighting?: \"day\" | \"night\"}",
                "result": "{width, height, png_base64}",
                "notes": "does not move the document camera or change its lighting",
                "params_schema": schema::<RenderParams>(),
                "result_schema": {
                    "type": "object",
                    "properties": {
                        "width": {"type": "integer"},
                        "height": {"type": "integer"},
                        "png_base64": {"type": "string"}
                    },
                    "required": ["width", "height", "png_base64"]
                },
            },
            "tool_input": {
                "params": "ToolInput: {events: [InputEvent], width?, height?}",
                "result": "ToolInputResult: {editor, inference, preview, commands: [{command, outcome?, error?}], revision}",
                "notes": "runs the events through the window's editor and active tool (inference, locks, typed values); a model point is looked for where it is, even behind other geometry, at the pixel the view shows it at its turn; an event that cannot run stops the rest (the ones before it stay done; data: ToolInputStopped); modifiers a modifiers event holds stay held across calls until another lets them go or cancel",
                "params_schema": schema::<ToolInputParams>(),
                "result_schema": schema::<ToolInputResult>(),
            },
            "subscribe": {
                "params": null,
                "result": "{revision}",
                "notes": "connection only",
                "params_schema": null,
                "result_schema": {
                    "type": "object",
                    "properties": {"revision": {"type": "integer", "minimum": 0}},
                    "required": ["revision"]
                },
            },
            "describe": {
                "params": null,
                "result": "this document",
                "params_schema": null,
                "result_schema": {"type": "object"},
            },
            "quit": {
                "params": "{discard?: bool}",
                "result": "{}",
                "notes": "ends the program after replying; fails with unsaved_changes unless discard (the window keeps discarded work in a recovery file); once accepted, every later request is refused",
                "params_schema": {"type": "object", "properties": {"discard": {"type": "boolean"}}},
                "result_schema": {"type": "object"},
            }
        },
        "notifications": {
            "changed": "{revision, changes: ChangeSummary} after every model change, in order",
            "resync": "{revision}: changes were missed; re-read the model with query entities",
            "selection": "{ids, revision}: the selection changed (as the selection query returns)",
            "camera": "the camera query's result without type: the camera or lighting changed",
            "editor": "EditorState: the editor changed",
            "note": "selection, camera and editor are coalesced to at most one of each per frame (window) or per round of requests (headless)"
        },
        "errors": {
            "-32700": "parse error", "-32600": "invalid request", "-32601": "method not found",
            "-32602": "invalid params", "-32000": "command failed (data: CommandError; for tool_input stopped partway, ToolInputStopped)",
            "-32001": "batch failed (data: BatchError)", "-32002": "rendering unavailable",
            "-32003": "connection lost (MCP --live: the instance's socket closed)"
        },
        "schemas": {
            "Command": object_root(schema::<Command>()),
            "Query": object_root(schema::<Query>()),
            "EditorQuery": object_root(schema::<EditorQuery>()),
            "PickHit": schema_for!(PickHit),
            "Inferred": schema_for!(Inferred),
            "ScreenCommand": object_root(schema::<ScreenCommand>()),
            "EditorCommand": object_root(schema::<EditorCommand>()),
            "EditorState": schema_for!(EditorState),
            "ToolInput": schema_for!(ToolInputParams),
            "ToolInputResult": schema_for!(ToolInputResult),
            "ToolInputStopped": schema_for!(ToolInputStopped),
            "QueryResult": schema_for!(QueryResult),
            "Outcome": schema_for!(Outcome),
            "ExecuteResult": schema_for!(ExecuteResult),
            "BatchResult": schema_for!(BatchResult),
            "CommandError": schema_for!(CommandError),
            "BatchError": schema_for!(BatchError),
            "ChangeEvent": schema_for!(ChangeEvent),
            "BatchRef": batch_ref_schema(),
        }
    })
}

/// `batch`'s params schema, its entity and material ids also taking batch references.
fn batch_params_schema() -> Value {
    let mut schema = schema::<BatchParams>();
    if let Some(Value::Object(defs)) = schema.get_mut("$defs") {
        with_batch_refs(defs);
    }
    schema
}

/// `T`'s JSON Schema as JSON, self-contained (its definitions under `$defs`).
fn schema<T: JsonSchema>() -> Value {
    schema_for!(T).into()
}

/// What `query` returns: a `QueryResult`, or for an `EditorQuery` its pick, infer or editor
/// reply, each tagged with `type`.
fn query_result_schema() -> Value {
    let mut defs = Map::new();
    let tagged = |tag: &str| json!({"properties": {"type": {"const": tag}}, "required": ["type"]});
    let query_result = hoist(&mut defs, schema::<QueryResult>());
    let pick_hit = hoist(&mut defs, schema::<PickHit>());
    let inferred = hoist(&mut defs, schema::<Inferred>());
    let editor = hoist(&mut defs, schema::<EditorState>());
    let pick = json!({
        "type": "object",
        "properties": {
            "type": {"const": "pick"},
            "hits": {"type": "array", "items": pick_hit}
        },
        "required": ["type", "hits"]
    });
    json!({
        "type": "object",
        "anyOf": [
            query_result,
            pick,
            {"allOf": [inferred, tagged("infer")]},
            {"allOf": [editor, tagged("editor")]}
        ],
        "$defs": defs
    })
}

/// Params by name: an array (params by position) is refused, so `quit [true]` cannot discard
/// changes by accident.
fn params_as<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    if params.is_array() {
        return Err(RpcError::new(
            codes::INVALID_PARAMS,
            "invalid params: params must be an object (by name), not an array",
        ));
    }
    serde_json::from_value(params)
        .map_err(|e| RpcError::new(codes::INVALID_PARAMS, format!("invalid params: {e}")))
}

fn command_failed(e: CommandError) -> RpcError {
    RpcError::new(codes::COMMAND_FAILED, e.message.clone()).with_data(json!(e))
}
