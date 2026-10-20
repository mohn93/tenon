//! Sweeping a face's loops along a path (the Follow Me tool).

use std::collections::{BTreeMap, BTreeSet};

use glam::{DQuat, DVec2, DVec3};

use crate::geom::{
    ANGLE_TOL, LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon, project_on_segment,
};
use crate::ids::{CurveId, EdgeId, EntityId, FaceId, MaterialId, VertexId};
use crate::model::{ChangeSet, Model, Op};

/// Why a sweep cannot be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepError {
    /// The profile is too big for a turn of the path: its sides would fold over there.
    Folds,
    /// The path turns straight back, or the profile lies along the path.
    Degenerate,
}

impl std::fmt::Display for SweepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SweepError::Folds => write!(f, "the profile is too big for a turn of the path"),
            SweepError::Degenerate => write!(
                f,
                "the path turns straight back, or the profile lies along the path"
            ),
        }
    }
}

impl std::error::Error for SweepError {}

/// The profile's loops placed along a path.
#[derive(Clone, Debug, PartialEq)]
pub struct Sweep {
    /// Per path vertex from the start, the profile's loops there (points in the profile's
    /// order). A closed path has one more: its first section again.
    pub sections: Vec<Vec<Vec<DVec3>>>,
    /// The ends are capped (the path is open).
    pub caps: bool,
}

/// The least length a swept side, or a section's side or width, may have.
pub const MIN_LENGTH: f64 = 10.0 * LENGTH_TOL;

/// Two path directions summing to less than this turn straight back.
const U_TURN: f64 = 1e-6;

