//! Camera commands (orbit, pan, zoom, zoom extents, standard views, projection) and the camera
//! query, driven through JSON commands the way the API sends them.

use std::io::Write;
use std::path::PathBuf;

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, ErrorKind> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| e.kind)
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn vec3(v: &Value) -> DVec3 {
    let a: [f64; 3] = serde_json::from_value(v.clone()).unwrap();
    DVec3::from(a)
}

/// The camera query's eye, target and up.
fn view(d: &Document) -> (DVec3, DVec3, DVec3) {
    let c = ask(d, json!({"type": "camera"}));
    (vec3(&c["eye"]), vec3(&c["target"]), vec3(&c["up"]))
}

fn close(a: DVec3, b: DVec3) -> bool {
    a.abs_diff_eq(b, 1e-9)
}

/// Looking along +Y at the origin from 10 m away.
fn front(d: &mut Document) {
    run(
        d,
        json!({"type": "set_camera", "eye": [0, -10, 0], "target": [0, 0, 0]}),
    )
    .unwrap();
}

/// A 2 x 3 x 1.5 m box with its corner at the origin.
fn block(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 3, 0]}),
    )
    .unwrap();
    let face = ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        d,
        json!({"type": "push_pull", "face": face, "distance": 1.5}),
    )
    .unwrap();
}

#[test]
fn orbit_turns_the_eye_about_the_target() {
    let mut d = doc();
    front(&mut d);
    let out = run(&mut d, json!({"type": "orbit", "yaw": 90})).unwrap();
    assert_eq!(out["changes"]["created"], json!([]));
    let (eye, target, up) = view(&d);
    assert!(close(eye, DVec3::new(10.0, 0.0, 0.0)), "{eye}");
    assert!(close(target, DVec3::ZERO) && close(up, DVec3::Z));
    run(&mut d, json!({"type": "orbit", "pitch": 30})).unwrap();
    let (eye, _, _) = view(&d);
    let (s, c) = 30f64.to_radians().sin_cos();
    assert!(close(eye, DVec3::new(10.0 * c, 0.0, 10.0 * s)), "{eye}");
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    assert_eq!(d.revision(), 0);
}

#[test]
fn orbit_stops_short_of_straight_down() {
    let mut d = doc();
    front(&mut d);
    run(&mut d, json!({"type": "orbit", "pitch": 200})).unwrap();
    let (eye, target, up) = view(&d);
    let polar = (target - eye).angle_between(-DVec3::Z);
    assert!((polar - 0.01).abs() < 1e-9, "{polar}");
    assert!(close(up, DVec3::Z));
    assert!(((eye - target).length() - 10.0).abs() < 1e-9);
}

#[test]
fn pan_moves_eye_and_target_along_the_screen() {
    let mut d = doc();
    front(&mut d);
    run(&mut d, json!({"type": "pan", "right": 2, "up": 1})).unwrap();
    let (eye, target, _) = view(&d);
    assert!(close(eye, DVec3::new(2.0, -10.0, 1.0)), "{eye}");
    assert!(close(target, DVec3::new(2.0, 0.0, 1.0)), "{target}");
    let before = view(&d);
    let far = run(&mut d, json!({"type": "pan", "right": 2e6}));
    assert_eq!(far, Err(ErrorKind::InvalidInput));
    assert_eq!(view(&d), before, "a refused pan leaves the camera");
}

#[test]
fn zoom_divides_the_distance_by_the_factor() {
    let mut d = doc();
    front(&mut d);
    run(&mut d, json!({"type": "zoom", "factor": 2})).unwrap();
    let (eye, target, _) = view(&d);
    assert!(close(eye, DVec3::new(0.0, -5.0, 0.0)) && close(target, DVec3::ZERO));
    run(&mut d, json!({"type": "zoom", "factor": 0.25})).unwrap();
    assert!(close(view(&d).0, DVec3::new(0.0, -20.0, 0.0)));
    for bad in [0.0, -2.0] {
        assert_eq!(
            run(&mut d, json!({"type": "zoom", "factor": bad})),
            Err(ErrorKind::InvalidInput)
        );
    }
    run(&mut d, json!({"type": "zoom", "factor": 1e12})).unwrap();
    let (eye, target, _) = view(&d);
    assert!(
        (eye.distance(target) - 0.01).abs() < 1e-12,
        "never nearer than 1 cm"
    );
}

