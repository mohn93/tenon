//! What tools see: the open context's mesh plus every other placed mesh of the scene, all in
//! the open context's coordinates. What hidden tags hide is not seen (see `kernel::tag_shown_among`):
//! instances on hidden tags are left out with all they place (but for those the open context
//! is in), guides and annotations on hidden tags too, and each part tells which of its faces,
//! edges and vertices show.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::ops::Deref;
use std::sync::OnceLock;

use glam::{DAffine3, DMat3, DVec2, DVec3};
use kernel::geom::LENGTH_TOL;
use kernel::{
    EdgeId, EntityId, FaceId, Guide, GuideId, InstanceId, MeshId, Model, Scene, TagId, VertexId,
    tag_shown_among,
};
use session::{AnnotationShape, Camera};

/// Whose geometry a part is, seen from the open context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The open context's own mesh: editable, its entities can be picked.
    Open,
    /// Geometry placed (at any depth) by this instance of the open context: picking it picks
    /// the instance.
    Instance(InstanceId),
    /// Geometry outside the open context: seen and snapped to, never picked.
    Outside,
}

/// An annotation's label as the window last laid it out: whose it is, the dimension or note
/// (an id in its own mesh), and its rectangle in physical view pixels (as the cursor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Label {
    pub owner: Owner,
    pub id: EntityId,
    pub min: DVec2,
    pub max: DVec2,
}

impl Label {
    /// Whether `p` lies on the label.
    pub fn contains(&self, p: DVec2) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }
}

/// Where one mesh is placed, seen from the open context.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    /// Maps the mesh's coordinates into the open context's. Usually rigid (placements turn and
    /// move), though a file may hold any invertible placement.
    pub place: DAffine3,
    /// Maps the open context's coordinates into the mesh's: the inverse of `place`.
    pub back: DAffine3,
    pub owner: Owner,
    /// A box in the open context's coordinates around everything of the mesh its spatial index
    /// holds (vertices, edges and face triangles). None when there is no box to go by (a mesh
    /// without vertices), so the part is never skipped.
    bounds: Option<[DVec3; 2]>,
    /// How `back` stretches lengths.
    stretch: Stretch,
    /// Which of the layout's meshes is placed.
    mesh: usize,
}

impl Placed {
    /// False when the ray `origin + t * dir` meets nothing of the mesh with `0 <= t <= reach`,
    /// as told by its box (true when unsure). A ray carried into the mesh's coordinates by
    /// `back` reaches the same points at the same `t`.
    pub fn may_meet_ray(&self, origin: DVec3, dir: DVec3, reach: f64) -> bool {
        self.bounds
            .is_none_or(|[lo, hi]| ray_meets_box(origin, dir, lo, hi, reach))
    }

    /// False when nothing of the mesh lies within `r0 + slope * t` of the ray `origin + t *
    /// dir` (unit `dir`; `t` counted from 0, and the reach is `r0` behind the origin), as told
    /// by its box (true when unsure): the cone around a cursor that `Index::near_ray` searches.
    pub fn may_meet_cone(&self, origin: DVec3, dir: DVec3, r0: f64, slope: f64) -> bool {
        self.bounds.is_none_or(|[lo, hi]| {
            let centre = (lo + hi) * 0.5;
            let radius = (hi - lo).length() * 0.5;
            let rel = centre - origin;
            let t = rel.dot(dir);
            let off = (rel - dir * t).length();
            off <= radius + r0 + slope * (t + radius).max(0.0)
        })
    }

    /// How `back` stretches lengths, for carrying a cone into the mesh's coordinates.
    pub(crate) fn stretch(&self) -> Stretch {
        self.stretch
    }
}

