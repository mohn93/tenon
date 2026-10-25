//! Exporting SketchUp 2017 files: whole scenes written and read back, through JSON commands
//! and through the `skp` crate's own reader.

use std::collections::BTreeMap;
use std::path::PathBuf;

use glam::{DVec2, DVec3};
use kernel::{MeshId, Scene};
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

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../skp/tests/fixtures")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-skp-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Exports to `path` (removed first): the outcome's `export` report.
fn export_to(d: &mut Document, path: &std::path::Path) -> Value {
    let _ = std::fs::remove_file(path);
    run(d, json!({"type": "export", "path": path.to_str().unwrap()})).unwrap()["export"].clone()
}

/// A new document with the SketchUp file at `path` open.
fn open(path: &str) -> Document {
    let mut d = doc();
    run(&mut d, json!({"type": "open", "path": path})).unwrap_or_else(|e| panic!("{e:?}"));
    d.scene().validate().unwrap();
    d
}

/// What one mesh holds, for comparing scenes.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct MeshCounts {
    faces: usize,
    holes: usize,
    edges: usize,
    soft: usize,
    instances: usize,
    guides: usize,
}

fn counts(scene: &Scene, mesh: MeshId) -> MeshCounts {
    let m = scene.mesh(mesh).unwrap();
    MeshCounts {
        faces: m.faces().count(),
        holes: m.faces().map(|(_, f)| f.holes.len()).sum(),
        edges: m.edges().count(),
        soft: m.edges().filter(|(_, e)| e.soft).count(),
        instances: m.instances().count(),
        guides: m.guides().count(),
    }
}

/// Every non-empty definition by (name, kind), with what it holds and how often it is placed.
/// A blank name is written as the kind's, as SketchUp files carry no blank names.
fn definitions(scene: &Scene) -> BTreeMap<(String, String), (MeshCounts, usize)> {
    scene
        .definitions()
        .filter(|(_, def)| !def.mesh.is_empty())
        .map(|(id, def)| {
            let kind = format!("{:?}", def.kind);
            let name = match def.name.trim() {
                "" => kind.clone(),
                n => n.to_string(),
            };
            (
                (name, kind),
                (
                    counts(scene, MeshId::Definition(id)),
                    scene.instance_count(id),
                ),
            )
        })
        .collect()
}

/// Per mesh (the root as "", definitions as in `definitions`): each face's front and back
/// material names and each instance's material name, sorted.
type Paints = (Vec<(Option<String>, Option<String>)>, Vec<Option<String>>);

fn paints(scene: &Scene) -> BTreeMap<String, Paints> {
    let name = |m: Option<kernel::MaterialId>| m.map(|m| scene.material(m).unwrap().name.clone());
    let of = |m: &kernel::Model| {
        let mut faces: Vec<_> = m
            .faces()
            .map(|(_, f)| (name(f.front), name(f.back)))
            .collect();
        let mut instances: Vec<_> = m.instances().map(|(_, i)| name(i.material)).collect();
        faces.sort();
        instances.sort();
        (faces, instances)
    };
    let mut out = BTreeMap::new();
    out.insert(String::new(), of(scene.root()));
    for (_, def) in scene.definitions().filter(|(_, d)| !d.mesh.is_empty()) {
        let kind = format!("{:?}", def.kind);
        let name = match def.name.trim() {
            "" => kind.as_str(),
            n => n,
        };
        let key = format!("{name} ({kind})");
        out.insert(key, of(&def.mesh));
    }
    out
}

/// Every material by name: color, opacity, and the texture's bytes and size.
type MaterialSummary = ([u8; 3], f32, Option<(Vec<u8>, f64, f64)>);

fn materials(scene: &Scene) -> BTreeMap<String, MaterialSummary> {
    scene
        .materials()
        .map(|(_, m)| {
            (
                m.name.clone(),
                (
                    m.color,
                    m.opacity,
                    m.texture
                        .as_ref()
                        .map(|t| (t.image.encoded.clone(), t.width, t.height)),
                ),
            )
        })
        .collect()
}

