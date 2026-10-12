//! A bounding volume hierarchy over the model's vertices, edges and face triangles, so ray
//! picks and cursor snaps visit only the geometry near a ray instead of the whole model.

use glam::DVec3;

use crate::geom::{LENGTH_TOL, Plane};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::Model;

/// A piece of geometry held by the index, with its position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Point(VertexId, DVec3),
    Segment(EdgeId, DVec3, DVec3),
    Triangle(FaceId, [DVec3; 3]),
}

impl Shape {
    /// Bounding box, padded by `LENGTH_TOL` so flat shapes have some thickness.
    fn bounds(&self) -> (DVec3, DVec3) {
        let (min, max) = match *self {
            Shape::Point(_, p) => (p, p),
            Shape::Segment(_, a, b) => (a.min(b), a.max(b)),
            Shape::Triangle(_, [a, b, c]) => (a.min(b).min(c), a.max(b).max(c)),
        };
        (
            min - DVec3::splat(LENGTH_TOL),
            max + DVec3::splat(LENGTH_TOL),
        )
    }
}

#[derive(Clone, Debug)]
enum Kids {
    /// Shapes `start..end`.
    Leaf(usize, usize),
    /// Node positions of the two halves.
    Inner(usize, usize),
}

#[derive(Clone, Debug)]
struct Node {
    min: DVec3,
    max: DVec3,
    kids: Kids,
}

/// Most shapes in a leaf.
const LEAF: usize = 4;

/// Spatial index over a model's geometry. Get it with [`Model::index`].
#[derive(Clone, Debug, Default)]
pub struct Index {
    nodes: Vec<Node>,
    shapes: Vec<Shape>,
}

impl Index {
    pub(crate) fn build(model: &Model) -> Index {
        let mut shapes: Vec<Shape> = model
            .vertices()
            .map(|(id, v)| Shape::Point(id, v.pos))
            .collect();
        for (id, e) in model.edges() {
            shapes.push(Shape::Segment(id, model.pos(e.v[0]), model.pos(e.v[1])));
        }
        for (id, _) in model.faces() {
            for t in model.triangulate(id).unwrap_or_default() {
                shapes.push(Shape::Triangle(id, t));
            }
        }
        Index::from_shapes(shapes)
    }

    /// An index over the model's edges only: cheaper to build, for `segments_near_plane`.
    pub(crate) fn of_edges(model: &Model) -> Index {
        let shapes = model
            .edges()
            .map(|(id, e)| Shape::Segment(id, model.pos(e.v[0]), model.pos(e.v[1])))
            .collect();
        Index::from_shapes(shapes)
    }

    fn from_shapes(shapes: Vec<Shape>) -> Index {
        let mut index = Index {
            nodes: Vec::new(),
            shapes,
        };
        if !index.shapes.is_empty() {
            index.split(0, index.shapes.len());
        }
        index
    }

