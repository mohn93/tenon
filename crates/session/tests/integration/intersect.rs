//! Intersect Faces, driven through JSON commands.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec2, DVec3};
use kernel::{EntityId, MeshId};
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn try_run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn run(d: &mut Document, v: Value) -> Value {
    try_run(d, v).unwrap()
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn face_ids(d: &Document) -> BTreeSet<String> {
    ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap().to_string())
        .collect()
}

/// Draws an axis-aligned box from `lo` to `hi` and returns the ids of its six faces.
fn boxed(d: &mut Document, lo: [f64; 3], hi: [f64; 3]) -> Vec<Value> {
    let before = face_ids(d);
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": lo, "opposite": [hi[0], hi[1], lo[2]]}),
    );
    let base = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let down = ask(d, json!({"type": "entity", "id": base}))["normal"][2]
        .as_f64()
        .unwrap()
        < 0.0;
    let height = hi[2] - lo[2];
    run(
        d,
        json!({"type": "push_pull", "face": base, "distance": if down { -height } else { height }}),
    );
    face_ids(d)
        .difference(&before)
        .map(|f| json!(f))
        .collect::<Vec<_>>()
}

fn intersect(d: &mut Document, ids: &[Value], with: &str) -> Value {
    run(
        d,
        json!({"type": "intersect_faces", "ids": ids, "with": with}),
    )
}

fn list_len(report: &Value, key: &str) -> usize {
    report["intersect"][key].as_array().unwrap().len()
}

fn validate(d: &Document) {
    d.scene().validate().unwrap();
}

/// Everything in the model's meshes, to compare before and after.
fn mesh_state(d: &Document, mesh: MeshId) -> String {
    let m = d.scene().mesh(mesh).unwrap();
    format!(
        "{:?}|{:?}|{:?}",
        m.vertices().collect::<Vec<_>>(),
        m.edges().collect::<Vec<_>>(),
        m.faces().collect::<Vec<_>>()
    )
}

fn center(m: &kernel::Model, f: kernel::FaceId) -> DVec3 {
    let face = m.face(f).unwrap();
    face.outer
        .iter()
        .map(|v| m.vertex(*v).unwrap().pos)
        .sum::<DVec3>()
        / face.outer.len() as f64
}

fn inside(p: DVec3, lo: f64, hi: f64) -> bool {
    p.min_element() > lo + 1e-6 && p.max_element() < hi - 1e-6
}

fn instance_id(out: &Value) -> String {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| id.starts_with('i'))
        .unwrap()
}

#[test]
fn two_boxes_split_along_their_crossing_and_the_overlap_can_be_erased() {
    let mut d = doc();
    let _a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [1.0; 3], [3.0; 3]);
    let faces_before = face_ids(&d).len();
    let out = intersect(&mut d, &b, "model");
    validate(&d);
    assert_eq!(list_len(&out, "split_faces"), 6, "{out}");
    assert!(list_len(&out, "added_edges") >= 6, "{out}");
    assert!(list_len(&out, "created_faces") > 6, "{out}");
    assert!(face_ids(&d).len() > faces_before);
    // Every id the report names exists.
    for key in ["added_edges", "created_faces"] {
        for id in out["intersect"][key].as_array().unwrap() {
            ask(&d, json!({"type": "entity", "id": id}));
        }
    }
    // Erase the pieces of each box that lie inside the other.
    let m = d.model();
    let doomed: Vec<Value> = m
        .faces()
        .filter(|(f, _)| {
            let c = center(m, *f);
            inside(c, 1.0, 3.0) || inside(c, 0.0, 2.0)
        })
        .map(|(f, _)| json!(EntityId::Face(f)))
        .collect();
    assert_eq!(doomed.len(), 6, "three pieces from each box");
    run(&mut d, json!({"type": "erase", "ids": doomed}));
    validate(&d);
}

#[test]
fn undo_restores_the_model_and_redo_repeats_it() {
    let mut d = doc();
    boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [1.0; 3], [3.0; 3]);
    let before = mesh_state(&d, MeshId::Root);
    intersect(&mut d, &b, "model");
    let after = mesh_state(&d, MeshId::Root);
    assert_ne!(before, after);
    let h = ask(&d, json!({"type": "history"}));
    assert_eq!(h["undo"][0], "intersect_faces");
    run(&mut d, json!({"type": "undo"}));
    assert_eq!(mesh_state(&d, MeshId::Root), before);
    run(&mut d, json!({"type": "redo"}));
    assert_eq!(mesh_state(&d, MeshId::Root), after);
}

