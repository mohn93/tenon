//! Driving a joint: the parts connected to it through joints move so that every joint of the
//! assembly holds with the driven joint at its new value.
//!
//! The unknowns are a small rigid motion (a rotation vector about a pivot, then a translation)
//! per movable part, applied on the left of its current transform, so a scaled or mirrored
//! part keeps its scale and mirror. Each joint gives residuals from its two world frames (see
//! [`rows`]), the driven joint one more per driven quantity. Damped least squares
//! (Levenberg-Marquardt) with a central-difference Jacobian, built joint by joint, solves
//! them; a large drive is taken in small steps from the current value, so a closed loop keeps
//! its branch (a piston stays on its side of the crank).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::{DAffine3, DQuat, DVec3};

use crate::MeshId;
use crate::edit::KernelError;
use crate::ids::{InstanceId, JointId};
use crate::joint::{
    Joint, JointFrame, JointKind, VALUE_TOLERANCE, frame_angle, frame_distance, measure_joint,
    wrap_angle,
};
use crate::scene::{Scene, SceneChange, Step};

/// The greatest residual (meters or radians, as a vector length) of a solved pose.
pub const DRIVE_TOLERANCE: f64 = 1e-9;

/// The largest turn of one drive step (10 degrees).
const MAX_ANGLE_STEP: f64 = 10.0 * std::f64::consts::PI / 180.0;

/// How many steps a sliding drive is taken in.
const SLIDE_STEPS: usize = 10;

/// Iterations per step at most.
const MAX_ITERATIONS: usize = 50;

/// A squared residual this small is solved.
const CONVERGED: f64 = 1e-24;

/// The central-difference step of the Jacobian (radians or meters).
const NUDGE: f64 = 1e-6;

/// The least and greatest damping.
const MIN_DAMPING: f64 = 1e-10;
const MAX_DAMPING: f64 = 1e10;

/// One side of a joint in the solve: a part held where it is (or the ground, at identity), or
/// the movable part with that index.
#[derive(Clone, Copy)]
enum Body {
    Held(DAffine3),
    Part(usize),
}

/// A joint of the assembly as the solver sees it.
struct Link {
    id: JointId,
    kind: JointKind,
    a: Body,
    b: Body,
    frame_a: JointFrame,
    frame_b: JointFrame,
    driven: bool,
}

/// The assembly being solved: its joints, and the movable parts' current transforms.
struct System {
    links: Vec<Link>,
    parts: Vec<InstanceId>,
    poses: Vec<DAffine3>,
    /// Where each part's rotation unknowns turn about: the mean of its joints' origins.
    pivots: Vec<DVec3>,
}

/// The driven value and slide of one step.
#[derive(Clone, Copy)]
struct Target {
    value: f64,
    slide: f64,
}

/// At most 8 residuals per joint: 6 for fixed, 2 for a driven cylindrical.
type Rows = ([f64; 8], usize);

/// The residuals of a joint of `kind` whose frames stand at `a` and `b`: zero when it holds.
/// With `u`, `v` across `a`'s axis: revolute 5 (`b`'s origin off `a`'s along u, v and the
/// axis; `b`'s axis along u and v), cylindrical 4 (as revolute, sliding free), prismatic 5
/// (cylindrical's, and `b`'s reference along v), fixed 6 (the origins' offset, `b`'s axis along
/// u and v, its reference along v), ball 3 (the origins' offset). With `drive`, also the
/// driven value (and slide, for cylindrical) less the target.
fn rows(kind: JointKind, a: &JointFrame, b: &JointFrame, drive: Option<Target>) -> Rows {
    let u = a.reference;
    let v = a.axis.cross(u);
    let d = b.origin - a.origin;
    let mut out = [0.0; 8];
    let mut n = 0;
    let mut push = |x: f64| {
        out[n] = x;
        n += 1;
    };
    match kind {
        JointKind::Revolute => {
            for x in [
                d.dot(u),
                d.dot(v),
                d.dot(a.axis),
                b.axis.dot(u),
                b.axis.dot(v),
            ] {
                push(x);
            }
        }
        JointKind::Cylindrical => {
            for x in [d.dot(u), d.dot(v), b.axis.dot(u), b.axis.dot(v)] {
                push(x);
            }
        }
        JointKind::Prismatic => {
            for x in [
                d.dot(u),
                d.dot(v),
                b.axis.dot(u),
                b.axis.dot(v),
                b.reference.dot(v),
            ] {
                push(x);
            }
        }
        JointKind::Fixed => {
            for x in [
                d.x,
                d.y,
                d.z,
                b.axis.dot(u),
                b.axis.dot(v),
                b.reference.dot(v),
            ] {
                push(x);
            }
        }
        JointKind::Ball => {
            for x in [d.x, d.y, d.z] {
                push(x);
            }
        }
    }
    if let Some(t) = drive {
        match kind {
            JointKind::Revolute => push(wrap_angle(frame_angle(a, b) - t.value)),
            JointKind::Prismatic => push(frame_distance(a, b) - t.value),
            JointKind::Cylindrical => {
                push(wrap_angle(frame_angle(a, b) - t.value));
                push(frame_distance(a, b) - t.slide);
            }
            JointKind::Fixed | JointKind::Ball => {}
        }
    }
    (out, n)
}

