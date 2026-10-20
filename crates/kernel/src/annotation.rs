//! Annotations: dimensions and notes. They are drawn over the model, bound no faces and are
//! exported only to model files.
//!
//! A dimension or leader note is anchored to a vertex of its own mesh, or to a fixed point. A
//! vertex anchor follows every edit that keeps the vertex; when an edit removes the vertex
//! the anchor becomes a point where the vertex last was, in the same change set, so undo
//! attaches it again (see `freeze_annotations`). A radial dimension measures a curve that is
//! a circle or an arc; when the curve is gone or no longer one, it keeps the circle it last
//! had.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::curves::CurveKind;
use crate::edit::KernelError;
use crate::geom::LENGTH_TOL;
use crate::ids::{CurveId, DimensionId, EdgeId, EntityId, FaceId, NoteId, TagId, VertexId};
use crate::model::{ChangeSet, Model, Op};
use crate::scene::{MeshId, Scene, SceneChange};

/// The most characters a note's text may have.
pub const MAX_NOTE_CHARS: usize = 1024;

/// The most lines a note's text may have.
pub const MAX_NOTE_LINES: usize = 64;

/// What a dimension end or a note's leader points at.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Anchor {
    /// A vertex of the same mesh: the anchor follows it.
    Vertex(VertexId),
    /// A fixed point in the mesh's coordinates.
    Point(DVec3),
}

/// The circle a radial dimension measures.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RadialAnchor {
    /// A curve of the same mesh that is a circle or an arc: the dimension follows it.
    Curve(CurveId),
    /// A fixed circle: `normal` is a unit vector, `radius` positive.
    Fixed {
        center: DVec3,
        normal: DVec3,
        radius: f64,
    },
}

/// A circle as a radial dimension measures it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub center: DVec3,
    /// Unit normal of the circle's plane.
    pub normal: DVec3,
    pub radius: f64,
}

/// A dimension: a measured distance or radius, labelled with its length.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Dimension {
    /// The distance between `a` and `b`. The dimension line is the segment a-b moved by
    /// `offset`, perpendicular to a-b (see `Model::dimension_line`).
    Linear {
        a: Anchor,
        b: Anchor,
        offset: DVec3,
        /// The tag the dimension is on; None is Untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<TagId>,
    },
    /// The radius (or with `diameter` the diameter) of a circle or arc; the label sits at
    /// `leader` from the center.
    Radial {
        curve: RadialAnchor,
        diameter: bool,
        leader: DVec3,
        /// The tag the dimension is on; None is Untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<TagId>,
    },
}

/// A note: text pointing at a point of the model, or fixed to the view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Note {
    /// Text at `position` with a leader line to `anchor`. Without text, the label is the
    /// anchored point's coordinates.
    Leader {
        anchor: Anchor,
        position: DVec3,
        text: Option<String>,
        /// The tag the note is on; None is Untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<TagId>,
    },
    /// Text fixed to the view, `at` fractions (0 to 1) of its width and height from the top
    /// left corner. Only the root mesh holds screen notes.
    Screen {
        at: [f64; 2],
        text: String,
        /// The tag the note is on; None is Untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<TagId>,
    },
}

impl Dimension {
    /// The tag the dimension is on; None is Untagged.
    pub fn tag(&self) -> Option<TagId> {
        match *self {
            Dimension::Linear { tag, .. } | Dimension::Radial { tag, .. } => tag,
        }
    }

    /// The same dimension on `tag`.
    pub fn with_tag(mut self, to: Option<TagId>) -> Dimension {
        match &mut self {
            Dimension::Linear { tag, .. } | Dimension::Radial { tag, .. } => *tag = to,
        }
        self
    }

    /// The dimension moved by the affine map `t`: point anchors, a fixed circle, the offset
    /// and the leader move; vertex anchors and curves stay (they move with their geometry).
    pub fn transformed(&self, t: DAffine3) -> Dimension {
        match *self {
            Dimension::Linear { a, b, offset, tag } => Dimension::Linear {
                a: a.transformed(t),
                b: b.transformed(t),
                offset: t.transform_vector3(offset),
                tag,
            },
            Dimension::Radial {
                curve,
                diameter,
                leader,
                tag,
            } => Dimension::Radial {
                curve: match curve {
                    RadialAnchor::Fixed {
                        center,
                        normal,
                        radius,
                    } => {
                        let c = Circle {
                            center,
                            normal,
                            radius,
                        }
                        .transformed(t);
                        RadialAnchor::Fixed {
                            center: c.center,
                            normal: c.normal,
                            radius: c.radius,
                        }
                    }
                    c => c,
                },
                diameter,
                leader: t.transform_vector3(leader),
                tag,
            },
        }
    }
}

