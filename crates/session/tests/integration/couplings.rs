//! Couplings between joints, through JSON commands and queries, and kept in files.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
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
        .join(format!("session-couplings-{}", std::process::id()));
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

/// A small box at `x`; its instance id.
fn part(d: &mut Document, x: f64) -> Value {
    ok(
        d,
        json!({"type": "add_box", "corner": [x, 0.0, 0.0], "size": [0.01, 0.01, 0.01]}),
    )["placed"][0]
        .clone()
}

/// A joint of `kind` from the ground to `b` at x along `axis`; its id.
fn ground_joint(d: &mut Document, kind: &str, b: &Value, x: f64, axis: [f64; 3]) -> Value {
    ok(
        d,
        json!({"type": "add_joint", "kind": kind, "b": b, "point": [x, 0.0, 0.0], "axis": axis, "reference": [0, 1, 0]}),
    )["changes"]["joints"][0]
        .clone()
}

const Z: [f64; 3] = [0.0, 0.0, 1.0];

/// Shafts turning about Z on the ground, at x = 0, 0.1, 0.2, ...: their parts and joints.
fn shafts(d: &mut Document, n: usize) -> (Vec<Value>, Vec<Value>) {
    let mut parts = Vec::new();
    let mut joints = Vec::new();
    for k in 0..n {
        let x = 0.1 * k as f64;
        let p = part(d, x);
        joints.push(ground_joint(d, "revolute", &p, x, Z));
        parts.push(p);
    }
    (parts, joints)
}

fn couple(d: &mut Document, driver: &Value, driven: &Value, ratio: f64) -> Value {
    ok(
        d,
        json!({"type": "add_coupling", "driver": driver, "driven": driven, "ratio": ratio}),
    )["changes"]["couplings"][0]
        .clone()
}

fn value(d: &Document, id: &Value) -> f64 {
    ask(d, json!({"type": "joint", "id": id}))["value"]
        .as_f64()
        .unwrap()
}

fn couplings(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "couplings"}))["couplings"]
        .as_array()
        .unwrap()
        .clone()
}

fn transforms(d: &Document) -> Vec<glam::DAffine3> {
    d.scene()
        .root()
        .instances()
        .map(|(_, i)| i.transform)
        .collect()
}

fn undo_depth(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["undo_depth"].clone()
}

#[track_caller]
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} vs {b}");
}

#[test]
fn a_cam_coupled_two_to_one_turns_once_in_two_crank_turns() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 2);
    let start = transforms(&d);
    let k = couple(&mut d, &j[0], &j[1], 0.5);
    assert!(k.as_str().unwrap().starts_with('k'), "{k}");
    let listed = couplings(&d);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], k);
    assert_eq!(listed[0]["driver"], j[0]);
    assert_eq!(listed[0]["driven"], j[1]);
    assert_eq!(listed[0]["ratio"], 0.5);
    assert_eq!(listed[0]["offset"], 0.0);
    let out = ok(
        &mut d,
        json!({"type": "drive", "joint": j[0], "value": 720}),
    );
    assert!(out["changes"]["joints"].as_array().unwrap().contains(&j[1]));
    assert_eq!(value(&d, &j[0]), 720.0);
    near(value(&d, &j[1]), 360.0);
    // A whole turn of the cam puts it back where it was.
    let cam = transforms(&d)[1];
    assert!(cam.abs_diff_eq(start[1], 1e-9), "{cam:?}");
}

#[test]
fn a_negative_ratio_turns_the_driven_joint_the_other_way() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 2);
    couple(&mut d, &j[0], &j[1], -2.0);
    ok(&mut d, json!({"type": "drive", "joint": j[0], "value": 30}));
    near(value(&d, &j[1]), -60.0);
    // Driving the driven joint turns the driver.
    ok(&mut d, json!({"type": "drive", "joint": j[1], "value": 90}));
    near(value(&d, &j[0]), -45.0);
}