fn assert_materials_match(got: &Scene, want: &Scene) {
    let (got, want) = (materials(got), materials(want));
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>()
    );
    for (name, (color, opacity, texture)) in &want {
        let (c, o, t) = &got[name];
        assert_eq!(c, color, "{name}");
        assert!((o - opacity).abs() < 1e-6, "{name}: {o} against {opacity}");
        match (t, texture) {
            (None, None) => {}
            (Some((bytes, w, h)), Some((want_bytes, want_w, want_h))) => {
                assert_eq!(bytes, want_bytes, "{name}");
                assert!(
                    (w - want_w).abs() < 1e-9 && (h - want_h).abs() < 1e-9,
                    "{name}"
                );
            }
            _ => panic!("{name}: texture {t:?} against {texture:?}"),
        }
    }
}

fn assert_bounds_match(got: &Scene, want: &Scene) {
    let (a, b) = (
        got.bounds(MeshId::Root).unwrap(),
        want.bounds(MeshId::Root).unwrap(),
    );
    assert!(
        a[0].abs_diff_eq(b[0], 1e-6) && a[1].abs_diff_eq(b[1], 1e-6),
        "{a:?} against {b:?}"
    );
}

/// The totals of what a scene holds over its root and every non-empty definition, each
/// counted once: definitions, instances, faces, edges, guides.
fn totals(scene: &Scene) -> [usize; 5] {
    let mut t = [0; 5];
    for (id, m) in scene.meshes() {
        if matches!(id, MeshId::Definition(_)) {
            if m.is_empty() {
                continue;
            }
            t[0] += 1;
        }
        t[1] += m.instances().count();
        t[2] += m.faces().count();
        t[3] += m.edges().count();
        t[4] += m.guides().count();
    }
    t
}

fn report_totals(report: &Value) -> [usize; 5] {
    ["definitions", "instances", "faces", "edges", "guides"]
        .map(|k| report[k].as_u64().unwrap_or(0) as usize)
}

#[test]
fn cupboard_round_trips() {
    let mut original = open(&fixture("cupboard.skp"));
    let path = scratch("cupboard.skp");
    let report = export_to(&mut original, &path);
    let back = open(path.to_str().unwrap());

    let (want, got) = (original.scene(), back.scene());
    assert_eq!(definitions(got), definitions(want));
    assert_eq!(counts(got, MeshId::Root), counts(want, MeshId::Root));
    assert_materials_match(got, want);
    assert_eq!(paints(got), paints(want));
    assert_bounds_match(got, want);

    // The knob and the hinge are each written once and placed twice.
    let defs = definitions(got);
    for name in ["knob", "Hinge"] {
        let (_, placed) = defs
            .iter()
            .find(|((n, _), _)| n == name)
            .unwrap_or_else(|| panic!("{name}: {defs:?}"))
            .1;
        assert_eq!(*placed, 2, "{name}");
    }

    // The report counts what was written, which is what reads back.
    assert_eq!(report_totals(&report), totals(got), "{report}");
    assert_eq!(report_totals(&report), totals(want), "{report}");
    assert_eq!(
        report["materials"].as_u64().unwrap() as usize,
        got.materials().count(),
        "{report}"
    );
    assert!(report.get("triangles").is_none_or(|t| t == 0), "{report}");
}

/// Every face and edge id of the root.
fn root_geometry(d: &Document) -> Vec<Value> {
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect()
}

