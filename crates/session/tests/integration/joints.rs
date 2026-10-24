//! Joints, driven through JSON commands and queries, and kept in files.

use std::io::Write;
use std::path::PathBuf;

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ok(d: &mut Document, v: Value) -> Value {
    run(d, v).unwrap_or_else(|e| panic!("{e:?}"))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-joints-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn edit_file(path: &PathBuf, f: impl FnOnce(&mut Value)) {
    let mut document = stored(path);
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

/// Two box groups, the second at x = 5; their instance ids.
fn two_parts(d: &mut Document) -> (Value, Value) {
    let mut place = |x: f64| {
        ok(
            d,
            json!({"type": "add_box", "corner": [x, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
        )["placed"][0]
            .clone()
    };
    let a = place(0.0);
    let b = place(5.0);
    (a, b)
}

fn joint_id(out: &Value) -> Value {
    out["changes"]["joints"][0].clone()
}

fn hinge(a: &Value, b: &Value) -> Value {
    json!({"type": "add_joint", "kind": "revolute", "a": a, "b": b, "point": [2.0, 0.0, 0.0], "axis": [0, 0, 1]})
}

fn close(a: &Value, b: [f64; 3]) {
    let a: Vec<f64> = a
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-9, "{a:?} vs {b:?}");
    }
}

#[test]
fn every_kind_is_added_and_listed_with_its_world_frame() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    for kind in ["revolute", "prismatic", "cylindrical", "fixed", "ball"] {
        let out = ok(
            &mut d,
            json!({"type": "add_joint", "kind": kind, "a": a, "b": b, "point": [2.0, 1.0, 0.0], "axis": [0, 0, 3]}),
        );
        let id = joint_id(&out);
        assert!(id.as_str().unwrap().starts_with('j'));
        let info = ask(&d, json!({"type": "joint", "id": id}));
        assert_eq!(info["kind"], kind);
        assert_eq!(info["a"], a);
        assert_eq!(info["b"], b);
        assert_eq!(info["value"], 0.0);
        close(&info["point"], [2.0, 1.0, 0.0]);
        close(&info["axis"], [0.0, 0.0, 1.0]);
        assert_eq!(info["slide"].is_null(), kind != "cylindrical");
    }
    let all = ask(&d, json!({"type": "joints"}));
    assert_eq!(all["joints"].as_array().unwrap().len(), 5);
    let names: Vec<&str> = all["joints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Joint 1", "Joint 2", "Joint 3", "Joint 4", "Joint 5"]
    );
}

#[test]
fn a_joint_to_the_ground_has_no_a() {
    let mut d = doc();
    let (a, _) = two_parts(&mut d);
    for ground in [
        json!({"type": "add_joint", "kind": "revolute", "b": a, "point": [0, 0, 0], "axis": [0, 1, 0]}),
        json!({"type": "add_joint", "kind": "fixed", "a": null, "b": a, "point": [0, 0, 0], "axis": [0, 1, 0]}),
    ] {
        let id = joint_id(&ok(&mut d, ground));
        let info = ask(&d, json!({"type": "joint", "id": id}));
        assert!(info["a"].is_null());
        close(&info["point"], [0.0, 0.0, 0.0]);
        close(&info["axis"], [0.0, 1.0, 0.0]);
    }
}

#[test]
fn frames_are_held_in_each_parts_own_coordinates() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    // Turn b a quarter about z and lift it; the joint is made at the world point (2, 1, 0).
    ok(
        &mut d,
        json!({"type": "set_transform", "id": b,
               "transform": {"origin": [5, 0, 2], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}}),
    );
    let id = joint_id(&ok(
        &mut d,
        json!({"type": "add_joint", "kind": "revolute", "a": a, "b": b, "point": [2.0, 1.0, 0.0], "axis": [0, 0, 1]}),
    ));
    let id: kernel::JointId = serde_json::from_value(id).unwrap();
    let j = d.scene().joint(id).unwrap().clone();
    // Part a is at the origin untouched: its frame is the world's. Part b's is the world point
    // seen through b's inverse transform: (2 - 5, 1 - 0, 0 - 2) turned back a quarter.
    assert!((j.frame_a.origin - DVec3::new(2.0, 1.0, 0.0)).length() < 1e-9);
    assert!((j.frame_b.origin - DVec3::new(1.0, 3.0, -2.0)).length() < 1e-9);
    assert!((j.frame_b.axis - DVec3::Z).length() < 1e-9);
    // Moving b leaves the frames as they are; the listing follows part a.
    ok(
        &mut d,
        json!({"type": "set_transform", "id": b, "transform": {"origin": [9, 9, 9]}}),
    );
    let j2 = d.scene().joint(id).unwrap();
    assert_eq!(j2.frame_b, j.frame_b);
}

