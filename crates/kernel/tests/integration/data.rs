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
fn the_section_cut_flag_survives_a_round_trip_and_copies_and_is_absent_when_on() {
    let mut s = scene();
    let id = s.root().instances().next().unwrap().0;
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(!json.contains("section_cut"), "the default is not written");
    s.edit(MeshId::Root, |m| {
        Ok(
            m.set_section_cut_entities(&[kernel::EntityId::Instance(id)], false)
                .0,
        )
    })
    .unwrap();
    assert_eq!(s.root().section_cut_of(id), Some(false));
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(json.contains("\"section_cut\":false"));
    assert_eq!(round_trip(&s), s);
    // Copies keep it.
    let before: Vec<_> = s.root().instances().map(|(i, _)| i).collect();
    s.copy_instances(MeshId::Root, &[id], p(10.0, 0.0, 0.0), 1)
        .unwrap();
    let copy = s
        .root()
        .instances()
        .map(|(i, _)| i)
        .find(|i| !before.contains(i))
        .unwrap();
    assert_eq!(s.root().section_cut_of(copy), Some(false));
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
        "two entities of different kinds share an id",
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
                material: None,
                tag: None,
                hidden: false,
                section_cut: true,
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

/// Plain data for a root mesh of loose vertices at these ids and positions.
fn loose_vertices(next_id: u64, vertices: &[(u64, [f64; 3])]) -> SceneData {
    SceneData {
        next_id,
        root: kernel::MeshData {
            vertices: vertices
                .iter()
                .map(|&(id, position)| kernel::VertexData { id, position })
                .collect(),
            ..Default::default()
        },
        definitions: vec![],
        materials: vec![],
        tags: Vec::new(),
        pages: Vec::new(),
        styles: Vec::new(),
        axes: None,
        location: None,
    }
}

#[test]
fn ids_beyond_exact_json_numbers_are_rejected() {
    rejects(|d| d.root.vertices[0].id = u64::MAX, "too large");
    rejects(|d| d.root.edges[0].id = kernel::MAX_ID + 1, "too large");
    rejects(|d| d.root.edges[0].id = kernel::MAX_ID, "too large");
    rejects(
        |d| d.root.edges[0].curve = Some(kernel::MAX_ID),
        "too large",
    );
    rejects(|d| d.next_id = u64::MAX, "too large");
    rejects(|d| d.next_id = kernel::MAX_ID + 1, "too large");
    rejects(|d| d.definitions[0].id = u64::MAX, "too large");
    let err = Scene::from_data(&loose_vertices(
        0,
        &[(3, [0.0; 3]), (u64::MAX, [1.0, 0.0, 0.0])],
    ))
    .unwrap_err();
    assert!(err.contains("too large"), "{err}");
}

#[test]
fn ids_at_the_limit_load_save_and_load_again() {
    let mut data = scene().to_data();
    data.next_id = kernel::MAX_ID;
    let s = Scene::from_data(&data).unwrap();
    assert_eq!(s.to_data().next_id, kernel::MAX_ID);
    let last = loose_vertices(0, &[(3, [0.0; 3]), (kernel::MAX_ID - 1, [1.0, 0.0, 0.0])]);
    let s = Scene::from_data(&last).unwrap();
    let saved = s.to_data();
    assert_eq!(saved.next_id, kernel::MAX_ID);
    assert_eq!(Scene::from_data(&saved).unwrap(), s);
}

#[test]
fn ids_at_the_limit_load_and_keep_working() {
    let data = loose_vertices(0, &[(3, [0.0; 3]), (kernel::MAX_ID - 1, [1.0, 0.0, 0.0])]);
    let mut s = Scene::from_data(&data).unwrap();
    let change = s
        .edit(MeshId::Root, |m| {
            m.add_edge(p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0))
        })
        .unwrap();
    let created = change.summary().created;
    assert!(!created.is_empty());
    assert!(s.root().vertices().any(|(id, _)| id.0 == 3), "v3 kept");
    assert!(
        s.root()
            .vertices()
            .any(|(id, _)| id.0 == kernel::MAX_ID - 1),
        "the largest id kept"
    );
    s.validate().unwrap();
}

#[test]
fn coordinates_survive_a_json_round_trip_bit_for_bit() {
    // Numbers that serde_json parses one unit in the last place off without float_roundtrip.
    let tricky = [
        [180.17933438838418, -930.0397635799367, 474.47532350305437],
        [-452.10277274836835, -420.07796691370356, 0.1 + 0.2],
    ];
    let data = loose_vertices(0, &[(1, tricky[0]), (2, tricky[1])]);
    let s = Scene::from_data(&data).unwrap();
    let back = round_trip(&s);
    for (id, v) in back.root().vertices() {
        let want = tricky[id.0 as usize - 1];
        assert_eq!(
            v.pos.to_array().map(f64::to_bits),
            want.map(f64::to_bits),
            "v{}",
            id.0
        );
    }
    assert_eq!(back, s);
}