#[test]
fn a_pinion_drives_a_rack_in_meters_per_degree() {
    let mut d = doc();
    let pinion = part(&mut d, 0.0);
    let rack = part(&mut d, 0.1);
    let jp = ground_joint(&mut d, "revolute", &pinion, 0.0, Z);
    let jr = ground_joint(&mut d, "prismatic", &rack, 0.1, [1.0, 0.0, 0.0]);
    let start = transforms(&d)[1];
    couple(&mut d, &jp, &jr, 0.01);
    near(couplings(&d)[0]["ratio"].as_f64().unwrap(), 0.01);
    ok(&mut d, json!({"type": "drive", "joint": jp, "value": 90}));
    near(value(&d, &jr), 0.9);
    let moved = transforms(&d)[1].translation - start.translation;
    near(moved.x, 0.9);
    near(moved.y, 0.0);
    // And a rack drives a pinion, in degrees per meter.
    let (_, j) = shafts(&mut d, 1);
    let out = ok(
        &mut d,
        json!({"type": "add_coupling", "driver": jr, "driven": j[0], "ratio": 100, "offset": 5}),
    );
    let k = out["changes"]["couplings"][0].clone();
    let info = couplings(&d).into_iter().find(|c| c["id"] == k).unwrap();
    near(info["ratio"].as_f64().unwrap(), 100.0);
    near(info["offset"].as_f64().unwrap(), 5.0);
    ok(&mut d, json!({"type": "drive", "joint": jp, "value": 100}));
    near(value(&d, &jr), 1.0);
    near(value(&d, &j[0]), 105.0);
}

#[test]
fn without_an_offset_a_coupling_holds_the_pose_it_was_added_in() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 2);
    ok(&mut d, json!({"type": "drive", "joint": j[0], "value": 30}));
    ok(&mut d, json!({"type": "drive", "joint": j[1], "value": 50}));
    let before = transforms(&d);
    let out = ok(
        &mut d,
        json!({"type": "add_coupling", "driver": j[0], "driven": j[1], "ratio": 0.5}),
    );
    assert_eq!(out["changes"]["modified"], json!([]), "{out}");
    assert_eq!(transforms(&d), before);
    near(couplings(&d)[0]["offset"].as_f64().unwrap(), 35.0);
    ok(&mut d, json!({"type": "drive", "joint": j[0], "value": 90}));
    near(value(&d, &j[1]), 80.0);
}

#[test]
fn contradictory_couplings_are_refused_at_drive_time_and_change_nothing() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 3);
    couple(&mut d, &j[0], &j[1], 2.0);
    couple(&mut d, &j[1], &j[2], 2.0);
    couple(&mut d, &j[2], &j[0], 2.0);
    let before = transforms(&d);
    let joints = ask(&d, json!({"type": "joints"}));
    let depth = undo_depth(&d);
    let e = run(&mut d, json!({"type": "drive", "joint": j[0], "value": 10})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("cannot all be met"), "{}", e.1);
    assert_eq!(transforms(&d), before);
    assert_eq!(ask(&d, json!({"type": "joints"})), joints);
    assert_eq!(undo_depth(&d), depth);
}

#[test]
fn bad_couplings_are_refused_and_change_nothing() {
    let mut d = doc();
    let (parts, j) = shafts(&mut d, 2);
    let fixed = ok(
        &mut d,
        json!({"type": "add_joint", "kind": "fixed", "a": parts[0], "b": parts[1], "point": [0, 0, 0], "axis": Z}),
    )["changes"]["joints"][0]
        .clone();
    let depth = undo_depth(&d);
    let refused = |d: &mut Document, driver: &Value, driven: &Value, ratio: f64| {
        run(
            d,
            json!({"type": "add_coupling", "driver": driver, "driven": driven, "ratio": ratio}),
        )
        .unwrap_err()
    };
    for ratio in [0.0, 1000.5, -2000.0] {
        let e = refused(&mut d, &j[0], &j[1], ratio);
        assert_eq!(e.0, ErrorKind::InvalidInput, "{ratio}");
        assert!(e.1.contains("ratio"), "{}", e.1);
    }
    assert_eq!(
        refused(&mut d, &j[0], &j[0], 1.0).0,
        ErrorKind::InvalidInput
    );
    let e = refused(&mut d, &j[0], &fixed, 1.0);
    assert!(e.1.contains("no value"), "{}", e.1);
    assert_eq!(
        refused(&mut d, &j[0], &json!("j99999"), 1.0).0,
        ErrorKind::NotFound
    );
    assert_eq!(undo_depth(&d), depth);
    assert!(couplings(&d).is_empty());
    couple(&mut d, &j[0], &j[1], 1000.0);
    let e = refused(&mut d, &j[1], &j[0], 1.0);
    assert!(e.1.contains("already coupled"), "{}", e.1);
}

