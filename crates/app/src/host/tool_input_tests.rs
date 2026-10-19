//! `tool_input` over JSON-RPC through the app's host: every tool driven the way a user drives
//! it in the window, with typed values, locks and modifier keys.

use api::{ApiRoots, handle};
use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use super::*;

fn editor() -> Editor {
    Editor::new(DVec2::new(800.0, 600.0), 1.0)
}

/// One request through an `AppHost`; the whole reply.
fn call(e: &mut Editor, method: &str, params: Value) -> Value {
    let (mut offscreen, mut quit) = (Offscreen::headless(), false);
    let mut host = AppHost {
        editor: e,
        offscreen: &mut offscreen,
        quit: &mut quit,
        roots: &ApiRoots::default(),
        recovery: None,
    };
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    serde_json::from_str(&handle(&mut host, &line.to_string()).unwrap()).unwrap()
}

/// The result of a request that must succeed.
fn ok(e: &mut Editor, method: &str, params: Value) -> Value {
    let r = call(e, method, params);
    assert!(r["error"].is_null(), "{r}");
    r["result"].clone()
}

/// Runs `events` and returns the result.
fn input(e: &mut Editor, events: Value) -> Value {
    ok(e, "tool_input", json!({"events": events}))
}

fn exec(e: &mut Editor, command: Value) -> Value {
    ok(e, "execute", command)
}

fn ask(e: &mut Editor, query: Value) -> Value {
    ok(e, "query", query)
}

fn tool(e: &mut Editor, name: &str) {
    exec(e, json!({"type": "set_tool", "tool": name}));
}

/// The `type`s of the commands a result says were run.
fn ran(r: &Value) -> Vec<String> {
    r["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["command"]["type"].as_str().unwrap().to_string())
        .collect()
}

fn vec3(v: &Value) -> DVec3 {
    DVec3::new(
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    )
}

fn near(v: &Value, p: [f64; 3]) -> bool {
    vec3(v).distance(DVec3::from(p)) < 1e-6
}

/// The ids among `created` whose kind starts with `prefix`.
fn created(outcome: &Value, prefix: char) -> Vec<Value> {
    outcome["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with(prefix))
        .cloned()
        .collect()
}

/// A 2 x 2 m square face on the ground at the origin; returns its face id.
fn square(e: &mut Editor) -> Value {
    let r = exec(
        e,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    created(&r, 'f')[0].clone()
}

fn select_all(e: &mut Editor) {
    let all = ask(e, json!({"type": "entities"}));
    let ids: Vec<Value> = ["edges", "faces", "instances"]
        .iter()
        .flat_map(|k| all[k].as_array().cloned().unwrap_or_default())
        .map(|x| x["id"].clone())
        .collect();
    exec(e, json!({"type": "select", "ids": ids}));
}

fn count(e: &mut Editor, kind: &str) -> u64 {
    ask(e, json!({"type": "stats"}))[kind].as_u64().unwrap()
}

#[test]
fn a_fresh_editor_reports_the_select_tool() {
    let mut e = editor();
    assert_eq!(ask(&mut e, json!({"type": "editor"}))["tool"], "select");
}

#[test]
fn the_line_tool_draws_by_clicks_and_a_typed_length() {
    let mut e = editor();
    tool(&mut e, "line");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [1, 0, 0]}
        ]),
    );
    assert_eq!(r["editor"]["phase"], "active", "{r}");
    assert!(near(&r["editor"]["points"][0], [0.0, 0.0, 0.0]));
    assert_eq!(r["inference"]["kind"], "axis", "{r}");
    assert_eq!(r["inference"]["axis"], "x");
    assert_eq!(r["preview"].as_array().unwrap().len(), 1, "the rubber band");
    assert_eq!(r["commands"], json!([]));

    let r = input(&mut e, json!([{"type": "type", "text": "3"}]));
    assert_eq!(ran(&r), ["add_edge"]);
    let c = &r["commands"][0];
    assert!(near(&c["command"]["end"], [3.0, 0.0, 0.0]), "{c}");
    assert_eq!(created(&c["outcome"], 'e').len(), 1);
    assert_eq!(r["revision"], 1);
    // The chain goes on from the end; Escape ends it.
    assert!(near(&r["editor"]["points"][1], [3.0, 0.0, 0.0]), "{r}");
    let r = input(&mut e, json!([{"type": "key", "key": "escape"}]));
    assert_eq!(r["editor"]["phase"], "idle");
}

#[test]
fn rectangle_and_circle_take_typed_sizes_and_sides() {
    let mut e = editor();
    tool(&mut e, "rectangle");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [1, 1, 0]},
            {"type": "type", "text": "2,3"}
        ]),
    );
    assert_eq!(ran(&r), ["add_rectangle"]);
    assert!(
        near(&r["commands"][0]["command"]["opposite"], [2.0, 3.0, 0.0]),
        "{r}"
    );

    tool(&mut e, "circle");
    let r = input(
        &mut e,
        json!([
            {"type": "type", "text": "8s"},
            {"type": "click", "point": [6, 0, 0]},
            {"type": "move", "point": [7, 0, 0]},
            {"type": "type", "text": "1.5"}
        ]),
    );
    assert_eq!(ran(&r), ["add_circle"]);
    let c = &r["commands"][0]["command"];
    assert_eq!(
        (c["radius"].clone(), c["segments"].clone()),
        (json!(1.5), json!(8))
    );
    assert!(near(&c["center"], [6.0, 0.0, 0.0]));
}

