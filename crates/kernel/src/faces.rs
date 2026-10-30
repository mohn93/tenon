//! Automatic face creation: after edges are added, the faces of every affected plane are rebuilt
//! from the bounded regions of that plane's edge graph.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DVec2, DVec3};

use crate::edit::use_indexes;
use crate::geom::{LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon};
use crate::ids::{EdgeId, FaceId, MaterialId, TagId, VertexId};
use crate::index::{Index, Shape};
use crate::model::{Face, Model};
use crate::regions::{PlaneGraph, Region};
use crate::texture::TexturePlacement;

impl Model {
    /// Updates the faces of every plane spanned by one of `new_edges` and an edge touching it.
    pub(crate) fn create_faces(&mut self, new_edges: &[EdgeId]) {
        let mut planes = PlaneSet::default();
        for &e in new_edges {
            for (plane, through) in self.candidate_planes(e) {
                planes.insert(self.plane_of_face_through(plane, through));
            }
        }
        // Rebuilding faces leaves the edges alone, so one edge index serves every plane. Below
        // a few planes, scanning the edges is cheaper than building it.
        let index = (planes.list.len() > FEW_PLANES).then(|| Index::of_edges(self));
        let new_sides = self.sides_of(new_edges);
        for plane in planes.list {
            let edges = match &index {
                Some(index) => {
                    let mut ids = Vec::new();
                    index.segments_near_plane(&plane, |e| ids.push(e));
                    ids.sort();
                    ids.into_iter()
                        .map(|e| self.edge(e).unwrap().v)
                        .filter(|v| {
                            plane.contains(self.pos(v[0])) && plane.contains(self.pos(v[1]))
                        })
                        .collect()
                }
                None => self.edges_in_plane(&plane),
            };
            // Drawing only adds and splits edges, so every face in the plane has its sides
            // among these edges: look for the old faces at their vertices.
            let at: BTreeSet<FaceId> = edges
                .iter()
                .flatten()
                .flat_map(|v| self.faces_at(*v))
                .collect();
            let old = at
                .into_iter()
                .map(|id| (id, self.face(id).unwrap()))
                .filter(|(_, f)| self.face_in_plane(f, &plane))
                .map(|(id, f)| (id, f.clone()))
                .collect();
            self.rebuild_plane_from(plane, &new_sides, edges, old);
        }
    }

