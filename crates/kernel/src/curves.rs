//! Curves (chains of edges drawn as one circle or arc) and soft edges.
//!
//! A curve is only a tag on its edges: its shape (center, radius, angle) is worked out from
//! where the edges are, so it follows every move, turn, copy and undo without being stored.
//! Every edit ends with `upkeep`, which keeps two rules: the edges of a curve form one
//! unbranched chain that is not straight, and a soft edge bounds at least two faces.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::edit::{KernelError, check_segment};
use crate::geom::{LENGTH_TOL, PLANE_ANGLE_TOL, project_on_segment};
use crate::ids::{CurveId, EdgeId, VertexId};
use crate::model::{ChangeSet, Face, Model, Op};

/// A curve as its edges trace it now.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub id: CurveId,
    /// The edges in order along the curve.
    pub edges: Vec<EdgeId>,
    /// The vertices in order along the curve (counter-clockwise around the circle's normal
    /// when it lies on one). A closed curve's first vertex is not repeated at the end.
    pub vertices: Vec<VertexId>,
    /// True when the chain closes on itself.
    pub closed: bool,
    /// Straight segments: an edge other geometry split in two still counts once.
    pub segments: usize,
    /// Total length of the edges.
    pub length: f64,
    /// The circle through the curve's corners, when they all lie on one.
    pub circle: Option<CurveCircle>,
    /// True when the curve's edges are the sides of a polygon (see `Edge::polygon`).
    pub polygon: bool,
}

/// The circle a curve's corners lie on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveCircle {
    pub center: DVec3,
    /// The unit normal of the circle's plane, pointing up: positive z, or on a vertical plane
    /// positive y, else positive x. A curve has no direction of its own.
    pub normal: DVec3,
    pub radius: f64,
    /// The angle the curve sweeps around the center, in radians (TAU when closed).
    pub angle: f64,
}

/// What a curve traces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    /// A closed chain whose corners lie on a circle.
    Circle,
    /// An open chain whose corners lie on a circle.
    Arc,
    /// A chain whose corners no longer lie on one circle (some of its vertices were moved), or
    /// a polygon that was opened up or bent off its circle.
    Curve,
    /// A closed chain of polygon sides whose corners lie on a circle.
    Polygon,
}

impl Curve {
    pub fn kind(&self) -> CurveKind {
        match (self.polygon, self.closed, self.circle.is_some()) {
            (true, true, true) => CurveKind::Polygon,
            (true, _, _) => CurveKind::Curve,
            (false, true, true) => CurveKind::Circle,
            (false, false, true) => CurveKind::Arc,
            (false, _, false) => CurveKind::Curve,
        }
    }
}

impl Model {
    /// Draws the polyline through `points` (closed: the last point joins the first) with the
    /// rules of `add_edge`, and puts the edges along it on one new curve, as one ChangeSet;
    /// with `polygon` the curve's edges are polygon sides (see `Edge::polygon`).
    /// A polyline that is straight, or whose edges do not form one chain, gets no curve.
    pub fn add_curve(
        &mut self,
        points: &[DVec3],
        closed: bool,
        polygon: bool,
    ) -> Result<ChangeSet, KernelError> {
        let n = points.len();
        let sides = if closed { n } else { n.saturating_sub(1) };
        for i in 0..sides {
            check_segment(points[i], points[(i + 1) % n])?;
        }
        let segments: Vec<(DVec3, DVec3)> = (0..sides)
            .map(|i| (points[i], points[(i + 1) % n]))
            .collect();
        self.draw_segments(&segments);
        // A straight polyline never becomes a curve: do not spend an id on it.
        if !closed && corners(points, false).is_empty() {
            return Ok(self.take_changes());
        }
        let curve = self.new_curve();
        for i in 0..sides {
            self.tag_along(points[i], points[(i + 1) % n], Some(curve), false, polygon);
        }
        Ok(self.take_changes())
    }