impl Note {
    /// The tag the note is on; None is Untagged.
    pub fn tag(&self) -> Option<TagId> {
        match *self {
            Note::Leader { tag, .. } | Note::Screen { tag, .. } => tag,
        }
    }

    /// The same note on `tag`.
    pub fn with_tag(mut self, to: Option<TagId>) -> Note {
        match &mut self {
            Note::Leader { tag, .. } | Note::Screen { tag, .. } => *tag = to,
        }
        self
    }

    /// The note moved by the affine map `t`: a point anchor and the text position move; a
    /// vertex anchor stays (it moves with its vertex) and a screen note does not move.
    pub fn transformed(&self, t: DAffine3) -> Note {
        match self.clone() {
            Note::Leader {
                anchor,
                position,
                text,
                tag,
            } => Note::Leader {
                anchor: anchor.transformed(t),
                position: t.transform_point3(position),
                text,
                tag,
            },
            n @ Note::Screen { .. } => n,
        }
    }

    /// The text written; None for a leader note showing its anchor's coordinates.
    pub fn text(&self) -> Option<&str> {
        match self {
            Note::Leader { text, .. } => text.as_deref(),
            Note::Screen { text, .. } => Some(text),
        }
    }
}

impl Anchor {
    /// The vertex anchored to, if any.
    pub fn vertex(&self) -> Option<VertexId> {
        match self {
            Anchor::Vertex(v) => Some(*v),
            Anchor::Point(_) => None,
        }
    }

    fn transformed(self, t: DAffine3) -> Anchor {
        match self {
            Anchor::Point(p) => Anchor::Point(t.transform_point3(p)),
            a => a,
        }
    }
}

impl Circle {
    /// The circle under the affine map `t`; under a map that is not a similarity, the radius
    /// is how `t` stretches one direction in the plane.
    fn transformed(self, t: DAffine3) -> Circle {
        let normal = (t.matrix3.inverse().transpose() * self.normal)
            .try_normalize()
            .unwrap_or(self.normal);
        let along = t
            .transform_vector3(self.normal.any_orthonormal_vector())
            .length();
        Circle {
            center: t.transform_point3(self.center),
            normal,
            radius: self.radius * along,
        }
    }
}

/// Checks a note's text: 1 to `MAX_NOTE_CHARS` characters, not blank, at most
/// `MAX_NOTE_LINES` lines.
pub fn check_note_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("a note's text must not be blank".into());
    }
    if text.chars().count() > MAX_NOTE_CHARS {
        return Err(format!(
            "a note's text must be at most {MAX_NOTE_CHARS} characters"
        ));
    }
    if text.lines().count() > MAX_NOTE_LINES {
        return Err(format!(
            "a note's text must be at most {MAX_NOTE_LINES} lines"
        ));
    }
    Ok(())
}

/// True when a radial dimension may measure a curve of this kind.
fn measurable(kind: CurveKind) -> bool {
    matches!(kind, CurveKind::Circle | CurveKind::Arc)
}

impl Model {
    pub fn dimension(&self, id: DimensionId) -> Option<&Dimension> {
        self.dimensions.get(&id)
    }