#[test]
fn no_crossings_is_a_success_without_an_undo_step() {
    let mut d = doc();
    boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [5.0; 3], [6.0; 3]);
    let revision = d.revision();
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    let out = intersect(&mut d, &b, "model");
    assert_eq!(
        out["intersect"],
        json!({"added_edges": [], "split_faces": [], "created_faces": []})
    );
    assert_eq!(d.revision(), revision);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
}

#[test]
fn an_empty_selection_is_invalid_input() {
    let mut d = doc();
    boxed(&mut d, [0.0; 3], [2.0; 3]);
    run(&mut d, json!({"type": "select", "ids": []}));
    let (kind, message) =
        try_run(&mut d, json!({"type": "intersect_faces", "with": "model"})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(message, "Select the faces or groups to intersect");
}

#[test]
fn ids_default_to_the_selection() {
    let mut d = doc();
    boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [1.0; 3], [3.0; 3]);
    run(&mut d, json!({"type": "select", "ids": b}));
    let out = run(&mut d, json!({"type": "intersect_faces", "with": "model"}));
    assert_eq!(list_len(&out, "split_faces"), 6, "{out}");
}

#[test]
fn unknown_and_unsuitable_ids_are_refused() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let (kind, _) = try_run(
        &mut d,
        json!({"type": "intersect_faces", "ids": ["f9999"], "with": "model"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    let (kind, _) = try_run(
        &mut d,
        json!({"type": "intersect_faces", "ids": ["v0"], "with": "model"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(!a.is_empty());
}

#[test]
fn with_selection_only_crosses_the_selected_faces() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [1.0; 3], [3.0; 3]);
    // A third box crosses both but stays out of the selection.
    boxed(&mut d, [0.5, -1.0, 0.5], [1.5, 4.0, 1.5]);
    let ids: Vec<Value> = a.iter().chain(&b).cloned().collect();
    let out = intersect(&mut d, &ids, "selection");
    validate(&d);
    assert_eq!(list_len(&out, "split_faces"), 6, "{out}");
    // B alone against nothing selected with it: no crossings.
    let mut e = doc();
    boxed(&mut e, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut e, [1.0; 3], [3.0; 3]);
    let out = intersect(&mut e, &b, "selection");
    assert_eq!(list_len(&out, "added_edges"), 0, "{out}");
}

#[test]
fn a_group_is_never_modified_and_with_context_ignores_it() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let mut ids = a.clone();
    let edges: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    ids.extend(edges);
    let group = instance_id(&run(&mut d, json!({"type": "make_group", "ids": ids})));
    let definition = d.scene().root().instances().next().unwrap().1.definition;
    let group_mesh = MeshId::Definition(definition);
    let b = boxed(&mut d, [1.0; 3], [3.0; 3]);
    let before = mesh_state(&d, group_mesh);

    // With Context: the group is not part of the open context's own mesh.
    let revision = d.revision();
    let out = intersect(&mut d, &b, "context");
    assert_eq!(list_len(&out, "added_edges"), 0, "{out}");
    assert_eq!(d.revision(), revision);

    // With Model: the box is split by the group's faces, the group is untouched.
    let out = intersect(&mut d, &b, "model");
    validate(&d);
    assert_eq!(list_len(&out, "split_faces"), 3, "{out}");
    assert_eq!(mesh_state(&d, group_mesh), before);

    // The group as the selection crosses the root box too, and still changes only the root.
    let out = intersect(&mut d, &[json!(group)], "model");
    assert_eq!(mesh_state(&d, group_mesh), before);
    assert_eq!(list_len(&out, "split_faces"), 0, "{out}");
    assert_eq!(list_len(&out, "added_edges"), 0, "{out}");
}

#[test]
fn inside_an_open_transformed_group_the_edges_go_into_it() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let edges: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    let mut ids = a.clone();
    ids.extend(edges);
    let group = instance_id(&run(&mut d, json!({"type": "make_group", "ids": ids})));
    // Scale, turn and move the group, so its coordinates differ from the root's.
    run(
        &mut d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [1.5, 1.0, 2.0]}),
    );
    run(
        &mut d,
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "axis": [0, 0, 1], "angle": 0.5}),
    );
    run(
        &mut d,
        json!({"type": "move", "ids": [group], "offset": [0.2, 0.3, 0.1]}),
    );
    let root_box = boxed(&mut d, [1.0, 0.5, 1.0], [4.0, 2.5, 3.0]);
    let root_before = mesh_state(&d, MeshId::Root);

    run(&mut d, json!({"type": "open_instance", "id": group}));
    let faces: Vec<Value> = face_ids(&d).iter().map(|f| json!(f)).collect();
    let out = intersect(&mut d, &faces, "model");
    validate(&d);
    assert!(list_len(&out, "added_edges") > 0, "{out}");
    assert!(list_len(&out, "split_faces") > 0, "{out}");
    assert_eq!(mesh_state(&d, MeshId::Root), root_before);

    // Every new edge end, in the root, lies on the surface of the root box.
    let to_world: DAffine3 = d.context_transform();
    let on_surface = |p: DVec3| {
        let (lo, hi) = (DVec3::new(1.0, 0.5, 1.0), DVec3::new(4.0, 2.5, 3.0));
        let inside_box = p.cmpge(lo - 1e-6).all() && p.cmple(hi + 1e-6).all();
        let on_face = (0..3).any(|i| (p[i] - lo[i]).abs() < 1e-6 || (p[i] - hi[i]).abs() < 1e-6);
        inside_box && on_face
    };
    let m = d.model();
    let crossing: usize = m
        .vertices()
        .filter(|(_, v)| on_surface(to_world.transform_point3(v.pos)))
        .count();
    assert!(crossing >= 6, "{crossing} vertices on the box surface");
    assert!(!root_box.is_empty());
}

