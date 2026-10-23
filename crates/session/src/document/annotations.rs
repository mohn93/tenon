//! Dimensions, notes and the display units their labels are written in.

use glam::{DAffine3, DVec3};
use kernel::{
    Anchor, Circle, CurveKind, Dimension, DimensionId, EntityId, KernelError, Model, Note, NoteId,
    RadialAnchor, SceneChange, VertexId,
};

use super::{Document, check_coordinates};
use crate::command::{AnchorInput, CommandError, ErrorKind};
use crate::query::{AnchorInfo, DimensionInfo, DimensionKind, NoteInfo, NoteKind};
use crate::units::{LengthUnit, Units, format_length};

impl Document {
    /// The model's display units, which dimension and note labels are written in.
    pub fn units(&self) -> Units {
        self.units
    }

    /// The units change since `before`, for the undo step: None when they are the same.
    pub(super) fn units_change(&self, before: Units) -> Option<(Units, Units)> {
        (self.units != before).then_some((before, self.units))
    }

    pub(super) fn set_units_changes(
        &mut self,
        length: Option<LengthUnit>,
        precision: Option<u8>,
        length_snap: Option<f64>,
        angle_precision: Option<u8>,
        angle_snap: Option<f64>,
    ) -> Result<SceneChange, CommandError> {
        if length.is_none()
            && precision.is_none()
            && length_snap.is_none()
            && angle_precision.is_none()
            && angle_snap.is_none()
        {
            return Err(invalid(
                "give length, precision, length_snap, angle_precision or angle_snap",
            ));
        }
        let units = Units {
            length: length.unwrap_or(self.units.length),
            precision: precision.unwrap_or(self.units.precision),
            length_snap: length_snap.unwrap_or(self.units.length_snap),
            angle_precision: angle_precision.unwrap_or(self.units.angle_precision),
            angle_snap: angle_snap.unwrap_or(self.units.angle_snap),
        };
        units.check().map_err(|e| invalid(&e))?;
        self.units = units;
        // The units are the document's; `execute` records their change with the step.
        Ok(SceneChange::default())
    }

    pub(super) fn add_dimension_changes(
        &mut self,
        a: AnchorInput,
        b: AnchorInput,
        offset: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        let (a, b) = (self.anchor(a)?, self.anchor(b)?);
        self.add_linear(a, b, offset, &[])
    }

    pub(super) fn add_edge_dimension_changes(
        &mut self,
        edge: EntityId,
        offset: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        let e = self.edge_of(edge)?;
        let [a, b] = self.model().edge(e).expect("checked").v;
        self.add_linear(Anchor::Vertex(a), Anchor::Vertex(b), offset, &[edge])
    }

