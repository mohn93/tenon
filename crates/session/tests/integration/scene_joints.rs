//! Scenes that store joint values: kept in files, gone to with a solve in one undo step,
//! interpolated by transitions, and updated by the scene commands.

use std::path::PathBuf;

use glam::{DAffine3, DVec2};
use kernel::{EntityId, JointId, PageId};
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
        .join(format!("session-scene-joints-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

struct Rig {
    a: Value,
    b: Value,
    /// A hinge to the ground about Z through (0.5, 0.5) on the first box.
    ja: Value,
    /// A hinge to the ground about Z through (5.5, 0.5) on the second box.
    jb: Value,
}

/// Two unit boxes (x = 0 and x = 5), each hinged to the ground about its own vertical axis.
fn rig(d: &mut Document) -> Rig {
    let mut place = |x: f64| {
        ok(
            d,
            json!({"type": "add_box", "corner": [x, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
        )["placed"][0]
            .clone()
    };
    let a = place(0.0);
    let b = place(5.0);
    let mut joint = |part: &Value, x: f64| {
        ok(
            d,
            json!({"type": "add_joint", "kind": "revolute", "b": part, "point": [x, 0.5, 0.0], "axis": [0, 0, 1]}),
        )["changes"]["joints"][0]
            .clone()
    };
    let ja = joint(&a, 0.5);
    let jb = joint(&b, 5.5);
    Rig { a, b, ja, jb }
}

fn transform(d: &Document, part: &Value) -> DAffine3 {
    let id: EntityId = serde_json::from_value(part.clone()).unwrap();
    let EntityId::Instance(i) = id else { panic!() };
    d.scene().root().instance(i).unwrap().transform
}

fn joint_value(d: &Document, joint: &Value) -> f64 {
    ask(d, json!({"type": "joint", "id": joint}))["value"]
        .as_f64()
        .unwrap()
}

fn drive(d: &mut Document, joint: &Value, degrees: f64) {
    ok(
        d,
        json!({"type": "drive", "joint": joint, "value": degrees}),
    );
}

fn scene_id(out: &Value) -> Value {
    out["changes"]["scenes"][0].clone()
}

fn turned(d: &Document, part: &Value, degrees: f64) -> bool {
    let t = transform(d, part);
    let x = t.matrix3 * glam::DVec3::X;
    (x.y.atan2(x.x).to_degrees() - degrees).abs() < 1e-6
}

#[test]
fn a_scene_stores_joint_values_and_lists_them() {
    let mut d = doc();
    let r = rig(&mut d);
    drive(&mut d, &r.ja, 30.0);
    drive(&mut d, &r.jb, -20.0);
    let all = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": true})));
    let listed = ask(&d, json!({"type": "scenes"}));
    assert_eq!(
        listed["scenes"][0]["joints"],
        json!([r.ja, r.jb]),
        "{listed}"
    );
    let detail = ask(&d, json!({"type": "scene", "scene": all}));
    let values = detail["joints"].as_array().unwrap();
    assert_eq!(values[0]["joint"], r.ja);
    assert!((values[0]["value"].as_f64().unwrap() - 30.0).abs() < 1e-9);
    assert!((values[1]["value"].as_f64().unwrap() + 20.0).abs() < 1e-9);
    // Ids store just those; none stores none.
    let one = scene_id(&ok(
        &mut d,
        json!({"type": "add_scene", "name": "One", "joints": [r.jb]}),
    ));
    let detail = ask(&d, json!({"type": "scene", "scene": one}));
    assert_eq!(detail["joints"].as_array().unwrap().len(), 1);
    let plain = scene_id(&ok(&mut d, json!({"type": "add_scene", "name": "Plain"})));
    assert!(ask(&d, json!({"type": "scene", "scene": plain}))["joints"].is_null());
    let (kind, _) = run(&mut d, json!({"type": "add_scene", "joints": []})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _) = run(&mut d, json!({"type": "add_scene", "joints": ["j999"]})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}

#[test]
fn going_to_a_scene_drives_its_joints_in_one_undo_step() {
    let mut d = doc();
    let r = rig(&mut d);
    drive(&mut d, &r.ja, 30.0);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": [r.ja]})));
    drive(&mut d, &r.ja, 90.0);
    drive(&mut d, &r.jb, 45.0);
    let steps = ask(&d, json!({"type": "history"}))["undo"]
        .as_array()
        .unwrap()
        .len();
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    assert!(turned(&d, &r.a, 30.0));
    assert!((joint_value(&d, &r.ja) - 30.0).abs() < 1e-9);
    // The joint the scene does not store stays where it was.
    assert!(turned(&d, &r.b, 45.0));
    let history = ask(&d, json!({"type": "history"}))["undo"].clone();
    assert_eq!(history.as_array().unwrap().len(), steps + 1, "{history}");
    assert!(history.to_string().contains("Go to scene"), "{history}");
    ok(&mut d, json!({"type": "undo"}));
    assert!(turned(&d, &r.a, 90.0));
    assert!((joint_value(&d, &r.ja) - 90.0).abs() < 1e-9);
    assert!(turned(&d, &r.b, 45.0));
}

#[test]
fn going_to_a_scene_with_positions_and_joint_values_applies_both() {
    let mut d = doc();
    let r = rig(&mut d);
    drive(&mut d, &r.ja, 30.0);
    let s = scene_id(&ok(
        &mut d,
        json!({"type": "add_scene", "positions": true, "joints": [r.ja]}),
    ));
    drive(&mut d, &r.ja, 0.0);
    drive(&mut d, &r.jb, 60.0);
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    assert!(turned(&d, &r.a, 30.0));
    assert!(turned(&d, &r.b, 0.0));
}

#[test]
fn a_scene_that_cannot_be_met_is_refused_and_changes_nothing() {
    let mut d = doc();
    let r = rig(&mut d);
    let tag = ok(&mut d, json!({"type": "add_tag", "name": "T"}))["changes"]["tags"][0].clone();
    drive(&mut d, &r.ja, 80.0);
    let s = scene_id(&ok(
        &mut d,
        json!({"type": "add_scene", "camera": false, "joints": [r.ja]}),
    ));
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": s, "tags": true}),
    );
    ok(
        &mut d,
        json!({"type": "change_tag", "tag": tag, "visible": false}),
    );
    drive(&mut d, &r.ja, 0.0);
    ok(
        &mut d,
        json!({"type": "change_joint", "joint": r.ja, "limits": [-10, 50]}),
    );
    let (revision, before) = (d.revision(), transform(&d, &r.a));
    let (kind, message) = run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("limits"), "{message}");
    assert_eq!(d.revision(), revision);
    assert_eq!(transform(&d, &r.a), before);
    let tags = ask(&d, json!({"type": "tags"}));
    assert_eq!(
        tags["tags"][0]["visible"], false,
        "tag visibility unchanged: {tags}"
    );
    assert_ne!(
        d.selected_scene().map(|p| p.to_string()),
        Some(s.to_string())
    );
}

#[test]
fn update_and_change_scene_choose_the_stored_joint_values() {
    let mut d = doc();
    let r = rig(&mut d);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": [r.ja]})));
    let stored_values =
        |d: &Document| ask(d, json!({"type": "scene", "scene": s}))["joints"].clone();
    drive(&mut d, &r.ja, 40.0);
    // Absent keeps what is stored; update stores the same set again from now.
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": s, "name": "Renamed"}),
    );
    assert_eq!(stored_values(&d)[0]["value"], 0.0);
    ok(&mut d, json!({"type": "update_scene", "scene": s}));
    assert!((stored_values(&d)[0]["value"].as_f64().unwrap() - 40.0).abs() < 1e-9);
    // Ids replace the set; true stores the stored set again; false drops them.
    drive(&mut d, &r.jb, 15.0);
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": s, "joints": [r.jb]}),
    );
    assert_eq!(stored_values(&d).as_array().unwrap().len(), 1);
    assert_eq!(stored_values(&d)[0]["joint"], r.jb);
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": s, "joints": true}),
    );
    assert_eq!(stored_values(&d).as_array().unwrap().len(), 1);
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": s, "joints": false}),
    );
    assert!(stored_values(&d).is_null());
    // An update of a scene storing none stores none; true then stores every joint.
    ok(&mut d, json!({"type": "update_scene", "scene": s}));
    assert!(stored_values(&d).is_null());
    ok(
        &mut d,
        json!({"type": "update_scene", "scene": s, "joints": true}),
    );
    assert_eq!(stored_values(&d).as_array().unwrap().len(), 2);
}