#[test]
fn polygon_draws_from_clicks_and_typed_sides() {
    let mut e = editor();
    tool(&mut e, "polygon");
    let r = input(
        &mut e,
        json!([
            {"type": "type", "text": "5s"},
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [1, 0, 0]},
            {"type": "type", "text": "2"}
        ]),
    );
    assert_eq!(ran(&r), ["add_polygon"], "{r}");
    let c = &r["commands"][0]["command"];
    assert_eq!(
        (c["sides"].clone(), c["radius"].clone()),
        (json!(5), json!(2.0))
    );
    assert_eq!(c["circumscribed"], false);
    assert_eq!(created(&r["commands"][0]["outcome"], 'e').len(), 5);

    // The side count and a Ctrl tap's circumscribed mode outlast a switch to another tool.
    let r = input(&mut e, json!([{"type": "key", "key": "ctrl"}]));
    assert_eq!(
        r["editor"]["message"], "Polygons are drawn circumscribed",
        "{r}"
    );
    tool(&mut e, "select");
    tool(&mut e, "polygon");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [6, 0, 0]},
            {"type": "click", "point": [7, 0, 0]}
        ]),
    );
    assert_eq!(ran(&r), ["add_polygon"], "{r}");
    let c = &r["commands"][0]["command"];
    assert_eq!(c["sides"], 5, "{c}");
    assert!((c["radius"].as_f64().unwrap() - 1.0).abs() < 1e-9, "{c}");
    assert_eq!(c["circumscribed"], true);
    // A fresh session starts with six inscribed sides.
    let mut fresh = editor();
    tool(&mut fresh, "polygon");
    let r = input(
        &mut fresh,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "click", "point": [1, 0, 0]}
        ]),
    );
    let c = &r["commands"][0]["command"];
    assert_eq!(
        (c["sides"].clone(), c["circumscribed"].clone()),
        (json!(6), json!(false))
    );
}

#[test]
fn the_arc_tool_takes_two_clicks_and_a_typed_bulge() {
    let mut e = editor();
    tool(&mut e, "arc");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "click", "point": [2, 0, 0]},
            {"type": "move", "point": [1, 0.5, 0]}
        ]),
    );
    assert_eq!(r["editor"]["points"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["editor"]["measurement_label"], "Bulge");
    let r = input(&mut e, json!([{"type": "type", "text": "1"}]));
    assert_eq!(ran(&r), ["add_arc"], "{r}");
    let edges = created(&r["commands"][0]["outcome"], 'e');
    assert_eq!(edges.len(), 12);
    let curve = ask(&mut e, json!({"type": "curve", "id": edges[0]}));
    assert!(
        near(&curve["center"], [1.0, 0.0, 0.0]),
        "a half circle: {curve}"
    );
    assert!((curve["angle"].as_f64().unwrap() - 180.0).abs() < 1e-9);
}

#[test]
fn push_pull_extrudes_a_clicked_face_by_a_typed_distance() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [1, 1, 0]},
            {"type": "move", "point": [1, 1, 0.5]},
            {"type": "type", "text": "1.5"}
        ]),
    );
    assert_eq!(ran(&r), ["push_pull"], "{r}");
    assert_eq!(
        r["commands"][0]["command"]["distance"]
            .as_f64()
            .unwrap()
            .abs(),
        1.5
    );
    assert_eq!(count(&mut e, "faces"), 6);
    let top = ask(
        &mut e,
        json!({"type": "pick", "ray": {"origin": [1, 1, 10], "direction": [0, 0, -1]}}),
    );
    assert_eq!(top["hits"][0]["point"], json!([1.0, 1.0, 1.5]), "{top}");
}

#[test]
fn move_copies_after_a_ctrl_tap_and_arrays_replace_the_copy() {
    let mut e = editor();
    square(&mut e);
    select_all(&mut e);
    tool(&mut e, "move");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "key", "key": "ctrl"},
            {"type": "move", "point": [1, 0, 0]},
            {"type": "type", "text": "3"}
        ]),
    );
    assert_eq!(ran(&r), ["copy", "select"], "{r}");
    assert_eq!(
        r["commands"][0]["command"]["offset"],
        json!([3.0, 0.0, 0.0])
    );
    assert_eq!(count(&mut e, "faces"), 2);
    // "4x" makes four copies at that spacing in place of the one.
    let r = input(&mut e, json!([{"type": "type", "text": "4x"}]));
    assert_eq!(ran(&r), ["undo", "copy", "select"], "{r}");
    assert_eq!(r["commands"][1]["command"]["count"], 4);
    assert_eq!(count(&mut e, "faces"), 5);

    // Ctrl again moves instead: from a corner to a point 5 m along Y.
    select_all(&mut e);
    let r = input(
        &mut e,
        json!([
            {"type": "key", "key": "ctrl"},
            {"type": "click", "point": [0, 0, 0]},
            {"type": "click", "point": [0, 5, 0]}
        ]),
    );
    assert_eq!(ran(&r), ["move"], "{r}");
    assert!(
        near(&r["commands"][0]["command"]["offset"], [0.0, 5.0, 0.0]),
        "{r}"
    );
}

#[test]
fn rotate_turns_the_selection_by_a_typed_angle_and_copies() {
    let mut e = editor();
    square(&mut e);
    select_all(&mut e);
    tool(&mut e, "rotate");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [5, 5, 0]},
            {"type": "click", "point": [6, 5, 0]},
            {"type": "type", "text": "90"}
        ]),
    );
    assert_eq!(ran(&r), ["rotate"], "{r}");
    let c = &r["commands"][0]["command"];
    assert!(near(&c["center"], [5.0, 5.0, 0.0]), "{c}");
    assert_eq!(c["angle"].as_f64().unwrap().abs(), 90.0);
    // Ctrl copies; "3x" then makes a radial array of three.
    select_all(&mut e);
    let r = input(
        &mut e,
        json!([
            {"type": "key", "key": "ctrl"},
            {"type": "click", "point": [5, 5, 0]},
            {"type": "click", "point": [6, 5, 0]},
            {"type": "type", "text": "45"},
            {"type": "type", "text": "3x"}
        ]),
    );
    assert_eq!(
        ran(&r),
        ["rotate", "select", "undo", "rotate", "select"],
        "{r}"
    );
    assert_eq!(r["commands"][3]["command"]["copies"], 3);
    assert_eq!(count(&mut e, "faces"), 4);
}

