//! Scenes that store section planes: the active plane of each context and the section display,
//! gone to in one undo step, kept in files and dropped with the plane.

use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

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
        .join(format!("session-scene-sections-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

/// A plane added in the open context; returns its id.
fn plane(d: &mut Document, z: f64, activate: bool) -> String {
    let out = ok(
        d,
        json!({"type": "add_section_plane", "point": [0, 0, z], "normal": [0, 0, 1], "activate": activate}),
    );
    out["changes"]["created"][0].as_str().unwrap().to_string()
}

/// The ids of the planes active in the root and in the group `group` (None: the root only).
fn active_in_context(d: &Document) -> Option<String> {
    ask(d, json!({"type": "section_planes"}))["planes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["active"] == true)
        .map(|p| p["id"].as_str().unwrap().to_string())
}

struct Rig {
    root_a: String,
    root_b: String,
    group: String,
    inner: String,
}

/// A group holding the active plane `inner`, and in the root planes `root_a` (active) and
/// `root_b`.
fn rig(d: &mut Document) -> Rig {
    let group = ok(
        d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1]}),
    )["placed"][0]
        .as_str()
        .unwrap()
        .to_string();
    ok(d, json!({"type": "open_instance", "id": group}));
    let inner = plane(d, 0.5, true);
    ok(d, json!({"type": "close_instance"}));
    let root_b = plane(d, 5.0, false);
    let root_a = plane(d, 2.0, true);
    Rig {
        root_a,
        root_b,
        group,
        inner,
    }
}

fn inner_active(d: &mut Document, r: &Rig) -> Option<String> {
    ok(d, json!({"type": "open_instance", "id": r.group}));
    let a = active_in_context(d);
    ok(d, json!({"type": "close_instance"}));
    a
}

fn scene_id(out: &Value) -> String {
    out["changes"]["scenes"][0].as_str().unwrap().to_string()
}

fn display(d: &Document) -> Value {
    ask(d, json!({"type": "section_display"}))
}

#[test]
fn going_to_a_scene_restores_the_active_planes_of_every_context_and_the_display() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(
        &mut d,
        json!({"type": "set_section_display", "cuts": false, "fill_color": "material"}),
    );
    let s = scene_id(&ok(
        &mut d,
        json!({"type": "add_scene", "name": "Cut", "sections": true}),
    ));
    // Change everything the scene stores.
    ok(
        &mut d,
        json!({"type": "activate_section_plane", "plane": r.root_b}),
    );
    ok(&mut d, json!({"type": "open_instance", "id": r.group}));
    ok(&mut d, json!({"type": "deactivate_section"}));
    ok(&mut d, json!({"type": "close_instance"}));
    ok(
        &mut d,
        json!({"type": "set_section_display", "cuts": true, "fill": false, "fill_color": "dark"}),
    );
    assert_eq!(active_in_context(&d), Some(r.root_b.clone()));
    assert_eq!(inner_active(&mut d, &r), None);

    let steps = ask(&d, json!({"type": "history"}))["undo"]
        .as_array()
        .unwrap()
        .len();
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    assert_eq!(active_in_context(&d), Some(r.root_a.clone()));
    assert_eq!(inner_active(&mut d, &r), Some(r.inner.clone()));
    let shown = display(&d);
    assert_eq!(shown["cuts"], false);
    assert_eq!(shown["fill"], true);
    assert_eq!(shown["fill_color"], "material");

    // One undo step restores the planes (the display is a view setting, not undone).
    let history = ask(&d, json!({"type": "history"}))["undo"].clone();
    assert_eq!(history.as_array().unwrap().len(), steps + 1, "{history}");
    assert!(history.to_string().contains("Go to scene"), "{history}");
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(active_in_context(&d), Some(r.root_b.clone()));
    assert_eq!(inner_active(&mut d, &r), None);
    ok(&mut d, json!({"type": "redo"}));
    assert_eq!(active_in_context(&d), Some(r.root_a.clone()));
}

#[test]
fn a_scene_storing_no_plane_active_deactivates_the_others() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(&mut d, json!({"type": "deactivate_section"}));
    ok(&mut d, json!({"type": "open_instance", "id": r.group}));
    ok(&mut d, json!({"type": "deactivate_section"}));
    ok(&mut d, json!({"type": "close_instance"}));
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "sections": true})));
    ok(
        &mut d,
        json!({"type": "activate_section_plane", "plane": r.root_a}),
    );
    ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
    assert_eq!(active_in_context(&d), None);
    assert_eq!(inner_active(&mut d, &r), None);
}