#[test]
fn erasing_a_joint_drops_its_stored_values_and_undo_brings_them_back() {
    let mut d = doc();
    let r = rig(&mut d);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": true})));
    ok(&mut d, json!({"type": "delete_joint", "joint": r.jb}));
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["joints"].as_array().unwrap().len(), 1, "{detail}");
    ok(&mut d, json!({"type": "delete_joint", "joint": r.ja}));
    assert!(ask(&d, json!({"type": "scene", "scene": s}))["joints"].is_null());
    ok(&mut d, json!({"type": "undo", "count": 2}));
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["joints"].as_array().unwrap().len(), 2);
    // Erasing a part drops its joint and with it the stored value.
    let EntityId::Instance(_) = serde_json::from_value::<EntityId>(r.b.clone()).unwrap() else {
        panic!()
    };
    ok(&mut d, json!({"type": "erase", "ids": [r.b]}));
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["joints"].as_array().unwrap().len(), 1, "{detail}");
}

#[test]
fn a_batch_refers_to_joints_made_earlier_in_scene_joints() {
    let mut d = doc();
    let r = rig(&mut d);
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_joint", "kind": "revolute", "a": r.a, "b": r.b, "point": [3, 0.5, 0], "axis": [0, 0, 1]}),
        json!({"type": "add_scene", "name": "Ref", "joints": ["$0:j0", r.ja]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, true).unwrap();
    let made = serde_json::to_value(&outcomes[0]).unwrap()["changes"]["joints"][0].clone();
    let scenes = ask(&d, json!({"type": "scenes"}));
    let mut expected = vec![r.ja.clone(), made];
    expected.sort_by_key(|j| j.as_str().unwrap()[1..].parse::<u64>().unwrap());
    assert_eq!(scenes["scenes"][0]["joints"], json!(expected), "{scenes}");
    // A reference of the wrong kind in the list is refused before anything runs.
    let bad = BatchCommand::parse(json!({"type": "add_scene", "joints": ["$0:f0"]}), 1);
    assert!(bad.is_err());
}

#[test]
fn scene_joint_values_are_saved_at_schema_29_and_older_files_stay_older() {
    let mut d = doc();
    let r = rig(&mut d);
    drive(&mut d, &r.ja, 25.0);
    ok(
        &mut d,
        json!({"type": "add_scene", "name": "J", "joints": true}),
    );
    let path = scratch("joints.skr");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::SCENE_JOINTS_SCHEMA_VERSION
    );
    assert_eq!(session::file::SCENE_JOINTS_SCHEMA_VERSION, 29);
    let mut e = doc();
    ok(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    assert_eq!(e.scene(), d.scene());
    let detail = ask(&e, json!({"type": "scenes"}));
    assert_eq!(detail["scenes"][0]["joints"], json!([r.ja, r.jb]));

    // A scene without joint values writes none, and stays at the joints' schema.
    let mut plain = doc();
    rig(&mut plain);
    ok(&mut plain, json!({"type": "add_scene"}));
    let old = scratch("plain.skr");
    ok(
        &mut plain,
        json!({"type": "save", "path": old.to_str().unwrap()}),
    );
    assert_eq!(
        stored(&old)["schema_version"],
        session::file::JOINTS_SCHEMA_VERSION
    );
    assert!(stored(&old)["model"]["pages"][0].get("joints").is_none());
}

#[test]
fn a_motion_interpolates_joint_values_and_solves_every_frame() {
    let mut d = doc();
    let r = rig(&mut d);
    drive(&mut d, &r.ja, 60.0);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": [r.ja]})));
    drive(&mut d, &r.ja, 0.0);
    let id: PageId = serde_json::from_value(s.clone()).unwrap();
    let revision = d.revision();
    let motion = d.motion_between(None, id).unwrap();
    assert_eq!(d.revision(), revision);
    // The joint moves from 0 to 60 degrees, and the part it turns is not interpolated as a
    // transform.
    assert_eq!(motion.joints.len(), 1);
    assert_eq!(
        motion.joints[0].0,
        serde_json::from_value::<JointId>(r.ja.clone()).unwrap()
    );
    assert!(motion.moves.is_empty());
    assert!(!motion.is_still());
    for (t, degrees) in [(0.0, 0.0), (0.5, 30.0), (1.0, 60.0)] {
        let frame = motion.scene_at(d.scene(), t);
        let EntityId::Instance(a) = serde_json::from_value::<EntityId>(r.a.clone()).unwrap() else {
            panic!()
        };
        let x = frame.root().instance(a).unwrap().transform.matrix3 * glam::DVec3::X;
        assert!(
            (x.y.atan2(x.x).to_degrees() - degrees).abs() < 1e-6,
            "t {t}: {x:?}"
        );
    }
    assert!(turned(&d, &r.a, 0.0), "the document is not changed");
    // Going there leaves the document at the end of the motion.
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    assert!(turned(&d, &r.a, 60.0));
}

#[test]
fn a_motion_solves_couplings_for_the_joints_it_does_not_store() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(
        &mut d,
        json!({"type": "add_coupling", "driver": r.ja, "driven": r.jb, "ratio": 2.0}),
    );
    drive(&mut d, &r.ja, 40.0);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": [r.ja]})));
    drive(&mut d, &r.ja, 0.0);
    assert!(turned(&d, &r.b, 0.0));
    let id: PageId = serde_json::from_value(s).unwrap();
    let motion = d.motion_between(None, id).unwrap();
    // Both turning parts are the joints' and are solved, not interpolated.
    assert!(motion.moves.is_empty());
    let frame = motion.scene_at(d.scene(), 0.5);
    let EntityId::Instance(b) = serde_json::from_value::<EntityId>(r.b.clone()).unwrap() else {
        panic!()
    };
    let x = frame.root().instance(b).unwrap().transform.matrix3 * glam::DVec3::X;
    assert!((x.y.atan2(x.x).to_degrees() - 40.0).abs() < 1e-6, "{x:?}");
}

