use std::collections::BTreeSet;
use std::fmt;

use glam::DVec3;

use crate::geom::{LENGTH_TOL, project_on_segment, segment_crossing};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::index::{Index, Shape};
use crate::model::{ChangeSet, Model};

#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    NotFinite,
    ZeroLength,
    FaceNotFound,
    ZeroDistance,
    /// A push/pull would shrink a connected edge to nothing or turn it around.
    Collapse,
    /// A push/pull would carry the face through other faces.
    Obstructed,
    /// A move would bend this face out of its plane.
    Bend(FaceId),
    /// A move would shrink an edge to nothing, turn a face inside out, or land a vertex on
    /// another.
    Squash,
    /// An instance id names no instance in the mesh.
    InstanceNotFound,
    /// A definition id names no definition in the scene.
    DefinitionNotFound,
    /// Axes that are not a frame; the text says why.
    BadAxes(&'static str),
    /// A location that is out of range; the text says how.
    BadLocation(&'static str),
    /// A scene transition that is out of range; the text says how.
    BadTransition(String),
    /// A material id names no material in the scene.
    MaterialNotFound,
    /// A material name is empty, too long or taken; the text says which.
    MaterialName(String),
    /// A material opacity is not a finite number from 0 to 1; the text says which.
    MaterialOpacity(String),
    /// A texture's size is out of range, its image is broken, or a model would hold too many;
    /// the text says which.
    MaterialTexture(String),
    /// A material emission is not a finite number from 0 to `MAX_EMISSION`; the text says
    /// which.
    MaterialEmission(String),
    /// A material metalness or roughness is not a finite number from 0 to 1; the text says
    /// which.
    MaterialFinish(String),
    /// A material density is not a finite number from 1 to 30000 kg/m³; the text says which.
    MaterialDensity(String),
    /// A texture placement's offset, rotation or scale is out of range; the text says which.
    TexturePlacement(String),
    /// A tag id names no tag in the scene.
    TagNotFound,
    /// A tag name is blank, too long, "Untagged" or taken, or a deleted tag's entities would
    /// move to itself; the text says which.
    TagName(String),
    /// A page id names no page (saved scene) in the scene.
    PageNotFound,
    /// A page name is blank, too long or taken, its view is invalid, or a move index is out
    /// of range; the text says which.
    Page(String),
    /// A joint id names no joint in the scene.
    JointNotFound,
    /// A joint is invalid: a blank, long or taken name, parts that are not distinct instances
    /// of the root, a frame that is not a unit axis with a perpendicular reference, or limits
    /// that are out of order or leave the value out; the text says which.
    Joint(String),
    /// A coupling id names no coupling in the scene.
    CouplingNotFound,
    /// A cam follower id names no cam follower in the scene.
    CamFollowerNotFound,
    /// A style id names no style in the scene.
    StyleNotFound,
    /// A style name is blank, too long or taken, or its settings are out of range; the text
    /// says which.
    Style(String),
    /// An import makes nothing: nothing in the file makes a face, edge, guide or instance.
    NothingToImport,
    /// An import would make more faces than `MAX_IMPORT_FACES`.
    TooManyFaces,
    /// An import tree is malformed (a missing or cyclic definition, nesting too deep, too
    /// many definitions or instances, or a singular transform), or the mesh it should go
    /// straight into is not empty; the text says which.
    InvalidImport(String),
    /// A dimension or note is invalid: a missing vertex or note, a curve that is not a
    /// circle or an arc, text that is blank or too long, a screen note off the view or
    /// outside the root; the text says which.
    Annotation(String),
    /// A section plane is invalid or unknown: a normal that is not a unit vector, numbers that
    /// are not finite, or an id naming no plane of the mesh; the text says which.
    Section(String),
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::NotFinite => write!(f, "coordinates must be finite numbers"),
            KernelError::ZeroLength => write!(f, "edge endpoints coincide"),
            KernelError::FaceNotFound => write!(f, "no such face"),
            KernelError::ZeroDistance => write!(f, "distance is too small"),
            KernelError::Collapse => {
                write!(
                    f,
                    "distance would collapse or invert the edges joining the face"
                )
            }
            KernelError::Obstructed => {
                write!(f, "distance would push the face through other geometry")
            }
            KernelError::Bend(face) => {
                write!(f, "the move would bend face f{} out of its plane", face.0)
            }
            KernelError::Squash => write!(
                f,
                "the move would collapse an edge, turn a face over, or land a vertex on another"
            ),
            KernelError::InstanceNotFound => write!(f, "no such instance"),
            KernelError::DefinitionNotFound => write!(f, "no such definition"),
            KernelError::BadAxes(why) => write!(f, "{why}"),
            KernelError::BadLocation(why) => write!(f, "{why}"),
            KernelError::BadTransition(why) => write!(f, "{why}"),
            KernelError::MaterialNotFound => write!(f, "no such material"),
            KernelError::TagNotFound => write!(f, "no such tag"),
            KernelError::PageNotFound => write!(f, "no such scene"),
            KernelError::StyleNotFound => write!(f, "no such style"),
            KernelError::JointNotFound => write!(f, "no such joint"),
            KernelError::CouplingNotFound => write!(f, "no such coupling"),
            KernelError::CamFollowerNotFound => write!(f, "no such cam follower"),
            KernelError::MaterialName(why)
            | KernelError::MaterialOpacity(why)
            | KernelError::MaterialTexture(why)
            | KernelError::MaterialEmission(why)
            | KernelError::MaterialFinish(why)
            | KernelError::MaterialDensity(why)
            | KernelError::TexturePlacement(why)
            | KernelError::TagName(why)
            | KernelError::Page(why)
            | KernelError::Style(why)
            | KernelError::Joint(why)
            | KernelError::InvalidImport(why)
            | KernelError::Annotation(why)
            | KernelError::Section(why) => {
                write!(f, "{why}")
            }
            KernelError::NothingToImport => {
                write!(
                    f,
                    "nothing to import: nothing in the file makes a face, edge, guide or instance"
                )
            }
            KernelError::TooManyFaces => write!(
                f,
                "the polygons make more than {} faces",
                crate::import::MAX_IMPORT_FACES
            ),
        }
    }
}

