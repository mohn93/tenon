//! 3D text: letters as faces with holes in a new group, extruded with push/pull semantics,
//! with missing glyphs reported and the text limits enforced.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use kernel::{EntityId, Model, VertexId};
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Outcome, Text3dReport};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Outcome, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd).map_err(|e| (e.kind, e.message))
}

fn text(d: &mut Document, v: Value) -> Text3dReport {
    let mut v = v;
    v["type"] = json!("add_3d_text");
    run(d, v).expect("3D text").text_3d.expect("a text report")
}

/// The group's mesh.
fn mesh(d: &Document, r: &Text3dReport) -> Model {
    d.scene()
        .definition(r.definition)
        .expect("the definition")
        .mesh
        .clone()
}

/// How many faces use each side (vertex pair) of the mesh.
fn side_uses(m: &Model) -> HashMap<(VertexId, VertexId), usize> {
    let mut uses = HashMap::new();
    for (_, f) in m.faces() {
        for ring in std::iter::once(&f.outer).chain(&f.holes) {
            for (i, a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                *uses.entry((*a.min(&b), *a.max(&b))).or_insert(0) += 1;
            }
        }
    }
    uses
}

#[test]
fn letters_are_faces_with_holes_in_a_new_selected_group() {
    let mut d = doc();
    let r = text(
        &mut d,
        json!({"text": "Hello o8%", "position": [1.0, 2.0, 0.5], "height": 1.0}),
    );
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    d.scene().validate().unwrap();
    let EntityId::Instance(instance) = r.instance else {
        panic!("an instance: {:?}", r.instance)
    };
    assert_eq!(
        d.selection().iter().copied().collect::<Vec<_>>(),
        [r.instance]
    );
    let placed = d.model().instance(instance).expect("placed in the root");
    assert_eq!(placed.definition, r.definition);
    assert_eq!(placed.transform.translation, DVec3::new(1.0, 2.0, 0.5));
    let def = d.scene().definition(r.definition).unwrap();
    assert_eq!(def.name, "3D Text: Hello o8%");
    let m = mesh(&d, &r);
    // H, e, l, l, o, o, 8 and the two rings and slash of %.
    assert_eq!(m.faces().count(), 10);
    assert_eq!(r.faces, 10);
    let holes: usize = m.faces().map(|(_, f)| f.holes.len()).sum();
    // e, o, o: one each; 8 and %: two each.
    assert_eq!(holes, 7);
    for (_, f) in m.faces() {
        assert!(f.normal.abs_diff_eq(DVec3::Z, 1e-9), "{}", f.normal);
    }
    // Every loop is closed: each side bounds exactly one face.
    assert!(side_uses(&m).values().all(|&n| n == 1));
    assert_eq!(m.edges().count(), side_uses(&m).len());
    // Capital letters are the height tall, standing on the baseline.
    let ys: Vec<f64> = m.vertices().map(|(_, v)| v.pos.y).collect();
    let top = ys.iter().copied().fold(f64::MIN, f64::max);
    assert!((top - 1.0).abs() < 0.05, "{top}");
}

#[test]
fn depth_extrudes_each_letter_into_a_closed_solid_in_one_step() {
    let mut d = doc();
    let r = text(
        &mut d,
        json!({"text": "Hello o8%", "position": [0.0, 0.0, 0.0], "height": 0.5, "depth": 0.2}),
    );
    d.scene().validate().unwrap();
    let m = mesh(&d, &r);
    // Closed: every side is shared by exactly two faces.
    let uses = side_uses(&m);
    assert!(uses.values().all(|&n| n == 2), "open or doubled sides");
    // Each letter face gives a base and a top: 20 of them, the rest are sides.
    let flat = m
        .faces()
        .filter(|(_, f)| f.normal.z.abs() > 1.0 - 1e-9)
        .count();
    assert_eq!(flat, 20);
    let up = m.faces().filter(|(_, f)| f.normal.z > 0.5).count();
    assert_eq!(up, 10, "tops face up, bases down");
    let heights: Vec<f64> = m.vertices().map(|(_, v)| v.pos.z).collect();
    assert!(
        heights
            .iter()
            .all(|z| z.abs() < 1e-9 || (z - 0.2).abs() < 1e-9)
    );
    assert_eq!(r.faces, m.faces().count());
    // One undo step takes it all away.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(d.model().instances().next().is_none());
    assert!(d.scene().definition(r.definition).is_none());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(mesh(&d, &r).faces().count(), m.faces().count());
}