/// The face an `add_*` outcome created.
fn created_face(outcome: &Value) -> Value {
    outcome["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().is_some_and(|s| s.starts_with('f')))
        .unwrap()
        .clone()
}

fn selected(d: &Document) -> Value {
    ask(d, json!({"type": "selection"}))["ids"][0].clone()
}

fn png_file(name: &str) -> PathBuf {
    let path = scratch(name);
    image::RgbaImage::from_fn(4, 2, |x, y| {
        image::Rgba([60 * x as u8, 100 * y as u8, 200, 255])
    })
    .save(&path)
    .unwrap();
    path
}

/// A model drawn with commands: a cylinder (soft sides) grouped and painted with a
/// translucent material; a box made a component, placed twice with the copy mirrored; a
/// tube pulled from a rectangle with a hole, loose in the root, one wall textured and its
/// texture moved and turned; a guide line and a guide point. Returns the textured wall's
/// id.
fn drawn_model(d: &mut Document) -> Value {
    let circle = run(
        d,
        json!({"type": "add_circle", "center": [10, 0, 0], "radius": 1}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": created_face(&circle), "distance": 2}),
    )
    .unwrap();
    run(d, json!({"type": "make_group", "ids": root_geometry(d)})).unwrap();
    let cylinder = selected(d);
    let glass = run(
        d,
        json!({"type": "add_material", "name": "Glass", "color": [180, 220, 255],
               "opacity": 0.4}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    run(
        d,
        json!({"type": "paint", "ids": [cylinder], "material": glass}),
    )
    .unwrap();

    let rect = run(
        d,
        json!({"type": "add_rectangle", "corner": [20, 0, 0], "opposite": [22, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": created_face(&rect), "distance": 1}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "make_component", "ids": root_geometry(d), "name": "Crate"}),
    )
    .unwrap();
    let crate_ = selected(d);
    run(
        d,
        json!({"type": "copy", "ids": [crate_], "offset": [5, 0, 0]}),
    )
    .unwrap();
    let copy = selected(d);
    run(
        d,
        json!({"type": "scale", "ids": [copy], "origin": [25, 0, 0], "factors": [-1, 1, 1]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "rotate", "ids": [copy], "center": [25, 0, 0], "angle": 90}),
    )
    .unwrap();

    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 4, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "add_rectangle", "corner": [1, 1, 0], "opposite": [3, 3, 0]}),
    )
    .unwrap();
    let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    let ring = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| (f["area"].as_f64().unwrap() - 12.0).abs() < 1e-9)
        .expect("the plate with a hole")["id"]
        .clone();
    run(d, json!({"type": "push_pull", "face": ring, "distance": 2})).unwrap();

    let png = png_file("drawn-tiles.png");
    let tiles = run(
        d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": png.to_str().unwrap(), "width": 2}}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    // The tube's outer wall along y = 0.
    let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    let wall = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, -1.0, 0.0]) && f["area"] == json!(8.0))
        .expect("the outer wall")["id"]
        .clone();
    run(
        d,
        json!({"type": "paint", "ids": [wall], "material": tiles}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "paint", "ids": [wall], "material": glass, "side": "back"}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "position_texture", "ids": [wall], "offset": [0.5, 0.25],
               "rotation_deg": 30}),
    )
    .unwrap();

    run(
        d,
        json!({"type": "add_guide_line", "point": [0, 0, 5], "direction": [1, 1, 0]}),
    )
    .unwrap();
    run(d, json!({"type": "add_guide_point", "position": [1, 2, 3]})).unwrap();
    d.scene().validate().unwrap();
    wall
}

/// Reads a written file with the `skp` crate's own reader.
fn read_skp(bytes: &[u8]) -> skp::scene::Model {
    use skp::container::Container;
    use skp::source::MemorySource;
    let src = MemorySource::new(bytes.to_vec());
    let c = Container::open(&src).expect("the container opens");
    let (m, _, stats) =
        skp::legacy::read_model_and_index(&src, &c, &skp::scene::ReadOptions::default())
            .expect("the file reads");
    assert_eq!(stats.lost_entities, 0, "{:?}", stats.notes);
    m
}