/// The rigid motion turning by the rotation vector `w` about `pivot`, then moving by `t`.
fn rigid(w: DVec3, t: DVec3, pivot: DVec3) -> DAffine3 {
    DAffine3::from_translation(t + pivot)
        * DAffine3::from_quat(DQuat::from_scaled_axis(w))
        * DAffine3::from_translation(-pivot)
}

/// Unknown `k` (0-2 rotation about X, Y, Z; 3-5 translation along them) of a part, as a
/// vector of length `h`.
fn unknown(k: usize, h: f64) -> (DVec3, DVec3) {
    let e = DVec3::AXES[k % 3] * h;
    if k < 3 {
        (e, DVec3::ZERO)
    } else {
        (DVec3::ZERO, e)
    }
}

impl System {
    fn pose(&self, body: Body) -> DAffine3 {
        match body {
            Body::Held(t) => t,
            Body::Part(i) => self.poses[i],
        }
    }

    fn link_rows(&self, link: &Link, ta: &DAffine3, tb: &DAffine3, target: Target) -> Rows {
        let a = link.frame_a.world(ta);
        let b = link.frame_b.world(tb);
        rows(link.kind, &a, &b, link.driven.then_some(target))
    }

    /// The squared length of every residual with the parts at `poses`.
    fn cost(&self, poses: &[DAffine3], target: Target) -> f64 {
        let pose = |b: Body| match b {
            Body::Held(t) => t,
            Body::Part(i) => poses[i],
        };
        self.links
            .iter()
            .map(|l| {
                let (r, n) = self.link_rows(l, &pose(l.a), &pose(l.b), target);
                r[..n].iter().map(|x| x * x).sum::<f64>()
            })
            .sum()
    }

    /// The normal equations `J^T J` (row-major, 6 unknowns per part) and `J^T r` at the
    /// current poses.
    fn normal_equations(&self, target: Target) -> (Vec<f64>, Vec<f64>) {
        let n = 6 * self.parts.len();
        let mut jtj = vec![0.0; n * n];
        let mut jtr = vec![0.0; n];
        for link in &self.links {
            let (ta, tb) = (self.pose(link.a), self.pose(link.b));
            let (r0, m) = self.link_rows(link, &ta, &tb, target);
            // The columns of this joint: (unknown index, derivative of its rows).
            let mut cols: Vec<(usize, [f64; 8])> = Vec::with_capacity(12);
            for (side, body) in [(0, link.a), (1, link.b)] {
                let Body::Part(i) = body else { continue };
                for k in 0..6 {
                    let at = |h: f64| {
                        let (w, t) = unknown(k, h);
                        let moved = rigid(w, t, self.pivots[i]) * self.poses[i];
                        let (ta, tb) = if side == 0 { (moved, tb) } else { (ta, moved) };
                        self.link_rows(link, &ta, &tb, target).0
                    };
                    let (plus, minus) = (at(NUDGE), at(-NUDGE));
                    let mut d = [0.0; 8];
                    for r in 0..m {
                        d[r] = (plus[r] - minus[r]) / (2.0 * NUDGE);
                    }
                    cols.push((6 * i + k, d));
                }
            }
            for (p, dp) in &cols {
                jtr[*p] += (0..m).map(|r| dp[r] * r0[r]).sum::<f64>();
                for (q, dq) in &cols {
                    jtj[p * n + q] += (0..m).map(|r| dp[r] * dq[r]).sum::<f64>();
                }
            }
        }
        (jtj, jtr)
    }