    pub fn dimensions(&self) -> impl Iterator<Item = (DimensionId, &Dimension)> {
        self.dimensions.iter().map(|(k, v)| (*k, v))
    }

    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.get(&id)
    }

    pub fn notes(&self) -> impl Iterator<Item = (NoteId, &Note)> {
        self.notes.iter().map(|(k, v)| (*k, v))
    }

    /// Where an anchor is now; None for a missing vertex.
    pub fn anchor_point(&self, anchor: &Anchor) -> Option<DVec3> {
        match anchor {
            Anchor::Vertex(v) => self.position(*v),
            Anchor::Point(p) => Some(*p),
        }
    }

    /// The two measured points of a linear dimension.
    pub fn dimension_points(&self, id: DimensionId) -> Option<(DVec3, DVec3)> {
        match self.dimension(id)? {
            Dimension::Linear { a, b, .. } => Some((self.anchor_point(a)?, self.anchor_point(b)?)),
            Dimension::Radial { .. } => None,
        }
    }

    /// The dimension line of a linear dimension: its measured points moved by the part of
    /// its offset perpendicular to them.
    pub fn dimension_line(&self, id: DimensionId) -> Option<(DVec3, DVec3)> {
        let (a, b) = self.dimension_points(id)?;
        let Dimension::Linear { offset, .. } = self.dimension(id)? else {
            return None;
        };
        let shift = perpendicular(*offset, b - a);
        Some((a + shift, b + shift))
    }

    /// The circle a radial dimension measures now.
    pub fn radial_circle(&self, id: DimensionId) -> Option<Circle> {
        match self.dimension(id)? {
            Dimension::Radial { curve, .. } => self.circle_of(curve),
            Dimension::Linear { .. } => None,
        }
    }

    /// The point a leader note points at now.
    pub fn note_point(&self, id: NoteId) -> Option<DVec3> {
        match self.note(id)? {
            Note::Leader { anchor, .. } => self.anchor_point(anchor),
            Note::Screen { .. } => None,
        }
    }

    /// The circle of a radial anchor: a fixed one, or the curve's while it is a circle or an
    /// arc.
    fn circle_of(&self, anchor: &RadialAnchor) -> Option<Circle> {
        match *anchor {
            RadialAnchor::Fixed {
                center,
                normal,
                radius,
            } => Some(Circle {
                center,
                normal,
                radius,
            }),
            RadialAnchor::Curve(c) => {
                let curve = self.curve(c)?;
                if !measurable(curve.kind()) {
                    return None;
                }
                let circle = curve.circle?;
                Some(Circle {
                    center: circle.center,
                    normal: circle.normal,
                    radius: circle.radius,
                })
            }
        }
    }

    /// Checks an anchor of a new annotation: an existing vertex or a finite point.
    fn check_anchor(&self, anchor: &Anchor) -> Result<DVec3, KernelError> {
        match anchor {
            Anchor::Vertex(v) => self.position(*v).ok_or_else(|| {
                KernelError::Annotation(format!("no such vertex {}", EntityId::Vertex(*v)))
            }),
            Anchor::Point(p) if p.is_finite() => Ok(*p),
            Anchor::Point(_) => Err(KernelError::NotFinite),
        }
    }

    /// Adds a dimension as one ChangeSet. A linear dimension's ends must be existing vertices
    /// or finite points at least `LENGTH_TOL` apart (else `ZeroDistance`); its offset is kept
    /// perpendicular to them. A radial dimension's curve must be a circle or an arc, and a
    /// fixed circle needs a positive radius and a non-zero normal (kept as a unit vector).
    pub fn add_dimension(&mut self, dimension: Dimension) -> Result<ChangeSet, KernelError> {
        let dimension = match dimension {
            Dimension::Linear { a, b, offset, tag } => {
                let (pa, pb) = (self.check_anchor(&a)?, self.check_anchor(&b)?);
                if !offset.is_finite() {
                    return Err(KernelError::NotFinite);
                }
                if pa.distance(pb) < LENGTH_TOL {
                    return Err(KernelError::ZeroDistance);
                }
                Dimension::Linear {
                    a,
                    b,
                    offset: perpendicular(offset, pb - pa),
                    tag,
                }
            }
            Dimension::Radial {
                curve,
                diameter,
                leader,
                tag,
            } => {
                if !leader.is_finite() {
                    return Err(KernelError::NotFinite);
                }
                let curve = match curve {
                    RadialAnchor::Curve(c) => {
                        if self.circle_of(&curve).is_none() {
                            return Err(KernelError::Annotation(format!(
                                "curve {c} is not a circle or an arc"
                            )));
                        }
                        RadialAnchor::Curve(c)
                    }
                    RadialAnchor::Fixed {
                        center,
                        normal,
                        radius,
                    } => {
                        if !center.is_finite() || !radius.is_finite() {
                            return Err(KernelError::NotFinite);
                        }
                        let normal = normal.try_normalize().ok_or(KernelError::NotFinite)?;
                        if radius < LENGTH_TOL {
                            return Err(KernelError::ZeroDistance);
                        }
                        RadialAnchor::Fixed {
                            center,
                            normal,
                            radius,
                        }
                    }
                };
                Dimension::Radial {
                    curve,
                    diameter,
                    leader,
                    tag,
                }
            }
        };
        self.insert_dimension(dimension);
        Ok(self.take_changes())
    }

    /// Adds a note as one ChangeSet: a leader's anchor must be an existing vertex or a finite
    /// point and its position finite; a screen note's place must be within the view (0 to
    /// 1); text must pass `check_note_text`. Only `Scene::add_note` adds screen notes, to the
    /// root.
    pub(crate) fn add_note(&mut self, note: Note) -> Result<ChangeSet, KernelError> {
        check_note(self, &note)?;
        self.insert_note(note);
        Ok(self.take_changes())
    }

    /// Changes a note's text as one ChangeSet (None: a leader note's automatic text). A screen
    /// note needs text. Unchanged text changes nothing.
    pub fn set_note_text(
        &mut self,
        id: NoteId,
        text: Option<String>,
    ) -> Result<ChangeSet, KernelError> {
        let note = self.note(id).cloned().ok_or_else(|| {
            KernelError::Annotation(format!("no such note {}", EntityId::Note(id)))
        })?;
        if let Some(t) = &text {
            check_note_text(t).map_err(KernelError::Annotation)?;
        }
        let changed = match (note.clone(), text) {
            (
                Note::Leader {
                    anchor,
                    position,
                    tag,
                    ..
                },
                text,
            ) => Note::Leader {
                anchor,
                position,
                text,
                tag,
            },
            (Note::Screen { at, tag, .. }, Some(text)) => Note::Screen { at, text, tag },
            (Note::Screen { .. }, None) => {
                return Err(KernelError::Annotation("a screen note needs text".into()));
            }
        };
        if changed != note {
            self.remove_note(id);
            self.insert_note_as(id, changed);
        }
        Ok(self.take_changes())
    }

    /// Removes dimensions and notes as one ChangeSet; unknown ids are skipped.
    pub fn erase_annotations(&mut self, dimensions: &[DimensionId], notes: &[NoteId]) -> ChangeSet {
        self.erase_annotations_pending(dimensions, notes);
        self.take_changes()
    }

    /// `erase_annotations` without taking the changes.
    pub(crate) fn erase_annotations_pending(
        &mut self,
        dimensions: &[DimensionId],
        notes: &[NoteId],
    ) {
        for id in dimensions.iter().collect::<BTreeSet<_>>() {
            if self.dimension(*id).is_some() {
                self.remove_dimension(*id);
            }
        }
        for id in notes.iter().collect::<BTreeSet<_>>() {
            if self.note(*id).is_some() {
                self.remove_note(*id);
            }
        }
    }

    /// Moves dimensions and notes by the affine map `t` as one ChangeSet, keeping their ids
    /// (each is reported as modified): point anchors, fixed circles, offsets, leaders and
    /// note positions move; vertex anchors and curves stay with their geometry, and screen
    /// notes stay where they are on the view. Unknown ids are skipped.
    pub fn transform_annotations(
        &mut self,
        dimensions: &[DimensionId],
        notes: &[NoteId],
        t: DAffine3,
    ) -> ChangeSet {
        for id in dimensions.iter().collect::<BTreeSet<_>>() {
            if let Some(d) = self.dimension(*id).copied() {
                let moved = d.transformed(t);
                if moved != d {
                    self.remove_dimension(*id);
                    self.insert_dimension_as(*id, moved);
                }
            }
        }
        for id in notes.iter().collect::<BTreeSet<_>>() {
            if let Some(n) = self.note(*id).cloned() {
                let moved = n.transformed(t);
                if moved != n {
                    self.remove_note(*id);
                    self.insert_note_as(*id, moved);
                }
            }
        }
        self.take_changes()
    }

    /// Adds a copy of each dimension and note per rigid motion in `places`, with new ids, as
    /// one ChangeSet. An anchor on a vertex in `copied` (the vertices of the geometry copied
    /// with them, see `vertices_of`) anchors to that vertex's copy, found where the copy
    /// put it; any other vertex anchor becomes a point. A radial dimension whose curve was
    /// copied measures the copy; otherwise it keeps the circle fixed. Unknown ids are skipped.
    pub fn copy_annotations(
        &mut self,
        dimensions: &[DimensionId],
        notes: &[NoteId],
        places: &[DAffine3],
        copied: &BTreeSet<VertexId>,
    ) -> ChangeSet {
        for t in places {
            let lifted = self.lift(dimensions, notes, *t, copied);
            self.land(lifted, false);
        }
        self.take_changes()
    }

    /// The vertices of `edges` and of `faces`' loops: what a copy, group or clip of them
    /// takes along.
    pub fn vertices_of(&self, edges: &[EdgeId], faces: &[FaceId]) -> BTreeSet<VertexId> {
        let mut out: BTreeSet<VertexId> = edges
            .iter()
            .filter_map(|e| self.edge(*e))
            .flat_map(|e| e.v)
            .collect();
        for f in faces.iter().filter_map(|f| self.face(*f)) {
            out.extend(std::iter::once(&f.outer).chain(&f.holes).flatten());
        }
        out
    }

    /// Every vertex of the mesh.
    pub(crate) fn all_vertices(&self) -> BTreeSet<VertexId> {
        self.vertices().map(|(id, _)| id).collect()
    }

    /// Every dimension and note of the mesh.
    pub(crate) fn all_annotations(&self) -> (Vec<DimensionId>, Vec<NoteId>) {
        (
            self.dimensions.keys().copied().collect(),
            self.notes.keys().copied().collect(),
        )
    }

    /// Takes dimensions and notes out (without changing anything) placed by `t`, to be put
    /// into a mesh by `land` once the geometry is drawn there. Vertex anchors become points;
    /// those on a vertex in `copied` are attached again to the vertex found there. Curves
    /// become fixed circles; a curve whose vertices are all in `copied` is looked up again.
    /// Unknown ids, and radial dimensions whose curve has no circle, are skipped.
    pub(crate) fn lift(
        &self,
        dimensions: &[DimensionId],
        notes: &[NoteId],
        t: DAffine3,
        copied: &BTreeSet<VertexId>,
    ) -> Vec<Lifted> {
        let mut out = Vec::new();
        let point = |a: &Anchor| -> Option<(Anchor, bool)> {
            match a {
                Anchor::Vertex(v) => Some((Anchor::Point(self.position(*v)?), copied.contains(v))),
                Anchor::Point(p) => Some((Anchor::Point(*p), false)),
            }
        };
        for id in dimensions.iter().collect::<BTreeSet<_>>() {
            let Some(d) = self.dimension(*id) else {
                continue;
            };
            let lifted = match *d {
                Dimension::Linear { a, b, offset, tag } => {
                    let (Some((a, ta)), Some((b, tb))) = (point(&a), point(&b)) else {
                        continue;
                    };
                    Lifted {
                        item: Item::Dimension(*id, Dimension::Linear { a, b, offset, tag }),
                        ties: Ties {
                            a: ta,
                            b: tb,
                            curve: None,
                        },
                    }
                }
                Dimension::Radial {
                    curve,
                    diameter,
                    leader,
                    tag,
                } => {
                    let Some(circle) = self.circle_of(&curve) else {
                        continue;
                    };
                    let tie = match curve {
                        RadialAnchor::Curve(c) => self.curve(c).and_then(|c| {
                            let all = c.vertices.iter().all(|v| copied.contains(v));
                            let first = self.edge(*c.edges.first()?)?;
                            all.then(|| (self.pos(first.v[0]), self.pos(first.v[1])))
                        }),
                        RadialAnchor::Fixed { .. } => None,
                    };
                    Lifted {
                        item: Item::Dimension(
                            *id,
                            Dimension::Radial {
                                curve: RadialAnchor::Fixed {
                                    center: circle.center,
                                    normal: circle.normal,
                                    radius: circle.radius,
                                },
                                diameter,
                                leader,
                                tag,
                            },
                        ),
                        ties: Ties {
                            a: false,
                            b: false,
                            curve: tie,
                        },
                    }
                }
            };
            out.push(lifted.transformed(t));
        }
        for id in notes.iter().collect::<BTreeSet<_>>() {
            let Some(n) = self.note(*id) else { continue };
            let lifted = match n.clone() {
                Note::Leader {
                    anchor,
                    position,
                    text,
                    tag,
                } => {
                    let Some((anchor, tie)) = point(&anchor) else {
                        continue;
                    };
                    Lifted {
                        item: Item::Note(
                            *id,
                            Note::Leader {
                                anchor,
                                position,
                                text,
                                tag,
                            },
                        ),
                        ties: Ties {
                            a: tie,
                            b: false,
                            curve: None,
                        },
                    }
                }
                n @ Note::Screen { .. } => Lifted {
                    item: Item::Note(*id, n),
                    ties: Ties::default(),
                },
            };
            out.push(lifted.transformed(t));
        }
        out
    }

    /// Puts lifted annotations into this mesh (see `lift`), under their old ids with
    /// `keep_ids`, else under new ones. Returns the ids they got.
    pub(crate) fn land(&mut self, lifted: Vec<Lifted>, keep_ids: bool) -> Vec<EntityId> {
        let mut out = Vec::new();
        for l in lifted {
            let attach = |m: &Model, a: Anchor, tie: bool| match a {
                Anchor::Point(p) if tie => m.find_vertex(p).map_or(a, Anchor::Vertex),
                a => a,
            };
            match l.item {
                Item::Dimension(id, d) => {
                    let d = match d {
                        Dimension::Linear { a, b, offset, tag } => {
                            let a = attach(self, a, l.ties.a);
                            // Two ends never anchor to one vertex: the second stays a point.
                            let attached = attach(self, b, l.ties.b);
                            let b = if a.vertex().is_some() && a == attached {
                                b
                            } else {
                                attached
                            };
                            Dimension::Linear { a, b, offset, tag }
                        }
                        Dimension::Radial {
                            curve,
                            diameter,
                            leader,
                            tag,
                        } => {
                            let found = l.ties.curve.and_then(|(p, q)| {
                                let e =
                                    self.edge_between(self.find_vertex(p)?, self.find_vertex(q)?)?;
                                let c = self.edge(e)?.curve?;
                                let curve = RadialAnchor::Curve(c);
                                self.circle_of(&curve).map(|_| curve)
                            });
                            Dimension::Radial {
                                curve: found.unwrap_or(curve),
                                diameter,
                                leader,
                                tag,
                            }
                        }
                    };
                    let id = if keep_ids {
                        self.insert_dimension_as(id, d);
                        id
                    } else {
                        self.insert_dimension(d)
                    };
                    out.push(EntityId::Dimension(id));
                }
                Item::Note(id, n) => {
                    let n = match n {
                        Note::Leader {
                            anchor,
                            position,
                            text,
                            tag,
                        } => Note::Leader {
                            anchor: attach(self, anchor, l.ties.a),
                            position,
                            text,
                            tag,
                        },
                        n => n,
                    };
                    let id = if keep_ids {
                        self.insert_note_as(id, n);
                        id
                    } else {
                        self.insert_note(n)
                    };
                    out.push(EntityId::Note(id));
                }
            }
        }
        out
    }

    /// Freezes the anchors the pending edit cut loose, recording the change with the edit: a
    /// vertex anchor whose vertex is gone becomes a point where the vertex last was, and a
    /// radial dimension whose curve is gone or no longer a circle or an arc keeps the circle
    /// it had before the edit (one whose curve had none then either goes). Runs from
    /// `upkeep`, after the curves are mended.
    pub(crate) fn freeze_annotations(&mut self) {
        if self.dimensions.is_empty() && self.notes.is_empty() {
            return;
        }
        // Where each vertex the edit removed last was (one added back again is not gone).
        let mut gone: BTreeMap<VertexId, DVec3> = BTreeMap::new();
        let mut geometry = false;
        for op in self.pending() {
            match op {
                Op::RemoveVertex(id, v) => {
                    gone.insert(*id, v.pos);
                    geometry = true;
                }
                Op::AddVertex(id, _) => {
                    gone.remove(id);
                    geometry = true;
                }
                Op::AddEdge(..) | Op::RemoveEdge(..) | Op::ChangeEdge(..) | Op::MoveVertex(..) => {
                    geometry = true;
                }
                _ => {}
            }
        }
        if !geometry {
            return;
        }
        let freeze = |m: &Model, a: Anchor| match a {
            Anchor::Vertex(v) if m.position(v).is_none() => gone.get(&v).map(|p| Anchor::Point(*p)),
            _ => None,
        };
        let mut dimensions: Vec<(DimensionId, Option<Dimension>)> = Vec::new();
        let mut lost_curves: Vec<(DimensionId, CurveId)> = Vec::new();
        for (id, d) in &self.dimensions {
            match *d {
                Dimension::Linear { a, b, offset, tag } => {
                    let (fa, fb) = (freeze(self, a), freeze(self, b));
                    if fa.is_some() || fb.is_some() {
                        dimensions.push((
                            *id,
                            Some(Dimension::Linear {
                                a: fa.unwrap_or(a),
                                b: fb.unwrap_or(b),
                                offset,
                                tag,
                            }),
                        ));
                    }
                }
                Dimension::Radial {
                    curve: curve @ RadialAnchor::Curve(c),
                    ..
                } => {
                    if self.circle_of(&curve).is_none() {
                        lost_curves.push((*id, c));
                    }
                }
                Dimension::Radial { .. } => {}
            }
        }
        if !lost_curves.is_empty() {
            let curves: BTreeSet<CurveId> = lost_curves.iter().map(|(_, c)| *c).collect();
            let before = self.circles_before(&curves);
            for (id, c) in lost_curves {
                let Dimension::Radial {
                    diameter,
                    leader,
                    tag,
                    ..
                } = self.dimensions[&id]
                else {
                    continue;
                };
                let frozen = before.get(&c).map(|circle| Dimension::Radial {
                    curve: RadialAnchor::Fixed {
                        center: circle.center,
                        normal: circle.normal,
                        radius: circle.radius,
                    },
                    diameter,
                    leader,
                    tag,
                });
                dimensions.push((id, frozen));
            }
        }
        let mut notes: Vec<(NoteId, Note)> = Vec::new();
        for (id, n) in &self.notes {
            if let Note::Leader {
                anchor,
                position,
                text,
                tag,
            } = n
                && let Some(anchor) = freeze(self, *anchor)
            {
                notes.push((
                    *id,
                    Note::Leader {
                        anchor,
                        position: *position,
                        text: text.clone(),
                        tag: *tag,
                    },
                ));
            }
        }
        for (id, d) in dimensions {
            self.remove_dimension(id);
            if let Some(d) = d {
                self.insert_dimension_as(id, d);
            }
        }
        for (id, n) in notes {
            self.remove_note(id);
            self.insert_note_as(id, n);
        }
    }

    /// The circles `curves` had before the pending edit (those that were a circle or an arc
    /// then): the edit is undone, looked at and done again, without recording anything.
    fn circles_before(&mut self, curves: &BTreeSet<CurveId>) -> BTreeMap<CurveId, Circle> {
        let ops = self.pending().to_vec();
        for op in ops.iter().rev() {
            self.apply_op(op.inverse());
        }
        let circles = curves
            .iter()
            .filter_map(|c| Some((*c, self.circle_of(&RadialAnchor::Curve(*c))?)))
            .collect();
        for op in ops {
            self.apply_op(op);
        }
        circles
    }

    /// Checks the annotations: vertex anchors on existing vertices, a linear dimension's ends
    /// not on one vertex, finite numbers, curves that are circles or arcs, fixed circles with
    /// a unit normal and a positive radius, screen notes within the view, and valid text.
    pub(crate) fn validate_annotations(&self) -> Result<(), String> {
        let missing = |a: &Anchor| a.vertex().filter(|v| self.position(*v).is_none());
        for (id, d) in &self.dimensions {
            let name = EntityId::Dimension(*id);
            if let Dimension::Linear { a, b, .. } = d
                && let Some(v) = missing(a).or(missing(b))
            {
                return Err(format!(
                    "{name} is anchored to vertex {}, which is missing",
                    EntityId::Vertex(v)
                ));
            }
            let ok = match d {
                Dimension::Linear { a, b, offset, .. } => {
                    if a.vertex().is_some() && a == b {
                        return Err(format!("{name} measures from a vertex to itself"));
                    }
                    self.anchor_is_valid(a) && self.anchor_is_valid(b) && offset.is_finite()
                }
                Dimension::Radial { curve, leader, .. } => {
                    if let RadialAnchor::Curve(c) = curve
                        && self.circle_of(curve).is_none()
                    {
                        return Err(format!(
                            "{name} measures curve {c}, which is not a circle or an arc"
                        ));
                    }
                    leader.is_finite()
                        && match curve {
                            RadialAnchor::Curve(_) => true,
                            RadialAnchor::Fixed {
                                center,
                                normal,
                                radius,
                            } => {
                                center.is_finite()
                                    && normal.is_finite()
                                    && (normal.length() - 1.0).abs() <= 1e-9
                                    && radius.is_finite()
                                    && *radius > 0.0
                            }
                        }
                }
            };
            if !ok {
                return Err(format!(
                    "{name} has a number that is not finite or an invalid circle"
                ));
            }
        }
        for (id, n) in &self.notes {
            let name = EntityId::Note(*id);
            if let Note::Leader { anchor, .. } = n
                && let Some(v) = missing(anchor)
            {
                return Err(format!(
                    "{name} is anchored to vertex {}, which is missing",
                    EntityId::Vertex(v)
                ));
            }
            check_note(self, n).map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(())
    }

    fn anchor_is_valid(&self, anchor: &Anchor) -> bool {
        match anchor {
            Anchor::Vertex(v) => self.position(*v).is_some(),
            Anchor::Point(p) => p.is_finite(),
        }
    }
}

