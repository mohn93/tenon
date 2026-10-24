//! Joints: the scene's list of connections between root instances, how erasing a part reaches
//! them, and how they are kept in files.

use glam::{DAffine3, DVec3};
use kernel::{
    DefinitionKind, EntityId, InstanceId, Joint, JointFrame, JointId, JointKind, KernelError,
    MeshId, Model, Scene, SceneData,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A scene with a unit box component placed at the origin and at x = 5 and 10; returns the
/// three instances.
fn parts() -> (Scene, [InstanceId; 3]) {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
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
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (first, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Box",
        )
        .unwrap();
    let d = s.root().instance(first).unwrap().definition;
    let at = |x: f64| DAffine3::from_translation(p(x, 0.0, 0.0));
    let (b, _) = s.place_instance(MeshId::Root, d, at(5.0)).unwrap();
    let (c, _) = s.place_instance(MeshId::Root, d, at(10.0)).unwrap();
    (s, [first, b, c])
}

fn frame() -> JointFrame {
    JointFrame {
        origin: DVec3::ZERO,
        axis: DVec3::Z,
        reference: DVec3::X,
    }
}

fn joint(name: &str, a: Option<InstanceId>, b: InstanceId) -> Joint {
    Joint {
        name: name.into(),
        kind: JointKind::Revolute,
        a,
        b,
        frame_a: frame(),
        frame_b: frame(),
        value: 0.0,
        slide: 0.0,
        limits: None,
    }
}

#[test]
fn joints_get_ids_from_the_shared_counter_and_undo() {
    let (mut s, [a, b, _]) = parts();
    let before = s.clone();
    let (id, change) = s.add_joint(joint("Hinge", Some(a), b)).unwrap();
    assert_eq!(id.to_string(), format!("j{}", id.0));
    assert!(s.joint(id).is_some());
    assert_eq!(change.summary().joints, vec![id]);
    s.validate().unwrap();
    let (next, _) = s.add_joint(joint("Other", None, b)).unwrap();
    assert!(next.0 > id.0);

    let mut undone = s.clone();
    undone.apply(&change.inverse());
    assert!(undone.joint(id).is_none());
    s.apply(&change.inverse());
    s.apply(&change);
    assert!(s.joint(id).is_some());
    assert_eq!(before.joints().count(), 0);
}

#[test]
fn refusals_name_the_problem() {
    let (mut s, [a, b, _]) = parts();
    let refused = |s: &mut Scene, j: Joint| match s.add_joint(j) {
        Err(KernelError::Joint(why)) => why,
        other => panic!("expected a joint error, got {other:?}"),
    };
    assert!(refused(&mut s, joint("x", Some(a), a)).contains("two different"));
    assert!(refused(&mut s, joint("x", Some(InstanceId(9999)), b)).contains("not an instance"));
    assert!(refused(&mut s, joint(" ", Some(a), b)).contains("needs a name"));
    let mut bad = joint("x", Some(a), b);
    bad.frame_b.axis = DVec3::new(0.0, 0.0, 2.0);
    assert!(refused(&mut s, bad).contains("unit"));
    let mut bad = joint("x", Some(a), b);
    bad.frame_a.reference = DVec3::Z;
    assert!(refused(&mut s, bad).contains("perpendicular"));
    let mut bad = joint("x", Some(a), b);
    bad.limits = Some((1.0, 2.0));
    assert!(refused(&mut s, bad).contains("current value"));
    let mut bad = joint("x", Some(a), b);
    bad.limits = Some((1.0, -1.0));
    assert!(refused(&mut s, bad).contains("least below"));
    let mut bad = joint("x", Some(a), b);
    bad.kind = JointKind::Fixed;
    bad.limits = Some((-1.0, 1.0));
    assert!(refused(&mut s, bad).contains("no value"));
    let mut bad = joint("x", Some(a), b);
    bad.slide = 1.0;
    assert!(refused(&mut s, bad).contains("slides"));
    s.add_joint(joint("Hinge", Some(a), b)).unwrap();
    assert!(refused(&mut s, joint("Hinge", None, b)).contains("already exists"));
    assert_eq!(s.joints().count(), 1);
}

#[test]
fn names_are_trimmed_and_free_names_take_the_lowest_number() {
    let (mut s, [a, b, c]) = parts();
    assert_eq!(s.free_joint_name(), "Joint 1");
    s.add_joint(joint("Joint 1", Some(a), b)).unwrap();
    s.add_joint(joint("  Joint 3 ", Some(b), c)).unwrap();
    assert_eq!(s.free_joint_name(), "Joint 2");
    assert!(s.joints().any(|(_, j)| j.name == "Joint 3"));
}

#[test]
fn changing_and_deleting_are_one_step_and_nothing_to_change_is_none() {
    let (mut s, [a, b, _]) = parts();
    let (id, _) = s.add_joint(joint("Hinge", Some(a), b)).unwrap();
    let same = s.joint(id).unwrap().clone();
    assert!(s.change_joint(id, same.clone()).unwrap().is_empty());
    let mut to = same.clone();
    to.name = "Elbow".into();
    to.limits = Some((-1.0, 1.0));
    let change = s.change_joint(id, to).unwrap();
    assert_eq!(change.steps.len(), 1);
    assert_eq!(s.joint(id).unwrap().name, "Elbow");
    s.apply(&change.inverse());
    assert_eq!(s.joint(id).unwrap(), &same);
    let gone = s.delete_joint(id).unwrap();
    assert!(s.joint(id).is_none());
    assert_eq!(gone.summary().joints, vec![id]);
    s.apply(&gone.inverse());
    assert_eq!(s.joint(id).unwrap(), &same);
    assert_eq!(
        s.delete_joint(JointId(99999)).unwrap_err(),
        KernelError::JointNotFound
    );
}

#[test]
fn erasing_a_part_drops_its_joints_in_the_same_change() {
    let (mut s, [a, b, c]) = parts();
    let (ab, _) = s.add_joint(joint("AB", Some(a), b)).unwrap();
    let (bc, _) = s.add_joint(joint("BC", Some(b), c)).unwrap();
    let (ground, _) = s.add_joint(joint("Ground", None, a)).unwrap();
    let before = s.clone();
    let change = s.erase_instances(MeshId::Root, &[a]);
    assert!(s.joint(ab).is_none() && s.joint(ground).is_none());
    assert!(s.joint(bc).is_some());
    let listed = change.summary().joints;
    assert!(listed.contains(&ab) && listed.contains(&ground) && !listed.contains(&bc));
    s.validate().unwrap();
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn world_and_local_frames_are_inverses_even_for_a_mirrored_part() {
    let mirror = DAffine3::from_cols(
        DVec3::new(-2.0, 0.0, 0.0),
        DVec3::new(0.0, 2.0, 0.0),
        DVec3::new(0.0, 0.0, 2.0),
        p(3.0, 1.0, 0.5),
    );
    let world = JointFrame {
        origin: p(1.0, 2.0, 3.0),
        axis: DVec3::new(1.0, 1.0, 0.0).normalize(),
        reference: DVec3::Z,
    };
    let local = JointFrame::local(&world, &mirror).unwrap();
    let back = local.world(&mirror);
    assert!((back.origin - world.origin).length() < 1e-12);
    assert!((back.axis - world.axis).length() < 1e-12);
    assert!((back.reference - world.reference).length() < 1e-12);
    assert!((local.axis.length() - 1.0).abs() < 1e-12);
    assert!(local.axis.dot(local.reference).abs() < 1e-12);
}

#[test]
fn joints_round_trip_through_data() {
    let (mut s, [a, b, _]) = parts();
    let mut j = joint("Slide", Some(a), b);
    j.kind = JointKind::Cylindrical;
    j.value = 0.25;
    j.slide = 0.5;
    j.limits = Some((-1.0, 1.0));
    s.add_joint(j).unwrap();
    s.add_joint(joint("Ground", None, a)).unwrap();
    let data = s.to_data();
    assert_eq!(data.joints.len(), 2);
    let json = serde_json::to_string(&data).unwrap();
    let again = Scene::from_data(&serde_json::from_str::<SceneData>(&json).unwrap()).unwrap();
    assert_eq!(again, s);
    assert_eq!(again.to_data(), data);

    let mut bad = data.clone();
    bad.joints[0].b = 424242;
    assert!(
        Scene::from_data(&bad)
            .unwrap_err()
            .contains("not an instance")
    );
    let mut bad = data.clone();
    bad.joints[1].id = bad.joints[0].id;
    assert!(Scene::from_data(&bad).is_err());
    let mut bad = data;
    bad.joints[1].name = "Slide".into();
    assert!(
        Scene::from_data(&bad)
            .unwrap_err()
            .contains("named the same")
    );
}
