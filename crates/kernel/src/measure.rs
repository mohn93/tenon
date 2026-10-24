//! Measuring: exact distances between entities (points, segments, guide lines, planar polygons
//! with holes, and whole instances through their triangles with bounding-box pruning), angles
//! between edges or faces, and least-squares fits of circles and cylinders to round features.
//!
//! Everything is worked out in the coordinates of the mesh the measuring starts from; an entity
//! inside nested instances is addressed by the path of instances leading to it.

use std::collections::BTreeSet;

use glam::{DAffine3, DMat3, DVec3};

use crate::ids::{EdgeId, EntityId, FaceId, VertexId};
use crate::mass::symmetric_eigen;
use crate::model::{Guide, Model};
use crate::scene::{MeshId, Scene};

/// The deepest nesting of instances a path or an instance's triangles follow.
const MAX_DEPTH: usize = 64;
/// Facets whose normals have a smaller second moment (of their total) along the second-largest
/// axis are flat, not round.
const FLAT: f64 = 1e-6;
/// A cylinder's facet normals lie in one plane; the third moment of their total beyond this is
/// a surface curved both ways (a sphere, a cone, a torus).
const NOT_CYLINDER: f64 = 1e-4;

/// Why a measurement could not be made.
#[derive(Clone, Debug, PartialEq)]
pub enum MeasureError {
    /// A named entity does not exist (or a path step is not where it should be).
    Missing(EntityId),
    /// The request does not make sense for the entities named.
    Invalid(String),
}

fn invalid<T>(m: impl Into<String>) -> Result<T, MeasureError> {
    Err(MeasureError::Invalid(m.into()))
}

type Tri = [DVec3; 3];

/// What is measured: a point, a segment or an infinite line (`p + d t` for `t` in `lo..=hi`),
/// or a set of triangles (a face, or everything in an instance).
#[derive(Clone, Debug)]
pub enum Item {
    Point(DVec3),
    Seg {
        p: DVec3,
        d: DVec3,
        lo: f64,
        hi: f64,
    },
    Tris(Vec<Tri>),
}

impl Item {
    fn segment(a: DVec3, b: DVec3) -> Item {
        Item::Seg {
            p: a,
            d: b - a,
            lo: 0.0,
            hi: 1.0,
        }
    }
}

/// The minimum distance and the closest point on each of the two items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Closest {
    pub distance: f64,
    pub a: DVec3,
    pub b: DVec3,
}

impl Closest {
    fn new(a: DVec3, b: DVec3) -> Closest {
        Closest {
            distance: a.distance(b),
            a,
            b,
        }
    }

    fn swapped(self) -> Closest {
        Closest {
            distance: self.distance,
            a: self.b,
            b: self.a,
        }
    }

    fn min(self, other: Closest) -> Closest {
        if other.distance < self.distance {
            other
        } else {
            self
        }
    }
}

const NONE: Closest = Closest {
    distance: f64::INFINITY,
    a: DVec3::ZERO,
    b: DVec3::ZERO,
};

// ---- primitives ---------------------------------------------------------------------------

fn seg_point(p: DVec3, d: DVec3, lo: f64, hi: f64, q: DVec3) -> DVec3 {
    let dd = d.dot(d);
    let t = if dd > 0.0 {
        ((q - p).dot(d) / dd).clamp(lo, hi)
    } else {
        0.0
    };
    p + d * t
}