/// Checks a note: a leader anchored to an existing vertex or a finite point, finite
/// positions, a screen note within the view, and valid text.
fn check_note(m: &Model, note: &Note) -> Result<(), KernelError> {
    match note {
        Note::Leader {
            anchor,
            position,
            text,
            ..
        } => {
            m.check_anchor(anchor)?;
            if !position.is_finite() {
                return Err(KernelError::NotFinite);
            }
            if let Some(t) = text {
                check_note_text(t).map_err(KernelError::Annotation)?;
            }
        }
        Note::Screen { at, text, .. } => {
            if !at.iter().all(|x| x.is_finite()) {
                return Err(KernelError::NotFinite);
            }
            if !at.iter().all(|x| (0.0..=1.0).contains(x)) {
                return Err(KernelError::Annotation(
                    "a screen note's place must be within the view (0 to 1)".into(),
                ));
            }
            check_note_text(text).map_err(KernelError::Annotation)?;
        }
    }
    Ok(())
}

/// The part of `v` perpendicular to `along` (all of it when `along` has no direction).
fn perpendicular(v: DVec3, along: DVec3) -> DVec3 {
    match along.try_normalize() {
        Some(u) => v - u * v.dot(u),
        None => v,
    }
}

/// An annotation taken out of a mesh by `Model::lift`.
pub(crate) struct Lifted {
    item: Item,
    ties: Ties,
}