#[test]
fn scale_a_box_by_its_corner_and_retype() {
    let mut e = editor();
    cube(&mut e);
    select_all(&mut e);
    tool(&mut e, "scale");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [2, 2, 1]},
            {"type": "move", "point": [3, 3, 1.5]},
            {"type": "type", "text": "2"}
        ]),
    );
    assert_eq!(ran(&r), ["scale"], "{r}");
    let c = &r["commands"][0]["command"];
    assert!(near(&c["origin"], [0.0, 0.0, 0.0]), "{c}");
    assert!(near(&c["factors"], [2.0, 2.0, 2.0]), "{c}");
    // Typed again right after: the scale is redone with the new factor.
    let r = input(&mut e, json!([{"type": "type", "text": "3"}]));
    assert_eq!(ran(&r), ["undo", "scale"], "{r}");
    let c = &r["commands"][1]["command"];
    assert!(near(&c["origin"], [0.0, 0.0, 0.0]), "{c}");
    assert!(near(&c["factors"], [3.0, 3.0, 3.0]), "{c}");
    let top = ask(&mut e, json!({"type": "infer", "point": [6, 6, 3]}));
    assert_eq!(top["kind"], "endpoint", "{top}");
    assert!(near(&top["point"], [6.0, 6.0, 3.0]), "{top}");

    // After another tool's command, a fresh Scale tool has nothing to change.
    tool(&mut e, "line");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [4, -1, 0]},
            {"type": "click", "point": [5, -1, 0]},
            {"type": "key", "key": "escape"}
        ]),
    );
    assert_eq!(ran(&r), ["add_edge"], "{r}");
    tool(&mut e, "scale");
    let r = input(&mut e, json!([{"type": "type", "text": "2"}]));
    assert_eq!(ran(&r), Vec::<String>::new(), "{r}");
    assert_eq!(r["editor"]["message"], "Click a grip first", "{r}");
}

#[test]
fn scale_about_center_with_ctrl() {
    let mut e = editor();
    cube(&mut e);
    select_all(&mut e);
    tool(&mut e, "scale");
    let r = input(
        &mut e,
        json!([
            {"type": "key", "key": "ctrl"},
            {"type": "click", "point": [2, 2, 1]},
            {"type": "type", "text": "2"}
        ]),
    );
    assert_eq!(ran(&r), ["scale"], "{r}");
    let c = &r["commands"][0]["command"];
    assert!(near(&c["origin"], [1.0, 1.0, 0.5]), "{c}");
    assert!(near(&c["factors"], [2.0, 2.0, 2.0]), "{c}");
}

#[test]
fn the_tape_measure_adds_guides_and_ctrl_only_measures() {
    let mut e = editor();
    exec(
        &mut e,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [4, 0, 0]}),
    );
    tool(&mut e, "tape_measure");
    // From the edge: a parallel guide line at the typed distance.
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [1, 0, 0]},
            {"type": "move", "point": [1, 1, 0]},
            {"type": "type", "text": "1.25"}
        ]),
    );
    assert_eq!(ran(&r), ["add_guide_line"], "{r}");
    assert!(
        near(&r["commands"][0]["command"]["point"], [1.0, 1.25, 0.0]),
        "{r}"
    );
    // Ctrl: measure only.
    let r = input(
        &mut e,
        json!([
            {"type": "key", "key": "ctrl"},
            {"type": "click", "point": [0, 0, 0]},
            {"type": "click", "point": [4, 0, 0]}
        ]),
    );
    assert_eq!(ran(&r), Vec::<String>::new(), "{r}");
    assert_eq!(r["editor"]["message"], "Distance: 4 m", "{r}");
}

#[test]
fn erase_and_paint_act_on_what_is_clicked_and_cmd_samples() {
    let mut e = editor();
    let face = square(&mut e);
    let m = exec(
        &mut e,
        json!({"type": "add_material", "name": "Brick", "color": [200, 80, 60]}),
    )["changes"]["materials"][0]
        .clone();
    exec(&mut e, json!({"type": "set_paint_material", "material": m}));
    tool(&mut e, "paint");
    let r = input(&mut e, json!([{"type": "click", "point": [1, 1, 0]}]));
    assert_eq!(ran(&r), ["paint"], "{r}");
    assert_eq!(r["commands"][0]["command"]["ids"], json!([face]));
    // With Cmd held a click picks up the material clicked.
    exec(
        &mut e,
        json!({"type": "set_paint_material", "material": null}),
    );
    let r = input(
        &mut e,
        json!([
            {"type": "modifiers", "cmd": true},
            {"type": "click", "point": [1, 1, 0]}
        ]),
    );
    assert_eq!(ran(&r), Vec::<String>::new());
    assert_eq!(r["editor"]["paint_material"], m, "{r}");
    assert_eq!(r["editor"]["message"], "Painting with Brick");

    tool(&mut e, "erase");
    let r = input(&mut e, json!([{"type": "click", "point": [1, 0, 0]}]));
    assert_eq!(ran(&r), ["erase"], "{r}");
    assert_eq!(count(&mut e, "faces"), 0, "the edge took its face");
}

