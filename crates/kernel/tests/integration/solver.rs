//! Driving joints: the solver moves every part connected to a driven joint so that all the
//! joints of the assembly hold, checked against linkages whose motion is known exactly.

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::time::Instant;

use glam::{DAffine3, DQuat, DVec3};
use kernel::{
    DefinitionKind, EntityId, InstanceId, Joint, JointFrame, JointId, JointKind, MeshId, Model,
    Scene,
};

pub(crate) fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A scene with `n` instances of a small box component, each placed by `place(k)`.
pub(crate) fn parts_placed(
    n: usize,
    place: impl Fn(usize) -> DAffine3,
) -> (Scene, Vec<InstanceId>) {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            p(0.0, 0.0, 0.0),
            p(0.01, 0.0, 0.0),
            p(0.01, 0.01, 0.0),
            p(0.0, 0.01, 0.0),
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
    m.push_pull(face, 0.01).unwrap();
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
            "Part",
        )
        .unwrap();
    let d = s.root().instance(first).unwrap().definition;
    s.set_instance_transform(MeshId::Root, first, place(0))
        .unwrap();
    let mut ids = vec![first];
    for k in 1..n {
        ids.push(s.place_instance(MeshId::Root, d, place(k)).unwrap().0);
    }
    (s, ids)
}

pub(crate) fn parts(n: usize) -> (Scene, Vec<InstanceId>) {
    parts_placed(n, |_| DAffine3::IDENTITY)
}

pub(crate) fn transform(s: &Scene, i: InstanceId) -> DAffine3 {
    s.root().instance(i).unwrap().transform
}

/// Adds a joint whose world frame is (`point`, `axis`, `reference`) as the parts stand now.
pub(crate) fn join(
    s: &mut Scene,
    kind: JointKind,
    a: Option<InstanceId>,
    b: InstanceId,
    point: DVec3,
    axis: DVec3,
    reference: DVec3,
) -> JointId {
    let world = JointFrame {
        origin: point,
        axis: axis.normalize(),
        reference: reference.normalize(),
    };
    let local = |i: Option<InstanceId>| {
        let t = i.map_or(DAffine3::IDENTITY, |i| transform(s, i));
        JointFrame::local(&world, &t).unwrap()
    };
    let name = s.free_joint_name();
    let joint = Joint {
        name,
        kind,
        a,
        b,
        frame_a: local(a),
        frame_b: local(Some(b)),
        value: 0.0,
        slide: 0.0,
        limits: None,
    };
    s.add_joint(joint).unwrap().0
}

pub(crate) fn value(s: &Scene, j: JointId) -> f64 {
    s.joint(j).unwrap().value
}

pub(crate) const R: f64 = 0.05;
pub(crate) const L: f64 = 0.15;

/// A slider-crank in the XY plane: a crank turning about Z at the origin (pin at (R, 0)), a rod
/// of length L and a piston sliding along X. Returns the scene, [crank, rod, piston] and the
/// joints [crank, crank pin, wrist pin, bore].
pub(crate) fn slider_crank(rod_place: DAffine3) -> (Scene, [InstanceId; 3], [JointId; 4]) {
    let (mut s, ids) = parts_placed(3, |k| {
        if k == 1 {
            rod_place
        } else {
            DAffine3::IDENTITY
        }
    });
    let [crank, rod, piston] = [ids[0], ids[1], ids[2]];
    let z = DVec3::Z;
    let x = DVec3::X;
    let j0 = join(&mut s, JointKind::Revolute, None, crank, DVec3::ZERO, z, x);
    let j1 = join(
        &mut s,
        JointKind::Revolute,
        Some(crank),
        rod,
        p(R, 0.0, 0.0),
        z,
        x,
    );
    let j2 = join(
        &mut s,
        JointKind::Revolute,
        Some(rod),
        piston,
        p(R + L, 0.0, 0.0),
        z,
        x,
    );
    let j3 = join(
        &mut s,
        JointKind::Prismatic,
        None,
        piston,
        p(R + L, 0.0, 0.0),
        x,
        DVec3::Y,
    );
    (s, [crank, rod, piston], [j0, j1, j2, j3])
}

