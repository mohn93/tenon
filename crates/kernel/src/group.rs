//! Making groups and components out of geometry.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};

use crate::annotation::Note;
use crate::edit::KernelError;
use crate::geom::LENGTH_TOL;
use crate::ids::{DefinitionId, EdgeId, FaceId, InstanceId, NoteId, VertexId};
use crate::model::{Guide, Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Parts, Scene, SceneChange};
use crate::transform::Stencil;

impl Scene {
    /// Moves edges, faces (with their edges) and instances of mesh `parent` into a new
    /// definition and places one instance of it, with an identity transform, where they were.
    /// Edges that still bound a face left behind stay in `parent` too. Returns the new
    /// instance.
    pub fn make_group(
        &mut self,
        parent: MeshId,
        edges: &[EdgeId],
        faces: &[FaceId],
        instances: &[InstanceId],
        kind: DefinitionKind,
        name: &str,
    ) -> Result<(InstanceId, SceneChange), KernelError> {
        let parts = Parts {
            edges: edges.to_vec(),
            faces: faces.to_vec(),
            instances: instances.to_vec(),
            ..Parts::default()
        };
        self.make_group_with(parent, &parts, kind, name)
    }

    /// `make_group` of `parts`' edges, faces and instances that also moves its guides,
    /// dimensions and notes into the new definition, keeping their ids. An annotation
    /// anchored to a vertex of the grouped geometry stays anchored to it there; one anchored
    /// to another vertex is anchored to where that vertex is, and a curve not grouped is
    /// measured as a fixed circle. Screen notes stay in the root.
    pub fn make_group_with(
        &mut self,
        parent: MeshId,
        parts: &Parts,
        kind: DefinitionKind,
        name: &str,
    ) -> Result<(InstanceId, SceneChange), KernelError> {
        let (edges, faces) = (&parts.edges[..], &parts.faces[..]);
        let source = self.mesh(parent).expect("group in a missing mesh");
        let stencil = source.stencil(edges, faces);
        let moved: Vec<(InstanceId, Instance)> = parts
            .instances
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| source.instance(*id).map(|i| (*id, i.clone())))
            .collect();
        let guides: Vec<(crate::ids::GuideId, Guide)> = parts
            .guides
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| source.guide(*id).map(|g| (*id, *g)))
            .collect();
        let notes: Vec<NoteId> = parts
            .notes
            .iter()
            .copied()
            .filter(|n| !matches!(source.note(*n), Some(Note::Screen { .. })))
            .collect();
        let copied = source.vertices_of(edges, faces);
        let annotations = source.lift(&parts.dimensions, &notes, DAffine3::IDENTITY, &copied);

        let (definition, added) = self.add_definition(name, kind);
        let mut placed = None;
        let left = self.edit(parent, |m| {
            for (id, _) in &moved {
                m.remove_instance(*id);
            }
            for (id, _) in &guides {
                m.remove_guide(*id);
            }
            // The annotations go before the geometry, so they are not frozen on the way out.
            m.erase_annotations_pending(&parts.dimensions, &notes);
            m.detach(edges, faces);
            placed = Some(m.insert_instance(Instance::new(definition, DAffine3::IDENTITY)));
            Ok(m.take_changes())
        })?;
        let filled = self.edit(MeshId::Definition(definition), |m| {
            m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
            for (id, i) in moved {
                m.insert_instance_as(id, i);
            }
            for (id, g) in guides {
                m.insert_guide_as(id, g);
            }
            m.land(annotations, true);
            Ok(m.take_changes())
        })?;
        let change = SceneChange::join([SceneChange::from(added), left, filled]);
        Ok((placed.expect("instance placed"), change))
    }
}