/// The closest point of a triangle to `q`.
fn tri_point(t: &Tri, q: DVec3) -> DVec3 {
    let [a, b, c] = *t;
    let ab = b - a;
    let ac = c - a;
    let ap = q - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = q - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = q - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
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

/// Closest points of two segments or lines (`p + d t`, `t` in `lo..=hi`).
fn seg_seg(s1: (DVec3, DVec3, f64, f64), s2: (DVec3, DVec3, f64, f64)) -> Closest {
    let (p1, d1, lo1, hi1) = s1;
    let (p2, d2, lo2, hi2) = s2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-300 && e <= 1e-300 {
        s = 0.0;
        t = 0.0;
    } else if a <= 1e-300 {
        s = 0.0;
        t = (f / e).clamp(lo2, hi2);
    } else {
        let c = d1.dot(r);
        if e <= 1e-300 {
            t = 0.0;
            s = (-c / a).clamp(lo1, hi1);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > 1e-12 * a * e {
                ((b * f - c * e) / denom).clamp(lo1, hi1)
            } else {
                0.0f64.clamp(lo1, hi1)
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < lo2 || t0 > hi2 {
                t0 = t0.clamp(lo2, hi2);
                s0 = ((b * t0 - c) / a).clamp(lo1, hi1);
            }
            s = s0;
            t = t0;
        }
    }
    Closest::new(p1 + d1 * s, p2 + d2 * t)
}

/// Whether `x`, in the plane of the triangle, lies inside it (within rounding).
fn in_tri(t: &Tri, x: DVec3) -> bool {
    let [a, b, c] = *t;
    let n = (b - a).cross(c - a);
    let tol = -1e-12 * n.length_squared();
    n.dot((b - a).cross(x - a)) >= tol
        && n.dot((c - b).cross(x - b)) >= tol
        && n.dot((a - c).cross(x - c)) >= tol
}

/// A segment or line against a triangle.
fn seg_tri(p: DVec3, d: DVec3, lo: f64, hi: f64, t: &Tri) -> Closest {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let dn = n.dot(d);
    if dn.abs() > 1e-14 * n.length() * d.length() {
        let s = n.dot(t[0] - p) / dn;
        if s >= lo && s <= hi {
            let x = p + d * s;
            if in_tri(t, x) {
                return Closest::new(x, x);
            }
        }
    }
    let mut best = NONE;
    for (s, ok) in [(lo, lo.is_finite()), (hi, hi.is_finite())] {
        if ok {
            let q = p + d * s;
            best = best.min(Closest::new(q, tri_point(t, q)));
        }
    }
    for i in 0..3 {
        let (a, b) = (t[i], t[(i + 1) % 3]);
        best = best.min(seg_seg((p, d, lo, hi), (a, b - a, 0.0, 1.0)));
    }
    best
}

fn tri_tri(x: &Tri, y: &Tri) -> Closest {
    let mut best = NONE;
    for i in 0..3 {
        let (a, b) = (x[i], x[(i + 1) % 3]);
        best = best.min(seg_tri(a, b - a, 0.0, 1.0, y));
        let (a, b) = (y[i], y[(i + 1) % 3]);
        best = best.min(seg_tri(a, b - a, 0.0, 1.0, x).swapped());
    }
    best
}

// ---- bounding volume hierarchy -------------------------------------------------------------

#[derive(Clone, Copy)]
struct Bx {
    lo: DVec3,
    hi: DVec3,
}

impl Bx {
    fn of(t: &Tri) -> Bx {
        Bx {
            lo: t[0].min(t[1]).min(t[2]),
            hi: t[0].max(t[1]).max(t[2]),
        }
    }

    fn union(self, o: Bx) -> Bx {
        Bx {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
        }
    }

    fn gap(self, o: Bx) -> f64 {
        let g = (self.lo - o.hi).max(o.lo - self.hi).max(DVec3::ZERO);
        g.length()
    }

    fn gap_to_point(self, p: DVec3) -> f64 {
        (self.lo - p).max(p - self.hi).max(DVec3::ZERO).length()
    }
}

struct Node {
    b: Bx,
    /// Triangles `start..end` of the order when a leaf, else the two children.
    kind: NodeKind,
}

enum NodeKind {
    Leaf(usize, usize),
    Inner(usize, usize),
}

struct Bvh<'a> {
    tris: &'a [Tri],
    order: Vec<usize>,
    nodes: Vec<Node>,
}

const LEAF: usize = 4;

impl<'a> Bvh<'a> {
    fn new(tris: &'a [Tri]) -> Bvh<'a> {
        let mut bvh = Bvh {
            tris,
            order: (0..tris.len()).collect(),
            nodes: Vec::new(),
        };
        if !tris.is_empty() {
            bvh.build(0, tris.len());
        }
        bvh
    }

    fn build(&mut self, start: usize, end: usize) -> usize {
        let b = self.order[start..end]
            .iter()
            .map(|&i| Bx::of(&self.tris[i]))
            .reduce(Bx::union)
            .unwrap();
        let at = self.nodes.len();
        self.nodes.push(Node {
            b,
            kind: NodeKind::Leaf(start, end),
        });
        if end - start <= LEAF {
            return at;
        }
        let ext = b.hi - b.lo;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        let tris = self.tris;
        let centre = |i: &usize| {
            let t = &tris[*i];
            (t[0] + t[1] + t[2])[axis]
        };
        self.order[start..end].sort_by(|x, y| centre(x).total_cmp(&centre(y)));
        let mid = (start + end) / 2;
        let l = self.build(start, mid);
        let r = self.build(mid, end);
        self.nodes[at].kind = NodeKind::Inner(l, r);
        at
    }

    fn nearest_point(&self, q: DVec3) -> Closest {
        let mut best = NONE;
        if !self.nodes.is_empty() {
            self.point_rec(0, q, &mut best);
        }
        best
    }

    fn point_rec(&self, n: usize, q: DVec3, best: &mut Closest) {
        let node = &self.nodes[n];
        if node.b.gap_to_point(q) >= best.distance {
            return;
        }
        match node.kind {
            NodeKind::Leaf(s, e) => {
                for &i in &self.order[s..e] {
                    let c = tri_point(&self.tris[i], q);
                    *best = (*best).min(Closest::new(q, c));
                }
            }
            NodeKind::Inner(l, r) => {
                let (gl, gr) = (
                    self.nodes[l].b.gap_to_point(q),
                    self.nodes[r].b.gap_to_point(q),
                );
                let (first, second) = if gl <= gr { (l, r) } else { (r, l) };
                self.point_rec(first, q, best);
                self.point_rec(second, q, best);
            }
        }
    }

    fn pair_rec(&self, n: usize, other: &Bvh, m: usize, best: &mut Closest) {
        let (na, nb) = (&self.nodes[n], &other.nodes[m]);
        if na.b.gap(nb.b) >= best.distance || best.distance == 0.0 {
            return;
        }
        match (&na.kind, &nb.kind) {
            (NodeKind::Leaf(s1, e1), NodeKind::Leaf(s2, e2)) => {
                for &i in &self.order[*s1..*e1] {
                    for &j in &other.order[*s2..*e2] {
                        *best = (*best).min(tri_tri(&self.tris[i], &other.tris[j]));
                    }
                }
            }
            (NodeKind::Inner(l, r), NodeKind::Leaf(..)) => {
                self.pair_pick(*l, *r, other, m, best, true);
            }
            (NodeKind::Leaf(..), NodeKind::Inner(l, r)) => {
                self.pair_pick(*l, *r, other, n, best, false);
            }
            (NodeKind::Inner(l1, r1), NodeKind::Inner(l2, r2)) => {
                // Split the node with the larger box.
                let vol = |b: Bx| {
                    let e = b.hi - b.lo;
                    e.x.max(e.y).max(e.z)
                };
                if vol(na.b) >= vol(nb.b) {
                    self.pair_pick(*l1, *r1, other, m, best, true);
                } else {
                    self.pair_pick(*l2, *r2, other, n, best, false);
                }
            }
        }
    }

    /// Visits the two children of one side against the fixed node of the other, nearer first.
    /// `split_self` says which tree the children belong to.
    fn pair_pick(
        &self,
        l: usize,
        r: usize,
        other: &Bvh,
        fixed: usize,
        best: &mut Closest,
        split_self: bool,
    ) {
        if split_self {
            let (gl, gr) = (
                self.nodes[l].b.gap(other.nodes[fixed].b),
                self.nodes[r].b.gap(other.nodes[fixed].b),
            );
            let (f, s) = if gl <= gr { (l, r) } else { (r, l) };
            self.pair_rec(f, other, fixed, best);
            self.pair_rec(s, other, fixed, best);
        } else {
            let (gl, gr) = (
                other.nodes[l].b.gap(self.nodes[fixed].b),
                other.nodes[r].b.gap(self.nodes[fixed].b),
            );
            let (f, s) = if gl <= gr { (l, r) } else { (r, l) };
            self.pair_rec(fixed, other, f, best);
            self.pair_rec(fixed, other, s, best);
        }
    }
}

/// The distance between two items and the closest point on each.
pub fn distance(a: &Item, b: &Item) -> Closest {
    use Item::*;
    match (a, b) {
        (Point(p), Point(q)) => Closest::new(*p, *q),
        (Point(p), Seg { p: sp, d, lo, hi }) => Closest::new(*p, seg_point(*sp, *d, *lo, *hi, *p)),
        (Point(p), Tris(t)) => Bvh::new(t).nearest_point(*p),
        (Seg { .. } | Tris(_), Point(_)) => distance(b, a).swapped(),
        (
            Seg {
                p: p1,
                d: d1,
                lo: l1,
                hi: h1,
            },
            Seg {
                p: p2,
                d: d2,
                lo: l2,
                hi: h2,
            },
        ) => seg_seg((*p1, *d1, *l1, *h1), (*p2, *d2, *l2, *h2)),
        (Seg { p, d, lo, hi }, Tris(t)) => {
            let mut best = NONE;
            let bvh = Bvh::new(t);
            // Only the bounds of finite segments prune; lines test every triangle.
            for tri in bvh.tris {
                if best.distance == 0.0 {
                    break;
                }
                best = best.min(seg_tri(*p, *d, *lo, *hi, tri));
            }
            best
        }
        (Tris(_), Seg { .. }) => distance(b, a).swapped(),
        (Tris(x), Tris(y)) => {
            let (bx, by) = (Bvh::new(x), Bvh::new(y));
            let mut best = NONE;
            if !x.is_empty() && !y.is_empty() {
                bx.pair_rec(0, &by, 0, &mut best);
            }
            best
        }
    }
}

// ---- entities ------------------------------------------------------------------------------

/// An entity found by a path: the mesh holding it, where that mesh stands in the starting
/// mesh's coordinates, and the entity itself.
#[derive(Clone, Copy, Debug)]
pub struct Located {
    pub mesh: MeshId,
    pub place: DAffine3,
    pub id: EntityId,
}

/// Follows `path` (instances of the mesh `start`, each in the one before, then the entity in
/// the last one's mesh; one id alone names an entity of `start`).
pub fn locate(scene: &Scene, start: MeshId, path: &[EntityId]) -> Result<Located, MeasureError> {
    let Some((last, steps)) = path.split_last() else {
        return invalid("an empty path names nothing");
    };
    if steps.len() > MAX_DEPTH {
        return invalid("the path is too deep");
    }
    let mut mesh = start;
    let mut place = DAffine3::IDENTITY;
    for step in steps {
        let EntityId::Instance(i) = step else {
            return invalid(format!(
                "{step} is not a group or component, so it holds nothing"
            ));
        };
        let inst = scene
            .mesh(mesh)
            .and_then(|m| m.instance(*i))
            .ok_or(MeasureError::Missing(*step))?;
        place *= inst.transform;
        mesh = MeshId::Definition(inst.definition);
    }
    let m = scene.mesh(mesh).ok_or(MeasureError::Missing(*last))?;
    let exists = match *last {
        EntityId::Vertex(v) => m.vertex(v).is_some(),
        EntityId::Edge(e) => m.edge(e).is_some(),
        EntityId::Face(f) => m.face(f).is_some(),
        EntityId::Instance(i) => m.instance(i).is_some(),
        EntityId::Guide(g) => m.guide(g).is_some(),
        _ => return invalid(format!("{last} cannot be measured")),
    };
    if !exists {
        return Err(MeasureError::Missing(*last));
    }
    Ok(Located {
        mesh,
        place,
        id: *last,
    })
}

/// The triangles of `mesh` and the instances in it, placed by `place`; faces that cannot be
/// triangulated go in `skipped` as paths from `path` ("i4/i7/f12").
fn mesh_triangles(
    scene: &Scene,
    mesh: MeshId,
    place: DAffine3,
    (depth, path): (usize, &str),
    out: &mut Vec<Tri>,
    skipped: &mut Vec<String>,
) -> Result<(), MeasureError> {
    if depth > MAX_DEPTH {
        return invalid("instances are nested too deep");
    }
    let Some(m) = scene.mesh(mesh) else {
        return Ok(());
    };
    for (id, _) in m.faces() {
        match m.triangulate(id) {
            Some(tris) => out.extend(
                tris.into_iter()
                    .map(|t| t.map(|p| place.transform_point3(p))),
            ),
            None => skipped.push(format!("{path}/f{}", id.0)),
        }
    }
    for (id, inst) in m.instances() {
        mesh_triangles(
            scene,
            MeshId::Definition(inst.definition),
            place * inst.transform,
            (depth + 1, &format!("{path}/i{}", id.0)),
            out,
            skipped,
        )?;
    }
    Ok(())
}

/// The geometry of a located entity, in the starting mesh's coordinates. Faces of an instance
/// that cannot be triangulated are left out and named in `skipped`.
pub fn item(scene: &Scene, at: &Located, skipped: &mut Vec<String>) -> Result<Item, MeasureError> {
    let m = scene.mesh(at.mesh).ok_or(MeasureError::Missing(at.id))?;
    let w = |p: DVec3| at.place.transform_point3(p);
    Ok(match at.id {
        EntityId::Vertex(v) => Item::Point(w(vertex(m, v)?)),
        EntityId::Edge(e) => {
            let [a, b] = edge_ends(m, e)?;
            Item::segment(w(a), w(b))
        }
        EntityId::Face(f) => {
            let tris: Vec<Tri> = m
                .triangulate(f)
                .ok_or_else(|| MeasureError::Invalid(format!("{} cannot be triangulated", at.id)))?
                .into_iter()
                .map(|t| t.map(w))
                .collect();
            if tris.is_empty() {
                return invalid(format!("{} cannot be triangulated", at.id));
            }
            Item::Tris(tris)
        }
        EntityId::Instance(i) => {
            let inst = m.instance(i).ok_or(MeasureError::Missing(at.id))?;
            let mut tris = Vec::new();
            mesh_triangles(
                scene,
                MeshId::Definition(inst.definition),
                at.place * inst.transform,
                (0, &at.id.to_string()),
                &mut tris,
                skipped,
            )?;
            if tris.is_empty() {
                return invalid(format!("{} holds no faces", at.id));
            }
            Item::Tris(tris)
        }
        EntityId::Guide(g) => match *m.guide(g).ok_or(MeasureError::Missing(at.id))? {
            Guide::Point { position, .. } => Item::Point(w(position)),
            Guide::Line {
                point, direction, ..
            } => Item::Seg {
                p: w(point),
                d: at.place.matrix3 * direction,
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            },
        },
        _ => return invalid(format!("{} cannot be measured", at.id)),
    })
}

fn vertex(m: &Model, v: VertexId) -> Result<DVec3, MeasureError> {
    m.position(v)
        .ok_or(MeasureError::Missing(EntityId::Vertex(v)))
}

fn edge_ends(m: &Model, e: EdgeId) -> Result<[DVec3; 2], MeasureError> {
    let [a, b] = m.edge(e).ok_or(MeasureError::Missing(EntityId::Edge(e)))?.v;
    Ok([vertex(m, a)?, vertex(m, b)?])
}

/// The minimum distance between two entities, and the faces of instances among them that
/// could not be triangulated and were left out. Instances are measured between their
/// surfaces: one inside another is the gap between the inner one's faces and the outer one's.
pub fn measure_distance(
    scene: &Scene,
    start: MeshId,
    a: &[EntityId],
    b: &[EntityId],
) -> Result<(Closest, Vec<String>), MeasureError> {
    let (la, lb) = (locate(scene, start, a)?, locate(scene, start, b)?);
    let mut skipped = Vec::new();
    let (ia, ib) = (
        item(scene, &la, &mut skipped)?,
        item(scene, &lb, &mut skipped)?,
    );
    Ok((distance(&ia, &ib), skipped))
}

/// An angle between two directions, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angle {
    /// 0 to 180: between the edges' directions (first vertex to second) or the faces' normals.
    pub degrees: f64,
    /// 0 to 90: between the lines or planes, whichever way they point.
    pub acute: f64,
}

/// The angle between two edges or two faces.
pub fn measure_angle(
    scene: &Scene,
    start: MeshId,
    a: &[EntityId],
    b: &[EntityId],
) -> Result<Angle, MeasureError> {
    let (la, lb) = (locate(scene, start, a)?, locate(scene, start, b)?);
    let dir = |l: &Located| -> Result<(DVec3, &'static str), MeasureError> {
        let m = scene.mesh(l.mesh).ok_or(MeasureError::Missing(l.id))?;
        match l.id {
            EntityId::Edge(e) => {
                let [p, q] = edge_ends(m, e)?;
                Ok((l.place.matrix3 * (q - p), "edge"))
            }
            EntityId::Face(f) => {
                let n = m.face(f).ok_or(MeasureError::Missing(l.id))?.normal;
                Ok((l.place.matrix3.inverse().transpose() * n, "face"))
            }
            _ => invalid(format!("{} is not an edge or a face", l.id)),
        }
    };
    let ((u, ku), (v, kv)) = (dir(&la)?, dir(&lb)?);
    if ku != kv {
        return invalid("an angle is between two edges or two faces, not one of each");
    }
    let (Some(u), Some(v)) = (u.try_normalize(), v.try_normalize()) else {
        return invalid("a direction is degenerate (zero length)");
    };
    // atan2 keeps small and near-straight angles precise, where acos loses digits.
    let degrees = u.cross(v).length().atan2(u.dot(v)).to_degrees();
    Ok(Angle {
        degrees,
        acute: degrees.min(180.0 - degrees),
    })
}

// ---- round features ------------------------------------------------------------------------

/// What a round feature is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundKind {
    /// A circle or arc: a curve or the outline of a flat disc.
    Circle,
    /// A cylinder (or part of one) of soft-connected facets.
    Cylinder,
}