#[test]
fn a_mirrored_part_gets_a_frame_that_meets_the_world_frame() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    ok(
        &mut d,
        json!({"type": "set_transform", "id": b,
               "transform": {"origin": [5, 0, 0], "x_axis": [-1, 0, 0], "y_axis": [0, 1, 0], "z_axis": [0, 0, 1]}}),
    );
    let id = joint_id(&ok(
        &mut d,
        json!({"type": "add_joint", "kind": "revolute", "a": a, "b": b, "point": [2.0, 1.0, 0.0],
               "axis": [1, 0, 1], "reference": [0, 1, 0]}),
    ));
    let id: kernel::JointId = serde_json::from_value(id).unwrap();
    let j = d.scene().joint(id).unwrap();
    let t = d
        .scene()
        .root()
        .instance(match b.as_str().unwrap().parse().unwrap() {
            kernel::EntityId::Instance(i) => i,
            _ => unreachable!(),
        })
        .unwrap()
        .transform;
    let world = j.frame_b.world(&t);
    assert!((world.origin - DVec3::new(2.0, 1.0, 0.0)).length() < 1e-9);
    assert!((world.axis - DVec3::new(1.0, 0.0, 1.0).normalize()).length() < 1e-9);
    assert!((world.reference - DVec3::Y).length() < 1e-9);
}

#[test]
fn the_default_reference_is_the_planes_u_axis() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    // The rectangle u axis of a vertical normal is X; of a horizontal normal, Z cross normal.
    let id = joint_id(&ok(&mut d, hinge(&a, &b)));
    let id: kernel::JointId = serde_json::from_value(id).unwrap();
    assert!((d.scene().joint(id).unwrap().frame_a.reference - DVec3::X).length() < 1e-9);
    let id2 = joint_id(&ok(
        &mut d,
        json!({"type": "add_joint", "kind": "revolute", "a": a, "b": b, "point": [2, 0, 0], "axis": [1, 0, 0]}),
    ));
    let id2: kernel::JointId = serde_json::from_value(id2).unwrap();
    let r = d.scene().joint(id2).unwrap().frame_a.reference;
    let expected = DVec3::Z.cross(DVec3::X).normalize();
    assert!((r - expected).length() < 1e-9);
}

#[test]
fn bad_joints_are_invalid_input_and_change_nothing() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let revision = ask(&d, json!({"type": "stats"}))["revision"].clone();
    let mut bad = |v: Value| match run(&mut d, v) {
        Err((ErrorKind::InvalidInput, why)) => why,
        other => panic!("expected invalid_input, got {other:?}"),
    };
    let base = |f: &dyn Fn(&mut Value)| {
        let mut v = hinge(&a, &b);
        f(&mut v);
        v
    };
    assert!(bad(base(&|v| v["a"] = v["b"].clone())).contains("two different"));
    assert!(bad(base(&|v| v["a"] = json!("i99999"))).contains("not an instance"));
    assert!(bad(base(&|v| v["b"] = json!("e1"))).contains("not an instance"));
    assert!(bad(base(&|v| v["axis"] = json!([0, 0, 0]))).contains("axis"));
    assert!(bad(base(&|v| v["reference"] = json!([0, 0, 2]))).contains("reference"));
    assert!(bad(base(&|v| v["limits"] = json!([10, 20]))).contains("contain 0"));
    assert!(bad(base(&|v| v["limits"] = json!([20, -20]))).contains("min below max"));
    assert!(bad(base(&|v| v["name"] = json!("  "))).contains("name"));
    let fixed_limits = bad(base(&|v| {
        v["kind"] = json!("fixed");
        v["limits"] = json!([-1, 1]);
    }));
    assert!(fixed_limits.contains("no value"), "{fixed_limits}");
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], revision);
    assert!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_joint_needs_the_root_open() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    ok(&mut d, json!({"type": "open_instance", "id": a}));
    let e = run(&mut d, hinge(&a, &b)).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("root"), "{}", e.1);
}

