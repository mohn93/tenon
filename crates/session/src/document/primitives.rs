//! Solid primitives: `add_box`, `add_cylinder`, `add_tube`, `add_cone`, `add_prism`, `add_sphere` and
//! `add_torus`.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};
use kernel::{DefinitionKind, EntityId, MaterialId, SceneChange, Solid};

use super::{Document, check_coordinates, unit_vector};
use crate::command::{CommandError, ErrorKind, PrimitiveAs};
use crate::rectangle::rectangle_axes;

/// What every primitive command shares: where the solid goes and how it is named and painted.
pub(super) struct Placement<'a> {
    placement: PrimitiveAs,
    name: Option<&'a str>,
    material: Option<MaterialId>,
}

pub(super) fn placement_of(
    placement: PrimitiveAs,
    name: &Option<String>,
    material: Option<MaterialId>,
) -> Placement<'_> {
    Placement {
        placement,
        name: name.as_deref(),
        material,
    }
}

type Base = ([f64; 3], Option<[f64; 3]>);

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

fn refusal(e: kernel::PrimitiveError) -> CommandError {
    invalid(e.to_string())
}

/// The frame of a curved primitive: local +Z along `axis` (default Z), local +X along the
/// plane's u axis for it (the one `add_circle` starts on), the origin at `base`. Also returns
/// the unit axis.
fn frame((base, axis): Base) -> Result<(DAffine3, DVec3), CommandError> {
    check_coordinates(&[base])?;
    let axis = match axis {
        None => DVec3::Z,
        Some(a) => unit_vector(DVec3::from(a))
            .ok_or_else(|| invalid("axis must be a finite, non-zero vector"))?,
    };
    let (u, v) = rectangle_axes(axis);
    Ok((DAffine3::from_cols(u, v, axis, DVec3::from(base)), axis))
}

impl Document {
    pub(super) fn add_box_changes(
        &mut self,
        corner: [f64; 3],
        size: [f64; 3],
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[corner])?;
        let solid = Solid::cuboid(DVec3::from(size)).map_err(refusal)?;
        let at = DAffine3::from_translation(DVec3::from(corner));
        self.place_solid("Box", &solid, at, options)
    }

    pub(super) fn add_cylinder_changes(
        &mut self,
        base: Base,
        (radius, height, segments): (f64, f64, Option<u32>),
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        let (at, _) = frame(base)?;
        let solid = Solid::cylinder(radius, height, segments.unwrap_or(kernel::DEFAULT_SEGMENTS))
            .map_err(refusal)?;
        self.place_solid("Cylinder", &solid, at, options)
    }

    pub(super) fn add_tube_changes(
        &mut self,
        base: Base,
        (radius, inner_radius, height, segments): (f64, f64, f64, Option<u32>),
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        let (at, _) = frame(base)?;
        let solid = Solid::tube(
            radius,
            inner_radius,
            height,
            segments.unwrap_or(kernel::DEFAULT_SEGMENTS),
        )
        .map_err(refusal)?;
        self.place_solid("Tube", &solid, at, options)
    }

    pub(super) fn add_cone_changes(
        &mut self,
        base: Base,
        (radius, top_radius, height, segments): (f64, f64, f64, Option<u32>),
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        let (at, _) = frame(base)?;
        let solid = Solid::cone(
            radius,
            top_radius,
            height,
            segments.unwrap_or(kernel::DEFAULT_SEGMENTS),
        )
        .map_err(refusal)?;
        self.place_solid("Cone", &solid, at, options)
    }

    pub(super) fn add_prism_changes(
        &mut self,
        base: Base,
        (radius, height, sides, direction): (f64, f64, Option<u32>, Option<[f64; 3]>),
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        let (at, axis) = frame(base)?;
        // The first corner's angle from the plane's u axis.
        let first = match direction {
            None => 0.0,
            Some(d) => {
                let d = DVec3::from(d);
                let flat = d - axis * d.dot(axis);
                if !d.is_finite() || flat.length() < 1e-9 * d.length().max(1.0) {
                    return Err(invalid(
                        "direction must be a finite vector that is not along the axis",
                    ));
                }
                let (u, v) = rectangle_axes(axis);
                flat.dot(v).atan2(flat.dot(u))
            }
        };
        let solid = Solid::prism(
            radius,
            height,
            sides.unwrap_or(kernel::DEFAULT_SIDES),
            first,
        )
        .map_err(refusal)?;
        self.place_solid("Prism", &solid, at, options)
    }

    pub(super) fn add_sphere_changes(
        &mut self,
        center: Base,
        (radius, segments): (f64, Option<u32>),
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        let (at, _) = frame(center)?;
        let solid =
            Solid::sphere(radius, segments.unwrap_or(kernel::DEFAULT_SEGMENTS)).map_err(refusal)?;
        self.place_solid("Sphere", &solid, at, options)
    }

    pub(super) fn add_torus_changes(
        &mut self,
        center: Base,
        (radius, tube_radius, segments, tube_segments): (f64, f64, Option<u32>, Option<u32>),
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        let (at, _) = frame(center)?;
        let solid = Solid::torus(
            radius,
            tube_radius,
            segments.unwrap_or(kernel::DEFAULT_SEGMENTS),
            tube_segments.unwrap_or(kernel::DEFAULT_TUBE_SEGMENTS),
        )
        .map_err(refusal)?;
        self.place_solid("Torus", &solid, at, options)
    }

    /// Puts `solid` (in its own frame) at `at` in the open context: as a new group or
    /// component with one placed instance (selected), or as loose geometry.
    fn place_solid(
        &mut self,
        kind: &str,
        solid: &Solid,
        at: DAffine3,
        options: Placement<'_>,
    ) -> Result<SceneChange, CommandError> {
        if options.name.is_some_and(|n| n.trim().is_empty()) {
            return Err(invalid("name must not be blank"));
        }
        if let Some(m) = options.material
            && self.scene.material(m).is_none()
        {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no material {m}"),
            ));
        }
        let placed: Vec<[f64; 3]> = solid
            .points()
            .iter()
            .map(|p| at.transform_point3(*p).to_array())
            .collect();
        check_coordinates(&placed)?;
        let mesh = self.active();
        let kernel_error = |e: kernel::KernelError| invalid(e.to_string());
        if options.placement == PrimitiveAs::Geometry {
            return self
                .scene
                .draw_solid(mesh, solid, at, options.material)
                .map_err(kernel_error);
        }
        let (definition_kind, name) = match options.placement {
            PrimitiveAs::Component => (
                DefinitionKind::Component,
                options.name.unwrap_or(kind).to_string(),
            ),
            _ => (
                DefinitionKind::Group,
                options.name.unwrap_or_default().to_string(),
            ),
        };
        let (instance, change) = self
            .scene
            .add_solid(mesh, solid, definition_kind, &name, at, options.material)
            .map_err(kernel_error)?;
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        self.made_definitions = self.model().instance(instance).map(|i| vec![i.definition]);
        Ok(change)
    }
}