#[test]
fn select_clicks_toggles_with_shift_boxes_and_double_click_opens() {
    let mut e = editor();
    let face = square(&mut e);
    exec(
        &mut e,
        json!({"type": "add_edge", "start": [4, 0, 0], "end": [4, 2, 0]}),
    );
    tool(&mut e, "select");
    let r = input(&mut e, json!([{"type": "click", "point": [1, 1, 0]}]));
    assert_eq!(ran(&r), ["select"]);
    assert_eq!(
        ask(&mut e, json!({"type": "selection"}))["ids"],
        json!([face])
    );
    // Shift-click toggles the edge in.
    let r = input(
        &mut e,
        json!([
            {"type": "modifiers", "shift": true},
            {"type": "click", "point": [4, 1, 0]}
        ]),
    );
    assert_eq!(r["commands"][0]["command"]["mode"], "toggle", "{r}");
    assert_eq!(
        ask(&mut e, json!({"type": "selection"}))["ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Shift stays held after the call until a modifiers event lets it go.
    assert_eq!(r["editor"]["modifiers"]["shift"], true, "{r}");
    // A box dragged right around everything selects it all.
    let r = input(
        &mut e,
        json!([
            {"type": "modifiers"},
            {"type": "drag", "pixels": [[1, 1], [400, 300], [799, 599]]}
        ]),
    );
    assert_eq!(ran(&r), ["select"], "{r}");
    assert_eq!(
        ask(&mut e, json!({"type": "selection"}))["ids"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    // Group the square; a double-click opens the group.
    exec(&mut e, json!({"type": "select", "ids": [face]}));
    exec(&mut e, json!({"type": "make_group", "ids": [face]}));
    let r = input(
        &mut e,
        json!([{"type": "double_click", "point": [1, 1, 0]}]),
    );
    assert_eq!(ran(&r), ["select", "select", "open_instance"], "{r}");
    assert_eq!(
        ask(&mut e, json!({"type": "context"}))["path"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_double_click_under_alt_is_a_view_action_and_opens_nothing() {
    let mut e = editor();
    let face = square(&mut e);
    exec(&mut e, json!({"type": "make_group", "ids": [face]}));
    tool(&mut e, "select");
    input(
        &mut e,
        json!([
            {"type": "modifiers", "alt": true},
            {"type": "double_click", "point": [1, 1, 0]}
        ]),
    );
    assert_eq!(
        ask(&mut e, json!({"type": "context"}))["path"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn shift_and_arrow_keys_lock_the_direction() {
    let mut e = editor();
    tool(&mut e, "line");
    // Shift pressed over the red axis holds the cursor to it while it clicks off the axis.
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [2, 0, 0]},
            {"type": "modifiers", "shift": true},
            {"type": "click", "point": [2, 1, 0]}
        ]),
    );
    assert_eq!(ran(&r), ["add_edge"], "{r}");
    let end = vec3(&r["commands"][0]["command"]["end"]);
    assert!(
        end.x > 1.0 && end.y == 0.0 && end.z == 0.0,
        "on the red axis: {r}"
    );
    // Shift stays held after the call, and so does its lock, until Shift is let go.
    assert_eq!(r["editor"]["lock"], "Locked: red axis", "{r}");
    assert_eq!(r["editor"]["modifiers"]["shift"], true, "{r}");
    let r = input(&mut e, json!([{"type": "modifiers"}]));
    assert_eq!(r["editor"]["lock"], Value::Null, "{r}");

    // The up arrow locks the blue axis and stays until the operation ends.
    let r = input(&mut e, json!([{"type": "key", "key": "up"}]));
    assert_eq!(r["editor"]["lock"], "Locked: blue axis", "{r}");
    let r = input(
        &mut e,
        json!([
            {"type": "move", "point": [3, 1, 0]},
            {"type": "type", "text": "1"}
        ]),
    );
    assert!(
        near(&r["commands"][0]["command"]["end"], [end.x, 0.0, 1.0]),
        "{r}"
    );
}

#[test]
fn keys_switch_tools_group_and_undo() {
    let mut e = editor();
    let face = square(&mut e);
    let r = input(&mut e, json!([{"type": "key", "key": "r"}]));
    assert_eq!(r["editor"]["tool"], "rectangle");
    exec(&mut e, json!({"type": "select", "ids": [face]}));
    // "G" (Shift+G) makes a component of the selection.
    let r = input(&mut e, json!([{"type": "key", "key": "G"}]));
    assert_eq!(ran(&r), ["make_component"], "{r}");
    assert_eq!(r["commands"][0]["command"]["name"], "Component 1");
    let r = input(
        &mut e,
        json!([
            {"type": "key", "key": "z", "cmd": true},
            {"type": "key", "key": "z", "cmd": true, "shift": true}
        ]),
    );
    assert_eq!(ran(&r), ["undo", "redo"], "{r}");
    assert_eq!(count(&mut e, "instances"), 1);
}

#[test]
fn pixels_scale_from_the_view_given_and_pixels_outside_it_are_refused() {
    let mut e = editor();
    tool(&mut e, "line");
    let a = e.doc().camera.project(DVec3::ZERO).unwrap();
    // The same spot in a view half the size.
    let r = input_in(
        &mut e,
        json!([{"type": "click", "pixel": (a / 2.0).to_array()}]),
        400,
        300,
    );
    assert!(near(&r["editor"]["points"][0], [0.0, 0.0, 0.0]), "{r}");

    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "click", "pixel": [1, 1]}], "width": 400, "height": 400}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    // A pixel outside the view is refused before any event runs.
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [
            {"type": "key", "key": "escape"},
            {"type": "move", "pixel": [900, 10]},
            {"type": "key", "key": "r"}
        ]}),
    );
    assert_eq!(
        r["error"]["message"],
        "event 1: pixel must lie in the view, 0 to 800 across and 0 to 600 down",
        "{r}"
    );
    assert!(r["error"]["data"]["event"].is_null(), "{r}");
    assert_ne!(e.phase(), tools::Phase::Idle);
    assert_eq!(e.tool_kind(), tools::ToolKind::Line);
    // A point behind the camera is not in view.
    let behind = e.doc().camera.eye * 2.0 - e.doc().camera.target;
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "move", "point": behind.to_array()}]}),
    );
    assert_eq!(
        r["error"]["message"], "event 0: the point is not in view",
        "{r}"
    );
}

fn input_in(e: &mut Editor, events: Value, width: u32, height: u32) -> Value {
    ok(
        e,
        "tool_input",
        json!({"events": events, "width": width, "height": height}),
    )
}

#[test]
fn a_middle_drag_orbits_and_draws_nothing() {
    let mut e = editor();
    tool(&mut e, "line");
    square(&mut e);
    let eye = e.doc().camera.eye;
    let r = input(
        &mut e,
        json!([{"type": "drag", "pixels": [[400, 300], [500, 300]], "button": "middle"}]),
    );
    assert_eq!(ran(&r), Vec::<String>::new());
    assert_eq!(r["editor"]["phase"], "idle");
    assert!(e.doc().camera.eye.distance(eye) > 0.1, "the view turned");
}

/// A 2 x 2 x 1 m box on the ground at the origin.
fn cube(e: &mut Editor) {
    let face = square(e);
    let normal = ask(e, json!({"type": "entity", "id": face}))["normal"][2]
        .as_f64()
        .unwrap();
    exec(
        e,
        json!({"type": "push_pull", "face": face, "distance": normal.signum()}),
    );
}

/// The face of the open context whose normal is `n`.
fn face_facing(e: &mut Editor, n: [f64; 3]) -> Value {
    ask(e, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| near(&f["normal"], n))
        .unwrap()["id"]
        .clone()
}

/// A point behind the default camera, out of view.
fn behind(e: &Editor) -> [f64; 3] {
    let c = &e.doc().camera;
    (c.eye * 2.0 - c.target).to_array()
}

