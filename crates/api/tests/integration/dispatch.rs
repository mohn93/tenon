use api::{HeadlessHost, codes, handle};
use base64::Engine;
use serde_json::{Value, json};

fn call(host: &mut HeadlessHost, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}).to_string();
    let reply: Value = serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap();
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 7);
    reply
}

fn square() -> Value {
    let c = [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]];
    json!(
        (0..4)
            .map(|i| json!({"type": "add_edge", "start": c[i], "end": c[(i + 1) % 4]}))
            .collect::<Vec<_>>()
    )
}

#[test]
fn execute_returns_changes_and_revision() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
    assert_eq!(r["result"]["changes"]["created"], json!(["v0", "v1", "e2"]));
    assert_eq!(r["result"]["revision"], 1);
}

#[test]
fn command_errors_carry_structured_data() {
    let mut h = HeadlessHost::new();
    let r = call(&mut h, "execute", json!({"type": "undo"}));
    assert_eq!(r["error"]["code"], codes::COMMAND_FAILED);
    assert_eq!(r["error"]["data"]["kind"], "nothing_to_undo");
    let r = call(&mut h, "query", json!({"type": "entity", "id": "f9"}));
    assert_eq!(r["error"]["data"]["ids"], json!(["f9"]));
}

#[test]
fn undo_expect_with_an_unknown_key_is_invalid_and_undoes_nothing() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "undo", "expect": {"label": "add_edge", "orgin": "agent"}}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(stats["result"]["edges"], 1);
    assert_eq!(stats["result"]["revision"], 1);
}

#[test]
fn batch_and_query_round_trip() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": square(), "atomic": true}),
    );
    assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 4);
    assert_eq!(r["result"]["revision"], 1);
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(stats["result"]["faces"], 1);
}

#[test]
fn outliner_query_round_trip() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "batch",
        json!({"commands": square(), "atomic": true}),
    );
    let all = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["edge", "face"]}),
    );
    let ids: Vec<Value> = ["edges", "faces"]
        .iter()
        .flat_map(|k| all["result"][k].as_array().unwrap().clone())
        .map(|e| e["id"].clone())
        .collect();
    let r = call(&mut h, "execute", json!({"type": "make_group", "ids": ids}));
    let group = r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    let tree = call(&mut h, "query", json!({"type": "outliner"}));
    let result = &tree["result"];
    assert_eq!(result["type"], "outliner");
    assert_eq!(result["revision"], 2);
    let node = &result["instances"][0];
    assert_eq!(node["id"], group);
    assert_eq!(node["kind"], "group");
    assert_eq!(node["selected"], true, "make_group selects the new group");
    assert_eq!(node["children"], json!([]));
}

#[test]
fn failed_atomic_batch_reports_where_it_stopped() {
    let mut h = HeadlessHost::new();
    let mut cmds = square();
    cmds.as_array_mut().unwrap().push(json!({"type": "undo"}));
    let r = call(&mut h, "batch", json!({"commands": cmds, "atomic": true}));
    assert_eq!(r["error"]["code"], codes::BATCH_FAILED);
    assert_eq!(r["error"]["data"]["index"], 4);
    assert_eq!(r["error"]["data"]["rolled_back"], true);
    assert_eq!(
        call(&mut h, "query", json!({"type": "stats"}))["result"]["edges"],
        0
    );
}

#[test]
fn protocol_errors_use_standard_codes() {
    let mut h = HeadlessHost::new();
    let parse: Value = serde_json::from_str(&handle(&mut h, "{not json").unwrap()).unwrap();
    assert_eq!(parse["error"]["code"], codes::PARSE_ERROR);
    assert_eq!(parse["id"], Value::Null);

    let no_version = json!({"id": 1, "method": "describe"}).to_string();
    let r: Value = serde_json::from_str(&handle(&mut h, &no_version).unwrap()).unwrap();
    assert_eq!(r["error"]["code"], codes::INVALID_REQUEST);
    assert_eq!(r["id"], 1);

    assert_eq!(
        call(&mut h, "fly", json!(null))["error"]["code"],
        codes::METHOD_NOT_FOUND
    );
    let bad = call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0]}),
    );
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS);
}

#[test]
fn notifications_get_no_reply_but_still_run() {
    let mut h = HeadlessHost::new();
    let line = json!({"jsonrpc": "2.0", "method": "execute", "params": {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}}).to_string();
    assert!(handle(&mut h, &line).is_none());
    assert_eq!(h.doc.revision(), 1);
}

