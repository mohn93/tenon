//! The seven MCP tools: their input schemas, how their arguments map onto API methods, and how
//! the replies become tool results.

use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use session::{Command, Lighting, Query};

use super::Backend;
use crate::dispatch::{EditorQuery, MAX_IMAGE_SIDE};
use crate::editor::EditorCommand;
use crate::input::ToolInputParams;
use crate::probe::{InferQuery, PickQuery, ScreenCommand, View};
use crate::protocol::{RpcError, codes};
use crate::schema::{hoist, object_root, with_batch_refs};

/// `render_view`'s image size when the call leaves a side out.
const DEFAULT_WIDTH: u32 = 960;
const DEFAULT_HEIGHT: u32 = 600;

/// The tools in the order `tools/list` gives them, with whether each only reads.
const TOOLS: [(&str, bool); 7] = [
    ("describe", true),
    ("transact", false),
    ("query", true),
    ("pick", true),
    ("infer", true),
    ("render_view", true),
    ("tool_input", false),
];

/// `tools/list`'s tools.
pub(super) fn list() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|&(name, read_only)| {
            json!({
                "name": name,
                "description": description(name),
                "inputSchema": input_schema(name),
                "annotations": {"readOnlyHint": read_only},
            })
        })
        .collect()
}

fn description(tool: &str) -> &'static str {
    match tool {
        "describe" => {
            "Describes the modeler's API (units are meters, Z is up); pass schema with one of schema_names or a method name (execute, batch, query, render_view, tool_input) to get its JSON Schema."
        }
        "transact" => {
            "Changes the model with one or more commands (points in meters, Z up), run as one atomic undo step unless atomic is false; call describe for the command schemas. Two or more commands may refer to ids an earlier one of them made: \"$0:f0\" is the first face command 0 created, \"$0:e*\" all its edges, \"$0:*\" every edge, face, instance and guide it created, \"$1:m0\" the material command 1 added."
        }
        "query" => {
            "Reads the model with one query (entities, selection, camera, stats, history and more; lengths in meters, Z up), named by type; each type's other fields are in describe's query schema (describe with schema \"query\"), as is the result schema."
        }
        "pick" => {
            "Lists what lies under a view pixel or along a model ray (meters, Z up): faces front to back and the edges and guides near it; call describe for the schemas."
        }
        "infer" => {
            "Reports what a drawing tool's click at a view pixel or a model point (meters, Z up) would snap to; call describe for the schemas."
        }
        "render_view" => {
            "Renders the model to a PNG image (960 x 600 pixels unless given) from the document camera or a camera whose eye and target are in meters with Z up, without moving the document camera; call describe for the schemas."
        }
        "tool_input" => {
            "Drives the active drawing tool with mouse and keyboard events as a person would (model points in meters, Z up) and reports the commands it ran; call describe for the schemas."
        }
        _ => unreachable!("no tool {tool}"),
    }
}

