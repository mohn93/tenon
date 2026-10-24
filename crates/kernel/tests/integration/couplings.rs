//! Couplings: gear, chain and rack ratios between joints' values, held by the solver.

use std::f64::consts::PI;
use std::time::Instant;

use glam::{DAffine3, DQuat, DVec3};
use kernel::{
    Coupling, CouplingId, InstanceId, JointId, JointKind, KernelError, MeshId, Scene, SceneData,
};

use crate::solver::{
    L, R, check_engine, engine, join, p, parts, piston_x, slider_crank, transform, value, wrists,
};

fn deg(x: f64) -> f64 {
    x.to_radians()
}

/// The turn by `angle` about the Z axis through `pivot`.
fn turned_about(pivot: DVec3, angle: f64) -> DAffine3 {
    DAffine3::from_translation(pivot)
        * DAffine3::from_quat(DQuat::from_rotation_z(angle))
        * DAffine3::from_translation(-pivot)
}

const CAM_AT: DVec3 = DVec3::new(0.1, 0.0, 0.0);

/// Two parts on revolute joints to the ground about Z, a crank at the origin and a cam at
/// `CAM_AT`. Returns the scene, [crank, cam] and their joints.
fn two_shafts() -> (Scene, [InstanceId; 2], [JointId; 2]) {
    let (mut s, ids) = parts(2);
    let crank = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    let cam = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[1],
        CAM_AT,
        DVec3::Z,
        DVec3::X,
    );
    (s, [ids[0], ids[1]], [crank, cam])
}

fn couple(s: &mut Scene, driver: JointId, driven: JointId, ratio: f64, offset: f64) -> CouplingId {
    s.add_coupling(Coupling {
        driver,
        driven,
        ratio,
        offset,
    })
    .unwrap()
    .0
}

fn assert_near(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() < 1e-9, "{what}: {a} vs {b}");
}

#[test]
fn a_two_to_one_gear_train_turns_the_cam_once_in_two_crank_turns() {
    let (mut s, [crank, cam], [jc, jm]) = two_shafts();
    let k = couple(&mut s, jc, jm, 0.5, 0.0);
    assert!(s.coupling(k).is_some());
    s.drive_joint(jc, deg(90.0), None).unwrap();
    assert_near(value(&s, jm), deg(45.0), "cam at 90");
    assert!(transform(&s, cam).abs_diff_eq(turned_about(CAM_AT, deg(45.0)), 1e-9));
    assert!(transform(&s, crank).abs_diff_eq(turned_about(DVec3::ZERO, deg(90.0)), 1e-9));
    // Two whole turns in one drive: the cam's value counts on past a turn.
    s.drive_joint(jc, deg(720.0), None).unwrap();
    assert_eq!(value(&s, jc), deg(720.0));
    assert_near(value(&s, jm), deg(360.0), "cam at 720");
    assert!(transform(&s, cam).abs_diff_eq(DAffine3::IDENTITY, 1e-9));
    // And back down through every value.
    for k in (0..8).rev() {
        let t = deg(90.0 * k as f64);
        s.drive_joint(jc, t, None).unwrap();
        assert_near(value(&s, jm), t / 2.0, "cam on the way back");
        assert!(transform(&s, cam).abs_diff_eq(turned_about(CAM_AT, t / 2.0), 1e-9));
    }
}

#[test]
fn driving_the_driven_joint_turns_the_driver_and_a_negative_ratio_turns_against_it() {
    let (mut s, [crank, cam], [jc, jm]) = two_shafts();
    couple(&mut s, jc, jm, -1.0, 0.0);
    s.drive_joint(jc, deg(30.0), None).unwrap();
    assert_near(value(&s, jm), deg(-30.0), "meshed gears turn opposite");
    assert!(transform(&s, cam).abs_diff_eq(turned_about(CAM_AT, deg(-30.0)), 1e-9));
    s.drive_joint(jm, deg(200.0), None).unwrap();
    assert_near(
        value(&s, jc),
        deg(-200.0),
        "driven back through the coupling",
    );
    assert!(transform(&s, crank).abs_diff_eq(turned_about(DVec3::ZERO, deg(-200.0)), 1e-9));
}

#[test]
fn a_large_ratio_turns_the_follower_many_times_without_losing_count() {
    let (mut s, [_, cam], [jc, jm]) = two_shafts();
    couple(&mut s, jc, jm, 10.0, 0.0);
    // 10 degrees of crank turn the cam 100: the drive takes steps small enough for the cam.
    s.drive_joint(jc, deg(91.0), None).unwrap();
    assert_near(value(&s, jm), deg(910.0), "cam after 910 degrees");
    assert!(transform(&s, cam).abs_diff_eq(turned_about(CAM_AT, deg(910.0)), 1e-9));
}