/// How a linear map `B` stretches lengths. `BᵀB` is `c²` times the identity (`c²` the mean
/// of its diagonal) plus a matrix `E` of Frobenius norm `off`, and no unit vector `w` has
/// `|wᵀ E v|` above that for a unit `v`. So `|B w|² = c² + wᵀ E w <= most²`, `most` being
/// `sqrt(c² + off)`, and for `w` across `v`, `B w · B v = wᵀ E v` is at most `off`. A map
/// that turns, mirrors and scales evenly has `off` 0, but for rounding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stretch {
    pub most: f64,
    pub off: f64,
}

impl Stretch {
    pub(crate) fn of(b: DMat3) -> Stretch {
        let m = b.transpose() * b;
        let mean = (m.x_axis.x + m.y_axis.y + m.z_axis.z) / 3.0;
        let e = m - DMat3::from_diagonal(DVec3::splat(mean));
        let off =
            (e.x_axis.length_squared() + e.y_axis.length_squared() + e.z_axis.length_squared())
                .sqrt();
        Stretch {
            most: (mean + off).sqrt(),
            off,
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

/// The box around a mesh's vertices, padded by `LENGTH_TOL` as its spatial index pads every
/// shape: it holds every vertex, edge and face triangle (whose corners are vertices) of the
/// index. None for a mesh without vertices.
fn mesh_box(model: &Model) -> Option<[DVec3; 2]> {
    let mut points = model.vertices().map(|(_, v)| v.pos);
    let first = points.next()?;
    let (lo, hi) = points.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
    Some([lo - DVec3::splat(LENGTH_TOL), hi + DVec3::splat(LENGTH_TOL)])
}

/// The box around the box `lo..hi` placed by `place`, padded so that rounding in where points
/// land never puts them outside it.
fn placed_box(place: DAffine3, [lo, hi]: [DVec3; 2]) -> [DVec3; 2] {
    let (mut a, mut b) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    for k in 0..8 {
        let corner = DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        );
        let p = place.transform_point3(corner);
        a = a.min(p);
        b = b.max(p);
    }
    let pad = LENGTH_TOL + 1e-9 * a.abs().max(b.abs()).max_element();
    [a - pad, b + pad]
}

/// One placed mesh: the mesh, plus where and whose it is (its `Placed`, through `Deref`).
#[derive(Clone, Copy, Debug)]
pub struct Part<'s> {
    pub model: &'s Model,
    placed: &'s Placed,
    /// The tags hidden.
    hidden: &'s BTreeSet<TagId>,
}

impl Part<'_> {
    fn shows(&self, tag: Option<TagId>) -> bool {
        tag_shown_among(self.hidden, tag)
    }

    /// Whether the face shows: it is not on a hidden tag.
    pub fn shows_face(&self, f: FaceId) -> bool {
        self.hidden.is_empty() || self.model.face(f).is_some_and(|f| self.shows(f.tag))
    }

    /// Whether the edge shows: it is not on a hidden tag.
    pub fn shows_edge(&self, e: EdgeId) -> bool {
        self.hidden.is_empty() || self.model.edge(e).is_some_and(|e| self.shows(e.tag))
    }

    /// Whether the vertex shows: some edge or face using it does (see `Model::vertex_shown`).
    pub fn shows_vertex(&self, v: VertexId) -> bool {
        self.model.vertex_shown(v, self.hidden)
    }
}

impl Deref for Part<'_> {
    type Target = Placed;

    fn deref(&self) -> &Placed {
        self.placed
    }
}