fn input_schema(tool: &str) -> Value {
    match tool {
        "describe" => json!({
            "type": "object",
            "properties": {
                "schema": {
                    "type": "string",
                    "description": "A name from schema_names, or a method name for its params and result schemas; leave out for the API overview."
                }
            },
            "additionalProperties": false
        }),
        "transact" => {
            let mut defs = Map::new();
            let items = json!({"anyOf": [
                hoist(&mut defs, schema::<Command>()),
                hoist(&mut defs, schema::<ScreenCommand>()),
                hoist(&mut defs, schema::<EditorCommand>()),
            ]});
            with_batch_refs(&mut defs);
            json!({
                "type": "object",
                "properties": {
                    "commands": {
                        "type": "array",
                        "minItems": 1,
                        "items": items,
                        "description": "Run in order. One command may also be an editor command, a select by screen box, undo, redo, save or open."
                    },
                    "atomic": {
                        "type": "boolean",
                        "description": "Two or more commands: one undo step, all or nothing (default true)."
                    }
                },
                "required": ["commands"],
                "additionalProperties": false,
                "$defs": defs
            })
        }
        "query" => {
            let mut types = query_types(&schema::<Query>());
            types.extend(query_types(&schema::<EditorQuery>()));
            json!({
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "enum": types,
                        "description": "The query; describe gives each type's other fields."
                    }
                },
                "required": ["type"],
                "additionalProperties": true
            })
        }
        "pick" => rooted(schema::<PickQuery>()),
        "infer" => rooted(schema::<InferQuery>()),
        "render_view" => {
            let mut defs = Map::new();
            let side = |default: u32| {
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_IMAGE_SIDE,
                    "description": format!("Pixels (default {default}).")
                })
            };
            let camera = hoist(&mut defs, schema::<View>());
            let lighting = hoist(&mut defs, schema::<Lighting>());
            let scene = hoist(&mut defs, schema::<kernel::PageId>());
            json!({
                "type": "object",
                "properties": {
                    "width": side(DEFAULT_WIDTH),
                    "height": side(DEFAULT_HEIGHT),
                    "camera": camera,
                    "lighting": lighting,
                    "scene": {"description": "Render a frame of going to this scene (its camera and stored instance positions) instead of the document camera; not with camera.", "allOf": [scene.clone()]},
                    "t": {"type": "number", "minimum": 0, "maximum": 1, "description": "With scene: how far along, 0 (the current view, or from) to 1 (the scene; default), linear."},
                    "from": {"description": "With scene: the scene to start from (default the current view and positions).", "allOf": [scene]}
                },
                "additionalProperties": false,
                "$defs": defs
            })
        }
        "tool_input" => rooted(schema::<ToolInputParams>()),
        _ => unreachable!("no tool {tool}"),
    }
}

/// The `type` names of an internally tagged enum's schema, one per `oneOf` variant.
///
/// # Panics
///
/// When the schema has no `oneOf` or a variant has no constant `type`.
fn query_types(schema: &Value) -> Vec<Value> {
    schema["oneOf"]
        .as_array()
        .expect("a tagged enum's schema has oneOf")
        .iter()
        .map(|variant| {
            let name = &variant["properties"]["type"]["const"];
            assert!(name.is_string(), "a query variant has no constant type");
            name.clone()
        })
        .collect()
}

/// `T`'s JSON Schema as JSON, self-contained.
fn schema<T: JsonSchema>() -> Value {
    schema_for!(T).into()
}

/// `schema` with an object root and its definitions under the root's `$defs`, without
/// `$schema` or `title`.
fn rooted(schema: Value) -> Value {
    let mut defs = Map::new();
    let mut schema = hoist(&mut defs, schema);
    if !defs.is_empty() {
        schema["$defs"] = Value::Object(defs);
    }
    object_root(schema)
}

/// Runs `tools/call`. An unknown tool or a call without a name is a protocol error; anything
/// that fails past that is a tool result with `isError`.
pub(super) fn call(
    backend: &mut dyn Backend,
    params: &Value,
    structured: bool,
) -> Result<Value, RpcError> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(RpcError::new(
            codes::INVALID_PARAMS,
            "tools/call needs the tool's name",
        ));
    };
    if !TOOLS.iter().any(|&(tool, _)| tool == name) {
        return Err(RpcError::new(
            codes::INVALID_PARAMS,
            format!("unknown tool {name:?}"),
        ));
    }
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(a) => a.clone(),
    };
    Ok(match run(backend, name, arguments) {
        Ok(reply) => tool_result(reply.value, reply.image, false, structured),
        Err(e) => {
            let mut error = json!({"code": e.code, "message": e.message});
            if let Some(data) = e.data {
                error["data"] = data;
            }
            tool_result(error, None, true, structured)
        }
    })
}

/// A tool's reply: its JSON and, for `render_view`, the base64 PNG.
struct Reply {
    value: Value,
    image: Option<String>,
}

impl From<Value> for Reply {
    fn from(value: Value) -> Self {
        Self { value, image: None }
    }
}

fn tool_result(value: Value, image: Option<String>, is_error: bool, structured: bool) -> Value {
    let mut content = Vec::new();
    if let Some(data) = image {
        content.push(json!({"type": "image", "data": data, "mimeType": "image/png"}));
    }
    content.push(json!({"type": "text", "text": value.to_string()}));
    let mut result = json!({"content": content, "isError": is_error});
    if structured && value.is_object() {
        result["structuredContent"] = value;
    }
    result
}

