use glam::{DAffine3, DVec3};
use kernel::{
    DefinitionKind, EdgeId, EntityId, FaceId, InstanceId, MeshId, Model, Scene, SceneChange,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A scene whose root holds a 2 x 3 x 1 box with its corner at `at`.
fn box_scene(at: DVec3) -> Scene {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            at,
            at + p(2.0, 0.0, 0.0),
            at + p(2.0, 3.0, 0.0),
            at + p(0.0, 3.0, 0.0),
        ])
        .unwrap();
    let face = c
        .summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap();
    m.push_pull(face, 1.0).unwrap();
    m.validate().unwrap();
    Scene::from_root(m)
}

fn all(m: &Model) -> (Vec<EdgeId>, Vec<FaceId>) {
    (
        m.edges().map(|(id, _)| id).collect(),
        m.faces().map(|(id, _)| id).collect(),
    )
}

fn counts(m: &Model) -> (usize, usize, usize, usize) {
    (
        m.vertices().count(),
        m.edges().count(),
        m.faces().count(),
        m.instances().count(),
    )
}

fn group_all(s: &mut Scene, kind: DefinitionKind) -> (InstanceId, SceneChange) {
    let (edges, faces) = all(s.root());
    s.make_group(MeshId::Root, &edges, &faces, &[], kind, "Box")
        .unwrap()
}

fn definition_of(s: &Scene, i: InstanceId) -> kernel::DefinitionId {
    s.root().instance(i).unwrap().definition
}

#[test]
fn grouping_a_box_moves_it_into_a_definition() {
    let mut s = box_scene(DVec3::ZERO);
    let before = s.clone();
    let (i, change) = group_all(&mut s, DefinitionKind::Group);
    s.validate().unwrap();

    assert_eq!(counts(s.root()), (0, 0, 0, 1));
    let instance = s.root().instance(i).unwrap();
    assert_eq!(instance.transform, DAffine3::IDENTITY);
    let d = s.definition(instance.definition).unwrap();
    assert_eq!((d.name.as_str(), d.kind), ("Box", DefinitionKind::Group));
    assert_eq!(counts(&d.mesh), (8, 12, 6, 0));
    assert_eq!(s.instance_count(instance.definition), 1);

    let summary = change.summary();
    assert!(summary.created.contains(&EntityId::Instance(i)));
    assert_eq!(summary.deleted.len(), 8 + 12 + 6);
    assert_eq!(summary.created.len(), 1 + 8 + 12 + 6);

    let after = s.clone();
    s.apply(&change.inverse());
    assert_eq!(s, before);
    s.apply(&change);
    assert_eq!(s, after);
}

#[test]
fn grouped_faces_keep_their_orientation() {
    let mut s = box_scene(DVec3::ZERO);
    let normals = |m: &Model| {
        let mut n: Vec<[i64; 3]> = m
            .faces()
            .map(|(_, f)| f.normal.to_array().map(|c| c.round() as i64))
            .collect();
        n.sort();
        n
    };
    let before = normals(s.root());
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let d = definition_of(&s, i);
    assert_eq!(normals(&s.definition(d).unwrap().mesh), before);
}

#[test]
fn ids_stay_unique_across_meshes_and_are_never_reused() {
    let mut s = box_scene(DVec3::ZERO);
    let (_, change) = group_all(&mut s, DefinitionKind::Group);
    let used = change.summary().created;
    s.apply(&change.inverse());
    let again = s
        .edit(MeshId::Root, |m| {
            m.add_edge(p(5.0, 0.0, 0.0), p(6.0, 0.0, 0.0))
        })
        .unwrap()
        .summary()
        .created;
    for id in again {
        assert!(!used.contains(&id), "{id} reused");
    }
    s.validate().unwrap();
}

