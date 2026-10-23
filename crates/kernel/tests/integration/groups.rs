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

#[test]
fn an_instance_listed_twice_moves_once_by_move_instances() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let change = s
        .edit(MeshId::Root, |m| {
            Ok(m.move_instances(&[i, i], p(1.0, 2.0, 3.0)))
        })
        .unwrap();
    assert_eq!(
        s.root().instance(i).unwrap().transform,
        DAffine3::from_translation(p(1.0, 2.0, 3.0))
    );
    assert_eq!(change.summary().modified, vec![EntityId::Instance(i)]);
}

fn root_counts(s: &Scene) -> (usize, usize, usize, usize) {
    counts(s.root())
}

#[test]
fn exploding_a_group_puts_its_geometry_back() {
    let mut s = box_scene(DVec3::ZERO);
    let original = root_counts(&s);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let d = definition_of(&s, i);
    let before = s.clone();
    let change = s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    assert_eq!(root_counts(&s), original);
    assert!(s.definition(d).is_none(), "the group's definition goes");
    assert!(change.summary().deleted.contains(&EntityId::Instance(i)));
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn exploding_a_moved_instance_places_the_geometry_where_it_was_shown() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    s.edit(MeshId::Root, |m| {
        Ok(m.move_instances(&[i], p(10.0, 0.0, 0.0)))
    })
    .unwrap();
    assert_eq!(
        s.bounds(MeshId::Root),
        Some([p(10.0, 0.0, 0.0), p(12.0, 3.0, 1.0)])
    );
    s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    assert_eq!(
        s.bounds(MeshId::Root),
        Some([p(10.0, 0.0, 0.0), p(12.0, 3.0, 1.0)])
    );
}

#[test]
fn an_exploded_face_merges_with_what_it_touches() {
    let mut s = box_scene(DVec3::ZERO);
    let top = s
        .root()
        .faces()
        .find(|(_, f)| f.normal.z > 0.5)
        .map(|(id, _)| id)
        .unwrap();
    let (i, _) = s
        .make_group(MeshId::Root, &[], &[top], &[], DefinitionKind::Group, "Top")
        .unwrap();
    s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    assert_eq!(root_counts(&s), (8, 12, 6, 0));
    let up = s.root().faces().filter(|(_, f)| f.normal.z > 0.5).count();
    assert_eq!(up, 1);
}

#[test]
fn a_component_definition_outlives_its_exploded_instances() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Component);
    let d = definition_of(&s, i);
    s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    assert!(s.definition(d).is_some());
    assert_eq!(s.instance_count(d), 0);
}

#[test]
fn exploding_a_nested_group_lifts_the_inner_instance() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Component);
    s.edit(MeshId::Root, |m| {
        Ok(m.move_instances(&[inner], p(0.0, 5.0, 0.0)))
    })
    .unwrap();
    let (outer, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[inner],
            DefinitionKind::Group,
            "Outer",
        )
        .unwrap();
    s.edit(MeshId::Root, |m| {
        Ok(m.move_instances(&[outer], p(1.0, 0.0, 0.0)))
    })
    .unwrap();
    s.explode(MeshId::Root, outer).unwrap();
    s.validate().unwrap();
    let lifted: Vec<_> = s.root().instances().collect();
    assert_eq!(lifted.len(), 1);
    assert_ne!(lifted[0].0, inner, "lifted instances get new ids");
    assert_eq!(
        lifted[0].1.transform,
        DAffine3::from_translation(p(1.0, 5.0, 0.0))
    );
}

#[test]
fn copies_of_a_component_share_its_definition() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Component);
    let d = definition_of(&s, i);
    let change = s
        .copy_instances(MeshId::Root, &[i], p(3.0, 0.0, 0.0), 3)
        .unwrap();
    s.validate().unwrap();
    assert_eq!(s.instance_count(d), 4);
    assert_eq!(s.definitions().count(), 1);
    assert_eq!(change.summary().created.len(), 3);
    assert_eq!(
        s.bounds(MeshId::Root),
        Some([p(0.0, 0.0, 0.0), p(11.0, 3.0, 1.0)])
    );
}