/// A circle or cylinder fitted to a round feature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Round {
    pub kind: RoundKind,
    /// The circle's center; the middle of the cylinder's axis segment.
    pub center: DVec3,
    /// The unit axis (the circle's normal), pointing up (positive z, else y, else x).
    pub axis: DVec3,
    pub radius: f64,
    /// A cylinder's extent along its axis.
    pub length: Option<f64>,
    /// The largest distance of a vertex from the fitted shape.
    pub max_deviation: f64,
    /// Vertices fitted.
    pub samples: usize,
}

fn upward(axis: DVec3) -> DVec3 {
    for c in [2, 1, 0] {
        if axis[c].abs() > 1e-9 {
            return if axis[c] > 0.0 { axis } else { -axis };
        }
    }
    axis
}

fn basis(axis: DVec3) -> (DVec3, DVec3) {
    let helper = if axis.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let u = helper.cross(axis).normalize();
    (u, axis.cross(u))
}

/// Least-squares circle through 2D points: algebraic fit refined to the geometric optimum.
/// Returns the center and radius.
fn fit_circle_2d(pts: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    let n = pts.len() as f64;
    if pts.len() < 3 {
        return None;
    }
    let (mx, my) = pts.iter().fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
    let (mx, my) = (mx / n, my / n);
    // Minimize sum (x^2 + y^2 + a x + b y + c)^2 on centered coordinates.
    let mut m = DMat3::ZERO;
    let mut rhs = DVec3::ZERO;
    let mut scale: f64 = 0.0;
    for p in pts {
        let (x, y) = (p.0 - mx, p.1 - my);
        scale = scale.max(x.abs()).max(y.abs());
    }
    if scale <= 0.0 {
        return None;
    }
    for p in pts {
        let (x, y) = ((p.0 - mx) / scale, (p.1 - my) / scale);
        let row = DVec3::new(x, y, 1.0);
        m += DMat3::from_cols(row * row.x, row * row.y, row * row.z);
        rhs -= row * (x * x + y * y);
    }
    if m.determinant().abs() < 1e-14 * n * n * n {
        return None;
    }
    let sol = m.inverse() * rhs;
    let (mut cx, mut cy) = (-sol.x / 2.0, -sol.y / 2.0);
    let r2 = cx * cx + cy * cy - sol.z;
    if r2.is_nan() || r2 <= 0.0 {
        return None;
    }
    cx = cx * scale + mx;
    cy = cy * scale + my;
    // Geometric refinement (fixed point on the center).
    let mut r = 0.0;
    for _ in 0..200 {
        let ds: Vec<f64> = pts
            .iter()
            .map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt())
            .collect();
        if ds.iter().any(|d| *d <= 0.0) {
            break;
        }
        r = ds.iter().sum::<f64>() / n;
        let ux: f64 = pts
            .iter()
            .zip(&ds)
            .map(|(p, d)| (cx - p.0) / d)
            .sum::<f64>()
            / n;
        let uy: f64 = pts
            .iter()
            .zip(&ds)
            .map(|(p, d)| (cy - p.1) / d)
            .sum::<f64>()
            / n;
        let (nx, ny) = (mx + r * ux, my + r * uy);
        let step = (nx - cx).hypot(ny - cy);
        cx = nx;
        cy = ny;
        if step <= 1e-14 * r.max(1e-300) {
            break;
        }
    }
    let r = if r > 0.0 {
        pts.iter()
            .map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt())
            .sum::<f64>()
            / n
    } else {
        r2.sqrt() * scale
    };
    Some((cx, cy, r))
}

