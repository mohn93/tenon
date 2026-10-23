//! Intersect Faces: edges where faces cross other faces.

use std::collections::{BTreeSet, HashMap, HashSet};

use glam::{DAffine3, DVec3};
use kernel::geom::LENGTH_TOL;
use kernel::intersect::{FaceShape, face_segments, face_shape};
use kernel::{EntityId, FaceId, InstanceId, MAX_IMPORT_DEPTH, MeshId, Scene};

use super::Document;
use crate::clip::MAX_CLIP_INSTANCES;
use crate::command::{CommandError, ErrorKind, IntersectReport, IntersectScope};

/// Most face pairs tested after the broad phase.
const MAX_PAIRS: usize = 200_000;
/// Most segments drawn at once.
const MAX_SEGMENTS: usize = 100_000;

const TOO_MANY: &str = "Too many faces cross to intersect at once";

/// What bounds one run.
#[derive(Clone, Copy)]
struct Limits {
    pairs: usize,
    segments: usize,
}

/// A mesh as placed in the root.
struct Placement {
    path: Vec<InstanceId>,
    mesh: MeshId,
    place: DAffine3,
    /// Root coordinates to the mesh's own; None for a placement that flattens space.
    back: Option<DAffine3>,
}

/// A face as placed: the index of its placement and its id in the mesh.
type Key = (usize, FaceId);

impl Document {
    /// Draws, in the open context, the edges where the faces in `ids` (the selection when
    /// empty) cross the faces `with` names.
    pub(super) fn intersect_faces_changes(
        &mut self,
        ids: &[EntityId],
        with: IntersectScope,
    ) -> Result<kernel::SceneChange, CommandError> {
        self.intersect_with_limits(
            ids,
            with,
            Limits {
                pairs: MAX_PAIRS,
                segments: MAX_SEGMENTS,
            },
        )
    }

    fn intersect_with_limits(
        &mut self,
        ids: &[EntityId],
        with: IntersectScope,
        limits: Limits,
    ) -> Result<kernel::SceneChange, CommandError> {
        let nothing = || {
            CommandError::new(
                ErrorKind::InvalidInput,
                "Select the faces or groups to intersect",
            )
        };
        let chosen: Vec<EntityId> = if ids.is_empty() {
            self.selection.iter().copied().collect()
        } else {
            self.parts(ids)?;
            ids.to_vec()
        };
        let parts = kernel::Parts::of(&chosen);
        if parts.faces.is_empty() && parts.instances.is_empty() {
            return Err(nothing());
        }

        let scene = &self.scene;
        let too_many = || CommandError::new(ErrorKind::InvalidInput, TOO_MANY);
        let places = placements(scene, &self.context).ok_or_else(too_many)?;
        let context_path = &self.context;
        let context = places
            .iter()
            .position(|p| p.path == *context_path)
            .expect("the open context is placed");
        let Some(to_local) = places[context].back else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the open group is flat and cannot be intersected inside",
            ));
        };

        // The faces being intersected.
        let mut sources: BTreeSet<Key> = BTreeSet::new();
        let context_mesh = places[context].mesh;
        if let Some(model) = scene.mesh(context_mesh) {
            for f in &parts.faces {
                if model.face(*f).is_some_and(|f| scene.tag_shown(f.tag)) {
                    sources.insert((context, *f));
                }
            }
        }
        for instance in &parts.instances {
            let mut prefix = context_path.clone();
            prefix.push(*instance);
            for (index, p) in places.iter().enumerate() {
                if p.path.starts_with(&prefix)
                    && let Some(model) = scene.mesh(p.mesh)
                {
                    sources.extend(
                        model
                            .faces()
                            .filter(|(_, f)| scene.tag_shown(f.tag))
                            .map(|(id, _)| (index, id)),
                    );
                }
            }
        }

        let source_places: BTreeSet<usize> = sources.iter().map(|k| k.0).collect();
        let targets: Vec<usize> = match with {
            IntersectScope::Model => (0..places.len()).collect(),
            IntersectScope::Selection => source_places.iter().copied().collect(),
            IntersectScope::Context => vec![context],
        };
        let only_sources = with == IntersectScope::Selection;

        let mut shapes: HashMap<Key, Option<FaceShape>> = HashMap::new();
        let mut shape_of = |key: Key| -> Option<FaceShape> {
            shapes
                .entry(key)
                .or_insert_with(|| {
                    let p = &places[key.0];
                    scene
                        .mesh(p.mesh)
                        .and_then(|m| face_shape(m, key.1, p.place))
                })
                .clone()
        };

        // Broad phase: pairs whose boxes meet, each once.
        let mut pairs: Vec<(Key, Key)> = Vec::new();
        let mut seen: HashSet<(Key, Key)> = HashSet::new();
        for &a in &sources {
            let Some(shape) = shape_of(a) else { continue };
            let (lo, hi) = bounds(shape.outer.iter().copied());
            for &t in &targets {
                let target = &places[t];
                let Some(back) = target.back else { continue };
                let Some(model) = scene.mesh(target.mesh) else {
                    continue;
                };
                let (blo, bhi) = box_through(back, lo, hi);
                let pad = DVec3::splat(LENGTH_TOL);
                for f in model.faces_near_box(blo - pad, bhi + pad) {
                    let b = (t, f);
                    if a == b || (only_sources && !sources.contains(&b)) {
                        continue;
                    }
                    if !model.face(f).is_some_and(|f| scene.tag_shown(f.tag)) {
                        continue;
                    }
                    let pair = if a < b { (a, b) } else { (b, a) };
                    if seen.insert(pair) {
                        if pairs.len() >= limits.pairs {
                            return Err(too_many());
                        }
                        pairs.push(pair);
                    }
                }
            }
        }

        // Narrow phase, in world coordinates.
        let mut world: Vec<(DVec3, DVec3)> = Vec::new();
        for (a, b) in pairs {
            let (Some(sa), Some(sb)) = (shape_of(a), shape_of(b)) else {
                continue;
            };
            world.extend(face_segments(&sa, &sb));
            if world.len() > limits.segments {
                return Err(too_many());
            }
        }
        let local: Vec<(DVec3, DVec3)> = world
            .iter()
            .map(|(p, q)| (to_local.transform_point3(*p), to_local.transform_point3(*q)))
            .collect();

        let change = self
            .edit_mesh(|m| m.add_segments(&local))
            .map_err(|e| self.kernel_error(e, &chosen))?;
        let summary = change.summary();
        let pick = |list: &[EntityId], faces: bool| -> Vec<EntityId> {
            list.iter()
                .copied()
                .filter(|id| matches!(id, EntityId::Face(_)) == faces)
                .filter(|id| matches!(id, EntityId::Face(_) | EntityId::Edge(_)))
                .collect()
        };
        self.intersect_report = Some(IntersectReport {
            added_edges: pick(&summary.created, false),
            split_faces: pick(&summary.deleted, true),
            created_faces: pick(&summary.created, true),
        });
        Ok(change)
    }
}