impl Scene {
    /// Replaces an instance in mesh `parent` by its definition's geometry, placed by the
    /// instance's transform and drawn like new edges, so it merges with what it touches.
    /// Instances, guides and annotations inside it come out into `parent`, with new ids (an
    /// annotation anchored to a vertex stays anchored to it); a group inside a
    /// component gets a copy of its definition. A group's definition goes with its last
    /// instance; a component's stays. The instance's material passes to the face sides and
    /// instances that come out without one, and its tag to the edges, faces, instances and
    /// guides that come out Untagged. Painted faces' texture placements take up the
    /// instance's transform, so their textures stay where they were.
    pub fn explode(
        &mut self,
        parent: MeshId,
        instance: InstanceId,
    ) -> Result<SceneChange, KernelError> {
        let placed = self
            .mesh(parent)
            .and_then(|m| m.instance(instance))
            .cloned()
            .ok_or(KernelError::InstanceNotFound)?;
        let definition = self
            .definition(placed.definition)
            .expect("instance of a missing definition");
        // A definition that outlives this explode still places its groups, so the groups
        // lifted out of it get copies of their definitions: a group is placed only once.
        let survives = definition.kind == DefinitionKind::Component
            || self.instance_count(placed.definition) > 1;
        let stencil = whole(&definition.mesh)
            .painted_over(placed.material)
            .tagged_over(placed.tag)
            .textures_carried(&placed.transform);
        let inner: Vec<Instance> = definition
            .mesh
            .instances()
            .map(|(_, i)| i.clone())
            .collect();
        let guides: Vec<Guide> = definition
            .mesh
            .guides()
            .map(|(_, g)| {
                g.transformed(placed.transform)
                    .with_tag(g.tag().or(placed.tag))
            })
            .collect();
        let (dimensions, notes) = definition.mesh.all_annotations();
        let annotations: Vec<_> = definition
            .mesh
            .lift(
                &dimensions,
                &notes,
                placed.transform,
                &definition.mesh.all_vertices(),
            )
            .into_iter()
            .map(|a| a.retagged(|t| t.or(placed.tag)))
            .collect();
        let mut copied = SceneChange::default();
        let mut children = Vec::new();
        for i in inner {
            let definition = match self.definition(i.definition).map(|d| d.kind) {
                Some(DefinitionKind::Group) if survives => {
                    let (d, made) = self.duplicate(i.definition);
                    copied = SceneChange::join([copied, made]);
                    d
                }
                _ => i.definition,
            };
            children.push(Instance {
                definition,
                transform: placed.transform * i.transform,
                material: i.material.or(placed.material),
                tag: i.tag.or(placed.tag),
            });
        }
        let exploded = self.edit(parent, |m| {
            m.remove_instance(instance);
            m.draw_stencil(&stencil, &[placed.transform]);
            for child in children {
                m.insert_instance(child);
            }
            for g in guides {
                m.insert_guide(g);
            }
            m.land(annotations, false);
            Ok(m.take_changes())
        })?;
        Ok(SceneChange::join([copied, exploded, self.prune()]))
    }

