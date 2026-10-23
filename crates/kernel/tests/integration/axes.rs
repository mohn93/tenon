use glam::{DAffine3, DQuat, DVec3};
use kernel::{Axes, DefinitionKind, EntityId, InstanceId, KernelError, MeshId, Model, Scene};

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
    Scene::from_root(m)
}

/// Every vertex of the model, in root coordinates, once per way it is placed, sorted.
fn world_points(s: &Scene) -> Vec<[i64; 3]> {
    fn walk(s: &Scene, mesh: MeshId, to_root: DAffine3, out: &mut Vec<DVec3>) {
        let m = s.mesh(mesh).unwrap();
        out.extend(m.vertices().map(|(_, v)| to_root.transform_point3(v.pos)));
        for (_, i) in m.instances() {
            walk(
                s,
                MeshId::Definition(i.definition),
                to_root * i.transform,
                out,
            );
        }
    }
    let mut points = Vec::new();
    walk(s, MeshId::Root, DAffine3::IDENTITY, &mut points);
    let mut rounded: Vec<[i64; 3]> = points
        .iter()
        .map(|p| [p.x, p.y, p.z].map(|c| (c * 1e6).round() as i64))
        .collect();
    rounded.sort();
    rounded
}

#[test]
fn axes_are_orthonormalized_and_bad_ones_refused() {
    let a = Axes::new(p(1.0, 2.0, 3.0), p(2.0, 0.0, 0.0), p(1.0, 3.0, 0.0)).unwrap();
    assert_eq!(a.x, DVec3::X);
    assert_eq!(a.y, DVec3::Y);
    assert_eq!(a.z, DVec3::Z);
    let a = Axes::new(DVec3::ZERO, p(0.0, 1.0, 0.0), p(-1.0, 0.0, 0.0)).unwrap();
    assert_eq!(a.z, DVec3::Z);
    assert!(Axes::WORLD.is_world());
    assert!(!a.is_world());
    for (x, y) in [
        (DVec3::ZERO, DVec3::Y),
        (DVec3::X, DVec3::ZERO),
        (DVec3::X, p(-3.0, 0.0, 0.0)),
        (DVec3::X, p(f64::NAN, 0.0, 0.0)),
    ] {
        assert!(Axes::new(DVec3::ZERO, x, y).is_err());
    }
    assert!(matches!(
        Axes::new(DVec3::ZERO, DVec3::X, DVec3::X),
        Err(KernelError::BadAxes(_))
    ));
}

#[test]
fn moving_the_model_axes_undoes_exactly_and_survives_data() {
    let mut s = box_scene(DVec3::ZERO);
    let before = s.clone();
    let a = Axes::new(p(1.0, 2.0, 3.0), p(1.0, 1.0, 0.0), p(-1.0, 1.0, 0.0)).unwrap();
    let change = s.set_axes(a);
    assert_eq!(s.axes(), a);
    assert!(s.set_axes(a).is_empty());
    let data = s.to_data();
    assert!(data.axes.is_some());
    assert_eq!(Scene::from_data(&data).unwrap().axes(), a);
    assert!(box_scene(DVec3::ZERO).to_data().axes.is_none());
    s.apply(&change.inverse());
    assert_eq!(s, before);
    let mut bad = data.clone();
    bad.axes.as_mut().unwrap().y = [1.0, 1.0, 0.0];
    assert!(Scene::from_data(&bad).is_err());
}

#[test]
fn a_definition_axes_move_without_moving_anything_in_the_world() {
    let mut s = box_scene(p(4.0, 1.0, 0.0));
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (chair, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Chair",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[chair], p(0.0, 10.0, 0.0), 1)
        .unwrap();
    let chairs: Vec<InstanceId> = s.root().instances().map(|(i, _)| i).collect();
    // A component holding both chairs, placed twice (turned the second time).
    let (pair, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &chairs,
            DefinitionKind::Component,
            "Pair",
        )
        .unwrap();
    s.copy_instances_placed(
        MeshId::Root,
        &[pair],
        &[DAffine3::from_rotation_translation(
            DQuat::from_rotation_z(0.7),
            p(-20.0, 5.0, 2.0),
        )],
    )
    .unwrap();
    s.validate().unwrap();
    let before = s.clone();
    let world = world_points(&s);
    let pair_def = s.root().instance(pair).unwrap().definition;
    let chair_def = s
        .definition(pair_def)
        .unwrap()
        .mesh
        .instances()
        .next()
        .unwrap()
        .1
        .definition;
    // An arbitrary rigid frame in the chair's coordinates.
    let frame = DAffine3::from_rotation_translation(
        DQuat::from_axis_angle(p(1.0, 2.0, 3.0).normalize(), 0.9),
        p(4.5, 1.5, 0.25),
    );
    let change = s.reorigin_definition(chair_def, frame).unwrap();
    s.validate().unwrap();
    assert_ne!(s, before);
    assert_eq!(world_points(&s), world, "nothing moves in the world");
    // The definition's vertices moved by the frame's inverse.
    let some = s
        .definition(chair_def)
        .unwrap()
        .mesh
        .vertices()
        .next()
        .unwrap()
        .1
        .pos;
    let was = before
        .definition(chair_def)
        .unwrap()
        .mesh
        .vertices()
        .next()
        .unwrap()
        .1
        .pos;
    assert!(some.distance(frame.inverse().transform_point3(was)) < 1e-9);
    s.apply(&change.inverse());
    assert_eq!(s, before);
    // A frame that is not a rigid motion is refused.
    assert!(
        s.reorigin_definition(chair_def, DAffine3::from_scale(p(2.0, 1.0, 1.0)))
            .is_err()
    );
}