#[test]
fn an_event_that_fails_partway_reports_what_already_ran() {
    let mut e = editor();
    let out = behind(&e);
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [
            {"type": "key", "key": "r"},
            {"type": "click", "point": [1, 1, 0]},
            {"type": "click", "point": [2, 2, 0]},
            {"type": "move", "point": out},
            {"type": "key", "key": "l"}
        ]}),
    );
    let data = &r["error"]["data"];
    assert_eq!(data["kind"], "invalid_input", "{r}");
    assert_eq!(data["message"], "event 3: the point is not in view");
    assert_eq!(data["event"], 3);
    let done = &data["completed"];
    assert_eq!(ran(done), ["add_rectangle"], "{r}");
    assert!(done["commands"][0]["outcome"]["changes"]["created"].is_array());
    assert_eq!(done["revision"], 1);
    assert_eq!(
        done["editor"]["tool"], "rectangle",
        "the last event never ran"
    );
    assert_eq!(e.doc().revision(), 1);
}

#[test]
fn a_drag_that_fails_partway_lets_go_of_its_button() {
    let mut e = editor();
    square(&mut e);
    let out = behind(&e);
    // A middle drag whose last place is out of sight: the orbit stops there.
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "drag", "button": "middle",
                           "points": [[1, 1, 0], [2, 1, 0], out]}]}),
    );
    assert_eq!(r["error"]["data"]["event"], 0, "{r}");
    let eye = e.doc().camera.eye;
    input(&mut e, json!([{"type": "move", "pixel": [600, 300]}]));
    assert_eq!(e.doc().camera.eye, eye, "no orbit is left going");

    // A Select box that cannot finish is dropped: the tool is idle again.
    tool(&mut e, "select");
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "drag", "points": [[0, 0, 0], [2, 2, 0], out]}]}),
    );
    assert_eq!(
        r["error"]["data"]["completed"]["editor"]["phase"], "idle",
        "{r}"
    );
    let r = input(&mut e, json!([{"type": "move", "pixel": [700, 500]}]));
    assert_eq!(r["editor"]["phase"], "idle", "{r}");
    assert!(
        r["preview"].as_array().unwrap().is_empty(),
        "no box follows: {r}"
    );
}

#[test]
fn a_point_is_found_where_it_is_even_behind_other_geometry() {
    let mut e = editor();
    cube(&mut e);
    // The box's side facing +x cannot be seen from the default view, from -x and -y.
    let hidden = [2.0, 1.0, 0.5];
    let r = input(&mut e, json!([{"type": "move", "point": hidden}]));
    assert_eq!(r["inference"]["kind"], "on_face", "{r}");
    assert!(near(&r["inference"]["point"], hidden), "{r}");
    let q = ask(&mut e, json!({"type": "infer", "point": hidden}));
    assert_eq!(q["kind"], "on_face", "{q}");
    let side = face_facing(&mut e, [1.0, 0.0, 0.0]);
    assert_eq!(q["entities"][0]["id"], side, "{q}");
    // A hidden corner snaps as an endpoint; a point in the air inside the box is free space.
    let r = input(&mut e, json!([{"type": "move", "point": [2, 2, 0]}]));
    assert_eq!(r["inference"]["kind"], "endpoint", "{r}");
    let r = input(&mut e, json!([{"type": "move", "point": [1, 1, 0.5]}]));
    assert_eq!(r["inference"]["kind"], "nothing", "{r}");
    assert!(near(&r["inference"]["point"], [1.0, 1.0, 0.5]), "{r}");
    // The tools' own picks look there too: Paint paints the hidden side.
    tool(&mut e, "paint");
    let r = input(&mut e, json!([{"type": "click", "point": hidden}]));
    assert_eq!(r["commands"][0]["command"]["ids"], json!([side]), "{r}");
    // A pixel keeps the window's front-most behaviour.
    tool(&mut e, "line");
    let px = e.doc().camera.project(DVec3::from(hidden)).unwrap();
    let r = input(&mut e, json!([{"type": "move", "pixel": px.to_array()}]));
    assert!(!near(&r["inference"]["point"], hidden), "{r}");
}

#[test]
fn modifiers_stay_held_across_calls_until_let_go() {
    let mut e = editor();
    tool(&mut e, "line");
    // Shift pressed in one call locks the red axis; the next call moves off it and types.
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [2, 0, 0]},
            {"type": "modifiers", "shift": true}
        ]),
    );
    assert_eq!(r["editor"]["lock"], "Locked: red axis", "{r}");
    assert_eq!(
        r["editor"]["modifiers"],
        json!({"shift": true, "alt": false, "cmd": false})
    );
    let r = input(
        &mut e,
        json!([
            {"type": "move", "point": [2, 1, 0]},
            {"type": "type", "text": "3"}
        ]),
    );
    assert!(
        near(&r["commands"][0]["command"]["end"], [3.0, 0.0, 0.0]),
        "{r}"
    );
    assert_eq!(
        ask(&mut e, json!({"type": "editor"}))["modifiers"]["shift"],
        true
    );
    // Cancel lets go of them.
    exec(&mut e, json!({"type": "cancel"}));
    assert_eq!(e.modifiers(), api::Mods::default());

    // Held Cmd: a key with no binding and typing are refused.
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "modifiers", "cmd": true}, {"type": "key", "key": "s"}]}),
    );
    assert_eq!(
        r["error"]["data"]["message"],
        "event 1: with cmd or ctrl held only z (undo, with shift redo), a (select all, with \
         shift none) and d (duplicate) are taken",
        "{r}"
    );
    assert_eq!(
        r["error"]["data"]["completed"]["editor"]["modifiers"]["cmd"],
        true
    );
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "type", "text": "3"}]}),
    );
    assert_eq!(r["error"]["data"]["event"], 0, "{r}");
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [
            {"type": "modifiers", "alt": true},
            {"type": "key", "key": "l"}
        ]}),
    );
    assert_eq!(
        r["error"]["data"]["message"], "event 1: with alt held no key is taken",
        "{r}"
    );
    input(&mut e, json!([{"type": "modifiers"}]));
    assert_eq!(e.modifiers(), api::Mods::default());

    // In the window, letting go never releases what the user holds.
    e.set_modifiers(true, false);
    input(
        &mut e,
        json!([{"type": "modifiers", "cmd": true}, {"type": "modifiers"}]),
    );
    assert!(e.modifiers().shift);
    e.set_modifiers(false, false);
    assert!(!e.modifiers().shift);
}