/// `scene()` with a circle drawn in the root, a guide in the root and a material.
fn scene_with_every_kind() -> Scene {
    let mut s = scene();
    let ring: Vec<DVec3> = (0..12)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 12.0;
            p(-5.0 + a.cos(), a.sin(), 0.0)
        })
        .collect();
    s.edit(MeshId::Root, |m| m.add_curve(&ring, true, false))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_guide(kernel::Guide::Point {
            position: p(0.0, 0.0, 5.0),
            tag: None,
            hidden: false,
        })
    })
    .unwrap();
    s.add_material("Red", [200, 0, 0]).unwrap();
    s.validate().unwrap();
    s
}

#[test]
fn no_number_is_the_id_of_two_things() {
    let s = scene_with_every_kind();
    assert_eq!(round_trip(&s), s);
    let data = s.to_data();
    let curve = data.root.edges.iter().find_map(|e| e.curve).unwrap();
    let definition = data.definitions[0].id;
    let inner_vertex = data.definitions[0].mesh.vertices[0].id;
    let inner_face = data.definitions[0].mesh.faces[0].id;
    let root_vertex = data.root.vertices[0].id;
    let recurve = move |d: &mut SceneData, to: u64| {
        for e in &mut d.root.edges {
            if e.curve == Some(curve) {
                e.curve = Some(to);
            }
        }
    };
    type Change = Box<dyn Fn(&mut SceneData)>;
    let changes: Vec<(&str, Change, String)> = vec![
        (
            "curve = definition",
            Box::new(move |d| recurve(d, definition)),
            format!("d{definition} and c{definition} share an id"),
        ),
        (
            "curve = vertex in a definition",
            Box::new(move |d| recurve(d, inner_vertex)),
            format!("c{inner_vertex} and v{inner_vertex} share an id"),
        ),
        (
            "material = face in a definition",
            Box::new(move |d| d.materials[0].id = inner_face),
            format!("m{inner_face} and f{inner_face} share an id"),
        ),
        (
            "material = definition",
            Box::new(move |d| d.materials[0].id = definition),
            format!("d{definition} and m{definition} share an id"),
        ),
        (
            "guide in a definition = vertex in the root",
            Box::new(move |d| {
                d.definitions[0].mesh.guides.push(kernel::GuideData {
                    id: root_vertex,
                    point: [0.0, 0.0, 9.0],
                    direction: None,
                    tag: None,
                    hidden: false,
                })
            }),
            format!("v{root_vertex} and g{root_vertex} share an id"),
        ),
    ];
    for (what, change, expected) in changes {
        let mut d = data.clone();
        change(&mut d);
        let err = Scene::from_data(&d).unwrap_err();
        assert_eq!(err, expected, "{what}");
    }
}

#[test]
fn transforms_that_cannot_be_inverted_are_rejected() {
    rejects(
        |d| d.root.instances[0].transform = [0.0; 12],
        "not invertible",
    );
    rejects(
        |d| {
            // Squash the z axis flat.
            let t = &mut d.root.instances[0].transform;
            t[6..9].copy_from_slice(&[0.0, 0.0, 1e-15]);
        },
        "not invertible",
    );
}

#[test]
fn many_vertices_validate() {
    // 20k vertices 1 mm apart on a grid, then one more within tolerance of one of them,
    // straddling a tolerance-sized cell boundary.
    let mut vertices = Vec::new();
    for i in 0..20_000u64 {
        let (x, y, z) = (i % 100, (i / 100) % 20, i / 2000);
        vertices.push((i + 1, [x as f64 * 1e-3, y as f64 * 1e-3, z as f64 * 1e-3]));
    }
    let s = Scene::from_data(&loose_vertices(0, &vertices)).unwrap();
    assert_eq!(s.root().vertices().count(), 20_000);
    vertices.push((50_000, [-0.4e-6, 0.0, 0.0]));
    let err = Scene::from_data(&loose_vertices(0, &vertices)).unwrap_err();
    assert!(err.contains("coincide"), "{err}");
}

/// Plain data for a root placing a chain of `n` nested groups; when `cycle`, the last group
/// places the first.
fn nested(n: u64, cycle: bool) -> SceneData {
    let instance = |id: u64, definition: u64| kernel::InstanceData {
        id,
        definition,
        transform: glam::DAffine3::IDENTITY.to_cols_array(),
        material: None,
        tag: None,
        hidden: false,
        section_cut: true,
    };
    let definitions = (0..n)
        .map(|k| kernel::DefinitionData {
            id: k + 1,
            name: String::new(),
            kind: DefinitionKind::Group,
            mesh: kernel::MeshData {
                instances: if k + 1 < n {
                    vec![instance(n + k + 2, k + 2)]
                } else if cycle {
                    vec![instance(n + k + 2, 1)]
                } else {
                    vec![]
                },
                ..Default::default()
            },
        })
        .collect();
    SceneData {
        next_id: 0,
        root: kernel::MeshData {
            instances: vec![instance(n + 1, 1)],
            ..Default::default()
        },
        definitions,
        materials: vec![],
        tags: Vec::new(),
        pages: Vec::new(),
        styles: Vec::new(),
        axes: None,
        location: None,
    }
}

#[test]
fn deeply_nested_groups_validate() {
    let s = Scene::from_data(&nested(2000, false)).unwrap();
    assert_eq!(s.definitions().count(), 2000);
    let err = Scene::from_data(&nested(2000, true)).unwrap_err();
    assert!(err.contains("places itself"), "{err}");
}