#[test]
fn render_frames_solve_joints_without_changing_the_document() {
    let mut d = doc();
    let r = rig(&mut d);
    let revision = d.revision();
    let frame = d
        .scene_with_joints(
            d.scene().clone(),
            &[(serde_json::from_value(r.ja.clone()).unwrap(), 45.0)],
        )
        .unwrap();
    let EntityId::Instance(a) = serde_json::from_value::<EntityId>(r.a.clone()).unwrap() else {
        panic!()
    };
    let x = frame.root().instance(a).unwrap().transform.matrix3 * glam::DVec3::X;
    assert!((x.y.atan2(x.x).to_degrees() - 45.0).abs() < 1e-6);
    assert_eq!(d.revision(), revision);
    assert!(turned(&d, &r.a, 0.0));
    // A joint that is not there is not_found; limits that are not met refuse.
    let err = d
        .scene_with_joints(d.scene().clone(), &[(JointId(9999), 1.0)])
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    ok(
        &mut d,
        json!({"type": "change_joint", "joint": r.ja, "limits": [-10, 50]}),
    );
    let err = d
        .scene_with_joints(
            d.scene().clone(),
            &[(serde_json::from_value(r.ja).unwrap(), 80.0)],
        )
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    assert!(err.message.contains("limits"), "{}", err.message);
}