#[test]
fn the_scene_commands_follow_the_stored_property_convention() {
    let mut d = doc();
    let r = rig(&mut d);
    let plain = scene_id(&ok(&mut d, json!({"type": "add_scene", "name": "Plain"})));
    let scenes = ask(&d, json!({"type": "scenes"}));
    assert!(scenes["scenes"][0].get("sections").is_none(), "{scenes}");
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": plain}))["sections"],
        Value::Null
    );

    // Going to a scene without them changes no plane.
    ok(&mut d, json!({"type": "go_to_scene", "scene": plain}));
    assert_eq!(active_in_context(&d), Some(r.root_a.clone()));

    // True stores them when absent; the scene then reports them.
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": plain, "sections": true}),
    );
    let detail = ask(&d, json!({"type": "scene", "scene": plain}));
    let stored_planes = detail["sections"]["active"].as_array().unwrap();
    assert_eq!(stored_planes.len(), 2, "{detail}");
    assert!(stored_planes.iter().any(|p| p["plane"] == json!(r.root_a)));
    assert!(stored_planes.iter().any(|p| p["plane"] == json!(r.inner)));
    assert_eq!(detail["sections"]["cuts"], true);
    assert_eq!(
        ask(&d, json!({"type": "scenes"}))["scenes"][0]["sections"],
        true
    );

    // True again keeps what was stored; update stores the current ones again.
    ok(
        &mut d,
        json!({"type": "activate_section_plane", "plane": r.root_b}),
    );
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": plain, "sections": true}),
    );
    let detail = ask(&d, json!({"type": "scene", "scene": plain}));
    assert!(
        detail["sections"]["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["plane"] == json!(r.root_a)),
        "{detail}"
    );
    ok(&mut d, json!({"type": "update_scene", "scene": plain}));
    let detail = ask(&d, json!({"type": "scene", "scene": plain}));
    assert!(
        detail["sections"]["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["plane"] == json!(r.root_b)),
        "{detail}"
    );

    // False drops them; update then leaves them off.
    ok(
        &mut d,
        json!({"type": "change_scene", "scene": plain, "sections": false}),
    );
    ok(&mut d, json!({"type": "update_scene", "scene": plain}));
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": plain}))["sections"],
        Value::Null
    );
    // Sections alone are a valid change.
    assert!(
        run(
            &mut d,
            json!({"type": "change_scene", "scene": plain, "sections": false})
        )
        .is_ok()
    );
    // add_scene with false stores nothing.
    let none = scene_id(&ok(&mut d, json!({"type": "add_scene", "sections": false})));
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": none}))["sections"],
        Value::Null
    );
}

#[test]
fn an_erased_plane_is_dropped_from_the_scene_and_undo_brings_it_back() {
    let mut d = doc();
    let r = rig(&mut d);
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "sections": true})));
    ok(&mut d, json!({"type": "erase", "ids": [r.root_a]}));
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    let planes = detail["sections"]["active"].as_array().unwrap();
    assert_eq!(planes.len(), 1, "{detail}");
    assert_eq!(planes[0]["plane"], json!(r.inner));
    ok(&mut d, json!({"type": "undo"}));
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["sections"]["active"].as_array().unwrap().len(), 2);
}

#[test]
fn files_with_scene_sections_use_the_new_schema_and_old_files_are_unchanged() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(
        &mut d,
        json!({"type": "add_scene", "name": "Plain", "sections": false}),
    );
    let old = scratch("plain.tnn");
    ok(
        &mut d,
        json!({"type": "save", "path": old.to_str().unwrap()}),
    );
    let v = stored(&old);
    assert!(
        v["schema_version"].as_u64().unwrap() < session::file::SCENE_SECTIONS_SCHEMA_VERSION as u64
    );
    assert!(v["model"]["pages"][0].get("sections").is_none());

    ok(
        &mut d,
        json!({"type": "set_section_display", "fill": false}),
    );
    let s = scene_id(&ok(
        &mut d,
        json!({"type": "add_scene", "name": "Cut", "sections": true}),
    ));
    let path = scratch("sections.tnn");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::SCENE_SECTIONS_SCHEMA_VERSION
    );
    assert_eq!(session::file::SCENE_SECTIONS_SCHEMA_VERSION, 30);

    let mut e = doc();
    ok(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    assert_eq!(e.scene(), d.scene());
    let detail = ask(&e, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["sections"]["fill"], false, "{detail}");
    assert_eq!(detail["sections"]["active"].as_array().unwrap().len(), 2);
    // Gone to after opening, it activates the stored planes.
    ok(&mut e, json!({"type": "deactivate_section"}));
    ok(&mut e, json!({"type": "go_to_scene", "scene": s}));
    assert_eq!(active_in_context(&e), Some(r.root_a));
    let _ = (r.root_b, r.group, r.inner);
}

