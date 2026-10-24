//! Joints: connections between parts that say how they may move relative to each other.

use glam::{DAffine3, DVec3};
use kernel::{
    Coupling, CouplingId, EntityId, InstanceId, Joint, JointFrame, JointId, JointKind, KernelError,
    SceneChange, perpendicular,
};

use super::{Document, check_coordinates, unit_vector};
use crate::command::{CommandError, ErrorKind};
use crate::query::{CouplingInfo, JointInfo};
use crate::rectangle::rectangle_axes;

/// The fields of `add_joint`.
pub(super) struct NewJoint<'a> {
    pub kind: JointKind,
    pub a: Option<EntityId>,
    pub b: EntityId,
    pub point: [f64; 3],
    pub axis: [f64; 3],
    pub reference: Option<[f64; 3]>,
    pub name: Option<&'a str>,
    pub limits: Option<[f64; 2]>,
}

/// The largest size of a coupling's ratio, in the joints' units (degrees or meters).
pub const MAX_COUPLING_RATIO: f64 = 1000.0;

/// How many radians or meters one unit of a joint of `kind` is in the API (a degree or a
/// meter).
fn unit(kind: JointKind) -> f64 {
    if kind.is_angular() {
        1f64.to_radians()
    } else {
        1.0
    }
}

fn invalid(why: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, why)
}

impl Document {
    /// Adds a joint holding the parts as they are: the frames are the given world frame seen
    /// from each part's current transform, and the value is 0.
    pub(super) fn add_joint_changes(
        &mut self,
        new: NewJoint<'_>,
    ) -> Result<SceneChange, CommandError> {
        if !self.context.is_empty() {
            return Err(invalid(
                "joints join instances of the root: close the open instances first",
            ));
        }
        check_coordinates(&[new.point])?;
        let axis = unit_vector(DVec3::from(new.axis))
            .ok_or_else(|| invalid("axis must be a finite, non-zero vector"))?;
        let reference = match new.reference {
            None => rectangle_axes(axis).0,
            Some(r) => {
                let r = DVec3::from(r);
                if !r.is_finite() {
                    return Err(invalid("reference must be a finite vector"));
                }
                perpendicular(r, axis)
                    .ok_or_else(|| invalid("reference must not be zero or along the axis"))?
            }
        };
        let world = JointFrame {
            origin: DVec3::from(new.point),
            axis,
            reference,
        };
        let part = |id: EntityId, which: &str| -> Result<InstanceId, CommandError> {
            match id {
                EntityId::Instance(i) if self.scene.root().instance(i).is_some() => Ok(i),
                _ => Err(invalid(format!(
                    "part {which} ({id}) is not an instance of the root: joints join only those"
                ))),
            }
        };
        let a = new.a.map(|a| part(a, "a")).transpose()?;
        let b = part(new.b, "b")?;
        if a == Some(b) {
            return Err(invalid("a joint needs two different parts"));
        }
        let local = |part: Option<InstanceId>| -> Result<JointFrame, CommandError> {
            let transform = part.map_or(DAffine3::IDENTITY, |i| {
                self.scene.root().instance(i).expect("checked").transform
            });
            JointFrame::local(&world, &transform)
                .ok_or_else(|| invalid("the part's transform cannot hold a joint frame"))
        };
        let limits = match new.limits {
            None => None,
            Some(l) => Some(limit_values(new.kind, l)?),
        };
        if let Some((lo, hi)) = limits
            && (lo > 0.0 || hi < 0.0)
        {
            return Err(invalid("limits must contain 0, the value a new joint has"));
        }
        let joint = Joint {
            name: new
                .name
                .map_or_else(|| self.scene.free_joint_name(), str::to_string),
            kind: new.kind,
            a,
            b,
            frame_a: local(a)?,
            frame_b: local(Some(b))?,
            value: 0.0,
            slide: 0.0,
            limits,
        };
        let (_, change) = self
            .scene
            .add_joint(joint)
            .map_err(|e| joint_error(e, None))?;
        Ok(change)
    }

    /// Renames a joint and/or sets or clears its limits.
    pub(super) fn change_joint_changes(
        &mut self,
        id: JointId,
        name: Option<&str>,
        limits: Option<Option<[f64; 2]>>,
    ) -> Result<SceneChange, CommandError> {
        if name.is_none() && limits.is_none() {
            return Err(invalid("change_joint needs a name or limits"));
        }
        let mut to = self.joint_ref(id)?.clone();
        if let Some(name) = name {
            to.name = name.to_string();
        }
        match limits {
            None => {}
            Some(None) => to.limits = None,
            Some(Some(l)) => to.limits = Some(limit_values(to.kind, l)?),
        }
        self.scene
            .change_joint(id, to)
            .map_err(|e| joint_error(e, Some(id)))
    }

    /// Removes a joint.
    pub(super) fn delete_joint_changes(
        &mut self,
        id: JointId,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .delete_joint(id)
            .map_err(|e| joint_error(e, Some(id)))
    }