    /// `plane`, fitted to the three vertices `through` (edge `e`'s ends and a third), or, when a
    /// face at an end of `e` has the third as a vertex of its loops and its own plane holds all three (to
    /// `LENGTH_TOL`), that face's plane: a plane fitted to a short edge is tilted from the face
    /// it cuts by the error of the points over the edge's length (3e-6 rad for a 4e-5 edge and
    /// a 1e-7 error), and a long face then has its far vertices off the fitted plane and is not
    /// rebuilt with it: the cut would add a face over a part of the face the cut left whole. A
    /// third vertex that is not the face's (an outline leaving it at a shallow angle) does not
    /// make its plane that of the outline. Of several such faces (e along a crease) the one
    /// tilted least from `plane`.
    fn plane_of_face_through(&self, plane: Plane, through: [VertexId; 3]) -> Plane {
        let own = self
            .faces_at(through[0])
            .chain(self.faces_at(through[1]))
            .filter_map(|id| {
                let f = self.face(id)?;
                let has = |v: VertexId| {
                    std::iter::once(&f.outer)
                        .chain(&f.holes)
                        .any(|l| l.contains(&v))
                };
                if !has(through[2]) {
                    return None;
                }
                let own = Plane::through(self.pos(f.outer[0]), f.normal);
                let holds = through.iter().all(|v| own.contains(self.pos(*v)));
                holds.then(|| (own.normal.cross(plane.normal).length(), own))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        own.map_or(plane, |(_, own)| own)
    }

    /// The edges lying in `plane`, in id order.
    fn edges_in_plane(&self, plane: &Plane) -> Vec<[VertexId; 2]> {
        self.edges()
            .map(|(_, e)| e.v)
            .filter(|v| plane.contains(self.pos(v[0])) && plane.contains(self.pos(v[1])))
            .collect()
    }

    /// Planes spanned by edge e and each non-collinear edge reachable from its endpoints,
    /// walking through vertices whose edges continue along e's line. The collinearity test is
    /// relative to e's first vertex, so an edge on the same line can walk elsewhere: each new
    /// edge is walked on its own.
    fn candidate_planes(&self, e: EdgeId) -> Vec<(Plane, [VertexId; 3])> {
        let [a, b] = self.edge(e).unwrap().v;
        let (pa, pb) = (self.pos(a), self.pos(b));
        let mut planes = Vec::new();
        for start in [a, b] {
            let mut visited: BTreeSet<VertexId> = BTreeSet::from([a, b]);
            let mut stack = vec![start];
            while let Some(v) = stack.pop() {
                for f in self.edges_at(v) {
                    if f == e {
                        continue;
                    }
                    let fv = self.edge(f).unwrap().v;
                    let c = if fv[0] == v { fv[1] } else { fv[0] };
                    match Plane::from_points(pa, pb, self.pos(c)) {
                        Some(p) => planes.push((p, [a, b, c])),
                        None => {
                            if visited.insert(c) {
                                stack.push(c);
                            }
                        }
                    }
                }
            }
        }
        planes
    }

    /// Whether `f` lies in `plane`: every vertex of its outer loop on it, which is how a plane
    /// gathers its edges. A face tilted under `PLANE_ANGLE_TOL` with only its first vertex on
    /// the plane is not in it when it is long enough for its far vertices to be off: its far
    /// edges are not gathered, and the rebuild would drop the face. A plane fitted to a few
    /// points of a short edge is tilted more than that from the facet it cuts, and the facet
    /// is in it all the same.
    fn face_in_plane(&self, f: &Face, plane: &Plane) -> bool {
        f.outer.iter().all(|v| plane.contains(self.pos(*v)))
    }

    /// Recomputes the regions of `plane` and makes the face set match them. `plane` is in
    /// canonical orientation; regions are wound counter-clockwise around its normal.
    pub(crate) fn rebuild_plane(&mut self, plane: Plane, new_edges: &[EdgeId]) {
        let edges = self.edges_in_plane(&plane);
        // After erasing or welding, a face in the plane can have lost all its edges: scan.
        let old = self
            .faces()
            .filter(|(_, f)| self.face_in_plane(f, &plane))
            .map(|(id, f)| (id, f.clone()))
            .collect();
        let new_sides = self.sides_of(new_edges);
        self.rebuild_plane_from(plane, &new_sides, edges, old);
    }

    /// The vertex pairs (smaller id first) of these edges.
    fn sides_of(&self, edges: &[EdgeId]) -> BTreeSet<(VertexId, VertexId)> {
        edges
            .iter()
            .filter_map(|e| self.edge(*e))
            .map(|e| side(e.v[0], e.v[1]))
            .collect()
    }

    /// `rebuild_plane` given the edges lying in the plane and the faces in it, both in id
    /// order.
    fn rebuild_plane_from(
        &mut self,
        plane: Plane,
        new_sides: &BTreeSet<(VertexId, VertexId)>,
        edges: Vec<[VertexId; 2]>,
        old: Vec<(FaceId, Face)>,
    ) {
        let pos: BTreeMap<VertexId, DVec2> = edges
            .iter()
            .flatten()
            .map(|v| (*v, plane.project(self.pos(*v))))
            .collect();
        let graph = PlaneGraph { pos, edges };
        let project = |l: &[VertexId]| -> Vec<DVec2> {
            l.iter().map(|v| plane.project(self.pos(*v))).collect()
        };
        let old_polys: Vec<(Vec<DVec2>, Vec<Vec<DVec2>>)> = old
            .iter()
            .map(|(_, f)| {
                (
                    project(&f.outer),
                    f.holes.iter().map(|h| project(h)).collect(),
                )
            })
            .collect();

        let old_sides: BTreeSet<(VertexId, VertexId)> = old
            .iter()
            .flat_map(|(_, f)| std::iter::once(&f.outer).chain(&f.holes))
            .flat_map(|l| (0..l.len()).map(|i| side(l[i], l[(i + 1) % l.len()])))
            .collect();
        // Each wanted region with the normal and materials it gets if it is new: those of the old
        // face it lies in (a piece of a split face keeps its paint), or the plane's own normal
        // and no materials when it lies in none.
        let regions = graph.regions();
        // Many regions among many edges: find the edges near a side and the old faces around
        // a point with indexes (same results as scanning).
        let many = use_indexes(regions.len() * (graph.edges.len() + old.len()) > MANY_PAIRS);
        let edge_index = many.then(|| graph.edge_index());
        let old_boxes = many.then(|| {
            let boxes: Vec<(DVec3, DVec3)> = old_polys
                .iter()
                .map(|(o, hs)| box_of(std::iter::once(o).chain(hs)))
                .collect();
            Index::of_segments(&boxes)
        });
        // The old faces whose box (over all their loops) holds p, in order: a point outside a
        // polygon's box is outside the polygon.
        let old_around = |p: DVec2| -> Vec<usize> {
            match &old_boxes {
                None => (0..old_polys.len()).collect(),
                Some(index) => {
                    let mut found = Vec::new();
                    index.near_segment(p.extend(0.0), p.extend(0.0), 0.0, |shape| {
                        if let Shape::Segment(k, ..) = shape {
                            found.push(k.0 as usize);
                        }
                    });
                    found.sort();
                    found
                }
            }
        };
        let wanted: Vec<(Region, DVec3, Paint)> = regions
            .into_iter()
            .filter_map(|r| {
                let p = graph.interior_point(&r, edge_index.as_ref())?;
                let around = old_around(p);
                let container = around.iter().copied().find(|i| {
                    let (o, hs) = &old_polys[*i];
                    point_in_polygon(p, o) && !hs.iter().any(|h| point_in_polygon(p, h))
                });
                let in_old_face = container.is_some();
                let in_old_hole = around
                    .iter()
                    .any(|i| old_polys[*i].1.iter().any(|h| point_in_polygon(p, h)));
                let uses_new = std::iter::once(&r.outer).chain(&r.holes).any(|l| {
                    (0..l.len()).any(|i| new_sides.contains(&side(l[i], l[(i + 1) % l.len()])))
                });
                // Inside an opening, only a loop drawn apart from every old face's boundary
                // gets a face; a line across the opening leaves its pieces open.
                let apart = (0..r.outer.len()).all(|i| {
                    !old_sides.contains(&side(r.outer[i], r.outer[(i + 1) % r.outer.len()]))
                });
                let keep = in_old_face || (uses_new && (!in_old_hole || apart));
                let normal = container.map_or(plane.normal, |i| old[i].1.normal);
                let paint = container.map_or(([None, None], None, None, false), |i| {
                    (
                        old[i].1.paint(),
                        old[i].1.texture,
                        old[i].1.tag,
                        old[i].1.hidden,
                    )
                });
                keep.then_some((r, normal, paint))
            })
            .collect();

        self.reconcile(&old, wanted, plane);
    }

    /// Keeps old faces that match a wanted region, updates their holes if needed, inserts the
    /// rest, and removes old faces that match nothing. All through recorded primitives.
    fn reconcile(
        &mut self,
        old: &[(FaceId, Face)],
        wanted: Vec<(Region, DVec3, Paint)>,
        plane: Plane,
    ) {
        let mut matched: BTreeSet<FaceId> = BTreeSet::new();
        for (r, normal, ([front, back], texture, tag, hidden)) in wanted {
            let outer: BTreeSet<VertexId> = r.outer.iter().copied().collect();
            // Faces in one plane may face opposite ways (push/pull flips a base face), so an
            // old face matches by vertex set and keeps its own orientation.
            let same = old.iter().find(|(id, f)| {
                !matched.contains(id) && f.outer.iter().copied().collect::<BTreeSet<_>>() == outer
            });
            match same {
                Some((id, f)) => {
                    matched.insert(*id);
                    if loop_sets(&f.holes) != loop_sets(&r.holes) {
                        let mut holes = r.holes;
                        if f.normal.dot(plane.normal) < 0.0 {
                            holes.iter_mut().for_each(|h| h.reverse());
                        }
                        let face = Face {
                            outer: f.outer.clone(),
                            holes,
                            ..f.clone()
                        };
                        self.replace_face(*id, face);
                    }
                }
                None => {
                    let Region {
                        mut outer,
                        mut holes,
                    } = r;
                    if normal.dot(plane.normal) < 0.0 {
                        outer.reverse();
                        holes.iter_mut().for_each(|h| h.reverse());
                    }
                    self.insert_face(Face {
                        outer,
                        holes,
                        normal,
                        front,
                        back,
                        texture,
                        tag,
                        hidden,
                    });
                }
            }
        }
        for (id, _) in old {
            if !matched.contains(id) {
                self.remove_face(*id);
            }
        }
    }
}

/// A face's front and back materials, its texture placement, its tag and whether it is
/// hidden, which the pieces split from it keep.
type Paint = (
    [Option<MaterialId>; 2],
    Option<TexturePlacement>,
    Option<TagId>,
    bool,
);

/// Above this many planes, `create_faces` finds each plane's edges with an index.
const FEW_PLANES: usize = 4;

/// Above this many (region, edge or old face) pairs, `rebuild_plane_from` looks things up with
/// indexes instead of scanning.
const MANY_PAIRS: usize = 4096;

/// The box around 2D loops, as the diagonal of a segment at z = 0 (for `Index::of_segments`).
fn box_of<'a>(loops: impl Iterator<Item = &'a Vec<DVec2>>) -> (DVec3, DVec3) {
    let (lo, hi) = loops
        .flatten()
        .fold((DVec2::MAX, DVec2::MIN), |(lo, hi), p| {
            (lo.min(*p), hi.max(*p))
        });
    (lo.extend(0.0), hi.extend(0.0))
}

/// Distinct planes in the order first inserted, where `Plane::same_as` decides what is
/// distinct. A grid over normal and offset keeps lookups near constant time.
#[derive(Default)]
struct PlaneSet {
    list: Vec<Plane>,
    cells: HashMap<[i64; 4], Vec<usize>>,
}

/// Grid cell size for normal components and offsets.
const PLANE_CELL: f64 = 1e-3;

impl PlaneSet {
    fn key(n: DVec3, d: f64) -> [f64; 4] {
        [n.x, n.y, n.z, d]
    }