#[test]
fn a_render_frame_of_a_scene_has_its_planes_active() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(
        &mut d,
        json!({"type": "set_section_display", "fill": false}),
    );
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "sections": true})));
    ok(
        &mut d,
        json!({"type": "activate_section_plane", "plane": r.root_b}),
    );
    let id: kernel::PageId = s.parse().unwrap();
    let (frame, shown) = d
        .frame_with_scene_sections(d.scene().clone(), id)
        .expect("the scene stores sections");
    assert!(!shown.fill);
    let active = frame
        .root()
        .active_section()
        .map(|s| format!("{}", kernel::EntityId::Section(s)));
    assert_eq!(active, Some(r.root_a));
    // The document itself is untouched.
    assert_eq!(active_in_context(&d), Some(r.root_b));
}

fn inner_planes_stored(d: &Document, s: &str) -> usize {
    ask(d, json!({"type": "scene", "scene": s}))["sections"]["active"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| !p["context"].is_null())
        .count()
}

#[test]
fn exploding_or_erasing_a_group_drops_its_plane_from_the_scene() {
    for how in ["explode", "erase"] {
        let mut d = doc();
        let r = rig(&mut d);
        let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "sections": true})));
        assert_eq!(inner_planes_stored(&d, &s), 1);
        ok(&mut d, json!({"type": how, "ids": [r.group]}));
        assert_eq!(inner_planes_stored(&d, &s), 0, "{how}");
        // Going to it does not fail.
        ok(&mut d, json!({"type": "go_to_scene", "scene": s}));
        while inner_planes_stored(&d, &s) == 0 {
            ok(&mut d, json!({"type": "undo"}));
        }
        assert_eq!(inner_planes_stored(&d, &s), 1, "{how} undone");
    }
}

#[test]
fn purging_a_definition_drops_its_plane_from_the_scene() {
    let mut d = doc();
    let r = rig(&mut d);
    ok(
        &mut d,
        json!({"type": "make_component", "ids": [r.group], "name": "C"}),
    );
    let inst = ask(&d, json!({"type": "outliner"}))["instances"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let s = scene_id(&ok(&mut d, json!({"type": "add_scene", "sections": true})));
    assert!(inner_planes_stored(&d, &s) >= 1);
    ok(&mut d, json!({"type": "erase", "ids": [inst]}));
    ok(&mut d, json!({"type": "purge_definitions"}));
    assert_eq!(inner_planes_stored(&d, &s), 0);
    while inner_planes_stored(&d, &s) == 0 {
        ok(&mut d, json!({"type": "undo"}));
    }
}

#[test]
fn a_file_whose_scene_stores_a_missing_plane_is_refused() {
    use std::io::Write;
    let mut d = doc();
    let _ = rig(&mut d);
    ok(&mut d, json!({"type": "add_scene", "sections": true}));
    let path = scratch("missing.tnn");
    ok(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    );
    let mut v = stored(&path);
    v["model"]["pages"][0]["sections"]["active"][0]["plane"] = json!(99999);
    let bad = scratch("missing-bad.tnn");
    let mut zin = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut zout = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        zout.start_file(&name, opts).unwrap();
        if name == "document.json" {
            zout.write_all(v.to_string().as_bytes()).unwrap();
        } else {
            std::io::copy(&mut f, &mut zout).unwrap();
        }
    }
    zout.finish().unwrap();
    let mut e = doc();
    let err = run(
        &mut e,
        json!({"type": "open", "path": bad.to_str().unwrap()}),
    );
    assert!(err.is_err(), "{err:?}");
}
