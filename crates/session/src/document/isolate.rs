//! The notes and dimensions a viewer's Isolate hides with the parts: a root leader note points
//! at whatever its anchor lies nearest, a linear dimension measures what its ends lie nearest
//! and a radial one the circle's rim, and each goes when that is a hidden part.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use glam::{DAffine3, DVec3};
use kernel::bvh::closest_on_triangle;
use kernel::{Anchor, DimensionId, FaceId, InstanceId, MeshId, Note, NoteId, Scene};

/// The box around `points`; None for none.
fn enclose(points: impl IntoIterator<Item = DVec3>) -> Option<[DVec3; 2]> {
    points.into_iter().fold(None, |b, p| match b {
        None => Some([p, p]),
        Some([lo, hi]) => Some([lo.min(p), hi.max(p)]),
    })
}

/// The box around box `b` placed by `t`.
fn placed(t: DAffine3, [lo, hi]: [DVec3; 2]) -> [DVec3; 2] {
    let corners = (0..8).map(|k| {
        t.transform_point3(DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        ))
    });
    enclose(corners).expect("eight corners")
}

/// How far `p` is from the box `b` (0 inside it).
fn box_distance(p: DVec3, [lo, hi]: [DVec3; 2]) -> f64 {
    (lo - p).max(p - hi).max(DVec3::ZERO).length()
}

/// A mesh's shown faces, each with the box around it.
type FaceBoxes = Vec<(FaceId, [DVec3; 2])>;

/// What the meshes visited show, each worked out once: the box around what a mesh shows, the
/// box around each of its own shown faces, and the triangles of the faces a note's search
/// reaches past their boxes (most are never triangulated).
#[derive(Default)]
struct Meshes {
    bounds: HashMap<MeshId, Option<[DVec3; 2]>>,
    faces: HashMap<MeshId, Rc<FaceBoxes>>,
    triangles: HashMap<(MeshId, FaceId), Rc<Vec<[DVec3; 3]>>>,
}

impl Meshes {
    /// The box around what `mesh` shows, in its coordinates: `Scene::shown_bounds` (shown
    /// edges, faces and instances), with every mesh measured once however often it is placed.
    fn bounds(&mut self, scene: &Scene, mesh: MeshId) -> Option<[DVec3; 2]> {
        if let Some(b) = self.bounds.get(&mesh) {
            return *b;
        }
        // None while it is measured, so a definition that places itself ends.
        self.bounds.insert(mesh, None);
        let m = scene.mesh(mesh)?;
        let mut b = if m.has_hidden_edge_or_face() || scene.tags().any(|(_, t)| !t.visible) {
            let edges = m
                .edges()
                .filter(|(_, e)| scene.edge_shown(e))
                .flat_map(|(_, e)| e.v);
            let faces = m
                .faces()
                .filter(|(_, f)| scene.face_shown(f))
                .flat_map(|(_, f)| f.outer.iter().copied());
            enclose(edges.chain(faces).filter_map(|v| m.position(v)))
        } else {
            // Nothing of its own hidden: every vertex shows.
            enclose(m.vertices().map(|(_, v)| v.pos))
        };
        for (_, i) in m.instances() {
            if scene.placed_instance_shown(i)
                && let Some(inner) = self.bounds(scene, MeshId::Definition(i.definition))
            {
                let [lo, hi] = placed(i.transform, inner);
                b = enclose(b.into_iter().flatten().chain([lo, hi]));
            }
        }
        self.bounds.insert(mesh, b);
        b
    }

    /// `mesh`'s own shown faces with the box around each, in its coordinates.
    fn faces(&mut self, scene: &Scene, mesh: MeshId) -> Rc<FaceBoxes> {
        self.faces
            .entry(mesh)
            .or_insert_with(|| {
                let Some(m) = scene.mesh(mesh) else {
                    return Rc::new(Vec::new());
                };
                Rc::new(
                    m.faces()
                        .filter(|(_, f)| scene.face_shown(f))
                        .filter_map(|(id, f)| {
                            let b = enclose(f.outer.iter().filter_map(|v| m.position(*v)))?;
                            Some((id, b))
                        })
                        .collect(),
                )
            })
            .clone()
    }

    /// The triangles of face `face` of `mesh`, in the mesh's coordinates.
    fn triangles(&mut self, scene: &Scene, mesh: MeshId, face: FaceId) -> Rc<Vec<[DVec3; 3]>> {
        self.triangles
            .entry((mesh, face))
            .or_insert_with(|| {
                Rc::new(
                    scene
                        .mesh(mesh)
                        .and_then(|m| m.triangulate(face))
                        .unwrap_or_default(),
                )
            })
            .clone()
    }