/// A circle fitted to points in space: the plane through them, then the circle in it.
pub fn fit_circle(points: &[DVec3]) -> Result<Round, String> {
    if points.len() < 3 {
        return Err("a circle needs at least three points".into());
    }
    let n = points.len() as f64;
    let c0 = points.iter().copied().sum::<DVec3>() / n;
    let mut cov = DMat3::ZERO;
    for p in points {
        let d = *p - c0;
        cov += DMat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    let (vals, vecs) = symmetric_eigen(cov);
    if vals[1] <= 1e-12 * vals[2].max(f64::MIN_POSITIVE) {
        return Err("the points lie on a line".into());
    }
    let axis = vecs[0];
    let (u, v) = basis(axis);
    let flat: Vec<(f64, f64)> = points
        .iter()
        .map(|p| ((*p - c0).dot(u), (*p - c0).dot(v)))
        .collect();
    let (cx, cy, r) = fit_circle_2d(&flat).ok_or("the points do not lie on a circle")?;
    let center = c0 + u * cx + v * cy;
    let dev = points
        .iter()
        .map(|p| {
            let d = *p - center;
            let h = d.dot(axis);
            let planar = (d - axis * h).length();
            ((planar - r).powi(2) + h * h).sqrt()
        })
        .fold(0.0, f64::max);
    Ok(Round {
        kind: RoundKind::Circle,
        center,
        axis: upward(axis),
        radius: r,
        length: None,
        max_deviation: dev,
        samples: points.len(),
    })
}

/// A cylinder fitted to facet vertices and facet normals: the axis is the direction the normals
/// are perpendicular to, the circle is fitted to the vertices projected along it.
pub fn fit_cylinder(points: &[DVec3], normals: &[DVec3]) -> Result<Round, String> {
    let mut cov = DMat3::ZERO;
    for n in normals {
        cov += DMat3::from_cols(*n * n.x, *n * n.y, *n * n.z);
    }
    let (vals, vecs) = symmetric_eigen(cov);
    let total: f64 = vals.iter().sum();
    if total <= 0.0 || vals[1] <= FLAT * total {
        return Err("the face is flat, not round".into());
    }
    if vals[0] > NOT_CYLINDER * total {
        return Err("the surface curves in more than one direction (not a cylinder)".into());
    }
    let axis = vecs[0];
    let (u, v) = basis(axis);
    let flat: Vec<(f64, f64)> = points.iter().map(|p| (p.dot(u), p.dot(v))).collect();
    let (cx, cy, r) = fit_circle_2d(&flat).ok_or("the vertices do not lie on a circle")?;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in points {
        let h = p.dot(axis);
        lo = lo.min(h);
        hi = hi.max(h);
    }
    let mid = (lo + hi) / 2.0;
    let dev = flat
        .iter()
        .map(|p| (((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt() - r).abs())
        .fold(0.0, f64::max);
    Ok(Round {
        kind: RoundKind::Cylinder,
        center: u * cx + v * cy + axis * mid,
        axis: upward(axis),
        radius: r,
        length: Some(hi - lo),
        max_deviation: dev,
        samples: points.len(),
    })
}

/// The faces joined to `start` through soft edges, `start` first.
fn soft_group(m: &Model, start: FaceId) -> Vec<FaceId> {
    let mut seen = BTreeSet::from([start]);
    let mut out = vec![start];
    let mut i = 0;
    while i < out.len() {
        for e in m.face_edges(out[i]) {
            if !m.edge(e).is_some_and(|e| e.soft) {
                continue;
            }
            if let Some(pair) = m.faces_beside(e) {
                for f in pair {
                    if seen.insert(f) {
                        out.push(f);
                    }
                }
            }
        }
        i += 1;
    }
    out
}

fn curve_circle(m: &Model, place: &DAffine3, edge: EdgeId) -> Result<Round, MeasureError> {
    let curve = m.curve_of(edge).ok_or_else(|| {
        MeasureError::Invalid(format!("e{} is not part of a circle or arc", edge.0))
    })?;
    if curve.polygon {
        return invalid(format!("e{} is a side of a polygon, not a circle", edge.0));
    }
    let all: Vec<DVec3> = curve
        .vertices
        .iter()
        .map(|v| vertex(m, *v))
        .collect::<Result<_, _>>()?;
    // Only the corners (and an arc's ends) lie on the circle: a vertex splitting a side, where
    // other geometry meets it, lies on the chord.
    let mut key = crate::curves::corners(&all, curve.closed);
    if !curve.closed {
        key.insert(0, 0);
        key.push(all.len() - 1);
    }
    let pts: Vec<DVec3> = if key.len() >= 3 {
        key.iter()
            .map(|&i| place.transform_point3(all[i]))
            .collect()
    } else {
        all.iter().map(|p| place.transform_point3(*p)).collect()
    };
    fit_circle(&pts).map_err(MeasureError::Invalid)
}

/// Fits the round feature a face or an edge lies on: a circle for a curve's edge or a flat
/// disc, a cylinder for a face joined by soft edges to others around one axis.
pub fn measure_round(
    scene: &Scene,
    start: MeshId,
    path: &[EntityId],
) -> Result<Round, MeasureError> {
    let at = locate(scene, start, path)?;
    let m = scene.mesh(at.mesh).ok_or(MeasureError::Missing(at.id))?;
    let normal_map = at.place.matrix3.inverse().transpose();
    let face_round = |f: FaceId| -> Result<Round, MeasureError> {
        let group = soft_group(m, f);
        if group.len() == 1 {
            let face = m.face(f).ok_or(MeasureError::Missing(EntityId::Face(f)))?;
            let ring: Vec<EdgeId> = (0..face.outer.len())
                .filter_map(|i| {
                    m.edge_between(face.outer[i], face.outer[(i + 1) % face.outer.len()])
                })
                .collect();
            let curve = ring.first().and_then(|e| m.curve_of(*e));
            let whole = curve.as_ref().is_some_and(|c| {
                c.closed
                    && !c.polygon
                    && c.edges.len() == ring.len()
                    && c.edges.iter().all(|e| ring.contains(e))
            });
            if whole {
                return curve_circle(m, &at.place, ring[0]);
            }
            return invalid(format!(
                "f{} is not on a round surface (no soft-connected facets and its outline is not a circle)",
                f.0
            ));
        }
        // Fit the ends of the facet generator lines: soft edges between two facets of the group
        // that turn. Other vertices (where a cross-bore or another cut meets a facet) lie on
        // the flat facets, inside the cylinder.
        let members: BTreeSet<FaceId> = group.iter().copied().collect();
        let mut verts = BTreeSet::new();
        let mut all = BTreeSet::new();
        let mut normals = Vec::new();
        for g in &group {
            let face = m
                .face(*g)
                .ok_or(MeasureError::Missing(EntityId::Face(*g)))?;
            for l in std::iter::once(&face.outer).chain(&face.holes) {
                all.extend(l.iter().copied());
            }
            if let Some(n) = (normal_map * face.normal).try_normalize() {
                normals.push(n);
            }
            for e in m.face_edges(*g) {
                let Some(edge) = m.edge(e).filter(|e| e.soft) else {
                    continue;
                };
                let Some([a, b]) = m.faces_beside(e) else {
                    continue;
                };
                let turns = match (m.face(a), m.face(b)) {
                    (Some(fa), Some(fb)) => fa.normal.cross(fb.normal).length() > 1e-9,
                    _ => false,
                };
                if turns && members.contains(&a) && members.contains(&b) {
                    verts.extend(edge.v);
                }
            }
        }
        if verts.len() < 3 {
            verts = all;
        }
        let pts: Vec<DVec3> = verts
            .into_iter()
            .map(|v| vertex(m, v).map(|p| at.place.transform_point3(p)))
            .collect::<Result<_, _>>()?;
        fit_cylinder(&pts, &normals).map_err(MeasureError::Invalid)
    };
    match at.id {
        EntityId::Face(f) => face_round(f),
        EntityId::Edge(e) => {
            if m.edge(e).is_some_and(|x| x.curve.is_some()) {
                curve_circle(m, &at.place, e)
            } else if m.edge(e).is_some_and(|x| x.soft) {
                let [f, _] = m.faces_beside(e).ok_or_else(|| {
                    MeasureError::Invalid(format!("e{} does not lie between two faces", e.0))
                })?;
                face_round(f)
            } else {
                invalid(format!("e{} is not part of a circle or arc", e.0))
            }
        }
        _ => invalid(format!("{} is not a face or an edge", at.id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn seg(a: DVec3, b: DVec3) -> Item {
        Item::segment(a, b)
    }

    #[test]
    fn skew_segments_have_interior_closest_points() {
        let c = distance(
            &seg(v(0.0, 0.0, 0.0), v(2.0, 0.0, 0.0)),
            &seg(v(1.0, -1.0, 1.0), v(1.0, 1.0, 1.0)),
        );
        assert!((c.distance - 1.0).abs() < 1e-12);
        assert!((c.a - v(1.0, 0.0, 0.0)).length() < 1e-12);
        assert!((c.b - v(1.0, 0.0, 1.0)).length() < 1e-12);
    }

    #[test]
    fn parallel_and_collinear_segments() {
        let c = distance(
            &seg(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0)),
            &seg(v(3.0, 0.0, 0.0), v(5.0, 0.0, 0.0)),
        );
        assert!((c.distance - 2.0).abs() < 1e-12);
        let c = distance(
            &seg(v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0)),
            &seg(v(1.0, 2.0, 0.0), v(3.0, 2.0, 0.0)),
        );
        assert!((c.distance - 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_guide_line_is_infinite() {
        let line = Item::Seg {
            p: v(0.0, 0.0, 0.0),
            d: v(1.0, 0.0, 0.0),
            lo: f64::NEG_INFINITY,
            hi: f64::INFINITY,
        };
        let c = distance(&line, &Item::Point(v(1000.0, 3.0, 4.0)));
        assert!((c.distance - 5.0).abs() < 1e-9);
        let c = distance(&line, &seg(v(5.0, 1.0, 2.0), v(6.0, 1.0, 2.0)));
        assert!((c.distance - 5.0f64.sqrt()).abs() < 1e-12);
        let tri = Item::Tris(vec![[v(0.0, 1.0, 1.0), v(0.0, 3.0, 1.0), v(0.0, 1.0, 4.0)]]);
        let c = distance(&line, &tri);
        assert!((c.distance - 2.0f64.sqrt()).abs() < 1e-12, "{c:?}");
    }

    #[test]
    fn segment_through_a_triangle_is_zero_and_beside_is_the_rim() {
        let tri = [v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0), v(0.0, 4.0, 0.0)];
        let t = Item::Tris(vec![tri]);
        let c = distance(&seg(v(1.0, 1.0, 1.0), v(1.0, 1.0, -1.0)), &t);
        assert_eq!(c.distance, 0.0);
        let c = distance(&seg(v(5.0, 5.0, 1.0), v(6.0, 6.0, 1.0)), &t);
        assert!((c.b - v(2.0, 2.0, 0.0)).length() < 1e-12);
        assert!((c.distance - 19.0f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn crossing_triangles_are_zero_and_the_bvh_matches_brute_force() {
        // Two grids of triangles, one raised: compare the pruned search with all pairs.
        let grid = |z: f64, off: f64, n: usize| -> Vec<Tri> {
            let mut out = Vec::new();
            for i in 0..n {
                for j in 0..n {
                    let (x, y) = (i as f64 + off, j as f64 * 1.3);
                    let a = v(x, y, z + 0.1 * ((i * j) % 3) as f64);
                    out.push([a, a + v(1.0, 0.0, 0.1), a + v(0.0, 1.0, 0.2)]);
                }
            }
            out
        };
        let (x, y) = (grid(0.0, 0.0, 9), grid(2.0, 0.37, 7));
        let c = distance(&Item::Tris(x.clone()), &Item::Tris(y.clone()));
        let mut brute = NONE;
        for a in &x {
            for b in &y {
                brute = brute.min(tri_tri(a, b));
            }
        }
        assert!((c.distance - brute.distance).abs() < 1e-12);
        let crossing = vec![[v(0.5, 0.5, -1.0), v(0.5, 0.5, 1.0), v(0.9, 0.5, 0.0)]];
        let c = distance(&Item::Tris(x), &Item::Tris(crossing));
        assert_eq!(c.distance, 0.0);
    }

    #[test]
    fn circle_fit_recovers_a_tilted_circle_and_reports_error() {
        let axis = v(1.0, 2.0, 2.0).normalize();
        let (u, w) = basis(axis);
        let center = v(1.0, -2.0, 3.0);
        let pts: Vec<DVec3> = (0..24)
            .map(|k| {
                let t = k as f64 * std::f64::consts::TAU / 24.0;
                center + (u * t.cos() + w * t.sin()) * 2.5
            })
            .collect();
        let r = fit_circle(&pts).unwrap();
        assert!((r.radius - 2.5).abs() < 1e-9);
        assert!((r.center - center).length() < 1e-9);
        assert!(r.axis.cross(axis).length() < 1e-9);
        assert!(r.max_deviation < 1e-9);
        // An arc of a quarter fits too.
        let r = fit_circle(&pts[..7]).unwrap();
        assert!((r.radius - 2.5).abs() < 1e-6);
        // A bump shows as deviation.
        let mut bumped = pts.clone();
        bumped[3] += axis * 0.2;
        assert!(fit_circle(&bumped).unwrap().max_deviation > 0.1);
        assert!(fit_circle(&[v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(2.0, 0.0, 0.0)]).is_err());
    }
}