#[test]
fn orbit_and_zoom_turn_and_zoom_about_a_given_point() {
    let mut d = doc();
    front(&mut d);
    // A quarter turn about (2, 0, 0): the eye swings from (0, -10, 0) to (12, -2, 0).
    run(
        &mut d,
        json!({"type": "orbit", "yaw": 90, "about": [2, 0, 0]}),
    )
    .unwrap();
    let (eye, target, up) = view(&d);
    assert!(close(eye, DVec3::new(12.0, -2.0, 0.0)), "{eye}");
    assert!(close(target, DVec3::new(2.0, -2.0, 0.0)), "{target}");
    assert!(close(up, DVec3::Z));

    // Zooming about a point keeps it on the same pixel and divides the eye's distance to it.
    front(&mut d);
    let p = DVec3::new(1.0, -4.0, 0.5);
    let px = d.camera.project(p).unwrap();
    let before = d.camera.eye.distance(p);
    run(
        &mut d,
        json!({"type": "zoom", "factor": 2, "about": p.to_array()}),
    )
    .unwrap();
    assert!(d.camera.project(p).unwrap().abs_diff_eq(px, 1e-6));
    assert!((d.camera.eye.distance(p) - before / 2.0).abs() < 1e-9);
    // In perspective the target comes to the point's depth.
    assert!((d.camera.distance() - d.camera.depth(p)).abs() < 1e-9);

    for bad in [
        json!({"type": "orbit", "yaw": 10, "about": [1e9, 0, 0]}),
        json!({"type": "zoom", "factor": 2, "about": [0, f64::MAX, 0]}),
    ] {
        assert_eq!(run(&mut d, bad), Err(ErrorKind::InvalidInput));
    }
    assert_eq!(d.revision(), 0, "camera commands are not edits");
}

#[test]
fn standard_views_look_along_the_axes_from_the_same_distance() {
    let mut d = doc();
    front(&mut d);
    run(&mut d, json!({"type": "pan", "right": 1, "up": 2})).unwrap();
    let t = DVec3::new(1.0, 0.0, 2.0);
    let cases = [
        ("top", DVec3::Z, DVec3::Y),
        ("bottom", -DVec3::Z, DVec3::Y),
        ("front", -DVec3::Y, DVec3::Z),
        ("back", DVec3::Y, DVec3::Z),
        ("left", -DVec3::X, DVec3::Z),
        ("right", DVec3::X, DVec3::Z),
        ("iso", DVec3::new(-1.0, -1.0, 1.0).normalize(), DVec3::Z),
    ];
    for (name, from, screen_up) in cases {
        run(&mut d, json!({"type": "standard_view", "view": name})).unwrap();
        let (eye, target, up) = view(&d);
        assert!(close(target, t), "{name}: {target}");
        assert!(close(eye, t + from * 10.0), "{name}: {eye}");
        assert!(close(up, screen_up), "{name}: {up}");
    }
    assert!(
        serde_json::from_value::<Command>(json!({"type": "standard_view", "view": "side"}))
            .is_err()
    );
}

#[test]
fn a_top_view_orbits_by_spinning_then_tilting() {
    let mut d = doc();
    run(&mut d, json!({"type": "standard_view", "view": "top"})).unwrap();
    run(&mut d, json!({"type": "orbit", "yaw": 90})).unwrap();
    let (eye, target, up) = view(&d);
    assert!(close(
        eye - target,
        DVec3::new(0.0, 0.0, eye.distance(target))
    ));
    assert!(
        close(up, -DVec3::X),
        "the screen's up turns with the yaw: {up}"
    );
    run(&mut d, json!({"type": "orbit", "pitch": -30})).unwrap();
    let (eye, target, up) = view(&d);
    assert!(close(up, DVec3::Z), "a tilted view has Z up: {up}");
    // Tilted toward what was the top of the screen (-X): the eye moved to +X.
    assert!(eye.x > target.x + 1.0 && eye.y.abs() < 1e-9, "{eye}");
}