impl std::error::Error for KernelError {}

impl Model {
    /// Adds the segment a-b with sticky-geometry rules: endpoints merge with nearby vertices,
    /// crossed edges are split, collinear overlaps merge, and closed coplanar loops become faces.
    /// Returns the changes as one ChangeSet. It is empty only when nothing changed; it can be
    /// non-empty without a new edge when an existing edge was only split.
    pub fn add_edge(&mut self, a: DVec3, b: DVec3) -> Result<ChangeSet, KernelError> {
        check_segment(a, b)?;
        self.draw_segment(a, b);
        Ok(self.take_changes())
    }

    /// Adds the closed polygon through `points` (each point joined to the next, the last to the
    /// first) as one ChangeSet, with the same rules as `add_edge`. The sides are drawn as one
    /// batch, so faces are updated once rather than after every side.
    pub fn add_polygon(&mut self, points: &[DVec3]) -> Result<ChangeSet, KernelError> {
        let n = points.len();
        let sides: Vec<(DVec3, DVec3)> = (0..n).map(|i| (points[i], points[(i + 1) % n])).collect();
        for (a, b) in &sides {
            check_segment(*a, *b)?;
        }
        self.draw_segments(&sides);
        Ok(self.take_changes())
    }

    /// Adds every segment as one batch with the rules of `add_edge` (see `draw_segments`), as
    /// one ChangeSet. Segments shorter than `LENGTH_TOL` are skipped, since the result of
    /// intersecting faces may hold slivers; a segment that is not finite is an error and nothing
    /// is drawn. Segments that already exist change nothing.
    pub fn add_segments(&mut self, segments: &[(DVec3, DVec3)]) -> Result<ChangeSet, KernelError> {
        if segments
            .iter()
            .any(|(a, b)| !a.is_finite() || !b.is_finite())
        {
            return Err(KernelError::NotFinite);
        }
        let kept: Vec<(DVec3, DVec3)> = segments
            .iter()
            .copied()
            .filter(|(a, b)| check_segment(*a, *b).is_ok())
            .collect();
        self.draw_segments(&kept);
        Ok(self.take_changes())
    }