/// Sweeps the profile (`profile[0]` its outer loop, counter-clockwise around `profile_normal`,
/// the rest its holes) along the path through `path` (closed: the last point joins the first),
/// starting at `path[0]`. The profile is carried as it is, not moved onto the path: segment k
/// carries it in a frame turned from the one before by the rotation taking the previous
/// direction to its own (the first frame is the identity). At an interior vertex the section is
/// the previous segment's profile projected along that segment onto the miter plane (normal the
/// sum of the two directions); an open path's ends are the profile projected onto the planes
/// square to the first and last segments. A closed path's frame that does not come back to
/// itself is twisted back evenly over the segments.
///
/// `Folds` when a profile point would not move at least `MIN_LENGTH` forward along a segment
/// between its two sections; `Degenerate` when the path turns straight back, has too few
/// points or a zero-length segment, or a section collapses: two neighbouring points closer than
/// `MIN_LENGTH`, or a loop narrower than that across (the profile lies along the path, or
/// nearly).
pub fn sweep(
    profile: &[Vec<DVec3>],
    profile_normal: DVec3,
    path: &[DVec3],
    closed: bool,
) -> Result<Sweep, SweepError> {
    let n = path.len();
    if profile.is_empty() || profile.iter().any(|l| l.len() < 3) || n < if closed { 3 } else { 2 } {
        return Err(SweepError::Degenerate);
    }
    let segs = if closed { n } else { n - 1 };
    let dirs: Vec<DVec3> = (0..segs)
        .map(|k| (path[(k + 1) % n] - path[k]).try_normalize())
        .collect::<Option<_>>()
        .ok_or(SweepError::Degenerate)?;
    // The miter normal at each joint (every vertex of a closed path, the inner ones of an
    // open one).
    let joints: Vec<usize> = if closed {
        (0..n).collect()
    } else {
        (1..n - 1).collect()
    };
    let mut miters = vec![DVec3::ZERO; n];
    for &i in &joints {
        let sum = dirs[(i + segs - 1) % segs] + dirs[i % segs];
        if sum.length() < U_TURN {
            return Err(SweepError::Degenerate);
        }
        miters[i] = sum.normalize();
    }
    if profile_normal.normalize_or_zero().dot(dirs[0]).abs() < 1e-9 {
        return Err(SweepError::Degenerate);
    }
    // Rotation-minimising frames: each segment's first axis turned on from the one before.
    let u0 = dirs[0].any_orthonormal_vector();
    let v0 = dirs[0].cross(u0);
    let mut us = vec![u0];
    for k in 1..segs {
        let u = DQuat::from_rotation_arc(dirs[k - 1], dirs[k]) * us[k - 1];
        us.push(u.reject_from_normalized(dirs[k]).normalize());
    }
    let mut twist = 0.0;
    if closed {
        let u = DQuat::from_rotation_arc(dirs[segs - 1], dirs[0]) * us[segs - 1];
        let phi = u.dot(v0).atan2(u.dot(u0));
        if phi.abs() > 1e-9 {
            twist = phi;
        }
    }
    let frames: Vec<(DVec3, DVec3)> = us
        .iter()
        .enumerate()
        .map(|(k, u)| {
            let u = DQuat::from_axis_angle(dirs[k], -twist * k as f64 / segs as f64) * *u;
            (u, dirs[k].cross(u))
        })
        .collect();
    // The profile's points across the first segment's line.
    let coords: Vec<Vec<DVec2>> = profile
        .iter()
        .map(|l| {
            l.iter()
                .map(|q| {
                    let r = *q - path[0];
                    DVec2::new(r.dot(u0), r.dot(v0))
                })
                .collect()
        })
        .collect();
    // The section at vertex i: segment k's profile projected along it onto the plane through
    // path[i] with normal m.
    let section = |i: usize, k: usize, m: DVec3| -> Vec<Vec<DVec3>> {
        let ((u, v), d) = (frames[k], dirs[k]);
        coords
            .iter()
            .map(|l| {
                l.iter()
                    .map(|c| {
                        let p = path[k] + u * c.x + v * c.y;
                        p + d * ((path[i] - p).dot(m) / d.dot(m))
                    })
                    .collect()
            })
            .collect()
    };
    let sections: Vec<Vec<Vec<DVec3>>> = if closed {
        // The section at vertex i comes from the segment arriving there; the first comes
        // again at the end.
        (0..=n)
            .map(|i| section(i % n, (i + segs - 1) % segs, miters[i % n]))
            .collect()
    } else {
        std::iter::once(section(0, 0, dirs[0]))
            .chain((1..n - 1).map(|i| section(i, i - 1, miters[i])))
            .chain(std::iter::once(section(n - 1, n - 2, dirs[n - 2])))
            .collect()
    };
    let collapsed = |l: &Vec<DVec3>| {
        (0..l.len()).any(|j| l[j].distance(l[(j + 1) % l.len()]) < MIN_LENGTH)
            || narrowest(l) < MIN_LENGTH
    };
    if sections.iter().flatten().any(collapsed) {
        return Err(SweepError::Degenerate);
    }
    for k in 0..segs {
        for (a, b) in sections[k].iter().zip(&sections[k + 1]) {
            if a.iter()
                .zip(b)
                .any(|(p, q)| (*q - *p).dot(dirs[k]) < MIN_LENGTH)
            {
                return Err(SweepError::Folds);
            }
        }
    }
    Ok(Sweep {
        sections,
        caps: !closed,
    })
}

/// A follow-me sweep worked out for a model by `Model::plan_follow_me`, drawn by
/// `Model::follow_me`.
#[derive(Clone, Debug)]
pub struct FollowMe {
    pub sweep: Sweep,
    face: FaceId,
    /// The path's points from where the sweep starts.
    path: Vec<DVec3>,
    /// The curve of each path segment in sweep order, with its polygon flag.
    path_curves: Vec<Option<(CurveId, bool)>>,
    /// The profile's loops (outer first) and the curve of each of their sides.
    profile: Vec<Vec<DVec3>>,
    side_curves: Vec<Vec<Option<(CurveId, bool)>>>,
    /// 1 when the profile's normal points along the path's first segment, -1 against it.
    turn: f64,
    /// The profile face's front and back materials, which the caps take.
    paint: [Option<MaterialId>; 2],
}