pub(crate) fn piston_x(t: f64) -> f64 {
    R * t.cos() + (L * L - R * R * t.sin().powi(2)).sqrt()
}

#[test]
fn a_slider_crank_follows_the_exact_piston_position_over_a_turn() {
    let (mut s, [crank, rod, piston], [j0, j1, j2, j3]) = slider_crank(DAffine3::IDENTITY);
    for k in 1..=24 {
        let t = (15.0 * k as f64).to_radians();
        let change = s.drive_joint(j0, t, None).unwrap();
        assert!(!change.is_empty());
        let wrist = transform(&s, piston).transform_point3(p(R + L, 0.0, 0.0));
        assert!((wrist.x - piston_x(t)).abs() < 1e-9, "{k}: {wrist}");
        assert!(wrist.y.abs() < 1e-9 && wrist.z.abs() < 1e-9);
        let pin = transform(&s, crank).transform_point3(p(R, 0.0, 0.0));
        assert!((pin - p(R * t.cos(), R * t.sin(), 0.0)).length() < 1e-9);
        // The rod joins the pin and the wrist.
        let rod_t = transform(&s, rod);
        assert!((rod_t.transform_point3(p(R, 0.0, 0.0)) - pin).length() < 1e-9);
        assert!((rod_t.transform_point3(p(R + L, 0.0, 0.0)) - wrist).length() < 1e-9);
        // Stored values follow the pose: the crank's is the target, the bore's the travel.
        assert_eq!(value(&s, j0), t);
        assert!((value(&s, j3) - (piston_x(t) - R - L)).abs() < 1e-9);
        // The rod leans at phi; the crank pin's value runs on continuously from 0.
        let phi = -(R * t.sin() / L).asin();
        assert!(
            (value(&s, j1) - (phi - t)).abs() < 1e-9,
            "{k}: {}",
            value(&s, j1)
        );
        assert!((value(&s, j2) + phi).abs() < 1e-9);
    }
    // A full turn is 360 degrees, not 0: the crank's value keeps counting.
    assert!((value(&s, j0) - TAU).abs() < 1e-12);
}

#[test]
fn a_four_bar_rocker_follows_its_exact_angle() {
    // Ground pivots O2 = (0, 0), O4 = (D, 0); crank A, coupler B, rocker C (crank-rocker).
    let (a, b, c, d) = (0.1, 0.3, 0.25, 0.3);
    let rocker_end = |t: f64| {
        let pa = p(a * t.cos(), a * t.sin(), 0.0);
        let o4 = p(d, 0.0, 0.0);
        let dist = (o4 - pa).length();
        let along = (dist * dist + b * b - c * c) / (2.0 * dist);
        let h = (b * b - along * along).sqrt();
        let u = (o4 - pa) / dist;
        let n = p(-u.y, u.x, 0.0);
        pa + u * along + n * h
    };
    let t0 = FRAC_PI_2;
    let pa = p(a * t0.cos(), a * t0.sin(), 0.0);
    let pb = rocker_end(t0);
    let o4 = p(d, 0.0, 0.0);
    let (mut s, ids) = parts(3);
    let [crank, coupler, rocker] = [ids[0], ids[1], ids[2]];
    let z = DVec3::Z;
    let j0 = join(&mut s, JointKind::Revolute, None, crank, DVec3::ZERO, z, pa);
    join(
        &mut s,
        JointKind::Revolute,
        Some(crank),
        coupler,
        pa,
        z,
        DVec3::X,
    );
    join(
        &mut s,
        JointKind::Revolute,
        Some(coupler),
        rocker,
        pb,
        z,
        DVec3::X,
    );
    let j3 = join(&mut s, JointKind::Revolute, None, rocker, o4, z, pb - o4);
    let start = (pb - o4).y.atan2((pb - o4).x);
    for k in 1..=24 {
        let drive = (15.0 * k as f64).to_radians();
        s.drive_joint(j0, drive, None).unwrap();
        let end = rocker_end(t0 + drive);
        let expected = (end - o4).y.atan2((end - o4).x) - start;
        assert!((value(&s, j3) - expected).abs() < 1e-9, "{k}");
        let rocker_tip = transform(&s, rocker).transform_point3(pb);
        assert!((rocker_tip - end).length() < 1e-9);
        // The coupler's far end is where the rocker's is.
        let coupler_end = transform(&s, coupler).transform_point3(pb);
        assert!(
            (coupler_end - end).length() < 1e-9,
            "{k}: {coupler_end} vs {end}"
        );
    }
}