/// Where every mesh of a scene is placed, seen from an open context: all that tools need to
/// know about the parts but the meshes themselves. It borrows nothing, so it can be kept while
/// the scene and the open context stay the same, and the scene looked at through it
/// (`Scenery::view`) without walking it again.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The meshes placed, each once.
    meshes: Vec<MeshId>,
    /// Every placed mesh, the open context's first, then the others in walk order.
    parts: Vec<Placed>,
    /// Each part's path: the instances from the root down to the one placing it (empty for
    /// the root).
    paths: Vec<Vec<InstanceId>>,
    /// Every guide of every part, placed in the open context's coordinates, with the index of
    /// the part it comes from, in part and id order.
    guides: Vec<(usize, GuideId, Guide)>,
    /// Every dimension and note of every part not on a hidden tag, placed in the open
    /// context's coordinates (a screen note as it is), with the index of the part it comes
    /// from, in part order, dimensions before notes, each in id order.
    annotations: Vec<(usize, EntityId, AnnotationShape)>,
    /// The instances of the open context, by id, each with its box once asked for.
    boxes: Vec<(InstanceId, OnceLock<Option<[DVec3; 2]>>)>,
    /// The tags hidden: what is on them is not seen.
    hidden: BTreeSet<TagId>,
}

impl Layout {
    /// The whole scene seen from the open context `context` (a path of instances from the
    /// root; see `session::Document::context`). None if the path does not resolve.
    pub fn of(scene: &Scene, context: &[InstanceId]) -> Option<Layout> {
        let (_, to_root) = scene.resolve(context)?;
        let hidden = scene.hidden_tags();
        let mut found = Vec::new();
        collect(
            scene,
            &hidden,
            MeshId::Root,
            to_root.inverse(),
            context,
            &mut Vec::new(),
            &mut found,
        );
        // The open mesh first, then the others in walk order.
        found.sort_by_key(|f| f.owner != Owner::Open);
        let mut slots: HashMap<MeshId, usize> = HashMap::new();
        let mut meshes = Vec::new();
        let mut models = Vec::new();
        let mut paths = Vec::new();
        let parts = found
            .into_iter()
            .map(|f| {
                let mesh = *slots.entry(f.id).or_insert_with(|| {
                    meshes.push(f.id);
                    models.push(f.model);
                    meshes.len() - 1
                });
                paths.push(f.path);
                (mesh, f.place, f.owner)
            })
            .collect();
        Some(Layout::new(meshes, &models, parts, paths, hidden))
    }

    /// A layout of `parts` (each the index of its mesh in `meshes`, its placement and its
    /// owner) reached by `paths`, with the tags `hidden`; `models` are the meshes themselves.
    fn new(
        meshes: Vec<MeshId>,
        models: &[&Model],
        parts: Vec<(usize, DAffine3, Owner)>,
        paths: Vec<Vec<InstanceId>>,
        hidden: BTreeSet<TagId>,
    ) -> Layout {
        let boxes: Vec<Option<[DVec3; 2]>> = models.iter().map(|m| mesh_box(m)).collect();
        let parts: Vec<Placed> = parts
            .into_iter()
            .map(|(mesh, place, owner)| {
                let back = place.inverse();
                Placed {
                    place,
                    back,
                    owner,
                    bounds: boxes[mesh].map(|b| placed_box(place, b)),
                    stretch: Stretch::of(back.matrix3),
                    mesh,
                }
            })
            .collect();
        let guides = parts
            .iter()
            .enumerate()
            .flat_map(|(k, part)| {
                models[part.mesh]
                    .guides()
                    .filter(|(_, g)| tag_shown_among(&hidden, g.tag()))
                    .map(move |(id, g)| (k, id, g.transformed(part.place)))
            })
            .collect();
        let annotations = parts
            .iter()
            .enumerate()
            .flat_map(|(k, part)| {
                let m = models[part.mesh];
                let dimensions = m
                    .dimensions()
                    .filter(|(_, d)| tag_shown_among(&hidden, d.tag()))
                    .map(|(id, _)| EntityId::Dimension(id));
                let notes = m
                    .notes()
                    .filter(|(_, n)| tag_shown_among(&hidden, n.tag()))
                    .map(|(id, _)| EntityId::Note(id));
                dimensions.chain(notes).filter_map(move |id| {
                    let shape = AnnotationShape::of(m, id)?;
                    Some((k, id, shape.transformed(part.place)))
                })
            })
            .collect();
        let mut instances: Vec<InstanceId> = parts
            .iter()
            .filter_map(|p| match p.owner {
                Owner::Instance(i) => Some(i),
                _ => None,
            })
            .collect();
        instances.sort();
        instances.dedup();
        Layout {
            meshes,
            parts,
            paths,
            guides,
            annotations,
            boxes: instances
                .into_iter()
                .map(|i| (i, OnceLock::new()))
                .collect(),
            hidden,
        }
    }