#[test]
fn describe_lists_methods_and_schemas() {
    let mut h = HeadlessHost::new();
    let d = call(&mut h, "describe", json!(null))["result"].clone();
    assert_eq!(d["schema_version"], 2);
    for m in [
        "execute",
        "batch",
        "query",
        "render_view",
        "subscribe",
        "describe",
    ] {
        assert!(d["methods"][m].is_object(), "missing method {m}");
    }
    // The execute result names every field an outcome has.
    let result = d["methods"]["execute"]["result"].as_str().unwrap();
    let outcome = d["schemas"]["Outcome"]["properties"].as_object().unwrap();
    assert!(outcome.len() >= 5, "{outcome:?}");
    for field in outcome.keys() {
        assert!(result.contains(field.as_str()), "{field} in {result}");
    }
    let command_schema = d["schemas"]["Command"].to_string();
    for variant in [
        "add_edge",
        "add_rectangle",
        "add_circle",
        "add_polygon",
        "add_arc",
        "add_curve",
        "add_helix",
        "add_rotated_rectangle",
        "look_around",
        "position_camera",
        "push_pull",
        "erase",
        "move",
        "copy",
        "copy_along_path",
        "rotate",
        "scale",
        "offset",
        "follow_me",
        "revolve",
        "add_guide_line",
        "add_guide_point",
        "delete_guides",
        "add_dimension",
        "add_edge_dimension",
        "add_radial_dimension",
        "add_note",
        "add_screen_note",
        "change_note",
        "add_3d_text",
        "import_image",
        "set_units",
        "add_section_plane",
        "flip_section_plane",
        "activate_section_plane",
        "deactivate_section",
        "set_section_display",
        "select",
        "undo",
        "redo",
        "set_camera",
        "make_group",
        "make_component",
        "explode",
        "open_instance",
        "close_instance",
        "open_path",
        "rename_definition",
        "place_instance",
        "set_transform",
        "add_box",
        "add_cylinder",
        "add_tube",
        "add_cone",
        "add_prism",
        "add_sphere",
        "add_torus",
        "purge_definitions",
        "add_material",
        "change_material",
        "delete_material",
        "paint",
        "add_tag",
        "change_tag",
        "delete_tag",
        "set_tag",
        "add_scene",
        "update_scene",
        "change_scene",
        "move_scene",
        "delete_scene",
        "go_to_scene",
        "add_style",
        "change_style",
        "delete_style",
        "set_style",
        "add_joint",
        "change_joint",
        "delete_joint",
        "drive",
        "add_coupling",
        "delete_coupling",
        "add_cam_follower",
        "delete_cam_follower",
        "position_texture",
        "soften_edges",
        "soften_by_angle",
        "intersect_faces",
        "solid_union",
        "solid_subtract",
        "solid_intersect",
        "solid_trim",
        "solid_split",
        "outer_shell",
        "make_solid",
        "chamfer_edges",
        "fillet_edges",
        "paste",
        "new",
        "open",
        "save",
        "export",
    ] {
        assert!(
            command_schema.contains(variant),
            "Command schema lacks {variant}"
        );
    }
    let query_schema = d["schemas"]["Query"].to_string();
    for variant in [
        "events",
        "definitions",
        "context",
        "file",
        "curve",
        "\"solid\"",
        "materials",
        "tags",
        "scenes",
        "styles",
        "joints",
        "\"joint\"",
        "couplings",
        "cam_followers",
        "\"scene\"",
        "outliner",
        "annotations",
        "units",
        "section_planes",
        "section_display",
        "interference",
        "cut_list",
        "model_health",
        "clearance",
        "sweep_check",
    ] {
        assert!(
            query_schema.contains(variant),
            "Query schema lacks {variant}"
        );
    }
}

#[test]
fn solid_query_reports_volume_over_rpc() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [2, 3, 4], "as": "group"}),
    );
    let id = r["result"]["placed"][0].clone();
    let r = call(&mut h, "query", json!({"type": "solid", "id": id}));
    assert_eq!(r["result"]["type"], "solid");
    assert_eq!(r["result"]["solid"], true);
    assert!((r["result"]["volume"].as_f64().unwrap() - 24.0).abs() < 1e-9);
    let r = call(&mut h, "query", json!({"type": "solid", "id": "f1"}));
    assert_eq!(r["error"]["code"], codes::COMMAND_FAILED);
}

#[test]
fn rectangle_and_push_pull_build_a_box() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 3, 0]}),
    );
    let face = r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": face, "distance": 1.5}),
    );
    assert_eq!(r["result"]["revision"], 2);
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(
        (&stats["result"]["vertices"], &stats["result"]["faces"]),
        (&json!(8), &json!(6))
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": "e2", "distance": 1}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input");
    assert_eq!(r["error"]["data"]["ids"], json!(["e2"]));
    let top = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["face"], "within": {"min": [-1, -1, 1.4], "max": [3, 4, 1.6]}}),
    );
    assert_eq!(top["result"]["faces"].as_array().unwrap().len(), 1);
}

#[test]
fn select_then_erase_the_selection() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    call(
        &mut h,
        "execute",
        json!({"type": "select", "ids": ["f8", "e2"]}),
    );
    let sel = call(&mut h, "query", json!({"type": "selection"}));
    assert_eq!(sel["result"]["ids"], json!(["e2", "f8"]));
    let ids = sel["result"]["ids"].clone();
    let r = call(&mut h, "execute", json!({"type": "erase", "ids": ids}));
    assert_eq!(r["result"]["revision"], 2);
    let sel = call(&mut h, "query", json!({"type": "selection"}));
    assert_eq!(sel["result"]["ids"], json!([]));
    let r = call(&mut h, "execute", json!({"type": "erase", "ids": ["v0"]}));
    assert_eq!(r["error"]["data"]["ids"], json!(["v0"]));
}

#[test]
fn move_and_copy_over_json_rpc() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}),
    );
    assert_eq!(
        r["result"]["changes"]["modified"].as_array().unwrap().len(),
        4
    );
    call(
        &mut h,
        "execute",
        json!({"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 2}),
    );
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(stats["result"]["faces"], 3);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "move", "ids": ["v0"], "offset": [0, 0, 1]}),
    );
    assert_eq!(
        r["error"]["data"]["ids"],
        json!(["f8"]),
        "lifting one corner bends f8"
    );
}

/// The id of the first face in a command result's `created` list.
fn created_face(r: &Value) -> Value {
    r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone()
}