/// Every placement of every mesh shown from the root, parents first, and the path to the open
/// context (shown or not). None when there are too many.
fn placements(scene: &Scene, context: &[InstanceId]) -> Option<Vec<Placement>> {
    fn walk(
        scene: &Scene,
        context: &[InstanceId],
        path: Vec<InstanceId>,
        mesh: MeshId,
        place: DAffine3,
        out: &mut Vec<Placement>,
    ) -> Option<()> {
        if out.len() as u64 >= MAX_CLIP_INSTANCES || path.len() > MAX_IMPORT_DEPTH {
            return None;
        }
        let model = scene.mesh(mesh)?;
        let back = (place.matrix3.determinant().abs() > 1e-18).then(|| place.inverse());
        out.push(Placement {
            path: path.clone(),
            mesh,
            place,
            back,
        });
        for (id, instance) in model.instances() {
            let mut next = path.clone();
            next.push(id);
            if scene.instance_shown(mesh, id, instance) || context.starts_with(&next) {
                walk(
                    scene,
                    context,
                    next,
                    MeshId::Definition(instance.definition),
                    place * instance.transform,
                    out,
                )?;
            }
        }
        Some(())
    }
    let mut out = Vec::new();
    walk(
        scene,
        context,
        Vec::new(),
        MeshId::Root,
        DAffine3::IDENTITY,
        &mut out,
    )?;
    Some(out)
}

fn bounds(points: impl Iterator<Item = DVec3>) -> (DVec3, DVec3) {
    points.fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), p| {
        (lo.min(p), hi.max(p))
    })
}

/// The box around the corners of `lo`..`hi` moved by `map`.
fn box_through(map: DAffine3, lo: DVec3, hi: DVec3) -> (DVec3, DVec3) {
    bounds((0..8).map(|i| {
        let pick = |bit: usize, a: f64, b: f64| if i & bit == 0 { a } else { b };
        map.transform_point3(DVec3::new(
            pick(1, lo.x, hi.x),
            pick(2, lo.y, hi.y),
            pick(4, lo.z, hi.z),
        ))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use glam::DVec2;

    fn run(d: &mut Document, v: serde_json::Value) -> crate::command::Outcome {
        d.execute(serde_json::from_value::<Command>(v).unwrap())
            .unwrap()
    }

    /// Two crossing boxes: 6 faces to select, a hexagon of crossings.
    fn boxes() -> (Document, Vec<EntityId>) {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        for (lo, hi) in [(0.0, 2.0), (1.0, 3.0)] {
            let out = run(
                &mut d,
                serde_json::json!({"type": "add_rectangle", "corner": [lo, lo, lo],
                    "opposite": [hi, hi, lo]}),
            );
            let base = out
                .changes
                .created
                .iter()
                .find_map(|id| match id {
                    EntityId::Face(f) => Some(*f),
                    _ => None,
                })
                .unwrap();
            let up = d.model().face(base).unwrap().normal.z > 0.0;
            run(
                &mut d,
                serde_json::json!({"type": "push_pull", "face": EntityId::Face(base),
                    "distance": if up { 2.0 } else { -2.0 }}),
            );
        }
        let faces = d.model().faces().map(|(f, _)| EntityId::Face(f)).collect();
        (d, faces)
    }

    #[test]
    fn the_segment_limit_refuses_and_changes_nothing() {
        let (mut d, faces) = boxes();
        let revision = d.revision();
        let limits = Limits {
            pairs: MAX_PAIRS,
            segments: 3,
        };
        let e = d
            .intersect_with_limits(&faces, IntersectScope::Selection, limits)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        assert_eq!(e.message, TOO_MANY);
        assert_eq!(d.revision(), revision);
        let ok = Limits {
            pairs: MAX_PAIRS,
            segments: MAX_SEGMENTS,
        };
        assert!(
            d.intersect_with_limits(&faces, IntersectScope::Selection, ok)
                .is_ok()
        );
    }
}