#[test]
fn a_drawn_model_round_trips() {
    let mut d = doc();
    let wall = drawn_model(&mut d);
    let path = scratch("drawn.skp");
    let report = export_to(&mut d, &path);
    let back = open(path.to_str().unwrap());
    let (want, got) = (d.scene(), back.scene());

    assert_eq!(definitions(got), definitions(want));
    let root = counts(got, MeshId::Root);
    assert_eq!(root, counts(want, MeshId::Root));
    assert_eq!(root.holes, 2, "the tube's top and bottom: {root:?}");
    assert_eq!(root.guides, 2, "{root:?}");
    let cylinder = definitions(got)
        .into_iter()
        .find(|((_, kind), _)| kind == "Group")
        .unwrap()
        .1
        .0;
    assert_eq!(cylinder.soft, 24, "{cylinder:?}");
    let crate_ = definitions(got)
        .into_iter()
        .find(|((name, _), _)| name == "Crate")
        .unwrap()
        .1;
    assert_eq!(crate_.1, 2, "the crate is placed twice");
    assert_materials_match(got, want);
    assert_eq!(paints(got), paints(want));
    assert_bounds_match(got, want);
    assert_eq!(report_totals(&report), totals(got), "{report}");
    assert_eq!(report["materials"], 2, "{report}");

    // Guides come back where they were.
    let guides: Vec<kernel::Guide> = got.root().guides().map(|(_, g)| *g).collect();
    let diagonal = DVec3::new(1.0, 1.0, 0.0).normalize();
    assert!(
        guides.iter().any(|g| matches!(g,
            kernel::Guide::Line { point, direction, .. }
                if point.abs_diff_eq(DVec3::new(0.0, 0.0, 5.0), 1e-9)
                    && direction.abs_diff_eq(diagonal, 1e-9))),
        "{guides:?}"
    );
    assert!(guides.iter().any(|g| matches!(g,
        kernel::Guide::Point { position, .. } if position.abs_diff_eq(DVec3::new(1.0, 2.0, 3.0), 1e-9))));

    // The mirrored crate stays mirrored.
    let dets: Vec<f64> = got
        .root()
        .instances()
        .map(|(_, i)| i.transform.matrix3.determinant())
        .collect();
    assert!(dets.iter().any(|d| (d + 1.0).abs() < 1e-9), "{dets:?}");
    // ... and turned a quarter: its transforms come back term for term.
    let transforms = |s: &Scene| {
        let mut t: Vec<[f64; 12]> = s
            .root()
            .instances()
            .map(|(_, i)| {
                let m = i.transform.matrix3.to_cols_array();
                let t = i.transform.translation.to_array();
                [
                    m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], t[0], t[1], t[2],
                ]
            })
            .collect();
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        t
    };
    let (a, b) = (transforms(got), transforms(want));
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert!(
            x.iter().zip(y).all(|(p, q)| (p - q).abs() < 1e-9),
            "{x:?} against {y:?}"
        );
    }
    assert!(
        b.iter()
            .any(|t| t[0].abs() < 1e-9 && (t[1].abs() - 1.0).abs() < 1e-9),
        "a quarter turn: {b:?}"
    );

    // The textured wall: its texture's placement is in the file, so each corner lies where
    // it did on the texture.
    let wall_id: kernel::EntityId = wall.as_str().unwrap().parse().unwrap();
    let kernel::EntityId::Face(wall_id) = wall_id else {
        panic!("{wall}");
    };
    let face = d.model().face(wall_id).unwrap().clone();
    let file = read_skp(&std::fs::read(&path).unwrap());
    let g = &file.root;
    let f = (0..g.faces())
        .find(|&f| g.uv_matrix(f).is_some())
        .expect("a pinned face");
    let n = g.normal(f);
    assert!(DVec3::from_array(n).abs_diff_eq(face.normal, 1e-9), "{n:?}");
    let map = g.uv_map(f);
    let mut ring = Vec::new();
    assert!(g.loop_ring(g.face_loop_ofs[f] as usize, &mut ring));
    assert_eq!(ring.len(), 4);
    for v in ring {
        let inches = g.vertex(v);
        let p = DVec3::from_array(inches) * 0.0254;
        let [u, v] = map.at(inches);
        let got = DVec2::new(u, v) * 0.0254;
        let want = face.texture_position(p);
        assert!(got.abs_diff_eq(want, 1e-6), "{p}: {got} against {want}");
    }
}