#[test]
fn a_rectangle_from_a_walls_foot_stands_in_the_wall() {
    let mut e = editor();
    cube(&mut e);
    tool(&mut e, "rectangle");
    // From the bottom edge of the front side (y = 0), across that side.
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0.5, 0, 0]},
            {"type": "move", "point": [1.5, 0, 0.8]},
            {"type": "type", "text": "1,0.5"}
        ]),
    );
    let c = &r["commands"][0]["command"];
    assert_eq!(c["type"], "add_rectangle", "{r}");
    assert!(
        near(&c["normal"], [0.0, -1.0, 0.0]) || near(&c["normal"], [0.0, 1.0, 0.0]),
        "{r}"
    );
    assert!(vec3(&c["opposite"]).y.abs() < 1e-9, "{r}");
}

#[test]
fn the_hover_waits_while_a_middle_drag_turns_the_view() {
    let mut e = editor();
    tool(&mut e, "line");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [1, 0, 0]}
        ]),
    );
    let before = r["preview"].clone();
    // The middle button held across calls: the preview stays where it was in the model.
    let r = input(
        &mut e,
        json!([
            {"type": "down", "pixel": [400, 300], "button": "middle"},
            {"type": "move", "pixel": [500, 350]},
            {"type": "move", "pixel": [550, 400]}
        ]),
    );
    assert_eq!(r["preview"], before, "{r}");
    // Released, the hover is worked out again.
    let r = input(
        &mut e,
        json!([{"type": "up", "pixel": [550, 400], "button": "middle"}]),
    );
    assert_ne!(r["preview"], before, "{r}");
}

#[test]
fn losing_focus_keeps_a_shift_lock_tool_input_holds() {
    let mut e = editor();
    tool(&mut e, "line");
    input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "move", "point": [2, 0, 0]},
            {"type": "modifiers", "shift": true},
            {"type": "move", "point": [2, 1, 0]}
        ]),
    );
    e.focus_lost();
    let r = input(&mut e, json!([{"type": "type", "text": "3"}]));
    assert_eq!(r["editor"]["modifiers"]["shift"], true, "{r}");
    assert!(
        near(&r["commands"][0]["command"]["end"], [3.0, 0.0, 0.0]),
        "{r}"
    );
}

#[test]
fn the_back_of_a_thin_wall_is_found_behind_its_front() {
    let mut e = editor();
    // A wall 0.1 m thick; its back (y = 0.1) is hidden behind its front from the default view.
    let r = exec(
        &mut e,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 0.1, 0]}),
    );
    let floor = created(&r, 'f')[0].clone();
    let up = ask(&mut e, json!({"type": "entity", "id": floor}))["normal"][2]
        .as_f64()
        .unwrap();
    exec(
        &mut e,
        json!({"type": "push_pull", "face": floor, "distance": 2.0 * up.signum()}),
    );
    let back = face_facing(&mut e, [0.0, 1.0, 0.0]);
    let q = ask(&mut e, json!({"type": "infer", "point": [2, 0.1, 1]}));
    assert_eq!(q["entities"][0]["id"], back, "{q}");
    tool(&mut e, "paint");
    let r = input(&mut e, json!([{"type": "click", "point": [2, 0.1, 1]}]));
    assert_eq!(r["commands"][0]["command"]["ids"], json!([back]), "{r}");
}

#[test]
fn a_failed_drag_leaves_a_press_made_before_it() {
    let mut e = editor();
    square(&mut e);
    let out = behind(&e);
    tool(&mut e, "select");
    input(
        &mut e,
        json!([
            {"type": "down", "pixel": [100, 100]},
            {"type": "move", "pixel": [300, 300]}
        ]),
    );
    let r = call(
        &mut e,
        "tool_input",
        json!({"events": [{"type": "drag", "button": "middle", "points": [[1, 1, 0], out]}]}),
    );
    assert_eq!(
        r["error"]["data"]["completed"]["editor"]["phase"], "active",
        "{r}"
    );
}

#[test]
fn results_report_the_hovered_target_and_the_snap_label() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    let r = input(&mut e, json!([{"type": "move", "point": [1, 1, 0]}]));
    assert!(
        r["editor"]["hovered"].as_str().unwrap().starts_with('f'),
        "{r}"
    );
    tool(&mut e, "line");
    let r = input(&mut e, json!([{"type": "move", "point": [0, 0, 0]}]));
    assert_eq!(r["editor"]["snap_label"], "Endpoint", "{r}");
    assert_eq!(r["editor"]["hovered"], Value::Null);
    tool(&mut e, "select");
    let r = input(&mut e, json!([{"type": "move", "point": [2, 1, 0]}]));
    assert_eq!(
        r["editor"]["snap_label"],
        Value::Null,
        "Select names no snap"
    );
    assert!(
        r["inference"].is_object(),
        "Select still reports the inference: {r}"
    );
    assert!(
        r["editor"]["hovered"].as_str().unwrap().starts_with('e'),
        "{r}"
    );
    assert_eq!(
        ask(&mut e, json!({"type": "editor"}))["hovered"],
        r["editor"]["hovered"]
    );
}

#[test]
fn a_window_over_the_view_hides_the_hover_but_not_from_tool_input() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    input(&mut e, json!([{"type": "move", "point": [1, 1, 0]}]));
    e.refresh_hover();
    e.set_covered(true);
    assert_eq!(
        ask(&mut e, json!({"type": "editor"}))["hovered"],
        Value::Null,
        "the window shows no highlight under a window"
    );
    let r = input(&mut e, json!([{"type": "move", "point": [1, 1, 0]}]));
    assert!(
        r["editor"]["hovered"].as_str().unwrap().starts_with('f'),
        "an agent's pointer is never under a window: {r}"
    );
    assert_eq!(
        ask(&mut e, json!({"type": "editor"}))["hovered"],
        r["editor"]["hovered"]
    );
}

#[test]
fn a_snap_inside_a_group_says_so() {
    let mut e = editor();
    let face = square(&mut e);
    exec(&mut e, json!({"type": "make_group", "ids": [face]}));
    tool(&mut e, "line");
    let r = input(&mut e, json!([{"type": "move", "point": [1, 0, 0]}]));
    assert_eq!(r["editor"]["snap_label"], "Midpoint in Group", "{r}");
}

