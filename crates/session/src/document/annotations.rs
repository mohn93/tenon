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

    pub(super) fn add_label_changes(
        &mut self,
        target: EntityId,
        text: &str,
        position: Option<[f64; 3]>,
    ) -> Result<SceneChange, CommandError> {
        let input = match target {
            EntityId::Vertex(_) => {
                self.anchor(AnchorInput::Vertex(target))?;
                AnchorInput::Vertex(target)
            }
            EntityId::Edge(_) => {
                let e = self.edge_of(target)?;
                let [a, b] = self.model().edge(e).expect("checked").v;
                let m = self.model();
                AnchorInput::Point(((point_of(m, a) + point_of(m, b)) * 0.5).to_array())
            }
            EntityId::Face(f) => {
                if self.model().face(f).is_none() {
                    return Err(self.missing_error(vec![target], format!("no entity {target}")));
                }
                let tris = self.model().triangulate(f).unwrap_or_default();
                let p = interior_point_near_centroid(&tris).ok_or_else(|| {
                    invalid("the face cannot be labelled: it has no area").with_ids(vec![target])
                })?;
                AnchorInput::Point(p.to_array())
            }
            EntityId::Instance(i) => {
                let Some(inst) = self.model().instance(i) else {
                    return Err(self.missing_error(vec![target], format!("no entity {target}")));
                };
                let inst = inst.clone();
                let Some([lo, hi]) = self.scene.instance_bounds(&inst) else {
                    return Err(
                        invalid("the instance is empty: nothing to label").with_ids(vec![target])
                    );
                };
                let center = (lo + hi) * 0.5;
                let eye = self
                    .context_transform()
                    .inverse()
                    .transform_point3(self.camera.eye);
                let mut tris = Vec::new();
                let hidden_tags = self.scene.hidden_tags();
                self.instance_triangles(&inst, 0, &hidden_tags, &mut tris);
                let toward = eye - center;
                let hit = (toward.length() > 1e-9)
                    .then(|| toward.normalize())
                    .and_then(|dir| {
                        tris.iter()
                            .filter_map(|t| kernel::ray_hits_triangle(center, dir, *t))
                            .filter(|t| *t > 1e-9 && *t <= toward.length())
                            .min_by(f64::total_cmp)
                            .map(|t| center + dir * t)
                    });
                AnchorInput::Point(hit.unwrap_or(center).to_array())
            }
            _ => {
                return Err(invalid(
                    "a label target is a face, edge, vertex or instance (f..., e..., v..., i...)",
                )
                .with_ids(vec![target]));
            }
        };
        let position = match position {
            Some(p) => p,
            None => {
                let anchor = match input {
                    AnchorInput::Vertex(EntityId::Vertex(v)) => point_of(self.model(), v),
                    AnchorInput::Point(p) => DVec3::from(p),
                    AnchorInput::Vertex(_) => unreachable!("checked above"),
                };
                self.label_position(anchor).to_array()
            }
        };
        self.add_note_changes(input, position, Some(text.to_string()))
    }

    /// Where a label's note goes by default: outward from the open context's box center
    /// through `anchor`, 15% of the box's diagonal away, raised by 5% of it.
    fn label_position(&self, anchor: DVec3) -> DVec3 {
        let (center, diagonal) = match self.scene.bounds(self.active()) {
            Some([lo, hi]) => ((lo + hi) * 0.5, (hi - lo).length()),
            None => (anchor, 0.0),
        };
        let diagonal = if diagonal > 1e-9 { diagonal } else { 1.0 };
        let out = (anchor - center).try_normalize().unwrap_or(DVec3::X);
        anchor + out * (0.15 * diagonal) + DVec3::Z * (0.05 * diagonal)
    }

    /// The triangles of an instance's geometry, nested instances included, in the open
    /// context's coordinates.
    /// The triangles of the faces of `inst` that are shown (not hidden, not on `hidden_tags`),
    /// placed, including those of its shown instances.
    fn instance_triangles(
        &self,
        inst: &kernel::Instance,
        depth: usize,
        hidden_tags: &std::collections::BTreeSet<kernel::TagId>,
        out: &mut Vec<[DVec3; 3]>,
    ) {
        let Some(m) = self.scene.mesh(kernel::MeshId::Definition(inst.definition)) else {
            return;
        };
        if depth > 16 {
            return;
        }
        for (f, face) in m.faces() {
            if face.hidden || !kernel::tag_shown_among(hidden_tags, face.tag) {
                continue;
            }
            for t in m.triangulate(f).unwrap_or_default() {
                out.push(t.map(|p| inst.transform.transform_point3(p)));
            }
        }
        for (_, child) in m.instances() {
            if child.hidden || !kernel::tag_shown_among(hidden_tags, child.tag) {
                continue;
            }
            let placed = kernel::Instance::new(child.definition, inst.transform * child.transform);
            self.instance_triangles(&placed, depth + 1, hidden_tags, out);
        }
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
            return Err(self.missing_error(vec![note], format!("no entity {note}")));
        }
        self.edit_mesh(|m| m.set_note_text(id, text))
            .map_err(|e| invalid(&e.to_string()).with_ids(vec![note]))
    }

    /// An anchor of the open context: an existing vertex, or a point within the limit.
    fn anchor(&self, input: AnchorInput) -> Result<Anchor, CommandError> {
        match input {
            AnchorInput::Vertex(EntityId::Vertex(v)) => match self.model().vertex(v) {
                Some(_) => Ok(Anchor::Vertex(v)),
                None => {
                    let id = EntityId::Vertex(v);
                    Err(self.missing_error(vec![id], format!("no entity {id}")))
                }
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
            return Err(self.missing_error(vec![id], format!("no entity {id}")));
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

fn point_of(m: &Model, v: VertexId) -> DVec3 {
    m.position(v).expect("checked")
}

/// The point inside a triangulated face nearest its area centroid: the centroid when a
/// triangle holds it off the face's edges, else the nearest triangle's closest point nudged toward that triangle's
/// center, so it lies inside the face and never in a hole.
fn interior_point_near_centroid(tris: &[[DVec3; 3]]) -> Option<DVec3> {
    let mut area = 0.0;
    let mut sum = DVec3::ZERO;
    for t in tris {
        let a = (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5;
        area += a;
        sum += a * (t[0] + t[1] + t[2]) / 3.0;
    }
    if area <= 0.0 {
        return None;
    }
    let centroid = sum / area;
    let (mut best, mut best_d) = (None, f64::INFINITY);
    for t in tris {
        let q = closest_on_triangle(centroid, *t);
        let d = q.distance(centroid);
        if d < best_d {
            let center = (t[0] + t[1] + t[2]) / 3.0;
            best_d = d;
            best = Some((q, q + (center - q) * 0.1));
        }
    }
    let (q, nudged) = best?;
    // On a side shared by two triangles the centroid is inside the face; on the face's own
    // edge (around a hole) it is nudged strictly inside too.
    Some(
        if q.distance(centroid) < 1e-9 && !on_face_edge(centroid, tris) {
            centroid
        } else {
            nudged
        },
    )
}

/// Whether `p` lies on a side of `tris` that no other triangle shares: an edge of the face.
fn on_face_edge(p: DVec3, tris: &[[DVec3; 3]]) -> bool {
    let same = |a: DVec3, b: DVec3| a.distance_squared(b) < 1e-24;
    let shared = |u: DVec3, v: DVec3, skip: usize| {
        tris.iter().enumerate().any(|(j, t)| {
            j != skip && t.iter().any(|&a| same(a, u)) && t.iter().any(|&a| same(a, v))
        })
    };
    tris.iter().enumerate().any(|(i, t)| {
        [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]
            .into_iter()
            .any(|(u, v)| {
                let along = (p - u).dot(v - u) / (v - u).length_squared().max(f64::MIN_POSITIVE);
                let foot = u + (v - u) * along.clamp(0.0, 1.0);
                foot.distance(p) < 1e-9 && !shared(u, v, i)
            })
    })
}

/// The point of triangle `t` nearest `p`.
fn closest_on_triangle(p: DVec3, [a, b, c]: [DVec3; 3]) -> DVec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `p` (in the plane of triangle `t`) lies inside it and off its sides.
    fn strictly_inside(p: DVec3, [a, b, c]: [DVec3; 3]) -> bool {
        let n = (b - a).cross(c - a);
        let area = n.length_squared();
        if area <= 0.0 {
            return false;
        }
        let tolerance = 1e-9 * area;
        [(a, b), (b, c), (c, a)]
            .iter()
            .all(|&(u, v)| (v - u).cross(p - u).dot(n) > tolerance)
    }

    #[test]
    fn a_centroid_on_the_faces_edge_is_nudged_strictly_inside() {
        // Two triangles meeting at a corner: the centroid is that corner, on both sides.
        let t1 = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let t2 = [DVec3::ZERO, -DVec3::X, -DVec3::Y];
        let p = interior_point_near_centroid(&[t1, t2]).unwrap();
        assert!(p != DVec3::ZERO);
        assert!(strictly_inside(p, t1) || strictly_inside(p, t2), "{p:?}");
        // On the diagonal two triangles share: inside the face, kept.
        let (a, b, c, d) = (
            DVec3::ZERO,
            DVec3::X * 2.0,
            DVec3::new(2.0, 1.0, 0.0),
            DVec3::Y,
        );
        let p = interior_point_near_centroid(&[[a, b, c], [a, c, d]]).unwrap();
        assert!((p - DVec3::new(1.0, 0.5, 0.0)).length() < 1e-12, "{p:?}");
        // A centroid well inside is kept.
        let big = [DVec3::ZERO, DVec3::X * 3.0, DVec3::Y * 3.0];
        let p = interior_point_near_centroid(&[big]).unwrap();
        assert!((p - DVec3::new(1.0, 1.0, 0.0)).length() < 1e-12);
    }
}