#[test]
fn a_mirrored_group_is_crossed_in_its_own_coordinates() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let edges: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    let mut ids = a.clone();
    ids.extend(edges);
    let group = instance_id(&run(&mut d, json!({"type": "make_group", "ids": ids})));
    run(
        &mut d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [-1.0, 1.0, 1.0]}),
    );
    // The mirrored group now spans x in -2..0.
    boxed(&mut d, [-3.0, 1.0, 1.0], [-1.0, 3.0, 3.0]);
    run(&mut d, json!({"type": "open_instance", "id": group}));
    let faces: Vec<Value> = face_ids(&d).iter().map(|f| json!(f)).collect();
    let out = intersect(&mut d, &faces, "model");
    validate(&d);
    assert_eq!(list_len(&out, "split_faces"), 3, "{out}");
    let to_world = d.context_transform();
    let m = d.model();
    // The crossing lies on the root box's walls: x = -1, y = 1 or z = 1 in the root.
    let on_walls = m
        .vertices()
        .filter(|(_, v)| {
            let p = to_world.transform_point3(v.pos);
            (p.x + 1.0).abs() < 1e-6 || (p.y - 1.0).abs() < 1e-6 || (p.z - 1.0).abs() < 1e-6
        })
        .count();
    assert!(on_walls >= 6, "{on_walls}");
}

#[test]
fn a_coplanar_rectangle_in_a_group_outlines_the_shared_region_in_the_root() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [1, 1, 0], "opposite": [3, 3, 0]}),
    );
    let root_face = face_ids(&d).into_iter().next().unwrap();
    let edges: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    let mut ids = vec![json!(root_face)];
    ids.extend(edges);
    // The first rectangle becomes a group; a second one is drawn in the root over it.
    let group = instance_id(&run(&mut d, json!({"type": "make_group", "ids": ids})));
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    let out = intersect(&mut d, &[json!(group)], "model");
    validate(&d);
    assert_eq!(
        out["intersect"]["split_faces"].as_array().unwrap().len(),
        1,
        "{out}"
    );
    assert_eq!(list_len(&out, "added_edges"), 6, "{out}");
    let mut areas: Vec<f64> = d
        .model()
        .faces()
        .map(|(f, _)| {
            ask(&d, json!({"type": "entity", "id": EntityId::Face(f)}))["area"]
                .as_f64()
                .unwrap()
        })
        .collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 2, "{areas:?}");
    assert!(
        (areas[0] - 1.0).abs() < 1e-9 && (areas[1] - 3.0).abs() < 1e-9,
        "{areas:?}"
    );
}