#[test]
fn a_copy_of_a_group_gets_its_own_definition() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
    let (outer, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[inner],
            DefinitionKind::Group,
            "Outer",
        )
        .unwrap();
    let before = s.clone();
    let change = s
        .copy_instances(MeshId::Root, &[outer], p(0.0, 4.0, 0.0), 1)
        .unwrap();
    s.validate().unwrap();
    assert_eq!(s.definitions().count(), 4, "outer and inner both copied");
    let copy = s
        .root()
        .instances()
        .find(|(id, _)| *id != outer)
        .map(|(_, i)| i.definition)
        .unwrap();
    assert_ne!(copy, definition_of(&s, outer));
    assert_eq!(s.instance_count(copy), 1);
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn erasing_a_group_removes_its_definition_and_undoes_exactly() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let d = definition_of(&s, i);
    let before = s.clone();
    let change = s.erase_instances(MeshId::Root, &[i]);
    s.validate().unwrap();
    assert_eq!(root_counts(&s), (0, 0, 0, 0));
    assert!(s.definition(d).is_none());
    assert_eq!(change.summary().deleted, vec![EntityId::Instance(i)]);
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn erasing_a_component_instance_keeps_the_definition() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Component);
    let d = definition_of(&s, i);
    s.erase_instances(MeshId::Root, &[i]);
    s.validate().unwrap();
    assert!(s.definition(d).is_some());
}

#[test]
fn paths_resolve_to_the_open_mesh_and_its_placement() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
    let (outer, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[inner],
            DefinitionKind::Group,
            "Outer",
        )
        .unwrap();
    s.edit(MeshId::Root, |m| {
        Ok(m.move_instances(&[outer], p(1.0, 2.0, 3.0)))
    })
    .unwrap();
    let d_outer = definition_of(&s, outer);
    let d_inner = s
        .definition(d_outer)
        .unwrap()
        .mesh
        .instance(inner)
        .unwrap()
        .definition;
    assert_eq!(s.resolve(&[]), Some((MeshId::Root, DAffine3::IDENTITY)));
    assert_eq!(
        s.resolve(&[outer, inner]),
        Some((
            MeshId::Definition(d_inner),
            DAffine3::from_translation(p(1.0, 2.0, 3.0))
        ))
    );
    assert_eq!(s.resolve(&[inner]), None, "inner is not in the root");
}

fn open_box_group(kind: DefinitionKind) -> (Scene, InstanceId) {
    let mut s = box_scene(DVec3::ZERO);
    let top = s
        .root()
        .faces()
        .find(|(_, f)| f.normal.z > 0.5)
        .map(|(id, _)| id)
        .unwrap();
    s.edit(MeshId::Root, |m| Ok(m.erase(&[], &[top]))).unwrap();
    let (i, _) = group_all(&mut s, kind);
    (s, i)
}

#[test]
fn grouping_an_open_box_keeps_it_open() {
    let (s, i) = open_box_group(DefinitionKind::Group);
    s.validate().unwrap();
    assert_eq!(root_counts(&s), (0, 0, 0, 1));
    let d = s.definition(definition_of(&s, i)).unwrap();
    assert_eq!(counts(&d.mesh), (8, 12, 5, 0));
}

#[test]
fn exploding_an_open_box_keeps_it_open() {
    let (mut s, i) = open_box_group(DefinitionKind::Group);
    s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    assert_eq!(root_counts(&s), (8, 12, 5, 0));
}

#[test]
fn copying_an_open_box_group_keeps_the_copy_open() {
    let (mut s, i) = open_box_group(DefinitionKind::Group);
    s.copy_instances(MeshId::Root, &[i], p(10.0, 0.0, 0.0), 1)
        .unwrap();
    s.validate().unwrap();
    for (_, d) in s.definitions() {
        assert_eq!(counts(&d.mesh), (8, 12, 5, 0));
    }
    assert_eq!(s.definitions().count(), 2);
}

#[test]
fn an_instance_listed_twice_is_copied_once() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Component);
    let change = s
        .copy_instances(MeshId::Root, &[i, i], p(3.0, 0.0, 0.0), 1)
        .unwrap();
    assert_eq!(change.summary().created.len(), 1);
}