    /// Puts every edge lying along the segment a-b on `curve` and sets its soft and polygon
    /// flags.
    pub(crate) fn tag_along(
        &mut self,
        a: DVec3,
        b: DVec3,
        curve: Option<CurveId>,
        soft: bool,
        polygon: bool,
    ) {
        for e in self.edges_along(a, b) {
            self.set_edge(e, curve, soft, polygon);
        }
    }

    /// The edges lying along the segment a-b (none when it is shorter than `LENGTH_TOL`).
    pub(crate) fn edges_along(&self, a: DVec3, b: DVec3) -> BTreeSet<EdgeId> {
        let mut along: BTreeSet<EdgeId> = BTreeSet::new();
        let len = a.distance(b);
        if len < LENGTH_TOL {
            return along;
        }
        let on = |p: DVec3| {
            let (t, d) = project_on_segment(p, a, b);
            let tol = LENGTH_TOL / len;
            d < LENGTH_TOL && t > -tol && t < 1.0 + tol
        };
        // The edges along a-b form chains reaching its ends (drawing a-b joined every vertex
        // on it), so walk out from the vertices at the ends.
        let ends: Vec<VertexId> = [a, b].iter().filter_map(|p| self.find_vertex(*p)).collect();
        if ends.len() < 2 {
            // An end is gone (push/pull removed strays): pieces may be cut off from it, so
            // look at every edge.
            for (e, edge) in self.edges() {
                if on(self.pos(edge.v[0])) && on(self.pos(edge.v[1])) {
                    along.insert(e);
                }
            }
        }
        let mut seen: BTreeSet<VertexId> = BTreeSet::new();
        let mut stack = ends;
        while let Some(v) = stack.pop() {
            if !seen.insert(v) {
                continue;
            }
            for e in self.edges_at(v) {
                let [p, q] = self.edge(e).unwrap().v;
                if on(self.pos(p)) && on(self.pos(q)) {
                    along.insert(e);
                    stack.push(if p == v { q } else { p });
                }
            }
        }
        along
    }

    /// The curve edge `edge` is part of, if any.
    pub fn curve_of(&self, edge: EdgeId) -> Option<Curve> {
        self.curve(self.edge(edge)?.curve?)
    }

