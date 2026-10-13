//! Moving and copying geometry.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DAffine3, DVec2, DVec3};

use crate::edit::KernelError;
use crate::geom::{
    LENGTH_TOL, Plane, point_in_polygon, project_on_segment, segment_distance_2d, vector_area,
};
use crate::ids::{CurveId, EdgeId, FaceId, MaterialId, VertexId};
use crate::index::{Index, Shape};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Moves `vertices` by `offset` as one ChangeSet. Faces that use some of them stretch and
    /// may tilt, but must stay flat and keep their winding; edges between moved and fixed
    /// vertices must keep a length; no moved vertex may land on a fixed one. Moved geometry is
    /// not merged with what it ends up touching.
    pub fn move_vertices(
        &mut self,
        vertices: &BTreeSet<VertexId>,
        offset: DVec3,
    ) -> Result<ChangeSet, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        self.transform_vertices(vertices, DAffine3::from_translation(offset))
    }

    /// Moves `vertices` by the rigid motion `t` (a rotation and a translation) as one
    /// ChangeSet, under the rules of `move_vertices`. Faces that move whole turn with `t`.
    /// Vertices `t` leaves in place (on a rotation's axis) are not reported, and a motion that
    /// moves nothing changes nothing.
    pub fn transform_vertices(
        &mut self,
        vertices: &BTreeSet<VertexId>,
        t: DAffine3,
    ) -> Result<ChangeSet, KernelError> {
        if !t.is_finite() {
            return Err(KernelError::NotFinite);
        }
        let moved: BTreeSet<VertexId> = vertices
            .iter()
            .copied()
            .filter(|v| self.vertex(*v).is_some())
            .collect();
        debug_assert!(
            {
                let m = t.matrix3;
                (m.transpose() * m).abs_diff_eq(glam::DMat3::IDENTITY, 1e-9)
            },
            "transform_vertices needs a rigid motion"
        );
        // A vertex a rotation leaves in place (on its axis) stays exactly where it is, not a
        // rounding error away. A pure translation moves every vertex by its full offset.
        let translation = t.matrix3 == glam::DMat3::IDENTITY;
        let at = |m: &Model, v: VertexId| {
            if moved.contains(&v) {
                let p = m.pos(v);
                let q = t.transform_point3(p);
                if !translation && q.distance(p) <= 1e-12 * p.length().max(1.0) {
                    p
                } else {
                    q
                }
            } else {
                m.pos(v)
            }
        };

        for (_, e) in self.edges() {
            let [a, b] = e.v;
            if moved.contains(&a) != moved.contains(&b)
                && at(self, a).distance(at(self, b)) < LENGTH_TOL
            {
                return Err(KernelError::Squash);
            }
        }
        for &v in &moved {
            let to = at(self, v);
            let lands = self
                .vertices_near(to, LENGTH_TOL)
                .iter()
                .any(|w| !moved.contains(w));
            if lands {
                return Err(KernelError::Squash);
            }
        }
        // Faces that move partly get a new plane; check it and compute their new normals.
        // Faces that move whole keep their shape and turn with `t`.
        let mut retilted: Vec<(FaceId, Face)> = Vec::new();
        for (id, f) in self.faces() {
            let all: Vec<VertexId> = loops(f).flatten().copied().collect();
            let touched = all.iter().filter(|v| moved.contains(v)).count();
            if touched == 0 {
                continue;
            }
            let normal = if touched == all.len() {
                t.transform_vector3(f.normal).normalize()
            } else {
                let outer: Vec<DVec3> = f.outer.iter().map(|v| at(self, *v)).collect();
                let area = vector_area(&outer);
                if area.dot(f.normal) <= LENGTH_TOL * LENGTH_TOL {
                    return Err(KernelError::Squash);
                }
                let normal = area.normalize();
                let origin = outer[0];
                if all
                    .iter()
                    .any(|v| (at(self, *v) - origin).dot(normal).abs() > LENGTH_TOL)
                {
                    return Err(KernelError::Bend(id));
                }
                let mut rings = vec![outer];
                for h in &f.holes {
                    let pts: Vec<DVec3> = h.iter().map(|v| at(self, *v)).collect();
                    if vector_area(&pts).dot(normal) >= 0.0 {
                        return Err(KernelError::Squash);
                    }
                    rings.push(pts);
                }
                if !rings_are_valid(&rings, normal) {
                    return Err(KernelError::Squash);
                }
                normal
            };
            if normal.distance(f.normal) > 1e-12 {
                let mut g = f.clone();
                g.normal = normal;
                retilted.push((id, g));
            }
        }
        for &v in &moved {
            let to = at(self, v);
            if to != self.pos(v) {
                self.move_vertex(v, to);
            }
        }
        for (id, f) in retilted {
            self.replace_face(id, f);
        }
        Ok(self.take_changes())
    }

    /// Copies `edges` and `faces` (with their edges) `count` times, copy k shifted by
    /// `k * offset`, as one ChangeSet. Copies are drawn like new edges, so they merge with
    /// geometry they touch and close loops into faces; each copied face keeps the orientation
    /// of its original, and an opening (a hole with no face in it) stays open unless it is
    /// drawn on an existing face: pieces of an existing face that copies split off stay faces.
    pub fn copy(
        &mut self,
        edges: &[EdgeId],
        faces: &[FaceId],
        offset: DVec3,
        count: u32,
    ) -> Result<ChangeSet, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        let places: Vec<DAffine3> = (1..=count)
            .map(|k| DAffine3::from_translation(offset * k as f64))
            .collect();
        self.copy_placed(edges, faces, &places)
    }

    /// Copies `edges` and `faces` (with their edges) once per rigid motion in `places`, as one
    /// ChangeSet, under the rules of `copy`.
    pub fn copy_placed(
        &mut self,
        edges: &[EdgeId],
        faces: &[FaceId],
        places: &[DAffine3],
    ) -> Result<ChangeSet, KernelError> {
        if !places.iter().all(|t| t.is_finite()) {
            return Err(KernelError::NotFinite);
        }
        let stencil = self.stencil(edges, faces);
        self.draw_stencil(&stencil, places);
        Ok(self.take_changes())
    }

    /// Captures `edges` and `faces` (with their edges) by position.
    pub(crate) fn stencil(&self, edges: &[EdgeId], faces: &[FaceId]) -> Stencil {
        let mut sides: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for e in edges.iter().filter_map(|e| self.edge(*e)) {
            sides.insert((e.v[0].min(e.v[1]), e.v[0].max(e.v[1])));
        }
        let mark = |(a, b): &(VertexId, VertexId)| {
            self.edge_between(*a, *b)
                .and_then(|e| self.edge(e))
                .map_or((None, false), |e| (e.curve, e.soft))
        };
        let ring = |l: &[VertexId]| l.iter().map(|v| self.pos(*v)).collect::<Vec<DVec3>>();
        let mut s = Stencil::default();
        for f in faces.iter().filter_map(|f| self.face(*f)) {
            for l in loops(f) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    sides.insert((a.min(b), a.max(b)));
                }
            }
            let mut face = StencilFace {
                outer: ring(&f.outer),
                normal: f.normal,
                paint: f.paint(),
                openings: Vec::new(),
                filled: Vec::new(),
            };
            for h in &f.holes {
                if self.face_with_outer(&h.iter().copied().collect()).is_none() {
                    face.openings.push(ring(h));
                } else {
                    face.filled.push(ring(h));
                }
            }
            s.faces.push(face);
        }
        s.segments = sides
            .iter()
            .map(|(a, b)| (self.pos(*a), self.pos(*b)))
            .collect();
        s.marks = sides.iter().map(mark).collect();
        s
    }

    /// Draws `stencil` once per placement, without ending the edit. Copies are drawn like new
    /// edges, so they merge with geometry they touch; each face keeps the orientation of its
    /// original, and openings stay open. Drawing adds no faces the stencil does not have, apart
    /// from faces closed together with existing geometry; a region of an existing face that
    /// the drawing splits off stays a face.
    pub(crate) fn draw_stencil(&mut self, stencil: &Stencil, places: &[DAffine3]) {
        let existing: BTreeSet<FaceId> = self.faces().map(|(id, _)| id).collect();
        let segments: Vec<(DVec3, DVec3)> = places
            .iter()
            .flat_map(|t| {
                (stencil.segments.iter())
                    .map(|(a, b)| (t.transform_point3(*a), t.transform_point3(*b)))
            })
            .collect();
        // Existing faces in a plane the drawing reaches, by position: (outer ring, holes).
        let ring = |m: &Model, l: &[VertexId]| l.iter().map(|v| m.pos(*v)).collect::<Vec<_>>();
        let segment_index = Index::of_segments(&segments);
        let before: Vec<(Vec<DVec3>, Vec<Vec<DVec3>>)> = self
            .faces()
            .filter(|(_, f)| {
                let o = self.pos(f.outer[0]);
                let on = |p: DVec3| (p - o).dot(f.normal).abs() < LENGTH_TOL;
                segment_index.any_near_plane(&Plane::through(o, f.normal), |k| {
                    let (a, b) = segments[k.0 as usize];
                    on(a) && on(b)
                })
            })
            .map(|(_, f)| {
                let holes = f.holes.iter().map(|h| ring(self, h)).collect();
                (ring(self, &f.outer), holes)
            })
            .collect();
        let placed: Vec<StencilFace> = places
            .iter()
            .flat_map(|t| stencil.faces.iter().map(|f| f.placed(t)))
            .collect();

        self.draw_segments(&segments);
        // A face whose sides were all there already is not closed by drawing them: close it.
        for f in &placed {
            let ring: Option<Vec<VertexId>> =
                f.outer.iter().map(|p| self.find_vertex(*p)).collect();
            let Some(ring) = ring else { continue };
            if self
                .face_with_outer(&ring.iter().copied().collect())
                .is_some()
            {
                continue;
            }
            let n = ring.len();
            let sides: Option<Vec<EdgeId>> = (0..n)
                .map(|i| self.edge_between(ring[i], ring[(i + 1) % n]))
                .collect();
            if let Some(sides) = sides {
                self.rebuild_plane(Plane::through(self.pos(ring[0]), f.normal), &sides);
            }
        }

        // A new face is a stencil face (or a piece of one that other geometry split) when it
        // lies within a placed face and not within one of that face's openings. Faces within
        // a filled hole count too: the original's hole has a face in it.
        let new: Vec<FaceId> = self
            .faces()
            .map(|(id, _)| id)
            .filter(|id| !existing.contains(id))
            .collect();
        // Each new face is sampled once: a face lies wholly inside or outside every ring
        // tested here (the rings were drawn), so any point inside it decides.
        let samples: BTreeMap<FaceId, FaceSample> = new
            .iter()
            .filter_map(|id| Some((*id, self.face_sample(*id)?)))
            .collect();
        let within =
            |id: FaceId, ring: &Option<RingTest>| samples.get(&id).is_some_and(|s| s.within(ring));
        // The new faces a ring may hold, in id order: those whose sample point lies in the
        // ring's box (a face within the ring has its sample there).
        let sample_index = Index::of_points(samples.iter().map(|(id, s)| (id.0, s.inside)));
        let candidates = |ring: &Option<RingTest>| -> Vec<FaceId> {
            let Some(r) = ring else { return Vec::new() };
            let pad = DVec3::splat(RING_BOX_PAD);
            let mut ids = Vec::new();
            sample_index.near_box(r.min - pad, r.max + pad, |shape| {
                if let Shape::Point(n, _) = shape {
                    ids.push(FaceId(n.0));
                }
            });
            ids.sort();
            ids
        };
        let tests = |rings: &[Vec<DVec3>]| rings.iter().map(|r| RingTest::new(r)).collect();
        let placed_tests: Vec<PlacedTest> = placed
            .iter()
            .map(|f| PlacedTest {
                outer: RingTest::new(&f.outer),
                openings: tests(&f.openings),
                filled: tests(&f.filled),
            })
            .collect();
        let own: BTreeSet<FaceId> = placed_tests
            .iter()
            .flat_map(|f| {
                candidates(&f.outer).into_iter().filter(|id| {
                    within(*id, &f.outer) && !f.openings.iter().any(|h| within(*id, h))
                })
            })
            .collect();
        // Give each piece of a stencil face that face's orientation and materials.
        for (f, t) in placed.iter().zip(&placed_tests) {
            for id in candidates(&t.outer) {
                let in_hole = t.openings.iter().chain(&t.filled).any(|h| within(id, h));
                if within(id, &t.outer) && !in_hole {
                    self.orient(id, f.normal);
                    self.set_paint(id, f.paint);
                }
            }
        }
        // A new face within an existing face (and not within one of its holes) is a piece of
        // that face the drawing split off.
        let before: Vec<(Option<RingTest>, Vec<Option<RingTest>>)> = before
            .iter()
            .map(|(outer, holes)| (RingTest::new(outer), tests(holes)))
            .collect();
        let split_off: BTreeSet<FaceId> = before
            .iter()
            .flat_map(|(outer, holes)| {
                candidates(outer)
                    .into_iter()
                    .filter(|id| within(*id, outer) && !holes.iter().any(|h| within(*id, h)))
            })
            .collect();
        // Drawing the sides also closes loops the stencil has no face on (openings, a missing
        // lid); take those out. A face stays if it is the stencil's, a piece of an existing
        // face, or if a side of it is not on the stencil.
        // The test below accepts points up to 1e-9 of a segment's length past its ends.
        let reach = segments
            .iter()
            .map(|(a, b)| a.distance(*b))
            .fold(0.0, f64::max);
        let on_stencil = |p: DVec3| {
            let mut on = false;
            segment_index.near_segment(p, p, LENGTH_TOL + 1e-9 * reach, |shape| {
                if let Shape::Segment(k, ..) = shape
                    && !on
                {
                    let (a, b) = segments[k.0 as usize];
                    let (u, d) = project_on_segment(p, a, b);
                    on = (-1e-9..=1.0 + 1e-9).contains(&u) && d < LENGTH_TOL;
                }
            });
            on
        };
        let extra: Vec<FaceId> = new
            .iter()
            .copied()
            .filter(|id| {
                let f = self.face(*id).unwrap();
                let n = f.outer.len();
                !own.contains(id)
                    && !split_off.contains(id)
                    && (0..n).all(|i| {
                        let (a, b) = (self.pos(f.outer[i]), self.pos(f.outer[(i + 1) % n]));
                        on_stencil(a) && on_stencil((a + b) / 2.0)
                    })
            })
            .collect();
        for id in extra {
            self.remove_face(id);
        }
        // Copied curve and soft edges stay so; each placement's curves get new ids.
        for t in places {
            let mut renamed: BTreeMap<CurveId, CurveId> = BTreeMap::new();
            for ((a, b), (curve, soft)) in stencil.segments.iter().zip(&stencil.marks) {
                if curve.is_none() && !soft {
                    continue;
                }
                let curve = curve.map(|c| *renamed.entry(c).or_insert_with(|| self.new_curve()));
                self.tag_along(t.transform_point3(*a), t.transform_point3(*b), curve, *soft);
            }
        }
    }

    /// A face's outer corners and a point inside it (the centre of its first triangle).
    fn face_sample(&self, id: FaceId) -> Option<FaceSample> {
        let face = self.face(id)?;
        let [a, b, c] = self.triangulate(id)?.first().copied()?;
        Some(FaceSample {
            outer: face.outer.iter().map(|v| self.pos(*v)).collect(),
            inside: (a + b + c) / 3.0,
        })
    }

    /// The face whose outer loop is exactly these vertices.
    fn face_with_outer(&self, ids: &BTreeSet<VertexId>) -> Option<FaceId> {
        let first = ids.first()?;
        self.faces_at(*first).find(|id| {
            let f = self.face(*id).unwrap();
            f.outer.len() == ids.len() && f.outer.iter().all(|v| ids.contains(v))
        })
    }
}