#[test]
fn grouping_one_face_of_a_box_leaves_its_shared_edges() {
    let mut s = box_scene(DVec3::ZERO);
    let top = s
        .root()
        .faces()
        .find(|(_, f)| f.normal.z > 0.5)
        .map(|(id, _)| id)
        .unwrap();
    let (i, change) = s
        .make_group(MeshId::Root, &[], &[top], &[], DefinitionKind::Group, "Top")
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (8, 12, 5, 1));
    let d = definition_of(&s, i);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (4, 4, 1, 0));
    assert_eq!(change.summary().deleted, vec![EntityId::Face(top)]);
}

#[test]
fn grouping_a_lone_face_takes_its_edges_along() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let face = m.faces().next().unwrap().0;
    let mut s = Scene::from_root(m);
    let (i, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[face],
            &[],
            DefinitionKind::Group,
            "Sheet",
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (0, 0, 0, 1));
    let d = definition_of(&s, i);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (4, 4, 1, 0));
}

#[test]
fn a_selected_edge_still_bounding_a_face_stays_behind() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let edge = m.edges().next().unwrap().0;
    let mut s = Scene::from_root(m);
    let (i, _) = s
        .make_group(
            MeshId::Root,
            &[edge],
            &[],
            &[],
            DefinitionKind::Group,
            "Edge",
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (4, 4, 1, 1));
    let d = definition_of(&s, i);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (2, 1, 0, 0));
}

#[test]
fn an_opening_stays_open_in_the_group() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(4.0, 0.0, 0.0),
        p(4.0, 4.0, 0.0),
        p(0.0, 4.0, 0.0),
    ])
    .unwrap();
    m.add_polygon(&[
        p(1.0, 1.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(1.0, 2.0, 0.0),
    ])
    .unwrap();
    let (small, big): (Vec<FaceId>, Vec<FaceId>) = m
        .faces()
        .map(|(id, _)| id)
        .partition(|id| m.area(*id).unwrap() < 2.0);
    m.erase(&[], &small);
    let mut s = Scene::from_root(m);
    let (edges, _) = all(s.root());
    let (i, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &big,
            &[],
            DefinitionKind::Group,
            "Frame",
        )
        .unwrap();
    s.validate().unwrap();
    let d = definition_of(&s, i);
    let mesh = &s.definition(d).unwrap().mesh;
    assert_eq!(counts(mesh), (8, 8, 1, 0));
    assert_eq!(mesh.faces().next().unwrap().1.holes.len(), 1);
}

#[test]
fn grouping_an_instance_with_geometry_nests_it() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Component);
    s.edit(MeshId::Root, |m| {
        m.add_edge(p(5.0, 0.0, 0.0), p(6.0, 0.0, 0.0))
    })
    .unwrap();
    let (edges, faces) = all(s.root());
    let (outer, change) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[inner],
            DefinitionKind::Group,
            "Both",
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (0, 0, 0, 1));
    let d = definition_of(&s, outer);
    let mesh = &s.definition(d).unwrap().mesh;
    assert_eq!(counts(mesh), (2, 1, 0, 1));
    assert!(mesh.instance(inner).is_some());
    assert!(
        change
            .summary()
            .modified
            .contains(&EntityId::Instance(inner))
    );
}

#[test]
fn an_instance_listed_twice_moves_once() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
    let (outer, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[inner, inner],
            DefinitionKind::Group,
            "Outer",
        )
        .unwrap();
    s.validate().unwrap();
    let d = definition_of(&s, outer);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (0, 0, 0, 1));
}

#[test]
fn moving_an_instance_translates_it_and_undoes_exactly() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let before = s.clone();
    let change = s
        .edit(MeshId::Root, |m| {
            Ok(m.move_instances(&[i], p(1.0, 2.0, 3.0)))
        })
        .unwrap();
    assert_eq!(
        s.root().instance(i).unwrap().transform,
        DAffine3::from_translation(p(1.0, 2.0, 3.0))
    );
    assert_eq!(change.summary().modified, vec![EntityId::Instance(i)]);
    s.apply(&change.inverse());
    assert_eq!(s, before);
}
