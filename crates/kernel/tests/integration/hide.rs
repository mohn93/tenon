//! Hiding edges, faces, instances and guides: the flag, undo, and how it follows geometry
//! through splits, groups, explodes and files.

use glam::DVec3;
use kernel::{
    ChangeSet, DefinitionKind, EntityId, FaceId, Guide, InstanceId, MeshId, Model, Scene, SceneData,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 2 x 2 square on the ground, in a scene of its own.
fn square_scene() -> (Scene, FaceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
    ])
    .unwrap();
    let face = m.faces().next().unwrap().0;
    (Scene::from_root(m), face)
}

fn inverse(c: &ChangeSet) -> ChangeSet {
    c.inverse()
}

fn grouped_square() -> (Scene, InstanceId) {
    let (mut s, face) = square_scene();
    let e: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(MeshId::Root, &e, &[face], &[], DefinitionKind::Group, "")
        .unwrap();
    (s, group)
}

#[test]
fn hide_and_show_each_kind_and_undo() {
    let (s, face) = square_scene();
    let mut m = s.root().clone();
    let edge = m.edges().next().unwrap().0;
    let g = {
        m.add_guide(Guide::point(p(1.0, 1.0, 1.0))).unwrap();
        m.guides().next().unwrap().0
    };
    let ids = [
        EntityId::Edge(edge),
        EntityId::Face(face),
        EntityId::Guide(g),
    ];
    let (c, changed) = m.hide_entities(&ids, true);
    assert_eq!(changed, ids.to_vec());
    assert!(m.edge(edge).unwrap().hidden);
    assert!(m.face(face).unwrap().hidden);
    assert!(m.guide(g).unwrap().hidden());
    // Hiding the face leaves the other edges as they are.
    assert_eq!(m.edges().filter(|(_, e)| e.hidden).count(), 1);
    assert_eq!(m.hidden_entities().len(), 3);
    // Already hidden ones are ignored.
    let (again, changed) = m.hide_entities(&ids, true);
    assert!(again.ops.is_empty() && changed.is_empty());
    m.apply(&inverse(&c));
    assert!(m.hidden_entities().is_empty());
    m.apply(&c);
    let (c2, changed) = m.hide_entities(&ids[..2], false);
    assert_eq!(changed.len(), 2);
    assert_eq!(m.hidden_entities(), vec![EntityId::Guide(g)]);
    m.apply(&inverse(&c2));
    assert_eq!(m.hidden_entities().len(), 3);
}

#[test]
fn vertices_and_unknown_ids_are_skipped() {
    let (s, _) = square_scene();
    let mut m = s.root().clone();
    let v = m.vertices().next().unwrap().0;
    let (c, changed) = m.hide_entities(
        &[EntityId::Vertex(v), EntityId::Edge(kernel::EdgeId(999_999))],
        true,
    );
    assert!(c.ops.is_empty() && changed.is_empty());
}

