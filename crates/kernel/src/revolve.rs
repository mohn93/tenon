//! Revolving a face about an axis (the lathe): the face's loops are turned step by step and
//! each side sweeps a ring of faces. Where a side touches the axis its faces collapse to
//! triangles (or to nothing when it lies along the axis), and a side square to the axis sweeps
//! one flat face (a disk or a ring) instead of a fan of coplanar pieces.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use glam::{DAffine3, DQuat, DVec2, DVec3};

use crate::geom::{LENGTH_TOL, Plane, point_in_polygon, vector_area};
use crate::ids::{CurveId, EntityId, FaceId, MaterialId, TagId};
use crate::model::{ChangeSet, Model};
use crate::primitives::{MIN_LENGTH, Solid};

/// Most steps a revolve may be cut into.
pub const MAX_STEPS: u32 = 360;

/// A refusal of a revolve; the text says why.
#[derive(Clone, Debug, PartialEq)]
pub struct RevolveError(pub String);

impl fmt::Display for RevolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RevolveError {}

fn invalid<T>(message: impl Into<String>) -> Result<T, RevolveError> {
    Err(RevolveError(message.into()))
}

/// A revolve worked out for a model by `Model::plan_revolve`, drawn by `Model::revolve`.
#[derive(Clone, Debug)]
pub struct Revolve {
    solid: Solid,
    face: FaceId,
    /// A full turn: the face is removed. Otherwise it stays as the start cap.
    full: bool,
    /// Where the face is: its loops, outer first.
    profile: Vec<Vec<DVec3>>,
    /// The direction out of the solid at the start cap (a partial turn).
    start_out: DVec3,
    /// The end cap's loops and its outward normal (a partial turn).
    end: Vec<Vec<DVec3>>,
    end_normal: DVec3,
    /// What the end cap takes from the face.
    paint: [Option<MaterialId>; 2],
    tag: Option<TagId>,
    hidden: bool,
}

impl Revolve {
    /// Every point the revolve places, to be checked against the model's limits.
    pub fn points(&self) -> &[DVec3] {
        self.solid.points()
    }
}

/// The steps a turn of `angle` degrees is cut into when a full turn has `segments`.
pub fn steps_for(angle: f64, segments: u32) -> u32 {
    ((f64::from(segments) * angle / 360.0 - 1e-9).ceil() as u32).max(1)
}

/// True when the points lie in one plane within `tol`.
fn planar(points: &[DVec3], tol: f64) -> bool {
    let normal = vector_area(points).normalize_or_zero();
    let centre = points.iter().sum::<DVec3>() / points.len() as f64;
    points.iter().all(|p| (*p - centre).dot(normal).abs() < tol)
}

/// True when every face of `solid` is flat within `LENGTH_TOL` and every side of every loop is
/// shared by exactly two loops.
fn solid_is_closed_and_flat(solid: &Solid) -> bool {
    let mut sides: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for f in &solid.faces {
        let loops = std::iter::once(&f.outer).chain(&f.holes);
        let mut all: Vec<DVec3> = Vec::new();
        for l in loops {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                *sides.entry((a.min(b), a.max(b))).or_default() += 1;
            }
            all.extend(l.iter().map(|i| solid.points()[*i]));
        }
        let outer: Vec<DVec3> = f.outer.iter().map(|i| solid.points()[*i]).collect();
        let normal = vector_area(&outer).normalize_or_zero();
        if normal == DVec3::ZERO
            || !all
                .iter()
                .all(|p| (*p - outer[0]).dot(normal).abs() < LENGTH_TOL)
        {
            return false;
        }
    }
    sides.values().all(|n| *n == 2)
}

/// `idx` without consecutive repeats, also across the end.
fn dedup(mut idx: Vec<usize>) -> Vec<usize> {
    idx.dedup();
    while idx.len() > 1 && idx.first() == idx.last() {
        idx.pop();
    }
    idx
}