#[test]
fn push_pull_opens_a_window_and_closes_a_recess_over_json_rpc() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 0.3, 0]}),
    );
    let base = created_face(&r);
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": base, "distance": 3}),
    );
    let faces = |h: &mut HeadlessHost| {
        call(h, "query", json!({"type": "stats"}))["result"]["faces"].clone()
    };
    assert_eq!(faces(&mut h), json!(6));

    // A window on the front (y = 0, facing -y) pushed back by the wall's thickness opens.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [1, 0, 1], "opposite": [2, 0, 2], "normal": [0, -1, 0]}),
    );
    let window = created_face(&r);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": window, "distance": -0.3}),
    );
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(faces(&mut h), json!(10));
    call(&mut h, "execute", json!({"type": "undo"}));

    // Pushed in part way it is a recess; pushed back flush, the wall is whole again.
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": window, "distance": -0.1}),
    );
    let floor = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["face"], "within": {"min": [1, 0.05, 1], "max": [2, 0.15, 2]}}),
    );
    let floor = floor["result"]["faces"][0]["id"].clone();
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": floor, "distance": 0.1}),
    );
    assert_eq!(faces(&mut h), json!(6));
}

#[test]
fn render_view_returns_a_png_or_reports_no_gpu() {
    let mut h = HeadlessHost::new();
    call(&mut h, "batch", json!({"commands": square()}));
    let r = call(&mut h, "render_view", json!({"width": 64, "height": 48}));
    if r["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(r["result"]["width"], 64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(r["result"]["png_base64"].as_str().unwrap())
        .unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (64, 48));

    let bad = call(&mut h, "render_view", json!({"width": 0, "height": 48}));
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS);
    let on_target = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "camera": {"eye": [0, 0, 5], "target": [0, 0, 5]}}),
    );
    assert_eq!(on_target["error"]["code"], codes::COMMAND_FAILED);
}

#[test]
fn render_view_takes_a_parallel_top_view() {
    let mut h = HeadlessHost::new();
    call(&mut h, "batch", json!({"commands": square()}));
    let before = h.doc.camera.clone();
    let top =
        json!({"eye": [1, 1, 5], "target": [1, 1, 0], "up": [0, 1, 0], "projection": "parallel"});
    let r = call(
        &mut h,
        "render_view",
        json!({"width": 64, "height": 48, "camera": top}),
    );
    assert_eq!(h.doc.camera, before);
    let vertical_up = json!({"eye": [1, 1, 5], "target": [1, 1, 0], "up": [0, 0, 1]});
    let bad = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "camera": vertical_up}),
    );
    assert_eq!(bad["error"]["code"], codes::COMMAND_FAILED);
    if r["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert!(r["error"].is_null(), "{r}");
}

#[test]
fn render_view_rejects_an_unknown_projection() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "camera": {"eye": [0, -5, 0], "target": [0, 0, 0], "projection": "ortho"}}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
}

#[test]
fn render_view_does_not_move_the_document_camera() {
    let mut h = HeadlessHost::new();
    let before = h.doc.camera.clone();
    call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "camera": {"eye": [10, 0, 5], "target": [0, 0, 0]}}),
    );
    assert_eq!(h.doc.camera, before);
}

/// The first pixel of a `render_view` reply's PNG.
fn first_pixel(reply: &Value) -> [u8; 3] {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(reply["result"]["png_base64"].as_str().unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut buf).unwrap();
    [buf[0], buf[1], buf[2]]
}

#[test]
fn render_view_takes_studio_lighting_without_changing_the_documents() {
    let mut h = HeadlessHost::new();
    let level = json!({"eye": [0, -6, 1.5], "target": [0, 0, 1.5]});
    let studio = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level, "lighting": "studio"}),
    );
    assert_eq!(h.doc.lighting, session::Lighting::Day);
    if studio["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    // The backdrop is the day's background; the pixels are the same where nothing is drawn.
    let day = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level}),
    );
    assert_eq!(first_pixel(&studio), first_pixel(&day));
    // The document's own studio lighting is what a render without `lighting` follows.
    let set = call(
        &mut h,
        "execute",
        json!({"type": "set_lighting", "mode": "studio"}),
    );
    assert!(set.get("error").is_none(), "{set}");
    assert_eq!(h.doc.lighting, session::Lighting::Studio);
}

#[test]
fn render_view_takes_the_lighting_without_changing_the_documents() {
    let mut h = HeadlessHost::new();
    // Looking level, the top left pixel is sky.
    let level = json!({"eye": [0, -6, 1.5], "target": [0, 0, 1.5]});
    let night = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level, "lighting": "night"}),
    );
    assert_eq!(h.doc.lighting, session::Lighting::Day);
    let bad = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "lighting": "dusk"}),
    );
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS);
    if night["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    let day = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level}),
    );
    let sky = |c: [f32; 4]| c.map(|x| (x * 255.0).round() as u8);
    let [r, g, b, _] = sky(render::grid::NIGHT_BACKGROUND);
    assert_eq!(first_pixel(&night), [r, g, b]);
    let [r, g, b, _] = sky(render::grid::BACKGROUND);
    assert_eq!(first_pixel(&day), [r, g, b]);
    // Without `lighting` it follows the document's.
    call(
        &mut h,
        "execute",
        json!({"type": "set_lighting", "mode": "night"}),
    );
    let document_night = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level}),
    );
    assert_eq!(first_pixel(&document_night), first_pixel(&night));
}