#[test]
fn set_camera_takes_vertical_views_with_a_screen_up() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [1, 1, 5], "target": [1, 1, 0]}),
    )
    .unwrap();
    assert!(close(view(&d).2, DVec3::Y), "default screen up is +Y");
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [1, 1, 5], "target": [1, 1, 0], "up": [2, 0, 7]}),
    )
    .unwrap();
    assert!(
        close(view(&d).2, DVec3::X),
        "the horizontal part, normalized"
    );
    let before = view(&d);
    let bad = run(
        &mut d,
        json!({"type": "set_camera", "eye": [1, 1, 5], "target": [1, 1, 0], "up": [0, 0, 1]}),
    );
    assert_eq!(bad, Err(ErrorKind::InvalidInput));
    assert_eq!(view(&d), before);
    // Any other view has Z up, whatever `up` says.
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [5, 0, 5], "target": [0, 0, 0], "up": [1, 0, 0]}),
    )
    .unwrap();
    assert!(close(view(&d).2, DVec3::Z));
}

#[test]
fn a_vertical_views_up_may_be_huge_or_tiny() {
    for up in [json!([1e200, 0, 0]), json!([1e-200, 0, 0])] {
        let mut d = doc();
        run(
            &mut d,
            json!({"type": "set_camera", "eye": [1, 1, 5], "target": [1, 1, 0], "up": up}),
        )
        .unwrap();
        assert!(close(view(&d).2, DVec3::X), "{up}");
    }
}

#[test]
fn parallel_projection_keeps_the_view_height_at_the_target() {
    let mut d = doc();
    front(&mut d);
    let perspective = ask(&d, json!({"type": "camera"}));
    assert_eq!(perspective["projection"], "perspective");
    let height = 2.0 * 10.0 * 17.5f64.to_radians().tan();
    assert!((perspective["view_height"].as_f64().unwrap() - height).abs() < 1e-9);
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    let parallel = ask(&d, json!({"type": "camera"}));
    assert_eq!(parallel["projection"], "parallel");
    assert_eq!(parallel["view_height"], perspective["view_height"]);
    assert_eq!(parallel["eye"], perspective["eye"]);
    run(&mut d, json!({"type": "zoom", "factor": 2})).unwrap();
    let zoomed = ask(&d, json!({"type": "camera"}))["view_height"]
        .as_f64()
        .unwrap();
    assert!((zoomed - height / 2.0).abs() < 1e-9);
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [0, -4, 0], "target": [0, 0, 0], "projection": "perspective"}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "camera"}))["projection"],
        "perspective"
    );
}

#[test]
fn zoom_extents_fits_the_whole_model() {
    let mut d = doc();
    let empty = ask(&d, json!({"type": "camera"}));
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"})), empty, "nothing to fit");

    block(&mut d);
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    let (eye, target, _) = view(&d);
    assert!(close(target, DVec3::new(1.0, 1.5, 0.75)), "{target}");
    let c = &d.camera;
    let corners = (0..8).map(|k| {
        DVec3::new(
            if k & 1 == 0 { 0.0 } else { 2.0 },
            if k & 2 == 0 { 0.0 } else { 3.0 },
            if k & 4 == 0 { 0.0 } else { 1.5 },
        )
    });
    for p in corners {
        let s = c.project(p).unwrap();
        assert!(
            s.x > 0.0 && s.x < 800.0 && s.y > 0.0 && s.y < 600.0,
            "{p} at {s}"
        );
    }
    // Fits as the view is: from the iso direction it stays there.
    let dir = (target - eye).normalize();
    assert!(dir.abs_diff_eq(DVec3::new(8.0, 12.0, -7.0).normalize(), 1e-9));

    // A parallel top view fits the box's 3 m depth (the taller side on screen) with 5% spare.
    run(&mut d, json!({"type": "standard_view", "view": "top"})).unwrap();
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    let cam = ask(&d, json!({"type": "camera"}));
    assert!(
        (cam["view_height"].as_f64().unwrap() - 3.15).abs() < 1e-9,
        "{cam}"
    );
    assert!(close(vec3(&cam["target"]), DVec3::new(1.0, 1.5, 0.75)));
}