    /// Adds a linear dimension between two checked anchors; `ids` are named in errors.
    fn add_linear(
        &mut self,
        a: Anchor,
        b: Anchor,
        offset: [f64; 3],
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[offset])?;
        let m = self.model();
        let (pa, pb) = (point(m, &a), point(m, &b));
        let shift = perpendicular(DVec3::from(offset), pb - pa);
        check_coordinates(&[(pa + shift).to_array(), (pb + shift).to_array()])?;
        self.edit_mesh(|m| {
            m.add_dimension(Dimension::Linear {
                a,
                b,
                offset: DVec3::from(offset),
                tag: None,
            })
        })
        .map_err(|e| match e {
            KernelError::ZeroDistance => {
                invalid("the two ends coincide: a dimension measures between two points apart")
                    .with_ids(ids.to_vec())
            }
            e => invalid(&e.to_string()).with_ids(ids.to_vec()),
        })
    }

    pub(super) fn add_radial_dimension_changes(
        &mut self,
        edge: EntityId,
        diameter: bool,
        leader: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        let e = self.edge_of(edge)?;
        let not_round =
            || invalid(&format!("{edge} is not part of a circle or an arc")).with_ids(vec![edge]);
        let curve = self.model().curve_of(e).ok_or_else(not_round)?;
        let Some(circle) = curve
            .circle
            .filter(|_| matches!(curve.kind(), CurveKind::Circle | CurveKind::Arc))
        else {
            return Err(not_round());
        };
        check_coordinates(&[leader])?;
        let reach = Circle {
            center: circle.center,
            normal: circle.normal,
            radius: circle.radius,
        }
        .extent();
        let mut points: Vec<[f64; 3]> = reach.iter().map(|p| p.to_array()).collect();
        points.push((circle.center + DVec3::from(leader)).to_array());
        check_coordinates(&points)?;
        self.edit_mesh(|m| {
            m.add_dimension(Dimension::Radial {
                curve: RadialAnchor::Curve(curve.id),
                diameter,
                leader: DVec3::from(leader),
                tag: None,
            })
        })
        .map_err(|e| invalid(&e.to_string()).with_ids(vec![edge]))
    }

    pub(super) fn add_note_changes(
        &mut self,
        anchor: AnchorInput,
        position: [f64; 3],
        text: Option<String>,
    ) -> Result<SceneChange, CommandError> {
        let anchor = self.anchor(anchor)?;
        check_coordinates(&[position])?;
        let mesh = self.active();
        self.scene
            .add_note(
                mesh,
                Note::Leader {
                    anchor,
                    position: DVec3::from(position),
                    text,
                    tag: None,
                },
            )
            .map_err(|e| invalid(&e.to_string()))
    }

    pub(super) fn add_screen_note_changes(
        &mut self,
        at: [f64; 2],
        text: &str,
    ) -> Result<SceneChange, CommandError> {
        let mesh = self.active();
        self.scene
            .add_note(
                mesh,
                Note::Screen {
                    at,
                    text: text.to_string(),
                    tag: None,
                },
            )
            .map_err(|e| invalid(&e.to_string()))
    }

    pub(super) fn change_note_changes(
        &mut self,
        note: EntityId,
        text: Option<String>,
    ) -> Result<SceneChange, CommandError> {
        let EntityId::Note(id) = note else {
            return Err(invalid("change_note needs a note id (n...)").with_ids(vec![note]));
        };
        if self.model().note(id).is_none() {
            return Err(not_found(note));
        }
        self.edit_mesh(|m| m.set_note_text(id, text))
            .map_err(|e| invalid(&e.to_string()).with_ids(vec![note]))
    }

    /// An anchor of the open context: an existing vertex, or a point within the limit.
    fn anchor(&self, input: AnchorInput) -> Result<Anchor, CommandError> {
        match input {
            AnchorInput::Vertex(EntityId::Vertex(v)) => match self.model().vertex(v) {
                Some(_) => Ok(Anchor::Vertex(v)),
                None => Err(not_found(EntityId::Vertex(v))),
            },
            AnchorInput::Vertex(id) => {
                Err(invalid("an anchor is a vertex id or a point").with_ids(vec![id]))
            }
            AnchorInput::Point(p) => {
                check_coordinates(&[p])?;
                Ok(Anchor::Point(DVec3::from(p)))
            }
        }
    }

    /// The edge `id` names in the open context.
    fn edge_of(&self, id: EntityId) -> Result<kernel::EdgeId, CommandError> {
        let EntityId::Edge(e) = id else {
            return Err(invalid("an edge id (e...) is needed here").with_ids(vec![id]));
        };
        if self.model().edge(e).is_none() {
            return Err(not_found(id));
        }
        Ok(e)
    }

    /// The `entity` info of a dimension of the open context.
    pub(super) fn dimension_info(&self, id: DimensionId) -> DimensionInfo {
        let m = self.model();
        let d = *m.dimension(id).expect("checked");
        let label = dimension_label(m, id, &self.units).unwrap_or_default();
        let mut info = DimensionInfo {
            id: EntityId::Dimension(id),
            kind: DimensionKind::Linear,
            a: None,
            b: None,
            offset: None,
            line: None,
            curve: None,
            center: None,
            normal: None,
            radius: None,
            diameter: false,
            leader: None,
            value: dimension_value(m, id).unwrap_or(0.0),
            label,
            tag: d.tag(),
        };
        match d {
            Dimension::Linear { a, b, offset, .. } => {
                info.a = Some(anchor_info(m, &a));
                info.b = Some(anchor_info(m, &b));
                info.offset = Some(offset.to_array());
                info.line = m
                    .dimension_line(id)
                    .map(|(p, q)| [p.to_array(), q.to_array()]);
            }
            Dimension::Radial {
                curve,
                diameter,
                leader,
                ..
            } => {
                info.kind = DimensionKind::Radial;
                if let RadialAnchor::Curve(c) = curve {
                    info.curve = Some(c.to_string());
                }
                if let Some(c) = m.radial_circle(id) {
                    info.center = Some(c.center.to_array());
                    info.normal = Some(c.normal.to_array());
                    info.radius = Some(c.radius);
                }
                info.diameter = diameter;
                info.leader = Some(leader.to_array());
            }
        }
        info
    }

    /// The `entity` info of a note of the open context.
    pub(super) fn note_info(&self, id: NoteId) -> NoteInfo {
        let m = self.model();
        let n = m.note(id).expect("checked");
        let label = note_label(m, id, &self.units, self.context_transform()).unwrap_or_default();
        match n {
            Note::Leader {
                anchor,
                position,
                text,
                tag,
            } => NoteInfo {
                id: EntityId::Note(id),
                kind: NoteKind::Leader,
                anchor: Some(anchor_info(m, anchor)),
                position: Some(position.to_array()),
                at: None,
                text: text.clone(),
                label,
                tag: *tag,
            },
            Note::Screen { at, text, tag } => NoteInfo {
                id: EntityId::Note(id),
                kind: NoteKind::Screen,
                anchor: None,
                position: None,
                at: Some(*at),
                text: Some(text.clone()),
                label,
                tag: *tag,
            },
        }
    }

    /// The points annotations of the open context would have under `t`: their anchors (a
    /// vertex anchor moves only when `follows` it), the ends of a linear dimension's line, a
    /// radial dimension's label point and its circle's center and reach along each axis (the
    /// circle mapped when it is fixed or its curve's vertices follow), and a leader note's
    /// text position.
    pub(super) fn annotation_points_placed(
        &self,
        dimensions: &[DimensionId],
        notes: &[NoteId],
        t: DAffine3,
        follows: &dyn Fn(VertexId) -> bool,
    ) -> Vec<DVec3> {
        let m = self.model();
        let at = |anchor: &Anchor| match *anchor {
            Anchor::Vertex(v) => {
                let p = m.position(v).unwrap_or_default();
                if follows(v) { t.transform_point3(p) } else { p }
            }
            Anchor::Point(p) => p,
        };
        let mut out = Vec::new();
        for id in dimensions {
            let Some(d) = m.dimension(*id) else { continue };
            match d.transformed(t) {
                Dimension::Linear { a, b, offset, .. } => {
                    let (pa, pb) = (at(&a), at(&b));
                    // The offset as stored, and the part of it drawn (see `dimension_line`).
                    let shift = perpendicular(offset, pb - pa);
                    out.extend([pa, pb, pa + offset, pa + shift, pb + shift]);
                }
                Dimension::Radial { curve, leader, .. } => {
                    let circle = match curve {
                        RadialAnchor::Fixed {
                            center,
                            normal,
                            radius,
                        } => Circle {
                            center,
                            normal,
                            radius,
                        },
                        RadialAnchor::Curve(c) => {
                            let Some(circle) = m.radial_circle(*id) else {
                                continue;
                            };
                            let moves = m
                                .curve(c)
                                .is_some_and(|c| c.vertices.iter().all(|v| follows(*v)));
                            if moves { circle.transformed(t) } else { circle }
                        }
                    };
                    out.extend(circle.extent());
                    out.push(circle.center + leader);
                }
            }
        }
        for id in notes {
            let Some(n) = m.note(*id) else { continue };
            if let Note::Leader {
                anchor, position, ..
            } = n.transformed(t)
            {
                out.extend([at(&anchor), position]);
            }
        }
        out
    }
}