impl Model {
    /// Works out the revolve of face `face` about the line through `center` along `axis` by
    /// `angle` degrees (counter-clockwise looking down the axis, 360 for a full turn), cut
    /// into `steps_for(angle, segments)` steps, without changing the model.
    ///
    /// The face may touch the axis, and a plane that holds the axis or lies beside it is fine,
    /// but it must not cross it: the face's plane must not be square to the axis, and no
    /// two points of the face may lie on one circle about the axis in one plane square to it
    /// (equivalently, seen from the axis along the plane's horizontal, the face is wholly on
    /// one side of the plane through the axis square to that horizontal; touching it is fine).
    pub fn plan_revolve(
        &self,
        face: FaceId,
        center: DVec3,
        axis: DVec3,
        angle: f64,
        segments: u32,
    ) -> Result<Revolve, RevolveError> {
        let Some(f) = self.face(face) else {
            return invalid("face does not exist");
        };
        if !center.is_finite() {
            return invalid("center must be finite");
        }
        let Some(axis) = axis.try_normalize().filter(|a| a.is_finite()) else {
            return invalid("axis must be a finite, non-zero vector");
        };
        if !angle.is_finite() || angle <= 0.0 || angle > 360.0 + 1e-9 {
            return invalid("angle must be above 0 and at most 360 degrees");
        }
        if !(1..=MAX_STEPS).contains(&segments) {
            return invalid(format!("segments must be between 1 and {MAX_STEPS}"));
        }
        let full = angle >= 360.0 - 1e-9;
        let angle = if full { 360.0 } else { angle };
        let count = steps_for(angle, segments);
        if count > MAX_STEPS {
            return invalid(format!("the turn would take more than {MAX_STEPS} steps"));
        }
        let step = angle.to_radians() / f64::from(count);
        if step >= std::f64::consts::PI - 1e-9 {
            return invalid("segments is too small: a step must turn by less than 180 degrees");
        }

        if f.normal.dot(axis).abs() > 1.0 - 1e-9 {
            return invalid(
                "the face is square to the axis: revolving it sweeps no solid (use push_pull)",
            );
        }
        let loops: Vec<Vec<DVec3>> = std::iter::once(&f.outer)
            .chain(&f.holes)
            .map(|l| l.iter().map(|v| self.pos(*v)).collect())
            .collect();
        // Seen along the plane's horizontal, the face must lie on one side of the axis.
        let across = f.normal.cross(axis).normalize();
        let (lo, hi) = loops[0].iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            let t = (*p - center).dot(across);
            (lo.min(t), hi.max(t))
        });
        if lo < -LENGTH_TOL && hi > LENGTH_TOL {
            return invalid(
                "the face crosses the axis (or lies across where the axis falls on its plane); it must lie on one side of the axis",
            );
        }
        let radius = |p: DVec3| {
            let d = p - center;
            (d - axis * d.dot(axis)).length()
        };
        // On the axis (collapsing to a point), or far enough from it for every step.
        let least = 2.0 * (step / 2.0).sin();
        let on_axis: Vec<Vec<bool>> = loops
            .iter()
            .map(|l| l.iter().map(|p| radius(*p) < LENGTH_TOL).collect())
            .collect();
        for (l, flags) in loops.iter().zip(&on_axis) {
            for (p, on) in l.iter().zip(flags) {
                if !on && radius(*p) * least < MIN_LENGTH {
                    return invalid(
                        "a point of the face is too close to the axis for that many segments",
                    );
                }
            }
        }

        // Points: per loop vertex one per section (a full turn's last section is its first).
        let sections = if full { count } else { count + 1 } as usize;
        let rotate = |k: usize, p: DVec3| -> DVec3 {
            if k == 0 {
                return p;
            }
            let angle = if !full && k == count as usize {
                angle.to_radians()
            } else {
                step * k as f64
            };
            center + DQuat::from_axis_angle(axis, angle) * (p - center)
        };
        let mut solid = Solid::default();
        let idx: Vec<Vec<Vec<usize>>> = loops
            .iter()
            .zip(&on_axis)
            .map(|(l, flags)| {
                l.iter()
                    .zip(flags)
                    .map(|(p, on)| {
                        if *on {
                            vec![solid.point(*p); sections]
                        } else {
                            (0..sections).map(|k| solid.point(rotate(k, *p))).collect()
                        }
                    })
                    .collect()
            })
            .collect();

        let curve_between = |a, b| {
            self.edge_between(a, b)
                .and_then(|e| self.edge(e))
                .and_then(|e| Some((e.curve?, e.polygon)))
        };
        let vertex_loops: Vec<&Vec<crate::ids::VertexId>> =
            std::iter::once(&f.outer).chain(&f.holes).collect();
        let curves: Vec<Vec<Option<(CurveId, bool)>>> = vertex_loops
            .iter()
            .map(|l| {
                (0..l.len())
                    .map(|j| curve_between(l[j], l[(j + 1) % l.len()]))
                    .collect()
            })
            .collect();

        let n = count as usize;
        for (l, ring) in idx.iter().enumerate() {
            let m = ring.len();
            for j in 0..m {
                let (p, q) = (&ring[j], &ring[(j + 1) % m]);
                let (pa, qa) = (on_axis[l][j], on_axis[l][(j + 1) % m]);
                if pa && qa {
                    continue;
                }
                let (pp, qp) = (loops[l][j], loops[l][(j + 1) % m]);
                if (qp - pp).dot(axis).abs() < LENGTH_TOL {
                    // Square to the axis: one flat face, a disk or a ring.
                    if full {
                        let up: Vec<usize> = q.clone();
                        let down: Vec<usize> = p.iter().rev().copied().collect();
                        if pa {
                            solid.face(up);
                        } else if qa {
                            solid.face(down);
                        } else if radius(qp) > radius(pp) {
                            solid.face_with_hole(up, p.clone());
                        } else {
                            solid.face_with_hole(down, q.clone());
                        }
                    } else {
                        let mut poly: Vec<usize> = if qa { vec![q[0]] } else { q.clone() };
                        if pa {
                            poly.push(p[0]);
                        } else {
                            poly.extend(p.iter().rev());
                        }
                        solid.face(poly);
                    }
                    continue;
                }
                for k in 0..n {
                    let k1 = (k + 1) % sections;
                    let quad = dedup(vec![p[k], q[k], q[k1], p[k1]]);
                    let pts: Vec<DVec3> = quad.iter().map(|i| solid.points()[*i]).collect();
                    if quad.len() == 4 && !planar(&pts, 0.1 * LENGTH_TOL) {
                        // A side skewed to the axis sweeps a twisted quad: two triangles
                        // along a soft diagonal, which reads as one surface.
                        solid.face(vec![quad[0], quad[1], quad[2]]);
                        solid.face(vec![quad[0], quad[2], quad[3]]);
                        solid.soften(quad[0], quad[2]);
                    } else {
                        solid.face(quad);
                    }
                    if full || k > 0 {
                        solid.soften(p[k], q[k]);
                    }
                }
            }
            // The path of each corner: soft where its two sides are on one smooth curve.
            for j in 0..m {
                if on_axis[l][j] {
                    continue;
                }
                let v = &ring[j];
                let before = curves[l][(j + m - 1) % m];
                if before.is_some_and(|(_, polygon)| !polygon) && before == curves[l][j] {
                    for k in 0..n {
                        solid.soften(v[k], v[(k + 1) % sections]);
                    }
                }
                if full {
                    solid.ring(v.clone(), false);
                }
            }
        }
        // The end cap, and the start cap that decides which way the solid faces.
        let end_loops: Vec<Vec<usize>> = idx
            .iter()
            .map(|l| (0..l.len()).map(|j| l[j][sections - 1]).collect())
            .collect();
        let end_points: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|p| rotate(sections - 1, *p)).collect())
            .collect();
        let mut end_normal = DVec3::ZERO;
        let mut start_out = DVec3::ZERO;
        if !full {
            solid.face_with_holes(end_loops[0].clone(), end_loops[1..].to_vec());
            let start: Vec<Vec<usize>> = idx
                .iter()
                .map(|l| (0..l.len()).map(|j| l[j][0]).collect())
                .collect();
            solid.face_with_holes(
                start[0].iter().rev().copied().collect(),
                start[1..].to_vec(),
            );
        }
        if solid.faces.is_empty() || solid.signed_volume().abs() < 1e-12 {
            return invalid("revolving the face makes no solid");
        }
        solid.face_outward();
        if !solid_is_closed_and_flat(&solid) {
            return invalid("revolving the face does not make a closed solid");
        }
        if !full {
            let last = solid.faces.len() - 1;
            start_out = solid.face_normal(last);
            end_normal = solid.face_normal(last - 1);
            // The start cap is the face itself.
            solid.faces.truncate(last);
        }
        Ok(Revolve {
            solid,
            face,
            full,
            profile: loops,
            start_out,
            end: end_points,
            end_normal,
            paint: f.paint(),
            tag: f.tag,
            hidden: f.hidden,
        })
    }

    /// Draws a revolve worked out by `plan_revolve`, as one ChangeSet, with the rules of
    /// `add_edge` (it merges with what it touches). A full turn removes the face and what is
    /// left of its edges along the axis; a partial turn keeps the face as the start cap and
    /// closes the end with a copy of it (its materials, tag and visibility, not its texture placement). Faces point out of
    /// the solid. Edges between consecutive faces swept by one side are soft, and so are those
    /// at a corner between two sides on one curve that is not a polygon.
    pub fn revolve(&mut self, plan: &Revolve) -> ChangeSet {
        let before: BTreeSet<FaceId> = self.faces().map(|(f, _)| f).collect();
        if plan.full && self.face(plan.face).is_some() {
            self.remove_face(plan.face);
        }
        let mut changes = self.draw_solid(&plan.solid, &DAffine3::IDENTITY, None);
        if plan.full {
            self.clear_profile(&plan.profile);
        } else {
            // The start cap faces out of the solid, away from the turn.
            if self.face(plan.face).is_some() {
                self.orient(plan.face, plan.start_out);
                // Turning it over swapped its sides' materials: front is the outside again.
                self.set_paint(plan.face, plan.paint);
            }
            let plane = Plane::through(plan.end[0][0], plan.end_normal);
            let flat: Vec<Vec<DVec2>> = plan
                .end
                .iter()
                .map(|l| l.iter().map(|p| plane.project(*p)).collect())
                .collect();
            let made: Vec<FaceId> = self
                .faces()
                .map(|(f, _)| f)
                .filter(|f| !before.contains(f))
                .collect();
            for f in made {
                let aligned = self.face(f).unwrap().normal.dot(plan.end_normal) > 1.0 - 1e-6;
                let Some(p) = self.interior_point(f) else {
                    continue;
                };
                let q = plane.project(p);
                if aligned
                    && plane.contains(p)
                    && point_in_polygon(q, &flat[0])
                    && !flat[1..].iter().any(|h| point_in_polygon(q, h))
                {
                    self.set_paint(f, plan.paint);
                    self.set_tag(EntityId::Face(f), plan.tag);
                    self.set_hidden(EntityId::Face(f), plan.hidden);
                }
            }
        }
        changes.ops.extend(self.take_changes().ops);
        changes
    }
}
