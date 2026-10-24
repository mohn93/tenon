//! Joints: connections between two root instances (or one instance and the ground) that say
//! how the parts may move relative to each other. A joint holds a frame in each part's own
//! coordinates, so it stays right however the parts are moved, copied, scaled or mirrored.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{CouplingId, InstanceId, JointId};
use crate::scene::{Scene, SceneChange, Step};

/// How far a measured value may be from the stored one and still count as unchanged.
pub(crate) const VALUE_TOLERANCE: f64 = 1e-12;

/// The longest joint name, in characters.
pub const MAX_JOINT_NAME: usize = 100;

/// The name new joints take, followed by the lowest number no joint has ("Joint 1").
pub const JOINT_NAME_BASE: &str = "Joint";

/// What a joint lets the two parts do (frames A and B met in the world):
/// - `revolute`: origins equal, axes parallel; free rotation about the axis.
/// - `prismatic`: origins on one line along the axes, axes parallel and the references
///   equal; free sliding along the axis.
/// - `cylindrical`: as revolute, with sliding along the axis free too.
/// - `fixed`: the frames are equal.
/// - `ball`: the origins are equal; free rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    Revolute,
    Prismatic,
    Cylindrical,
    Fixed,
    Ball,
}

impl JointKind {
    /// Whether the joint's value is an angle (radians), not a distance.
    pub fn is_angular(self) -> bool {
        matches!(self, JointKind::Revolute | JointKind::Cylindrical)
    }

    /// Whether the joint's value can be limited (it moves along one degree of freedom).
    pub fn has_value(self) -> bool {
        !matches!(self, JointKind::Fixed | JointKind::Ball)
    }
}

/// A point, an axis and a reference direction perpendicular to it, in a part's own
/// coordinates (the root's, for the ground).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointFrame {
    pub origin: DVec3,
    /// A unit vector.
    pub axis: DVec3,
    /// A unit vector perpendicular to `axis`.
    pub reference: DVec3,
}

impl JointFrame {
    /// The frame in the coordinates `to_world` maps into. Directions are mapped by the linear
    /// part and made unit again, so the frame meets the same line however the part is scaled
    /// or mirrored.
    pub fn world(&self, to_world: &DAffine3) -> JointFrame {
        let axis = (to_world.matrix3 * self.axis).normalize();
        let reference = (to_world.matrix3 * self.reference).normalize();
        JointFrame {
            origin: to_world.transform_point3(self.origin),
            axis,
            reference: perpendicular(reference, axis).unwrap_or(reference),
        }
    }

    /// The frame `world` in the coordinates that `to_world` maps out of: the inverse of
    /// [`JointFrame::world`], with the reference made perpendicular to the axis again.
    /// None when the transform cannot be inverted or the result has no reference direction.
    pub fn local(world: &JointFrame, to_world: &DAffine3) -> Option<JointFrame> {
        if !to_world.is_finite() || to_world.matrix3.determinant().abs() < crate::MIN_DETERMINANT {
            return None;
        }
        let back = to_world.inverse();
        let axis = (back.matrix3 * world.axis).try_normalize()?;
        let reference = (back.matrix3 * world.reference).try_normalize()?;
        Some(JointFrame {
            origin: back.transform_point3(world.origin),
            axis,
            reference: perpendicular(reference, axis)?,
        })
    }

    fn check(&self, which: &str) -> Result<(), String> {
        let finite = self.origin.is_finite() && self.axis.is_finite() && self.reference.is_finite();
        if !finite {
            return Err(format!("frame {which} has a number that is not finite"));
        }
        if (self.axis.length() - 1.0).abs() > 1e-6 || (self.reference.length() - 1.0).abs() > 1e-6 {
            return Err(format!(
                "frame {which}'s axis and reference must be unit vectors"
            ));
        }
        if self.axis.dot(self.reference).abs() > 1e-6 {
            return Err(format!(
                "frame {which}'s reference must be perpendicular to its axis"
            ));
        }
        Ok(())
    }
}

