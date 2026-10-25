//! Driving a joint: the parts connected to it through joints move so that every joint of the
//! assembly holds with the driven joint at its new value.
//!
//! The unknowns are a small rigid motion (a rotation vector about a pivot, then a translation)
//! per movable part, applied on the left of its current transform, so a scaled or mirrored
//! part keeps its scale and mirror. Each joint gives residuals from its two world frames (see
//! [`rows`]), the driven joint one more per driven quantity. Damped least squares
//! (Levenberg-Marquardt) with a central-difference Jacobian, built joint by joint, solves
//! them; a large drive is taken in small steps from the current value, each starting from the
//! last step's motion taken again, so a closed loop keeps its branch (a piston stays on its
//! side of the crank, a parallelogram stays one through its flat point).
//!
//! A coupling between two joints of the assembly adds one residual, the driven joint's value
//! less the ratio times the driver's less the offset, both values measured from the pose as
//! the turn nearest their values at the start of the step (so a value counts on past a full
//! turn: a cam coupled 1:2 to a crank turns once in two). For a driven joint that turns, the
//! residual is taken less the whole turns that bring it nearest 0: a part stands the same
//! after a whole turn, so the coupling holds whichever turn the driven joint's stored value
//! counts (after going to a scene that stores positions, say, or with an offset past half a
//! turn). Joints linked only through a coupling (a crank and a cam on separate shafts) are one
//! assembly. A drive whose steps (of at most 10 degrees for every joint coupled to it) would
//! number more than [`MAX_DRIVE_STEPS`] is refused before any work.
//!
//! A cam follower in the assembly (its joint's part is reached from the cam, or through its
//! own joints) is met in a second pass of each step: once the other residuals are met with
//! the parts where they stand, its residual, the follower joint's value less the value where
//! its axis line touches the cam in the pose the first pass left it in (see [`crate::cam`]),
//! is added and the step solved again from there; the follower does not pull on the cam. A
//! cam the drive does not move is held where it is. A follower whose axis line misses the cam
//! once the other joints are met refuses the drive.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use glam::{DAffine3, DQuat, DVec3};

use crate::MeshId;
use crate::bvh::Bvh;
use crate::cam::contact;
use crate::edit::KernelError;
use crate::ids::{CamFollowerId, DefinitionId, InstanceId, JointId};
use crate::joint::{
    Joint, JointFrame, JointKind, VALUE_TOLERANCE, frame_angle, frame_distance, measure_joint,
    wrap_angle,
};
use crate::scene::{Scene, SceneChange, Step};

/// The greatest residual (meters or radians, as a vector length) of a solved pose.
pub const DRIVE_TOLERANCE: f64 = 1e-9;

/// How far past its limits (radians or meters) another joint may be measured and still be
/// taken as at them: its value is then set to the limit.
const LIMIT_TOLERANCE: f64 = 1e-9;

/// The largest turn of one drive step (10 degrees).
const MAX_ANGLE_STEP: f64 = 10.0 * std::f64::consts::PI / 180.0;

/// The most steps one drive may take: more is refused before any work (a ratio coupling a
/// joint that turns many times for each turn of the driven one, or a drive of many turns).
pub const MAX_DRIVE_STEPS: usize = 20_000;

/// How close (radians or meters) a joint of [`Scene::drive_joints`] must stand to its value
/// once every joint of the call is driven.
const HELD_TOLERANCE: f64 = 1e-6;

thread_local! {
    /// How many drive steps this thread has solved.
    static STEPS_SOLVED: Cell<u64> = const { Cell::new(0) };
}

/// How many drive steps (see [`Scene::drive_joint`]) this thread has solved so far: tests
/// measure the work a sequence of drives does.
pub fn solve_steps_taken() -> u64 {
    STEPS_SOLVED.with(Cell::get)
}

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

/// A coupling of the assembly: `links[driven]`'s value less `ratio` times `links[driver]`'s
/// less `offset` is held at 0.
struct Bond {
    driver: usize,
    driven: usize,
    ratio: f64,
    offset: f64,
}

/// A cam follower of the assembly: `links[link]`'s value less where its axis line touches
/// the cam (`tree` placed by `cam`'s pose) is held at 0.
struct Rider {
    id: CamFollowerId,
    link: usize,
    cam: Body,
    cam_id: InstanceId,
    /// Shared by the riders of one cam definition.
    tree: Rc<Bvh>,
    preload: f64,
    roller_radius: f64,
    /// The cam's pose once the other residuals are met: the second pass touches the cam
    /// there, so the follower sets its value without pulling on the cam.
    frozen: Option<DAffine3>,
}