#[test]
fn render_view_draws_with_the_chosen_style() {
    let mut h = HeadlessHost::new();
    let added = call(&mut h, "execute", json!({"type": "add_style"}));
    let id = added["result"]["changes"]["styles"][0].clone();
    assert!(id.is_string(), "{added}");
    let change =
        json!({"type": "change_style", "style": id, "settings": {"background": [10, 200, 30]}});
    call(&mut h, "execute", change);
    call(&mut h, "execute", json!({"type": "set_style", "style": id}));
    let level = json!({"eye": [0, -6, 1.5], "target": [0, 0, 1.5]});
    let styled = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level}),
    );
    if styled["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(first_pixel(&styled), [10, 200, 30]);
    // Night ignores the style's background.
    let night = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level, "lighting": "night"}),
    );
    let [r, g, b, _] = render::grid::NIGHT_BACKGROUND.map(|x| (x * 255.0).round() as u8);
    assert_eq!(first_pixel(&night), [r, g, b]);
    // Choosing nothing brings back the classic background.
    call(
        &mut h,
        "execute",
        json!({"type": "set_style", "style": null}),
    );
    let plain = call(
        &mut h,
        "render_view",
        json!({"width": 16, "height": 16, "camera": level}),
    );
    let [r, g, b, _] = render::grid::BACKGROUND.map(|x| (x * 255.0).round() as u8);
    assert_eq!(first_pixel(&plain), [r, g, b]);
}

#[test]
fn misspelled_params_are_invalid_params() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "render_view",
        json!({"widht": 8, "width": 8, "height": 8}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS);
    let r = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "camera": {"eye": [10, 0, 5], "target": [0, 0, 0], "fov": 1}}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS);
    let r = call(&mut h, "batch", json!({"commands": [], "atomik": true}));
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS);
}

#[test]
fn ids_must_be_string_number_or_null() {
    let mut h = HeadlessHost::new();
    for bad in [json!({"a": 1}), json!([1]), json!(true)] {
        let line = json!({"jsonrpc": "2.0", "id": bad, "method": "describe"}).to_string();
        let r: Value = serde_json::from_str(&handle(&mut h, &line).unwrap()).unwrap();
        assert_eq!(r["error"]["code"], codes::INVALID_REQUEST);
        assert_eq!(r["id"], Value::Null);
    }
    for ok in [json!("abc"), json!(3), Value::Null] {
        let line = json!({"jsonrpc": "2.0", "id": ok, "method": "describe"}).to_string();
        let r: Value = serde_json::from_str(&handle(&mut h, &line).unwrap()).unwrap();
        assert!(r.get("result").is_some());
    }
}

#[test]
fn components_over_json_rpc() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    );
    let all = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["edge", "face"]}),
    )["result"]
        .clone();
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "make_component", "ids": ids, "name": "Crate"}),
    );
    let first = call(&mut h, "query", json!({"type": "selection"}))["result"]["ids"][0].clone();
    assert!(
        r["result"]["changes"]["created"]
            .as_array()
            .unwrap()
            .contains(&first)
    );
    call(
        &mut h,
        "execute",
        json!({"type": "copy", "ids": [first], "offset": [3, 0, 0]}),
    );

    call(
        &mut h,
        "execute",
        json!({"type": "open_instance", "id": first}),
    );
    let faces = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["face"]}),
    );
    let top = faces["result"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["id"]
        .clone();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": top, "distance": 1.0}),
    );
    assert!(r["result"].is_object(), "{r}");
    let png = call(&mut h, "render_view", json!({"width": 64, "height": 64}));
    assert!(
        png["result"]["png_base64"].is_string()
            || png["error"]["code"] == codes::RENDER_UNAVAILABLE,
        "{png}"
    );
    call(&mut h, "execute", json!({"type": "close_instance"}));

    let placed = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["instance"]}),
    );
    let tops: Vec<Value> = placed["result"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["bounds"]["max"][2].clone())
        .collect();
    assert_eq!(tops, [json!(2.0), json!(2.0)], "both copies grew");
    let defs = call(&mut h, "query", json!({"type": "definitions"}));
    assert_eq!(defs["result"]["definitions"][0]["instances"], 2);

    let r = call(
        &mut h,
        "execute",
        json!({"type": "explode", "ids": [first]}),
    );
    assert!(r["result"].is_object(), "{r}");
    let stats = call(&mut h, "query", json!({"type": "stats"}))["result"].clone();
    assert_eq!(
        (stats["faces"].clone(), stats["instances"].clone()),
        (json!(6), json!(1))
    );
}

#[test]
fn import_reports_what_it_made_over_json_rpc() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("api-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tri.stl");
    std::fs::write(
        &path,
        "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\n\
         endloop\nendfacet\nendsolid t\n",
    )
    .unwrap();
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "import", "path": path.to_str().unwrap()}),
    );
    let result = &r["result"];
    assert_eq!(result["revision"], 1);
    assert_eq!(result["import"]["faces"], 1);
    assert_eq!(result["import"]["polygons"], 1);
    assert!(
        result["import"]["instance"]
            .as_str()
            .unwrap()
            .starts_with('i')
    );
    let plain = call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [5, 0, 0], "end": [6, 0, 0]}),
    );
    assert!(plain["result"].get("import").is_none());
}

#[test]
fn soften_over_json_rpc() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1, "segments": 12}),
    );
    let face = created_face(&r);
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": face, "distance": -2}),
    );
    // The pulled cylinder's 12 side edges are soft already; the 24 rim edges meet at 90 degrees.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "soften_by_angle", "max_angle_deg": 89}),
    );
    assert_eq!(r["result"]["soften"], json!({"changed": 0, "skipped": 24}));
    assert!(r["result"]["revision"].is_number());
    let r = call(
        &mut h,
        "execute",
        json!({"type": "soften_edges", "ids": [face], "soft": true}),
    );
    assert_eq!(r["result"]["soften"], json!({"changed": 12, "skipped": 0}));
    let r = call(
        &mut h,
        "execute",
        json!({"type": "soften_edges", "ids": [face]}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS);
}