#[test]
fn results_say_whether_the_message_is_an_error() {
    let mut e = editor();
    tool(&mut e, "rectangle");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [0, 0, 0]},
            {"type": "type", "text": "2,x"}
        ]),
    );
    assert_eq!(r["editor"]["message"], "Not width,height: 2,x", "{r}");
    assert_eq!(r["editor"]["message_kind"], "error");
    tool(&mut e, "circle");
    let r = input(&mut e, json!([{"type": "type", "text": "8s"}]));
    assert_eq!(r["editor"]["message_kind"], "info", "{r}");
    tool(&mut e, "line");
    assert_eq!(
        ask(&mut e, json!({"type": "editor"}))["message_kind"],
        Value::Null
    );
}

#[test]
fn a_drag_draws_like_two_clicks() {
    let mut e = editor();
    tool(&mut e, "rectangle");
    let r = input(
        &mut e,
        json!([{"type": "drag", "points": [[0, 0, 0], [2, 3, 0]]}]),
    );
    assert_eq!(ran(&r), ["add_rectangle"], "{r}");
    let c = &r["commands"][0]["command"];
    assert!(near(&c["corner"], [0.0, 0.0, 0.0]), "{c}");
    assert!(near(&c["opposite"], [2.0, 3.0, 0.0]), "{c}");
    assert_eq!(r["editor"]["phase"], "idle", "{r}");

    tool(&mut e, "push_pull");
    let r = input(
        &mut e,
        json!([{"type": "drag", "points": [[1, 1, 0], [1, 1, 1]]}]),
    );
    assert_eq!(ran(&r), ["push_pull"], "{r}");
    let distance = r["commands"][0]["command"]["distance"].as_f64().unwrap();
    assert!((distance - 1.0).abs() < 1e-6, "{r}");
    assert_eq!(r["editor"]["phase"], "idle", "{r}");
}

#[test]
fn navigation_drags_draw_nothing() {
    let mut e = editor();
    tool(&mut e, "rectangle");
    let middle = json!({"type": "drag", "pixels": [[300, 300], [500, 400]], "button": "middle"});
    let left = json!({"type": "drag", "pixels": [[300, 300], [500, 400]]});
    for events in [
        json!([middle]),
        json!([{"type": "modifiers", "shift": true}, middle]),
        json!([{"type": "modifiers", "alt": true}, left]),
    ] {
        let r = input(&mut e, events.clone());
        assert_eq!(ran(&r), Vec::<String>::new(), "{events}: {r}");
        assert_eq!(r["editor"]["phase"], "idle", "{events}: {r}");
        input(&mut e, json!([{"type": "modifiers"}]));
    }
    assert_eq!(e.modifiers(), api::Mods::default());
}

#[test]
fn a_release_whose_press_was_not_the_tools_draws_nothing() {
    let mut e = editor();
    tool(&mut e, "rectangle");
    input(&mut e, json!([{"type": "down", "pixel": [300, 300]}]));
    exec(&mut e, json!({"type": "cancel"}));
    let r = input(&mut e, json!([{"type": "up", "pixel": [600, 500]}]));
    assert_eq!(ran(&r), Vec::<String>::new(), "{r}");
    assert_eq!(r["editor"]["phase"], "idle", "{r}");
    // A fresh press and a release 1 px away start a rectangle and wait for its corner.
    let r = input(
        &mut e,
        json!([
            {"type": "down", "pixel": [300, 300]},
            {"type": "up", "pixel": [301, 300]}
        ]),
    );
    assert_eq!(ran(&r), Vec::<String>::new(), "{r}");
    assert_eq!(r["editor"]["phase"], "active", "{r}");
}

#[test]
fn a_typed_distance_after_a_pull_redoes_it_in_one_step() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [1, 1, 0]},
            {"type": "click", "point": [1, 1, 1]},
            {"type": "type", "text": "2"}
        ]),
    );
    assert_eq!(ran(&r), ["push_pull", "undo", "push_pull"], "{r}");
    let history = ask(&mut e, json!({"type": "history"}));
    let pulls = history["undo"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| *l == "push_pull")
        .count();
    assert_eq!(pulls, 1, "{history}");
    let all = ask(&mut e, json!({"type": "entities", "kinds": ["vertex"]}));
    assert!(
        all["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| (v["position"][2].as_f64().unwrap() - 2.0).abs() < 1e-6),
        "{all}"
    );
}

#[test]
fn a_double_click_repeats_the_last_pull() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    input(
        &mut e,
        json!([
            {"type": "click", "point": [1, 1, 0]},
            {"type": "type", "text": "1"}
        ]),
    );
    let r = exec(
        &mut e,
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [7, 2, 0]}),
    );
    let second = created(&r, 'f')[0].clone();
    let r = input(
        &mut e,
        json!([{"type": "double_click", "point": [6, 1, 0]}]),
    );
    assert_eq!(ran(&r), ["push_pull"], "{r}");
    let c = &r["commands"][0]["command"];
    assert_eq!(c["face"], second, "{r}");
    assert!((c["distance"].as_f64().unwrap() - 1.0).abs() < 1e-9, "{r}");
}

#[test]
fn separate_clicks_are_never_a_double_click_for_push_pull() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    let px = e.doc().camera.project(DVec3::new(1.0, 1.0, 0.0)).unwrap();
    let near = px - DVec2::new(0.0, 5.0);
    // Two `click` events 5 px apart: the second finishes the pull, however soon it is sent.
    let r = input(
        &mut e,
        json!([
            {"type": "click", "pixel": px.to_array()},
            {"type": "click", "pixel": near.to_array()}
        ]),
    );
    assert_eq!(ran(&r), ["push_pull"], "{r}");
    assert_eq!(r["editor"]["phase"], "idle", "{r}");
    // A `click`, then a `double_click`: the double-click's first press finishes the pull, its
    // second (quick) press starts one on the cap and the double-click repeats on it.
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    let r = input(
        &mut e,
        json!([
            {"type": "click", "pixel": px.to_array()},
            {"type": "double_click", "pixel": near.to_array()}
        ]),
    );
    assert_eq!(ran(&r), ["push_pull", "push_pull"], "{r}");
}

#[test]
fn a_retype_after_an_agent_edit_undoes_nothing() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    input(
        &mut e,
        json!([
            {"type": "click", "point": [1, 1, 0]},
            {"type": "type", "text": "1"}
        ]),
    );
    exec(
        &mut e,
        json!({"type": "add_edge", "start": [5, 5, 0], "end": [6, 5, 0]}),
    );
    let edges = count(&mut e, "edges");
    let r = input(&mut e, json!([{"type": "type", "text": "2"}]));
    assert_eq!(ran(&r), Vec::<String>::new(), "{r}");
    assert_eq!(count(&mut e, "edges"), edges, "the edge is still there");
    assert_eq!(r["editor"]["message"], "Click a face first", "{r}");
}