    /// Adds the node over shapes `start..end` and its subtree; returns the node's position.
    fn split(&mut self, start: usize, end: usize) -> usize {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let mut cmin = min;
        let mut cmax = max;
        for s in &self.shapes[start..end] {
            let (a, b) = s.bounds();
            min = min.min(a);
            max = max.max(b);
            cmin = cmin.min((a + b) * 0.5);
            cmax = cmax.max((a + b) * 0.5);
        }
        let at = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            kids: Kids::Leaf(start, end),
        });
        if end - start <= LEAF {
            return at;
        }
        // Halve at the median centre along the axis where the centres spread most.
        let axis = (cmax - cmin).max_position();
        let centre = |s: &Shape| {
            let (a, b) = s.bounds();
            (a[axis] + b[axis]) * 0.5
        };
        let mid = (start + end) / 2;
        self.shapes[start..end]
            .select_nth_unstable_by(mid - start, |a, b| centre(a).total_cmp(&centre(b)));
        let left = self.split(start, mid);
        let right = self.split(mid, end);
        self.nodes[at].kids = Kids::Inner(left, right);
        at
    }

    /// The nearest face triangle that the ray `origin + t * dir` meets with
    /// `min_t < t <= max_t`: its face and `t`.
    pub fn ray_hit(
        &self,
        origin: DVec3,
        dir: DVec3,
        min_t: f64,
        max_t: f64,
    ) -> Option<(FaceId, f64)> {
        let mut best: Option<(FaceId, f64)> = None;
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            let reach = best.map_or(max_t, |(_, t)| t);
            if !ray_meets_box(origin, dir, node.min, node.max, reach) {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => {
                    for shape in &self.shapes[s..e] {
                        if let Shape::Triangle(f, tri) = *shape
                            && let Some(t) = ray_hits_triangle(origin, dir, tri)
                            && t > min_t
                            && t <= best.map_or(max_t, |(_, bt)| bt)
                            && best.is_none_or(|(_, bt)| t < bt)
                        {
                            best = Some((f, t));
                        }
                    }
                }
            }
        }
        best
    }

    /// Every face the ray `origin + t * dir` meets with `min_t < t <= max_t`, each with the
    /// `t` of its nearest hit, in face order.
    pub fn ray_hits(
        &self,
        origin: DVec3,
        dir: DVec3,
        min_t: f64,
        max_t: f64,
    ) -> Vec<(FaceId, f64)> {
        let mut hits: std::collections::BTreeMap<FaceId, f64> = Default::default();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            if !ray_meets_box(origin, dir, node.min, node.max, max_t) {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => {
                    for shape in &self.shapes[s..e] {
                        if let Shape::Triangle(f, tri) = *shape
                            && let Some(t) = ray_hits_triangle(origin, dir, tri)
                            && t > min_t
                            && t <= max_t
                        {
                            let at = hits.entry(f).or_insert(t);
                            *at = at.min(t);
                        }
                    }
                }
            }
        }
        hits.into_iter().collect()
    }

    /// Calls `visit` for every edge whose bounding box comes within `LENGTH_TOL` of `plane`
    /// (a superset of the edges lying in it; callers test exactly).
    pub(crate) fn segments_near_plane(&self, plane: &Plane, mut visit: impl FnMut(EdgeId)) {
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            let centre = (node.min + node.max) * 0.5;
            let reach = ((node.max - node.min) * 0.5).dot(plane.normal.abs());
            if plane.distance(centre).abs() > reach + LENGTH_TOL {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => {
                    for shape in &self.shapes[s..e] {
                        if let Shape::Segment(id, ..) = shape {
                            visit(*id);
                        }
                    }
                }
            }
        }
    }

    /// Calls `visit` for every vertex and edge that may lie within `r0 + slope * t` of the ray
    /// `origin + t * dir` (unit `dir`; `t` counted from 0): a cone around the ray, such as the
    /// pixels around a cursor. Some shapes outside it are visited too; callers test exactly.
    pub fn near_ray(
        &self,
        origin: DVec3,
        dir: DVec3,
        r0: f64,
        slope: f64,
        mut visit: impl FnMut(&Shape),
    ) {
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            let centre = (node.min + node.max) * 0.5;
            let radius = (node.max - node.min).length() * 0.5;
            let rel = centre - origin;
            let t = rel.dot(dir);
            let off = (rel - dir * t).length();
            if off > radius + r0 + slope * (t + radius).max(0.0) {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => self.shapes[s..e]
                    .iter()
                    .filter(|s| !matches!(s, Shape::Triangle(..)))
                    .for_each(&mut visit),
            }
        }
    }
}

/// True if the ray meets the box at some `t` in `[0, reach]`.
fn ray_meets_box(origin: DVec3, dir: DVec3, min: DVec3, max: DVec3, reach: f64) -> bool {
    let (mut lo, mut hi) = (0.0_f64, reach);
    for k in 0..3 {
        if dir[k].abs() < 1e-300 {
            if origin[k] < min[k] || origin[k] > max[k] {
                return false;
            }
            continue;
        }
        let (a, b) = ((min[k] - origin[k]) / dir[k], (max[k] - origin[k]) / dir[k]);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
        if lo > hi {
            return false;
        }
    }
    true
}

/// Distance along unit `dir` from `from` to triangle `t`, if the ray meets it.
fn ray_hits_triangle(from: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = from - a;
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let eps = 1e-12;
    (u >= -eps && v >= -eps && u + v <= 1.0 + eps).then(|| e2.dot(q) / det)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_near_plane_finds_every_edge_in_the_plane() {
        // A box of six faces plus a tilted triangle.
        let mut m = Model::new();
        let p = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
        let (lo, hi) = (p(0.0, 0.0, 0.0), p(2.0, 1.0, 3.0));
        for axis in 0..3 {
            for at in [lo[axis], hi[axis]] {
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let corner = |a: f64, b: f64| {
                    let mut q = DVec3::ZERO;
                    q[axis] = at;
                    q[u] = a;
                    q[v] = b;
                    q
                };
                m.add_polygon(&[
                    corner(lo[u], lo[v]),
                    corner(hi[u], lo[v]),
                    corner(hi[u], hi[v]),
                    corner(lo[u], hi[v]),
                ])
                .unwrap();
            }
        }
        m.add_polygon(&[p(5.0, 0.0, 0.0), p(6.0, 0.0, 1.0), p(5.0, 1.0, 0.5)])
            .unwrap();
        let index = Index::of_edges(&m);
        for (_, f) in m.faces() {
            let plane = Plane::through(m.pos(f.outer[0]), f.normal);
            let on = |e: &crate::model::Edge| {
                plane.contains(m.pos(e.v[0])) && plane.contains(m.pos(e.v[1]))
            };
            let all: Vec<EdgeId> = m.edges().filter(|(_, e)| on(e)).map(|(id, _)| id).collect();
            let mut found = Vec::new();
            index.segments_near_plane(&plane, |e| found.push(e));
            found.retain(|e| on(m.edge(*e).unwrap()));
            found.sort();
            assert_eq!(found, all);
            assert!(all.len() >= 3);
        }
    }
}
