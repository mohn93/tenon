//! `tool_input` over JSON-RPC through the app's host: every tool driven the way a user drives
//! it in the window, with typed values, locks and modifier keys.

use api::handle;
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

fn count(e: &mut Editor, kind: &str) -> u64 {
    ask(e, json!({"type": "stats"}))[kind].as_u64().unwrap()
}

#[test]
fn the_line_tool_draws_by_clicks_and_a_typed_length() {
    let mut e = editor();
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
fn pixels_scale_from_the_view_given_and_bad_events_stop_the_rest() {
    let mut e = editor();
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
    // The first event runs; the second is outside the view and stops the third.
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
    assert_eq!(e.phase(), tools::Phase::Idle);
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