    /// The layout's meshes in `scene`, in its order. None if one is missing.
    fn models<'a>(&self, scene: &'a Scene) -> Option<Vec<&'a Model>> {
        self.meshes.iter().map(|id| scene.mesh(*id)).collect()
    }
}

/// Every placed mesh, the open context's first, all in the open context's coordinates.
#[derive(Clone, Debug)]
pub struct Scenery<'a> {
    /// The layout's meshes, in its order.
    models: Vec<&'a Model>,
    layout: Cow<'a, Layout>,
    /// When set, only what lies at this depth counts (see [`Slab`]).
    slab: Option<Slab>,
}

/// The depths around a model point the cursor stands for: what lies in front of the point or
/// behind it does not count, so the point is looked for where it is even when other geometry
/// hides it from the camera. Points and edges snap only within the snap radius's depth of the
/// point; faces count only through the point itself (within `LENGTH_TOL`), so a thin wall's
/// front does not stand in for its back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slab {
    /// The model point.
    pub point: DVec3,
    eye: DVec3,
    forward: DVec3,
    near: f64,
    far: f64,
    /// The depths faces count within.
    face_near: f64,
    face_far: f64,
}

impl Slab {
    /// The slab around `point` as deep either way as `radius_px` pixels are wide there, the
    /// distance inference snaps across.
    pub fn around(camera: &Camera, point: DVec3, radius_px: f64) -> Slab {
        let (eye, forward) = (camera.eye, camera.forward());
        let depth = (point - eye).dot(forward);
        let tight = LENGTH_TOL + depth.abs() * 1e-9;
        let half = radius_px * camera.pixel_size(point) + tight;
        Slab {
            point,
            eye,
            forward,
            near: depth - half,
            far: depth + half,
            face_near: depth - tight,
            face_far: depth + tight,
        }
    }

    /// True when `p` lies within the slab's depths.
    pub fn holds(&self, p: DVec3) -> bool {
        let depth = (p - self.eye).dot(self.forward);
        depth >= self.near && depth <= self.far
    }

    /// The stretch `t0..t1` (with `t0 >= 0`) of the line `origin + t * dir` within the depths
    /// faces count within, if any.
    pub fn face_span(&self, origin: DVec3, dir: DVec3) -> Option<(f64, f64)> {
        let depth0 = (origin - self.eye).dot(self.forward);
        let slope = dir.dot(self.forward);
        let (near, far) = (self.face_near, self.face_far);
        let (lo, hi) = if slope.abs() < 1e-12 {
            if !(near..=far).contains(&depth0) {
                return None;
            }
            (0.0, f64::INFINITY)
        } else {
            let (a, b) = ((near - depth0) / slope, (far - depth0) / slope);
            (a.min(b).max(0.0), a.max(b))
        };
        (lo <= hi).then_some((lo, hi))
    }
}

impl<'a> Scenery<'a> {
    /// Just one mesh, open, in its own coordinates.
    pub fn single(model: &'a Model) -> Self {
        let parts = vec![(0, DAffine3::IDENTITY, Owner::Open)];
        Scenery {
            slab: None,
            models: vec![model],
            layout: Cow::Owned(Layout::new(
                vec![MeshId::Root],
                &[model],
                parts,
                vec![Vec::new()],
                BTreeSet::new(),
            )),
        }
    }