/// Edges and faces captured by position, to be drawn again elsewhere (copies, groups).
#[derive(Clone, Debug, Default)]
pub(crate) struct Stencil {
    segments: Vec<(DVec3, DVec3)>,
    /// Each segment's curve and soft flag.
    marks: Vec<(Option<CurveId>, bool)>,
    faces: Vec<StencilFace>,
}

impl Stencil {
    /// The stencil with `material` on every face side that has no material of its own (an
    /// exploded instance's material passing to its faces).
    pub(crate) fn painted_over(mut self, material: Option<MaterialId>) -> Stencil {
        for f in &mut self.faces {
            for side in &mut f.paint {
                *side = side.or(material);
            }
        }
        self
    }
}

/// A face captured by position.
#[derive(Clone, Debug)]
struct StencilFace {
    outer: Vec<DVec3>,
    normal: DVec3,
    /// Front and back materials.
    paint: [Option<MaterialId>; 2],
    /// Holes no face fills.
    openings: Vec<Vec<DVec3>>,
    /// Holes another face fills.
    filled: Vec<Vec<DVec3>>,
}

impl StencilFace {
    fn placed(&self, t: &DAffine3) -> StencilFace {
        let ring = |r: &Vec<DVec3>| r.iter().map(|p| t.transform_point3(*p)).collect();
        StencilFace {
            outer: ring(&self.outer),
            normal: t.transform_vector3(self.normal).normalize(),
            paint: self.paint,
            openings: self.openings.iter().map(ring).collect(),
            filled: self.filled.iter().map(ring).collect(),
        }
    }
}

