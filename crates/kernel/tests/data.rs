//! Scenes as plain data (for files): round trips, rejected data, renaming and purging.

use glam::DVec3;
use kernel::{DefinitionKind, KernelError, MeshId, Model, Scene, SceneData};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A box grouped and made a component, copied twice, plus a loose edge in the root.
fn scene() -> Scene {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
    ])
    .unwrap();
    let base = m.faces().next().unwrap().0;
    m.push_pull(base, 1.0).unwrap();
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (group, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    let (crate_, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[group],
            DefinitionKind::Component,
            "Crate",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[crate_], p(3.0, 0.0, 0.0), 2)
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_edge(p(0.0, -2.0, 0.0), p(1.0, -2.0, 0.0))
    })
    .unwrap();
    s.validate().unwrap();
    s
}

fn round_trip(s: &Scene) -> Scene {
    let json = serde_json::to_string(&s.to_data()).unwrap();
    let data: SceneData = serde_json::from_str(&json).unwrap();
    Scene::from_data(&data).unwrap()
}

#[test]
fn a_scene_survives_a_round_trip_with_its_ids() {
    let s = scene();
    let back = round_trip(&s);
    assert_eq!(back, s);
    assert_eq!(back.to_data(), s.to_data());
}

#[test]
fn ids_after_loading_continue_where_they_stopped() {
    let mut s = scene();
    // Undoing an edit leaves its ids used: they must stay unused after a round trip too.
    let undone = s
        .edit(MeshId::Root, |m| {
            m.add_edge(p(5.0, 5.0, 0.0), p(6.0, 5.0, 0.0))
        })
        .unwrap();
    s.apply(&undone.inverse());
    let mut back = round_trip(&s);
    let fresh = back
        .edit(MeshId::Root, |m| {
            m.add_edge(p(7.0, 5.0, 0.0), p(8.0, 5.0, 0.0))
        })
        .unwrap()
        .summary()
        .created;
    for id in fresh {
        assert!(!undone.summary().created.contains(&id), "{id} reused");
    }
}

/// Plain data for `scene()`, changed by `f`, must be rejected with a message containing
/// `expected`.
fn rejects(f: impl FnOnce(&mut SceneData), expected: &str) {
    let mut data = scene().to_data();
    f(&mut data);
    let err = Scene::from_data(&data).unwrap_err();
    assert!(err.contains(expected), "{err:?} lacks {expected:?}");
}

#[test]
fn broken_data_is_rejected() {
    rejects(
        |d| {
            let v = d.root.vertices[0].clone();
            d.root.vertices.push(v);
        },
        "appears twice",
    );
    rejects(|d| d.root.edges[0].vertices[0] = 999_999, "missing vertex");
    rejects(|d| d.root.vertices[0].position[0] = f64::NAN, "not finite");
    rejects(
        |d| d.root.instances[0].definition = 999_999,
        "missing definition",
    );
    rejects(
        |d| {
            let first = d.root.instances[0].id;
            d.root.edges[0].id = first;
        },
        "share an id",
    );
    rejects(
        |d| {
            // The component places its group; make the group place the component as well.
            let component = d
                .definitions
                .iter()
                .find(|x| x.kind == DefinitionKind::Component)
                .unwrap()
                .id;
            let group = d
                .definitions
                .iter_mut()
                .find(|x| x.kind == DefinitionKind::Group)
                .unwrap();
            group.mesh.instances.push(kernel::InstanceData {
                id: 999_999,
                definition: component,
                transform: glam::DAffine3::IDENTITY.to_cols_array(),
            });
        },
        "places itself",
    );
}

#[test]
fn renaming_a_definition_undoes_exactly() {
    let mut s = scene();
    let (id, _) = s
        .definitions()
        .find(|(_, d)| d.kind == DefinitionKind::Component)
        .unwrap();
    let before = s.clone();
    let change = s.rename_definition(id, "Box").unwrap();
    assert_eq!(s.definition(id).unwrap().name, "Box");
    assert!(change.summary().created.is_empty() && change.summary().modified.is_empty());
    assert!(s.rename_definition(id, "Box").unwrap().is_empty());
    s.apply(&change.inverse());
    assert_eq!(s, before);
    assert_eq!(
        s.rename_definition(kernel::DefinitionId(999_999), "x"),
        Err(KernelError::DefinitionNotFound)
    );
}

#[test]
fn purging_removes_unused_definitions_and_what_only_they_place() {
    let mut s = scene();
    let instances: Vec<_> = s.root().instances().map(|(id, _)| id).collect();
    s.erase_instances(MeshId::Root, &instances);
    // The component stays without instances, and keeps its group alive.
    assert_eq!(s.definitions().count(), 2);
    let before = s.clone();
    let change = s.purge();
    assert_eq!(s.definitions().count(), 0);
    s.validate().unwrap();
    s.apply(&change.inverse());
    assert_eq!(s, before);
    let mut used = scene();
    assert!(used.purge().is_empty(), "everything is placed");
}
