//! Sweep checks: a joint driven through a range on a copy of the scene, every pair of parts
//! checked at each step.

use std::f64::consts::PI;
use std::time::Instant;

use glam::{DAffine3, DQuat, DVec3};
use kernel::interfere::{
    SWEEP_REPORT_WITHIN, SWEEP_TOO_COSTLY, SweepPair, SweepReport, sweep_check, sweep_check_budget,
};
use kernel::{
    DefinitionKind, InstanceId, Joint, JointFrame, JointId, JointKind, MeshId, Scene, Solid,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A box from `lo` to `hi` in its own coordinates, placed by `place`.
fn block(s: &mut Scene, lo: DVec3, hi: DVec3, place: DAffine3) -> InstanceId {
    let solid = Solid::cuboid(hi - lo).unwrap();
    s.add_solid(
        MeshId::Root,
        &solid,
        DefinitionKind::Group,
        "P",
        place * DAffine3::from_translation(lo),
        None,
    )
    .unwrap()
    .0
}

fn transform(s: &Scene, i: InstanceId) -> DAffine3 {
    s.root().instance(i).unwrap().transform
}

/// Adds a joint whose world frame is (`point`, `axis`, `reference`) as the parts stand now.
fn join(
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

const R: f64 = 0.05;
const L: f64 = 0.15;
/// The piston's half length along its stroke.
const H: f64 = 0.02;

fn piston_x(t: f64) -> f64 {
    R * t.cos() + (L * L - R * R * t.sin().powi(2)).sqrt()
}

/// Where the valve's face stands: the piston's front reaches it at 40 degrees either side of
/// top dead centre.
fn valve_x() -> f64 {
    piston_x(40f64.to_radians()) + H
}

/// The stop behind the piston clears it by 2 mm at bottom dead centre.
const STOP_X: f64 = L - R - H - 0.002;

struct Crank {
    scene: Scene,
    crank: InstanceId,
    rod: InstanceId,
    piston: InstanceId,
    valve: InstanceId,
    stop: InstanceId,
    joints: [JointId; 4],
}

/// A slider-crank in the XY plane at crank angle 0, its layers stacked in Z: the crank
/// (z 0 to 0.01) turning about Z at the origin, the rod (0.01 to 0.02) from the crank pin at
/// (R, 0) to the wrist pin at (R + L, 0), the piston (0.02 to 0.05) sliding along X; a
/// valve in front of the piston and a stop behind it, both fixed.
fn slider_crank() -> Crank {
    let mut s = Scene::new();
    let crank = block(
        &mut s,
        p(-0.01, -0.01, 0.0),
        p(R + 0.01, 0.01, 0.01),
        DAffine3::IDENTITY,
    );
    let rod = block(
        &mut s,
        p(R - 0.005, -0.005, 0.01),
        p(R + L + 0.005, 0.005, 0.02),
        DAffine3::IDENTITY,
    );
    let piston = block(
        &mut s,
        p(R + L - H, -0.02, 0.02),
        p(R + L + H, 0.02, 0.05),
        DAffine3::IDENTITY,
    );
    let vx = valve_x();
    let valve = block(
        &mut s,
        p(vx, -0.01, 0.02),
        p(vx + 0.03, 0.01, 0.05),
        DAffine3::IDENTITY,
    );
    let stop = block(
        &mut s,
        p(STOP_X - 0.02, -0.01, 0.035),
        p(STOP_X, 0.01, 0.05),
        DAffine3::IDENTITY,
    );
    let (z, x) = (DVec3::Z, DVec3::X);
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
    Crank {
        scene: s,
        crank,
        rod,
        piston,
        valve,
        stop,
        joints: [j0, j1, j2, j3],
    }
}

fn pair(r: &SweepReport, a: InstanceId, b: InstanceId) -> Option<&SweepPair> {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    r.pairs.iter().find(|q| q.a == a && q.b == b)
}

#[test]
fn a_piston_hits_the_valve_between_the_known_crank_angles() {
    let c = slider_crank();
    // One degree a step, from 90 to 450 degrees: past top dead centre at 360.
    let r = sweep_check(
        &c.scene,
        c.joints[0],
        90f64.to_radians(),
        450f64.to_radians(),
        361,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    assert_eq!(r.stopped, None);
    assert_eq!(r.steps, 361);
    assert!(r.skipped.is_empty());
    let hit = pair(&r, c.piston, c.valve).expect("the piston and valve meet");
    let (first, last) = hit.interferes.expect("the piston hits the valve");
    let step = 1f64.to_radians() + 1e-9;
    let (start, end) = (320f64.to_radians(), 400f64.to_radians());
    assert!(
        first >= start - 1e-9 && first - start <= step,
        "{}",
        first.to_degrees()
    );
    assert!(
        last <= end + 1e-9 && end - last <= step,
        "{}",
        last.to_degrees()
    );
    // Deepest at top dead centre: the piston's front 0.2 + H against the valve's face, over
    // the valve's width (0.02) and the shared height (0.03).
    let deepest = (R + L + H - valve_x()) * 0.02 * 0.03;
    assert!(
        (hit.volume - deepest).abs() < 1e-9,
        "{} vs {deepest}",
        hit.volume
    );
    assert!((hit.volume_at.unwrap() - 360f64.to_radians()).abs() < 1e-9);
    assert_eq!(hit.clearance, 0.0);
    assert!(hit.failed.is_none());
}

#[test]
fn the_smallest_clearance_and_its_angle_are_reported() {
    let c = slider_crank();
    let r = sweep_check(
        &c.scene,
        c.joints[0],
        90f64.to_radians(),
        450f64.to_radians(),
        361,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    let near = pair(&r, c.piston, c.stop).expect("the stop comes within 5 mm");
    assert_eq!(near.interferes, None);
    assert_eq!(near.volume, 0.0);
    assert!((near.clearance - 0.002).abs() < 1e-9, "{}", near.clearance);
    assert!((near.clearance_at - 180f64.to_radians()).abs() < 1e-9);
    // Its clearance at the start (90 degrees) is beyond 5 mm.
    assert!(piston_x(90f64.to_radians()) - H - STOP_X > SWEEP_REPORT_WITHIN);
    // Asked for pairs within 1 mm, the stop (2 mm at best) is left out.
    let close = sweep_check(
        &c.scene,
        c.joints[0],
        90f64.to_radians(),
        450f64.to_radians(),
        37,
        None,
        &[c.piston, c.stop],
        0.001,
    )
    .unwrap();
    assert!(close.pairs.is_empty());
}

#[test]
fn touching_layers_and_far_pairs_are_not_interference() {
    let c = slider_crank();
    let r = sweep_check(
        &c.scene,
        c.joints[0],
        0.0,
        360f64.to_radians(),
        37,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    // The stacked layers touch face to face at every step: clearance 0, no interference.
    for (a, b) in [(c.crank, c.rod), (c.rod, c.piston)] {
        let q = pair(&r, a, b).expect("stacked layers touch");
        assert_eq!(q.interferes, None, "{a:?} {b:?}");
        assert_eq!(q.clearance, 0.0);
    }
    // The crank never comes near the valve, the stop or the piston (a layer apart).
    for b in [c.valve, c.stop, c.piston] {
        assert!(pair(&r, c.crank, b).is_none(), "{b:?}");
    }
    assert!(pair(&r, c.valve, c.stop).is_none());
}

#[test]
fn a_step_the_solver_cannot_meet_stops_the_sweep() {
    let mut c = slider_crank();
    // The bore stops the piston 5 cm short of bottom dead centre.
    let mut bore = c.scene.joint(c.joints[3]).unwrap().clone();
    bore.limits = Some((-0.05, 0.01));
    c.scene.change_joint(c.joints[3], bore).unwrap();
    let r = sweep_check(
        &c.scene,
        c.joints[0],
        0.0,
        180f64.to_radians(),
        19,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    // The wrist reaches 0.15 between 80 and 90 degrees.
    assert!(piston_x(80f64.to_radians()) > 0.15 && piston_x(90f64.to_radians()) < 0.15);
    let (value, why) = r.stopped.clone().expect("the sweep stops");
    assert!(
        (value - 90f64.to_radians()).abs() < 1e-9,
        "{}",
        value.to_degrees()
    );
    assert!(why.contains("cannot all be met"), "{why}");
    assert_eq!(r.steps, 9);
    // The steps before still count: the layers touched.
    assert!(pair(&r, c.crank, c.rod).is_some());
}

#[test]
fn a_sweep_leaves_the_scene_as_it_was() {
    let c = slider_crank();
    let before: Vec<DAffine3> = c
        .scene
        .root()
        .instances()
        .map(|(_, i)| i.transform)
        .collect();
    let values: Vec<f64> = c.scene.joints().map(|(_, j)| j.value).collect();
    sweep_check(
        &c.scene,
        c.joints[0],
        0.0,
        360f64.to_radians(),
        13,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    let after: Vec<DAffine3> = c
        .scene
        .root()
        .instances()
        .map(|(_, i)| i.transform)
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        values,
        c.scene.joints().map(|(_, j)| j.value).collect::<Vec<_>>()
    );
}

#[test]
fn bad_sweeps_are_refused() {
    let c = slider_crank();
    let run = |steps: usize, within: f64, ids: &[InstanceId], joint: JointId| {
        sweep_check(&c.scene, joint, 0.0, 1.0, steps, None, ids, within)
    };
    assert!(run(1, 0.005, &[], c.joints[0]).is_err());
    assert!(run(10_001, 0.005, &[], c.joints[0]).is_err());
    assert!(run(10, -1.0, &[], c.joints[0]).is_err());
    assert!(run(10, f64::NAN, &[], c.joints[0]).is_err());
    assert!(run(10, 0.005, &[], JointId(9999)).is_err());
    // A missing part is skipped with why, not an error.
    let r = run(2, 0.005, &[c.crank, InstanceId(9999)], c.joints[0]).unwrap();
    assert_eq!(r.skipped.len(), 1);
}

/// An inline six of box parts: a crank (a stub of shaft ahead of the cylinders, clear of the
/// rods) turning about X; per cylinder a rod, a piston sliding
/// along Z, two valves above the piston (2 mm clear at top dead centre), four bolts and two
/// liner plates (3.5 mm clear of the piston's sides). 61 parts.
fn six() -> (Scene, JointId, Vec<(InstanceId, [InstanceId; 2])>) {
    const ER: f64 = 0.04;
    const EL: f64 = 0.14;
    let mut s = Scene::new();
    let crank = block(
        &mut s,
        p(-0.12, -0.01, -0.01),
        p(-0.06, 0.01, 0.01),
        DAffine3::IDENTITY,
    );
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
    let mut valves = Vec::new();
    for c in 0..6 {
        let xc = 0.1 * c as f64;
        let throw = (c % 3) as f64 * 120f64.to_radians();
        let pin = p(xc, ER * throw.sin(), ER * throw.cos());
        let wrist = p(xc, 0.0, pin.z + (EL * EL - pin.y * pin.y).sqrt());
        let dir = (wrist - pin).normalize();
        let tilt = DQuat::from_rotation_arc(DVec3::Z, dir);
        let rod = block(
            &mut s,
            p(-0.005, -0.005, -0.005),
            p(0.005, 0.005, EL - 0.025),
            DAffine3::from_rotation_translation(tilt, pin),
        );
        let piston = block(
            &mut s,
            p(xc - 0.03, -0.03, wrist.z - 0.02),
            p(xc + 0.03, 0.03, wrist.z + 0.03),
            DAffine3::IDENTITY,
        );
        let top = ER + EL + 0.03 + 0.002;
        let pair = [-1.0, 1.0].map(|side| {
            block(
                &mut s,
                p(xc + side * 0.015 - 0.01, -0.01, top),
                p(xc + side * 0.015 + 0.01, 0.01, top + 0.05),
                DAffine3::IDENTITY,
            )
        });
        valves.push((piston, pair));
        for (bx, by) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            let at = p(xc + bx * 0.04, by * 0.04, 0.0);
            block(
                &mut s,
                at + p(-0.0025, -0.0025, 0.1),
                at + p(0.0025, 0.0025, 0.25),
                DAffine3::IDENTITY,
            );
        }
        for side in [-1.0, 1.0] {
            let y0: f64 = side * 0.0335;
            let y1 = side * 0.04;
            block(
                &mut s,
                p(xc - 0.03, y0.min(y1), 0.1),
                p(xc + 0.03, y0.max(y1), 0.25),
                DAffine3::IDENTITY,
            );
        }
        join(
            &mut s,
            JointKind::Revolute,
            Some(crank),
            rod,
            pin,
            x,
            DVec3::Z,
        );
        join(
            &mut s,
            JointKind::Revolute,
            Some(rod),
            piston,
            wrist,
            x,
            DVec3::Z,
        );
        join(
            &mut s,
            JointKind::Prismatic,
            None,
            piston,
            wrist,
            DVec3::Z,
            x,
        );
    }
    (s, crank_joint, valves)
}

fn sweep_six(steps: usize) -> f64 {
    let (s, j, valves) = six();
    assert_eq!(s.root().instances().count(), 61);
    let t = Instant::now();
    let r = sweep_check(
        &s,
        j,
        0.0,
        360f64.to_radians(),
        steps,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    let elapsed = t.elapsed().as_secs_f64();
    assert_eq!(r.stopped, None);
    let hits: Vec<_> = r.pairs.iter().filter(|q| q.interferes.is_some()).collect();
    assert!(hits.is_empty(), "{hits:?}");
    // Each piston comes 2 mm from its valves at top dead centre, where the crank has turned
    // by its throw: 0, 120 or 240 degrees.
    for (c, (piston, pair)) in valves.iter().enumerate() {
        let tdc = [0.0, 120.0, 240.0][c % 3];
        for v in pair {
            let q = pair_of(&r, *piston, *v);
            assert!((q.clearance - 0.002).abs() < 1e-9, "{c}: {}", q.clearance);
            let at = q.clearance_at.to_degrees();
            let off = (at - tdc).rem_euclid(360.0);
            assert!(off.min(360.0 - off) < 1e-6, "{c}: {at}");
        }
    }
    elapsed
}

fn pair_of(r: &SweepReport, a: InstanceId, b: InstanceId) -> &SweepPair {
    pair(r, a, b).unwrap_or_else(|| panic!("{a:?} {b:?} missing"))
}

#[test]
fn a_sixty_part_engine_sweeps_36_steps() {
    sweep_six(37);
}

#[test]
#[ignore = "timing; run in release"]
fn perf_a_sixty_part_engine_sweeps_720_steps_in_under_5_s() {
    let elapsed = sweep_six(721);
    eprintln!("720 step sweep of 61 parts: {elapsed:.2} s");
    assert!(elapsed < 5.0, "{elapsed}");
}

#[test]
fn a_sweep_that_runs_too_many_booleans_stops_with_a_reason() {
    let c = slider_crank();
    let r = sweep_check_budget(
        &c.scene,
        c.joints[0],
        90f64.to_radians(),
        450f64.to_radians(),
        361,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
        3,
    )
    .unwrap();
    let (_, why) = r.stopped.expect("the sweep stops");
    assert_eq!(why, SWEEP_TOO_COSTLY);
    assert!(r.steps < 361, "{}", r.steps);
}

#[test]
fn parts_the_joint_does_not_move_are_checked_once() {
    // Two fixed blocks overlapping each other beside the slider-crank: their pair stands the
    // same at every step, so its boolean runs once and a budget of 3 does not stop the sweep.
    let mut c = slider_crank();
    let a = block(
        &mut c.scene,
        p(0.0, 0.2, 0.0),
        p(0.02, 0.22, 0.02),
        DAffine3::IDENTITY,
    );
    let b = block(
        &mut c.scene,
        p(0.01, 0.2, 0.0),
        p(0.03, 0.22, 0.02),
        DAffine3::IDENTITY,
    );
    let ids = [c.piston, c.stop, a, b];
    let r = sweep_check_budget(
        &c.scene,
        c.joints[0],
        0.0,
        360f64.to_radians(),
        73,
        None,
        &ids,
        SWEEP_REPORT_WITHIN,
        3,
    )
    .unwrap();
    assert_eq!(r.stopped, None);
    assert_eq!(r.steps, 73);
    let q = pair_of(&r, a, b);
    let (first, last) = q.interferes.expect("the fixed blocks overlap");
    assert_eq!(first, 0.0);
    assert!((last - 360f64.to_radians()).abs() < 1e-12, "{last}");
    assert!(
        (q.volume - 0.01 * 0.02 * 0.02).abs() < 1e-12,
        "{}",
        q.volume
    );
    assert_eq!(q.volume_at, Some(0.0));
    assert_eq!(q.clearance, 0.0);
    // the moving pairs are still checked at every step: the piston clears the stop by 2 mm
    // at bottom dead centre
    let stop = pair_of(&r, c.piston, c.stop);
    assert!(stop.interferes.is_none());
    assert!((stop.clearance - 0.002).abs() < 1e-9, "{}", stop.clearance);
    assert!((stop.clearance_at.to_degrees() - 180.0).abs() < 1e-6);
}

/// A shaft turning about Z with ten eccentric discs fixed to it, each turning inside a ring
/// 1 mm clear at its nearest, and a pin driven through each ring: round parts of many
/// triangles near each other at every step, a boolean per disc and shaft at every step (they
/// turn together), and the pins and rings, which do not move, checked once. 31 parts.
fn discs() -> (Scene, JointId, Vec<(InstanceId, InstanceId, InstanceId)>) {
    let mut s = Scene::new();
    let add = |s: &mut Scene, solid: Solid, at: DVec3| {
        s.add_solid(
            MeshId::Root,
            &solid,
            DefinitionKind::Group,
            "P",
            DAffine3::from_translation(at),
            None,
        )
        .unwrap()
        .0
    };
    let shaft = add(
        &mut s,
        Solid::cylinder(0.01, 0.6, 48).unwrap(),
        p(0.0, 0.0, -0.05),
    );
    let turn = join(
        &mut s,
        JointKind::Revolute,
        None,
        shaft,
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    let mut rigs = Vec::new();
    for k in 0..10 {
        let z = 0.05 * k as f64;
        let disc = add(
            &mut s,
            Solid::cylinder(0.03, 0.01, 96).unwrap(),
            p(0.005, 0.0, z),
        );
        join(
            &mut s,
            JointKind::Fixed,
            Some(shaft),
            disc,
            p(0.0, 0.0, z),
            DVec3::Z,
            DVec3::X,
        );
        let ring = add(
            &mut s,
            Solid::tube(0.05, 0.036, 0.01, 96).unwrap(),
            p(0.0, 0.0, z),
        );
        let pin = add(
            &mut s,
            Solid::cylinder(0.004, 0.03, 32).unwrap(),
            p(0.043, 0.0, z - 0.01),
        );
        rigs.push((disc, ring, pin));
    }
    (s, turn, rigs)
}

fn sweep_discs(steps: usize) -> f64 {
    let (s, j, rigs) = discs();
    let t = Instant::now();
    let r = sweep_check(
        &s,
        j,
        0.0,
        360f64.to_radians(),
        steps,
        None,
        &[],
        SWEEP_REPORT_WITHIN,
    )
    .unwrap();
    let elapsed = t.elapsed().as_secs_f64();
    assert_eq!((r.stopped.clone(), r.steps), (None, steps));
    for (disc, ring, pin) in rigs {
        let q = pair_of(&r, disc, ring);
        assert!(q.interferes.is_none(), "{q:?}");
        assert!(
            q.clearance > 0.0009 && q.clearance < 0.001,
            "{}",
            q.clearance
        );
        let q = pair_of(&r, ring, pin);
        let area = 0.5 * 32.0 * 0.004f64.powi(2) * (2.0 * PI / 32.0).sin();
        assert!((q.volume - area * 0.01).abs() < 1e-12, "{}", q.volume);
        assert_eq!(q.interferes, Some((0.0, 360f64.to_radians())));
    }
    elapsed
}

#[test]
fn round_parts_turning_near_each_other_sweep() {
    sweep_discs(5);
}

#[test]
#[ignore = "timing; run in release"]
fn perf_round_parts_turning_near_each_other_sweep_72_steps_in_under_3_s() {
    let elapsed = sweep_discs(73);
    eprintln!("72 step sweep of 31 round parts: {elapsed:.2} s");
    assert!(elapsed < 3.0, "{elapsed}");
}