#[test]
fn intersect_faces_over_json_rpc() {
    let mut h = HeadlessHost::new();
    // A rectangle pierced by a wall through its middle.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 4, 0]}),
    );
    let floor = created_face(&r);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [-1, 2, -1], "opposite": [5, 2, 1],
            "normal": [0, 1, 0]}),
    );
    let wall = created_face(&r);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "intersect_faces", "ids": [wall], "with": "model"}),
    );
    let report = &r["result"]["intersect"];
    assert_eq!(report["split_faces"], json!([floor]), "{r}");
    assert_eq!(report["created_faces"].as_array().unwrap().len(), 2, "{r}");
    assert!(!report["added_edges"].as_array().unwrap().is_empty());
    // Nothing left to cross: a success with an empty report.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "intersect_faces", "ids": [wall]}),
    );
    assert_eq!(
        r["result"]["intersect"],
        json!({"added_edges": [], "split_faces": [], "created_faces": []})
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "intersect_faces", "ids": [wall], "with": "everything"}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    let r = call(&mut h, "execute", json!({"type": "select", "ids": []}));
    assert!(r.get("error").is_none());
    let r = call(&mut h, "execute", json!({"type": "intersect_faces"}));
    assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{r}");
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
}

#[test]
fn select_all_runs_and_box_refuses_all() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "batch",
        json!({"commands": square(), "atomic": true}),
    );
    let r = call(&mut h, "execute", json!({"type": "select", "all": true}));
    assert!(r.get("error").is_none(), "{r}");
    let all = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["edge", "face"]}),
    );
    let mut every: Vec<Value> = ["edges", "faces"]
        .iter()
        .flat_map(|k| all["result"][k].as_array().unwrap().clone())
        .map(|e| e["id"].clone())
        .collect();
    every.sort_by_key(|id| id.to_string());
    let selection = call(&mut h, "query", json!({"type": "selection"}));
    let mut ids = selection["result"]["ids"].as_array().unwrap().clone();
    ids.sort_by_key(|id| id.to_string());
    assert_eq!(ids.len(), 5);
    assert_eq!(ids, every);

    let r = call(
        &mut h,
        "execute",
        json!({"type": "select", "box": {"from": [0, 0], "to": [100, 100], "width": 100,
            "height": 100}, "all": true}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
}

#[test]
fn batch_reports_the_net_changes() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": square(), "atomic": true}),
    );
    let net = &r["result"]["changes"];
    assert_eq!(net["created"].as_array().unwrap().len(), 4 + 4 + 1); // 4 vertices, 4 edges, the face
    assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 4);
}

#[test]
fn api_steps_are_agent_steps() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
    let r = call(&mut h, "query", json!({"type": "history"}));
    assert_eq!(r["result"]["undo_origins"], json!(["agent"]));
}

#[test]
fn describe_has_schemas_per_method() {
    let d = api::describe();
    assert_eq!(d["schema_version"], 2);
    for m in [
        "execute",
        "batch",
        "query",
        "render_view",
        "tool_input",
        "quit",
    ] {
        assert!(d["methods"][m]["params_schema"].is_object(), "{m}");
        assert!(d["methods"][m]["result_schema"].is_object(), "{m}");
    }
    for m in ["subscribe", "describe"] {
        assert!(d["methods"][m]["params_schema"].is_null(), "{m}");
    }
    let exec = d["methods"]["execute"]["result_schema"].to_string();
    assert!(exec.contains("\"revision\""));
    assert!(
        d["methods"]["batch"]["result_schema"]
            .to_string()
            .contains("\"changes\"")
    );
    // execute takes a Command, a ScreenCommand or an EditorCommand; query a Query or an
    // EditorQuery.
    let execute = &d["methods"]["execute"]["params_schema"];
    assert_eq!(execute["type"], "object");
    assert_eq!(execute["anyOf"].as_array().unwrap().len(), 3);
    let query = &d["methods"]["query"]["params_schema"];
    assert_eq!(query["type"], "object");
    assert_eq!(query["anyOf"].as_array().unwrap().len(), 2);
}

/// Every `$ref` under `v`.
fn refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x) {
                    ("$ref", Value::String(r)) => out.push(r.clone()),
                    _ => refs(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
        _ => {}
    }
}

#[test]
fn method_schemas_are_self_contained() {
    let d = api::describe();
    let methods = d["methods"].as_object().unwrap();
    for (name, method) in methods {
        for key in ["params_schema", "result_schema"] {
            let schema = &method[key];
            if schema.is_null() {
                continue;
            }
            let mut found = Vec::new();
            refs(schema, &mut found);
            for r in found {
                let def = r
                    .strip_prefix("#/$defs/")
                    .unwrap_or_else(|| panic!("{name} {key}: ref {r} is not to $defs"));
                assert!(
                    schema["$defs"][def].is_object(),
                    "{name} {key}: ref {r} does not resolve"
                );
            }
        }
    }
}

/// The variant of a tagged enum schema (`oneOf`/`anyOf` of objects) whose `type` is `tag`.
fn variant<'a>(schema: &'a Value, tag: &str) -> &'a Value {
    let variants = schema["oneOf"]
        .as_array()
        .or_else(|| schema["anyOf"].as_array())
        .unwrap();
    variants
        .iter()
        .map(|v| match v["$ref"].as_str() {
            Some(r) => &schema["$defs"][r.trim_start_matches("#/$defs/")],
            None => v,
        })
        .find(|v| {
            let t = &v["properties"]["type"];
            t["const"] == tag || t["enum"] == json!([tag])
        })
        .unwrap_or_else(|| panic!("no variant {tag}"))
}