#[test]
fn parallel_zoom_extents_keeps_a_deep_thin_model_in_the_depth_range() {
    let mut d = doc();
    // A 0.1 x 60 x 0.1 m beam along Y.
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [0.1, 60, 0]}),
    )
    .unwrap();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 0.1}),
    )
    .unwrap();
    run(&mut d, json!({"type": "standard_view", "view": "front"})).unwrap();
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    let vertices = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}));
    let vertices = vertices["vertices"].as_array().unwrap();
    assert_eq!(vertices.len(), 8);
    for v in vertices {
        let p = vec3(&v["position"]);
        assert!(d.camera.sees_depth(p), "{p} is outside the depth range");
    }
}

#[test]
fn zoom_extents_ignores_guides() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_guide_line", "point": [5, 5, 0], "direction": [1, 0, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_guide_point", "position": [100, 0, 0]}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "camera"}));
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"})), before);
}

#[test]
fn zoom_extents_counts_groups_wherever_the_context_is() {
    let mut d = doc();
    block(&mut d);
    let ids: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        &mut d,
        json!({"type": "move", "ids": [group], "offset": [10, 0, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    assert!(
        close(view(&d).1, DVec3::new(11.0, 1.5, 0.75)),
        "root coordinates"
    );
}

#[test]
fn camera_commands_are_not_edits() {
    let mut d = doc();
    block(&mut d);
    let revision = d.revision();
    for cmd in [
        json!({"type": "orbit", "yaw": 10, "pitch": 5}),
        json!({"type": "pan", "right": 1}),
        json!({"type": "zoom", "factor": 1.5}),
        json!({"type": "zoom_extents"}),
        json!({"type": "standard_view", "view": "front"}),
        json!({"type": "set_projection", "projection": "parallel"}),
    ] {
        let parsed: Command = serde_json::from_value(cmd.clone()).unwrap();
        assert!(!parsed.changes_model(), "{cmd}");
        run(&mut d, cmd).unwrap();
    }
    assert_eq!(d.revision(), revision);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 2);
}

#[test]
fn a_failed_atomic_batch_puts_the_camera_back() {
    let mut d = doc();
    let camera = d.camera.clone();
    let cmds: Vec<Command> = [
        json!({"type": "standard_view", "view": "top"}),
        json!({"type": "set_projection", "projection": "parallel"}),
        json!({"type": "zoom", "factor": -1}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).unwrap())
    .collect();
    let err = d.execute_batch(cmds, true).unwrap_err();
    assert_eq!(err.index, 2);
    assert_eq!(d.camera, camera);
}

#[test]
fn the_view_and_projection_are_saved_with_the_model() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-camera-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.tnn");
    let mut d = doc();
    block(&mut d);
    run(&mut d, json!({"type": "standard_view", "view": "top"})).unwrap();
    run(&mut d, json!({"type": "orbit", "yaw": 30})).unwrap();
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    let camera = ask(&d, json!({"type": "camera"}));
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&e, json!({"type": "camera"})), camera);

    // A version 3 file (before projections) has neither field and opens in perspective.
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(
        document["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    assert_eq!(document["camera"]["projection"], "parallel");
    document["schema_version"] = json!(3);
    document["camera"] = json!({"eye": [-1, -2, 3], "target": [0, 0, 0], "fov_y_deg": 35});
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
    let mut old = doc();
    run(
        &mut old,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let c = ask(&old, json!({"type": "camera"}));
    assert_eq!(c["projection"], "perspective");
    assert_eq!(c["up"], json!([0.0, 0.0, 1.0]));
}