enum Item {
    Dimension(DimensionId, Dimension),
    Note(NoteId, Note),
}

/// Which point anchors to attach again to a vertex found at them, and the ends of one edge of
/// a curve to look up again.
#[derive(Default)]
struct Ties {
    a: bool,
    b: bool,
    curve: Option<(DVec3, DVec3)>,
}

impl Lifted {
    fn transformed(self, t: DAffine3) -> Lifted {
        Lifted {
            item: match self.item {
                Item::Dimension(id, d) => Item::Dimension(id, d.transformed(t)),
                Item::Note(id, n) => Item::Note(id, n.transformed(t)),
            },
            ties: Ties {
                curve: self
                    .ties
                    .curve
                    .map(|(p, q)| (t.transform_point3(p), t.transform_point3(q))),
                ..self.ties
            },
        }
    }

    /// The same annotation on the tag `retag` gives for its tag.
    pub(crate) fn retagged(self, retag: impl Fn(Option<TagId>) -> Option<TagId>) -> Lifted {
        Lifted {
            item: match self.item {
                Item::Dimension(id, d) => Item::Dimension(id, d.with_tag(retag(d.tag()))),
                Item::Note(id, n) => {
                    let tag = retag(n.tag());
                    Item::Note(id, n.with_tag(tag))
                }
            },
            ties: self.ties,
        }
    }

    /// True for a screen note (only the root mesh holds them).
    pub(crate) fn is_screen_note(&self) -> bool {
        matches!(self.item, Item::Note(_, Note::Screen { .. }))
    }
}

impl Scene {
    /// Adds a note to mesh `mesh` as one change (see `Model::add_note`); a screen note goes
    /// only into the root (else `Annotation`). A missing mesh is `DefinitionNotFound`.
    pub fn add_note(&mut self, mesh: MeshId, note: Note) -> Result<SceneChange, KernelError> {
        if self.mesh(mesh).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        if matches!(note, Note::Screen { .. }) && mesh != MeshId::Root {
            return Err(KernelError::Annotation(
                "screen notes go only into the model itself, not into a group or component".into(),
            ));
        }
        self.edit(mesh, |m| m.add_note(note))
    }
}