/// The faces' normals, rounded, sorted: the solid's shape without its ids.
fn normals(m: &Model) -> Vec<[i64; 3]> {
    let mut n: Vec<[i64; 3]> = m
        .faces()
        .map(|(_, f)| (f.normal * 1e6).round().as_i64vec3().to_array())
        .collect();
    n.sort();
    n
}

#[test]
fn depth_makes_what_push_pull_makes_of_the_flat_letters() {
    let mut d = doc();
    let solid = text(
        &mut d,
        json!({"text": "o8%", "position": [0.0, 0.0, 0.0], "height": 1.0, "depth": 0.3}),
    );
    let flat = text(
        &mut d,
        json!({"text": "o8%", "position": [0.0, 0.0, 0.0], "height": 1.0}),
    );
    run(
        &mut d,
        json!({"type": "open_instance", "id": flat.instance}),
    )
    .unwrap();
    let letters: Vec<_> = d.model().faces().map(|(id, _)| id).collect();
    for face in letters {
        run(
            &mut d,
            json!({"type": "push_pull", "face": EntityId::Face(face), "distance": 0.3}),
        )
        .unwrap();
    }
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let (a, b) = (mesh(&d, &solid), mesh(&d, &flat));
    assert_eq!(a.faces().count(), b.faces().count());
    assert_eq!(a.edges().count(), b.edges().count());
    assert_eq!(a.vertices().count(), b.vertices().count());
    assert_eq!(normals(&a), normals(&b));
}

#[test]
fn lines_step_down_by_1_2_heights() {
    let mut d = doc();
    let r = text(
        &mut d,
        json!({"text": "H\nH\r\nH", "position": [0.0, 0.0, 0.0], "height": 2.0}),
    );
    let m = mesh(&d, &r);
    assert_eq!(m.faces().count(), 3);
    let ys: Vec<f64> = m.vertices().map(|(_, v)| v.pos.y).collect();
    let low = ys.iter().copied().fold(f64::MAX, f64::min);
    assert!((low + 2.0 * 1.2 * 2.0).abs() < 1e-9, "{low}");
    let xs: Vec<f64> = m.vertices().map(|(_, v)| v.pos.x).collect();
    let right = xs.iter().copied().fold(f64::MIN, f64::max);
    assert!(right < 2.0, "each line starts again at the left: {right}");
}

#[test]
fn characters_without_glyphs_are_skipped_and_reported() {
    let mut d = doc();
    let o = run(
        &mut d,
        json!({"type": "add_3d_text", "text": "A \u{4E2D}B", "position": [0.0, 0.0, 0.0], "height": 1.0}),
    )
    .unwrap();
    let v = serde_json::to_value(&o).unwrap();
    let skipped = &v["text_3d"]["skipped"];
    assert_eq!(skipped.as_array().unwrap().len(), 1, "{v}");
    assert_eq!(skipped[0]["character"], "\u{4E2D}");
    assert_eq!(skipped[0]["index"], 2);
    assert!(skipped[0]["reason"].as_str().unwrap().contains("no glyph"));
    let r = o.text_3d.unwrap();
    assert_eq!(
        mesh(&d, &r).faces().count(),
        2,
        "A and B, the space only moves on"
    );
}