#[test]
fn export_protections() {
    let mut d = doc();
    drawn_model(&mut d);

    // The open model's own file.
    let model = scratch("model.dat");
    run(
        &mut d,
        json!({"type": "save", "path": model.to_str().unwrap()}),
    )
    .unwrap();
    let saved = std::fs::read(&model).unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": model.to_str().unwrap(), "format": "skp"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert_eq!(std::fs::read(&model).unwrap(), saved);

    // A file that is not a SketchUp file, unless overwrite.
    let other = scratch("notes.skp");
    std::fs::write(&other, b"my notes").unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": other.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert_eq!(std::fs::read(&other).unwrap(), b"my notes");
    run(
        &mut d,
        json!({"type": "export", "path": other.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    assert_ne!(std::fs::read(&other).unwrap(), b"my notes");

    // An existing SketchUp file is replaced.
    let existing = scratch("existing.skp");
    std::fs::copy(fixture("cupboard.skp"), &existing).unwrap();
    run(
        &mut d,
        json!({"type": "export", "path": existing.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        std::fs::read(&other).unwrap(),
        "the same model gives the same bytes"
    );

    // Nothing to write.
    let mut empty = doc();
    let (kind, message) = run(
        &mut empty,
        json!({"type": "export", "path": scratch("empty.skp").to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("nothing to export"), "{message}");
    // Guides alone are something.
    run(
        &mut empty,
        json!({"type": "add_guide_point", "position": [1, 2, 3]}),
    )
    .unwrap();
    let report = export_to(&mut empty, &scratch("guide.skp"));
    assert_eq!(report["guides"], 1, "{report}");
}

#[test]
fn warnings() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1}),
    )
    .unwrap();
    let lamp = run(
        &mut d,
        json!({"type": "add_material", "name": "Lamp", "color": [255, 240, 200],
               "emission": 2}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "paint", "ids": [face], "material": lamp}),
    )
    .unwrap();
    let report = export_to(&mut d, &scratch("warnings.skp"));
    let warnings: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(
        warnings,
        [
            "1 material glows; SketchUp files cannot store that",
            "1 curve was written as separate edges",
        ],
        "{report}"
    );

    // Without either, no warnings at all.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let report = export_to(&mut d, &scratch("quiet.skp"));
    assert!(report.get("warnings").is_none(), "{report}");
    assert_eq!(report["faces"], 1, "{report}");
    assert_eq!(report["edges"], 4, "{report}");
    assert!(report.get("definitions").is_none(), "{report}");
}

#[test]
fn scenes_are_not_written_and_say_so() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "add_scene"})).unwrap();
    let report = export_to(&mut d, &scratch("one-scene.skp"));
    assert_eq!(
        report["warnings"],
        json!(["1 scene was not written"]),
        "{report}"
    );
    run(&mut d, json!({"type": "add_scene"})).unwrap();
    run(&mut d, json!({"type": "add_scene"})).unwrap();
    let report = export_to(&mut d, &scratch("three-scenes.skp"));
    assert_eq!(
        report["warnings"],
        json!(["3 scenes were not written"]),
        "{report}"
    );
}

#[test]
fn empty_groups_are_left_out_with_a_warning() {
    // A face beside a group whose contents were all erased: the group stays, empty.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let rect = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [3, 0, 0], "opposite": [4, 1, 0]}),
    )
    .unwrap();
    let created: Vec<&Value> = rect["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| !id.as_str().unwrap().starts_with('v'))
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": created})).unwrap();
    let group = selected(&d);
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inside = root_geometry(&d);
    run(&mut d, json!({"type": "erase", "ids": inside})).unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entities", "kinds": ["instance"]}))["instances"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the empty group stays"
    );
    let report = export_to(&mut d, &scratch("empty-group.skp"));
    assert_eq!(
        report["warnings"],
        json!(["1 empty group or component was left out"]),
        "{report}"
    );
    assert!(report.get("instances").is_none(), "{report}");
    assert_eq!(report["faces"], 1, "{report}");
}

#[test]
fn copied_groups_get_names_of_their_own() {
    // A copied group gets a definition of its own under the same (blank) name; SketchUp
    // needs each definition named apart.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let ids = root_geometry(&d);
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = selected(&d);
    run(
        &mut d,
        json!({"type": "copy", "ids": [group], "offset": [2, 0, 0], "count": 2}),
    )
    .unwrap();
    let path = scratch("copied-groups.skp");
    let report = export_to(&mut d, &path);
    assert_eq!(report["definitions"], 3, "{report}");
    let back = open(path.to_str().unwrap());
    let mut names: Vec<String> = back
        .scene()
        .definitions()
        .map(|(_, def)| def.name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["Group", "Group 2", "Group 3"]);
}

