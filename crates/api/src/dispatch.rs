//! Method dispatch: maps JSON-RPC methods onto session commands, queries and rendering.

use base64::Engine;
use kernel::{ChangeSummary, JointId, PageId, Scene, StyleId, StyleSettings};
use render::print::Printed;
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use session::print::{DEFAULT_DPI, Paper, PrintSpec};
use session::{
    BatchCommand, BatchError, Camera, ChangeEvent, Command, CommandError, Document, ErrorKind,
    Lighting, Outcome, Projection, Query, QueryResult, SectionDisplay, SectionFillColor,
    check_references,
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

/// Largest `supersample` `render_view` accepts.
pub const MAX_SUPERSAMPLE: u32 = 4;

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
    /// Not allowed with `scene`.
    camera: Option<View>,
    /// Day, night or studio for this image (default: the document's).
    lighting: Option<Lighting>,
    /// Render the frame `t` of the way from `from` (default: the current view and positions)
    /// to this scene's camera and stored positions, without changing the document.
    scene: Option<PageId>,
    /// 0 to 1 (default 1, linear: no easing); only with `scene`.
    t: Option<f64>,
    /// The scene to start from; only with `scene`.
    from: Option<PageId>,
    /// Joint values (a joint id to degrees for an angle, meters for a distance) to render at:
    /// solved on a copy of the model (couplings and cam followers too), after the scene frame
    /// when `scene` is given, without changing the document. Refused when they cannot be met.
    joints: Option<std::collections::BTreeMap<String, f64>>,
    /// A style to draw with for this image (default: the document's chosen style).
    #[schemars(with = "Option<String>")]
    style: Option<StyleId>,
    /// Perspective or parallel for this image, over the camera's own.
    projection: Option<Projection>,
    /// What to show of the section planes for this image: each part left out follows the
    /// document's section display.
    section: Option<SectionOverride>,
    /// Sun shadows on or off for this image (default: the document's).
    shadows: Option<bool>,
    /// 1 to 4 (default 1): renders at that many times the size and averages down to
    /// width x height for smoother edges. Width and height times it are at most twice the
    /// largest side (8192).
    supersample: Option<u32>,
    /// `style` (default): the style's background. `transparent`: pixels with nothing drawn on
    /// them (no faces, edges or guides; no ground grid or axes either) are clear, alpha 0.
    background: Option<Background>,
    /// `false` hides the ground grid, the axes and the ground shadows for this image (default
    /// true), so a model can sit at z = 0.
    ground: Option<bool>,
}

/// The parts of the section display a render overrides; each left out follows the document's.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SectionOverride {
    planes: Option<bool>,
    cuts: Option<bool>,
    fill: Option<bool>,
    fill_color: Option<SectionFillColor>,
}

/// What a render shows behind the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Background {
    #[default]
    Style,
    Transparent,
}

/// How `Host::render_view` draws one image. Everything is resolved: the overrides of the call
/// are applied over the document's settings, which the host must leave as they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// The size of the image to return, in pixels. With `supersample` above 1 the host draws
    /// this many times larger on each side (its own `width` and `height` are those larger
    /// sizes: see `RenderOptions::drawn`).
    pub width: u32,
    pub height: u32,
    /// Times the size the image is drawn at before it is averaged down (1 to 4).
    pub supersample: u32,
    pub lighting: Lighting,
    /// The style to draw with (None: the plain look).
    pub style: Option<StyleSettings>,
    pub section: SectionDisplay,
    /// Whether the sun's shadows are drawn.
    pub shadows: bool,
    /// Clear (alpha 0) where nothing is drawn; the pixels then come premultiplied by alpha.
    pub transparent: bool,
    /// The ground grid, axes and ground shadows are drawn.
    pub ground: bool,
}

impl RenderOptions {
    /// The size the host draws at: the image size times `supersample`.
    pub fn drawn(&self) -> (u32, u32) {
        (
            self.width * self.supersample,
            self.height * self.supersample,
        )
    }

