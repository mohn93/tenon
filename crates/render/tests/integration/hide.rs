//! Entities hidden by their own flag in what the renderer draws, at any instance depth.

use glam::DVec3;
use kernel::{DefinitionKind, EntityId, Guide, InstanceId, MeshId, Model, Scene};
use render::scene::{SceneBase, scene_base};

fn square_at(m: &mut Model, x: f64) {
    let p = |dx: f64, y: f64| DVec3::new(x + dx, y, 0.0);
    m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
        .unwrap();
}

fn counts(b: &SceneBase) -> (usize, usize, usize) {
    (b.faces.len(), b.edges.len(), b.guides.len())
}

fn hide(s: &mut Scene, mesh: MeshId, ids: &[EntityId]) {
    s.edit(mesh, |m| Ok(m.hide_entities(ids, true).0)).unwrap();
}

/// A square component at x 0, placed again at x 3, a loose square at x 6 and a guide point.
/// Returns the scene, the component's definition mesh and its two instances.
fn scene() -> (Scene, MeshId, Vec<InstanceId>) {
    let mut m = Model::new();
    square_at(&mut m, 0.0);
    m.add_guide(Guide::point(DVec3::new(9.0, 9.0, 0.0)))
        .unwrap();
    let edges: Vec<_> = m.edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    let mut s = Scene::from_root(m);
    let (first, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Square",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[first], DVec3::new(3.0, 0.0, 0.0), 1)
        .unwrap();
    s.edit(MeshId::Root, |m| {
        let p = |dx: f64, y: f64| DVec3::new(6.0 + dx, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
    })
    .unwrap();
    let ids = s.root().instances().map(|(id, _)| id).collect();
    let def = MeshId::Definition(s.root().instance(first).unwrap().definition);
    (s, def, ids)
}

#[test]
fn nothing_hidden_changes_nothing() {
    let (s, _, _) = scene();
    // Three squares of two triangles, each edge two vertices, 4 edges apiece.
    assert_eq!(counts(&scene_base(&s, &[])), (18, 24, 1));
}

#[test]
fn a_hidden_face_in_a_definition_is_left_out_of_both_placements() {
    let (mut s, def, _) = scene();
    let face = s.mesh(def).unwrap().faces().next().unwrap().0;
    hide(&mut s, def, &[EntityId::Face(face)]);
    let base = scene_base(&s, &[]);
    assert_eq!(counts(&base), (6, 24, 1), "only the loose square's face");
    assert!(base.faces.iter().all(|v| v.pos[0] >= 6.0));
    assert_eq!(base.casters.len(), 6, "no shadow from a hidden face");
}

#[test]
fn a_hidden_edge_is_not_drawn_but_its_face_is() {
    let (mut s, def, _) = scene();
    let edge = s.mesh(def).unwrap().edges().next().unwrap().0;
    hide(&mut s, def, &[EntityId::Edge(edge)]);
    assert_eq!(counts(&scene_base(&s, &[])), (18, 20, 1), "one edge, twice");
    let root_edge = s.root().edges().next().unwrap().0;
    hide(&mut s, MeshId::Root, &[EntityId::Edge(root_edge)]);
    assert_eq!(counts(&scene_base(&s, &[])), (18, 18, 1));
}

#[test]
fn a_hidden_instance_and_a_hidden_guide_are_left_out() {
    let (mut s, _, ids) = scene();
    hide(&mut s, MeshId::Root, &[EntityId::Instance(ids[0])]);
    let base = scene_base(&s, &[]);
    assert_eq!(counts(&base), (12, 16, 1));
    let guide = s.root().guides().next().unwrap().0;
    hide(&mut s, MeshId::Root, &[EntityId::Guide(guide)]);
    assert_eq!(counts(&scene_base(&s, &[])), (12, 16, 0));
    // Opened, the instance being edited stays drawn.
    assert_eq!(scene_base(&s, &[ids[0]]).faces.len(), 18);
}

#[test]
fn a_hidden_instance_two_levels_deep_hides_what_it_places() {
    let (mut s, _, ids) = scene();
    let (outer, _) = s
        .make_group(MeshId::Root, &[], &[], &ids, DefinitionKind::Group, "")
        .unwrap();
    let outer_def = MeshId::Definition(s.root().instance(outer).unwrap().definition);
    assert_eq!(scene_base(&s, &[]).faces.len(), 18);
    let inner = s.mesh(outer_def).unwrap().instances().next().unwrap().0;
    hide(&mut s, outer_def, &[EntityId::Instance(inner)]);
    assert_eq!(counts(&scene_base(&s, &[])), (12, 16, 1));
}