#[test]
fn the_opened_original_is_kept_unless_overwrite() {
    // A model opened from a SketchUp file has no file of its own, but its original holds
    // what the export drops.
    let original = scratch("original.skp");
    std::fs::copy(fixture("cupboard.skp"), &original).unwrap();
    let before = std::fs::read(&original).unwrap();
    let mut d = open(original.to_str().unwrap());
    let dir = original.parent().unwrap();
    let dotted = dir
        .join("..")
        .join(dir.file_name().unwrap())
        .join("original.skp");
    for path in [&original, &dotted] {
        let (kind, message) = run(
            &mut d,
            json!({"type": "export", "path": path.to_str().unwrap()}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
        assert!(message.contains("opened from"), "{message}");
        assert!(message.contains("overwrite"), "{message}");
    }
    // Other formats are refused the same way.
    let (kind, _) = run(
        &mut d,
        json!({"type": "export", "path": original.to_str().unwrap(), "format": "obj"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(
        std::fs::read(&original).unwrap(),
        before,
        "the original is intact"
    );

    // Another path is fine, and overwrite replaces the original.
    export_to(&mut d, &scratch("beside-original.skp"));
    run(
        &mut d,
        json!({"type": "export", "path": original.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    assert_ne!(std::fs::read(&original).unwrap(), before);
    assert_eq!(
        std::fs::read(&original).unwrap(),
        std::fs::read(scratch_kept("beside-original.skp")).unwrap()
    );
}

/// A path in the scratch directory, left as it is.
fn scratch_kept(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-skp-export-{}", std::process::id()))
        .join(name)
}

#[cfg(unix)]
#[test]
fn under_api_root_a_link_is_refused() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let real = scratch("link-target.skp");
    std::fs::copy(fixture("cupboard.skp"), &real).unwrap();
    let before = std::fs::read(&real).unwrap();
    let link = scratch("link.skp");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let export = |refuse_links| Command::Export {
        path: link.to_str().unwrap().to_string(),
        format: None,
        overwrite: false,
        refuse_links,
        scale: None,
        selection: false,
    };
    let e = d.execute(export(true)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput, "{}", e.message);
    assert!(e.message.contains("symbolic link"), "{}", e.message);
    assert_eq!(std::fs::read(&real).unwrap(), before);
    // Without the API root, a link to a SketchUp file is judged by that file.
    d.execute(export(false)).unwrap();
    assert!(std::fs::symlink_metadata(&link).is_ok());
}

#[test]
fn only_empty_groups_is_nothing_to_export() {
    let mut d = doc();
    let rect = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [3, 0, 0], "opposite": [4, 1, 0]}),
    )
    .unwrap();
    let created: Vec<&Value> = rect["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| !id.as_str().unwrap().starts_with('v'))
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": created})).unwrap();
    let group = selected(&d);
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inside = root_geometry(&d);
    run(&mut d, json!({"type": "erase", "ids": inside})).unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": scratch("hollow.skp").to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(
        message,
        "nothing to export: only empty groups or components"
    );
}

/// A rectangle on the ground: its face and its four edges.
fn rectangle(d: &mut Document, corner: [f64; 2], opposite: [f64; 2]) -> (String, Vec<String>) {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [corner[0], corner[1], 0],
               "opposite": [opposite[0], opposite[1], 0]}),
    )
    .unwrap();
    let created: Vec<String> = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let face = created
        .iter()
        .find(|id| id.starts_with('f'))
        .unwrap()
        .clone();
    let edges = created
        .into_iter()
        .filter(|id| id.starts_with('e'))
        .collect();
    (face, edges)
}

fn add_tag(d: &mut Document, v: Value) -> String {
    let mut v = v;
    v["type"] = json!("add_tag");
    run(d, v).unwrap()["changes"]["tags"][0]
        .as_str()
        .unwrap()
        .to_string()
}

fn set_tag(d: &mut Document, ids: &[&str], tag: &str) {
    run(d, json!({"type": "set_tag", "ids": ids, "tag": tag})).unwrap();
}

/// Each tag's name, colour, visibility and count.
fn tag_summary(d: &Document) -> Vec<(String, Value, bool, u64)> {
    ask(d, json!({"type": "tags"}))["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["color"].clone(),
                t["visible"].as_bool().unwrap(),
                t["count"].as_u64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn tags_round_trip() {
    let mut d = doc();
    let walls = add_tag(&mut d, json!({"name": "Walls", "color": [200, 30, 40]}));
    let roof = add_tag(&mut d, json!({"name": "Roof", "color": [10, 20, 30]}));
    let layer0 = add_tag(&mut d, json!({"name": "Layer0", "color": [5, 6, 7]}));
    let spare = add_tag(&mut d, json!({"name": "Spare", "color": [1, 1, 1]}));
    run(
        &mut d,
        json!({"type": "change_tag", "tag": spare, "visible": false}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "change_tag", "tag": roof, "visible": false}),
    )
    .unwrap();

    // In the root: a face and one of its edges on Walls; a guide point on Walls and a guide
    // line on the tag named Layer0.
    let (face, edges) = rectangle(&mut d, [0.0, 0.0], [2.0, 2.0]);
    set_tag(&mut d, &[&face, &edges[0]], &walls);
    let point = run(
        &mut d,
        json!({"type": "add_guide_point", "position": [1, 1, 1]}),
    )
    .unwrap()["changes"]["created"][0]
        .as_str()
        .unwrap()
        .to_string();
    set_tag(&mut d, &[&point], &walls);
    let line = run(
        &mut d,
        json!({"type": "add_guide_line", "point": [0, 0, 3], "direction": [1, 0, 0]}),
    )
    .unwrap()["changes"]["created"][0]
        .as_str()
        .unwrap()
        .to_string();
    set_tag(&mut d, &[&line], &layer0);

    // A group on the hidden Roof, its face on Walls.
    let (inner, _) = rectangle(&mut d, [5.0, 0.0], [7.0, 2.0]);
    let group =
        run(&mut d, json!({"type": "make_group", "ids": [inner]})).unwrap()["changes"]["created"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .find(|id| id.starts_with('i'))
            .unwrap();
    set_tag(&mut d, &[&group], &roof);
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inner_face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    set_tag(&mut d, &[&inner_face], &walls);
    run(&mut d, json!({"type": "close_instance"})).unwrap();

    let path = scratch("tags.skp");
    let report = export_to(&mut d, &path);
    // Every tag is written, Spare (on nothing, hidden) too.
    assert_eq!(report["tags"], 4, "{report}");

    // Read back by the reader: Layer0 first, then the tags, the user's "Layer0" renamed.
    let index = {
        use skp::container::Container;
        use skp::legacy::read_model_and_index;
        use skp::scene::ReadOptions;
        let src = skp::source::MemorySource::new(std::fs::read(&path).unwrap());
        let c = Container::open(&src).unwrap();
        read_model_and_index(&src, &c, &ReadOptions::default())
            .unwrap()
            .1
    };
    let layers: Vec<(&str, bool)> = index
        .layers
        .iter()
        .map(|l| (l.name.as_str(), l.hidden))
        .collect();
    assert_eq!(
        layers,
        [
            ("Layer0", false),
            ("Walls", false),
            ("Roof", true),
            ("Layer0 2", false),
            ("Spare", true)
        ]
    );

    // Opened: the same tags, colours, visibility and entities on them.
    let back = open(path.to_str().unwrap());
    assert_eq!(
        tag_summary(&back),
        [
            ("Walls".to_string(), json!([200, 30, 40]), true, 4),
            ("Roof".to_string(), json!([10, 20, 30]), false, 1),
            ("Layer0 2".to_string(), json!([5, 6, 7]), true, 1),
            ("Spare".to_string(), json!([1, 1, 1]), false, 0),
        ]
    );
    let walls_back = ask(&back, json!({"type": "tags"}))["tags"][0]["id"].clone();
    let faces = ask(&back, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(faces.len(), 1);
    let id = faces[0]["id"].as_str().unwrap();
    assert_eq!(
        ask(&back, json!({"type": "entity", "id": id}))["tag"],
        walls_back
    );

    // Exported again and opened, the same tags.
    let mut back = back;
    let again = scratch("tags-again.skp");
    export_to(&mut back, &again);
    assert_eq!(
        tag_summary(&open(again.to_str().unwrap())),
        tag_summary(&back)
    );
}