#[test]
fn command_schemas_have_an_object_root_and_bounded_points() {
    let d = api::describe();
    for name in [
        "Command",
        "Query",
        "EditorCommand",
        "EditorQuery",
        "ScreenCommand",
    ] {
        assert_eq!(d["schemas"][name]["type"], "object", "{name}");
    }
    let bounded = |p: &Value| {
        assert_eq!(p["items"]["minimum"], -1e6, "{p}");
        assert_eq!(p["items"]["maximum"], 1e6, "{p}");
    };
    let command = &d["schemas"]["Command"];
    let add_edge = variant(command, "add_edge");
    bounded(&add_edge["properties"]["start"]);
    bounded(&add_edge["properties"]["end"]);
    bounded(&variant(command, "add_arc")["properties"]["through"]);
    bounded(&variant(command, "set_camera")["properties"]["eye"]);
    // Directions are not points: they stay unbounded.
    let guide = variant(command, "add_guide_line");
    bounded(&guide["properties"]["point"]);
    assert!(guide["properties"]["direction"]["items"]["maximum"].is_null());
    // The API's own model points carry the bounds too.
    let tool_input = d["schemas"]["ToolInput"].to_string();
    assert!(tool_input.contains("1000000.0"), "{tool_input}");
}

fn rectangle() -> Value {
    json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 3, 0]})
}

#[test]
fn batch_references_build_a_box_over_json_rpc() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [rectangle(), {"type": "push_pull", "face": "$0:f0", "distance": 1}]}),
    );
    let result = &r["result"];
    assert_eq!(result["outcomes"].as_array().unwrap().len(), 2, "{r}");
    let faces = result["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with('f'))
        .count();
    assert_eq!(faces, 6, "{r}");
    assert!(result["revision"].is_u64(), "{r}");
}

#[test]
fn a_bad_reference_is_invalid_params_and_runs_nothing() {
    let mut h = HeadlessHost::new();
    let before = call(&mut h, "query", json!({"type": "history"}))["result"]["revision"].clone();
    assert!(before.is_u64(), "{before}");
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [
            {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]},
            {"type": "push_pull", "face": "$1:f0", "distance": 1}
        ]}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("invalid params: commands[1]: "),
        "{r}"
    );
    let after = call(&mut h, "query", json!({"type": "history"}))["result"]["revision"].clone();
    assert_eq!(before, after);
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(stats["result"]["edges"], 0);
}

#[test]
fn a_reference_past_what_was_made_is_a_batch_failure() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [rectangle(), {"type": "push_pull", "face": "$0:f3", "distance": 1}], "atomic": true}),
    );
    assert_eq!(r["error"]["code"], codes::BATCH_FAILED, "{r}");
    let data = &r["error"]["data"];
    assert_eq!(data["index"], 1);
    assert_eq!(data["error"]["kind"], "invalid_input");
    assert_eq!(data["rolled_back"], true);
}

#[test]
fn a_reference_across_new_is_invalid_params_and_runs_nothing() {
    let mut h = HeadlessHost::new();
    let before = call(&mut h, "query", json!({"type": "history"}))["result"]["revision"].clone();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [
            rectangle(),
            {"type": "new", "discard": true},
            {"type": "add_rectangle", "corner": [5, 5, 0], "opposite": [7, 7, 0]},
            {"type": "push_pull", "face": "$0:f0", "distance": 1}
        ], "atomic": false}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(message.starts_with("invalid params: commands[3]: "), "{r}");
    assert!(message.contains("replaced the document"), "{r}");
    let after = call(&mut h, "query", json!({"type": "history"}))["result"]["revision"].clone();
    assert_eq!(before, after);
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(stats["result"]["faces"], 0);
}

#[test]
fn a_huge_written_id_without_references_fails_at_its_turn() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [{"type": "erase", "ids": ["f4611686018427387904"]}]}),
    );
    assert_eq!(r["error"]["code"], codes::BATCH_FAILED, "{r}");
    assert_eq!(r["error"]["data"]["error"]["kind"], "not_found", "{r}");
}

#[test]
fn an_invalid_command_names_its_index() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [
            {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]},
            {"type": "no_such"}
        ]}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("commands[1]"),
        "{r}"
    );
}

#[test]
fn describe_batch_schema_accepts_references() {
    let d = api::describe();
    let defs = &d["methods"]["batch"]["params_schema"]["$defs"];
    let entity = defs["EntityId"]["anyOf"].as_array().unwrap();
    assert_eq!(entity[1], json!({"$ref": "#/$defs/BatchRef"}));
    let material = defs["MaterialId"]["anyOf"].as_array().unwrap();
    assert_eq!(material[1], json!({"$ref": "#/$defs/BatchRef"}));
    let definition = defs["DefinitionId"]["anyOf"].as_array().unwrap();
    assert_eq!(definition[1], json!({"$ref": "#/$defs/BatchRef"}));
    assert_eq!(
        defs["BatchRef"]["pattern"],
        r"^\$(0|[1-9][0-9]*):(([vefigrnsmdyjkw])(0|[1-9][0-9]*|\*)|\*)$"
    );
    let style = defs["StyleId"]["anyOf"].as_array().unwrap();
    assert_eq!(style[1], json!({"$ref": "#/$defs/BatchRef"}));
    let coupling = defs["CouplingId"]["anyOf"].as_array().unwrap();
    assert_eq!(coupling[1], json!({"$ref": "#/$defs/BatchRef"}));
    let follower = defs["CamFollowerId"]["anyOf"].as_array().unwrap();
    assert_eq!(follower[1], json!({"$ref": "#/$defs/BatchRef"}));
    assert!(
        d["methods"]["execute"]["params_schema"]["$defs"]
            .get("BatchRef")
            .is_none()
    );
    assert!(d["schemas"]["BatchRef"].is_object());
}

