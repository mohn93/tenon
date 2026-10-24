//! Cam followers through JSON commands and queries, and kept in files.

use std::f64::consts::PI;
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
        .join(format!("session-cams-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

/// The base circle's radius, and the lobe: a circle of radius `RL` whose center is `C` below
/// the cam's axis.
const RB: f64 = 0.03;
const C: f64 = 0.022;
const RL: f64 = 0.018;
const H: f64 = C + RL - RB;
const W: f64 = 0.01;
const SEGMENTS: u32 = 72;

/// How far a facet of the cam may sit inside the round shape it stands for.
fn facet_error() -> f64 {
    (C + RL) * (1.0 - (PI / SEGMENTS as f64).cos())
}

struct Rig {
    cam: Value,
    follower: Value,
    crank_joint: Value,
    cam_joint: Value,
    follower_joint: Value,
}

/// A cam (a base circle cylinder united with a lobe cylinder pointing down) turning about Y,
/// coupled 0.5 to a crank at x = 0.2, and a part on a prismatic joint whose axis runs along
/// `axis` through `at` (the part's bottom middle).
fn rig(d: &mut Document, at: [f64; 3], axis: [f64; 3]) -> Rig {
    let cylinder = |d: &mut Document, z: f64, r: f64| {
        ok(
            d,
            json!({"type": "add_cylinder", "base": [0.0, -W / 2.0, z], "axis": [0, 1, 0],
                   "radius": r, "height": W, "segments": SEGMENTS}),
        )["placed"][0]
            .clone()
    };
    let base = cylinder(d, 0.0, RB);
    let lobe = cylinder(d, -C, RL);
    let cam = ok(d, json!({"type": "solid_union", "ids": [base, lobe]}))["placed"][0].clone();
    let crank = ok(
        d,
        json!({"type": "add_box", "corner": [0.2, 0.0, 0.0], "size": [0.01, 0.01, 0.01]}),
    )["placed"][0]
        .clone();
    let follower = ok(
        d,
        json!({"type": "add_box", "corner": [at[0] - 0.005, at[1] - 0.005, at[2]], "size": [0.01, 0.01, 0.01]}),
    )["placed"][0]
        .clone();
    let joint = |d: &mut Document, kind: &str, b: &Value, point: [f64; 3], axis: [f64; 3]| {
        ok(
            d,
            json!({"type": "add_joint", "kind": kind, "b": b, "point": point, "axis": axis}),
        )["changes"]["joints"][0]
            .clone()
    };
    let crank_joint = joint(d, "revolute", &crank, [0.2, 0.0, 0.0], [0.0, 1.0, 0.0]);
    let cam_joint = joint(d, "revolute", &cam, [0.0; 3], [0.0, 1.0, 0.0]);
    let follower_joint = joint(d, "prismatic", &follower, at, axis);
    ok(
        d,
        json!({"type": "add_coupling", "driver": crank_joint, "driven": cam_joint, "ratio": 0.5}),
    );
    Rig {
        cam,
        follower,
        crank_joint,
        cam_joint,
        follower_joint,
    }
}

fn valve(d: &mut Document) -> Rig {
    rig(d, [0.0, 0.0, RB], [0.0, 0.0, 1.0])
}

fn follow(d: &mut Document, r: &Rig, extra: Value) -> Value {
    let mut command =
        json!({"type": "add_cam_follower", "cam": r.cam, "follower": r.follower_joint});
    for (k, v) in extra.as_object().unwrap() {
        command[k] = v.clone();
    }
    ok(d, command)["changes"]["cam_followers"][0].clone()
}

fn value(d: &Document, id: &Value) -> f64 {
    ask(d, json!({"type": "joint", "id": id}))["value"]
        .as_f64()
        .unwrap()
}

fn followers(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "cam_followers"}))["cam_followers"]
        .as_array()
        .unwrap()
        .clone()
}

