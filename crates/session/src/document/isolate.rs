//! The notes a viewer's Isolate hides with the parts: a root leader note points at whatever
//! its anchor lies nearest, and goes when that is a hidden part.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use glam::{DAffine3, DVec3};
use kernel::bvh::closest_on_triangle;
use kernel::{Anchor, InstanceId, MeshId, Note, NoteId, Scene};

/// What a mesh shows: its own shown faces' triangles in its coordinates, and the box around
/// them and its shown instances.
struct Shown {
    triangles: Vec<[DVec3; 3]>,
    bounds: Option<[DVec3; 2]>,
}

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

/// What the meshes asked for show, each worked out once.
#[derive(Default)]
struct Meshes(HashMap<MeshId, Rc<Shown>>);

impl Meshes {
    /// What `mesh` shows. A definition reached again through itself adds nothing.
    fn shown(&mut self, scene: &Scene, mesh: MeshId) -> Rc<Shown> {
        if let Some(s) = self.0.get(&mesh) {
            return s.clone();
        }
        // Empty while it is worked out, so a mesh that places itself ends.
        self.0.insert(
            mesh,
            Rc::new(Shown {
                triangles: Vec::new(),
                bounds: None,
            }),
        );
        let mut triangles = Vec::new();
        let mut boxes = Vec::new();
        if let Some(m) = scene.mesh(mesh) {
            for (id, face) in m.faces() {
                if scene.face_shown(face) {
                    triangles.extend(m.triangulate(id).unwrap_or_default());
                }
            }
            for (_, instance) in m.instances() {
                if scene.placed_instance_shown(instance)
                    && let Some(b) = self
                        .shown(scene, MeshId::Definition(instance.definition))
                        .bounds
                {
                    boxes.push(placed(instance.transform, b));
                }
            }
        }
        let bounds = enclose(
            triangles
                .iter()
                .flatten()
                .copied()
                .chain(boxes.into_iter().flatten()),
        );
        let shown = Rc::new(Shown { triangles, bounds });
        self.0.insert(mesh, shown.clone());
        shown
    }

    /// Lowers `best` to how far `p` (root coordinates) is from what `mesh`, placed by `place`
    /// (into root coordinates), shows when that is nearer. True if it was.
    fn nearer(
        &mut self,
        scene: &Scene,
        mesh: MeshId,
        place: DAffine3,
        p: DVec3,
        best: &mut f64,
    ) -> bool {
        let shown = self.shown(scene, mesh);
        match shown.bounds {
            Some(b) if box_distance(p, placed(place, b)) < *best => {}
            _ => return false,
        }
        let mut found = false;
        let local = place.inverse().transform_point3(p);
        for t in &shown.triangles {
            let d = place
                .transform_point3(closest_on_triangle(local, *t))
                .distance(p);
            if d < *best {
                *best = d;
                found = true;
            }
        }
        if let Some(m) = scene.mesh(mesh) {
            for (_, instance) in m.instances() {
                if scene.placed_instance_shown(instance) {
                    let inner = MeshId::Definition(instance.definition);
                    found |= self.nearer(scene, inner, place * instance.transform, p, best);
                }
            }
        }
        found
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
    let mut out = BTreeSet::new();
    if anchored.is_empty() || hidden.is_empty() {
        return out;
    }
    let mut meshes = Meshes::default();
    let own: Vec<[DVec3; 3]> = root
        .faces()
        .filter(|(_, f)| scene.face_shown(f))
        .flat_map(|(id, _)| root.triangulate(id).unwrap_or_default())
        .collect();
    let instances: Vec<(InstanceId, DAffine3, MeshId, [DVec3; 2])> = root
        .instances()
        .filter(|(_, i)| scene.placed_instance_shown(i))
        .filter_map(|(id, i)| {
            let mesh = MeshId::Definition(i.definition);
            let b = meshes.shown(scene, mesh).bounds?;
            Some((id, i.transform, mesh, placed(i.transform, b)))
        })
        .collect();
    for (note, p) in anchored {
        let mut best = own
            .iter()
            .map(|t| closest_on_triangle(p, *t).distance(p))
            .fold(f64::INFINITY, f64::min);
        let mut nearest = None;
        // Nearest boxes first, so most parts are passed over by their box.
        let mut order: Vec<(f64, usize)> = instances
            .iter()
            .enumerate()
            .map(|(k, (.., b))| (box_distance(p, *b), k))
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (near, k) in order {
            if near >= best {
                break;
            }
            let (id, place, mesh, _) = instances[k];
            if meshes.nearer(scene, mesh, place, p, &mut best) {
                nearest = Some(id);
            }
        }
        if nearest.is_some_and(|id| hidden.contains(&id)) {
            out.insert(note);
        }
    }
    out
}