#[test]
fn render_view_honours_the_section_display_settings() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    call(
        &mut h,
        "execute",
        json!({"type": "add_section_plane", "point": [1, 0, 0], "normal": [1, 0, 0]}),
    );
    let top =
        json!({"eye": [1, 1, 6], "target": [1, 1, 0], "up": [0, 1, 0], "projection": "parallel"});
    let view = |h: &mut HeadlessHost| {
        call(
            h,
            "render_view",
            json!({"width": 64, "height": 48, "camera": top}),
        )
    };
    let cut = view(&mut h);
    if cut["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    call(
        &mut h,
        "execute",
        json!({"type": "set_section_display", "cuts": false}),
    );
    let whole = view(&mut h);
    assert_ne!(
        cut["result"]["png_base64"], whole["result"]["png_base64"],
        "the cut half is missing only with cuts on"
    );
    call(
        &mut h,
        "execute",
        json!({"type": "set_section_display", "cuts": true}),
    );
    assert_eq!(
        view(&mut h)["result"]["png_base64"],
        cut["result"]["png_base64"]
    );
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("api-pdf-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_gpu(r: &Value) -> bool {
    r["error"]["message"] == "printing needs a GPU adapter"
}

#[test]
fn export_pdf_writes_a_pdf_with_one_page_per_sheet_or_reports_no_gpu() {
    let mut h = HeadlessHost::new();
    call(&mut h, "batch", json!({"commands": square()}));
    // Look down at it: parallel, so a scale can apply.
    call(
        &mut h,
        "execute",
        json!({"type": "set_camera", "eye": [1, 1, 50], "target": [1, 1, 0], "fov_y_deg": 30,
               "up": [0, 1, 0], "projection": "parallel"}),
    );
    let dir = scratch_dir("ok");
    let fit = dir.join("fit.pdf");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": fit.display().to_string(), "dpi": 72}),
    );
    if no_gpu(&r) {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(r["result"]["pdf"]["sheets"], 1, "{r}");
    assert_eq!(r["result"]["pdf"]["warnings"], json!([]));
    let bytes = std::fs::read(&fit).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(String::from_utf8_lossy(&bytes).contains("/Count 1"));

    // Exactly 1:200 on A4 landscape: the 2 m square fits one sheet; 1:5 needs several.
    let tiled = dir.join("tiled.pdf");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": tiled.display().to_string(), "dpi": 72,
               "scale": 5, "paper": "a4", "landscape": false}),
    );
    assert!(r["result"]["pdf"]["sheets"].as_u64().unwrap() > 1, "{r}");

    // A second run is byte for byte the same.
    let again = dir.join("again.pdf");
    call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": again.display().to_string(), "dpi": 72}),
    );
    assert_eq!(std::fs::read(&again).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_pdf_adds_the_cut_list_pages_when_asked() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1.2, 0.0254, 0.019],
               "as": "group", "name": "Shelf (long)"}),
    );
    call(
        &mut h,
        "execute",
        json!({"type": "set_units", "length": "millimeters", "precision": 1}),
    );
    call(
        &mut h,
        "execute",
        json!({"type": "set_camera", "eye": [0.6, 0.0, 50], "target": [0.6, 0.0, 0], "fov_y_deg": 30,
               "up": [0, 1, 0], "projection": "parallel"}),
    );
    let dir = scratch_dir("cut-list");
    let plain = dir.join("plain.pdf");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": plain.display().to_string(), "dpi": 72}),
    );
    if no_gpu(&r) {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(r["result"]["pdf"]["sheets"], 1, "{r}");
    let with = dir.join("with.pdf");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": with.display().to_string(), "dpi": 72,
               "cut_list": true}),
    );
    assert_eq!(r["result"]["pdf"]["sheets"], 2, "{r}");
    let text = String::from_utf8_lossy(&std::fs::read(&with).unwrap()).into_owned();
    assert!(text.contains("/Count 2"));
    assert!(
        text.contains("(Cut list) Tj") && text.contains("Shelf \\(long\\)"),
        "{text}"
    );
    assert!(text.contains("Length \\(mm\\)"), "{text}");
    assert!(text.contains("(1200 mm) Tj"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_pdf_refuses_bad_input_and_other_files_before_rendering() {
    let mut h = HeadlessHost::new();
    call(&mut h, "batch", json!({"commands": square()}));
    let dir = scratch_dir("refuse");
    let other = dir.join("notes.pdf");
    std::fs::write(&other, "not a pdf").unwrap();
    let path = other.display().to_string();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": path}),
    );
    assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overwrite")
    );
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "not a pdf");

    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": dir.join("a.pdf").display().to_string(), "paper": "b5"}),
    );
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown paper")
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": dir.join("a.tnn").display().to_string()}),
    );
    assert!(r["error"]["message"].as_str().unwrap().contains(".tnn"));
    // A perspective view cannot print at a scale (or, without a GPU, cannot print at all).
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export_pdf", "path": dir.join("a.pdf").display().to_string(), "scale": 100}),
    );
    assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("parallel") || message == "printing needs a GPU adapter",
        "{message}"
    );
    assert!(!dir.join("a.pdf").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A box group at x = 0 and its id, and a scene "Moved" that stores its position and a camera
/// seen from the front; the box then moves 3 m and the camera turns to the top.
fn scene_with_a_moving_box(h: &mut HeadlessHost) -> (Value, Value) {
    let boxed = call(
        h,
        "execute",
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1]}),
    );
    let id = boxed["result"]["placed"][0].clone();
    assert!(id.is_string(), "{boxed}");
    call(
        h,
        "execute",
        json!({"type": "set_camera", "eye": [0.5, -8, 0.5], "target": [0.5, 0, 0.5]}),
    );
    let scene = call(
        h,
        "execute",
        json!({"type": "add_scene", "name": "Moved", "positions": [id]}),
    );
    let scene = scene["result"]["changes"]["scenes"][0].clone();
    assert!(scene.is_string());
    call(
        h,
        "execute",
        json!({"type": "move", "ids": [id], "offset": [3, 0, 0]}),
    );
    call(
        h,
        "execute",
        json!({"type": "set_camera", "eye": [0, 0, 9], "target": [0, 0, 0], "up": [0, 1, 0]}),
    );
    (id, scene)
}

