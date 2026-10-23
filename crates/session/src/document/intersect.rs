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
/// Most face box queries of the broad phase.
const MAX_QUERIES: usize = 4_000_000;

const TOO_MANY: &str = "Too many faces cross to intersect at once";
const TOO_MANY_PLACED: &str = "Too many groups and components to intersect at once";
const TOO_DEEP: &str = "Groups and components are nested too deeply to intersect";

/// What bounds one run.
#[derive(Clone, Copy)]
struct Limits {
    pairs: usize,
    segments: usize,
    /// Most bounding box queries of the broad phase, whether or not they find anything.
    queries: usize,
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
                queries: MAX_QUERIES,
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
        let selected: BTreeSet<InstanceId> = parts.instances.iter().copied().collect();
        let places = placements(scene, &self.context, with, &selected)?;
        let context_path = &self.context;
        let context = places
            .iter()
            .position(|p| p.path == *context_path)
            .expect("the open context is placed");
        let Some(to_local) = places[context].back else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "The open group is flat and cannot be intersected inside",
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

        // The selected instance each placement lies in, so a group is never crossed with itself.
        let mut owner: Vec<Option<InstanceId>> = vec![None; places.len()];
        for instance in &selected {
            let mut prefix = context_path.clone();
            prefix.push(*instance);
            for (index, p) in places.iter().enumerate() {
                if p.path.starts_with(&prefix) {
                    owner[index] = Some(*instance);
                }
            }
        }

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
        let source_boxes: Vec<(Key, (DVec3, DVec3))> = sources
            .iter()
            .filter_map(|&k| {
                let shape = shape_of(k)?;
                let b = bounds(shape.outer.iter().copied());
                Some((k, b))
            })
            .collect();
        let pad = DVec3::splat(LENGTH_TOL);
        let all = source_boxes.iter().fold(
            (DVec3::INFINITY, DVec3::NEG_INFINITY),
            |(lo, hi), (_, (l, h))| (lo.min(*l), hi.max(*h)),
        );
        let (all_lo, all_hi) = (all.0 - pad, all.1 + pad);

        // The placements whose box meets the sources' box, with that box in the root.
        let mut mesh_boxes: HashMap<MeshId, Option<(DVec3, DVec3)>> = HashMap::new();
        let source_places: BTreeSet<usize> = sources.iter().map(|k| k.0).collect();
        let candidates: Vec<usize> = match with {
            IntersectScope::Model => (0..places.len()).collect(),
            IntersectScope::Selection => source_places.iter().copied().collect(),
            IntersectScope::Context => vec![context],
        };
        let mut targets: Vec<(usize, (DVec3, DVec3))> = Vec::new();
        for t in candidates {
            let p = &places[t];
            let local = *mesh_boxes.entry(p.mesh).or_insert_with(|| {
                let model = scene.mesh(p.mesh)?;
                model.faces().next()?;
                let b = bounds(model.vertices().map(|(_, v)| v.pos));
                b.0.is_finite().then_some(b)
            });
            let Some((lo, hi)) = local else { continue };
            let (wlo, whi) = box_through(p.place, lo, hi);
            if (wlo - pad).cmple(all_hi).all() && (whi + pad).cmpge(all_lo).all() {
                targets.push((t, (wlo - pad, whi + pad)));
            }
        }
        let only_sources = with == IntersectScope::Selection;