fn run(backend: &mut dyn Backend, tool: &str, arguments: Value) -> Result<Reply, RpcError> {
    match tool {
        "describe" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                schema: Option<String>,
            }
            let args: Args = arguments_as(arguments)?;
            let document = backend.call("describe", json!({}))?;
            match args.schema {
                None => Ok(overview(document).into()),
                Some(name) => named_schema(&document, &name).map(Reply::from),
            }
        }
        "transact" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                commands: Vec<Value>,
                atomic: Option<bool>,
            }
            let mut args: Args = arguments_as(arguments)?;
            match args.commands.len() {
                0 => Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    "invalid params: commands must hold at least one command",
                )),
                1 => {
                    let command = args.commands.pop().expect("one command");
                    Ok(as_batch(backend.call("execute", command)?).into())
                }
                _ => {
                    let atomic = args.atomic.unwrap_or(true);
                    let params = json!({"commands": args.commands, "atomic": atomic});
                    Ok(backend.call("batch", params)?.into())
                }
            }
        }
        "query" | "tool_input" => {
            let arguments = Value::Object(object(arguments)?);
            Ok(backend.call(tool, arguments)?.into())
        }
        "pick" | "infer" => {
            let mut arguments = object(arguments)?;
            arguments.insert("type".into(), json!(tool));
            Ok(backend.call("query", Value::Object(arguments))?.into())
        }
        "render_view" => {
            let mut arguments = object(arguments)?;
            for (side, default) in [("width", DEFAULT_WIDTH), ("height", DEFAULT_HEIGHT)] {
                let value = arguments.entry(side).or_insert(Value::Null);
                if value.is_null() {
                    *value = json!(default);
                }
            }
            let mut value = backend.call("render_view", Value::Object(arguments))?;
            let image = match value.as_object_mut().and_then(|v| v.remove("png_base64")) {
                Some(Value::String(png)) => Some(png),
                _ => None,
            };
            Ok(Reply { value, image })
        }
        _ => unreachable!("no tool {tool}"),
    }
}

/// `execute`'s reply in `batch`'s shape (`{outcomes, changes, revision}`), or as it is for an
/// editor command (`{editor, revision}`).
fn as_batch(reply: Value) -> Value {
    let Value::Object(mut outcome) = reply else {
        return reply;
    };
    if outcome.contains_key("editor") {
        return Value::Object(outcome);
    }
    let revision = outcome.remove("revision").unwrap_or(Value::Null);
    let changes = outcome.get("changes").cloned().unwrap_or(Value::Null);
    json!({"outcomes": [outcome], "changes": changes, "revision": revision})
}

/// The describe document without its schemas (the full one is hundreds of kilobytes), with
/// the names `describe {schema}` takes instead.
fn overview(mut document: Value) -> Value {
    let names: Vec<String> = match document.as_object_mut().and_then(|d| d.remove("schemas")) {
        Some(Value::Object(schemas)) => schemas.into_iter().map(|(name, _)| name).collect(),
        _ => Vec::new(),
    };
    if let Some(methods) = document["methods"].as_object_mut() {
        for method in methods.values_mut().filter_map(Value::as_object_mut) {
            method.remove("params_schema");
            method.remove("result_schema");
        }
    }
    document["schema_names"] = json!(names);
    document
}

/// The describe document's schema called `name`, or the params and result schemas of the
/// method called `name`.
fn named_schema(document: &Value, name: &str) -> Result<Value, RpcError> {
    if let Some(schema) = document["schemas"].get(name) {
        return Ok(schema.clone());
    }
    if let Some(method) = document["methods"].get(name) {
        return Ok(json!({
            "params_schema": method["params_schema"],
            "result_schema": method["result_schema"],
        }));
    }
    Err(RpcError::new(
        codes::INVALID_PARAMS,
        format!(
            "no schema or method named {name:?}; describe without schema lists schema_names and methods"
        ),
    ))
}

/// The arguments as a JSON object.
fn object(arguments: Value) -> Result<Map<String, Value>, RpcError> {
    match arguments {
        Value::Object(m) => Ok(m),
        _ => Err(RpcError::new(
            codes::INVALID_PARAMS,
            "invalid params: arguments must be an object",
        )),
    }
}

fn arguments_as<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, RpcError> {
    serde_json::from_value(Value::Object(object(arguments)?))
        .map_err(|e| RpcError::new(codes::INVALID_PARAMS, format!("invalid params: {e}")))
}