/// A ring of points prepared for `FaceSample::within`: its plane and its outline in it.
struct RingTest {
    origin: DVec3,
    plane: Plane,
    outline: Vec<DVec2>,
    lo: DVec2,
    hi: DVec2,
    /// The ring's box in model space.
    min: DVec3,
    max: DVec3,
}

/// How far past a ring's box a face sample within the ring may lie (it is within
/// `LENGTH_TOL` of the ring's plane; the rest covers rounding).
const RING_BOX_PAD: f64 = 10.0 * LENGTH_TOL;

impl RingTest {
    /// None for a ring with no area.
    fn new(ring: &[DVec3]) -> Option<RingTest> {
        let n = vector_area(ring);
        if n.length() < LENGTH_TOL * LENGTH_TOL {
            return None;
        }
        let plane = Plane::through(ring[0], n.normalize());
        let outline: Vec<DVec2> = ring.iter().map(|p| plane.project(*p)).collect();
        let (lo, hi) = outline
            .iter()
            .fold((DVec2::MAX, DVec2::MIN), |(lo, hi), p| {
                (lo.min(*p), hi.max(*p))
            });
        let (min, max) = ring.iter().fold((DVec3::MAX, DVec3::MIN), |(lo, hi), p| {
            (lo.min(*p), hi.max(*p))
        });
        Some(RingTest {
            origin: ring[0],
            plane,
            outline,
            lo,
            hi,
            min,
            max,
        })
    }
}