#[test]
fn delete_and_erase_drop_couplings_as_one_undo_step() {
    let mut d = doc();
    let (parts, j) = shafts(&mut d, 2);
    let k = couple(&mut d, &j[0], &j[1], 0.5);
    let listed = couplings(&d);

    let out = ok(&mut d, json!({"type": "delete_coupling", "coupling": k}));
    assert_eq!(out["changes"]["couplings"], json!([k]));
    assert!(couplings(&d).is_empty());
    let e = run(&mut d, json!({"type": "delete_coupling", "coupling": k})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(couplings(&d), listed);

    let out = ok(&mut d, json!({"type": "delete_joint", "joint": j[1]}));
    assert_eq!(out["changes"]["couplings"], json!([k]));
    assert!(couplings(&d).is_empty());
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(couplings(&d), listed);

    let out = ok(&mut d, json!({"type": "erase", "ids": [parts[0]]}));
    assert_eq!(out["changes"]["couplings"], json!([k]));
    assert!(couplings(&d).is_empty());
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(couplings(&d), listed);
    d.scene().validate().unwrap();

    // A coupled drive undoes in one step too.
    ok(
        &mut d,
        json!({"type": "drive", "joint": j[0], "value": 400}),
    );
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(value(&d, &j[0]), 0.0);
    assert_eq!(value(&d, &j[1]), 0.0);
}

#[test]
fn couplings_save_at_schema_27_and_read_back() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 2);
    couple(&mut d, &j[0], &j[1], 0.5);
    let listing = couplings(&d);
    let path = scratch("couplings.tnn");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    let document = stored(&path);
    assert_eq!(document["schema_version"], 27);
    assert_eq!(session::file::COUPLINGS_SCHEMA_VERSION, 27);
    assert_eq!(document["model"]["couplings"].as_array().unwrap().len(), 1);

    let mut e = doc();
    ok(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    assert_eq!(couplings(&e), listing);
    assert_eq!(e.scene(), d.scene());
    ok(
        &mut e,
        json!({"type": "drive", "joint": j[0], "value": 720}),
    );
    near(value(&e, &j[1]), 360.0);

    // A coupling naming a joint the file does not have is refused.
    let broken = scratch("broken.tnn");
    std::fs::copy(&path, &broken).unwrap();
    edit_file(&broken, |doc| {
        doc["model"]["couplings"][0]["driven"] = json!(987654)
    });
    let err = run(
        &mut e,
        json!({"type": "open", "path": broken.to_str().unwrap(), "discard": true}),
    )
    .unwrap_err();
    assert!(err.1.contains("does not exist"), "{}", err.1);

    // Without couplings the joints version is written.
    ok(
        &mut d,
        json!({"type": "delete_coupling", "coupling": listing[0]["id"]}),
    );
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    assert_eq!(stored(&path)["schema_version"], 25);
}