/// The length a dimension measures, in meters: the distance between its ends, or its
/// circle's radius or diameter.
pub fn dimension_value(model: &Model, id: DimensionId) -> Option<f64> {
    match model.dimension(id)? {
        Dimension::Linear { .. } => model.dimension_points(id).map(|(a, b)| a.distance(b)),
        Dimension::Radial { diameter, .. } => {
            let r = model.radial_circle(id)?.radius;
            Some(if *diameter { 2.0 * r } else { r })
        }
    }
}

/// A dimension's label in `units`: the length ("1.5 m"), or for a radial dimension "R 1.5 m"
/// (radius) or "Ø 3 m" (diameter).
pub fn dimension_label(model: &Model, id: DimensionId, units: &Units) -> Option<String> {
    let value = format_length(dimension_value(model, id)?, units);
    Some(match model.dimension(id)? {
        Dimension::Linear { .. } => value,
        Dimension::Radial {
            diameter: false, ..
        } => format!("R {value}"),
        Dimension::Radial { diameter: true, .. } => format!("Ø {value}"),
    })
}

/// A note's label: its text, or for a leader note without text the anchored point's
/// coordinates taken into root coordinates by `to_world` and written in `units`,
/// "(1 m, 2 m, 0 m)".
pub fn note_label(model: &Model, id: NoteId, units: &Units, to_world: DAffine3) -> Option<String> {
    let note = model.note(id)?;
    if let Some(text) = note.text() {
        return Some(text.to_string());
    }
    let p = to_world.transform_point3(model.note_point(id)?);
    let [x, y, z] = p.to_array().map(|c| format_length(c, units));
    Some(format!("({x}, {y}, {z})"))
}

/// Where an anchor of a checked annotation is.
fn point(m: &Model, anchor: &Anchor) -> DVec3 {
    m.anchor_point(anchor).unwrap_or_default()
}

fn anchor_info(m: &Model, anchor: &Anchor) -> AnchorInfo {
    AnchorInfo {
        point: point(m, anchor).to_array(),
        vertex: anchor.vertex().map(EntityId::Vertex),
    }
}

/// The part of `v` perpendicular to `along` (all of `v` when `along` is zero).
fn perpendicular(v: DVec3, along: DVec3) -> DVec3 {
    match along.try_normalize() {
        Some(u) => v - u * v.dot(u),
        None => v,
    }
}

fn invalid(message: &str) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

fn not_found(id: EntityId) -> CommandError {
    CommandError::new(ErrorKind::NotFound, format!("no entity {id}")).with_ids(vec![id])
}