    /// `add_edge` without input checks or ending the edit; the caller checked the segment.
    pub(crate) fn draw_segment(&mut self, a: DVec3, b: DVec3) {
        self.draw_segments(&[(a, b)]);
    }

    /// Inserts every segment (with the rules of `add_edge`), then updates the faces of each
    /// plane the new edges reach, once. Faces come out as if the segments were drawn one by
    /// one, except that a region closed only by these segments is never treated as an
    /// existing face by a later one (it matters only inside an opening; see `rebuild_plane`).
    /// A batch that cuts a face open to an opening leaves the opening open; drawn one by one,
    /// a later line can fill it once an earlier one has cut the face open to it (a known
    /// limitation: the model keeps no record of openings).
    pub(crate) fn draw_segments(&mut self, segments: &[(DVec3, DVec3)]) {
        let mut fresh: BTreeSet<EdgeId> = BTreeSet::new();
        let mut near = Nearby::for_batch(segments.len());
        for (a, b) in segments {
            self.insert_segment(*a, *b, &mut fresh, &mut near);
        }
        let fresh: Vec<EdgeId> = fresh.into_iter().collect();
        self.create_faces(&fresh);
    }

    pub(crate) fn find_vertex(&self, p: DVec3) -> Option<VertexId> {
        self.vertices_near(p, LENGTH_TOL).first().copied()
    }

    /// Existing vertex at p, or a new one (splitting an edge if p lies inside it).
    fn vertex_at(&mut self, p: DVec3, fresh: &mut BTreeSet<EdgeId>, near: &mut Nearby) -> VertexId {
        if let Some(v) = self.find_vertex(p) {
            return v;
        }
        let hit = near.edges(self, p, p).into_iter().find(|id| {
            let e = self.edge(*id).unwrap();
            let (t, dist) = project_on_segment(p, self.pos(e.v[0]), self.pos(e.v[1]));
            dist < LENGTH_TOL && t > 0.0 && t < 1.0
        });
        match hit {
            Some(e) => self.split_edge(e, p, fresh),
            None => self.insert_vertex(p),
        }
    }

    /// Splits edge e at p, inserting the new vertex into every face loop that used the edge.
    /// When e is in `fresh` (drawn in this batch), its pieces replace it there.
    fn split_edge(&mut self, e: EdgeId, p: DVec3, fresh: &mut BTreeSet<EdgeId>) -> VertexId {
        let old = self.remove_edge(e);
        let [a, b] = old.v;
        let m = self.insert_vertex(p);
        // Both pieces stay on the edge's curve and tag and keep its soft, polygon and hidden flags.
        let first = self.insert_edge_like(a, m, &old);
        let second = self.insert_edge_like(m, b, &old);
        if fresh.remove(&e) {
            fresh.extend([first, second]);
        }
        for id in self.faces_on_side(a, b) {
            let mut f = self.face(id).unwrap().clone();
            for l in std::iter::once(&mut f.outer).chain(f.holes.iter_mut()) {
                insert_between(l, a, b, m);
            }
            self.replace_face(id, f);
        }
        m
    }