impl Model {
    /// Works out the sweep of face `face` along the path through vertices `path` (in chain
    /// order; closed: the last joins the first), without changing the model. A vertex in the
    /// middle of a straight run of the path is passed over: the run is one segment. The sweep
    /// starts at the path end nearest the face's plane (closed: the nearest path vertex, other
    /// than one where the path turns within that plane) and walks the path from there; ties go
    /// to the one nearer the centre of the face's outer loop, then to the first in chain order.
    /// A closed path is walked toward the neighbour that leaves the face's plane more steeply
    /// (forward in chain order on a tie). `Degenerate` also when the face or a vertex does not
    /// exist.
    pub fn plan_follow_me(
        &self,
        face: FaceId,
        path: &[VertexId],
        closed: bool,
    ) -> Result<FollowMe, SweepError> {
        let f = self.face(face).ok_or(SweepError::Degenerate)?;
        let all: Vec<DVec3> = path
            .iter()
            .map(|v| self.position(*v))
            .collect::<Option<_>>()
            .ok_or(SweepError::Degenerate)?;
        if all.len() < 2 {
            return Err(SweepError::Degenerate);
        }
        let curve_between = |a: VertexId, b: VertexId| {
            self.edge_between(a, b)
                .and_then(|e| self.edge(e))
                .and_then(|e| Some((e.curve?, e.polygon)))
        };
        // A straight run of the path is one segment; it keeps its edges' curve if they share
        // one.
        let keep = corners(&all, closed);
        if keep.len() < if closed { 3 } else { 2 } {
            return Err(SweepError::Degenerate);
        }
        let mut runs: BTreeMap<(VertexId, VertexId), Option<(CurveId, bool)>> = BTreeMap::new();
        for r in 0..if closed { keep.len() } else { keep.len() - 1 } {
            let (from, to) = (keep[r], keep[(r + 1) % keep.len()]);
            let mut i = from;
            let mut curve = None;
            loop {
                let j = (i + 1) % all.len();
                let c = curve_between(path[i], path[j]);
                curve = if i == from || curve == c { c } else { None };
                if j == to || curve.is_none() {
                    break;
                }
                i = j;
            }
            runs.insert((path[from], path[to]), curve);
            runs.insert((path[to], path[from]), curve);
        }
        let path: Vec<VertexId> = keep.iter().map(|i| path[*i]).collect();
        let points: Vec<DVec3> = keep.iter().map(|i| all[*i]).collect();
        let n = points.len();
        let loops: Vec<&Vec<VertexId>> = std::iter::once(&f.outer).chain(&f.holes).collect();
        let profile: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|v| self.pos(*v)).collect())
            .collect();
        let origin = profile[0][0];
        let centre = profile[0].iter().sum::<DVec3>() / profile[0].len() as f64;
        let off_plane = |i: usize| (points[i] - origin).dot(f.normal).abs();
        let mut candidates: Vec<usize> = if closed {
            (0..n).collect()
        } else {
            vec![0, n - 1]
        };
        if closed {
            // A vertex where the path turns within the face's plane cannot start the sweep.
            let in_plane = |i: usize| off_plane(i % n) < LENGTH_TOL;
            let leaving: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&i| !(in_plane(i) && in_plane(i + 1) && in_plane(i + n - 1)))
                .collect();
            if !leaving.is_empty() {
                candidates = leaving;
            }
        }
        let start = candidates
            .into_iter()
            .reduce(|a, b| {
                let (da, db) = (off_plane(a), off_plane(b));
                let nearer = points[b].distance(centre) < points[a].distance(centre) - LENGTH_TOL;
                if db < da - LENGTH_TOL || ((db - da).abs() <= LENGTH_TOL && nearer) {
                    b
                } else {
                    a
                }
            })
            .unwrap();
        let mut order: Vec<VertexId> = path.to_vec();
        if closed {
            order.rotate_left(start);
            // Walked the way that leaves the face's plane more steeply (ahead in chain order
            // on a tie), so a profile standing on a path edge does not set off along it.
            let across = |p: DVec3| (p - points[start]).normalize_or_zero().dot(f.normal).abs();
            if across(points[(start + n - 1) % n]) > across(points[(start + 1) % n]) + 1e-9 {
                order[1..].reverse();
            }
        } else if start != 0 {
            order.reverse();
        }
        let points: Vec<DVec3> = order.iter().map(|v| self.pos(*v)).collect();
        let segs = if closed { n } else { n - 1 };
        let path_curves = (0..segs)
            .map(|k| runs[&(order[k], order[(k + 1) % n])])
            .collect();
        let side_curves = loops
            .iter()
            .map(|l| {
                (0..l.len())
                    .map(|j| curve_between(l[j], l[(j + 1) % l.len()]))
                    .collect()
            })
            .collect();
        let sweep = sweep(&profile, f.normal, &points, closed)?;
        let turn = if f.normal.dot(points[1] - points[0]) > 0.0 {
            1.0
        } else {
            -1.0
        };
        Ok(FollowMe {
            sweep,
            face,
            path: points,
            path_curves,
            profile,
            side_curves,
            turn,
            paint: f.paint(),
        })
    }

    /// Draws a sweep worked out by `plan_follow_me`, as one ChangeSet: for every profile side
    /// and path segment the face between the side's two sections (two triangles with a soft
    /// diagonal where a closed path's twist bends it), and for an open path the two end caps
    /// (with the profile's holes, and its front and back materials). Everything is drawn with
    /// the rules of `add_edge`, so it merges with what it touches; a region the sweep closes off
    /// by itself (the hole of a ring) gets no face. The profile face is removed; its edges stay
    /// only where they bound faces, and not where they cross a side face (a profile not square to
    /// the path), whose pieces merge.
    /// New faces point out of the swept solid. An edge between the faces of two profile sides on
    /// one curve that is not a polygon is soft, and so is an edge between the faces of two path
    /// segments on one such curve.
    pub fn follow_me(&mut self, plan: &FollowMe) -> ChangeSet {
        let secs = &plan.sweep.sections;
        let segs = secs.len() - 1;
        let n = plan.path.len();
        let dirs: Vec<DVec3> = (0..segs)
            .map(|k| (plan.path[(k + 1) % n] - plan.path[k]).normalize())
            .collect();
        // A closed path's last section is its first.
        let distinct = if plan.sweep.caps { segs + 1 } else { segs };
        if self.face(plan.face).is_some() {
            self.remove_face(plan.face);
        }
        let before: BTreeSet<FaceId> = self.faces().map(|(f, _)| f).collect();

        let mut segments: Vec<(DVec3, DVec3)> = Vec::new();
        for sec in &secs[..distinct] {
            for l in sec {
                segments.extend(sides(l));
            }
        }
        // Each side face (or triangle of one) with the direction out of the solid.
        let mut pieces: Vec<(Vec<DVec3>, DVec3)> = Vec::new();
        let mut diagonals: Vec<(DVec3, DVec3)> = Vec::new();
        for k in 0..segs {
            for (la, lb) in secs[k].iter().zip(&secs[k + 1]) {
                let m = la.len();
                for j in 0..m {
                    let j1 = (j + 1) % m;
                    let (a, b, c, d) = (la[j], la[j1], lb[j1], lb[j]);
                    segments.push((a, d));
                    let out = (b - a).cross(dirs[k]) * plan.turn;
                    if planar(&[a, b, c, d]) {
                        pieces.push((vec![a, b, c, d], out));
                    } else {
                        segments.push((a, c));
                        diagonals.push((a, c));
                        pieces.push((vec![a, b, c], out));
                        pieces.push((vec![a, c, d], out));
                    }
                }
            }
        }
        self.draw_segments(&segments);

        // Each section's plane (unit normal) and loops flattened into it.
        let flat: Vec<(Plane, DVec3, Vec<Vec<DVec2>>)> = secs[..distinct]
            .iter()
            .map(|sec| {
                let normal = crate::geom::vector_area(&sec[0]).normalize();
                let plane = Plane::through(sec[0][0], normal);
                let loops = sec
                    .iter()
                    .map(|l| l.iter().map(|p| plane.project(*p)).collect())
                    .collect();
                (plane, normal, loops)
            })
            .collect();
        // A cap whose loops were already edges (the profile's own) is filled here.
        let caps: Vec<(usize, DVec3)> = if plan.sweep.caps {
            vec![(0, -dirs[0]), (segs, dirs[segs - 1])]
        } else {
            Vec::new()
        };
        for &(i, _) in &caps {
            let edges: Vec<EdgeId> = secs[i]
                .iter()
                .flat_map(|l| sides(l))
                .flat_map(|(a, b)| self.edges_along(a, b))
                .collect();
            self.rebuild_plane(flat[i].0, &edges);
        }
        // So is a side face lying where the profile face was (a profile drawn on a face the
        // sweep runs along).
        let normal = crate::geom::vector_area(&plan.profile[0]).normalize();
        let plane = Plane::through(plan.profile[0][0], normal);
        let covers = pieces.iter().any(|(poly, _)| {
            let n = crate::geom::vector_area(poly).normalize_or_zero();
            n.cross(normal).length() < PLANE_ANGLE_TOL && plane.contains(poly[0])
        });
        if covers {
            let edges: Vec<EdgeId> = plan
                .profile
                .iter()
                .flat_map(|l| sides(l))
                .flat_map(|(a, b)| self.edges_along(a, b))
                .collect();
            self.rebuild_plane(plane, &edges);
        }

        let made: Vec<FaceId> = self
            .faces()
            .map(|(f, _)| f)
            .filter(|f| !before.contains(f))
            .collect();
        // The faces the drawing split, as they were: a new face inside one is a piece of it.
        let split: Vec<(Plane, Vec<Vec<DVec2>>)> = self
            .pending()
            .iter()
            .filter_map(|op| match op {
                Op::RemoveFace(id, f) if before.contains(id) => Some(f),
                _ => None,
            })
            .map(|f| {
                let plane = Plane::through(self.pos(f.outer[0]), f.normal);
                let loops = std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .map(|l| l.iter().map(|v| plane.project(self.pos(*v))).collect())
                    .collect();
                (plane, loops)
            })
            .collect();
        for f in made {
            let normal = self.face(f).unwrap().normal;
            let Some(p) = self.interior_point(f) else {
                continue;
            };
            let in_section = flat.iter().position(|(plane, sn, loops)| {
                normal.cross(*sn).length() < PLANE_ANGLE_TOL
                    && plane.contains(p)
                    && point_in_polygon(plane.project(p), &loops[0])
            });
            if let Some(i) = in_section {
                let (plane, _, loops) = &flat[i];
                let q = plane.project(p);
                match caps.iter().find(|(c, _)| *c == i) {
                    Some((_, out)) if !loops[1..].iter().any(|h| point_in_polygon(q, h)) => {
                        self.orient(f, *out);
                        self.set_paint(f, plan.paint);
                        // New geometry is Untagged, whatever the profile was on.
                        self.set_tag(EntityId::Face(f), None);
                    }
                    // A hole of a cap, or a section inside the sweep, stays open.
                    _ => {
                        self.remove_face(f);
                    }
                }
                continue;
            }
            if let Some((_, out)) = pieces.iter().find(|(poly, _)| in_piece(poly, p)) {
                self.orient(f, *out);
                self.set_tag(EntityId::Face(f), None);
                continue;
            }
            // Neither a side, a cap nor a piece of a face that was there: a region the sweep
            // closed off (the hole in a ring) stays open.
            let in_split = split.iter().any(|(plane, loops)| {
                let q = plane.project(p);
                plane.contains(p)
                    && point_in_polygon(q, &loops[0])
                    && !loops[1..].iter().any(|h| point_in_polygon(q, h))
            });
            if !in_split {
                self.remove_face(f);
            }
        }
        self.clear_profile(&plan.profile);
        self.merge_across_profile(&plan.profile, &pieces);

        // Soft edges.
        let mut soft: BTreeSet<EdgeId> = BTreeSet::new();
        let smooth = |c: &[Option<(CurveId, bool)>], before: usize, after: usize| {
            c[before].is_some_and(|(_, polygon)| !polygon) && c[before] == c[after]
        };
        for k in 0..segs {
            for (l, (la, lb)) in secs[k].iter().zip(&secs[k + 1]).enumerate() {
                let m = la.len();
                for j in 0..m {
                    if smooth(&plan.side_curves[l], (j + m - 1) % m, j) {
                        soft.extend(self.edges_along(la[j], lb[j]));
                    }
                }
            }
        }
        let joints = if plan.sweep.caps { 1..segs } else { 0..segs };
        for i in joints {
            if smooth(&plan.path_curves, (i + segs - 1) % segs, i) {
                for l in &secs[i] {
                    for (a, b) in sides(l) {
                        soft.extend(self.edges_along(a, b));
                    }
                }
            }
        }
        for (a, c) in diagonals {
            soft.extend(self.edges_along(a, c));
        }
        for e in soft {
            let edge = self.edge(e).unwrap();
            let (curve, polygon) = (edge.curve, edge.polygon);
            self.set_edge(e, curve, true, polygon);
        }
        self.take_changes()
    }

    /// Erases the edges along the profile's loop sides that cross a side face (`pieces`), between
    /// two of its pieces in one plane, so the pieces merge back into one face. A profile not
    /// square to the path's first segment lies across that segment's side faces.
    fn merge_across_profile(&mut self, profile: &[Vec<DVec3>], pieces: &[(Vec<DVec3>, DVec3)]) {
        let segments: Vec<(DVec3, DVec3)> = profile.iter().flat_map(|l| sides(l)).collect();
        let across: Vec<EdgeId> = self
            .edges()
            .filter(|(_, e)| {
                let [a, b] = e.v;
                let (pa, pb) = (self.pos(a), self.pos(b));
                let mid = (pa + pb) / 2.0;
                segments
                    .iter()
                    .any(|s| on_segment(pa, *s) && on_segment(pb, *s))
                    && pieces.iter().any(|(poly, _)| inside_piece(poly, mid))
                    && matches!(self.faces_on_side(a, b)[..],
                        [f, g] if self.shared_plane(f, g).is_some())
            })
            .map(|(e, _)| e)
            .collect();
        self.erase_pending(&across, &[]);
    }

    /// Removes the edges along the profile's loop sides that bound no face, then the vertices
    /// they leave without edges.
    fn clear_profile(&mut self, profile: &[Vec<DVec3>]) {
        let segments: Vec<(DVec3, DVec3)> = profile.iter().flat_map(|l| sides(l)).collect();
        let on = on_segment;
        let strays: Vec<EdgeId> = self
            .edges()
            .filter(|(_, e)| {
                let [a, b] = e.v;
                let (pa, pb) = (self.pos(a), self.pos(b));
                self.faces_on_side(a, b).is_empty()
                    && segments.iter().any(|s| on(pa, *s) && on(pb, *s))
            })
            .map(|(e, _)| e)
            .collect();
        let mut ends: BTreeSet<VertexId> = BTreeSet::new();
        for e in strays {
            ends.extend(self.remove_edge(e).v);
        }
        for v in ends {
            if self.edges_at(v).is_empty() {
                self.remove_vertex(v);
            }
        }
    }
}