#[test]
fn exploding_bare_edges_onto_a_face_keeps_the_piece_they_cut_out() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(1.0, 1.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(1.0, 2.0, 0.0),
    ])
    .unwrap();
    let mut s = Scene::from_root(m);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let d = definition_of(&s, i);
    let inner = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
    s.edit(MeshId::Definition(d), |m| Ok(m.erase(&[], &[inner])))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ])
    })
    .unwrap();
    assert_eq!(root_counts(&s), (4, 4, 1, 1));
    let before = s.clone();
    let change = s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    // The outline splits the face; the piece inside it stays a face.
    assert_eq!(root_counts(&s), (8, 8, 2, 0));
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn exploding_a_component_with_a_group_inside_keeps_groups_single_use() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut s = Scene::from_root(m);
    let start = s.clone();
    let mut history = Vec::new();
    let (g, c) = group_all(&mut s, DefinitionKind::Group);
    history.push(c);
    let (comp, c) = s
        .make_group(MeshId::Root, &[], &[], &[g], DefinitionKind::Component, "C")
        .unwrap();
    history.push(c);
    history.push(
        s.copy_instances(MeshId::Root, &[comp], p(3.0, 0.0, 0.0), 2)
            .unwrap(),
    );
    let after_copy = s.clone();
    let change = s.explode(MeshId::Root, comp).unwrap();
    s.validate().unwrap();
    let after = s.clone();
    for (id, d) in s.definitions() {
        if d.kind == DefinitionKind::Group {
            assert_eq!(s.instance_count(id), 1, "group {id} is placed once");
        }
    }
    assert_eq!(root_counts(&s), (0, 0, 0, 3));
    s.apply(&change.inverse());
    assert_eq!(s, after_copy);
    s.apply(&change);
    assert_eq!(s, after);
    s.apply(&change.inverse());
    for c in history.iter().rev() {
        s.apply(&c.inverse());
    }
    assert_eq!(s, start);
}

#[test]
fn exploding_a_group_with_a_group_inside_reuses_its_definition() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
    let d = definition_of(&s, inner);
    let (outer, _) = s
        .make_group(MeshId::Root, &[], &[], &[inner], DefinitionKind::Group, "")
        .unwrap();
    s.explode(MeshId::Root, outer).unwrap();
    s.validate().unwrap();
    assert_eq!(s.definitions().count(), 1);
    assert_eq!(s.instance_count(d), 1);
}

/// A quarter turn counter-clockwise about Z through the origin, exact.
fn quarter_turn() -> DAffine3 {
    DAffine3::from_cols(DVec3::Y, -DVec3::X, DVec3::Z, DVec3::ZERO)
}

#[test]
fn turning_an_instance_turns_its_bounds_and_undoes_exactly() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let before = s.clone();
    let change = s
        .edit(MeshId::Root, |m| {
            Ok(m.transform_instances(&[i, i], quarter_turn()))
        })
        .unwrap();
    assert_eq!(s.root().instance(i).unwrap().transform, quarter_turn());
    assert_eq!(change.summary().modified, vec![EntityId::Instance(i)]);
    assert_eq!(
        s.bounds(MeshId::Root),
        Some([p(-3.0, 0.0, 0.0), p(0.0, 2.0, 1.0)])
    );
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn copies_placed_by_turns_keep_the_original_transform_inside() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Component);
    let shift = DAffine3::from_translation(p(1.0, 0.0, 0.0));
    s.edit(MeshId::Root, |m| {
        Ok(m.move_instances(&[i], p(1.0, 0.0, 0.0)))
    })
    .unwrap();
    let half = quarter_turn() * quarter_turn();
    s.copy_instances_placed(MeshId::Root, &[i], &[quarter_turn(), half])
        .unwrap();
    s.validate().unwrap();
    let mut transforms: Vec<DAffine3> = s.root().instances().map(|(_, i)| i.transform).collect();
    transforms.sort_by(|a, b| a.translation.x.total_cmp(&b.translation.x));
    assert_eq!(
        transforms,
        vec![half * shift, quarter_turn() * shift, shift],
        "each copy is the original turned"
    );
}

#[test]
fn copying_or_erasing_instances_of_a_missing_mesh_is_refused_without_a_panic() {
    let mut s = box_scene(p(0.0, 0.0, 0.0));
    let (group, _) = group_all(&mut s, DefinitionKind::Group);
    let before = s.clone();
    let missing = MeshId::Definition(kernel::DefinitionId(999_999));
    assert_eq!(
        s.copy_instances(missing, &[group], p(5.0, 0.0, 0.0), 1),
        Err(kernel::KernelError::DefinitionNotFound)
    );
    assert!(s.erase_instances(missing, &[group]).is_empty());
    assert_eq!(s, before);
}