/// The signed angle from `a`'s reference to `b`'s about `a`'s axis, in [-pi, pi].
pub(crate) fn frame_angle(a: &JointFrame, b: &JointFrame) -> f64 {
    a.axis
        .dot(a.reference.cross(b.reference))
        .atan2(a.reference.dot(b.reference))
}

/// The signed distance from `a`'s origin to `b`'s along `a`'s axis.
pub(crate) fn frame_distance(a: &JointFrame, b: &JointFrame) -> f64 {
    (b.origin - a.origin).dot(a.axis)
}

/// `x` less the whole turns that bring it nearest 0, in [-pi, pi].
pub(crate) fn wrap_angle(x: f64) -> f64 {
    x - std::f64::consts::TAU * (x / std::f64::consts::TAU).round()
}

/// The value and slide of a joint of `kind` whose frames stand at `a` and `b` in the world,
/// an angle taken as the turn nearest `value` (so it counts on past a full turn). A fixed or
/// ball joint keeps `value` and `slide`.
pub fn measure_joint(
    kind: JointKind,
    a: &JointFrame,
    b: &JointFrame,
    value: f64,
    slide: f64,
) -> (f64, f64) {
    let angle = || value + wrap_angle(frame_angle(a, b) - value);
    match kind {
        JointKind::Revolute => (angle(), slide),
        JointKind::Prismatic => (frame_distance(a, b), slide),
        JointKind::Cylindrical => (angle(), frame_distance(a, b)),
        JointKind::Fixed | JointKind::Ball => (value, slide),
    }
}

/// `v` with its part along `axis` removed, made unit; None when nothing is left (`v` is
/// parallel to `axis`, or zero).
pub fn perpendicular(v: DVec3, axis: DVec3) -> Option<DVec3> {
    let p = v - axis * v.dot(axis);
    (p.length() > 1e-9 * v.length().max(1e-300)).then(|| p.normalize())
}

/// A joint between root instance `b` and root instance `a` (None: the ground, the fixed world).
#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    pub kind: JointKind,
    pub a: Option<InstanceId>,
    pub b: InstanceId,
    /// In `a`'s own coordinates (the root's, for the ground).
    pub frame_a: JointFrame,
    /// In `b`'s own coordinates.
    pub frame_b: JointFrame,
    /// The angle (radians) or distance (meters) the joint stands at; 0 when created.
    pub value: f64,
    /// The sliding distance of a cylindrical joint (meters); 0 for every other kind.
    pub slide: f64,
    /// The least and greatest value (radians or meters); the value must lie within.
    pub limits: Option<(f64, f64)>,
}

/// A coupling between two joints' values, as gears, chains, belts and racks make: the solver
/// holds `driven value = ratio * driver value + offset`, values in the joints' own units
/// (radians for an angle, meters for a distance; a cylindrical joint's angle). `ratio` is
/// finite and not zero; `offset` is finite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coupling {
    pub driver: JointId,
    pub driven: JointId,
    pub ratio: f64,
    pub offset: f64,
}

/// The least and greatest size of a coupling's ratio (in the joints' own units) between two
/// joints that both turn or both slide: a 1000:1 gearbox either way.
pub const COUPLING_RATIO_LIKE: (f64, f64) = (1e-3, 1e3);

/// The least and greatest size of a coupling's ratio (meters per radian, or radians per meter)
/// between a joint that turns and one that slides: a rack on a pinion of a micrometer's to a
/// kilometer's radius.
pub const COUPLING_RATIO_MIXED: (f64, f64) = (1e-6, 1e6);

/// The least and greatest size of a coupling's ratio from a `driver` joint to a `driven` one.
pub fn coupling_ratio_range(driver: JointKind, driven: JointKind) -> (f64, f64) {
    if driver.is_angular() == driven.is_angular() {
        COUPLING_RATIO_LIKE
    } else {
        COUPLING_RATIO_MIXED
    }
}