    /// The poses after the update `x` (6 unknowns per part).
    fn updated(&self, x: &[f64]) -> Vec<DAffine3> {
        self.poses
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let w = DVec3::new(x[6 * i], x[6 * i + 1], x[6 * i + 2]);
                let d = DVec3::new(x[6 * i + 3], x[6 * i + 4], x[6 * i + 5]);
                rigid(w, d, self.pivots[i]) * *t
            })
            .collect()
    }

    /// Moves the parts toward meeting every joint with the driven one at `target`; the squared
    /// residual left.
    fn solve(&mut self, target: Target) -> f64 {
        let n = 6 * self.parts.len();
        let mut cost = self.cost(&self.poses, target);
        let mut damping = 1e-6;
        for _ in 0..MAX_ITERATIONS {
            if cost < CONVERGED || n == 0 {
                break;
            }
            let (jtj, jtr) = self.normal_equations(target);
            let mut improved = false;
            while damping < MAX_DAMPING {
                let mut a = jtj.clone();
                for i in 0..n {
                    a[i * n + i] += damping;
                }
                let mut x: Vec<f64> = jtr.iter().map(|g| -g).collect();
                if cholesky_solve(&mut a, n, &mut x) {
                    let trial = self.updated(&x);
                    let c = self.cost(&trial, target);
                    if c < cost {
                        self.poses = trial;
                        cost = c;
                        damping = (damping * 0.1).max(MIN_DAMPING);
                        improved = true;
                        break;
                    }
                }
                damping *= 10.0;
            }
            if !improved {
                break;
            }
        }
        cost
    }

    /// Every link's value and slide at the current poses, each angle the turn nearest the
    /// one in `values`; the driven link takes `target` exactly.
    fn measure(&self, values: &mut [(f64, f64)], target: Target) {
        for (link, v) in self.links.iter().zip(values.iter_mut()) {
            if link.driven {
                *v = (target.value, target.slide);
                continue;
            }
            let a = link.frame_a.world(&self.pose(link.a));
            let b = link.frame_b.world(&self.pose(link.b));
            *v = measure_joint(link.kind, &a, &b, v.0, v.1);
        }
    }
}

/// Solves `a x = b` for a symmetric positive definite `a` (row-major `n` x `n`, overwritten
/// by its Cholesky factor); `b` becomes `x`. False when `a` is not positive definite.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d.is_nan() || d <= 0.0 {
            return false;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= a[i * n + k] * b[k];
        }
        b[i] = s / a[i * n + i];
    }
    for i in (0..n).rev() {
        let mut s = b[i];
        for k in i + 1..n {
            s -= a[k * n + i] * b[k];
        }
        b[i] = s / a[i * n + i];
    }
    b.iter().all(|x| x.is_finite())
}