    /// Adds `plane` unless the same plane is in already; true when added.
    fn insert(&mut self, plane: Plane) -> bool {
        // `same_as` accepts normals less than PLANE_ANGLE_TOL apart (either way round) and
        // offsets less than LENGTH_TOL apart: look in every cell within twice that.
        let reach = [
            2.0 * PLANE_ANGLE_TOL,
            2.0 * PLANE_ANGLE_TOL,
            2.0 * PLANE_ANGLE_TOL,
            2.0 * LENGTH_TOL,
        ];
        let cell = |x: f64| (x / PLANE_CELL).floor() as i64;
        for key in [
            Self::key(plane.normal, plane.d),
            Self::key(-plane.normal, -plane.d),
        ] {
            let lo: Vec<i64> = (0..4).map(|k| cell(key[k] - reach[k])).collect();
            let hi: Vec<i64> = (0..4).map(|k| cell(key[k] + reach[k])).collect();
            for a in lo[0]..=hi[0] {
                for b in lo[1]..=hi[1] {
                    for c in lo[2]..=hi[2] {
                        for d in lo[3]..=hi[3] {
                            let found = self.cells.get(&[a, b, c, d]).into_iter().flatten();
                            if found.into_iter().any(|&i| self.list[i].same_as(&plane)) {
                                return false;
                            }
                        }
                    }
                }
            }
        }
        let k = Self::key(plane.normal, plane.d).map(cell);
        self.cells.entry(k).or_default().push(self.list.len());
        self.list.push(plane);
        true
    }
}

fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}