/// Fails, saying why, when a joint name is blank (after trimming) or too long.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a joint needs a name".into());
    }
    if name.chars().count() > MAX_JOINT_NAME {
        return Err(format!(
            "a joint name may have at most {MAX_JOINT_NAME} characters"
        ));
    }
    Ok(())
}

impl Scene {
    /// The frames of `joint` in the world, through its parts' current transforms (a part that
    /// is not an instance of the root, which a stored joint never names, stands at identity).
    pub fn joint_world_frames(&self, joint: &Joint) -> (JointFrame, JointFrame) {
        let to_world = |i: Option<InstanceId>| {
            i.and_then(|i| self.root().instance(i))
                .map_or(DAffine3::IDENTITY, |i| i.transform)
        };
        (
            joint.frame_a.world(&to_world(joint.a)),
            joint.frame_b.world(&to_world(Some(joint.b))),
        )
    }

    /// Sets every joint's stored value (and slide) from the parts' current pose, as
    /// [`measure_joint`] reads it, each angle the turn nearest the stored one. The steps,
    /// applied. A joint whose new value falls outside its limits loses them (the value must
    /// lie within, and the pose is what it is).
    pub fn update_joint_values(&mut self) -> SceneChange {
        let changed: Vec<(JointId, Joint, Joint)> = self
            .joints()
            .filter_map(|(id, j)| {
                let (a, b) = self.joint_world_frames(j);
                let (value, slide) = measure_joint(j.kind, &a, &b, j.value, j.slide);
                let same = (value - j.value).abs() < VALUE_TOLERANCE
                    && (slide - j.slide).abs() < VALUE_TOLERANCE;
                if same {
                    return None;
                }
                let mut to = j.clone();
                to.value = value;
                to.slide = slide;
                if to.limits.is_some_and(|(lo, hi)| value < lo || value > hi) {
                    to.limits = None;
                }
                Some((id, j.clone(), to))
            })
            .collect();
        let mut steps = Vec::new();
        for (id, from, to) in changed {
            let step = Step::ChangeJoint(id, from, to);
            self.apply_step(&step);
            steps.push(step);
        }
        SceneChange { steps }
    }

    pub fn joint(&self, id: JointId) -> Option<&Joint> {
        self.joints.get(&id)
    }

    /// Every joint, in id (creation) order.
    pub fn joints(&self) -> impl Iterator<Item = (JointId, &Joint)> {
        self.joints.iter().map(|(k, v)| (*k, v))
    }