/// A V engine along X: a crank turning about X, and per cylinder a rod on a crank pin and a
/// piston sliding along its bank's bore (banks at +/-30 degrees from Z). Cylinders share pins
/// in pairs, the pins 120 degrees apart. Returns the scene, the crank, the pistons with their
/// bore (start, direction) and wrist pin, and the crank's joint.
pub(crate) struct Engine {
    pub(crate) scene: Scene,
    pub(crate) crank: InstanceId,
    pub(crate) crank_joint: JointId,
    pub(crate) pistons: Vec<(InstanceId, DVec3, DVec3)>,
    pub(crate) rods: Vec<(InstanceId, DVec3)>,
    pub(crate) joints: Vec<JointId>,
}

pub(crate) const ER: f64 = 0.04;
pub(crate) const EL: f64 = 0.14;

pub(crate) fn engine(cylinders: usize, flywheel: bool) -> Engine {
    let n = 1 + 2 * cylinders + usize::from(flywheel);
    let (mut s, ids) = parts(n);
    let crank = ids[0];
    let x = DVec3::X;
    let crank_joint = join(
        &mut s,
        JointKind::Revolute,
        None,
        crank,
        DVec3::ZERO,
        x,
        DVec3::Z,
    );
    let mut joints = vec![crank_joint];
    let mut pistons = Vec::new();
    let mut rods = Vec::new();
    for c in 0..cylinders {
        let (rod, piston) = (ids[1 + 2 * c], ids[2 + 2 * c]);
        let throw = (c / 2) as f64 * TAU / 3.0;
        let bank = if c % 2 == 0 { PI / 6.0 } else { -PI / 6.0 };
        let px = 0.1 * c as f64;
        let pin = p(px, ER * throw.sin(), ER * throw.cos());
        let dir = p(0.0, bank.sin(), bank.cos());
        let base = p(px, 0.0, 0.0);
        let rel = pin - base;
        let along = rel.dot(dir) + (EL * EL - rel.length_squared() + rel.dot(dir).powi(2)).sqrt();
        let wrist = base + dir * along;
        joints.push(join(
            &mut s,
            JointKind::Revolute,
            Some(crank),
            rod,
            pin,
            x,
            DVec3::Z,
        ));
        joints.push(join(
            &mut s,
            JointKind::Revolute,
            Some(rod),
            piston,
            wrist,
            x,
            DVec3::Z,
        ));
        joints.push(join(
            &mut s,
            JointKind::Prismatic,
            None,
            piston,
            wrist,
            dir,
            x,
        ));
        pistons.push((piston, base, dir));
        rods.push((rod, pin));
    }
    if flywheel {
        let f = *ids.last().unwrap();
        joints.push(join(
            &mut s,
            JointKind::Fixed,
            Some(crank),
            f,
            p(-0.1, 0.0, 0.0),
            x,
            DVec3::Z,
        ));
    }
    Engine {
        scene: s,
        crank,
        crank_joint,
        pistons,
        rods,
        joints,
    }
}

/// Every piston's wrist pin lies on its bore line at the exact distance for the crank angle.
pub(crate) fn check_engine(e: &Engine, wrists: &[DVec3]) {
    let s = &e.scene;
    for (k, ((piston, base, dir), wrist0)) in e.pistons.iter().zip(wrists).enumerate() {
        let wrist = transform(s, *piston).transform_point3(*wrist0);
        let off = wrist - *base;
        let across = off - *dir * off.dot(*dir);
        assert!(
            across.length() < 1e-9,
            "piston {k} off its bore by {across}"
        );
        let (rod, pin0) = e.rods[k];
        let pin = transform(s, e.crank).transform_point3(pin0);
        assert!((transform(s, rod).transform_point3(pin0) - pin).length() < 1e-9);
        let rel = pin - *base;
        let along = rel.dot(*dir) + (EL * EL - rel.length_squared() + rel.dot(*dir).powi(2)).sqrt();
        assert!((off.dot(*dir) - along).abs() < 1e-9, "piston {k}");
    }
}