fn drive(d: &mut Document, joint: &Value, v: f64) -> Value {
    ok(d, json!({"type": "drive", "joint": joint, "value": v}))
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

#[test]
fn a_valve_lifts_once_per_two_crank_turns_by_the_lobe_height() {
    let mut d = doc();
    let r = valve(&mut d);
    let w = follow(&mut d, &r, json!({}));
    assert!(w.as_str().unwrap().starts_with('w'), "{w}");
    let listed = followers(&d);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], w);
    assert_eq!(listed[0]["cam"], r.cam);
    assert_eq!(listed[0]["follower"], r.follower_joint);
    assert_eq!(listed[0]["roller_radius"], 0.0);
    // Pushed down, toward the cam's middle.
    assert_eq!(listed[0]["preload_direction"], json!([0.0, 0.0, -1.0]));

    let sag = facet_error();
    let mut lifts = Vec::new();
    for k in 0..=36 {
        let out = drive(&mut d, &r.crank_joint, 20.0 * k as f64);
        if k > 0 {
            let joints = out["changes"]["joints"].as_array().unwrap();
            assert!(joints.contains(&r.cam_joint));
        }
        assert!((value(&d, &r.cam_joint) - 10.0 * k as f64).abs() < 1e-6);
        lifts.push(value(&d, &r.follower_joint));
    }
    let peak = lifts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((peak - H).abs() <= sag, "peak {peak} vs {H}");
    assert!(
        (lifts[18] - peak).abs() < 1e-12,
        "the peak is at 360 degrees"
    );
    let up: Vec<bool> = lifts.iter().map(|l| *l > H / 2.0).collect();
    let rises = up.windows(2).filter(|w| !w[0] && w[1]).count();
    let falls = up.windows(2).filter(|w| w[0] && !w[1]).count();
    assert_eq!((rises, falls), (1, 1), "{lifts:?}");
    // Cam within 90 degrees of the start (lobe down): the follower is on the base circle.
    for (k, l) in lifts.iter().enumerate() {
        let cam = 10.0 * k as f64;
        if !(90.0..=270.0).contains(&cam) {
            assert!(l.abs() <= sag, "base lift at {cam}: {l}");
        }
    }
}

#[test]
fn a_roller_radius_stands_the_follower_back_and_undo_restores_it() {
    let mut d = doc();
    let r = valve(&mut d);
    let w = follow(&mut d, &r, json!({}));
    drive(&mut d, &r.crank_joint, 360.0);
    let knife = value(&d, &r.follower_joint);
    drive(&mut d, &r.crank_joint, 0.0);
    ok(
        &mut d,
        json!({"type": "delete_cam_follower", "cam_follower": w}),
    );
    let w = follow(&mut d, &r, json!({"roller_radius": 0.005}));
    assert_eq!(followers(&d)[0]["roller_radius"], 0.005);
    let start = transforms(&d);
    let depth = undo_depth(&d);
    drive(&mut d, &r.crank_joint, 360.0);
    assert!((value(&d, &r.follower_joint) - (knife + 0.005)).abs() < 1e-9);
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(undo_depth(&d), depth);
    assert_eq!(transforms(&d), start);
    assert_eq!(followers(&d)[0]["id"], w);
}