        // Broad phase: pairs whose boxes meet, each once.
        let mut pairs: Vec<(Key, Key)> = Vec::new();
        let mut seen: HashSet<(Key, Key)> = HashSet::new();
        let mut queries = 0usize;
        for (a, (lo, hi)) in &source_boxes {
            let a = *a;
            for &(t, (wlo, whi)) in &targets {
                if !(hi.cmpge(wlo).all() && lo.cmple(whi).all()) {
                    continue;
                }
                queries += 1;
                if queries > limits.queries {
                    return Err(too_many());
                }
                let target = &places[t];
                let Some(back) = target.back else { continue };
                let Some(model) = scene.mesh(target.mesh) else {
                    continue;
                };
                let (blo, bhi) = box_through(back, *lo, *hi);
                for f in model.faces_near_box(blo - pad, bhi + pad) {
                    let b = (t, f);
                    if a == b || (only_sources && !sources.contains(&b)) {
                        continue;
                    }
                    // Faces of one placement (other than the open context) or of one selected
                    // instance never cross each other.
                    if (a.0 == t && t != context)
                        || (owner[a.0].is_some() && owner[a.0] == owner[t])
                    {
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

/// The placements `with` needs, parents first: for `model` every mesh shown from the root; for
/// the others the open context and what the selected instances `selected` place. The open
/// context is always there, shown or not.
fn placements(
    scene: &Scene,
    context: &[InstanceId],
    with: IntersectScope,
    selected: &BTreeSet<InstanceId>,
) -> Result<Vec<Placement>, CommandError> {
    struct Walk<'a> {
        scene: &'a Scene,
        context: &'a [InstanceId],
        out: Vec<Placement>,
    }
    impl Walk<'_> {
        fn go(
            &mut self,
            path: Vec<InstanceId>,
            mesh: MeshId,
            place: DAffine3,
            only: Option<&BTreeSet<InstanceId>>,
        ) -> Result<(), CommandError> {
            if self.out.len() as u64 >= MAX_CLIP_INSTANCES {
                return Err(CommandError::new(ErrorKind::InvalidInput, TOO_MANY_PLACED));
            }
            if path.len() > MAX_IMPORT_DEPTH {
                return Err(CommandError::new(ErrorKind::InvalidInput, TOO_DEEP));
            }
            let Some(model) = self.scene.mesh(mesh) else {
                return Ok(());
            };
            let back = (place.matrix3.determinant().abs() > 1e-18).then(|| place.inverse());
            self.out.push(Placement {
                path: path.clone(),
                mesh,
                place,
                back,
            });
            for (id, instance) in model.instances() {
                if only.is_some_and(|o| !o.contains(&id)) {
                    continue;
                }
                let mut next = path.clone();
                next.push(id);
                if self.scene.instance_shown(mesh, id, instance) || self.context.starts_with(&next)
                {
                    self.go(
                        next,
                        MeshId::Definition(instance.definition),
                        place * instance.transform,
                        None,
                    )?;
                }
            }
            Ok(())
        }
    }
    let mut walk = Walk {
        scene,
        context,
        out: Vec::new(),
    };
    if with == IntersectScope::Model {
        walk.go(Vec::new(), MeshId::Root, DAffine3::IDENTITY, None)?;
    } else {
        let (mesh, place) = scene.resolve(context).expect("the open context exists");
        walk.go(context.to_vec(), mesh, place, Some(selected))?;
    }
    Ok(walk.out)
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
            queries: MAX_QUERIES,
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
            queries: MAX_QUERIES,
        };
        assert!(
            d.intersect_with_limits(&faces, IntersectScope::Selection, ok)
                .is_ok()
        );
    }

    #[test]
    fn the_pair_limit_refuses_and_changes_nothing() {
        let (mut d, faces) = boxes();
        let revision = d.revision();
        let limits = Limits {
            pairs: 10,
            segments: MAX_SEGMENTS,
            queries: MAX_QUERIES,
        };
        let e = d
            .intersect_with_limits(&faces, IntersectScope::Selection, limits)
            .unwrap_err();
        assert_eq!(e.message, TOO_MANY);
        assert_eq!(d.revision(), revision);
        let ok = Limits {
            pairs: MAX_PAIRS,
            ..limits
        };
        assert!(
            d.intersect_with_limits(&faces, IntersectScope::Selection, ok)
                .is_ok()
        );
    }

    #[test]
    fn the_query_limit_refuses() {
        let (mut d, faces) = boxes();
        let limits = Limits {
            pairs: MAX_PAIRS,
            segments: MAX_SEGMENTS,
            queries: 2,
        };
        let e = d
            .intersect_with_limits(&faces, IntersectScope::Selection, limits)
            .unwrap_err();
        assert_eq!(e.message, TOO_MANY);
    }
}