#[test]
fn outlines_only_are_edges_without_faces() {
    let mut d = doc();
    let r = text(
        &mut d,
        json!({"text": "o", "position": [0.0, 0.0, 0.0], "height": 1.0, "filled": false}),
    );
    let m = mesh(&d, &r);
    assert_eq!(m.faces().count(), 0);
    assert!(m.edges().count() >= 2 * 8, "two rings of edges");
    assert_eq!(r.faces, 0);
    let err = run(
        &mut d,
        json!({"type": "add_3d_text", "text": "o", "position": [0.0, 0.0, 0.0], "height": 1.0,
               "filled": false, "depth": 0.1}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
}

#[test]
fn text_height_depth_and_extent_limits_are_refused() {
    let mut d = doc();
    let long = "a".repeat(1025);
    let many_lines = vec!["a"; 65].join("\n");
    let refused = [
        json!({"text": "", "position": [0.0, 0.0, 0.0], "height": 1.0}),
        json!({"text": "  \n ", "position": [0.0, 0.0, 0.0], "height": 1.0}),
        json!({"text": long, "position": [0.0, 0.0, 0.0], "height": 1.0}),
        json!({"text": many_lines, "position": [0.0, 0.0, 0.0], "height": 1.0}),
        json!({"text": "a", "position": [0.0, 0.0, 0.0], "height": 0.0}),
        json!({"text": "a", "position": [0.0, 0.0, 0.0], "height": -1.0}),
        json!({"text": "a", "position": [0.0, 0.0, 0.0], "height": 2.0e6}),
        json!({"text": "a", "position": [0.0, 0.0, 0.0], "height": 1.0, "depth": -0.1}),
        json!({"text": "a", "position": [2.0e6, 0.0, 0.0], "height": 1.0}),
        json!({"text": "abc", "position": [999_999.0, 0.0, 0.0], "height": 1.0}),
        json!({"text": "a", "position": [0.0, 0.0, 999_999.5], "height": 1.0, "depth": 1.0}),
        json!({"text": "\u{4E2D}", "position": [0.0, 0.0, 0.0], "height": 1.0}),
    ];
    for mut v in refused {
        v["type"] = json!("add_3d_text");
        let err = run(&mut d, v.clone()).expect_err(&v.to_string());
        assert_eq!(err.0, ErrorKind::InvalidInput, "{v}: {}", err.1);
    }
    assert_eq!(d.revision(), 0, "nothing changed");
    assert!(d.scene().definitions().next().is_none());
    // The limits themselves are fine.
    let at_limit = vec!["a"; 64].join("\n");
    text(
        &mut d,
        json!({"text": at_limit, "position": [0.0, 0.0, 0.0], "height": 0.001}),
    );
}

#[test]
fn text_is_drawn_in_the_open_context_and_in_atomic_batches() {
    let mut d = doc();
    let first = text(
        &mut d,
        json!({"text": "I", "position": [0.0, 0.0, 0.0], "height": 1.0}),
    );
    run(
        &mut d,
        json!({"type": "open_instance", "id": first.instance}),
    )
    .unwrap();
    let inner = text(
        &mut d,
        json!({"text": "J", "position": [5.0, 0.0, 0.0], "height": 1.0}),
    );
    let placed: Vec<_> = mesh(&d, &first).instances().map(|(id, _)| id).collect();
    assert_eq!(placed.len(), 1, "the new group is inside the open one");
    assert_eq!(EntityId::Instance(placed[0]), inner.instance);
    run(&mut d, json!({"type": "close_instance"})).unwrap();

    let commands: Vec<BatchCommand> = [
        json!({"type": "add_3d_text", "text": "K", "position": [0.0, 5.0, 0.0], "height": 1.0, "depth": 0.1}),
        json!({"type": "move", "ids": ["$0:i0"], "offset": [1.0, 0.0, 0.0]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, true).unwrap();
    assert!(outcomes[0].text_3d.is_some());
}

/// The meshes of the group's own definition and of the groups nested in it.
fn glyph_meshes(d: &Document, r: &Text3dReport) -> Vec<Model> {
    let main = mesh(d, r);
    let mut out = vec![main.clone()];
    for (_, i) in main.instances() {
        out.push(d.scene().definition(i.definition).unwrap().mesh.clone());
    }
    out
}

#[test]
fn glyphs_that_touch_are_separate_closed_solids() {
    for text_in in ["__", "\u{2014}\u{2014}"] {
        let mut d = doc();
        let r = text(
            &mut d,
            json!({"text": text_in, "position": [0.0, 0.0, 0.0], "height": 1.0, "depth": 0.2}),
        );
        assert!(r.skipped.is_empty(), "{text_in}: {:?}", r.skipped);
        d.scene().validate().unwrap();
        let meshes = glyph_meshes(&d, &r);
        let solids: Vec<&Model> = meshes.iter().filter(|m| m.faces().count() > 0).collect();
        assert_eq!(solids.len(), 2, "{text_in}: one solid per glyph");
        for m in solids {
            assert_eq!(m.faces().count(), 6, "{text_in}");
            assert!(side_uses(m).values().all(|&n| n == 2), "{text_in}");
        }
        assert_eq!(r.faces, 12, "{text_in}");
    }
}

#[test]
fn flat_touching_glyphs_share_no_edge() {
    let mut d = doc();
    let r = text(
        &mut d,
        json!({"text": "__", "position": [0.0, 0.0, 0.0], "height": 1.0}),
    );
    let meshes = glyph_meshes(&d, &r);
    let solids: Vec<&Model> = meshes.iter().filter(|m| m.faces().count() > 0).collect();
    assert_eq!(solids.len(), 2);
    for m in &solids {
        assert_eq!(m.faces().count(), 1);
        assert!(side_uses(m).values().all(|&n| n == 1));
    }
}

#[test]
fn glyphs_that_touch_nothing_stay_in_the_main_group() {
    let mut d = doc();
    let r = text(
        &mut d,
        json!({"text": "Hi", "position": [0.0, 0.0, 0.0], "height": 1.0}),
    );
    let m = mesh(&d, &r);
    assert!(m.instances().next().is_none());
    // H, i and the dot of the i.
    assert_eq!(m.faces().count(), 3);
}