#[test]
fn a_batch_couples_joints_made_earlier_and_deletes_the_coupling() {
    let mut d = doc();
    let a = part(&mut d, 0.0);
    let b = part(&mut d, 0.1);
    let commands: Vec<BatchCommand> = vec![
        json!({"type": "add_joint", "kind": "revolute", "b": a, "point": [0, 0, 0], "axis": Z}),
        json!({"type": "add_joint", "kind": "revolute", "b": b, "point": [0.1, 0, 0], "axis": Z}),
        json!({"type": "add_coupling", "driver": "$0:j0", "driven": "$1:j0", "ratio": 0.5}),
        json!({"type": "drive", "joint": "$0:j0", "value": 90}),
        json!({"type": "delete_coupling", "coupling": "$2:k0"}),
        json!({"type": "add_coupling", "driver": "$1:j0", "driven": "$0:j0", "ratio": 2}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    let listed = couplings(&d);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["ratio"], 2.0);
    let joints = ask(&d, json!({"type": "joints"}))["joints"].clone();
    near(joints[1]["value"].as_f64().unwrap(), 45.0);
    // The coupling just added holds that pose: 90 = 2 * 45 + 0.
    near(listed[0]["offset"].as_f64().unwrap(), 0.0);

    let parse = |v: Value| BatchCommand::parse(v, 1);
    assert!(parse(json!({"type": "delete_coupling", "coupling": "$0:j0"})).is_err());
    assert!(parse(json!({"type": "delete_coupling", "coupling": "$0:k*"})).is_err());
    assert!(
        parse(json!({"type": "add_coupling", "driver": "$0:k0", "driven": "j1", "ratio": 1}))
            .is_err()
    );
    assert!(parse(json!({"type": "erase", "ids": ["$0:k0"]})).is_err());
}

#[test]
fn tiny_ratios_are_refused_and_a_ten_millimeter_pinion_drives_its_rack() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 2);
    for ratio in [1e-300, 1e-4, -1e-9] {
        let e = run(
            &mut d,
            json!({"type": "add_coupling", "driver": j[0], "driven": j[1], "ratio": ratio}),
        )
        .unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{ratio}");
        assert!(e.1.contains("from 0.001 to 1000"), "{}", e.1);
    }
    assert!(couplings(&d).is_empty());
    // A 10 mm pinion moves its rack 0.01 m per radian: about 0.000175 m per degree.
    let pinion = part(&mut d, 0.5);
    let rack = part(&mut d, 0.6);
    let jp = ground_joint(&mut d, "revolute", &pinion, 0.5, Z);
    let jr = ground_joint(&mut d, "prismatic", &rack, 0.6, [1.0, 0.0, 0.0]);
    let per_degree = 0.01 * 1f64.to_radians();
    couple(&mut d, &jp, &jr, per_degree);
    ok(&mut d, json!({"type": "drive", "joint": jp, "value": 90}));
    near(value(&d, &jr), 0.01 * std::f64::consts::FRAC_PI_2);
}

#[test]
fn offsets_past_half_a_turn_are_accepted_and_met() {
    for offset in [190.0, 270.0, -270.0, 360.0] {
        let mut d = doc();
        let (_, j) = shafts(&mut d, 2);
        ok(
            &mut d,
            json!({"type": "add_coupling", "driver": j[0], "driven": j[1], "ratio": 0.5, "offset": offset}),
        );
        ok(
            &mut d,
            json!({"type": "drive", "joint": j[0], "value": 100}),
        );
        let want = 50.0 + offset;
        let off = value(&d, &j[1]) - want;
        assert!(
            (off - 360.0 * (off / 360.0).round()).abs() < 1e-9,
            "{offset}: {off}"
        );
        let x = transforms(&d)[1].matrix3 * glam::DVec3::X;
        let turned = x.y.atan2(x.x).to_degrees() - want;
        assert!(
            (turned - 360.0 * (turned / 360.0).round()).abs() < 1e-6,
            "{offset}: {turned}"
        );
    }
}

#[test]
fn a_coupling_keeps_working_after_going_to_a_scene_with_positions() {
    let mut d = doc();
    let (_, j) = shafts(&mut d, 2);
    couple(&mut d, &j[0], &j[1], 0.5);
    let s =
        ok(&mut d, json!({"type": "add_scene", "positions": true}))["changes"]["scenes"][0].clone();
    ok(
        &mut d,
        json!({"type": "drive", "joint": j[0], "value": 200}),
    );
    near(value(&d, &j[1]), 100.0);
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    // The parts stand as at crank 0; the values read back whichever turn is nearest.
    for crank in [30.0, 250.0, -90.0, 0.0] {
        ok(
            &mut d,
            json!({"type": "drive", "joint": j[0], "value": crank}),
        );
        let x = transforms(&d)[1].matrix3 * glam::DVec3::X;
        let cam = x.y.atan2(x.x).to_degrees();
        let off = value(&d, &j[1]) - 0.5 * value(&d, &j[0]);
        let off = off - 360.0 * (off / 360.0).round();
        assert!(off.abs() < 1e-6, "crank {crank}: {off}");
        let shown = value(&d, &j[1]) - cam;
        assert!(
            (shown - 360.0 * (shown / 360.0).round()).abs() < 1e-6,
            "crank {crank}: the cam stands at its value"
        );
    }
}