fn png_of(reply: &Value) -> String {
    reply["result"]["png_base64"]
        .as_str()
        .unwrap_or_else(|| panic!("{reply}"))
        .to_string()
}

fn no_render_gpu(reply: &Value) -> bool {
    if reply["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return true;
    }
    false
}

#[test]
fn render_view_of_a_scene_refuses_a_camera_and_stray_frame_params() {
    let mut h = HeadlessHost::new();
    let (_, scene) = scene_with_a_moving_box(&mut h);
    let both = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "scene": scene,
               "camera": {"eye": [0, -5, 1], "target": [0, 0, 1]}}),
    );
    assert_eq!(both["error"]["code"], codes::INVALID_PARAMS, "{both}");
    for stray in [json!({"t": 0.5}), json!({"from": scene})] {
        let mut p = json!({"width": 8, "height": 8});
        p.as_object_mut()
            .unwrap()
            .extend(stray.as_object().unwrap().clone());
        let r = call(&mut h, "render_view", p);
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    }
    for t in [json!(-0.1), json!(1.5)] {
        let r = call(
            &mut h,
            "render_view",
            json!({"width": 8, "height": 8, "scene": scene, "t": t}),
        );
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    }
    let missing = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "scene": "p99"}),
    );
    assert_eq!(missing["error"]["code"], codes::COMMAND_FAILED, "{missing}");
}

#[test]
fn render_view_of_a_scene_is_the_current_image_at_t0_and_the_scene_image_at_t1() {
    let mut h = HeadlessHost::new();
    let (_, scene) = scene_with_a_moving_box(&mut h);
    let size = json!({"width": 48, "height": 32});
    let current = call(&mut h, "render_view", size.clone());
    if no_render_gpu(&current) {
        return;
    }
    let frame = |h: &mut HeadlessHost, t: Option<f64>| {
        let mut p = size.clone();
        p["scene"] = scene.clone();
        if let Some(t) = t {
            p["t"] = json!(t);
        }
        call(h, "render_view", p)
    };
    let revision = h.doc.revision();
    let undo = h.doc.camera.clone();
    let start = frame(&mut h, Some(0.0));
    let middle = frame(&mut h, Some(0.5));
    let end = frame(&mut h, None);
    assert_eq!(
        png_of(&start),
        png_of(&current),
        "t = 0 is the current view"
    );
    assert_ne!(png_of(&middle), png_of(&current));
    assert_ne!(png_of(&middle), png_of(&end));
    assert_ne!(png_of(&end), png_of(&current));
    assert_eq!(h.doc.revision(), revision, "the document is unchanged");
    assert_eq!(h.doc.camera, undo);
    // What going to the scene makes is the image at t = 1.
    call(
        &mut h,
        "execute",
        json!({"type": "go_to_scene", "scene": scene}),
    );
    let there = call(&mut h, "render_view", size.clone());
    assert_eq!(png_of(&end), png_of(&there));
}

#[test]
fn render_view_from_a_scene_starts_at_that_scene() {
    let mut h = HeadlessHost::new();
    let (id, scene) = scene_with_a_moving_box(&mut h);
    // A second scene, stored where the box and camera now are.
    let top = call(
        &mut h,
        "execute",
        json!({"type": "add_scene", "name": "Top", "positions": [id]}),
    );
    let top = top["result"]["changes"]["scenes"][0].clone();
    let size = json!({"width": 48, "height": 32});
    let start = call(
        &mut h,
        "render_view",
        json!({"width": 48, "height": 32, "scene": top, "from": scene, "t": 0.0}),
    );
    if no_render_gpu(&start) {
        return;
    }
    let end = call(
        &mut h,
        "render_view",
        json!({"width": 48, "height": 32, "scene": top, "from": scene, "t": 1.0}),
    );
    // t = 0 from "Moved" is the picture going to "Moved" makes; t = 1 the one of "Top".
    let alone = call(
        &mut h,
        "render_view",
        json!({"width": 48, "height": 32, "scene": scene, "t": 1.0}),
    );
    assert_eq!(png_of(&start), png_of(&alone));
    let current = call(&mut h, "render_view", size);
    assert_eq!(
        png_of(&end),
        png_of(&current),
        "Top is where things are now"
    );
    assert_ne!(png_of(&start), png_of(&end));
}