#[test]
fn a_lone_group_is_not_crossed_with_itself() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let edges: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    let mut ids = a;
    ids.extend(edges);
    let group = instance_id(&run(&mut d, json!({"type": "make_group", "ids": ids})));
    let before = mesh_state(&d, MeshId::Root);
    let revision = d.revision();
    let out = intersect(&mut d, &[json!(group)], "model");
    assert_eq!(
        out["intersect"],
        json!({"added_edges": [], "split_faces": [], "created_faces": []})
    );
    assert_eq!(mesh_state(&d, MeshId::Root), before);
    assert_eq!(d.revision(), revision);
}

#[test]
fn nested_groups_inside_one_selected_group_are_not_crossed_with_each_other() {
    let mut d = doc();
    // Two overlapping boxes, each its own group, inside one outer group.
    let mut inner = Vec::new();
    for (lo, hi) in [(0.0, 2.0), (1.0, 3.0)] {
        let before: BTreeSet<String> =
            ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].as_str().unwrap().to_string())
                .collect();
        let faces = boxed(&mut d, [lo; 3], [hi; 3]);
        let mut ids = faces;
        ids.extend(
            ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].clone())
                .filter(|e| !before.contains(e.as_str().unwrap())),
        );
        let g = instance_id(&run(&mut d, json!({"type": "make_group", "ids": ids})));
        inner.push(json!(g));
    }
    let outer = instance_id(&run(&mut d, json!({"type": "make_group", "ids": inner})));
    let before = mesh_state(&d, MeshId::Root);
    let revision = d.revision();
    let out = intersect(&mut d, &[json!(outer)], "model");
    assert_eq!(
        out["intersect"],
        json!({"added_edges": [], "split_faces": [], "created_faces": []})
    );
    assert_eq!(mesh_state(&d, MeshId::Root), before);
    assert_eq!(d.revision(), revision);
}

#[test]
fn touching_boxes_add_nothing() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [2.0, 0.0, 0.0], [4.0, 2.0, 2.0]);
    let revision = d.revision();
    let ids: Vec<Value> = a.iter().chain(&b).cloned().collect();
    let out = intersect(&mut d, &ids, "model");
    assert_eq!(
        out["intersect"],
        json!({"added_edges": [], "split_faces": [], "created_faces": []})
    );
    assert_eq!(d.revision(), revision);
}

#[test]
fn a_cut_through_a_hole_leaves_no_edge_across_it() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 4, 0]}),
    );
    let inner = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [1, 1, 0], "opposite": [3, 3, 0]}),
    );
    let hole_face = inner["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    run(&mut d, json!({"type": "erase", "ids": [hole_face]}));
    let plate: Vec<Value> = face_ids(&d).iter().map(|f| json!(f)).collect();
    assert_eq!(plate.len(), 1);
    let wall = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [-1, 2, -1], "opposite": [5, 2, 1],
            "normal": [0, 1, 0]}),
    );
    let wall_face = wall["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let out = intersect(&mut d, &[wall_face], "model");
    validate(&d);
    assert!(list_len(&out, "added_edges") >= 2, "{out}");
    let m = d.model();
    for id in out["intersect"]["added_edges"].as_array().unwrap() {
        let EntityId::Edge(e) = id.as_str().unwrap().parse().unwrap() else {
            panic!("an edge id")
        };
        let edge = m.edge(e).unwrap();
        let (pa, pb) = (
            m.vertex(edge.v[0]).unwrap().pos,
            m.vertex(edge.v[1]).unwrap().pos,
        );
        let mid = (pa + pb) / 2.0;
        // Edges in the plate's plane never run through the hole.
        if pa.z.abs() < 1e-9
            && pb.z.abs() < 1e-9
            && (pa.y - 2.0).abs() < 1e-9
            && (pb.y - 2.0).abs() < 1e-9
        {
            assert!(!(mid.x > 1.0 + 1e-9 && mid.x < 3.0 - 1e-9), "{pa} {pb}");
        }
    }
    // The plate splits in two pieces, both with their hole edge segments.
    assert_eq!(list_len(&out, "split_faces"), 1, "{out}");
}

#[test]
fn reports_work_as_batch_references() {
    let mut d = doc();
    boxed(&mut d, [0.0; 3], [2.0; 3]);
    let b = boxed(&mut d, [1.0; 3], [3.0; 3]);
    let commands: Vec<BatchCommand> = vec![
        json!({"type": "intersect_faces", "ids": b, "with": "model"}),
        json!({"type": "select", "ids": ["$0:f*"]}),
        json!({"type": "erase", "ids": ["$0:e*"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, true).unwrap();
    assert!(outcomes[0].intersect.is_some());
    validate(&d);
}
