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
    assert_eq!(d["schema_version"], 1);
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
    let command_schema = d["schemas"]["Command"].to_string();
    for variant in [
        "add_edge",
        "add_rectangle",
        "add_circle",
        "push_pull",
        "erase",
        "move",
        "copy",
        "rotate",
        "select",
        "undo",
        "redo",
        "set_camera",
        "make_group",
        "make_component",
        "explode",
        "open_instance",
        "close_instance",
        "rename_definition",
        "purge_definitions",
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
    for variant in ["events", "definitions", "context", "file"] {
        assert!(
            query_schema.contains(variant),
            "Query schema lacks {variant}"
        );
    }
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
    let vertical = call(
        &mut h,
        "render_view",
        json!({"width": 8, "height": 8, "camera": {"eye": [0, 0, 5], "target": [0, 0, 0]}}),
    );
    assert_eq!(vertical["error"]["code"], codes::COMMAND_FAILED);
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
