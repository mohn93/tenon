//! The `pick` and `infer` queries over JSON-RPC.

use api::{HeadlessHost, codes, handle};
use serde_json::{Value, json};

fn call(host: &mut HeadlessHost, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

fn query(host: &mut HeadlessHost, params: Value) -> Value {
    let reply = call(host, "query", params);
    assert!(reply["error"].is_null(), "{reply}");
    reply["result"].clone()
}

/// A 2 x 2 x 1 m box with its bottom corner at the origin.
fn boxed() -> HeadlessHost {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    let face = r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": face, "distance": 1}),
    );
    h
}

/// Looking straight down at the box's middle from 10 m up, 100 x 100 pixels.
fn above() -> Value {
    json!({"eye": [1, 1, 10], "target": [1, 1, 0]})
}

#[test]
fn a_pixel_picks_every_face_front_to_back() {
    let mut h = boxed();
    let r = query(
        &mut h,
        json!({"type": "pick", "pixel": [50, 50], "width": 100, "height": 100, "camera": above()}),
    );
    assert_eq!(r["type"], "pick");
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2, "{r}");
    assert_eq!(hits[0]["distance"], 9.0);
    assert_eq!(hits[0]["point"], json!([1.0, 1.0, 1.0]));
    assert_eq!(hits[0]["normal"], json!([0.0, 0.0, 1.0]));
    assert_eq!(hits[0]["side"], "front");
    assert_eq!(hits[0]["path"], json!([]));
    // The bottom faces down: the ray meets its back.
    assert_eq!(hits[1]["distance"], 10.0);
    assert_eq!(hits[1]["side"], "back");
    // The document camera did not move.
    assert_ne!(
        query(&mut h, json!({"type": "camera"}))["eye"],
        json!([1.0, 1.0, 10.0])
    );
}

#[test]
fn a_ray_picks_faces_and_with_a_radius_edges() {
    let mut h = boxed();
    let ray = json!({"origin": [1.99, 1, 10], "direction": [0, 0, -3]});
    let faces = query(&mut h, json!({"type": "pick", "ray": ray}));
    let ids = |r: &Value| -> Vec<String> {
        r["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap()[..1].to_string())
            .collect()
    };
    assert_eq!(ids(&faces), vec!["f", "f"]);
    let near = query(&mut h, json!({"type": "pick", "ray": ray, "radius": 0.02}));
    let mut kinds = ids(&near);
    kinds.sort();
    // The top and bottom edges along x = 2, and the side face x = 2 the ray runs beside
    // is not crossed.
    assert_eq!(kinds, vec!["e", "e", "f", "f"], "{near}");
}

#[test]
fn hits_inside_a_group_carry_the_path_from_the_root() {
    let mut h = boxed();
    let all = query(
        &mut h,
        json!({"type": "entities", "kinds": ["edge", "face"]}),
    );
    let ids: Vec<Value> = ["edges", "faces"]
        .iter()
        .flat_map(|k| all[k].as_array().unwrap().clone())
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
    let ray = json!({"origin": [1, 1, 10], "direction": [0, 0, -1]});
    let r = query(&mut h, json!({"type": "pick", "ray": ray}));
    assert_eq!(r["hits"][0]["path"], json!([group]));
    // Inside the group, the same face is the open context's, with the same path.
    call(
        &mut h,
        "execute",
        json!({"type": "open_instance", "id": group}),
    );
    let inside = query(&mut h, json!({"type": "pick", "ray": ray}));
    assert_eq!(inside["hits"][0]["id"], r["hits"][0]["id"]);
    assert_eq!(inside["hits"][0]["path"], json!([group]));
}

#[test]
fn infer_snaps_like_a_tool_click() {
    let mut h = boxed();
    let r = query(
        &mut h,
        json!({"type": "infer", "point": [1.98, 2, 1], "width": 100, "height": 100, "camera": above()}),
    );
    assert_eq!(r["type"], "infer");
    assert_eq!(r["kind"], "endpoint");
    assert_eq!(r["point"], json!([2.0, 2.0, 1.0]));
    let entities = r["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 1);
    assert!(entities[0]["id"].as_str().unwrap().starts_with('v'));
    assert_eq!(entities[0]["path"], json!([]));

    let r = query(&mut h, json!({"type": "infer", "point": [1.3, 1.4, 1]}));
    assert_eq!(r["kind"], "on_face");
    assert!(r["entities"][0]["id"].as_str().unwrap().starts_with('f'));

    // From a point, as a tool's second click: along an axis.
    let r = query(
        &mut h,
        json!({"type": "infer", "point": [5, 3, 0], "from": [5, 0, 0]}),
    );
    assert_eq!(
        (r["kind"].clone(), r["axis"].clone()),
        (json!("axis"), json!("y"))
    );
    assert_eq!(r["entities"], json!([]));
}

#[test]
fn pick_and_infer_reject_what_they_cannot_use() {
    let mut h = boxed();
    let bad = [
        json!({"type": "pick"}),
        json!({"type": "pick", "pixel": [1, 1], "ray": {"origin": [0, 0, 1], "direction": [0, 0, -1]}}),
        json!({"type": "pick", "pixel": [5000, 1]}),
        json!({"type": "pick", "pixel": [1, 1], "width": 10}),
        json!({"type": "pick", "ray": {"origin": [0, 0, 1], "direction": [0, 0, 0]}}),
        json!({"type": "pick", "ray": {"origin": [0, 0, 1], "direction": [0, 0, -1]}, "width": 10, "height": 10}),
        json!({"type": "pick", "pixel": [1, 1], "radius": -1}),
        json!({"type": "infer", "point": [0, 0, 0], "pixel": [1, 1]}),
        // Behind the default camera.
        json!({"type": "infer", "point": [-100, -150, 80]}),
    ];
    for params in bad {
        let r = call(&mut h, "query", params.clone());
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{params}: {r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{params}");
    }
    let r = call(
        &mut h,
        "query",
        json!({"type": "pick", "pixel": [1, 1], "radius_px": 2}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
}

#[test]
fn describe_gives_the_view_queries_schemas() {
    let d = api::describe();
    for name in ["EditorQuery", "PickHit", "Inferred"] {
        assert!(d["schemas"][name].is_object(), "{name}");
    }
    let text = d["schemas"]["EditorQuery"].to_string();
    assert!(
        text.contains("\"pick\"") && text.contains("\"infer\""),
        "{text}"
    );
}