    /// The curve with this id, if any edge of the mesh is on it.
    pub fn curve(&self, id: CurveId) -> Option<Curve> {
        let members: Vec<(EdgeId, [VertexId; 2])> = self
            .edges()
            .filter(|(_, e)| e.curve == Some(id))
            .map(|(e, edge)| (e, edge.v))
            .collect();
        let chain = chain(&members)?;
        // Every edge of a curve has the same polygon flag.
        let polygon = self.edge(members[0].0).unwrap().polygon;
        let points: Vec<DVec3> = chain.vertices.iter().map(|v| self.pos(*v)).collect();
        let corners = corners(&points, chain.closed);
        let segments = if chain.closed {
            corners.len()
        } else {
            corners.len() + 1
        };
        let length = chain
            .edges
            .iter()
            .map(|e| {
                let [a, b] = self.edge(*e).unwrap().v;
                self.pos(a).distance(self.pos(b))
            })
            .sum();
        let mut curve = Curve {
            id,
            edges: chain.edges,
            vertices: chain.vertices,
            closed: chain.closed,
            segments,
            length,
            circle: None,
            polygon,
        };
        // The corners, plus the ends of an open curve, are the points that lie on the circle.
        let mut key: Vec<usize> = corners.clone();
        if !curve.closed {
            key.insert(0, 0);
            key.push(points.len() - 1);
        }
        let turn: DVec3 = corners.iter().map(|&i| turn_at(&points, i)).sum();
        let Some(mut normal) = turn.try_normalize() else {
            return Some(curve);
        };
        let up = normal.z > 1e-9
            || (normal.z.abs() <= 1e-9 && normal.y > 1e-9)
            || (normal.z.abs() <= 1e-9 && normal.y.abs() <= 1e-9 && normal.x > 0.0);
        if !up {
            normal = -normal;
            curve.vertices.reverse();
            curve.edges.reverse();
            if curve.closed {
                // Keep the same first vertex.
                curve.vertices.rotate_right(1);
            }
        }
        if curve.closed {
            // Start at the oldest vertex: for a drawn circle, the first corner.
            let first = (0..curve.vertices.len())
                .min_by_key(|&i| curve.vertices[i])
                .unwrap();
            curve.vertices.rotate_left(first);
            let start = curve.vertices[0];
            let at = curve
                .edges
                .iter()
                .position(|e| {
                    let [a, b] = self.edge(*e).unwrap().v;
                    let next = curve.vertices[1 % curve.vertices.len()];
                    (a == start && b == next) || (b == start && a == next)
                })
                .unwrap_or(0);
            curve.edges.rotate_left(at);
        }
        let key: Vec<DVec3> = key.iter().map(|&i| points[i]).collect();
        let k = key.len();
        if k < 3 {
            return Some(curve);
        }
        let (a, b, c) = if curve.closed {
            (key[0], key[k / 3], key[2 * k / 3])
        } else {
            (key[0], key[k / 2], key[k - 1])
        };
        let Some(center) = circumcenter(a, b, c) else {
            return Some(curve);
        };
        let radius = a.distance(center);
        let round = key.iter().all(|p| {
            (p.distance(center) - radius).abs() <= LENGTH_TOL
                && (*p - center).dot(normal).abs() <= LENGTH_TOL
        });
        if !round {
            return Some(curve);
        }
        let angle = if curve.closed {
            TAU
        } else {
            let ordered: Vec<DVec3> = curve.vertices.iter().map(|v| self.pos(*v)).collect();
            ordered
                .windows(2)
                .map(|w| {
                    let (u, v) = (w[0] - center, w[1] - center);
                    u.cross(v).dot(normal).atan2(u.dot(v))
                })
                .sum()
        };
        curve.circle = Some(CurveCircle {
            center,
            normal,
            radius,
            angle,
        });
        Some(curve)
    }

    /// Brings the curves and soft edges the pending edit touched back in line: a curve whose
    /// edges fell apart into several chains keeps its id on the oldest chain and the others
    /// become curves of their own; a chain that branches or is straight (a lone edge) stops
    /// being a curve; a soft edge left bounding fewer than two faces stops being soft. Last,
    /// annotations whose vertex or curve the edit took away are frozen in place (see
    /// `freeze_annotations`).
    pub(crate) fn upkeep(&mut self) {
        let mut curves: BTreeSet<CurveId> = BTreeSet::new();
        let mut soft: BTreeSet<EdgeId> = BTreeSet::new();
        let mut lost: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        let mut moved: BTreeSet<VertexId> = BTreeSet::new();
        for op in self.pending() {
            match op {
                Op::AddEdge(id, e) => {
                    curves.extend(e.curve);
                    if e.soft {
                        soft.insert(*id);
                    }
                }
                Op::RemoveEdge(_, e) => curves.extend(e.curve),
                Op::ChangeEdge(id, from, to) => {
                    curves.extend(from.curve);
                    curves.extend(to.curve);
                    if to.soft {
                        soft.insert(*id);
                    }
                }
                Op::RemoveFace(_, f) => lost.extend(face_sides(f)),
                Op::MoveVertex(v, _, _) => {
                    moved.insert(*v);
                }
                _ => {}
            }
        }
        for v in &moved {
            for e in self.edges_at(*v) {
                curves.extend(self.edge(e).unwrap().curve);
            }
        }
        for (a, b) in &lost {
            if let Some(e) = self.edge_between(*a, *b)
                && self.edge(e).unwrap().soft
            {
                soft.insert(e);
            }
        }
        if !curves.is_empty() {
            self.mend_curves(&curves);
        }
        if !soft.is_empty() {
            let hard: Vec<EdgeId> = soft
                .into_iter()
                .filter(|e| {
                    self.edge(*e)
                        .is_some_and(|edge| edge.soft && self.side_uses(edge.v[0], edge.v[1]) < 2)
                })
                .collect();
            for e in hard {
                let edge = self.edge(e).unwrap();
                let (curve, polygon) = (edge.curve, edge.polygon);
                self.set_edge(e, curve, false, polygon);
            }
        }
        self.freeze_annotations(&curves);
    }