#[test]
fn a_pinion_drives_a_rack_by_its_ratio() {
    let (mut s, ids) = parts(2);
    let [pinion, rack] = [ids[0], ids[1]];
    let jp = join(
        &mut s,
        JointKind::Revolute,
        None,
        pinion,
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    let jr = join(
        &mut s,
        JointKind::Prismatic,
        None,
        rack,
        p(0.0, -0.05, 0.0),
        DVec3::X,
        DVec3::Y,
    );
    // 0.01 m per degree, as meters per radian.
    couple(&mut s, jp, jr, 0.01 * 180.0 / PI, 0.0);
    s.drive_joint(jp, deg(90.0), None).unwrap();
    assert_near(value(&s, jr), 0.9, "rack after 90 degrees");
    let moved = transform(&s, rack).translation;
    assert!((moved - p(0.9, 0.0, 0.0)).length() < 1e-9, "{moved}");
    // The rack drives the pinion back.
    s.drive_joint(jr, -0.3, None).unwrap();
    assert_near(value(&s, jp), deg(-30.0), "pinion");
}

#[test]
fn an_offset_holds_the_values_the_joints_had_when_coupled() {
    let (mut s, [_, cam], [jc, jm]) = two_shafts();
    s.drive_joint(jc, deg(30.0), None).unwrap();
    s.drive_joint(jm, deg(100.0), None).unwrap();
    let before = s.clone();
    // The offset that holds now: 100 - 0.5 * 30.
    let offset = value(&s, jm) - 0.5 * value(&s, jc);
    let k = couple(&mut s, jc, jm, 0.5, offset);
    assert_eq!(s.coupling(k).unwrap().offset, offset);
    // Adding it moved nothing; a drive to the same value changes nothing.
    assert_eq!(s.root(), before.root());
    assert!(s.drive_joint(jc, deg(30.0), None).unwrap().is_empty());
    s.drive_joint(jc, deg(90.0), None).unwrap();
    assert_near(value(&s, jm), deg(130.0), "cam");
    assert!(transform(&s, cam).abs_diff_eq(turned_about(CAM_AT, deg(130.0)), 1e-9));
}

#[test]
fn contradictory_couplings_are_refused_at_drive_time_and_change_nothing() {
    let (mut s, ids) = parts(3);
    let j: Vec<JointId> = ids
        .iter()
        .enumerate()
        .map(|(k, i)| {
            join(
                &mut s,
                JointKind::Revolute,
                None,
                *i,
                p(0.1 * k as f64, 0.0, 0.0),
                DVec3::Z,
                DVec3::X,
            )
        })
        .collect();
    couple(&mut s, j[0], j[1], 2.0, 0.0);
    couple(&mut s, j[1], j[2], 2.0, 0.0);
    // j0 = 2 j2 and j2 = 4 j0: only 0 meets both.
    couple(&mut s, j[2], j[0], 2.0, 0.0);
    let before = s.clone();
    let err = s.drive_joint(j[0], deg(10.0), None).unwrap_err();
    assert!(
        matches!(&err, KernelError::Joint(why) if why.contains("cannot all be met")),
        "{err}"
    );
    assert_eq!(s, before);
}

#[test]
fn bad_couplings_are_refused() {
    let (mut s, ids) = parts(3);
    let a = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    let b = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[1],
        CAM_AT,
        DVec3::Z,
        DVec3::X,
    );
    let fixed = join(
        &mut s,
        JointKind::Fixed,
        Some(ids[1]),
        ids[2],
        CAM_AT,
        DVec3::Z,
        DVec3::X,
    );
    let try_add = |s: &mut Scene, driver, driven, ratio: f64, offset: f64| {
        let before = s.clone();
        let r = s.add_coupling(Coupling {
            driver,
            driven,
            ratio,
            offset,
        });
        if r.is_err() {
            assert_eq!(*s, before);
        }
        r.map(|_| ()).map_err(|e| e.to_string())
    };
    assert!(
        try_add(&mut s, a, b, 0.0, 0.0)
            .unwrap_err()
            .contains("ratio")
    );
    assert!(try_add(&mut s, a, b, f64::NAN, 0.0).is_err());
    assert!(
        try_add(&mut s, a, b, 1.0, f64::INFINITY)
            .unwrap_err()
            .contains("offset")
    );
    assert!(
        try_add(&mut s, a, a, 1.0, 0.0)
            .unwrap_err()
            .contains("two different")
    );
    assert!(
        try_add(&mut s, a, fixed, 1.0, 0.0)
            .unwrap_err()
            .contains("no value")
    );
    assert!(
        try_add(&mut s, a, JointId(9999), 1.0, 0.0)
            .unwrap_err()
            .contains("does not exist")
    );
    try_add(&mut s, a, b, 1.0, 0.0).unwrap();
    assert!(
        try_add(&mut s, b, a, 1.0, 0.0)
            .unwrap_err()
            .contains("already coupled")
    );
}