    /// The options that draw the document as it is, `width` by `height`.
    pub fn of(doc: &Document, width: u32, height: u32) -> Self {
        RenderOptions {
            width,
            height,
            supersample: 1,
            lighting: doc.lighting,
            style: doc.chosen_style().copied(),
            section: doc.section_display,
            shadows: doc.shadows.on,
            transparent: false,
            ground: true,
        }
    }
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
    /// Renders the current model from `camera` as `options` say, to tightly packed RGBA8
    /// pixels of `options.drawn()` size, or None when no GPU is available. Changes nothing of
    /// the document or of how later renders look. With `frame`, draws that scene (the
    /// document's with some instances moved) instead of the document's, with the document's
    /// open context and selection.
    fn render_view(
        &mut self,
        camera: &Camera,
        options: &RenderOptions,
        frame: Option<&Scene>,
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
    cut_list: Option<bool>,
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
        cut_list: cut_list.unwrap_or(false),
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
                    cut_list,
                    overwrite,
                } => {
                    let spec = print_spec(paper.as_deref(), landscape, scale, dpi, cut_list);
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
    let bad_params = |why: &str| Err(RpcError::new(codes::INVALID_PARAMS, why));
    if p.scene.is_some() && p.camera.is_some() {
        return bad_params("camera is not allowed with scene");
    }
    if p.scene.is_none() && (p.t.is_some() || p.from.is_some()) {
        return bad_params("t and from need scene");
    }
    let t = p.t.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&t) {
        return bad_params("t must be from 0 to 1");
    }
    let doc = host.document();
    let (mut camera, frame) = match (p.scene, p.camera) {
        (Some(scene), _) => {
            let motion = doc.motion_between(p.from, scene).map_err(command_failed)?;
            (motion.camera_at(t), Some(motion.scene_at(doc.scene(), t)))
        }
        (None, Some(v)) => (v.camera(doc).map_err(command_failed)?, None),
        (None, None) => (doc.camera.clone(), None),
    };
    let invalid = |why: String| command_failed(CommandError::new(ErrorKind::InvalidInput, why));
    let frame = match p.joints.as_ref().filter(|j| !j.is_empty()) {
        Some(joints) => {
            let mut values = Vec::with_capacity(joints.len());
            for (key, value) in joints {
                let id: JointId = key.parse().map_err(|_| {
                    RpcError::new(
                        codes::INVALID_PARAMS,
                        format!("joints: {key:?} is not a joint id"),
                    )
                })?;
                values.push((id, *value));
            }
            let base = frame.unwrap_or_else(|| doc.scene().clone());
            Some(
                doc.scene_with_joints(base, &values)
                    .map_err(command_failed)?,
            )
        }
        None => frame,
    };
    if let Some(projection) = p.projection {
        camera.projection = projection;
    }
    let mut options = RenderOptions::of(doc, p.width, p.height);
    if let Some(lighting) = p.lighting {
        options.lighting = lighting;
    }
    if let Some(id) = p.style {
        let style = doc
            .scene()
            .style(id)
            .ok_or_else(|| invalid(format!("no style {id}")))?;
        options.style = Some(style.settings);
    }
    if let Some(o) = p.section {
        let s = &mut options.section;
        s.planes = o.planes.unwrap_or(s.planes);
        s.cuts = o.cuts.unwrap_or(s.cuts);
        s.fill = o.fill.unwrap_or(s.fill);
        s.fill_color = o.fill_color.unwrap_or(s.fill_color);
    }
    options.shadows = p.shadows.unwrap_or(options.shadows);
    options.transparent = p.background == Some(Background::Transparent);
    options.ground = p.ground.unwrap_or(true);
    let k = p.supersample.unwrap_or(1);
    if !(1..=MAX_SUPERSAMPLE).contains(&k) {
        return Err(invalid(format!(
            "supersample must be between 1 and {MAX_SUPERSAMPLE}"
        )));
    }
    if p.width * k > 2 * MAX_IMAGE_SIDE || p.height * k > 2 * MAX_IMAGE_SIDE {
        return Err(invalid(format!(
            "width and height times supersample must be at most {}",
            2 * MAX_IMAGE_SIDE
        )));
    }
    options.supersample = k;
    let mut rgba = host
        .render_view(&camera, &options, frame.as_ref())
        .ok_or_else(|| {
            RpcError::new(codes::RENDER_UNAVAILABLE, "no GPU available for rendering")
        })?;
    if k > 1 {
        rgba = render::snapshot::box_downsample(&rgba, p.width, p.height, k);
    }
    if options.transparent {
        render::snapshot::unpremultiply(&mut rgba);
    }
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
                "result": "{changes: ChangeSummary, import?: ImportReport, export?: ExportReport, soften?: SoftenReport, intersect?: IntersectReport, changed?: [EntityId] (hide, unhide, unhide_last, unhide_all, reverse_faces, flip), orient?: {flipped: [EntityId], conflicts: integer} (orient_faces), make_solid?: {entries: [{definition, parts, fixed, welded, strays_removed, internal_removed, holes_filled, faces_flipped, left, reasons}], solid: integer} (make_solid), positioned?: integer, purged?: integer, definitions?: [DefinitionId] (make_unique, make_group, make_component, add_box/cylinder/tube/cone/prism/sphere/torus as group or component; the new groups of solid_union, solid_subtract, solid_intersect, solid_trim, solid_split, outer_shell), placed?: [EntityId] (whenever the command placed instances: those left in the open context, e.g. copy, rotate with copies, copy_along_path, make_group, make_component, the primitives as group or component, explode, paste, insert_library_item, place_instance, import, solid_union, solid_subtract, solid_intersect, solid_trim, solid_split, outer_shell), text_3d?: Text3dReport, material?: MaterialId (insert_library_item of a material), units?: Units, empty?: true (solid_subtract, solid_intersect or solid_trim that left nothing; placed is then empty), revision}: the Outcome plus revision",
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
                "notes": "atomic: one undo step, all-or-nothing; undo/redo not allowed; an id may be a BatchRef to ids an earlier command of the batch made (\"$0:f0\", \"$0:d0\" for a definition make_unique, make_group, make_component or a primitive (as group or component) made, \"$0:y0\" for a style add_style made, \"$0:j0\" for a joint add_joint made (also inside the `joints` list of add_scene, update_scene and change_scene), \"$0:k0\" for a coupling add_coupling made, \"$0:w0\" for a cam follower add_cam_follower made; a bad one is -32602 before anything runs, one past what was made fails its command with invalid_input)",
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
                "params": "{width, height, camera?: {eye, target, fov_y_deg?, up?, projection?}, lighting?: \"day\" | \"night\" | \"studio\", style?: StyleId, projection?: \"perspective\" | \"parallel\", section?: {planes?, cuts?, fill?, fill_color?: \"dark\" | \"material\"}, shadows?: bool, supersample?: 1..4 (width and height times it at most 8192), background?: \"style\" | \"transparent\", ground?: bool (default true), scene?: PageId, t?: 0 to 1 (default 1), from?: PageId, joints?: {JointId: degrees or meters}}",
                "result": "{width, height, png_base64}",
                "notes": "does not move the document camera or change its lighting; with scene it renders the frame t of the way (linear) from `from` (default the current view and positions) to the scene's camera and stored positions, without changing the document (not with camera; t and from need scene); joints renders with those joint values solved on a copy of the model (couplings and cam followers too; after the scene frame when scene is given), without changing the document, and is refused with invalid_input when they cannot be met; the document's tags, lighting and style are kept unless lighting or style override them, and projection overrides the frame camera's projection",
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