/// A component "Chair" placed twice, holding one instance of a box made as `inner_kind`.
fn two_chairs(inner_kind: DefinitionKind) -> (Scene, [InstanceId; 2]) {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, inner_kind);
    let (chair, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[inner],
            DefinitionKind::Component,
            "Chair",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[chair], p(5.0, 0.0, 0.0), 1)
        .unwrap();
    let ids: Vec<InstanceId> = s.root().instances().map(|(i, _)| i).collect();
    (s, [ids[0], ids[1]])
}

#[test]
fn make_unique_gives_the_instance_a_copy_with_nested_groups_copied() {
    let (mut s, [a, b]) = two_chairs(DefinitionKind::Group);
    let shared = definition_of(&s, a);
    let inner_before = s
        .definition(shared)
        .unwrap()
        .mesh
        .instances()
        .next()
        .unwrap()
        .1
        .definition;
    let before = s.clone();
    let (made, change) = s.make_unique(MeshId::Root, &[b]).unwrap();
    s.validate().unwrap();
    assert_eq!(made.len(), 1);
    assert_eq!(definition_of(&s, a), shared);
    assert_eq!(definition_of(&s, b), made[0]);
    assert_eq!(s.definition(made[0]).unwrap().name, "Chair#2");
    assert_eq!(
        s.definition(made[0]).unwrap().kind,
        DefinitionKind::Component
    );
    let inner_after = s
        .definition(made[0])
        .unwrap()
        .mesh
        .instances()
        .next()
        .unwrap()
        .1
        .definition;
    assert_ne!(inner_after, inner_before, "a nested group is copied");
    assert_eq!(s.instance_count(shared), 1);
    // Nothing else changed: the first chair and its group are as they were.
    assert_eq!(
        s.definition(shared).unwrap().mesh,
        before.definition(shared).unwrap().mesh
    );
    assert_eq!(
        s.root().instance(b).unwrap().transform,
        before.root().instance(b).unwrap().transform
    );
    s.apply(&change.inverse());
    s.validate().unwrap();
    assert_eq!(s.definitions().count(), before.definitions().count());
    assert_eq!(definition_of(&s, b), shared);
    assert!(s.definition(made[0]).is_none());
}

#[test]
fn make_unique_shares_nested_components() {
    let (mut s, [_, b]) = two_chairs(DefinitionKind::Component);
    let shared = definition_of(&s, b);
    let inner = s
        .definition(shared)
        .unwrap()
        .mesh
        .instances()
        .next()
        .unwrap()
        .1
        .definition;
    let (made, _) = s.make_unique(MeshId::Root, &[b]).unwrap();
    let copied = s
        .definition(made[0])
        .unwrap()
        .mesh
        .instances()
        .next()
        .unwrap()
        .1
        .definition;
    assert_eq!(copied, inner);
    assert_eq!(s.instance_count(inner), 2);
}

#[test]
fn make_unique_makes_one_copy_for_all_selected_instances_of_a_definition() {
    let (mut s, [a, b]) = two_chairs(DefinitionKind::Group);
    let (made, _) = s.make_unique(MeshId::Root, &[a, b, a]).unwrap();
    assert_eq!(made.len(), 1);
    assert_eq!(definition_of(&s, a), definition_of(&s, b));
    assert_eq!(s.instance_count(made[0]), 2);
}

#[test]
fn make_unique_leaves_groups_and_sole_instances_alone() {
    let mut s = box_scene(DVec3::ZERO);
    let (g, _) = group_all(&mut s, DefinitionKind::Group);
    let before = s.clone();
    let (made, change) = s.make_unique(MeshId::Root, &[g]).unwrap();
    assert!(made.is_empty());
    assert!(change.is_empty());
    assert_eq!(s.definitions().count(), before.definitions().count());
    let (mut s, [a, b]) = two_chairs(DefinitionKind::Group);
    s.erase_instances(MeshId::Root, &[a]);
    let (made, change) = s.make_unique(MeshId::Root, &[b]).unwrap();
    assert!(made.is_empty() && change.is_empty());
}

#[test]
fn unique_names_count_up_from_two_and_drop_a_trailing_number() {
    let (mut s, [a, b]) = two_chairs(DefinitionKind::Group);
    assert_eq!(s.unique_name("Chair"), "Chair#2");
    assert_eq!(s.unique_name("Chair#7"), "Chair#2");
    assert_eq!(s.unique_name("Chair #"), "Chair ##2");
    s.make_unique(MeshId::Root, &[b]).unwrap();
    assert_eq!(s.unique_name("Chair"), "Chair#3");
    assert_eq!(s.unique_name("Chair#2"), "Chair#3");
    let _ = a;
}