    /// Inserts edges covering a-b, splitting at every vertex and edge crossing along the way.
    /// Adds the edges that did not exist before to `fresh`.
    fn insert_segment(
        &mut self,
        a: DVec3,
        b: DVec3,
        fresh: &mut BTreeSet<EdgeId>,
        near: &mut Nearby,
    ) {
        near.refresh(self);
        let va = self.vertex_at(a, fresh, near);
        let vb = self.vertex_at(b, fresh, near);
        let (pa, pb) = (self.pos(va), self.pos(vb));

        let mut others = near.edges(self, pa, pb);
        others.retain(|e| {
            let v = self.edge(*e).unwrap().v;
            !v.contains(&va) && !v.contains(&vb)
        });
        for e in others {
            let [p, q] = self.edge(e).unwrap().v;
            if let Some(x) = segment_crossing(self.pos(p), self.pos(q), pa, pb) {
                self.split_edge(e, x, fresh);
            }
        }

        let mut along: Vec<(f64, VertexId)> = near
            .vertices(self, pa, pb)
            .into_iter()
            .filter_map(|id| {
                let (t, dist) = project_on_segment(self.pos(id), pa, pb);
                (dist < LENGTH_TOL && (-1e-12..=1.0 + 1e-12).contains(&t)).then_some((t, id))
            })
            .collect();
        along.sort_by(|x, y| x.0.total_cmp(&y.0));

        for w in along.windows(2) {
            let (u, v) = (w[0].1, w[1].1);
            if self.edge_between(u, v).is_none() {
                fresh.insert(self.insert_edge(u, v));
            }
        }
    }
}

/// Batches of at least this many segments find nearby edges and vertices with an index;
/// smaller ones scan the model, which is cheaper than building the index.
const INDEXED_BATCH: usize = 8;

/// How many ids the batch may hand out past the newest index level before the next level is
/// built (few in tests, so they build and merge levels often).
const TAIL: u64 = if cfg!(test) { 4 } else { 256 };

/// How `insert_segment` finds the edges and vertices near a segment. Either way it gets every
/// one that may be near, in id order, and tests them exactly, so the drawing is the same.
pub(crate) enum Nearby {
    /// Every edge and vertex of the model.
    Scan,
    /// Indexes over the vertices and edges with ids in consecutive ranges (ones removed since
    /// are skipped), plus a short tail of everything newer, which is scanned. A level is added
    /// when the tail grows past `TAIL` ids and merged with the one before it while it is at
    /// least half that one's size, so each id is indexed O(log n) times.
    Indexed(Vec<Level>),
}

/// An index over the vertices and edges with ids from the previous level's `end` (0 for the
/// first) up to `end`.
pub(crate) struct Level {
    index: Index,
    end: u64,
}