    /// The whole scene seen from the open context `context` (a path of instances from the
    /// root; see `session::Document::context`). None if the path does not resolve.
    pub fn of(scene: &'a Scene, context: &[InstanceId]) -> Option<Self> {
        let layout = Layout::of(scene, context)?;
        Some(Scenery {
            models: layout.models(scene)?,
            layout: Cow::Owned(layout),
            slab: None,
        })
    }

    /// `scene` seen through `layout`, which must have been made from this scene as it is now
    /// (no edit since) and the open context it is to be seen from. None if a mesh the layout
    /// places is missing.
    pub fn view(layout: &'a Layout, scene: &'a Scene) -> Option<Self> {
        Some(Scenery {
            models: layout.models(scene)?,
            layout: Cow::Borrowed(layout),
            slab: None,
        })
    }

    /// The same scenery seen only within `slab`: for a model point the cursor stands for.
    pub fn within(mut self, slab: Slab) -> Self {
        self.slab = Some(slab);
        self
    }

    /// The slab the scenery is seen within, if any.
    pub fn slab(&self) -> Option<Slab> {
        self.slab
    }

    /// How many meshes are placed.
    pub fn len(&self) -> usize {
        self.layout.parts.len()
    }

    /// Never true: the open context's mesh is always there.
    pub fn is_empty(&self) -> bool {
        self.layout.parts.is_empty()
    }

    /// The `k`th placed mesh.
    pub fn part(&self, k: usize) -> Part<'_> {
        let placed = &self.layout.parts[k];
        Part {
            model: self.models[placed.mesh],
            placed,
            hidden: &self.layout.hidden,
        }
    }

    /// Every placed mesh, the open context's first.
    pub fn parts(&self) -> impl ExactSizeIterator<Item = Part<'_>> + '_ {
        self.layout.parts.iter().map(|placed| Part {
            model: self.models[placed.mesh],
            placed,
            hidden: &self.layout.hidden,
        })
    }

    /// The instances from the root down to the one placing the `k`th part (empty for the
    /// root's own mesh), whatever the open context.
    pub fn path(&self, k: usize) -> &[InstanceId] {
        &self.layout.paths[k]
    }

    /// Every guide of every part, placed in the open context's coordinates, with the index of
    /// the part it comes from, in part and id order.
    pub fn guides(&self) -> &[(usize, GuideId, Guide)] {
        &self.layout.guides
    }

    /// Every dimension and note of every part, placed in the open context's coordinates (a
    /// screen note as it is), with the index of the part it comes from. Those on hidden tags
    /// are left out.
    pub fn annotations(&self) -> &[(usize, EntityId, AnnotationShape)] {
        &self.layout.annotations
    }

    /// Whether what is on `tag` (None: Untagged) shows.
    pub fn shows(&self, tag: Option<TagId>) -> bool {
        tag_shown_among(&self.layout.hidden, tag)
    }

    /// The open context's own mesh.
    pub fn open(&self) -> &'a Model {
        self.models[self.layout.parts[0].mesh]
    }

    /// True when an instance is open (there is geometry outside the open context or the root
    /// is not what is open).
    pub fn inside_instance(&self) -> bool {
        self.layout.parts.iter().any(|p| p.owner == Owner::Outside)
    }

    /// The box around everything an instance of the open context places, in the open
    /// context's coordinates. None when it places nothing. Worked out once per layout.
    pub fn instance_box(&self, id: InstanceId) -> Option<[DVec3; 2]> {
        let k = self
            .layout
            .boxes
            .binary_search_by_key(&id, |(i, _)| *i)
            .ok()?;
        *self.layout.boxes[k].1.get_or_init(|| {
            let mut points = self
                .parts()
                .filter(|p| p.owner == Owner::Instance(id))
                .flat_map(|p| {
                    p.model
                        .vertices()
                        .filter(move |(v, _)| p.shows_vertex(*v))
                        .map(move |(_, v)| p.place.transform_point3(v.pos))
                });
            let first = points.next()?;
            Some(points.fold([first, first], |[lo, hi], q| [lo.min(q), hi.max(q)]))
        })
    }
}