    /// Adds `joint` after checking it (see [`Scene::check_joint`]); its name is trimmed.
    pub fn add_joint(&mut self, joint: Joint) -> Result<(JointId, SceneChange), KernelError> {
        let joint = self.checked_joint(joint, None)?;
        let id = self.alloc_joint();
        let step = Step::AddJoint(id, joint);
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Replaces a joint with `to`, checked as `add_joint` does. One step; nothing to change
    /// changes nothing.
    pub fn change_joint(&mut self, id: JointId, to: Joint) -> Result<SceneChange, KernelError> {
        let from = self.joint(id).ok_or(KernelError::JointNotFound)?.clone();
        let to = self.checked_joint(to, Some(id))?;
        if to == from {
            return Ok(SceneChange::default());
        }
        let step = Step::ChangeJoint(id, from, to);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes a joint, its couplings and its cam follower; undo puts them back.
    pub fn delete_joint(&mut self, id: JointId) -> Result<SceneChange, KernelError> {
        let joint = self.joint(id).ok_or(KernelError::JointNotFound)?.clone();
        let ids = BTreeSet::from([id]);
        let mut steps = self.drop_couplings_of(&ids).steps;
        steps.extend(self.drop_cam_followers_of(&ids).steps);
        let step = Step::RemoveJoint(id, joint);
        self.apply_step(&step);
        steps.push(step);
        steps.extend(self.drop_stale_page_joints().steps);
        Ok(SceneChange { steps })
    }

    pub fn coupling(&self, id: CouplingId) -> Option<&Coupling> {
        self.couplings.get(&id)
    }

    /// Every coupling, in id (creation) order.
    pub fn couplings(&self) -> impl Iterator<Item = (CouplingId, &Coupling)> {
        self.couplings.iter().map(|(k, v)| (*k, v))
    }

    /// Adds `coupling` after checking it (see [`Scene::coupling_problem`]). Adding one never
    /// moves anything: the next drive meets it.
    pub fn add_coupling(
        &mut self,
        coupling: Coupling,
    ) -> Result<(CouplingId, SceneChange), KernelError> {
        self.coupling_problem(&coupling, None)
            .map_err(KernelError::Joint)?;
        let id = self.alloc_coupling();
        let step = Step::AddCoupling(id, coupling);
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Removes a coupling; undo puts it back.
    pub fn delete_coupling(&mut self, id: CouplingId) -> Result<SceneChange, KernelError> {
        let coupling = *self.coupling(id).ok_or(KernelError::CouplingNotFound)?;
        let step = Step::RemoveCoupling(id, coupling);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Why `coupling` is not valid, if it is not: a ratio that is not finite or is zero, an
    /// offset that is not finite, a joint that does not exist or has no value (fixed, ball), a
    /// ratio outside [`coupling_ratio_range`],
    /// the same joint twice, a joint that follows a cam, or two joints another coupling (but
    /// `except`) already couples.
    pub(crate) fn coupling_problem(
        &self,
        coupling: &Coupling,
        except: Option<CouplingId>,
    ) -> Result<(), String> {
        if !coupling.ratio.is_finite() || coupling.ratio == 0.0 {
            return Err("a coupling's ratio must be a finite number other than 0".into());
        }
        if !coupling.offset.is_finite() {
            return Err("a coupling's offset must be a finite number".into());
        }
        if coupling.driver == coupling.driven {
            return Err("a coupling needs two different joints".into());
        }
        for (label, id) in [("driver", coupling.driver), ("driven", coupling.driven)] {
            let joint = self
                .joint(id)
                .ok_or_else(|| format!("the {label} joint {id} does not exist"))?;
            if !joint.kind.has_value() {
                return Err(format!(
                    "the {label} joint {id} is fixed or ball: it has no value to couple"
                ));
            }
        }
        let kind = |id: JointId| self.joint(id).map(|j| j.kind);
        if let (Some(from), Some(to)) = (kind(coupling.driver), kind(coupling.driven)) {
            let (lo, hi) = coupling_ratio_range(from, to);
            if !(lo..=hi).contains(&coupling.ratio.abs()) {
                return Err(format!(
                    "a coupling's ratio must be from {lo} to {hi} in size (in the joints' own \
                     units: radians or meters)"
                ));
            }
        }
        for id in [coupling.driver, coupling.driven] {
            if let Some((w, _)) = self.follower_of(id) {
                return Err(format!(
                    "joint {id} follows a cam ({w}): its value comes from the cam"
                ));
            }
        }
        let pair = |c: &Coupling| {
            (c.driver == coupling.driver && c.driven == coupling.driven)
                || (c.driver == coupling.driven && c.driven == coupling.driver)
        };
        if let Some((id, _)) = self
            .couplings()
            .find(|(id, c)| Some(*id) != except && pair(c))
        {
            return Err(format!(
                "joints {} and {} are already coupled by {id}",
                coupling.driver, coupling.driven
            ));
        }
        Ok(())
    }

    /// The steps that remove every coupling of a joint in `joints`, applied.
    fn drop_couplings_of(&mut self, joints: &BTreeSet<JointId>) -> SceneChange {
        let gone: Vec<(CouplingId, Coupling)> = self
            .couplings()
            .filter(|(_, c)| joints.contains(&c.driver) || joints.contains(&c.driven))
            .map(|(id, c)| (id, *c))
            .collect();
        let mut steps = Vec::new();
        for (id, c) in gone {
            let step = Step::RemoveCoupling(id, c);
            self.apply_step(&step);
            steps.push(step);
        }
        SceneChange { steps }
    }

    /// "Joint" followed by the lowest number from 1 no joint's name has ("Joint 1").
    pub fn free_joint_name(&self) -> String {
        let taken: BTreeSet<&str> = self.joints().map(|(_, j)| j.name.as_str()).collect();
        (1..)
            .map(|n| format!("{JOINT_NAME_BASE} {n}"))
            .find(|n| !taken.contains(n.as_str()))
            .expect("a free name")
    }

    /// The steps that remove every joint naming an instance that is no longer an instance of
    /// the root (erased, grouped or exploded), those joints' couplings and every cam follower
    /// of such a joint or of a cam that is gone, applied.
    pub(crate) fn drop_stale_joints(&mut self) -> SceneChange {
        let stale: Vec<(JointId, Joint)> = self
            .joints()
            .filter(|(_, j)| {
                let gone = |i: &InstanceId| self.root().instance(*i).is_none();
                gone(&j.b) || j.a.as_ref().is_some_and(gone)
            })
            .map(|(id, j)| (id, j.clone()))
            .collect();
        let ids: BTreeSet<JointId> = stale.iter().map(|(id, _)| *id).collect();
        let mut steps = self.drop_couplings_of(&ids).steps;
        steps.extend(self.drop_cam_followers_of(&ids).steps);
        for (id, joint) in stale {
            let step = Step::RemoveJoint(id, joint);
            self.apply_step(&step);
            steps.push(step);
        }
        steps.extend(self.drop_stale_page_joints().steps);
        SceneChange { steps }
    }

    /// `joint` with its name trimmed, after checking it (see [`Scene::joint_problem`]) and
    /// that no other joint has its name.
    fn checked_joint(
        &self,
        mut joint: Joint,
        except: Option<JointId>,
    ) -> Result<Joint, KernelError> {
        self.joint_problem(&joint).map_err(KernelError::Joint)?;
        joint.name = joint.name.trim().to_string();
        let taken = self
            .joints()
            .any(|(id, j)| Some(id) != except && j.name == joint.name);
        if taken {
            return Err(KernelError::Joint(format!(
                "a joint named {:?} already exists",
                joint.name
            )));
        }
        Ok(joint)
    }

    /// Why `joint` is not valid, if it is not: a blank or long name, a number that is not
    /// finite, a part that is not an instance of the root, the same part twice, a frame that is
    /// not a unit axis with a perpendicular unit reference, a slide on a joint that is not
    /// cylindrical, limits on a joint with no value to limit, limits that are not increasing
    /// or that leave the value out.
    pub(crate) fn joint_problem(&self, joint: &Joint) -> Result<(), String> {
        check_name(&joint.name)?;
        for (label, part) in [("a", joint.a), ("b", Some(joint.b))] {
            if let Some(i) = part
                && self.root().instance(i).is_none()
            {
                return Err(format!(
                    "part {label} (i{}) is not an instance of the root: joints join only those",
                    i.0
                ));
            }
        }
        if joint.a == Some(joint.b) {
            return Err("a joint needs two different parts".into());
        }
        joint.frame_a.check("a")?;
        joint.frame_b.check("b")?;
        if !joint.value.is_finite() || !joint.slide.is_finite() {
            return Err("a joint's value must be a finite number".into());
        }
        if joint.slide != 0.0 && joint.kind != JointKind::Cylindrical {
            return Err("only a cylindrical joint slides".into());
        }
        if let Some((lo, hi)) = joint.limits {
            if !joint.kind.has_value() {
                return Err("a fixed or ball joint has no value to limit".into());
            }
            if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                return Err(
                    "limits must be two finite numbers, the least below the greatest".into(),
                );
            }
            if joint.value < lo || joint.value > hi {
                return Err("the limits must contain the joint's current value".into());
            }
        }
        Ok(())
    }
}