/// How a step's solve ended.
enum Outcome {
    Met,
    Unmet,
    /// The rider with this index has no contact once the other residuals are met.
    NoContact(usize),
}

/// The assembly being solved: its joints, and the movable parts' current transforms.
struct System {
    links: Vec<Link>,
    bonds: Vec<Bond>,
    riders: Vec<Rider>,
    /// Whether the riders' residuals count (the second pass of a step).
    riding: bool,
    /// Each link's value and slide at the start of the step: a value is measured as the turn
    /// nearest it.
    near: Vec<(f64, f64)>,
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

/// At most 10 residuals per joint: 9 for fixed, 9 for a driven prismatic.
type Rows = ([f64; 10], usize);

/// The residuals of a joint of `kind` whose frames stand at `a` and `b`: zero when it holds.
/// Directions are compared as differences (`b`'s axis less `a`'s, in world X, Y and Z), not
/// as perpendicular components, so a part turned half a turn does not meet its joint. With
/// `u`, `v` across `a`'s axis: revolute 6 (the origins' offset; the axes' difference),
/// cylindrical 5 (`b`'s origin off `a`'s along u and v, sliding free; the axes' difference),
/// prismatic 8 (cylindrical's, and the references' difference), fixed 9 (the origins'
/// offset, the axes' and the references' differences), ball 3 (the origins' offset). With
/// `drive`, also the driven value (and slide, for cylindrical) less the target.
fn rows(kind: JointKind, a: &JointFrame, b: &JointFrame, drive: Option<Target>) -> Rows {
    let u = a.reference;
    let v = a.axis.cross(u);
    let d = b.origin - a.origin;
    let axes = b.axis - a.axis;
    let references = b.reference - a.reference;
    let mut out = [0.0; 10];
    let mut n = 0;
    let mut push = |xs: &[f64]| {
        for x in xs {
            out[n] = *x;
            n += 1;
        }
    };
    match kind {
        JointKind::Revolute => {
            push(&[d.x, d.y, d.z]);
            push(&axes.to_array());
        }
        JointKind::Cylindrical => {
            push(&[d.dot(u), d.dot(v)]);
            push(&axes.to_array());
        }
        JointKind::Prismatic => {
            push(&[d.dot(u), d.dot(v)]);
            push(&axes.to_array());
            push(&references.to_array());
        }
        JointKind::Fixed => {
            push(&[d.x, d.y, d.z]);
            push(&axes.to_array());
            push(&references.to_array());
        }
        JointKind::Ball => push(&[d.x, d.y, d.z]),
    }
    if let Some(t) = drive {
        match kind {
            JointKind::Revolute => push(&[wrap_angle(frame_angle(a, b) - t.value)]),
            JointKind::Prismatic => push(&[frame_distance(a, b) - t.value]),
            JointKind::Cylindrical => {
                push(&[
                    wrap_angle(frame_angle(a, b) - t.value),
                    frame_distance(a, b) - t.slide,
                ]);
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
        let joints: f64 = self
            .links
            .iter()
            .map(|l| {
                let (r, n) = self.link_rows(l, &pose(l.a), &pose(l.b), target);
                r[..n].iter().map(|x| x * x).sum::<f64>()
            })
            .sum();
        let bonds: f64 = self
            .bonds
            .iter()
            .map(|b| self.bond_residual(b, &pose).powi(2))
            .sum();
        let mut riders = 0.0;
        if self.riding {
            for r in &self.riders {
                // A pose where a follower misses its cam is no pose at all.
                match self.rider_residual(r, &pose) {
                    Some(x) => riders += x * x,
                    None => return f64::NAN,
                }
            }
        }
        joints + bonds + riders
    }

    /// The residual of a cam follower with the parts at `pose`: zero when it touches; None
    /// when its axis line misses the cam.
    fn rider_residual(&self, rider: &Rider, pose: &dyn Fn(Body) -> DAffine3) -> Option<f64> {
        let link = &self.links[rider.link];
        let a = link.frame_a.world(&pose(link.a));
        let b = link.frame_b.world(&pose(link.b));
        let touch = contact(
            &rider.tree,
            &rider.frozen.unwrap_or_else(|| pose(rider.cam)),
            &a,
            rider.preload,
            rider.roller_radius,
        )?;
        Some(frame_distance(&a, &b) - touch)
    }

    /// The first rider whose axis line misses its cam with the parts where they stand.
    fn lost_contact(&self) -> Option<usize> {
        let current = |b: Body| self.pose(b);
        self.riders
            .iter()
            .position(|r| self.rider_residual(r, &current).is_none())
    }

    /// Solves one step: the joints and couplings, then (with any riders) everything again
    /// with the riders' residuals.
    fn solve_step(&mut self, target: Target) -> Outcome {
        STEPS_SOLVED.with(|n| n.set(n.get() + 1));
        let met = |cost: f64| !cost.is_nan() && cost.sqrt() <= DRIVE_TOLERANCE;
        self.riding = false;
        if !met(self.solve(target)) {
            return Outcome::Unmet;
        }
        if self.riders.is_empty() {
            return Outcome::Met;
        }
        if let Some(k) = self.lost_contact() {
            return Outcome::NoContact(k);
        }
        for r in &mut self.riders {
            r.frozen = Some(match r.cam {
                Body::Held(t) => t,
                Body::Part(i) => self.poses[i],
            });
        }
        self.riding = true;
        let cost = self.solve(target);
        self.riding = false;
        for r in &mut self.riders {
            r.frozen = None;
        }
        if met(cost) {
            Outcome::Met
        } else {
            Outcome::Unmet
        }
    }

    /// The value of `links[k]` with the parts at `pose`, the turn nearest its value at the
    /// start of the step.
    fn link_value(&self, k: usize, pose: &dyn Fn(Body) -> DAffine3) -> f64 {
        let link = &self.links[k];
        let a = link.frame_a.world(&pose(link.a));
        let b = link.frame_b.world(&pose(link.b));
        let (value, slide) = self.near[k];
        measure_joint(link.kind, &a, &b, value, slide).0
    }

    /// The residual of a coupling with the parts at `pose`: zero when it holds. A driven joint
    /// that turns compares less whole turns, the residual in [-pi, pi].
    fn bond_residual(&self, bond: &Bond, pose: &dyn Fn(Body) -> DAffine3) -> f64 {
        let r = self.link_value(bond.driven, pose)
            - bond.ratio * self.link_value(bond.driver, pose)
            - bond.offset;
        if self.links[bond.driven].kind.is_angular() {
            wrap_angle(r)
        } else {
            r
        }
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
            let mut cols: Vec<(usize, [f64; 10])> = Vec::with_capacity(12);
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
                    let mut d = [0.0; 10];
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
        for bond in &self.bonds {
            let current = |b: Body| self.pose(b);
            let r0 = self.bond_residual(bond, &current);
            let mut parts: Vec<usize> = [bond.driver, bond.driven]
                .iter()
                .flat_map(|k| [self.links[*k].a, self.links[*k].b])
                .filter_map(|b| match b {
                    Body::Part(i) => Some(i),
                    Body::Held(_) => None,
                })
                .collect();
            parts.sort_unstable();
            parts.dedup();
            let mut cols: Vec<(usize, f64)> = Vec::with_capacity(6 * parts.len());
            for i in parts {
                for k in 0..6 {
                    let at = |h: f64| {
                        let (w, t) = unknown(k, h);
                        let moved = rigid(w, t, self.pivots[i]) * self.poses[i];
                        let pose = |b: Body| match b {
                            Body::Part(j) if j == i => moved,
                            b => self.pose(b),
                        };
                        self.bond_residual(bond, &pose)
                    };
                    cols.push((6 * i + k, (at(NUDGE) - at(-NUDGE)) / (2.0 * NUDGE)));
                }
            }
            for (p, dp) in &cols {
                jtr[*p] += dp * r0;
                for (q, dq) in &cols {
                    jtj[p * n + q] += dp * dq;
                }
            }
        }
        if self.riding {
            for rider in &self.riders {
                let current = |b: Body| self.pose(b);
                let Some(r0) = self.rider_residual(rider, &current) else {
                    continue;
                };
                let link = &self.links[rider.link];
                let mut parts: Vec<usize> = [link.a, link.b]
                    .iter()
                    .filter_map(|b| match b {
                        Body::Part(i) => Some(*i),
                        Body::Held(_) => None,
                    })
                    .collect();
                parts.sort_unstable();
                parts.dedup();
                let mut cols: Vec<(usize, f64)> = Vec::with_capacity(6 * parts.len());
                for i in parts {
                    for k in 0..6 {
                        let at = |h: f64| {
                            let (w, t) = unknown(k, h);
                            let moved = rigid(w, t, self.pivots[i]) * self.poses[i];
                            let pose = |b: Body| match b {
                                Body::Part(j) if j == i => moved,
                                b => self.pose(b),
                            };
                            self.rider_residual(rider, &pose)
                        };
                        // A nudge off the cam's edge gives no slope that way.
                        let slope = match (at(NUDGE), at(-NUDGE)) {
                            (Some(p), Some(m)) => (p - m) / (2.0 * NUDGE),
                            _ => 0.0,
                        };
                        cols.push((6 * i + k, slope));
                    }
                }
                for (p, dp) in &cols {
                    jtr[*p] += dp * r0;
                    for (q, dq) in &cols {
                        jtj[p * n + q] += dp * dq;
                    }
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

    /// How many steps keep every angle coupled to the driven link's turning by at most 10
    /// degrees a step, when the driven link's value changes by `change`: each coupled link's
    /// change follows through the couplings, either way.
    fn coupled_steps(&self, change: f64) -> usize {
        let mut changes: Vec<Option<f64>> = self
            .links
            .iter()
            .map(|l| l.driven.then_some(change))
            .collect();
        let mut more = true;
        while more {
            more = false;
            for b in &self.bonds {
                match (changes[b.driver], changes[b.driven]) {
                    (Some(d), None) => changes[b.driven] = Some(b.ratio * d),
                    (None, Some(d)) => changes[b.driver] = Some(d / b.ratio),
                    _ => continue,
                }
                more = true;
            }
        }
        self.links
            .iter()
            .zip(&changes)
            .filter(|(l, _)| l.kind.is_angular())
            .filter_map(|(_, c)| *c)
            .map(|c| turn_steps(c.abs()))
            .max()
            .unwrap_or(1)
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
///
/// The factor of an assembly is mostly zeros (each joint ties two parts), so each column's
/// sums run over only the nonzero entries of its row of the factor. The terms left out are
/// products with a zero, which change no sum, so the factor is that of the full sums; a
/// factor entry that is infinite or NaN is nonzero and taken, failing its row's diagonal
/// either way.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    let mut nonzero: Vec<usize> = Vec::with_capacity(n);
    for j in 0..n {
        nonzero.clear();
        nonzero.extend((0..j).filter(|k| a[j * n + k] != 0.0));
        let mut d = a[j * n + j];
        for &k in &nonzero {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d.is_nan() || d <= 0.0 {
            return false;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for &k in &nonzero {
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

/// The triangle tree of each cam definition, in its own coordinates.
pub(crate) type CamTrees = BTreeMap<DefinitionId, Rc<Bvh>>;

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
    /// where another joint passes its limits (checked after every step, so a joint cannot pass
    /// a limit and come back) is refused and changes nothing; the current value (within
    /// 1e-12) changes nothing.
    pub fn drive_joint(
        &mut self,
        id: JointId,
        value: f64,
        slide: Option<f64>,
    ) -> Result<SceneChange, KernelError> {
        self.drive_joint_with(id, value, slide, &mut CamTrees::new())
    }

    /// [`Scene::drive_joint`] taking the cams' trees from `cams` and keeping the ones it
    /// builds there, for drives one after another while no definition changes (a sweep).
    pub(crate) fn drive_joint_with(
        &mut self,
        id: JointId,
        value: f64,
        slide: Option<f64>,
        cams: &mut CamTrees,
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
        if let Some((w, c)) = self.follower_of(id) {
            return Err(refuse(format!(
                "joint {:?} follows cam i{} ({w}): its value comes from the cam, so drive the \
                 cam's joints instead",
                driven.name, c.cam.0
            )));
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
        if (value - driven.value).abs() < VALUE_TOLERANCE
            && (slide - driven.slide).abs() < VALUE_TOLERANCE
        {
            return Ok(SceneChange::default());
        }
        let mut system = self.assembly(id, &driven, cams);
        let mut values: Vec<(f64, f64)> = system
            .links
            .iter()
            .map(|l| {
                let j = &self.joints[&l.id];
                (j.value, j.slide)
            })
            .collect();
        let steps =
            drive_steps(&driven, value, slide).max(system.coupled_steps(value - driven.value));
        if steps > MAX_DRIVE_STEPS {
            return Err(refuse(format!(
                "driving to {} would take more than {MAX_DRIVE_STEPS} steps (of at most 10 \
                 degrees for the joint and every joint coupled to it): drive it in smaller moves",
                shown(driven.kind, value)
            )));
        }
        let unmet = || {
            refuse(format!(
                "the joints cannot all be met at {}",
                shown(driven.kind, value)
            ))
        };
        // The poses before the last step: its motion, taken again, is the next step's first
        // guess, so a linkage passing a change point (a parallelogram going flat) keeps on
        // through it on its branch.
        let mut previous: Option<Vec<DAffine3>> = None;
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
            system.near.clone_from(&values);
            let from = system.poses.clone();
            let mut outcome = Outcome::Unmet;
            if let Some(before) = &previous {
                system.poses = from
                    .iter()
                    .zip(before)
                    .map(|(now, was)| (*now * was.inverse()) * *now)
                    .collect();
                outcome = system.solve_step(target);
            }
            if !matches!(outcome, Outcome::Met) {
                system.poses.clone_from(&from);
                outcome = system.solve_step(target);
            }
            match outcome {
                Outcome::Met => {}
                Outcome::Unmet => return Err(unmet()),
                Outcome::NoContact(k) => {
                    let rider = &system.riders[k];
                    let name = &self.joints[&system.links[rider.link].id].name;
                    return Err(refuse(format!(
                        "the joints cannot all be met at {}: cam follower {} (joint {name:?}) \
                         loses contact with cam i{}, its axis line missing the cam",
                        shown(driven.kind, value),
                        rider.id,
                        rider.cam_id.0
                    )));
                }
            }
            previous = Some(from);
            system.measure(&mut values, target);
            for (link, v) in system.links.iter().zip(values.iter_mut()) {
                let Some((lo, hi)) = self.joints[&link.id].limits else {
                    continue;
                };
                if link.driven {
                    continue;
                }
                if v.0 < lo - LIMIT_TOLERANCE || v.0 > hi + LIMIT_TOLERANCE {
                    return Err(refuse(format!(
                        "the joints cannot all be met at {}: joint {:?} would pass its limits",
                        shown(driven.kind, value),
                        self.joints[&link.id].name
                    )));
                }
                v.0 = v.0.clamp(lo, hi);
            }
        }
        let mut joint_steps = Vec::new();
        for (link, (v, sl)) in system.links.iter().zip(&values) {
            let from = &self.joints[&link.id];
            let same = (v - from.value).abs() < VALUE_TOLERANCE
                && (sl - from.slide).abs() < VALUE_TOLERANCE;
            if same && !link.driven {
                continue;
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

    /// Drives each joint of `values` (id, value, slide: radians or meters; no slide keeps it)
    /// in turn, as [`Scene::drive_joint`] does, in the order of their ids (a joint named twice
    /// takes its last value); the joints that follow a cam, and those that no longer exist or
    /// have no value, are skipped. Once all are driven, each must still stand at its value
    /// (within 1e-6; an angle of a coupled joint a whole number of turns off counts): a later
    /// drive that moved an earlier joint (two coupled joints asked for values the coupling does
    /// not allow) refuses. The steps, applied. A joint that cannot be driven or held refuses
    /// the whole call and changes nothing.
    pub fn drive_joints(
        &mut self,
        values: &[(JointId, f64, Option<f64>)],
    ) -> Result<SceneChange, KernelError> {
        self.drive_joints_held(values, true)
    }

    /// [`Scene::drive_joints`] without the check that each joint still stands at its value
    /// at the end: a frame between two sets of joint values (a scene transition), where
    /// values interpolated one by one need not agree with each other.
    pub fn drive_frame_joints(
        &mut self,
        values: &[(JointId, f64, Option<f64>)],
    ) -> Result<SceneChange, KernelError> {
        self.drive_joints_held(values, false)
    }

    fn drive_joints_held(
        &mut self,
        values: &[(JointId, f64, Option<f64>)],
        held: bool,
    ) -> Result<SceneChange, KernelError> {
        let wanted: BTreeMap<JointId, (f64, Option<f64>)> = values
            .iter()
            .filter(|(id, _, _)| {
                self.joint(*id).is_some_and(|j| j.kind.has_value())
                    && self.follower_of(*id).is_none()
            })
            .map(|(id, v, s)| (*id, (*v, *s)))
            .collect();
        let mut changes: Vec<SceneChange> = Vec::new();
        let mut result = Ok(());
        for (id, (value, slide)) in &wanted {
            let cylindrical = self.joints[id].kind == JointKind::Cylindrical;
            match self.drive_joint(*id, *value, slide.filter(|_| cylindrical)) {
                Ok(change) => changes.push(change),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        if held && result.is_ok() {
            result = self.check_joint_values(&wanted);
        }
        let done = SceneChange::join(changes);
        if let Err(e) = result {
            self.apply(&done.inverse());
            return Err(e);
        }
        Ok(done)
    }

    /// Fails, naming the first, when a joint of `wanted` (value, slide: radians or meters; no
    /// slide is not checked) does not stand at its value within [`HELD_TOLERANCE`]; an angle of
    /// a coupled joint compares less whole turns.
    fn check_joint_values(
        &self,
        wanted: &BTreeMap<JointId, (f64, Option<f64>)>,
    ) -> Result<(), KernelError> {
        for (id, (value, slide)) in wanted {
            let j = &self.joints[id];
            let coupled = self
                .couplings()
                .any(|(_, c)| c.driver == *id || c.driven == *id);
            let off = j.value - value;
            let off = if coupled && j.kind.is_angular() {
                wrap_angle(off)
            } else {
                off
            };
            let slid = match slide {
                Some(s) if j.kind == JointKind::Cylindrical => (j.slide - s).abs(),
                _ => 0.0,
            };
            if off.abs() > HELD_TOLERANCE || slid > HELD_TOLERANCE {
                return Err(refuse(format!(
                    "joint {:?} cannot stand at {} with the other values asked for: driving \
                     the others (through its couplings) moved it to {}",
                    j.name,
                    shown(j.kind, *value),
                    shown(j.kind, j.value)
                )));
            }
        }
        Ok(())
    }

    /// The parts connected to `driven` through joints (not through the ground) and couplings,
    /// and the cam followers' parts a cam brings in; the joints between them, and the parts
    /// that move (all but the part `a` of the driven joint when nothing is grounded).
    fn assembly_parts(&self, driven: &Joint) -> (Vec<(JointId, &Joint)>, Vec<InstanceId>) {
        let mut neighbours: BTreeMap<InstanceId, Vec<InstanceId>> = BTreeMap::new();
        let mut join = |x: InstanceId, y: InstanceId| {
            neighbours.entry(x).or_default().push(y);
            neighbours.entry(y).or_default().push(x);
        };
        for (_, j) in self.joints() {
            if let Some(a) = j.a {
                join(a, j.b);
            }
        }
        for (_, c) in self.couplings() {
            let (Some(x), Some(y)) = (self.joint(c.driver), self.joint(c.driven)) else {
                continue;
            };
            join(x.b, y.b);
        }
        // A cam brings its followers' parts in; a follower does not bring its cam (a cam the
        // drive does not reach is held).
        let mut rides: BTreeMap<InstanceId, Vec<InstanceId>> = BTreeMap::new();
        for (_, c) in self.cam_followers() {
            if let Some(j) = self.joint(c.follower) {
                rides.entry(c.cam).or_default().push(j.b);
            }
        }
        let mut members: BTreeSet<InstanceId> = BTreeSet::new();
        let mut queue: VecDeque<InstanceId> = driven.a.into_iter().chain([driven.b]).collect();
        while let Some(i) = queue.pop_front() {
            if members.insert(i) {
                queue.extend(neighbours.get(&i).into_iter().flatten().copied());
                queue.extend(rides.get(&i).into_iter().flatten().copied());
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
        (joints, parts)
    }

    /// The parts that move when any joint of `ids` is driven.
    pub fn joint_parts(&self, ids: impl IntoIterator<Item = JointId>) -> BTreeSet<InstanceId> {
        ids.into_iter()
            .filter_map(|id| self.joint(id))
            .flat_map(|j| self.assembly_parts(j).1)
            .collect()
    }

    /// The assembly of joint `id`: the parts connected to it through joints (not through the
    /// ground) and couplings, every joint and coupling between them, and which parts move.
    fn assembly(&self, id: JointId, driven: &Joint, trees: &mut CamTrees) -> System {
        let (joints, parts) = self.assembly_parts(driven);
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
        let link_index: BTreeMap<JointId, usize> =
            links.iter().enumerate().map(|(k, l)| (l.id, k)).collect();
        let bonds: Vec<Bond> = self
            .couplings()
            .filter_map(|(_, c)| {
                Some(Bond {
                    driver: *link_index.get(&c.driver)?,
                    driven: *link_index.get(&c.driven)?,
                    ratio: c.ratio,
                    offset: c.offset,
                })
            })
            .collect();
        // One tree per cam definition, however many followers ride on it.
        let riders: Vec<Rider> = self
            .cam_followers()
            .filter_map(|(w, c)| {
                let link = *link_index.get(&c.follower)?;
                let cam = self.root().instance(c.cam)?;
                let tree = trees
                    .entry(cam.definition)
                    .or_insert_with(|| Rc::new(self.cam_tree(cam.definition)))
                    .clone();
                Some(Rider {
                    id: w,
                    link,
                    cam: body(Some(c.cam)),
                    cam_id: c.cam,
                    tree,
                    preload: c.preload,
                    roller_radius: c.roller_radius,
                    frozen: None,
                })
            })
            .collect();
        let near = links
            .iter()
            .map(|l| {
                let j = &self.joints[&l.id];
                (j.value, j.slide)
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
            bonds,
            riders,
            riding: false,
            near,
            parts,
            poses,
            pivots,
        }
    }
}

/// How many steps of at most 10 degrees turn through `angle` (not negative).
fn turn_steps(angle: f64) -> usize {
    // A hair under a whole number of steps is that number (720 degrees is 72 steps).
    (angle / MAX_ANGLE_STEP - 1e-9).ceil().max(1.0) as usize
}

/// How many steps a drive of `joint` to `value` and `slide` takes: turns of at most 10
/// degrees, slides in tenths.
fn drive_steps(joint: &Joint, value: f64, slide: f64) -> usize {
    let turn = |from: f64, to: f64| turn_steps((to - from).abs());
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
    fn cholesky_skipping_zeros_matches_the_full_sums_exactly() {
        // Like an assembly's normal equations: parts in a chain and a few on a hub, 6 unknowns
        // each, every joint adding a dense block between its two parts.
        let (parts, n) = (20, 120);
        let mut a = vec![0.0; n * n];
        let mut seed = 1u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let joints: Vec<(usize, usize)> = (1..parts)
            .map(|i| (i - 1, i))
            .chain([(3, 11), (3, 15), (7, 19)])
            .collect();
        for (p, q) in joints {
            let cols: Vec<usize> = (0..6)
                .map(|k| 6 * p + k)
                .chain((0..6).map(|k| 6 * q + k))
                .collect();
            let rows: Vec<Vec<f64>> = (0..3)
                .map(|_| cols.iter().map(|_| next()).collect())
                .collect();
            for (x, &i) in cols.iter().enumerate() {
                for (y, &j) in cols.iter().enumerate() {
                    a[i * n + j] += rows.iter().map(|r| r[x] * r[y]).sum::<f64>();
                }
            }
        }
        for i in 0..n {
            a[i * n + i] += 1e-6;
        }
        let b: Vec<f64> = (0..n).map(|_| next()).collect();
        // The full sums, as the factor was computed before.
        let (mut full, mut x) = (a.clone(), b.clone());
        for j in 0..n {
            let mut d = full[j * n + j];
            for k in 0..j {
                d -= full[j * n + k] * full[j * n + k];
            }
            let d = d.sqrt();
            full[j * n + j] = d;
            for i in j + 1..n {
                let mut s = full[i * n + j];
                for k in 0..j {
                    s -= full[i * n + k] * full[j * n + k];
                }
                full[i * n + j] = s / d;
            }
        }
        for i in 0..n {
            let mut s = x[i];
            for k in 0..i {
                s -= full[i * n + k] * x[k];
            }
            x[i] = s / full[i * n + i];
        }
        for i in (0..n).rev() {
            let mut s = x[i];
            for k in i + 1..n {
                s -= full[k * n + i] * x[k];
            }
            x[i] = s / full[i * n + i];
        }
        let (mut sparse, mut y) = (a, b);
        assert!(cholesky_solve(&mut sparse, n, &mut y));
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&y), bits(&x));
        let lower = |m: &[f64]| {
            (0..n)
                .flat_map(|i| (0..=i).map(move |k| m[i * n + k].to_bits()))
                .collect::<Vec<_>>()
        };
        assert_eq!(lower(&sparse), lower(&full));
    }

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