#[test]
fn render_values_a_coupling_does_not_allow_are_refused() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(
        &mut d,
        json!({"type": "add_coupling", "driver": r.ja, "driven": r.jb, "ratio": 0.5}),
    );
    let ja: JointId = serde_json::from_value(r.ja.clone()).unwrap();
    let jb: JointId = serde_json::from_value(r.jb.clone()).unwrap();
    // The cam's 0 turns the crank back off its 90, whichever order they are given in.
    for values in [[(ja, 90.0), (jb, 0.0)], [(jb, 0.0), (ja, 90.0)]] {
        let err = d.scene_with_joints(d.scene().clone(), &values).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(
            err.message.contains("cannot stand at 90 degrees"),
            "{}",
            err.message
        );
    }
    // Values it allows render.
    let frame = d
        .scene_with_joints(d.scene().clone(), &[(jb, 45.0), (ja, 90.0)])
        .unwrap();
    assert!((frame.joint(jb).unwrap().value - 45f64.to_radians()).abs() < 1e-9);
    // A scene that stored values before the coupling came cannot be gone to now.
    let mut d = doc();
    let r = rig(&mut d);
    drive(&mut d, &r.ja, 90.0);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": true})));
    drive(&mut d, &r.ja, 0.0);
    ok(
        &mut d,
        json!({"type": "add_coupling", "driver": r.ja, "driven": r.jb, "ratio": 0.5, "offset": 0}),
    );
    let before = (transform(&d, &r.a), transform(&d, &r.b));
    let e = run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput, "{}", e.1);
    assert!(e.1.contains("cannot stand at 90 degrees"), "{}", e.1);
    assert_eq!((transform(&d, &r.a), transform(&d, &r.b)), before);
}