    fn mend_curves(&mut self, curves: &BTreeSet<CurveId>) {
        let mut members: BTreeMap<CurveId, Vec<(EdgeId, [VertexId; 2])>> = BTreeMap::new();
        for (id, e) in self.edges() {
            if let Some(c) = e.curve.filter(|c| curves.contains(c)) {
                members.entry(c).or_default().push((id, e.v));
            }
        }
        for (c, edges) in members {
            let mut kept = false;
            for part in components(&edges) {
                let fine = chain(&part).is_some_and(|ch| !self.straight(&ch));
                let tag = if !fine {
                    None
                } else if !kept {
                    kept = true;
                    Some(c)
                } else {
                    Some(self.new_curve())
                };
                if tag != Some(c) {
                    for (e, _) in part {
                        let edge = self.edge(e).unwrap();
                        let (soft, polygon) = (edge.soft, edge.polygon);
                        self.set_edge(e, tag, soft, polygon);
                    }
                }
            }
        }
    }

    /// True when an open chain does not turn anywhere (a closed one always turns).
    fn straight(&self, chain: &Chain) -> bool {
        let points: Vec<DVec3> = chain.vertices.iter().map(|v| self.pos(*v)).collect();
        !chain.closed && corners(&points, false).is_empty()
    }

    /// How many times the face loops use the side a-b.
    fn side_uses(&self, a: VertexId, b: VertexId) -> usize {
        let s = side(a, b);
        self.faces_at(a)
            .map(|f| {
                face_sides(self.face(f).unwrap())
                    .into_iter()
                    .filter(|x| *x == s)
                    .count()
            })
            .sum()
    }