/// The indices of the path's corners: its points less those in the middle of a straight run
/// (turning by less than `ANGLE_TOL`). An open path keeps its ends.
fn corners(path: &[DVec3], closed: bool) -> Vec<usize> {
    let n = path.len();
    let straight = |a: DVec3, b: DVec3, c: DVec3| {
        let (u, v) = (b - a, c - b);
        u.cross(v).length() <= ANGLE_TOL * u.length() * v.length() && u.dot(v) > 0.0
    };
    let first = if closed {
        match (0..n).find(|&i| !straight(path[(i + n - 1) % n], path[i], path[(i + 1) % n])) {
            Some(i) => i,
            None => return (0..n).collect(),
        }
    } else {
        0
    };
    let mut keep = vec![first];
    let last = if closed { n } else { n - 1 };
    for s in 1..last {
        let i = (first + s) % n;
        if !straight(path[*keep.last().unwrap()], path[i], path[(i + 1) % n]) {
            keep.push(i);
        }
    }
    if !closed {
        keep.push(n - 1);
    }
    keep.sort_unstable();
    keep
}

/// The loop's smallest width in its plane, measured square to each of its sides (0 when the
/// loop has no area).
fn narrowest(l: &[DVec3]) -> f64 {
    let Some(normal) = crate::geom::vector_area(l).try_normalize() else {
        return 0.0;
    };
    sides(l)
        .filter_map(|(a, b)| normal.cross(b - a).try_normalize())
        .map(|across| {
            let (lo, hi) = l.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
                let t = p.dot(across);
                (lo.min(t), hi.max(t))
            });
            hi - lo
        })
        .fold(f64::MAX, f64::min)
}