pub(crate) fn wrists(e: &Engine) -> Vec<DVec3> {
    e.pistons
        .iter()
        .map(|(piston, _, _)| {
            let j = e
                .scene
                .joints()
                .find(|(_, j)| j.b == *piston && j.kind == JointKind::Prismatic)
                .unwrap()
                .1;
            j.frame_b.origin
        })
        .collect()
}

#[test]
fn a_v6_turns_two_full_turns_and_back_with_every_piston_on_its_bore() {
    let mut e = engine(6, false);
    let w = wrists(&e);
    let start: Vec<DAffine3> = e
        .scene
        .root()
        .instances()
        .map(|(_, i)| i.transform)
        .collect();
    for k in 1..=4 {
        let t = (180.0 * k as f64).to_radians();
        e.scene.drive_joint(e.crank_joint, t, None).unwrap();
        assert_eq!(value(&e.scene, e.crank_joint), t);
        check_engine(&e, &w);
    }
    e.scene.drive_joint(e.crank_joint, 0.0, None).unwrap();
    check_engine(&e, &w);
    for &j in &e.joints {
        assert!(
            value(&e.scene, j).abs() < 1e-9,
            "{j} at {}",
            value(&e.scene, j)
        );
    }
    let end: Vec<DAffine3> = e
        .scene
        .root()
        .instances()
        .map(|(_, i)| i.transform)
        .collect();
    for (a, b) in start.iter().zip(&end) {
        assert!(a.abs_diff_eq(*b, 1e-9), "{a:?} vs {b:?}");
    }
}

#[test]
fn driving_outside_the_limits_is_refused_and_changes_nothing() {
    let (mut s, [crank, ..], [j0, ..]) = slider_crank(DAffine3::IDENTITY);
    let mut limited = s.joint(j0).unwrap().clone();
    limited.limits = Some((-0.5, 0.5));
    s.change_joint(j0, limited).unwrap();
    let before = transform(&s, crank);
    let e = s.drive_joint(j0, 0.6, None).unwrap_err();
    assert!(e.to_string().contains("limits"), "{e}");
    assert_eq!(transform(&s, crank), before);
    assert_eq!(value(&s, j0), 0.0);
    s.drive_joint(j0, 0.5, None).unwrap();
    assert_eq!(value(&s, j0), 0.5);
}

