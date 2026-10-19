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
fn inside_a_moved_and_turned_group_points_are_the_groups_own() {
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
    // Moved 10 m along x, then turned a quarter about the vertical line through (10, 0):
    // the group's (x, y, z) is placed at (10 - y, x, z), so its box covers x 8..10, y 0..2.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "move", "ids": [group], "offset": [10, 0, 0]}),
    );
    assert!(r["error"].is_null(), "{r}");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "rotate", "ids": [group], "center": [10, 0, 0], "axis": [0, 0, 1], "angle": 90}),
    );
    assert!(r["error"].is_null(), "{r}");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "open_instance", "id": group}),
    );
    assert!(r["error"].is_null(), "{r}");

    // Looking straight down at the group's (1, 1), placed at (9, 1).
    let camera = json!({"eye": [9, 1, 10], "target": [9, 1, 0]});
    let r = query(
        &mut h,
        json!({"type": "pick", "pixel": [50, 50], "width": 100, "height": 100, "camera": camera}),
    );
    let top = &r["hits"][0];
    assert!(top["id"].as_str().unwrap().starts_with('f'), "{r}");
    assert_eq!(top["point"], json!([1.0, 1.0, 1.0]), "{r}");
    assert_eq!(top["normal"], json!([0.0, 0.0, 1.0]), "{r}");
    assert_eq!(top["distance"], 9.0, "{r}");
    assert_eq!(top["path"], json!([group]));

    // The group's corner (2, 2, 1), placed at (8, 2, 1), given and returned in its own terms.
    let r = query(
        &mut h,
        json!({"type": "infer", "point": [1.98, 2, 1], "width": 100, "height": 100, "camera": camera}),
    );
    assert_eq!(r["kind"], "endpoint", "{r}");
    assert_eq!(r["point"], json!([2.0, 2.0, 1.0]), "{r}");
    assert_eq!(r["entities"][0]["path"], json!([group]), "{r}");
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
        json!({"type": "infer", "pixel": [1, 1], "radius": 101}),
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
fn select_takes_a_box_on_screen_inside_or_crossing() {
    let mut h = boxed();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_camera", "eye": [1, 1, 10], "target": [1, 1, 0]}),
    );
    assert!(r["error"].is_null(), "{r}");
    let select = |h: &mut HeadlessHost, b: Value| {
        let r = call(h, "execute", json!({"type": "select", "box": b}));
        assert!(r["error"].is_null(), "{r}");
        let ids = query(h, json!({"type": "selection"}))["ids"].clone();
        ids.as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap()[..1].to_string())
            .collect::<Vec<_>>()
    };
    let count = |ids: &[String], k: &str| ids.iter().filter(|i| *i == k).count();
    // The whole view: every edge and face lies inside.
    let all = select(
        &mut h,
        json!({"from": [0, 0], "to": [100, 100], "width": 100, "height": 100}),
    );
    assert_eq!((count(&all, "e"), count(&all, "f")), (12, 6), "{all:?}");
    // A small box in the middle holds nothing whole, but crosses into the top and bottom faces.
    let small = json!({"from": [55, 55], "to": [45, 45], "width": 100, "height": 100});
    assert!(select(&mut h, small.clone()).is_empty());
    let mut crossing = small;
    crossing["crossing"] = json!(true);
    let touched = select(&mut h, crossing);
    assert_eq!(
        (count(&touched, "e"), count(&touched, "f")),
        (0, 2),
        "{touched:?}"
    );
    // `mode` combines it with the selection; a box outside the view or with ids is refused.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "select", "box": {"from": [0, 0], "to": [100, 100], "width": 100,
               "height": 100}, "mode": "remove"}),
    );
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(
        query(&mut h, json!({"type": "selection"}))["ids"],
        json!([])
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "select", "box": {"from": [0, 0], "to": [101, 100], "width": 100,
               "height": 100}}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "select", "ids": [], "box": {"from": [0, 0], "to": [1, 1]}}),
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

#[test]
fn a_turned_group_scaled_along_model_axes_is_picked_at_its_new_extent() {
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
    let r = call(
        &mut h,
        "execute",
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "angle": 30}),
    );
    assert!(r["error"].is_null(), "{r}");
    // The box's top point (1.9, 0.1, 1) turned 30 degrees and then stretched twice along X.
    let (s, c) = 30f64.to_radians().sin_cos();
    let (x, y) = (2.0 * (1.9 * c - 0.1 * s), 1.9 * s + 0.1 * c);
    let ray = json!({"origin": [x, y, 10], "direction": [0, 0, -1]});
    let r = query(&mut h, json!({"type": "pick", "ray": ray}));
    assert_eq!(r["hits"], json!([]), "outside the turned box");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    );
    assert!(r["error"].is_null(), "{r}");
    let r = query(&mut h, json!({"type": "pick", "ray": ray}));
    assert_eq!(r["hits"][0]["path"], json!([group]));
    let p = &r["hits"][0]["point"];
    assert!((p[2].as_f64().unwrap() - 1.0).abs() < 1e-9, "{p}");
}