/// The sides of the closed loop through `l`.
fn sides(l: &[DVec3]) -> impl Iterator<Item = (DVec3, DVec3)> + '_ {
    (0..l.len()).map(move |j| (l[j], l[(j + 1) % l.len()]))
}

/// True when the points lie in one plane, well within the tolerance faces are made with.
fn planar(points: &[DVec3]) -> bool {
    let normal = crate::geom::vector_area(points).normalize_or_zero();
    let centre = points.iter().sum::<DVec3>() / points.len() as f64;
    points
        .iter()
        .all(|p| (*p - centre).dot(normal).abs() < 0.1 * LENGTH_TOL)
}

/// True when `p` lies on the segment from `a` to `b` (within `LENGTH_TOL`).
fn on_segment(p: DVec3, (a, b): (DVec3, DVec3)) -> bool {
    let (t, d) = project_on_segment(p, a, b);
    let tol = LENGTH_TOL / (b - a).length();
    d < LENGTH_TOL && t > -tol && t < 1.0 + tol
}

/// True when `p` lies on the planar polygon `poly` and not on its boundary.
fn inside_piece(poly: &[DVec3], p: DVec3) -> bool {
    in_piece(poly, p) && !sides(poly).any(|s| on_segment(p, s))
}

/// True when `p` lies on the planar polygon `poly` (within `LENGTH_TOL` of its plane).
fn in_piece(poly: &[DVec3], p: DVec3) -> bool {
    let normal = crate::geom::vector_area(poly).normalize_or_zero();
    if normal == DVec3::ZERO {
        return false;
    }
    let plane = Plane::through(poly[0], normal);
    if !plane.contains(p) {
        return false;
    }
    let flat: Vec<DVec2> = poly.iter().map(|q| plane.project(*q)).collect();
    point_in_polygon(plane.project(p), &flat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    /// A unit square in the plane x = 0, counter-clockwise around +x, its corner at the origin.
    fn square() -> Vec<Vec<DVec3>> {
        vec![vec![
            p(0.0, 0.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 1.0, 1.0),
            p(0.0, 0.0, 1.0),
        ]]
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn a_straight_path_gives_congruent_sections() {
        let path = [p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(3.0, 0.0, 0.0)];
        let s = sweep(&square(), DVec3::X, &path, false).unwrap();
        assert!(s.caps);
        assert_eq!(s.sections.len(), 3);
        for (sec, x) in s.sections.iter().zip([0.0, 1.0, 3.0]) {
            for (got, want) in sec[0].iter().zip(&square()[0]) {
                assert!(close(*got, *want + p(x, 0.0, 0.0)), "{sec:?}");
            }
        }
    }

    #[test]
    fn an_l_path_mitres_its_corner() {
        // Turning left (toward +y): the profile's y = 0 side is on the outside of the turn.
        let square: Vec<Vec<DVec3>> =
            vec![square()[0].iter().map(|q| *q - p(0.0, 0.5, 0.0)).collect()];
        let path = [p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(2.0, 2.0, 0.0)];
        let s = sweep(&square, DVec3::X, &path, false).unwrap();
        assert_eq!(s.sections.len(), 3);
        let m = p(1.0, 1.0, 0.0).normalize();
        for q in &s.sections[1][0] {
            assert!((*q - path[1]).dot(m).abs() < 1e-9, "{q}");
        }
        assert!(close(s.sections[1][0][0], p(2.5, -0.5, 0.0)));
        assert!(close(s.sections[1][0][1], p(1.5, 0.5, 0.0)));
        // The end is square to the last segment, the profile turned a quarter.
        for q in &s.sections[2][0] {
            assert!((q.y - 2.0).abs() < 1e-9, "{q}");
        }
        assert!(close(s.sections[2][0][0], p(2.5, 2.0, 0.0)));
    }

    #[test]
    fn a_closed_path_ends_on_its_first_section() {
        let path = [
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 4.0, 0.0),
            p(0.0, 4.0, 0.0),
        ];
        let s = sweep(&square(), DVec3::X, &path, true).unwrap();
        assert!(!s.caps);
        assert_eq!(s.sections.len(), 5);
        assert_eq!(s.sections[0], s.sections[4]);
        // The first section is on the miter plane at the start.
        let m = p(1.0, -1.0, 0.0).normalize();
        for q in &s.sections[0][0] {
            assert!(q.dot(m).abs() < 1e-9, "{q}");
        }
    }

    #[test]
    fn a_closed_path_that_twists_still_closes() {
        // Up a step and back: the frame comes back turned, and the twist is spread out.
        let path = [
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 4.0, 2.0),
            p(0.0, 4.0, 4.0),
            p(-2.0, 2.0, 1.0),
        ];
        let small: Vec<Vec<DVec3>> = vec![
            square()[0]
                .iter()
                .map(|q| (*q - p(0.0, 0.5, 0.5)) * 0.2)
                .collect(),
        ];
        let s = sweep(&small, DVec3::X, &path, true).unwrap();
        assert_eq!(s.sections.len(), 6);
        assert_eq!(s.sections[0], s.sections[5]);
    }

    #[test]
    fn a_big_profile_on_a_sharp_turn_folds() {
        let path = [p(0.0, 0.0, 0.0), p(0.2, 0.0, 0.0), p(0.2, 0.2, 0.0)];
        assert_eq!(
            sweep(&square(), DVec3::X, &path, false),
            Err(SweepError::Folds)
        );
    }

    #[test]
    fn a_profile_tilted_a_hair_off_the_path_is_degenerate_not_a_sliver() {
        // A diamond standing 2e-6 radians off the plane holding the path: its section square
        // to the path has corners half a metre apart but is only 2e-6 m wide.
        let w = p(1.0, 2e-6, 0.0);
        let tilted = vec![
            [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]
                .iter()
                .map(|(a, b)| w * *a + DVec3::Z * *b)
                .collect::<Vec<_>>(),
        ];
        let normal = crate::geom::vector_area(&tilted[0]).normalize();
        assert!(normal.dot(DVec3::X).abs() > 1e-9);
        let path = [p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)];
        assert_eq!(
            sweep(&tilted, normal, &path, false),
            Err(SweepError::Degenerate)
        );
    }

    #[test]
    fn a_vertex_in_a_straight_run_is_passed_over() {
        let path = [
            p(0.0, 0.0, 0.0),
            p(1.5, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 2.0, 0.0),
        ];
        assert_eq!(corners(&path, false), vec![0, 2, 3]);
        let ring = [
            p(1.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(0.0, 2.0, 0.0),
            p(0.0, 0.0, 0.0),
        ];
        assert_eq!(corners(&ring, true), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_closed_path_does_not_start_where_it_turns_in_the_profile_s_plane() {
        // The profile in the plane x = 0 at the corner s, where the path turns within that
        // plane: from there the path would set off along the profile.
        let mut m = Model::new();
        let s = p(0.0, 0.0, 0.0);
        let ring = [
            s,
            p(0.0, 0.0, 0.5),
            p(10.0, 0.0, 0.5),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ];
        for i in 0..5 {
            m.add_edge(ring[i], ring[(i + 1) % 5]).unwrap();
        }
        m.add_polygon(&[
            p(0.0, 0.02, 0.02),
            p(0.0, 0.1, 0.02),
            p(0.0, 0.1, 0.1),
            p(0.0, 0.02, 0.1),
        ])
        .unwrap();
        let face = m.faces().next().unwrap().0;
        let path: Vec<VertexId> = ring.iter().map(|q| m.find_vertex(*q).unwrap()).collect();
        let plan = m.plan_follow_me(face, &path, true).unwrap();
        assert!(!close(plan.path[0], s), "{:?}", plan.path);
    }

    #[test]
    fn a_u_turn_or_a_profile_along_the_path_is_degenerate() {
        let path = [p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.5, 0.0, 0.0)];
        assert_eq!(
            sweep(&square(), DVec3::X, &path, false),
            Err(SweepError::Degenerate)
        );
        let along = [p(0.0, 0.0, 0.0), p(0.0, 2.0, 0.0)];
        assert_eq!(
            sweep(&square(), DVec3::X, &along, false),
            Err(SweepError::Degenerate)
        );
    }
}