    /// Lowers `best` to how far `p` (root coordinates) is from what `mesh`, placed by `place`
    /// (into root coordinates), shows when that is nearer. True if it was. A mesh (or a
    /// nested instance) whose box is no nearer than `best` is passed over; nested instances
    /// are searched nearest box first. `depth` stops a definition that places itself.
    fn nearer(
        &mut self,
        scene: &Scene,
        mesh: MeshId,
        place: DAffine3,
        p: DVec3,
        best: &mut f64,
        depth: usize,
    ) -> bool {
        match self.bounds(scene, mesh) {
            Some(b) if depth <= MAX_DEPTH && box_distance(p, placed(place, b)) < *best => {}
            _ => return false,
        }
        let mut found = false;
        let local = place.inverse().transform_point3(p);
        // Faces nearest box first; only those whose box is nearer than the best are triangulated.
        let mut faces: Vec<(f64, FaceId)> = self
            .faces(scene, mesh)
            .iter()
            .map(|(id, b)| (box_distance(p, placed(place, *b)), *id))
            .collect();
        faces.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (near, face) in faces {
            if near >= *best {
                break;
            }
            for t in self.triangles(scene, mesh, face).iter() {
                let d = place
                    .transform_point3(closest_on_triangle(local, *t))
                    .distance(p);
                if d < *best {
                    *best = d;
                    found = true;
                }
            }
        }
        let Some(m) = scene.mesh(mesh) else {
            return found;
        };
        let mut inner: Vec<(f64, MeshId, DAffine3)> = Vec::new();
        for (_, instance) in m.instances() {
            if !scene.placed_instance_shown(instance) {
                continue;
            }
            let id = MeshId::Definition(instance.definition);
            let at = place * instance.transform;
            if let Some(b) = self.bounds(scene, id) {
                inner.push((box_distance(p, placed(at, b)), id, at));
            }
        }
        inner.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (near, id, at) in inner {
            if near >= *best {
                break;
            }
            found |= self.nearer(scene, id, at, p, best, depth + 1);
        }
        found
    }
}

/// How deep nested instances are searched.
const MAX_DEPTH: usize = 64;

/// A point on the rim of the circle `c`.
fn rim_point(c: &kernel::Circle) -> DVec3 {
    let n = c.normal.try_normalize().unwrap_or(DVec3::Z);
    let axis = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    c.center + n.cross(axis).normalize() * c.radius.abs()
}

/// What the root shows, for finding which root part lies nearest a point.
struct Nearest<'a> {
    scene: &'a Scene,
    meshes: Meshes,
    own: Vec<[DVec3; 3]>,
    instances: Vec<(InstanceId, DAffine3, MeshId, [DVec3; 2])>,
}

impl<'a> Nearest<'a> {
    fn new(scene: &'a Scene) -> Self {
        let root = scene.root();
        let mut meshes = Meshes::default();
        let own = root
            .faces()
            .filter(|(_, f)| scene.face_shown(f))
            .flat_map(|(id, _)| root.triangulate(id).unwrap_or_default())
            .collect();
        let instances = root
            .instances()
            .filter(|(_, i)| scene.placed_instance_shown(i))
            .filter_map(|(id, i)| {
                let mesh = MeshId::Definition(i.definition);
                let b = meshes.bounds(scene, mesh)?;
                Some((id, i.transform, mesh, placed(i.transform, b)))
            })
            .collect();
        Nearest {
            scene,
            meshes,
            own,
            instances,
        }
    }

    /// Whether the root instance nearest `p` (nearer than the root's own faces) is in `hidden`.
    fn is_hidden(&mut self, p: DVec3, hidden: &BTreeSet<InstanceId>) -> bool {
        let mut best = self
            .own
            .iter()
            .map(|t| closest_on_triangle(p, *t).distance(p))
            .fold(f64::INFINITY, f64::min);
        let mut nearest = None;
        // Nearest boxes first, so most parts are passed over by their box.
        let mut order: Vec<(f64, usize)> = self
            .instances
            .iter()
            .enumerate()
            .map(|(k, (.., b))| (box_distance(p, *b), k))
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (near, k) in order {
            if near >= best {
                break;
            }
            let (id, place, mesh, _) = self.instances[k];
            if self.meshes.nearer(self.scene, mesh, place, p, &mut best, 1) {
                nearest = Some(id);
            }
        }
        nearest.is_some_and(|id| hidden.contains(&id))
    }
}

/// The root leader notes whose anchor lies nearer one of the root instances `hidden` than
/// anything else the root shows (its own faces and its other instances). A note anchored to
/// a root vertex points at the root's own geometry and always stays.
pub(super) fn notes_pointing_at(scene: &Scene, hidden: &BTreeSet<InstanceId>) -> BTreeSet<NoteId> {
    let root = scene.root();
    let anchored: Vec<(NoteId, DVec3)> = root
        .notes()
        .filter_map(|(id, note)| match note {
            Note::Leader {
                anchor: Anchor::Point(_),
                ..
            } => Some((id, root.note_point(id)?)),
            _ => None,
        })
        .collect();
    if anchored.is_empty() || hidden.is_empty() {
        return BTreeSet::new();
    }
    let mut nearest = Nearest::new(scene);
    anchored
        .into_iter()
        .filter(|(_, p)| nearest.is_hidden(*p, hidden))
        .map(|(id, _)| id)
        .collect()
}

/// The root dimensions that measure one of the root instances `hidden`: a linear one when
/// either end lies nearest it, a radial one when a point of its circle's rim does (a circle of
/// the root's own edges lies on the root's own geometry and stays).
pub(super) fn dimensions_measuring(
    scene: &Scene,
    hidden: &BTreeSet<InstanceId>,
) -> BTreeSet<DimensionId> {
    let root = scene.root();
    let probes: Vec<(DimensionId, Vec<DVec3>)> = root
        .dimensions()
        .filter_map(|(id, d)| {
            let points = match d {
                kernel::Dimension::Linear { .. } => {
                    let (a, b) = root.dimension_points(id)?;
                    vec![a, b]
                }
                kernel::Dimension::Radial { .. } => vec![rim_point(&root.radial_circle(id)?)],
            };
            Some((id, points))
        })
        .collect();
    if probes.is_empty() || hidden.is_empty() {
        return BTreeSet::new();
    }
    let mut nearest = Nearest::new(scene);
    probes
        .into_iter()
        .filter(|(_, points)| points.iter().any(|p| nearest.is_hidden(*p, hidden)))
        .map(|(id, _)| id)
        .collect()
}
