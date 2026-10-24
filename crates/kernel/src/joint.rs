//! Joints: connections between two root instances (or one instance and the ground) that say
//! how the parts may move relative to each other. A joint holds a frame in each part's own
//! coordinates, so it stays right however the parts are moved, copied, scaled or mirrored.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{InstanceId, JointId};
use crate::scene::{Scene, SceneChange, Step};

/// The longest joint name, in characters.
pub const MAX_JOINT_NAME: usize = 100;

/// The name new joints take, followed by the lowest number no joint has ("Joint 1").
pub const JOINT_NAME_BASE: &str = "Joint";

/// What a joint lets the two parts do (frames A and B met in the world):
/// - `revolute`: origins on one line along the axes, axes parallel; free rotation about it.
/// - `prismatic`: as revolute with the references equal; free sliding along the axis.
/// - `cylindrical`: as revolute, with the sliding free too.
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

    /// Removes a joint; undo puts it back.
    pub fn delete_joint(&mut self, id: JointId) -> Result<SceneChange, KernelError> {
        let joint = self.joint(id).ok_or(KernelError::JointNotFound)?.clone();
        let step = Step::RemoveJoint(id, joint);
        self.apply_step(&step);
        Ok(step.into())
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
    /// the root (erased, grouped or exploded), applied.
    pub(crate) fn drop_stale_joints(&mut self) -> SceneChange {
        let stale: Vec<(JointId, Joint)> = self
            .joints()
            .filter(|(_, j)| {
                let gone = |i: &InstanceId| self.root().instance(*i).is_none();
                gone(&j.b) || j.a.as_ref().is_some_and(gone)
            })
            .map(|(id, j)| (id, j.clone()))
            .collect();
        let mut steps = Vec::new();
        for (id, joint) in stale {
            let step = Step::RemoveJoint(id, joint);
            self.apply_step(&step);
            steps.push(step);
        }
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
