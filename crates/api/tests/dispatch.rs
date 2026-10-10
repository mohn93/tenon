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
    for variant in ["add_edge", "undo", "redo", "set_camera"] {
        assert!(
            command_schema.contains(variant),
            "Command schema lacks {variant}"
        );
    }
    assert!(d["schemas"]["Query"].to_string().contains("events"));
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