#[test]
fn deleting_a_joint_or_erasing_its_part_drops_its_couplings_and_undo_restores_them() {
    let (mut s, [crank, _], [jc, jm]) = two_shafts();
    let k = couple(&mut s, jc, jm, 0.5, 0.0);
    let before = s.clone();
    let change = s.delete_coupling(k).unwrap();
    assert_eq!(change.summary().couplings, vec![k]);
    assert!(s.coupling(k).is_none());
    assert!(matches!(
        s.delete_coupling(k),
        Err(KernelError::CouplingNotFound)
    ));
    s.apply(&change.inverse());
    assert_eq!(s, before);

    let change = s.delete_joint(jm).unwrap();
    assert!(s.coupling(k).is_none() && s.joint(jm).is_none());
    assert_eq!(change.summary().couplings, vec![k]);
    s.validate().unwrap();
    s.apply(&change.inverse());
    assert_eq!(s, before);
    s.validate().unwrap();

    let change = s.erase_instances(MeshId::Root, &[crank]);
    assert!(s.coupling(k).is_none() && s.joint(jc).is_none());
    assert!(change.summary().couplings.contains(&k));
    s.validate().unwrap();
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn undoing_a_coupled_drive_restores_everything() {
    let (mut s, _, [jc, jm]) = two_shafts();
    couple(&mut s, jc, jm, 0.5, 0.0);
    let before = s.clone();
    let change = s.drive_joint(jc, deg(400.0), None).unwrap();
    assert!(change.summary().joints.contains(&jm));
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn couplings_round_trip_through_data() {
    let (mut s, _, [jc, jm]) = two_shafts();
    couple(&mut s, jc, jm, -0.5, 0.25);
    let data = s.to_data();
    assert_eq!(data.couplings.len(), 1);
    let json = serde_json::to_string(&data).unwrap();
    let again = Scene::from_data(&serde_json::from_str::<SceneData>(&json).unwrap()).unwrap();
    assert_eq!(again, s);
    assert_eq!(again.to_data(), data);

    let mut bad = data.clone();
    bad.couplings[0].driven = 424242;
    assert!(
        Scene::from_data(&bad)
            .unwrap_err()
            .contains("does not exist")
    );
    let mut bad = data.clone();
    bad.couplings[0].ratio = 0.0;
    assert!(Scene::from_data(&bad).is_err());
    let mut bad = data;
    bad.couplings[0].id = jc.0;
    assert!(Scene::from_data(&bad).unwrap_err().contains("share an id"));
}

/// Places a cam (another copy of the crank's part) on a revolute joint to the ground about
/// `axis` through `at`, coupled to `crank_joint` by `ratio`. Returns the cam and its joint.
fn add_cam(
    s: &mut Scene,
    like: InstanceId,
    crank_joint: JointId,
    at: DVec3,
    axis: DVec3,
    ratio: f64,
) -> (InstanceId, JointId) {
    let d = s.root().instance(like).unwrap().definition;
    let cam = s
        .place_instance(MeshId::Root, d, DAffine3::IDENTITY)
        .unwrap()
        .0;
    let reference = if axis.dot(DVec3::Z).abs() > 0.9 {
        DVec3::X
    } else {
        DVec3::Z
    };
    let j = join(s, JointKind::Revolute, None, cam, at, axis, reference);
    couple(s, crank_joint, j, ratio, 0.0);
    (cam, j)
}

#[test]
fn a_slider_crank_with_a_coupled_cam_keeps_both_over_two_turns() {
    let (mut s, [crank, _, piston], joints) = slider_crank(DAffine3::IDENTITY);
    let (cam, jm) = add_cam(&mut s, crank, joints[0], p(0.0, 0.2, 0.0), DVec3::Z, 0.5);
    let pivot = p(0.0, 0.2, 0.0);
    for k in 1..=8 {
        let t = deg(90.0 * k as f64);
        s.drive_joint(joints[0], t, None).unwrap();
        let wrist = transform(&s, piston).transform_point3(p(R + L, 0.0, 0.0));
        assert!((wrist - p(piston_x(t), 0.0, 0.0)).length() < 1e-9, "{k}");
        assert_near(value(&s, jm), t / 2.0, "cam");
        assert!(transform(&s, cam).abs_diff_eq(turned_about(pivot, t / 2.0), 1e-9));
    }
}

/// A V6 with a crank, 6 rods, 6 pistons and 2 cams coupled 1:2, turned two whole turns.
#[test]
#[ignore = "timing; run in release"]
fn perf_a_v6_with_two_coupled_cams_turns_720_degrees() {
    let mut e = engine(6, false);
    let w = wrists(&e);
    let mut cams = Vec::new();
    for y in [0.1, -0.1] {
        let at = p(0.0, y, 0.2);
        cams.push(add_cam(
            &mut e.scene,
            e.crank,
            e.crank_joint,
            at,
            DVec3::X,
            0.5,
        ));
    }
    let t = Instant::now();
    e.scene
        .drive_joint(e.crank_joint, deg(720.0), None)
        .unwrap();
    let took = t.elapsed().as_secs_f64();
    check_engine(&e, &w);
    for (cam, j) in &cams {
        assert_near(value(&e.scene, *j), deg(360.0), "cam");
        assert!(transform(&e.scene, *cam).abs_diff_eq(DAffine3::IDENTITY, 1e-9));
    }
    eprintln!("720 degree drive with two cams: {:.1} ms", took * 1e3);
    assert!(took < 0.25, "{took}");
}