#[test]
fn an_unreachable_drive_is_refused_and_changes_nothing() {
    // A link pinned to the crank and held by a ball at a fixed point locks the crank.
    let (mut s, ids) = parts(2);
    let [crank, link] = [ids[0], ids[1]];
    let j0 = join(
        &mut s,
        JointKind::Revolute,
        None,
        crank,
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    join(
        &mut s,
        JointKind::Revolute,
        Some(crank),
        link,
        p(0.05, 0.0, 0.0),
        DVec3::Z,
        DVec3::X,
    );
    join(
        &mut s,
        JointKind::Ball,
        None,
        link,
        p(0.2, 0.05, 0.0),
        DVec3::Z,
        DVec3::X,
    );
    let joints: Vec<Joint> = s.joints().map(|(_, j)| j.clone()).collect();
    let before = [transform(&s, crank), transform(&s, link)];
    let e = s.drive_joint(j0, 90f64.to_radians(), None).unwrap_err();
    assert!(
        e.to_string().contains("cannot all be met at 90 degrees"),
        "{e}"
    );
    assert_eq!([transform(&s, crank), transform(&s, link)], before);
    let after: Vec<Joint> = s.joints().map(|(_, j)| j.clone()).collect();
    assert_eq!(after, joints);
}

#[test]
fn a_drive_to_the_current_value_changes_nothing() {
    let (mut s, _, [j0, ..]) = slider_crank(DAffine3::IDENTITY);
    assert!(s.drive_joint(j0, 0.0, None).unwrap().is_empty());
    s.drive_joint(j0, 0.3, None).unwrap();
    assert!(s.drive_joint(j0, 0.3, None).unwrap().is_empty());
    // Within a hair of it (a value read back in degrees) is the same value.
    assert!(s.drive_joint(j0, 0.3 + 1e-13, None).unwrap().is_empty());
    assert_eq!(value(&s, j0), 0.3);
}

#[test]
fn another_joint_passing_its_limits_midway_is_refused() {
    // The bore runs from 0 down to -2R and back over a turn: limits of [-R, 0] hold at both
    // ends of the turn but not halfway round.
    let (mut s, [crank, ..], [j0, _, _, j3]) = slider_crank(DAffine3::IDENTITY);
    let mut limited = s.joint(j3).unwrap().clone();
    limited.limits = Some((-R, 0.0));
    s.change_joint(j3, limited).unwrap();
    let before = transform(&s, crank);
    let e = s.drive_joint(j0, TAU, None).unwrap_err();
    assert!(e.to_string().contains("would pass its limits"), "{e}");
    assert_eq!(transform(&s, crank), before);
    // A small turn that stays inside is fine, and the bore at its limit 0 counts as within.
    s.drive_joint(j0, 0.2, None).unwrap();
    s.drive_joint(j0, 0.0, None).unwrap();
    assert!(value(&s, j3) <= 0.0 && value(&s, j3) > -1e-9);
}

/// The world axis and reference of joint `j`'s frame on its part `b`.
fn frame_b(s: &Scene, j: JointId) -> JointFrame {
    let joint = s.joint(j).unwrap();
    joint.frame_b.world(&transform(s, joint.b))
}

/// A part turned about `about` (through the origin) by `angle` after its joint was made: a
/// drive either turns it back so the joint holds facing the right way, or is refused.
fn drive_turned(kind: JointKind, axis: DVec3, reference: DVec3, about: DVec3, angle: f64) {
    let (mut s, ids) = parts(1);
    let j = join(&mut s, kind, None, ids[0], DVec3::ZERO, axis, reference);
    let turn = DAffine3::from_axis_angle(about, angle);
    s.set_instance_transform(MeshId::Root, ids[0], turn * transform(&s, ids[0]))
        .unwrap();
    let target = if kind.is_angular() { 0.5 } else { 0.05 };
    match s.drive_joint(j, target, None) {
        Ok(_) => {
            let b = frame_b(&s, j);
            assert!((b.axis - axis).length() < 1e-9, "{kind:?}: axis {}", b.axis);
            if kind == JointKind::Prismatic {
                assert!((b.reference - reference).length() < 1e-9, "{}", b.reference);
            }
        }
        Err(e) => assert!(e.to_string().contains("cannot all be met"), "{e}"),
    }
}

#[test]
fn a_part_turned_half_a_turn_does_not_meet_its_joint() {
    drive_turned(JointKind::Revolute, DVec3::Z, DVec3::X, DVec3::X, PI);
    drive_turned(JointKind::Prismatic, DVec3::X, DVec3::Y, DVec3::X, PI);
    drive_turned(JointKind::Cylindrical, DVec3::Z, DVec3::X, DVec3::Y, PI);
}

#[test]
fn a_part_turned_most_of_the_way_over_is_turned_back() {
    // With residuals blind to direction the nearest pose would be the flipped one.
    let (mut s, ids) = parts(1);
    let j = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    s.set_instance_transform(MeshId::Root, ids[0], DAffine3::from_rotation_x(2.6))
        .unwrap();
    s.drive_joint(j, 0.5, None).unwrap();
    let b = frame_b(&s, j);
    assert!((b.axis - DVec3::Z).length() < 1e-9, "{}", b.axis);
    assert!((value(&s, j) - 0.5).abs() < 1e-12);
    let (mut s, ids) = parts(1);
    let j = join(
        &mut s,
        JointKind::Prismatic,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::X,
        DVec3::Y,
    );
    s.set_instance_transform(MeshId::Root, ids[0], DAffine3::from_rotation_x(2.6))
        .unwrap();
    s.drive_joint(j, 0.05, None).unwrap();
    let b = frame_b(&s, j);
    assert!((b.reference - DVec3::Y).length() < 1e-9, "{}", b.reference);
}

#[test]
fn a_parallelogram_four_bar_stays_a_parallelogram_through_its_flat_point() {
    // Crank and rocker 0.1, coupler and ground 0.3: at 180 degrees the four bars lie on one
    // line, where the linkage could fold into a crossed (antiparallelogram) shape.
    let (a, d) = (0.1, 0.3);
    let pa = p(0.0, a, 0.0);
    let pb = p(d, a, 0.0);
    let o4 = p(d, 0.0, 0.0);
    for drive in [120.0f64, 125.0, 200.0] {
        let (mut s, ids) = parts(3);
        let [crank, coupler, rocker] = [ids[0], ids[1], ids[2]];
        let z = DVec3::Z;
        let j0 = join(&mut s, JointKind::Revolute, None, crank, DVec3::ZERO, z, pa);
        join(
            &mut s,
            JointKind::Revolute,
            Some(crank),
            coupler,
            pa,
            z,
            DVec3::X,
        );
        join(
            &mut s,
            JointKind::Revolute,
            Some(coupler),
            rocker,
            pb,
            z,
            DVec3::X,
        );
        let j3 = join(&mut s, JointKind::Revolute, None, rocker, o4, z, pa);
        let t = drive.to_radians();
        s.drive_joint(j0, t, None).unwrap();
        assert!(
            (value(&s, j3) - t).abs() < 1e-9,
            "{drive}: the rocker turned {} degrees",
            value(&s, j3).to_degrees()
        );
        let tip = transform(&s, crank).transform_point3(pa);
        let rocker_tip = transform(&s, rocker).transform_point3(pb);
        assert!(
            (rocker_tip - tip - p(d, 0.0, 0.0)).length() < 1e-9,
            "{drive}"
        );
        // The coupler only moved, never turned.
        let m = transform(&s, coupler).matrix3;
        assert!(m.abs_diff_eq(glam::DMat3::IDENTITY, 1e-9), "{drive}");
    }
}

#[test]
fn undoing_a_drive_restores_every_transform_and_value_exactly() {
    let (mut s, ids, joints) = slider_crank(DAffine3::IDENTITY);
    s.drive_joint(joints[0], 0.4, None).unwrap();
    let transforms: Vec<DAffine3> = ids.iter().map(|i| transform(&s, *i)).collect();
    let values: Vec<Joint> = s.joints().map(|(_, j)| j.clone()).collect();
    let change = s.drive_joint(joints[0], 2.0, None).unwrap();
    s.apply(&change.inverse());
    let after: Vec<DAffine3> = ids.iter().map(|i| transform(&s, *i)).collect();
    assert_eq!(after, transforms);
    assert_eq!(
        s.joints().map(|(_, j)| j.clone()).collect::<Vec<_>>(),
        values
    );
}

#[test]
fn a_floating_assembly_holds_the_driven_joints_first_part() {
    let (mut s, ids) = parts_placed(2, |k| DAffine3::from_translation(p(k as f64, 0.0, 0.0)));
    let [a, b] = [ids[0], ids[1]];
    let hinge = join(
        &mut s,
        JointKind::Revolute,
        Some(a),
        b,
        p(0.5, 0.0, 0.0),
        DVec3::Z,
        DVec3::X,
    );
    let before_a = transform(&s, a);
    let before_b = transform(&s, b);
    s.drive_joint(hinge, FRAC_PI_2, None).unwrap();
    assert_eq!(transform(&s, a), before_a);
    let turn = DAffine3::from_translation(p(0.5, 0.0, 0.0))
        * DAffine3::from_quat(DQuat::from_rotation_z(FRAC_PI_2))
        * DAffine3::from_translation(p(-0.5, 0.0, 0.0));
    assert!(transform(&s, b).abs_diff_eq(turn * before_b, 1e-9));
    // Driving the other way round the same hinge, from b's side, holds b.
    let back = join(
        &mut s,
        JointKind::Revolute,
        Some(b),
        a,
        p(0.5, 0.0, 0.0),
        DVec3::Z,
        DVec3::X,
    );
    let held = transform(&s, b);
    s.drive_joint(back, 0.1, None).unwrap();
    assert_eq!(transform(&s, b), held);
}

#[test]
fn a_prismatic_drive_slides_and_a_cylindrical_drive_turns_and_slides() {
    let (mut s, ids) = parts(2);
    let slider = join(
        &mut s,
        JointKind::Prismatic,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::X,
        DVec3::Y,
    );
    s.drive_joint(slider, 0.25, None).unwrap();
    assert!(
        transform(&s, ids[0])
            .translation
            .abs_diff_eq(p(0.25, 0.0, 0.0), 1e-9)
    );
    let screw = join(
        &mut s,
        JointKind::Cylindrical,
        None,
        ids[1],
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    s.drive_joint(screw, FRAC_PI_2, Some(0.1)).unwrap();
    let t = transform(&s, ids[1]);
    assert!(
        t.transform_point3(p(1.0, 0.0, 0.0))
            .abs_diff_eq(p(0.0, 1.0, 0.1), 1e-9)
    );
    assert_eq!(s.joint(screw).unwrap().slide, 0.1);
    // Without a slide the slide stays.
    s.drive_joint(screw, 0.0, None).unwrap();
    assert!(
        transform(&s, ids[1])
            .transform_point3(p(1.0, 0.0, 0.0))
            .abs_diff_eq(p(1.0, 0.0, 0.1), 1e-9)
    );
}

#[test]
fn a_mirrored_rod_drives_as_its_unmirrored_twin() {
    let mirror =
        DAffine3::from_translation(p(0.3, 0.2, 0.0)) * DAffine3::from_scale(p(-1.0, 2.0, 1.0));
    let (mut s, [_, rod, piston], [j0, ..]) = slider_crank(mirror);
    for k in 1..=8 {
        let t = (45.0 * k as f64).to_radians();
        s.drive_joint(j0, t, None).unwrap();
        let wrist = transform(&s, piston).transform_point3(p(R + L, 0.0, 0.0));
        assert!((wrist.x - piston_x(t)).abs() < 1e-9, "{k}");
        // The rod keeps its mirror and scale: only rotated and moved.
        let m = transform(&s, rod).matrix3;
        assert!((m.determinant() - mirror.matrix3.determinant()).abs() < 1e-9);
    }
}

#[test]
fn measured_values_follow_a_moved_pose() {
    let (mut s, [crank, rod, piston], [j0, _, _, j3]) = slider_crank(DAffine3::IDENTITY);
    let mut moved = s.clone();
    moved.drive_joint(j0, 1.0, None).unwrap();
    let mut limited = s.joint(j0).unwrap().clone();
    limited.limits = Some((-0.5, 0.5));
    s.change_joint(j0, limited).unwrap();
    for i in [crank, rod, piston] {
        s.set_instance_transform(MeshId::Root, i, transform(&moved, i))
            .unwrap();
    }
    let change = s.update_joint_values();
    assert!(!change.is_empty());
    assert!((value(&s, j0) - 1.0).abs() < 1e-9);
    // The pose passed the crank's limits: they go, so the value can stand.
    assert_eq!(s.joint(j0).unwrap().limits, None);
    assert!((value(&s, j3) - value(&moved, j3)).abs() < 1e-9);
    assert!(s.update_joint_values().is_empty());
}

/// 30 parts and 43 joints: a 14-cylinder engine with a flywheel.
#[test]
#[ignore = "timing; run in release"]
fn perf_a_thirty_part_engine_drives_in_under_20_ms() {
    let mut e = engine(14, true);
    assert_eq!(e.scene.root().instances().count(), 30);
    assert!(e.joints.len() >= 40);
    let w = wrists(&e);
    let mut worst = 0.0f64;
    for k in 1..=36 {
        let t = Instant::now();
        e.scene
            .drive_joint(e.crank_joint, (10.0 * k as f64).to_radians(), None)
            .unwrap();
        worst = worst.max(t.elapsed().as_secs_f64());
    }
    check_engine(&e, &w);
    eprintln!("worst 10 degree drive: {:.2} ms", worst * 1e3);
    assert!(worst < 0.020, "{worst}");
}

/// A six-axis arm on the ground: base on a vertical axis, shoulder and elbow on horizontal
/// ones, a forearm roll, a wrist pitch and a tool roll, each joint 0.3 m further out along X.
/// Returns the scene, the parts and the joints J1 to J6.
fn six_axis_arm() -> (Scene, Vec<InstanceId>, Vec<JointId>) {
    let (mut s, ids) = parts(6);
    let axes = [DVec3::Z, DVec3::Y, DVec3::Y, DVec3::X, DVec3::Y, DVec3::X];
    let mut joints = Vec::new();
    for (k, axis) in axes.iter().enumerate() {
        let a = (k > 0).then(|| ids[k - 1]);
        let reference = if *axis == DVec3::Z {
            DVec3::X
        } else {
            DVec3::Z
        };
        let at = p(0.3 * k as f64, 0.0, 0.5);
        joints.push(join(
            &mut s,
            JointKind::Revolute,
            a,
            ids[k],
            at,
            *axis,
            reference,
        ));
    }
    (s, ids, joints)
}

#[test]
fn driving_one_joint_of_a_serial_arm_holds_the_others() {
    let (mut s, ids, joints) = six_axis_arm();
    for (k, j) in joints.iter().enumerate() {
        s.drive_joint(*j, 0.2 * (k as f64 + 1.0), None).unwrap();
    }
    let before: Vec<DAffine3> = ids.iter().map(|i| transform(&s, *i)).collect();
    let values: Vec<f64> = joints.iter().map(|j| value(&s, *j)).collect();
    // The wrist pitch, J5: the parts before it stay put, the two after it turn about it.
    s.drive_joint(joints[4], 50f64.to_radians(), None).unwrap();
    for (k, j) in joints.iter().enumerate() {
        let want = if k == 4 {
            50f64.to_radians()
        } else {
            values[k]
        };
        assert!(
            (value(&s, *j) - want).abs() < 1e-9,
            "J{}: {}",
            k + 1,
            value(&s, *j)
        );
    }
    for k in 0..4 {
        assert!(
            transform(&s, ids[k]).abs_diff_eq(before[k], 1e-9),
            "part {k} moved"
        );
    }
    let turn = transform(&s, ids[4]) * before[4].inverse();
    assert!(transform(&s, ids[5]).abs_diff_eq(turn * before[5], 1e-9));
}

#[test]
fn a_whole_arm_pose_is_reached_in_one_call() {
    let (mut s, _, joints) = six_axis_arm();
    let pose = [40.0, 33.0, -47.0, 120.0, 104.0, -200.0];
    let wanted: Vec<(JointId, f64, Option<f64>)> = joints
        .iter()
        .zip(pose)
        .map(|(j, d)| (*j, f64::to_radians(d), None))
        .collect();
    s.drive_joints(&wanted).unwrap();
    for (j, d) in joints.iter().zip(pose) {
        assert!((value(&s, *j) - d.to_radians()).abs() < 1e-9);
    }
}

#[test]
fn coupled_fingers_on_an_arm_hold_while_the_arm_turns() {
    let (mut s, ids, joints) = six_axis_arm();
    let d = s.root().instance(ids[0]).unwrap().definition;
    let mut finger = || {
        s.place_instance(MeshId::Root, d, DAffine3::IDENTITY)
            .unwrap()
            .0
    };
    let (f1, f2) = (finger(), finger());
    let (tool, at) = (ids[5], p(1.6, 0.0, 0.5));
    let g1 = join(
        &mut s,
        JointKind::Prismatic,
        Some(tool),
        f1,
        at,
        DVec3::Y,
        DVec3::Z,
    );
    let g2 = join(
        &mut s,
        JointKind::Prismatic,
        Some(tool),
        f2,
        at,
        -DVec3::Y,
        DVec3::Z,
    );
    s.add_coupling(kernel::Coupling {
        driver: g1,
        driven: g2,
        ratio: 1.0,
        offset: 0.0,
    })
    .unwrap();
    s.drive_joint(g1, 0.03, None).unwrap();
    assert!((value(&s, g2) - 0.03).abs() < 1e-9);
    s.drive_joint(joints[0], 1.0, None).unwrap();
    s.drive_joint(joints[2], -0.8, None).unwrap();
    for (j, want) in [(g1, 0.03), (g2, 0.03), (joints[1], 0.0), (joints[0], 1.0)] {
        assert!((value(&s, j) - want).abs() < 1e-9, "{}", value(&s, j));
    }
}