/// A mesh found placed in the scene.
struct Found<'a> {
    id: MeshId,
    model: &'a Model,
    /// Into the open context's coordinates.
    place: DAffine3,
    owner: Owner,
    /// The instances from the root down to the one placing it.
    path: Vec<InstanceId>,
}

/// Adds the mesh `id`, placed by `place` (into the open context's coordinates) and reached by
/// `path`, and everything it places, each with its owner. Instances on the tags `hidden` are
/// left out with all they place, unless the open context is in them.
fn collect<'a>(
    scene: &'a Scene,
    hidden: &BTreeSet<TagId>,
    id: MeshId,
    place: DAffine3,
    context: &[InstanceId],
    path: &mut Vec<InstanceId>,
    found: &mut Vec<Found<'a>>,
) {
    let Some(model) = scene.mesh(id) else { return };
    let owner = if path.as_slice() == context {
        Owner::Open
    } else if path.starts_with(context) {
        Owner::Instance(path[context.len()])
    } else {
        Owner::Outside
    };
    found.push(Found {
        id,
        model,
        place,
        owner,
        path: path.clone(),
    });
    for (iid, instance) in model.instances() {
        path.push(iid);
        let shown = tag_shown_among(hidden, instance.tag);
        if !shown && !context.starts_with(path) {
            path.pop();
            continue;
        }
        collect(
            scene,
            hidden,
            MeshId::Definition(instance.definition),
            place * instance.transform,
            context,
            path,
            found,
        );
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::DefinitionKind;

    /// A root holding a square at the origin, grouped, plus a loose edge; the group is moved
    /// by (5, 0, 0). Returns the scene and the instance.
    fn scene() -> (Scene, InstanceId) {
        let mut m = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
            .unwrap();
        let mut s = Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_edge(p(0.0, 3.0), p(1.0, 3.0))?;
            c.ops
                .extend(m.move_instances(&[i], DVec3::new(5.0, 0.0, 0.0)).ops);
            Ok(c)
        })
        .unwrap();
        (s, i)
    }

    #[test]
    fn from_the_root_instances_are_placed_where_they_are_drawn() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v.part(0).owner, Owner::Open);
        assert_eq!(v.part(1).owner, Owner::Instance(i));
        assert!(!v.inside_instance());
        assert_eq!(
            v.instance_box(i),
            Some([DVec3::new(5.0, 0.0, 0.0), DVec3::new(6.0, 1.0, 0.0)])
        );
    }

    #[test]
    fn parts_know_their_path_from_the_root() {
        let (s, i) = scene();
        for context in [vec![], vec![i]] {
            let v = Scenery::of(&s, &context).unwrap();
            let paths: Vec<&[InstanceId]> = (0..v.len()).map(|k| v.path(k)).collect();
            if context.is_empty() {
                assert_eq!(paths, vec![&[][..], &[i][..]]);
            } else {
                assert_eq!(paths, vec![&[i][..], &[][..]]);
            }
        }
        let m = Model::new();
        assert!(Scenery::single(&m).path(0).is_empty());
    }

    #[test]
    fn inside_an_instance_the_rest_is_outside_and_shifted() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[i]).unwrap();
        assert_eq!(v.part(0).owner, Owner::Open);
        assert_eq!(v.part(0).place, DAffine3::IDENTITY);
        assert_eq!(v.part(1).owner, Owner::Outside);
        // The root's loose edge at x 0..1 lies at x -5..-4 in the group's coordinates.
        assert_eq!(
            v.part(1).place,
            DAffine3::from_translation(DVec3::new(-5.0, 0.0, 0.0))
        );
        assert!(v.inside_instance());
        assert!(Scenery::of(&s, &[InstanceId(999)]).is_none());
    }
}
