//! The model's axes: the frame the drawing tools and standard views follow.

use glam::{DAffine3, DMat3, DVec3};

use crate::edit::KernelError;
use crate::ids::DefinitionId;
use crate::scene::{MeshId, Scene, SceneChange, Step};
use crate::texture::TexturePlacement;

/// An origin and three perpendicular unit axes, right-handed (`z = x cross y`), in the
/// coordinates of the root. The world axes are the default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axes {
    pub origin: DVec3,
    pub x: DVec3,
    pub y: DVec3,
    pub z: DVec3,
}

impl Axes {
    pub const WORLD: Axes = Axes {
        origin: DVec3::ZERO,
        x: DVec3::X,
        y: DVec3::Y,
        z: DVec3::Z,
    };

    /// The axes at `origin` along `x_axis`, with `y_axis` made perpendicular to it
    /// (Gram-Schmidt); both are normalized. Zero, parallel or non-finite vectors are refused.
    pub fn new(origin: DVec3, x_axis: DVec3, y_axis: DVec3) -> Result<Axes, KernelError> {
        if !(origin.is_finite() && x_axis.is_finite() && y_axis.is_finite()) {
            return Err(KernelError::NotFinite);
        }
        let x = x_axis.try_normalize().ok_or(KernelError::BadAxes(
            "x_axis must be a non-zero vector of a size that can be normalized",
        ))?;
        let y_unit = y_axis.try_normalize().ok_or(KernelError::BadAxes(
            "y_axis must be a non-zero vector of a size that can be normalized",
        ))?;
        let across = y_unit - x * y_unit.dot(x);
        if across.length() < 1e-9 {
            return Err(KernelError::BadAxes("x_axis and y_axis are parallel"));
        }
        let y = across.normalize();
        Ok(Axes {
            origin,
            x,
            y,
            z: x.cross(y),
        })
    }

    /// Axes stored as data: kept as they are when finite, unit, perpendicular and
    /// right-handed (within 1e-9).
    pub fn from_parts(origin: DVec3, x: DVec3, y: DVec3, z: DVec3) -> Result<Axes, KernelError> {
        let axes = Axes { origin, x, y, z };
        let m = DMat3::from_cols(x, y, z);
        let orthonormal = (m.transpose() * m).abs_diff_eq(DMat3::IDENTITY, 1e-9)
            && (m.determinant() - 1.0).abs() < 1e-9;
        if !(origin.is_finite() && m.is_finite() && orthonormal) {
            return Err(KernelError::BadAxes(
                "axes are not a finite, right-handed, orthonormal frame",
            ));
        }
        Ok(axes)
    }

    pub fn is_world(&self) -> bool {
        *self == Axes::WORLD
    }

    /// The map from the axes' own coordinates to the root's.
    pub fn frame(&self) -> DAffine3 {
        DAffine3 {
            matrix3: DMat3::from_cols(self.x, self.y, self.z),
            translation: self.origin,
        }
    }
}

impl Default for Axes {
    fn default() -> Self {
        Axes::WORLD
    }
}

impl Scene {
    /// The model axes.
    pub fn axes(&self) -> Axes {
        self.axes
    }

    /// Moves the model axes. Nothing to change changes nothing.
    pub fn set_axes(&mut self, axes: Axes) -> SceneChange {
        if axes == self.axes {
            return SceneChange::default();
        }
        let step = Step::SetAxes(self.axes, axes);
        self.apply_step(&step);
        step.into()
    }

    /// Makes `frame` (a frame in the definition's coordinates) the definition's origin and
    /// axes: its geometry, guides, annotations, section planes and nested instances move by
    /// `frame^-1`, and every instance of it, in every mesh, is placed by `transform * frame`,
    /// so nothing moves where the instances place it. `frame` must be a rigid motion.
    pub fn reorigin_definition(
        &mut self,
        definition: DefinitionId,
        frame: DAffine3,
    ) -> Result<SceneChange, KernelError> {
        if !frame.is_finite() {
            return Err(KernelError::NotFinite);
        }
        let m = frame.matrix3;
        if !(m.transpose() * m).abs_diff_eq(glam::DMat3::IDENTITY, 1e-9) || m.determinant() < 0.0 {
            return Err(KernelError::BadAxes("axes must be a rigid motion"));
        }
        if self.definition(definition).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        if frame == DAffine3::IDENTITY {
            return Ok(SceneChange::default());
        }
        let inverse = frame.inverse();
        let inside = self.edit(MeshId::Definition(definition), |m| {
            let vertices = m.all_vertices();
            let instances: Vec<_> = m.instances().map(|(id, _)| id).collect();
            let guides: Vec<_> = m.guides().map(|(id, _)| id).collect();
            let (dimensions, notes) = m.all_annotations();
            let sections = m.all_sections();
            m.hold_dimensions(&dimensions);
            // Textures are projected in the definition's coordinates, so each painted face's
            // placement takes up the move (as it does when an instance is exploded).
            let painted: Vec<_> = m
                .faces()
                .filter(|(_, f)| f.paint().iter().any(Option::is_some))
                .map(|(id, f)| (id, f.texture, f.normal))
                .collect();
            let mut c = match m.transform_vertices(&vertices, inverse) {
                Ok(c) => c,
                Err(e) => {
                    m.hold_dimensions(&[]);
                    return Err(e);
                }
            };
            for (id, texture, normal) in painted {
                m.set_texture(id, TexturePlacement::carried(texture, normal, &inverse));
            }
            c.ops.extend(m.take_changes().ops);
            c.ops.extend(m.transform_instances(&instances, inverse).ops);
            c.ops.extend(m.transform_guides(&guides, inverse).ops);
            c.ops
                .extend(m.transform_annotations(&dimensions, &notes, inverse).ops);
            c.ops.extend(m.transform_sections(&sections, inverse).ops);
            Ok(c)
        })?;
        let meshes: Vec<MeshId> = self.meshes().map(|(id, _)| id).collect();
        let mut changes = vec![inside];
        for mesh in meshes {
            let placed: Vec<_> = self
                .mesh(mesh)
                .into_iter()
                .flat_map(|m| m.instances())
                .filter(|(_, i)| i.definition == definition)
                .map(|(id, i)| (id, i.transform))
                .collect();
            if placed.is_empty() {
                continue;
            }
            changes.push(self.edit(mesh, |m| {
                for (id, t) in placed {
                    m.set_instance_transform(id, t * frame);
                }
                Ok(m.take_changes())
            })?);
        }
        Ok(SceneChange::join(changes))
    }
}