    /// Places `count` copies of instances of mesh `parent`, copy k shifted by `k * offset`.
    /// Copies of a component instance place the same definition; each copy of a group gets a
    /// copy of the group's definition (and of the groups inside it).
    pub fn copy_instances(
        &mut self,
        parent: MeshId,
        instances: &[InstanceId],
        offset: DVec3,
        count: u32,
    ) -> Result<SceneChange, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        let places: Vec<DAffine3> = (1..=count)
            .map(|k| DAffine3::from_translation(offset * k as f64))
            .collect();
        self.copy_instances_placed(parent, instances, &places)
    }

    /// Places copies of instances of mesh `parent`, one per rigid motion in `places` (applied
    /// after the original's transform), under the rules of `copy_instances`. A missing mesh is
    /// `DefinitionNotFound`.
    pub fn copy_instances_placed(
        &mut self,
        parent: MeshId,
        instances: &[InstanceId],
        places: &[DAffine3],
    ) -> Result<SceneChange, KernelError> {
        if !places.iter().all(|t| t.is_finite()) {
            return Err(KernelError::NotFinite);
        }
        let source = self.mesh(parent).ok_or(KernelError::DefinitionNotFound)?;
        let unique: BTreeSet<InstanceId> = instances.iter().copied().collect();
        let originals: Vec<Instance> = unique
            .iter()
            .map(|id| source.instance(*id).cloned())
            .collect::<Option<_>>()
            .ok_or(KernelError::InstanceNotFound)?;
        let mut steps = SceneChange::default();
        let mut copies = Vec::new();
        for place in places {
            for original in &originals {
                let definition = match self.definition(original.definition).map(|d| d.kind) {
                    Some(DefinitionKind::Group) => {
                        let (d, made) = self.duplicate(original.definition);
                        steps = SceneChange::join([steps, made]);
                        d
                    }
                    _ => original.definition,
                };
                copies.push(Instance {
                    definition,
                    transform: *place * original.transform,
                    material: original.material,
                    tag: original.tag,
                });
            }
        }
        let placed = self.edit(parent, |m| {
            for c in copies {
                m.insert_instance(c);
            }
            Ok(m.take_changes())
        })?;
        Ok(SceneChange::join([steps, placed]))
    }

    /// A new group definition holding a copy of `id`'s geometry, guides and annotations, with
    /// new ids; groups inside it are copied too, components inside are placed again.
    fn duplicate(&mut self, id: DefinitionId) -> (DefinitionId, SceneChange) {
        let original = self.definition(id).expect("copy of a missing definition");
        let (name, kind) = (original.name.clone(), original.kind);
        let stencil = whole(&original.mesh);
        let children: Vec<Instance> = original.mesh.instances().map(|(_, i)| i.clone()).collect();
        let guides: Vec<Guide> = original.mesh.guides().map(|(_, g)| *g).collect();
        let (dimensions, notes) = original.mesh.all_annotations();
        let annotations = original.mesh.lift(
            &dimensions,
            &notes,
            DAffine3::IDENTITY,
            &original.mesh.all_vertices(),
        );
        let mut change = SceneChange::default();
        let mut placed = Vec::new();
        for child in children {
            let definition = match self.definition(child.definition).map(|d| d.kind) {
                Some(DefinitionKind::Group) => {
                    let (d, made) = self.duplicate(child.definition);
                    change = SceneChange::join([change, made]);
                    d
                }
                _ => child.definition,
            };
            placed.push(Instance {
                definition,
                transform: child.transform,
                material: child.material,
                tag: child.tag,
            });
        }
        let (copy, added) = self.add_definition(&name, kind);
        let filled = self
            .edit(MeshId::Definition(copy), |m| {
                m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
                for i in placed {
                    m.insert_instance(i);
                }
                for g in guides {
                    m.insert_guide(g);
                }
                m.land(annotations, false);
                Ok(m.take_changes())
            })
            .expect("drawing a copy cannot fail");
        (copy, SceneChange::join([change, added.into(), filled]))
    }

    /// Removes instances from mesh `parent`. A group's definition goes with its last instance.
    /// A missing mesh changes nothing.
    pub fn erase_instances(&mut self, parent: MeshId, instances: &[InstanceId]) -> SceneChange {
        if self.mesh(parent).is_none() {
            return SceneChange::default();
        }
        let erased = self
            .edit(parent, |m| {
                for id in instances {
                    if m.instance(*id).is_some() {
                        m.remove_instance(*id);
                    }
                }
                Ok(m.take_changes())
            })
            .expect("erasing cannot fail");
        SceneChange::join([erased, self.prune()])
    }

    /// Removes group definitions that no instance places any more.
    fn prune(&mut self) -> SceneChange {
        let mut steps = Vec::new();
        loop {
            let unused = self
                .definitions()
                .find(|(id, d)| d.kind == DefinitionKind::Group && self.instance_count(*id) == 0)
                .map(|(id, _)| id);
            let Some(id) = unused else { break };
            steps.push(self.remove_definition(id));
        }
        SceneChange { steps }
    }
}

/// Every edge and face of a mesh, as a stencil.
pub(crate) fn whole(m: &Model) -> Stencil {
    let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
    let faces: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
    m.stencil(&edges, &faces)
}

impl Model {
    /// Removes faces and edges without healing anything: an edge given or bounding a removed
    /// face goes unless it still bounds a remaining face, and vertices left without edges go.
    pub(crate) fn detach(&mut self, edges: &[EdgeId], faces: &[FaceId]) {
        let mut sides: BTreeSet<EdgeId> = edges
            .iter()
            .copied()
            .filter(|e| self.edge(*e).is_some())
            .collect();
        for f in faces {
            let Some(face) = self.face(*f) else { continue };
            for l in std::iter::once(&face.outer).chain(&face.holes) {
                for i in 0..l.len() {
                    sides.extend(self.edge_between(l[i], l[(i + 1) % l.len()]));
                }
            }
        }
        for f in faces {
            if self.face(*f).is_some() {
                self.remove_face(*f);
            }
        }
        let mut ends: BTreeSet<VertexId> = BTreeSet::new();
        for e in sides {
            let [a, b] = self.edge(e).expect("side exists").v;
            let bounds_a_face = !self.faces_on_side(a, b).is_empty();
            if !bounds_a_face {
                self.remove_edge(e);
                ends.extend([a, b]);
            }
        }
        for v in ends {
            if self.edges_at(v).is_empty() {
                self.remove_vertex(v);
            }
        }
    }
}