#[cfg(test)]
thread_local! {
    /// Tests set this to make drawing always (true) or never (false) look things up with
    /// indexes, to compare the two.
    pub(crate) static FORCE_INDEXES: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// `usual`, or what a test forced with `FORCE_INDEXES`.
pub(crate) fn use_indexes(usual: bool) -> bool {
    #[cfg(test)]
    if let Some(forced) = FORCE_INDEXES.with(|f| f.get()) {
        return forced;
    }
    usual
}

impl Nearby {
    fn for_batch(segments: usize) -> Nearby {
        if use_indexes(segments >= INDEXED_BATCH) {
            Nearby::Indexed(Vec::new())
        } else {
            Nearby::Scan
        }
    }

    /// Indexes the tail when it is long (or everything, the first time).
    fn refresh(&mut self, m: &Model) {
        let Nearby::Indexed(levels) = self else {
            return;
        };
        let start = |levels: &[Level], k: usize| if k == 0 { 0 } else { levels[k - 1].end };
        let last = levels.last().map_or(0, |l| l.end);
        if !levels.is_empty() && m.next_id() - last <= TAIL {
            return;
        }
        levels.push(Level {
            index: Index::of_points_and_edges(m, last..m.next_id()),
            end: m.next_id(),
        });
        while let [.., a, b] = &levels[..]
            && 2 * b.index.len() >= a.index.len()
        {
            let n = levels.len();
            let end = b.end;
            let merged = Index::of_points_and_edges(m, start(levels, n - 2)..end);
            levels.truncate(n - 2);
            levels.push(Level { index: merged, end });
        }
    }

    /// The edges that may lie within `LENGTH_TOL` of the segment a-b, in id order.
    fn edges(&self, m: &Model, a: DVec3, b: DVec3) -> Vec<EdgeId> {
        match self {
            Nearby::Scan => m.edges().map(|(id, _)| id).collect(),
            Nearby::Indexed(levels) => {
                let mut found = Vec::new();
                for level in levels {
                    level.index.near_segment(a, b, LENGTH_TOL, |s| {
                        if let Shape::Segment(id, ..) = *s
                            && m.edge(id).is_some()
                        {
                            found.push(id);
                        }
                    });
                }
                found.sort();
                found.extend(m.edges_in(levels.last().map_or(0, |l| l.end)..u64::MAX));
                found
            }
        }
    }

    /// The vertices that may lie within `LENGTH_TOL` of the segment a-b, in id order.
    fn vertices(&self, m: &Model, a: DVec3, b: DVec3) -> Vec<VertexId> {
        match self {
            Nearby::Scan => m.vertices().map(|(id, _)| id).collect(),
            Nearby::Indexed(levels) => {
                let mut found = Vec::new();
                for level in levels {
                    level
                        .index
                        .near_segment(a, b, LENGTH_TOL + 1e-12 * a.distance(b), |s| {
                            if let Shape::Point(id, _) = *s
                                && m.vertex(id).is_some()
                            {
                                found.push(id);
                            }
                        });
                }
                found.sort();
                found.extend(m.vertices_in(levels.last().map_or(0, |l| l.end)..u64::MAX));
                found
            }
        }
    }
}

pub(crate) fn check_segment(a: DVec3, b: DVec3) -> Result<(), KernelError> {
    if !a.is_finite() || !b.is_finite() {
        return Err(KernelError::NotFinite);
    }
    if a.distance(b) < LENGTH_TOL {
        return Err(KernelError::ZeroLength);
    }
    Ok(())
}

pub(crate) fn loop_has_side(l: &[VertexId], a: VertexId, b: VertexId) -> bool {
    (0..l.len()).any(|i| {
        let (x, y) = (l[i], l[(i + 1) % l.len()]);
        (x, y) == (a, b) || (x, y) == (b, a)
    })
}

pub(crate) fn insert_between(l: &mut Vec<VertexId>, a: VertexId, b: VertexId, m: VertexId) {
    for i in 0..l.len() {
        let (x, y) = (l[i], l[(i + 1) % l.len()]);
        if (x, y) == (a, b) || (x, y) == (b, a) {
            l.insert(i + 1, m);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use glam::DVec3;

    use super::FORCE_INDEXES;
    use crate::data::SceneData;
    use crate::import::MeshPolygon;
    use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn everything(s: &Scene) -> (Vec<crate::EdgeId>, Vec<crate::FaceId>) {
        let m = s.root();
        (
            m.edges().map(|(id, _)| id).collect(),
            m.faces().map(|(id, _)| id).collect(),
        )
    }

    /// A 4 x 4 square on the ground at `x` with a 2 x 2 opening in the middle.
    fn frame(s: &mut Scene, x: f64) -> Vec<SceneChange> {
        let sq = |x0: f64, y0: f64, w: f64| {
            vec![
                p(x + x0, y0, 0.0),
                p(x + x0 + w, y0, 0.0),
                p(x + x0 + w, y0 + w, 0.0),
                p(x + x0, y0 + w, 0.0),
            ]
        };
        let mut out = vec![
            s.edit(MeshId::Root, |m| m.add_polygon(&sq(0.0, 0.0, 4.0)))
                .unwrap(),
            s.edit(MeshId::Root, |m| m.add_polygon(&sq(1.0, 1.0, 2.0)))
                .unwrap(),
        ];
        let inner = s
            .root()
            .faces()
            .find(|(id, _)| s.root().face_area(*id) == 4.0)
            .unwrap()
            .0;
        out.push(
            s.edit(MeshId::Root, |m| Ok(m.erase(&[], &[inner])))
                .unwrap(),
        );
        out
    }

    fn circle(c: DVec3, r: f64, sides: usize) -> Vec<DVec3> {
        (0..sides)
            .map(|i| {
                let a = TAU * i as f64 / sides as f64;
                c + p(a.cos(), a.sin(), 0.0) * r
            })
            .collect()
    }

    /// A bent `n` x `n` terrain.
    fn terrain(n: usize, at: DVec3) -> (Vec<DVec3>, Vec<MeshPolygon>) {
        let points = (0..=n)
            .flat_map(|y| {
                (0..=n).map(move |x| {
                    let (x, y) = (x as f64, y as f64);
                    at + p(x, y, (x * 0.37).sin() * (y * 0.23).cos())
                })
            })
            .collect();
        let k = |x: usize, y: usize| y * (n + 1) + x;
        let polygons = (0..n)
            .flat_map(|y| (0..n).map(move |x| (x, y)))
            .map(|(x, y)| MeshPolygon {
                corners: vec![k(x, y), k(x + 1, y), k(x + 1, y + 1), k(x, y + 1)],
                material: None,
            })
            .collect();
        (points, polygons)
    }

    type Scenario = fn(&mut Scene) -> Vec<SceneChange>;

    fn copies_with_openings(s: &mut Scene) -> Vec<SceneChange> {
        let mut out = frame(s, 0.0);
        let (edges, faces) = everything(s);
        out.extend([
            // Overlapping the original (edges cross the opening and the frame) and apart.
            s.edit(MeshId::Root, |m| {
                m.copy(&edges, &faces, p(2.0, 1.0, 0.0), 3)
            })
            .unwrap(),
            s.edit(MeshId::Root, |m| {
                m.copy(&edges, &faces, p(0.0, 9.0, 0.0), 2)
            })
            .unwrap(),
        ]);
        out
    }

    fn crossing_circles(s: &mut Scene) -> Vec<SceneChange> {
        let mut out = Vec::new();
        for k in 0..6 {
            let c = p(k as f64 * 0.7, (k % 2) as f64 * 0.3, 0.0);
            out.push(
                s.edit(MeshId::Root, |m| {
                    m.add_curve(&circle(c, 0.5, 24), true, false)
                })
                .unwrap(),
            );
        }
        let (edges, faces) = everything(s);
        out.push(
            s.edit(MeshId::Root, |m| {
                m.copy(&edges, &faces, p(0.35, 0.2, 0.0), 2)
            })
            .unwrap(),
        );
        out
    }

    fn tubes(s: &mut Scene) -> Vec<SceneChange> {
        let mut out = frame(s, 0.0);
        let ring = s.root().faces().next().unwrap().0;
        out.push(s.edit(MeshId::Root, |m| m.push_pull(ring, 2.0)).unwrap());
        let (edges, faces) = everything(s);
        out.push(
            s.edit(MeshId::Root, |m| {
                m.copy(&edges, &faces, p(2.0, 0.5, 1.0), 2)
            })
            .unwrap(),
        );
        // A tube through a slab: push a circle in the top down through.
        out.push(
            s.edit(MeshId::Root, |m| {
                m.add_curve(&circle(p(2.0, 2.0, 2.0), 0.5, 16), true, false)
            })
            .unwrap(),
        );
        out
    }

    fn imports_and_explodes(s: &mut Scene) -> Vec<SceneChange> {
        let mut out = Vec::new();
        // A ground square under part of the terrain, so the explode cuts into it.
        out.push(
            s.edit(MeshId::Root, |m| {
                m.add_polygon(&[
                    p(-1.0, -1.0, 0.0),
                    p(3.5, -1.0, 0.0),
                    p(3.5, 3.5, 0.0),
                    p(-1.0, 3.5, 0.0),
                ])
            })
            .unwrap(),
        );
        let (points, polygons) = terrain(8, p(0.0, 0.0, 0.0));
        let (i, _, change) = s
            .import_mesh(
                MeshId::Root,
                &points,
                &polygons,
                true,
                DefinitionKind::Group,
                "t",
            )
            .unwrap();
        out.push(change);
        let (points, polygons) = terrain(6, p(3.0, 2.0, 0.0));
        let (j, _, change) = s
            .import_mesh(
                MeshId::Root,
                &points,
                &polygons,
                false,
                DefinitionKind::Group,
                "u",
            )
            .unwrap();
        out.push(change);
        out.push(s.explode(MeshId::Root, i).unwrap());
        out.push(s.explode(MeshId::Root, j).unwrap());
        // Group what is there and explode it back over a copy of itself.
        let (edges, faces) = everything(s);
        let (g, change) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Group,
                "g",
            )
            .unwrap();
        out.push(change);
        out.push(
            s.copy_instances(MeshId::Root, &[g], p(1.5, 0.5, 0.0), 1)
                .unwrap(),
        );
        out.push(s.explode(MeshId::Root, g).unwrap());
        out
    }

    fn line_grid(s: &mut Scene) -> Vec<SceneChange> {
        // Lines crossing each other, ending on others, overlapping collinear ones, in one
        // batch (as a copy draws them) and one by one.
        let mut segs = Vec::new();
        for k in 0..6 {
            let t = k as f64;
            segs.push((p(0.0, t, 0.0), p(5.0, t, 0.0)));
            segs.push((p(t, 0.0, 0.0), p(t, 5.0, 0.0)));
            segs.push((p(t * 0.5, 0.0, 0.0), p(t * 0.5 + 2.0, 0.0, 0.0)));
            segs.push((p(0.0, 0.0, 0.0), p(5.0, t, 0.0)));
        }
        let mut out = vec![
            s.edit(MeshId::Root, |m| {
                m.draw_segments(&segs);
                Ok(m.take_changes())
            })
            .unwrap(),
        ];
        out.push(
            s.edit(MeshId::Root, |m| {
                m.add_edge(p(0.5, -1.0, 0.0), p(4.5, 6.0, 0.0))
            })
            .unwrap(),
        );
        out
    }

    fn run(indexed: bool, scenario: Scenario) -> (SceneData, Vec<SceneChange>) {
        FORCE_INDEXES.with(|f| f.set(Some(indexed)));
        let mut s = Scene::new();
        let changes = scenario(&mut s);
        FORCE_INDEXES.with(|f| f.set(None));
        s.validate().unwrap();
        (s.to_data(), changes)
    }

    #[test]
    fn drawing_with_the_index_gives_exactly_what_scanning_gives() {
        let scenarios: [(&str, Scenario); 5] = [
            ("copies with openings", copies_with_openings),
            ("crossing circles", crossing_circles),
            ("push/pull tubes", tubes),
            ("imports and explodes", imports_and_explodes),
            ("line grid", line_grid),
        ];
        for (what, scenario) in scenarios {
            let (scan, scan_changes) = run(false, scenario);
            let (indexed, indexed_changes) = run(true, scenario);
            assert_eq!(indexed, scan, "{what}");
            assert_eq!(indexed_changes, scan_changes, "{what}");
            // Undoing every change in reverse gives back an empty scene either way.
            let mut s = Scene::from_data(&indexed).unwrap();
            assert!(s.root().faces().count() > 0, "{what}");
            for c in indexed_changes.iter().rev() {
                s.apply(&c.inverse());
            }
            assert!(s.root().is_empty(), "{what}");
        }
    }
}