/// The rings of a placed stencil face, prepared.
struct PlacedTest {
    outer: Option<RingTest>,
    openings: Vec<Option<RingTest>>,
    filled: Vec<Option<RingTest>>,
}

/// A face's outer corners and a point inside it.
struct FaceSample {
    outer: Vec<DVec3>,
    inside: DVec3,
}

impl FaceSample {
    /// True when the face lies in the plane of `ring` and inside it.
    fn within(&self, ring: &Option<RingTest>) -> bool {
        let Some(r) = ring else { return false };
        let flat = |p: &DVec3| (*p - r.origin).dot(r.plane.normal).abs() < LENGTH_TOL;
        if !self.outer.iter().all(flat) {
            return false;
        }
        let p = r.plane.project(self.inside);
        p.cmpge(r.lo).all() && p.cmple(r.hi).all() && point_in_polygon(p, &r.outline)
    }
}

fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}

/// True when no two non-adjacent edges of the rings touch or cross (in the plane with this
/// normal) and every hole (ring 1..) lies inside the outer ring (ring 0).
fn rings_are_valid(rings: &[Vec<DVec3>], normal: DVec3) -> bool {
    let plane = Plane::through(rings[0][0], normal);
    let flat: Vec<Vec<glam::DVec2>> = rings
        .iter()
        .map(|r| r.iter().map(|p| plane.project(*p)).collect())
        .collect();
    let mut edges = Vec::new();
    for (r, ring) in flat.iter().enumerate() {
        for i in 0..ring.len() {
            edges.push((r, i, ring[i], ring[(i + 1) % ring.len()]));
        }
    }
    for (x, e) in edges.iter().enumerate() {
        for g in &edges[x + 1..] {
            if e.0 == g.0 {
                let n = flat[e.0].len();
                if (e.1 + 1) % n == g.1 || (g.1 + 1) % n == e.1 {
                    continue; // adjacent edges share a vertex
                }
            }
            let tol = LENGTH_TOL;
            let near = segment_distance_2d(e.2, g.2, g.3) < tol
                || segment_distance_2d(e.3, g.2, g.3) < tol
                || segment_distance_2d(g.2, e.2, e.3) < tol
                || segment_distance_2d(g.3, e.2, e.3) < tol;
            if near || segments_cross(e.2, e.3, g.2, g.3) {
                return false;
            }
        }
    }
    flat[1..].iter().all(|h| point_in_polygon(h[0], &flat[0]))
}

fn segments_cross(p: glam::DVec2, q: glam::DVec2, a: glam::DVec2, b: glam::DVec2) -> bool {
    let side = |o: glam::DVec2, u: glam::DVec2, w: glam::DVec2| (u - o).perp_dot(w - o);
    side(p, q, a) * side(p, q, b) < 0.0 && side(a, b, p) * side(a, b, q) < 0.0
}