fn loop_sets(loops: &[Vec<VertexId>]) -> BTreeSet<BTreeSet<VertexId>> {
    loops.iter().map(|l| l.iter().copied().collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(m: &mut Model, x0: f64, y0: f64, s: f64) {
        let c = [(x0, y0), (x0 + s, y0), (x0 + s, y0 + s), (x0, y0 + s)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
                .unwrap();
            m.validate().unwrap();
        }
    }

    /// A 4x4 face with a 2x2 opening whose inner face was removed. Returns the 4x4 face.
    fn face_with_opening() -> (Model, FaceId, Face) {
        let mut m = Model::new();
        square(&mut m, 0.0, 0.0, 4.0);
        square(&mut m, 1.0, 1.0, 2.0);
        let inner = m.faces().find(|(_, f)| f.holes.is_empty()).unwrap().0;
        m.remove_face(inner);
        m.take_changes();
        let (id, f) = m.faces().next().unwrap();
        let f = f.clone();
        assert_eq!(m.faces().count(), 1);
        assert_eq!(m.face_area(id), 12.0);
        (m, id, f)
    }

    /// A strip of four quads `size` wide, each tilted about the edge it shares with the last by
    /// the next of `tilts` (radians); returns the model and the height of the strip at x.
    fn tilted_strip(size: f64, tilts: [f64; 4]) -> (Model, impl Fn(f64) -> f64) {
        let mut slopes = Vec::new();
        let mut slope = 0.0;
        for t in tilts {
            slope += t;
            slopes.push(slope);
        }
        let mut heights = vec![0.0];
        for s in &slopes {
            heights.push(heights.last().unwrap() + size * s);
        }
        let at = move |x: f64| {
            let i = ((x / size) as usize).min(3);
            heights[i] + slopes[i] * (x - i as f64 * size)
        };
        let mut m = Model::new();
        for i in 0..4 {
            let (x0, x1) = (i as f64 * size, (i + 1) as f64 * size);
            let c = [(x0, 0.0), (x1, 0.0), (x1, size), (x0, size)];
            for k in 0..4 {
                let (a, b) = (c[k], c[(k + 1) % 4]);
                m.add_edge(DVec3::new(a.0, a.1, at(a.0)), DVec3::new(b.0, b.1, at(b.0)))
                    .unwrap();
            }
        }
        (m, at)
    }

    #[test]
    fn a_line_across_near_coplanar_faces_splits_each_once() {
        // Quads 1 mm wide, tilted 2e-6 to 5e-4 rad from the last: the plane fitted to a short
        // cut edge is tilted from the face it cuts by more than `PLANE_ANGLE_TOL`, but every
        // vertex is within `LENGTH_TOL` of it.
        let size = 1e-3;
        let (mut m, at) = tilted_strip(size, [0.0, 2e-6, 1e-4, 5e-4]);
        assert_eq!(m.faces().count(), 4);
        let area = |m: &Model| m.faces().map(|(id, _)| m.face_area(id)).sum::<f64>();
        let before = area(&m);
        let (y, x1) = (size / 2.0, 4.0 * size);
        m.add_edge(DVec3::new(0.0, y, at(0.0)), DVec3::new(x1, y, at(x1)))
            .unwrap();
        assert_eq!(m.faces().count(), 8);
        assert!((area(&m) - before).abs() < 1e-12 * before.max(1.0));
        m.validate().unwrap();
    }

    #[test]
    fn a_neighbour_tilted_beyond_the_tolerance_is_left_alone() {
        // The second quad is tilted 2e-3 rad and 0.5 m long, so its far vertices are well off
        // the first quad's plane: drawing on the first leaves it as it was.
        let mut m = Model::new();
        for (a, b) in [
            ((0.0, 0.0, 0.0), (1.0, 0.0, 0.0)),
            ((1.0, 0.0, 0.0), (1.0, 1.0, 0.0)),
            ((1.0, 1.0, 0.0), (0.0, 1.0, 0.0)),
            ((0.0, 1.0, 0.0), (0.0, 0.0, 0.0)),
            ((1.0, 0.0, 0.0), (1.5, 0.0, 1e-3)),
            ((1.5, 0.0, 1e-3), (1.5, 1.0, 1e-3)),
            ((1.5, 1.0, 1e-3), (1.0, 1.0, 0.0)),
        ] {
            m.add_edge(DVec3::from(a), DVec3::from(b)).unwrap();
        }
        assert_eq!(m.faces().count(), 2);
        let other = m
            .faces()
            .find(|(_, f)| f.normal.z.abs() < 1.0 - 1e-9)
            .map(|(id, f)| (id, f.clone()))
            .unwrap();
        m.add_edge(DVec3::new(0.25, 0.5, 0.0), DVec3::new(0.75, 0.5, 0.0))
            .unwrap();
        assert_eq!(m.faces().count(), 2);
        assert_eq!(m.face(other.0), Some(&other.1));
    }

    #[test]
    fn a_long_neighbour_tilted_under_the_angle_tolerance_is_left_alone() {
        // The second quad is 10 m long and tilted 5e-7 rad, under `PLANE_ANGLE_TOL`, so its far
        // vertices are 5e-6 off the first quad's plane. Whichever vertex its loop starts at,
        // drawing on the first quad must leave it as it was.
        let mut counts = Vec::new();
        for flip in [false, true] {
            let mut m = Model::new();
            let mut edges = vec![
                ((0.0, 0.0, 0.0), (1.0, 0.0, 0.0)),
                ((1.0, 0.0, 0.0), (1.0, 1.0, 0.0)),
                ((1.0, 1.0, 0.0), (0.0, 1.0, 0.0)),
                ((0.0, 1.0, 0.0), (0.0, 0.0, 0.0)),
                ((1.0, 0.0, 0.0), (11.0, 0.0, 5e-6)),
                ((11.0, 0.0, 5e-6), (11.0, 1.0, 5e-6)),
                ((11.0, 1.0, 5e-6), (1.0, 1.0, 0.0)),
            ];
            if flip {
                edges.reverse();
            }
            for (a, b) in edges {
                m.add_edge(DVec3::from(a), DVec3::from(b)).unwrap();
            }
            assert_eq!(m.faces().count(), 2);
            let other = m
                .faces()
                .find(|(_, f)| f.outer.iter().any(|v| m.pos(*v).x > 5.0))
                .map(|(id, f)| (id, f.clone()))
                .unwrap();
            m.add_edge(DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0)).unwrap();
            counts.push((m.faces().count(), m.face(other.0) == Some(&other.1)));
        }
        assert_eq!(counts, [(3, true), (3, true)]);
    }

    #[test]
    fn flipped_face_keeps_its_id_and_orientation_in_a_plane_with_both_orientations() {
        let mut m = Model::new();
        // The plane holds faces of both orientations.
        square(&mut m, 10.0, 0.0, 2.0);
        square(&mut m, 0.0, 0.0, 4.0);
        let (id, f) = m
            .faces()
            .find(|(id, _)| m.face_area(*id) == 16.0)
            .map(|(id, f)| (id, f.clone()))
            .unwrap();
        assert_eq!(f.normal, DVec3::Z);
        m.replace_face(id, f.reversed());
        m.take_changes();
        square(&mut m, 1.0, 1.0, 2.0);
        m.validate().unwrap();
        let f = m.face(id).unwrap();
        assert_eq!(f.normal, -DVec3::Z);
        assert_eq!(f.holes.len(), 1);
        assert_eq!(m.face_area(id), 12.0);
        assert_eq!(m.faces().count(), 3);
    }

    #[test]
    fn loop_drawn_inside_an_opening_is_filled() {
        let (mut m, outer, before) = face_with_opening();
        square(&mut m, 1.5, 1.5, 0.5);
        assert_eq!(m.faces().count(), 2);
        assert_eq!(m.face(outer), Some(&before));
        let small = m.faces().find(|(id, _)| *id != outer).unwrap().0;
        assert!((m.face_area(small) - 0.25).abs() < 1e-12);
    }

    /// The faces of a model by position: each outer loop's corners from the lowest, rounded
    /// to micrometers, with the hole count and the rounded normal; sorted.
    fn shapes(m: &Model) -> Vec<(Vec<[i64; 3]>, usize, [i64; 3])> {
        let r = |p: DVec3| (p * 1e6).round().to_array().map(|c| c as i64);
        let mut all: Vec<_> = m
            .faces()
            .map(|(_, f)| {
                let mut ring: Vec<[i64; 3]> = f.outer.iter().map(|v| r(m.pos(*v))).collect();
                let low = (0..ring.len()).min_by_key(|&i| ring[i]).unwrap();
                ring.rotate_left(low);
                (ring, f.holes.len(), r(f.normal))
            })
            .collect();
        all.sort();
        all
    }

    fn segments(points: &[(f64, f64)], closed: bool) -> Vec<(DVec3, DVec3)> {
        let p = |(x, y): (f64, f64)| DVec3::new(x, y, 0.0);
        let n = points.len();
        let sides = if closed { n } else { n - 1 };
        (0..sides)
            .map(|i| (p(points[i]), p(points[(i + 1) % n])))
            .collect()
    }

    #[test]
    fn drawing_segments_in_one_batch_makes_the_faces_drawing_them_one_by_one_makes() {
        let circle: Vec<(f64, f64)> = (0..24)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 24.0;
                (2.0 + a.cos(), 2.0 + a.sin())
            })
            .collect();
        type Case = (&'static str, Model, Vec<(DVec3, DVec3)>);
        let cases: Vec<Case> = vec![
            (
                "crossing lines",
                Model::new(),
                [
                    segments(&[(0.0, 1.0), (3.0, 1.0)], false),
                    segments(&[(0.0, 2.0), (3.0, 2.0)], false),
                    segments(&[(1.0, 0.0), (1.0, 3.0)], false),
                    segments(&[(2.0, 0.0), (2.0, 3.0)], false),
                    segments(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0)], true),
                ]
                .concat(),
            ),
            (
                "overlapping squares",
                Model::new(),
                [
                    segments(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)], true),
                    segments(&[(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)], true),
                ]
                .concat(),
            ),
            (
                "square across a face",
                {
                    let mut m = Model::new();
                    square(&mut m, 0.0, 0.0, 4.0);
                    m
                },
                segments(&[(3.0, 3.0), (5.0, 3.0), (5.0, 5.0), (3.0, 5.0)], true),
            ),
            (
                "circle inside a face",
                {
                    let mut m = Model::new();
                    square(&mut m, 0.0, 0.0, 4.0);
                    m
                },
                segments(&circle, true),
            ),
            (
                "square inside an opening",
                face_with_opening().0,
                segments(&[(1.5, 1.5), (2.5, 1.5), (2.5, 2.5), (1.5, 2.5)], true),
            ),
        ];
        for (what, start, segs) in cases {
            let mut batch = start.clone();
            batch.draw_segments(&segs);
            batch.take_changes();
            batch.validate().unwrap();
            let mut one_by_one = start;
            for (a, b) in &segs {
                one_by_one.draw_segment(*a, *b);
            }
            one_by_one.take_changes();
            assert_eq!(shapes(&batch), shapes(&one_by_one), "{what}");
            assert!(!shapes(&batch).is_empty(), "{what}");
        }
    }

    /// A 10x10 face with a 2x2 window (4..6) whose inner face was removed.
    fn wall_with_window() -> Model {
        let mut m = Model::new();
        square(&mut m, 0.0, 0.0, 10.0);
        square(&mut m, 4.0, 4.0, 2.0);
        let inner = m.faces().find(|(_, f)| f.holes.is_empty()).unwrap().0;
        m.remove_face(inner);
        m.take_changes();
        m
    }

    fn window_rectangle() -> Vec<(DVec3, DVec3)> {
        segments(&[(5.0, 4.5), (12.0, 4.5), (12.0, 5.5), (5.0, 5.5)], true)
    }

    fn areas(m: &Model) -> Vec<f64> {
        let mut a: Vec<f64> = m.faces().map(|(id, _)| m.face_area(id)).collect();
        a.sort_by(f64::total_cmp);
        a
    }

    #[test]
    fn a_rectangle_drawn_as_a_batch_out_of_a_window_leaves_the_window_open() {
        let mut batch = wall_with_window();
        batch.draw_segments(&window_rectangle());
        batch.take_changes();
        batch.validate().unwrap();
        assert_eq!(areas(&batch), [2.0, 4.0, 92.0]);
    }

    /// A chain of points (k, k^2 eps, 0) that bends far less than the tolerance, with a gap
    /// at 0..2 whose two edges close a loop with posts at both ends and a top edge.
    #[test]
    fn a_barely_bent_chain_gives_the_same_faces_in_a_batch_as_one_by_one() {
        let eps = 1e-10;
        let p = |k: i32| DVec3::new(k as f64, (k * k) as f64 * eps, 0.0);
        let mut start = Model::new();
        for k in -10..12 {
            if k == 0 || k == 1 {
                continue;
            }
            start.draw_segment(p(k), p(k + 1));
        }
        let top = |k: i32| p(k) + DVec3::new(0.0, 0.0, 1.0);
        start.draw_segment(p(-10), top(-10));
        start.draw_segment(p(12), top(12));
        start.draw_segment(top(-10), top(12));
        start.take_changes();
        let gap = [(p(0), p(1)), (p(1), p(2))];
        let mut batch = start.clone();
        batch.draw_segments(&gap);
        let mut one_by_one = start;
        for (a, b) in gap {
            one_by_one.draw_segment(a, b);
        }
        assert_eq!(one_by_one.faces().count(), 1);
        assert_eq!(batch.faces().count(), 1);
    }

    /// A chain on the x axis whose vertices sit `bends` off the line (one y per vertex, from
    /// x = xs[0]), drawn except for the edges in `gap`, plus posts at both ends and a top edge.
    /// Returns the face counts after drawing the gap as a batch and one segment at a time.
    fn chain_faces(xs: &[f64], bends: &[f64], gap: &[(usize, usize)]) -> (usize, usize) {
        let p = |k: usize| DVec3::new(xs[k], bends[k], 0.0);
        let last = xs.len() - 1;
        let mut start = Model::new();
        for k in 0..last {
            if !gap
                .iter()
                .any(|g| g.0.min(g.1) == k && g.0.max(g.1) == k + 1)
            {
                start.draw_segment(p(k), p(k + 1));
            }
        }
        let up = DVec3::new(0.0, 0.0, 1.0);
        start.draw_segment(p(0), p(0) + up);
        start.draw_segment(p(last), p(last) + up);
        start.draw_segment(p(0) + up, p(last) + up);
        start.take_changes();
        let segments: Vec<_> = gap.iter().map(|&(a, b)| (p(a), p(b))).collect();
        let mut batch = start.clone();
        batch.draw_segments(&segments);
        let mut one_by_one = start;
        for (a, b) in segments {
            one_by_one.draw_segment(a, b);
        }
        (batch.faces().count(), one_by_one.faces().count())
    }

    /// The reviewer's probe: the gap's second edge lies on e's line, but its walk starts nearer
    /// the bent vertices than e's does and passes them.
    #[test]
    fn a_gap_edge_whose_walk_passes_vertices_the_first_one_stops_at_gives_the_same_faces() {
        // P, L, A, B, C, R, Q: A-B and C-B are the gap, drawn (A, B) then (C, B).
        let xs = [-10.0, -0.01, 0.0, 1.0, 2.0, 2.01, 12.0];
        for dl in [5e-11, 1e-10, 5e-10, 2e-9] {
            for dr in [3e-9, 1e-8, 1e-7] {
                let bends = [0.0, dl, 0.0, 0.0, 0.0, dr, 0.0];
                let (batch, one_by_one) = chain_faces(&xs, &bends, &[(2, 3), (4, 3)]);
                assert_eq!(batch, one_by_one, "dl {dl} dr {dr}");
                assert_eq!(batch, 1, "dl {dl} dr {dr}");
            }
        }
    }

    /// Seeded near-straight chains with a gap of two edges drawn in either order and direction:
    /// the batch never finds fewer faces than one segment at a time, and as many when the
    /// two gap edges are drawn in chain order.
    #[test]
    fn near_straight_chains_give_the_same_faces_in_a_batch_as_one_by_one() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for trial in 0..3000 {
            let n = 6 + (next() * 11.0) as usize;
            let mut x = -10.0;
            let mut xs = vec![x];
            for _ in 0..n {
                x += if next() < 0.5 {
                    0.005 + next() * 0.05
                } else {
                    0.5 + next() * 3.0
                };
                xs.push(x);
            }
            let bends: Vec<f64> = (0..xs.len())
                .map(|_| {
                    if next() < 0.2 {
                        0.0
                    } else {
                        10f64.powf(-11.0 + 4.0 * next()) * if next() < 0.5 { -1.0 } else { 1.0 }
                    }
                })
                .collect();
            let g = 1 + (next() * (n - 3) as f64) as usize;
            let (a, b) = if next() < 0.5 { (g, g + 1) } else { (g + 1, g) };
            let (c, d) = if next() < 0.5 {
                (g + 1, g + 2)
            } else {
                (g + 2, g + 1)
            };
            let (batch, one_by_one) = chain_faces(&xs, &bends, &[(a, b), (c, d)]);
            assert!(
                batch >= one_by_one,
                "trial {trial}: batch {batch} one by one {one_by_one}, xs {xs:?} bends {bends:?}"
            );
        }
    }

    #[test]
    fn plane_set_keeps_one_of_each_plane_in_order() {
        let mut set = PlaneSet::default();
        let ground = Plane::through(DVec3::ZERO, DVec3::Z);
        assert!(set.insert(ground));
        // The same plane within tolerance, either way round, and across a grid cell border.
        assert!(!set.insert(Plane::through(DVec3::new(0.0, 0.0, 5e-7), DVec3::Z)));
        assert!(!set.insert(Plane::through(DVec3::ZERO, -DVec3::Z)));
        let tilt = DVec3::new(PLANE_CELL * 0.5e-3, 0.0, 1.0).normalize();
        assert!(!set.insert(Plane::through(DVec3::ZERO, tilt)));
        let wall = Plane::through(DVec3::new(PLANE_CELL, 0.0, 0.0), DVec3::X);
        assert!(set.insert(wall));
        let near = Plane::through(DVec3::new(PLANE_CELL + 5e-7, 0.0, 0.0), DVec3::X);
        assert!(!set.insert(near));
        // Apart by more than the tolerance: another plane.
        assert!(set.insert(Plane::through(DVec3::new(0.0, 0.0, 2e-6), DVec3::Z)));
        assert!(set.insert(Plane::through(
            DVec3::ZERO,
            DVec3::new(0.0, 1e-5, 1.0).normalize()
        )));
        assert_eq!(set.list.len(), 4);
        assert!(set.list[0].same_as(&ground) && set.list[1].same_as(&wall));
    }

    #[test]
    fn line_across_an_opening_leaves_it_open() {
        let (mut m, outer, _) = face_with_opening();
        m.add_edge(DVec3::new(1.0, 1.5, 0.0), DVec3::new(3.0, 1.5, 0.0))
            .unwrap();
        m.validate().unwrap();
        assert_eq!(m.faces().count(), 1);
        assert_eq!(m.face_area(outer), 12.0);
    }
    #[test]
    fn an_outline_leaving_a_face_at_a_shallow_angle_closes_a_face_of_its_own() {
        // The outline lies in the plane z = -theta * y through the corner of the square, and
        // its point c1 is within `LENGTH_TOL` of the square's plane: the square's plane is not
        // the outline's, as c1 is not the square's vertex.
        for (n, theta) in [(1e-3, 5e-4), (1e-4, 5e-3), (1e-2, 5e-5)] {
            let mut m = Model::new();
            square(&mut m, 0.0, 0.0, 1.0);
            let p = |x: f64, y: f64| DVec3::new(x, y, -theta * y);
            let (a, b) = (p(1.0, 0.0), p(1.001, 0.0));
            let outline = [b, p(1.001, -n), p(1.001, -1.0), p(1.0, -1.0), p(1.0, -n), a];
            for w in outline.windows(2) {
                m.add_edge(w[0], w[1]).unwrap();
            }
            m.add_edge(a, b).unwrap();
            m.validate().unwrap();
            assert_eq!(m.faces().count(), 2, "n {n} theta {theta}");
        }
    }
}