    /// Drives a joint to `value` (degrees or meters) and `slide` (meters), moving its
    /// assembly.
    pub(super) fn drive_changes(
        &mut self,
        id: JointId,
        value: f64,
        slide: Option<f64>,
    ) -> Result<SceneChange, CommandError> {
        let kind = self.joint_ref(id)?.kind;
        if !value.is_finite() || slide.is_some_and(|s| !s.is_finite()) {
            return Err(invalid("value and slide must be finite numbers"));
        }
        let value = if kind.is_angular() {
            value.to_radians()
        } else {
            value
        };
        self.scene
            .drive_joint(id, value, slide)
            .map_err(|e| joint_error(e, Some(id)))
    }

    /// Couples `driven` to `driver`: `ratio` and `offset` in the joints' API units (degrees or
    /// meters); no offset takes the one that holds now.
    pub(super) fn add_coupling_changes(
        &mut self,
        driver: JointId,
        driven: JointId,
        ratio: f64,
        offset: Option<f64>,
    ) -> Result<SceneChange, CommandError> {
        let (driver_joint, driven_joint) = (self.joint_ref(driver)?, self.joint_ref(driven)?);
        if !ratio.is_finite() || ratio == 0.0 || ratio.abs() > MAX_COUPLING_RATIO {
            return Err(invalid(format!(
                "ratio must be a finite number other than 0, at most {MAX_COUPLING_RATIO} in size"
            )));
        }
        if offset.is_some_and(|o| !o.is_finite()) {
            return Err(invalid("offset must be a finite number"));
        }
        let (from, to) = (unit(driver_joint.kind), unit(driven_joint.kind));
        let ratio = ratio * (to / from);
        let offset = match offset {
            Some(o) => o * to,
            None => driven_joint.value - ratio * driver_joint.value,
        };
        let coupling = Coupling {
            driver,
            driven,
            ratio,
            offset,
        };
        let (_, change) = self
            .scene
            .add_coupling(coupling)
            .map_err(|e| joint_error(e, None))?;
        Ok(change)
    }

    /// Removes a coupling.
    pub(super) fn delete_coupling_changes(
        &mut self,
        id: CouplingId,
    ) -> Result<SceneChange, CommandError> {
        self.scene.delete_coupling(id).map_err(|e| match e {
            KernelError::CouplingNotFound => {
                CommandError::new(ErrorKind::NotFound, format!("no coupling {id}"))
            }
            e => joint_error(e, None),
        })
    }

    /// Every coupling, in the order they were made, in the joints' API units.
    pub(super) fn coupling_infos(&self) -> Vec<CouplingInfo> {
        self.scene
            .couplings()
            .map(|(id, c)| {
                let kind =
                    |j: JointId| self.scene.joint(j).map_or(JointKind::Prismatic, |j| j.kind);
                let (from, to) = (unit(kind(c.driver)), unit(kind(c.driven)));
                CouplingInfo {
                    id,
                    driver: c.driver,
                    driven: c.driven,
                    ratio: c.ratio * (from / to),
                    offset: c.offset / to,
                }
            })
            .collect()
    }

    /// Every joint, in the order they were made.
    pub(super) fn joint_infos(&self) -> Vec<JointInfo> {
        self.scene
            .joints()
            .map(|(id, j)| self.joint_info(id, j))
            .collect()
    }

    /// One joint, or `not_found`.
    pub(super) fn joint_info_of(&self, id: JointId) -> Result<JointInfo, CommandError> {
        Ok(self.joint_info(id, self.joint_ref(id)?))
    }

    fn joint_ref(&self, id: JointId) -> Result<&Joint, CommandError> {
        self.scene
            .joint(id)
            .ok_or_else(|| joint_error(KernelError::JointNotFound, Some(id)))
    }

    fn joint_info(&self, id: JointId, j: &Joint) -> JointInfo {
        let to_world =
            j.a.and_then(|a| self.scene.root().instance(a))
                .map_or(DAffine3::IDENTITY, |i| i.transform);
        let frame = j.frame_a.world(&to_world);
        let (value, limits) = if j.kind.is_angular() {
            (
                j.value.to_degrees(),
                j.limits.map(|(lo, hi)| [lo.to_degrees(), hi.to_degrees()]),
            )
        } else {
            (j.value, j.limits.map(|(lo, hi)| [lo, hi]))
        };
        JointInfo {
            id,
            name: j.name.clone(),
            kind: j.kind,
            a: j.a.map(EntityId::Instance),
            b: EntityId::Instance(j.b),
            value,
            slide: (j.kind == JointKind::Cylindrical).then_some(j.slide),
            limits,
            point: frame.origin.to_array(),
            axis: frame.axis.to_array(),
        }
    }
}

/// `limits` in the API's units (degrees for angles, meters for distances) as stored (radians
/// or meters).
fn limit_values(kind: JointKind, [lo, hi]: [f64; 2]) -> Result<(f64, f64), CommandError> {
    if !kind.has_value() {
        return Err(invalid("a fixed or ball joint has no value to limit"));
    }
    if !(lo.is_finite() && hi.is_finite() && lo < hi) {
        return Err(invalid(
            "limits must be [min, max]: two finite numbers, min below max",
        ));
    }
    Ok(if kind.is_angular() {
        (lo.to_radians(), hi.to_radians())
    } else {
        (lo, hi)
    })
}

/// A kernel refusal as a command error; a missing joint is `not_found`.
fn joint_error(e: KernelError, id: Option<JointId>) -> CommandError {
    match (e, id) {
        (KernelError::JointNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no joint {id}"))
        }
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}