#[test]
fn names_are_unique_and_trimmed() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let mut named = hinge(&a, &b);
    named["name"] = json!("  Elbow ");
    let id = joint_id(&ok(&mut d, named.clone()));
    assert_eq!(ask(&d, json!({"type": "joint", "id": id}))["name"], "Elbow");
    let e = run(&mut d, named).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("already exists"));
    let next = joint_id(&ok(&mut d, hinge(&a, &b)));
    assert_eq!(
        ask(&d, json!({"type": "joint", "id": next}))["name"],
        "Joint 1"
    );
}

#[test]
fn limits_are_degrees_for_angles_and_meters_for_distances() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let mut r = hinge(&a, &b);
    r["limits"] = json!([-90, 45]);
    let rid = joint_id(&ok(&mut d, r));
    let info = ask(&d, json!({"type": "joint", "id": rid}));
    close(
        &json!([info["limits"][0], info["limits"][1], 0]),
        [-90.0, 45.0, 0.0],
    );
    let stored: kernel::JointId = serde_json::from_value(rid).unwrap();
    let (lo, hi) = d.scene().joint(stored).unwrap().limits.unwrap();
    assert!((lo + std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert!((hi - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    let p = json!({"type": "add_joint", "kind": "prismatic", "a": a, "b": b, "point": [2, 0, 0], "axis": [1, 0, 0], "limits": [-0.5, 2.0]});
    let pid = joint_id(&ok(&mut d, p));
    let info = ask(&d, json!({"type": "joint", "id": pid}));
    close(
        &json!([info["limits"][0], info["limits"][1], 0]),
        [-0.5, 2.0, 0.0],
    );
}

#[test]
fn change_and_delete_are_single_undo_steps() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let id = joint_id(&ok(&mut d, hinge(&a, &b)));
    let before = ask(&d, json!({"type": "joint", "id": id}));

    let out = ok(
        &mut d,
        json!({"type": "change_joint", "joint": id, "name": "Elbow", "limits": [-30, 30]}),
    );
    assert_eq!(out["changes"]["joints"], json!([id]));
    let after = ask(&d, json!({"type": "joint", "id": id}));
    assert_eq!(after["name"], "Elbow");
    close(
        &json!([after["limits"][0], after["limits"][1], 0]),
        [-30.0, 30.0, 0.0],
    );
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(ask(&d, json!({"type": "joint", "id": id})), before);
    ok(&mut d, json!({"type": "redo"}));

    // null clears the limits; nothing to change makes no step.
    ok(
        &mut d,
        json!({"type": "change_joint", "joint": id, "limits": null}),
    );
    assert!(ask(&d, json!({"type": "joint", "id": id}))["limits"].is_null());
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    let out = ok(
        &mut d,
        json!({"type": "change_joint", "joint": id, "name": "Elbow"}),
    );
    assert!(out["changes"]["joints"].is_null());
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
    let e = run(&mut d, json!({"type": "change_joint", "joint": id})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    let e = run(
        &mut d,
        json!({"type": "change_joint", "joint": id, "limits": [1, 2]}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);

    ok(&mut d, json!({"type": "delete_joint", "joint": id}));
    assert!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let e = run(&mut d, json!({"type": "delete_joint", "joint": id})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let e = run(
        &mut d,
        json!({"type": "change_joint", "joint": "j99999", "name": "x"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let e = d
        .query(&serde_json::from_value(json!({"type": "joint", "id": "j99999"})).unwrap())
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[test]
fn erasing_a_part_deletes_its_joints_and_undo_brings_them_back() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let ab = joint_id(&ok(&mut d, hinge(&a, &b)));
    let ground = joint_id(&ok(
        &mut d,
        json!({"type": "add_joint", "kind": "fixed", "b": a, "point": [0, 0, 0], "axis": [0, 0, 1]}),
    ));
    let before = ask(&d, json!({"type": "joints"}));
    let out = ok(&mut d, json!({"type": "erase", "ids": [a]}));
    let listed = out["changes"]["joints"].as_array().unwrap();
    assert!(listed.contains(&ab) && listed.contains(&ground));
    assert!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(
        ask(&d, json!({"type": "joints"}))["joints"],
        before["joints"]
    );
    d.scene().validate().unwrap();
}

#[test]
fn grouping_a_part_deletes_its_joints() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    ok(&mut d, hinge(&a, &b));
    ok(&mut d, json!({"type": "make_group", "ids": [a]}));
    assert!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn joints_save_at_schema_25_and_read_back() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let path = scratch("none.skr");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    assert!(stored(&path)["schema_version"].as_u64().unwrap() < 25);
    assert!(stored(&path)["model"].get("joints").is_none());

    let mut r = hinge(&a, &b);
    r["limits"] = json!([-45, 90]);
    ok(&mut d, r);
    let mut c = json!({"type": "add_joint", "kind": "cylindrical", "a": a, "b": b, "point": [3, 0, 0], "axis": [0, 1, 0]});
    c["name"] = json!("Slot");
    ok(&mut d, c);
    ok(
        &mut d,
        json!({"type": "add_joint", "kind": "ball", "b": b, "point": [5, 0, 0], "axis": [0, 0, 1]}),
    );
    let listing = ask(&d, json!({"type": "joints"}))["joints"].clone();

    let path = scratch("joints.skr");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    let doc_json = stored(&path);
    assert_eq!(doc_json["schema_version"], 25);
    assert_eq!(doc_json["model"]["joints"].as_array().unwrap().len(), 3);
    assert_eq!(session::file::JOINTS_SCHEMA_VERSION, 25);

    let mut e = doc();
    ok(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    assert_eq!(ask(&e, json!({"type": "joints"}))["joints"], listing);
    assert_eq!(e.scene(), d.scene());

    let newer = scratch("newer.skr");
    std::fs::copy(&path, &newer).unwrap();
    edit_file(&newer, |doc| doc["schema_version"] = json!(26));
    let err = run(
        &mut e,
        json!({"type": "open", "path": newer.to_str().unwrap()}),
    )
    .unwrap_err();
    assert!(err.1.contains("newer"), "{}", err.1);

    // A joint naming a part the file does not have is refused.
    let broken = scratch("broken.skr");
    std::fs::copy(&path, &broken).unwrap();
    edit_file(&broken, |doc| {
        doc["model"]["joints"][0]["b"] = json!(987654)
    });
    let err = run(
        &mut e,
        json!({"type": "open", "path": broken.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::File);
}

#[test]
fn a_batch_names_a_joint_made_earlier() {
    let mut d = doc();
    let (a, b) = two_parts(&mut d);
    let commands: Vec<BatchCommand> = vec![
        hinge(&a, &b),
        json!({"type": "change_joint", "joint": "$0:j0", "name": "Crank"}),
        json!({"type": "delete_joint", "joint": "$0:j0"}),
        hinge(&a, &b),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    let joints = ask(&d, json!({"type": "joints"}));
    assert_eq!(joints["joints"].as_array().unwrap().len(), 1);
    assert_eq!(joints["joints"][0]["name"], "Joint 1");

    // Instances of an earlier command work in a and b; a joint reference elsewhere is refused.
    let parts = BatchCommand::parse(
        json!({"type": "add_box", "corner": [0, 0, 5], "size": [1, 1, 1]}),
        0,
    )
    .unwrap();
    let parts2 = BatchCommand::parse(
        json!({"type": "add_box", "corner": [4, 0, 5], "size": [1, 1, 1]}),
        1,
    )
    .unwrap();
    let joint = BatchCommand::parse(
        json!({"type": "add_joint", "kind": "fixed", "a": "$0:i0", "b": "$1:i0", "point": [2, 0, 5], "axis": [0, 0, 1]}),
        2,
    )
    .unwrap();
    d.execute_batch(vec![parts, parts2, joint], true).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "joints"}))["joints"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(BatchCommand::parse(json!({"type": "erase", "ids": ["$0:j0"]}), 1).is_err());
    assert!(BatchCommand::parse(json!({"type": "delete_joint", "joint": "$0:i0"}), 1).is_err());
    assert!(BatchCommand::parse(json!({"type": "delete_joint", "joint": "$0:j*"}), 1).is_err());
}