#[test]
fn double_and_triple_clicks_select_bounds_and_connected() {
    let mut e = editor();
    let face = square(&mut e);
    exec(
        &mut e,
        json!({"type": "push_pull", "face": face, "distance": 1}),
    );
    tool(&mut e, "select");
    let selected = |e: &mut Editor| {
        ask(e, json!({"type": "selection"}))["ids"]
            .as_array()
            .unwrap()
            .len()
    };
    input(
        &mut e,
        json!([{"type": "double_click", "point": [1, 1, 1]}]),
    );
    assert_eq!(selected(&mut e), 5, "the top face and its edges");
    input(
        &mut e,
        json!([{"type": "triple_click", "point": [1, 1, 1]}]),
    );
    assert_eq!(selected(&mut e), 18, "the whole box");
    // A double-click on a group still opens it.
    select_all(&mut e);
    let all = ask(&mut e, json!({"type": "selection"}))["ids"].clone();
    exec(&mut e, json!({"type": "make_group", "ids": all}));
    input(
        &mut e,
        json!([{"type": "double_click", "point": [1, 1, 1]}]),
    );
    assert_eq!(
        ask(&mut e, json!({"type": "context"}))["path"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn arrow_keys_lock_rotates_plane_before_the_first_click() {
    let mut e = editor();
    cube(&mut e);
    tool(&mut e, "rotate");
    let key = |e: &mut Editor, k: &str| input(e, json!([{"type": "key", "key": k}]));
    assert_eq!(key(&mut e, "right")["editor"]["lock"], "Locked: red axis");
    assert_eq!(key(&mut e, "right")["editor"]["lock"], Value::Null);
    key(&mut e, "up");
    assert_eq!(key(&mut e, "down")["editor"]["lock"], Value::Null);
    // Locked to blue, the center on a side face (normal X) still turns about Z.
    assert_eq!(key(&mut e, "up")["editor"]["lock"], "Locked: blue axis");
    let side = face_facing(&mut e, [1.0, 0.0, 0.0]);
    exec(&mut e, json!({"type": "select", "ids": [side]}));
    let r = input(
        &mut e,
        json!([
            {"type": "click", "point": [2, 1, 0.5]},
            {"type": "type", "text": "90"}
        ]),
    );
    assert_eq!(ran(&r), ["rotate"], "{r}");
    assert_eq!(
        r["commands"][0]["command"]["axis"],
        json!([0.0, 0.0, 1.0]),
        "{r}"
    );
}

#[test]
fn dead_end_clicks_explain_without_flashing() {
    let mut e = editor();
    square(&mut e);
    tool(&mut e, "push_pull");
    let r = input(&mut e, json!([{"type": "click", "point": [-3, 4, 0]}]));
    assert_eq!(
        r["editor"]["message"], "Push/Pull works on faces: click a face to push or pull it",
        "{r}"
    );
    assert_eq!(r["editor"]["message_kind"], "info", "{r}");
    // A refusal is still an error.
    let r = input(&mut e, json!([{"type": "type", "text": "abc"}]));
    assert_eq!(r["editor"]["message_kind"], "error", "{r}");
}

#[test]
fn cmd_keys_select_all_none_and_duplicate() {
    let mut e = editor();
    square(&mut e);
    let selected = |e: &mut Editor| {
        ask(e, json!({"type": "selection"}))["ids"]
            .as_array()
            .unwrap()
            .len()
    };
    let cmd = |e: &mut Editor, key: &str, shift: bool| {
        call(
            e,
            "tool_input",
            json!({"events": [{"type": "key", "key": key, "cmd": true, "shift": shift}]}),
        )
    };
    // Four edges and a face.
    assert!(cmd(&mut e, "a", false)["error"].is_null());
    assert_eq!(selected(&mut e), 5);
    assert!(cmd(&mut e, "a", true)["error"].is_null());
    assert_eq!(selected(&mut e), 0);
    // Cmd+D with nothing selected: refused in the status, as in the window.
    let r = cmd(&mut e, "d", false);
    assert_eq!(r["result"]["editor"]["tool"], "select", "{r}");
    cmd(&mut e, "a", false);
    input(&mut e, json!([{"type": "move", "point": [2, 2, 0]}]));
    let r = cmd(&mut e, "d", false);
    assert_eq!(r["result"]["editor"]["tool"], "move", "{r}");
    assert_eq!(r["result"]["editor"]["phase"], "active", "{r}");
    let prompt = r["result"]["editor"]["prompt"].as_str().unwrap();
    assert!(prompt.starts_with("Copy:"), "{prompt}");
    // Cmd+Shift+D is no chord.
    let r = cmd(&mut e, "d", true);
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    // The editor command, with nothing selected, is refused.
    exec(&mut e, json!({"type": "cancel"}));
    exec(&mut e, json!({"type": "select", "ids": []}));
    let r = call(&mut e, "execute", json!({"type": "duplicate"}));
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
}

#[test]
fn a_turn_shows_only_its_direction_lock() {
    let mut e = editor();
    cube(&mut e);
    select_all(&mut e);
    tool(&mut e, "rotate");
    let key = |e: &mut Editor, k: &str| input(e, json!([{"type": "key", "key": k}]));
    assert_eq!(key(&mut e, "up")["editor"]["lock"], "Locked: blue axis");
    let r = input(&mut e, json!([{"type": "click", "point": [2, 1, 0.5]}]));
    assert_eq!(r["editor"]["phase"], "active", "{r}");
    assert_eq!(
        r["editor"]["lock"],
        Value::Null,
        "the plane lock is not a direction lock"
    );
    assert_eq!(key(&mut e, "right")["editor"]["lock"], "Locked: red axis");
    // The turn done, the plane lock is still there for the next one.
    let r = input(&mut e, json!([{"type": "type", "text": "90"}]));
    assert_eq!(ran(&r), ["rotate"], "{r}");
    assert_eq!(r["editor"]["phase"], "idle", "{r}");
    assert_eq!(r["editor"]["lock"], "Locked: blue axis", "{r}");
}