/// A joint value as people read it: degrees for an angle, meters for a distance.
fn shown(kind: JointKind, value: f64) -> String {
    let (x, unit) = if kind.is_angular() {
        (value.to_degrees(), " degrees")
    } else {
        (value, " m")
    };
    let text = format!("{x:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    let text = if text == "-0" { "0" } else { text };
    format!("{text}{unit}")
}

fn refuse(why: String) -> KernelError {
    KernelError::Joint(why)
}

impl Scene {
    /// Sets joint `id` to `value` (radians or meters; `slide` too for a cylindrical joint,
    /// absent keeping it) and moves the parts connected to it through joints so that every
    /// joint between them holds, the other joints' values free. Parts joined to the ground
    /// keep the ground fixed; an assembly with no ground holds the joint's part `a` in place.
    /// The moved parts' transforms and every changed joint value are the steps, applied. A
    /// value outside the joint's limits, a pose where the joints cannot all be met, or one
    /// where another joint passes its limits is refused and changes nothing; the current
    /// value changes nothing.
    pub fn drive_joint(
        &mut self,
        id: JointId,
        value: f64,
        slide: Option<f64>,
    ) -> Result<SceneChange, KernelError> {
        let driven = self.joint(id).ok_or(KernelError::JointNotFound)?.clone();
        if !driven.kind.has_value() {
            return Err(refuse("a fixed or ball joint cannot be driven".into()));
        }
        if !value.is_finite() || slide.is_some_and(|s| !s.is_finite()) {
            return Err(refuse("a joint's value must be a finite number".into()));
        }
        if slide.is_some() && driven.kind != JointKind::Cylindrical {
            return Err(refuse("only a cylindrical joint slides".into()));
        }
        let slide = slide.unwrap_or(driven.slide);
        if let Some((lo, hi)) = driven.limits
            && (value < lo || value > hi)
        {
            return Err(refuse(format!(
                "{} is outside the joint's limits ({} to {})",
                shown(driven.kind, value),
                shown(driven.kind, lo),
                shown(driven.kind, hi)
            )));
        }
        if value == driven.value && slide == driven.slide {
            return Ok(SceneChange::default());
        }
        let mut system = self.assembly(id, &driven);
        let mut values: Vec<(f64, f64)> = system
            .links
            .iter()
            .map(|l| {
                let j = &self.joints[&l.id];
                (j.value, j.slide)
            })
            .collect();
        let steps = drive_steps(&driven, value, slide);
        let unmet = || {
            refuse(format!(
                "the joints cannot all be met at {}",
                shown(driven.kind, value)
            ))
        };
        for s in 1..=steps {
            let f = s as f64 / steps as f64;
            let target = if s == steps {
                Target { value, slide }
            } else {
                Target {
                    value: driven.value + (value - driven.value) * f,
                    slide: driven.slide + (slide - driven.slide) * f,
                }
            };
            let cost = system.solve(target);
            if cost.is_nan() || cost.sqrt() > DRIVE_TOLERANCE {
                return Err(unmet());
            }
            system.measure(&mut values, target);
        }
        let mut joint_steps = Vec::new();
        for (link, (v, sl)) in system.links.iter().zip(&values) {
            let from = &self.joints[&link.id];
            let same = (v - from.value).abs() < VALUE_TOLERANCE
                && (sl - from.slide).abs() < VALUE_TOLERANCE;
            if same && !link.driven {
                continue;
            }
            if !link.driven && from.limits.is_some_and(|(lo, hi)| *v < lo || *v > hi) {
                return Err(refuse(format!(
                    "the joints cannot all be met at {}: joint {:?} would pass its limits",
                    shown(driven.kind, value),
                    from.name
                )));
            }
            let mut to = from.clone();
            to.value = *v;
            to.slide = *sl;
            joint_steps.push(Step::ChangeJoint(link.id, from.clone(), to));
        }
        let moved: Vec<(InstanceId, DAffine3)> = system
            .parts
            .iter()
            .zip(&system.poses)
            .filter(|(i, t)| {
                let from = self.root().instance(**i).expect("a part").transform;
                !from.abs_diff_eq(**t, 1e-14)
            })
            .map(|(i, t)| (*i, *t))
            .collect();
        let mut change = self.edit(MeshId::Root, |m| {
            for (i, t) in &moved {
                m.set_instance_transform(*i, *t);
            }
            Ok(m.take_changes())
        })?;
        for step in joint_steps {
            self.apply_step(&step);
            change.steps.push(step);
        }
        Ok(change)
    }

    /// The assembly of joint `id`: the parts connected to it through joints (not through the
    /// ground), every joint between them, and which parts move.
    fn assembly(&self, id: JointId, driven: &Joint) -> System {
        let mut neighbours: BTreeMap<InstanceId, Vec<InstanceId>> = BTreeMap::new();
        for (_, j) in self.joints() {
            if let Some(a) = j.a {
                neighbours.entry(a).or_default().push(j.b);
                neighbours.entry(j.b).or_default().push(a);
            }
        }
        let mut members: BTreeSet<InstanceId> = BTreeSet::new();
        let mut queue: VecDeque<InstanceId> = driven.a.into_iter().chain([driven.b]).collect();
        while let Some(i) = queue.pop_front() {
            if members.insert(i) {
                queue.extend(neighbours.get(&i).into_iter().flatten().copied());
            }
        }
        let joints: Vec<(JointId, &Joint)> = self
            .joints()
            .filter(|(_, j)| members.contains(&j.b))
            .collect();
        let grounded = joints.iter().any(|(_, j)| j.a.is_none());
        let held = if grounded { None } else { driven.a };
        let parts: Vec<InstanceId> = members
            .iter()
            .copied()
            .filter(|i| Some(*i) != held)
            .collect();
        let index: BTreeMap<InstanceId, usize> =
            parts.iter().enumerate().map(|(k, i)| (*i, k)).collect();
        let transform = |i: InstanceId| self.root().instance(i).expect("a part").transform;
        let body = |i: Option<InstanceId>| match i {
            None => Body::Held(DAffine3::IDENTITY),
            Some(i) => match index.get(&i) {
                Some(k) => Body::Part(*k),
                None => Body::Held(transform(i)),
            },
        };
        let links: Vec<Link> = joints
            .iter()
            .map(|(jid, j)| Link {
                id: *jid,
                kind: j.kind,
                a: body(j.a),
                b: body(Some(j.b)),
                frame_a: j.frame_a,
                frame_b: j.frame_b,
                driven: *jid == id,
            })
            .collect();
        let poses: Vec<DAffine3> = parts.iter().map(|i| transform(*i)).collect();
        let mut sums = vec![(DVec3::ZERO, 0usize); parts.len()];
        for (_, j) in &joints {
            let (a, b) = self.joint_world_frames(j);
            for (part, frame) in [(j.a, a), (Some(j.b), b)] {
                if let Some(k) = part.and_then(|i| index.get(&i)) {
                    sums[*k].0 += frame.origin;
                    sums[*k].1 += 1;
                }
            }
        }
        let pivots = sums
            .iter()
            .map(|(sum, n)| *sum / (*n).max(1) as f64)
            .collect();
        System {
            links,
            parts,
            poses,
            pivots,
        }
    }
}

/// How many steps a drive of `joint` to `value` and `slide` takes: turns of at most 10
/// degrees, slides in tenths.
fn drive_steps(joint: &Joint, value: f64, slide: f64) -> usize {
    // A hair under a whole number of steps is that number (720 degrees is 72 steps).
    let turn = |from: f64, to: f64| ((to - from).abs() / MAX_ANGLE_STEP - 1e-9).ceil() as usize;
    let slid = |from: f64, to: f64| if from == to { 1 } else { SLIDE_STEPS };
    let n = match joint.kind {
        JointKind::Revolute => turn(joint.value, value),
        JointKind::Prismatic => slid(joint.value, value),
        JointKind::Cylindrical => turn(joint.value, value).max(slid(joint.slide, slide)),
        JointKind::Fixed | JointKind::Ball => 1,
    };
    n.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_solves_a_small_system() {
        let mut a = vec![4.0, 2.0, 2.0, 3.0];
        let mut b = vec![2.0, 1.0];
        assert!(cholesky_solve(&mut a, 2, &mut b));
        assert!((b[0] - 0.5).abs() < 1e-15 && b[1].abs() < 1e-15);
        let mut singular = vec![1.0, 1.0, 1.0, 1.0];
        assert!(!cholesky_solve(&mut singular, 2, &mut [1.0, 1.0]));
    }

    #[test]
    fn values_read_as_people_write_them() {
        assert_eq!(shown(JointKind::Revolute, 90f64.to_radians()), "90 degrees");
        assert_eq!(shown(JointKind::Prismatic, 0.25), "0.25 m");
        assert_eq!(shown(JointKind::Revolute, -1e-12), "0 degrees");
    }

    #[test]
    fn big_turns_go_in_ten_degree_steps() {
        let j = Joint {
            name: "J".into(),
            kind: JointKind::Revolute,
            a: None,
            b: InstanceId(1),
            frame_a: JointFrame {
                origin: DVec3::ZERO,
                axis: DVec3::Z,
                reference: DVec3::X,
            },
            frame_b: JointFrame {
                origin: DVec3::ZERO,
                axis: DVec3::Z,
                reference: DVec3::X,
            },
            value: 0.0,
            slide: 0.0,
            limits: None,
        };
        assert_eq!(drive_steps(&j, 720f64.to_radians(), 0.0), 72);
        assert_eq!(drive_steps(&j, 1e-3, 0.0), 1);
    }
}