    /// How many face loops use each side (a vertex pair, smaller id first).
    fn face_counts(&self) -> BTreeMap<(VertexId, VertexId), usize> {
        let mut counts = BTreeMap::new();
        for (_, f) in self.faces() {
            for s in face_sides(f) {
                *counts.entry(s).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Checks the curve and soft-edge rules `upkeep` keeps.
    pub(crate) fn validate_curves(&self) -> Result<(), String> {
        let mut members: BTreeMap<CurveId, Vec<(EdgeId, [VertexId; 2])>> = BTreeMap::new();
        let mut polygon: BTreeMap<CurveId, bool> = BTreeMap::new();
        for (id, e) in self.edges() {
            if let Some(c) = e.curve {
                members.entry(c).or_default().push((id, e.v));
                if *polygon.entry(c).or_insert(e.polygon) != e.polygon {
                    return Err(format!("curve {c} has polygon and non-polygon edges"));
                }
            } else if e.polygon {
                return Err(format!("polygon edge {id:?} is on no curve"));
            }
        }
        for (c, edges) in &members {
            match chain(edges) {
                None => return Err(format!("curve {c} is not one unbranched chain")),
                Some(ch) if self.straight(&ch) => return Err(format!("curve {c} is straight")),
                Some(_) => {}
            }
        }
        let bounding = self.face_counts();
        for (id, e) in self.edges() {
            if e.soft && bounding.get(&side(e.v[0], e.v[1])).unwrap_or(&0) < &2 {
                return Err(format!("soft edge {id:?} bounds fewer than two faces"));
            }
        }
        Ok(())
    }
}

/// Edges in order along a chain, with its vertices.
pub(crate) struct Chain {
    pub(crate) edges: Vec<EdgeId>,
    pub(crate) vertices: Vec<VertexId>,
    pub(crate) closed: bool,
}

/// The edges as one unbranched chain (open or closed), or None if they branch, fall apart,
/// or are none.
pub(crate) fn chain(edges: &[(EdgeId, [VertexId; 2])]) -> Option<Chain> {
    let mut at: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for (i, (_, [a, b])) in edges.iter().enumerate() {
        at.entry(*a).or_default().push(i);
        at.entry(*b).or_default().push(i);
    }
    if edges.is_empty() || at.values().any(|e| e.len() > 2) {
        return None;
    }
    let ends: Vec<VertexId> = at
        .iter()
        .filter(|(_, e)| e.len() == 1)
        .map(|(v, _)| *v)
        .collect();
    let (start, closed) = match ends[..] {
        [] => (edges[0].1[0], true),
        [first, _] => (first, false),
        _ => return None,
    };
    let mut used = vec![false; edges.len()];
    let mut out = Chain {
        edges: Vec::new(),
        vertices: vec![start],
        closed,
    };
    let mut v = start;
    while let Some(&i) = at[&v].iter().find(|&&i| !used[i]) {
        used[i] = true;
        let (e, [a, b]) = edges[i];
        out.edges.push(e);
        v = if a == v { b } else { a };
        out.vertices.push(v);
    }
    if closed {
        out.vertices.pop();
    }
    (out.edges.len() == edges.len()).then_some(out)
}

/// The edges split into groups connected through shared vertices, oldest edge's group first.
fn components(edges: &[(EdgeId, [VertexId; 2])]) -> Vec<Vec<(EdgeId, [VertexId; 2])>> {
    let mut at: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for (i, (_, [a, b])) in edges.iter().enumerate() {
        at.entry(*a).or_default().push(i);
        at.entry(*b).or_default().push(i);
    }
    let mut group = vec![usize::MAX; edges.len()];
    let mut parts: Vec<Vec<(EdgeId, [VertexId; 2])>> = Vec::new();
    for first in 0..edges.len() {
        if group[first] != usize::MAX {
            continue;
        }
        let g = parts.len();
        parts.push(Vec::new());
        let mut stack = vec![first];
        group[first] = g;
        while let Some(i) = stack.pop() {
            parts[g].push(edges[i]);
            for v in edges[i].1 {
                for &j in &at[&v] {
                    if group[j] == usize::MAX {
                        group[j] = g;
                        stack.push(j);
                    }
                }
            }
        }
    }
    for p in &mut parts {
        p.sort();
    }
    parts.sort();
    parts
}

/// Indices of the points where the polyline turns (for an open one, not counting its ends).
pub(crate) fn corners(points: &[DVec3], closed: bool) -> Vec<usize> {
    let n = points.len();
    let range = if closed { 0..n } else { 1..n.saturating_sub(1) };
    range
        .filter(|&i| {
            let (d1, d2) = in_out(points, i);
            d1.cross(d2).length() > PLANE_ANGLE_TOL * d1.length() * d2.length()
        })
        .collect()
}

/// The directions into and out of point `i` of a polyline (wrapping around).
fn in_out(points: &[DVec3], i: usize) -> (DVec3, DVec3) {
    let n = points.len();
    let (prev, next) = (points[(i + n - 1) % n], points[(i + 1) % n]);
    (points[i] - prev, next - points[i])
}

/// The turn at point `i`: along the normal the polyline turns counter-clockwise around.
fn turn_at(points: &[DVec3], i: usize) -> DVec3 {
    let (d1, d2) = in_out(points, i);
    d1.cross(d2)
}

/// The center of the circle through three points, or None if they are in a line.
fn circumcenter(a: DVec3, b: DVec3, c: DVec3) -> Option<DVec3> {
    let (ab, ac) = (b - a, c - a);
    let n = ab.cross(ac);
    let n2 = n.length_squared();
    if n2 <= f64::EPSILON * ab.length_squared() * ac.length_squared() {
        return None;
    }
    Some(a + (n.cross(ab) * ac.length_squared() + ac.cross(n) * ab.length_squared()) / (2.0 * n2))
}

/// The vertex pair of a side, smaller id first.
fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}

/// Every side of a face's loops.
fn face_sides(f: &Face) -> Vec<(VertexId, VertexId)> {
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .flat_map(|l| (0..l.len()).map(move |i| side(l[i], l[(i + 1) % l.len()])))
        .collect()
}