/// A joint through `add_joint`; its id.
fn joint_cmd(
    d: &mut Document,
    kind: &str,
    a: Option<&Value>,
    b: &Value,
    point: glam::DVec3,
    axis: glam::DVec3,
    reference: glam::DVec3,
) -> Value {
    let mut cmd = json!({"type": "add_joint", "kind": kind, "b": b, "point": point.to_array(),
        "axis": axis.to_array(), "reference": reference.to_array()});
    if let Some(a) = a {
        cmd["a"] = a.clone();
    }
    ok(d, cmd)["changes"]["joints"][0].clone()
}

/// A V6 along X: a crank turning about X, per cylinder a rod on a crank pin (pins 120 degrees
/// apart, shared in pairs) and a piston sliding along its bank's bore (+/-30 degrees from Z),
/// and a cam shaft coupled 1:2 to the crank. The crank's joint.
fn v6(d: &mut Document) -> Value {
    use glam::DVec3;
    use std::f64::consts::{PI, TAU};
    let (r, l) = (0.04, 0.14);
    let mut n = 0.0;
    let mut part = |d: &mut Document| {
        n += 1.0;
        ok(
            d,
            json!({"type": "add_box", "corner": [n, 5.0, 5.0], "size": [0.01, 0.01, 0.01]}),
        )["placed"][0]
            .clone()
    };
    let crank = part(d);
    let crank_joint = joint_cmd(d, "revolute", None, &crank, DVec3::ZERO, DVec3::X, DVec3::Z);
    for c in 0..6 {
        let (rod, piston) = (part(d), part(d));
        let throw = (c / 2) as f64 * TAU / 3.0;
        let bank = if c % 2 == 0 { PI / 6.0 } else { -PI / 6.0 };
        let base = DVec3::new(0.1 * c as f64, 0.0, 0.0);
        let pin = base + DVec3::new(0.0, r * throw.sin(), r * throw.cos());
        let dir = DVec3::new(0.0, bank.sin(), bank.cos());
        let rel = pin - base;
        let along = rel.dot(dir) + (l * l - rel.length_squared() + rel.dot(dir).powi(2)).sqrt();
        let wrist = base + dir * along;
        joint_cmd(d, "revolute", Some(&crank), &rod, pin, DVec3::X, DVec3::Z);
        joint_cmd(
            d,
            "revolute",
            Some(&rod),
            &piston,
            wrist,
            DVec3::X,
            DVec3::Z,
        );
        joint_cmd(d, "prismatic", None, &piston, wrist, dir, DVec3::X);
    }
    let cam = part(d);
    let cam_joint = joint_cmd(
        d,
        "revolute",
        None,
        &cam,
        DVec3::new(0.0, 0.1, 0.2),
        DVec3::X,
        DVec3::Z,
    );
    ok(
        d,
        json!({"type": "add_coupling", "driver": crank_joint, "driven": cam_joint, "ratio": 0.5}),
    );
    crank_joint
}

#[test]
fn transition_frames_solve_on_from_the_last_frame() {
    let mut d = doc();
    let crank = v6(&mut d);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "joints": [crank]})));
    drive(&mut d, &crank, 270.0);
    let id: PageId = serde_json::from_value(s.clone()).unwrap();
    let motion = d.motion_between(None, id).unwrap();
    // As the window does: the document goes to the scene, then the frames run from 270 to 0.
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    let poses = |scene: &kernel::Scene| -> Vec<DAffine3> {
        scene.root().instances().map(|(_, i)| i.transform).collect()
    };
    let steps = kernel::solve_steps_taken();
    let mut last: Option<kernel::Scene> = None;
    let mut frames = Vec::new();
    for k in 0..=20 {
        let t = f64::from(k) / 20.0;
        let frame = motion.scene_after(d.scene(), last.as_ref(), t);
        frames.push(poses(&frame));
        last = Some(frame);
    }
    let taken = kernel::solve_steps_taken() - steps;
    // The first frame drives 270 degrees from the document (27 steps); each later one 13.5
    // (2 steps). Driving each from the document would take some 290.
    assert!(taken <= 27 + 20 * 2, "{taken} steps");
    // The poses are the one-shot solve's.
    for k in [5u32, 10, 15, 20] {
        let t = f64::from(k) / 20.0;
        let once = poses(&motion.scene_at(d.scene(), t));
        for (a, b) in frames[k as usize].iter().zip(&once) {
            assert!(a.abs_diff_eq(*b, 1e-6), "frame {k}: {a:?} vs {b:?}");
        }
    }
}