#[test]
fn a_split_edge_keeps_the_flag() {
    let (s, _) = square_scene();
    let mut m = s.root().clone();
    let all: Vec<_> = m.edges().map(|(e, _)| EntityId::Edge(e)).collect();
    m.hide_entities(&all, true);
    m.add_edge(p(1.0, -1.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    // The bottom side was split at (1, 0, 0): both halves stay hidden.
    let halves = m
        .edges()
        .filter(|(_, e)| {
            let ys = e.v.map(|v| m.vertex(v).unwrap().pos.y);
            ys[0].abs() < 1e-9 && ys[1].abs() < 1e-9
        })
        .count();
    assert_eq!(halves, 2);
    assert!(
        m.edges()
            .filter(|(_, e)| e.v.iter().all(|v| m.vertex(*v).unwrap().pos.y.abs() < 1e-9))
            .all(|(_, e)| e.hidden)
    );
}

#[test]
fn exploding_a_hidden_instance_hides_what_was_inside() {
    let (mut s, group) = grouped_square();
    s.edit(MeshId::Root, |m| {
        Ok(m.hide_entities(&[EntityId::Instance(group)], true).0)
    })
    .unwrap();
    s.explode(MeshId::Root, group).unwrap();
    let m = s.root();
    assert!(m.faces().all(|(_, f)| f.hidden));
    assert!(m.edges().all(|(_, e)| e.hidden));
}

#[test]
fn the_flag_round_trips_through_scene_data() {
    let (mut s, group) = grouped_square();
    let (face, edge) = {
        let d = s.root().instance(group).unwrap().definition;
        let m = s.mesh(MeshId::Definition(d)).unwrap();
        (m.faces().next().unwrap().0, m.edges().next().unwrap().0)
    };
    let d = MeshId::Definition(s.root().instance(group).unwrap().definition);
    s.edit(d, |m| {
        m.add_guide(Guide::line(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)))
            .unwrap();
        let g = m.guides().next().unwrap().0;
        Ok(m.hide_entities(
            &[
                EntityId::Face(face),
                EntityId::Edge(edge),
                EntityId::Guide(g),
            ],
            true,
        )
        .0)
    })
    .unwrap();
    s.edit(MeshId::Root, |m| {
        Ok(m.hide_entities(&[EntityId::Instance(group)], true).0)
    })
    .unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(json.contains("\"hidden\":true"));
    let back = Scene::from_data(&serde_json::from_str::<SceneData>(&json).unwrap()).unwrap();
    assert!(back.root().instance(group).unwrap().hidden);
    let m = back.mesh(d).unwrap();
    assert!(m.face(face).unwrap().hidden);
    assert!(m.edge(edge).unwrap().hidden);
    assert!(m.guides().next().unwrap().1.hidden());
}

#[test]
fn nothing_hidden_writes_no_flag() {
    let (s, _) = grouped_square();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(!json.contains("hidden"));
}

#[test]
fn a_copy_keeps_hidden_faces_and_edges() {
    let (s, face) = square_scene();
    let mut m = s.root().clone();
    let edge = m.edges().next().unwrap().0;
    m.hide_entities(&[EntityId::Face(face), EntityId::Edge(edge)], true);
    let edges: Vec<_> = m.edges().map(|(e, _)| e).collect();
    m.copy_placed(
        &edges,
        &[face],
        &[glam::DAffine3::from_translation(p(5.0, 0.0, 0.0))],
    )
    .unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().filter(|(_, f)| f.hidden).count(), 2);
    assert_eq!(m.edges().filter(|(_, e)| e.hidden).count(), 2);
}

#[test]
fn a_copy_over_existing_edges_leaves_them_shown() {
    let (s, _) = square_scene();
    let mut m = s.root().clone();
    // A hidden edge on top of the square's shown bottom side, copied onto it.
    m.add_edge(p(10.0, 0.0, 0.0), p(12.0, 0.0, 0.0)).unwrap();
    let loose = m.edges().last().unwrap().0;
    m.hide_entities(&[EntityId::Edge(loose)], true);
    let shown_before = m.edges().filter(|(_, e)| !e.hidden).count();
    m.copy_placed(
        &[loose],
        &[],
        &[glam::DAffine3::from_translation(p(-10.0, 0.0, 0.0))],
    )
    .unwrap();
    // The copy lies along the square's bottom side: that edge stays shown.
    assert_eq!(m.edges().filter(|(_, e)| !e.hidden).count(), shown_before);
    assert_eq!(m.edges().filter(|(_, e)| e.hidden).count(), 1);
}

#[test]
fn a_hidden_face_split_by_a_drawn_line_keeps_both_pieces_hidden() {
    let (s, face) = square_scene();
    let mut m = s.root().clone();
    m.hide_entities(&[EntityId::Face(face)], true);
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0)).unwrap();
    assert_eq!(m.faces().count(), 2);
    assert!(m.faces().all(|(_, f)| f.hidden));
}

#[test]
fn push_pull_caps_keep_the_flag_and_sides_are_shown() {
    let (s, face) = square_scene();
    let mut m = s.root().clone();
    m.hide_entities(&[EntityId::Face(face)], true);
    m.push_pull(face, 1.0).unwrap();
    m.validate().unwrap();
    // Base and cap stay hidden; the four sides are shown.
    assert_eq!(m.faces().count(), 6);
    assert_eq!(m.faces().filter(|(_, f)| f.hidden).count(), 2);
}

#[test]
fn exploding_a_hidden_instance_does_not_hide_what_was_already_there() {
    let (mut s, group) = grouped_square();
    // A shown square at the root sharing the group's footprint edge-for-edge would merge, so
    // use one that touches along one side only.
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(2.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 2.0, 0.0),
            p(2.0, 2.0, 0.0),
        ])
    })
    .unwrap();
    let shown: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    s.edit(MeshId::Root, |m| {
        Ok(m.hide_entities(&[EntityId::Instance(group)], true).0)
    })
    .unwrap();
    s.explode(MeshId::Root, group).unwrap();
    let m = s.root();
    // The shared side (x = 2) and the shown square's other edges stay shown.
    for e in shown {
        assert!(!m.edge(e).unwrap().hidden, "{e:?}");
    }
    assert_eq!(m.edges().filter(|(_, e)| e.hidden).count(), 3);
}