#[test]
fn a_follower_that_loses_the_cam_is_refused_and_nothing_changes() {
    let mut d = doc();
    // Along X at a height only the lobe's tip reaches, pushed toward -X.
    let r = rig(&mut d, [0.08, 0.0, 0.035], [1.0, 0.0, 0.0]);
    let e = run(
        &mut d,
        json!({"type": "add_cam_follower", "cam": r.cam, "follower": r.follower_joint, "preload_direction": [-1, 0, 0]}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("does not meet cam"), "{}", e.1);
    drive(&mut d, &r.crank_joint, 360.0);
    let w = follow(&mut d, &r, json!({"preload_direction": [-1, 0, 0]}));
    drive(&mut d, &r.crank_joint, 350.0);
    let before = transforms(&d);
    let depth = undo_depth(&d);
    let revision = ask(&d, json!({"type": "history"}))["revision"].clone();
    let e = run(
        &mut d,
        json!({"type": "drive", "joint": r.crank_joint, "value": 180}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(
        e.1.contains(w.as_str().unwrap()) && e.1.contains("loses contact"),
        "{}",
        e.1
    );
    assert_eq!(transforms(&d), before);
    assert_eq!(undo_depth(&d), depth);
    assert_eq!(ask(&d, json!({"type": "history"}))["revision"], revision);
}

#[test]
fn bad_followers_are_refused_and_change_nothing() {
    let mut d = doc();
    let r = valve(&mut d);
    let depth = undo_depth(&d);
    let bad = |d: &mut Document, extra: Value| {
        let mut command =
            json!({"type": "add_cam_follower", "cam": r.cam, "follower": r.follower_joint});
        for (k, v) in extra.as_object().unwrap() {
            command[k] = v.clone();
        }
        run(d, command).unwrap_err()
    };
    for extra in [
        json!({"roller_radius": -0.001}),
        json!({"preload_direction": [1, 0, 0]}),
        json!({"preload_direction": [0, 0, 0]}),
        json!({"cam": "f1"}),
        json!({"follower": r.cam_joint}),
        json!({"cam": r.follower}),
    ] {
        let e = bad(&mut d, extra.clone());
        assert_eq!(e.0, ErrorKind::InvalidInput, "{extra}: {}", e.1);
    }
    assert_eq!(
        bad(&mut d, json!({"follower": "j987654"})).0,
        ErrorKind::NotFound
    );
    assert_eq!(undo_depth(&d), depth);
    assert!(followers(&d).is_empty());
    // A follower joint is not driven or coupled itself.
    follow(&mut d, &r, json!({}));
    let e = run(
        &mut d,
        json!({"type": "drive", "joint": r.follower_joint, "value": 0.01}),
    )
    .unwrap_err();
    assert!(e.1.contains("follows cam"), "{}", e.1);
    let e = run(
        &mut d,
        json!({"type": "add_coupling", "driver": r.crank_joint, "driven": r.follower_joint, "ratio": 1}),
    )
    .unwrap_err();
    assert!(e.1.contains("follows a cam"), "{}", e.1);
}

#[test]
fn deleting_the_joint_or_erasing_the_cam_drops_the_follower_in_one_step() {
    let mut d = doc();
    let r = valve(&mut d);
    let w = follow(&mut d, &r, json!({}));
    let listed = followers(&d);

    let out = ok(
        &mut d,
        json!({"type": "delete_cam_follower", "cam_follower": w}),
    );
    assert_eq!(out["changes"]["cam_followers"], json!([w]));
    assert!(followers(&d).is_empty());
    let e = run(
        &mut d,
        json!({"type": "delete_cam_follower", "cam_follower": w}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(followers(&d), listed);

    let out = ok(
        &mut d,
        json!({"type": "delete_joint", "joint": r.follower_joint}),
    );
    assert_eq!(out["changes"]["cam_followers"], json!([w]));
    assert!(followers(&d).is_empty());
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(followers(&d), listed);

    for part in [&r.cam, &r.follower] {
        let out = ok(&mut d, json!({"type": "erase", "ids": [part]}));
        assert_eq!(out["changes"]["cam_followers"], json!([w]));
        assert!(followers(&d).is_empty());
        d.scene().validate().unwrap();
        ok(&mut d, json!({"type": "undo"}));
        assert_eq!(followers(&d), listed);
    }
    d.scene().validate().unwrap();
}

#[test]
fn followers_save_at_schema_28_and_read_back() {
    let mut d = doc();
    let r = valve(&mut d);
    follow(&mut d, &r, json!({"roller_radius": 0.002}));
    let listing = followers(&d);
    let path = scratch("cams.skr");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    let document = stored(&path);
    assert_eq!(document["schema_version"], 28);
    assert_eq!(session::file::CAM_FOLLOWERS_SCHEMA_VERSION, 28);
    let saved = &document["model"]["cam_followers"];
    assert_eq!(saved.as_array().unwrap().len(), 1);
    assert_eq!(saved[0]["preload"], -1.0);

    let mut e = doc();
    ok(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    assert_eq!(followers(&e), listing);
    assert_eq!(e.scene(), d.scene());
    drive(&mut e, &r.crank_joint, 360.0);
    drive(&mut d, &r.crank_joint, 360.0);
    assert_eq!(value(&e, &r.follower_joint), value(&d, &r.follower_joint));

    // Without followers the couplings version is written.
    ok(
        &mut d,
        json!({"type": "delete_cam_follower", "cam_follower": listing[0]["id"]}),
    );
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    assert_eq!(stored(&path)["schema_version"], 27);
}

#[test]
fn a_batch_adds_a_follower_to_parts_made_earlier_and_deletes_it() {
    let mut d = doc();
    let r = valve(&mut d);
    // This follower's axis line runs along the cam's end face (y = W / 2): a grazing contact.
    let commands: Vec<BatchCommand> = vec![
        json!({"type": "add_box", "corner": [-0.005, 0.0, RB], "size": [0.01, 0.01, 0.01]}),
        json!({"type": "add_joint", "kind": "prismatic", "b": "$0:i0", "point": [0, 0.005, RB], "axis": [0, 0, 1]}),
        json!({"type": "add_cam_follower", "cam": r.cam, "follower": "$1:j0"}),
        json!({"type": "drive", "joint": r.crank_joint, "value": 360}),
        json!({"type": "delete_cam_follower", "cam_follower": "$2:w0"}),
        json!({"type": "add_cam_follower", "cam": r.cam, "follower": "$1:j0", "roller_radius": 0.001}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    let listed = followers(&d);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["roller_radius"], 0.001);
    let joint = listed[0]["follower"].clone();
    assert!(
        (value(&d, &joint) - H).abs() <= facet_error(),
        "{}",
        value(&d, &joint)
    );

    let parse = |v: Value| BatchCommand::parse(v, 1);
    assert!(parse(json!({"type": "delete_cam_follower", "cam_follower": "$0:k0"})).is_err());
    assert!(parse(json!({"type": "delete_cam_follower", "cam_follower": "$0:w*"})).is_err());
    assert!(parse(json!({"type": "add_cam_follower", "cam": "i1", "follower": "$0:w0"})).is_err());
    assert!(parse(json!({"type": "erase", "ids": ["$0:w0"]})).is_err());
}
