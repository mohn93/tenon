//! Clips: a selection taken out of a scene as a standalone scene (copy), and a clip put into a
//! scene (paste).

use std::collections::{BTreeMap, BTreeSet};

use glam::DAffine3;

use crate::edit::KernelError;
use crate::group::whole;
use crate::ids::{DefinitionId, EdgeId, FaceId, GuideId, InstanceId, MaterialId, TagId};
use crate::model::{Guide, Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};

/// What an insert made in the mesh it inserted into (its top level only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inserted {
    /// The edges along the clip's edges, in id order (existing edges the clip's edges lie on
    /// included).
    pub edges: Vec<EdgeId>,
    /// The faces drawn as the clip's (pieces of them included, and faces they were drawn
    /// exactly onto), in id order.
    pub faces: Vec<FaceId>,
    pub instances: Vec<InstanceId>,
    pub guides: Vec<GuideId>,
}

impl Scene {
    /// A new standalone scene whose root holds the selected `edges`, `faces` (with their
    /// edges, as Move's copy takes them), `instances` and `guides` of mesh `mesh`, taken into
    /// world coordinates by `to_world`, with every definition the instances reach (under the
    /// same ids) and only the materials and tags the clip uses (under the same ids). Painted
    /// faces' texture placements take up `to_world`, so their textures stay where they were.
    /// Missing edges, faces and guides are skipped; a missing instance is `InstanceNotFound`,
    /// a missing mesh `DefinitionNotFound`.
    pub fn extract(
        &self,
        mesh: MeshId,
        edges: &[EdgeId],
        faces: &[FaceId],
        instances: &[InstanceId],
        guides: &[GuideId],
        to_world: DAffine3,
    ) -> Result<Scene, KernelError> {
        if !to_world.is_finite() {
            return Err(KernelError::NotFinite);
        }
        let source = self.mesh(mesh).ok_or(KernelError::DefinitionNotFound)?;
        let placed: Vec<Instance> = instances
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|id| source.instance(*id).cloned())
            .collect::<Option<_>>()
            .ok_or(KernelError::InstanceNotFound)?;
        let lines: Vec<Guide> = guides
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| source.guide(*id).copied())
            .collect();
        let stencil = source.stencil(edges, faces).textures_carried(&to_world);

        let mut reached = BTreeMap::new();
        let mut stack: Vec<DefinitionId> = placed.iter().map(|i| i.definition).collect();
        while let Some(id) = stack.pop() {
            if reached.contains_key(&id) {
                continue;
            }
            let d = self
                .definition(id)
                .expect("instance of a missing definition");
            stack.extend(d.mesh.instances().map(|(_, i)| i.definition));
            reached.insert(id, d.clone());
        }

        let mut clip = Scene::from_meshes(
            Model::new(),
            reached,
            self.materials.clone(),
            self.tags.clone(),
            Vec::new(),
            self.next_id(),
        );
        clip.edit(MeshId::Root, |m| {
            m.draw_stencil(&stencil, &[to_world]);
            for i in placed {
                m.insert_instance(Instance {
                    transform: to_world * i.transform,
                    ..i
                });
            }
            for g in lines {
                m.insert_guide(g.transformed(to_world));
            }
            Ok(m.take_changes())
        })?;

        let (used_materials, used_tags) = clip.used();
        clip.materials.retain(|id, _| used_materials.contains(id));
        clip.tags.retain(|id, _| used_tags.contains(id));
        Ok(clip)
    }

    /// Puts `clip`'s root contents into mesh `parent`, placed by `placement`, as one change.
    /// Edges and faces are drawn like Move's copies, so they merge with geometry they touch;
    /// painted faces' texture placements take up `placement`. Materials and tags are matched
    /// by name (an existing one is used as it is; a missing one is added). A component
    /// definition is the one `reuse` maps it to when that is an existing component that does
    /// not hold `parent`, else a new copy; a group always gets a new copy. On error nothing
    /// is changed.
    pub fn insert(
        &mut self,
        parent: MeshId,
        clip: &Scene,
        placement: DAffine3,
        reuse: &dyn Fn(DefinitionId) -> Option<DefinitionId>,
    ) -> Result<(Inserted, SceneChange), KernelError> {
        if !placement.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if self.mesh(parent).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        let mut paste = Paste {
            clip,
            parent,
            reuse,
            materials: BTreeMap::new(),
            tags: BTreeMap::new(),
            definitions: BTreeMap::new(),
            open: BTreeSet::new(),
            done: Vec::new(),
        };
        match paste.run(self, placement) {
            Ok(inserted) => Ok((inserted, SceneChange::join(paste.done))),
            Err(e) => {
                self.apply(&SceneChange::join(paste.done).inverse());
                Err(e)
            }
        }
    }

    /// The materials and tags anything in any mesh uses.
    fn used(&self) -> (BTreeSet<MaterialId>, BTreeSet<TagId>) {
        let mut materials = BTreeSet::new();
        let mut tags = BTreeSet::new();
        for (_, m) in self.meshes() {
            for (_, f) in m.faces() {
                materials.extend(f.paint().into_iter().flatten());
                tags.extend(f.tag);
            }
            for (_, e) in m.edges() {
                tags.extend(e.tag);
            }
            for (_, i) in m.instances() {
                materials.extend(i.material);
                tags.extend(i.tag);
            }
            for (_, g) in m.guides() {
                tags.extend(g.tag());
            }
        }
        (materials, tags)
    }

    /// True when definition `from` is `to` or places it (directly or deeper).
    fn reaches(&self, from: DefinitionId, to: DefinitionId) -> bool {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(d) = stack.pop() {
            if d == to {
                return true;
            }
            if !seen.insert(d) {
                continue;
            }
            if let Some(def) = self.definition(d) {
                stack.extend(def.mesh.instances().map(|(_, i)| i.definition));
            }
        }
        false
    }
}

/// One insert in progress: how the clip's materials, tags and definitions map to the target's,
/// and the changes made so far.
struct Paste<'a> {
    clip: &'a Scene,
    parent: MeshId,
    reuse: &'a dyn Fn(DefinitionId) -> Option<DefinitionId>,
    materials: BTreeMap<MaterialId, MaterialId>,
    tags: BTreeMap<TagId, TagId>,
    definitions: BTreeMap<DefinitionId, DefinitionId>,
    /// Clip definitions being copied, to catch one that places itself.
    open: BTreeSet<DefinitionId>,
    done: Vec<SceneChange>,
}

impl Paste<'_> {
    fn run(&mut self, scene: &mut Scene, placement: DAffine3) -> Result<Inserted, KernelError> {
        for (id, m) in self.clip.materials() {
            let existing = scene.materials().find(|(_, t)| t.name == m.name);
            let to = match existing.map(|(to, _)| to) {
                Some(to) => to,
                None => {
                    let (to, change) = scene.add_material_from(m.clone())?;
                    self.done.push(change);
                    to
                }
            };
            self.materials.insert(id, to);
        }
        for (id, t) in self.clip.tags() {
            let existing = scene.tags().find(|(_, s)| s.name == t.name);
            let to = match existing.map(|(to, _)| to) {
                Some(to) => to,
                None => {
                    let (to, change) = scene.add_tag(t.clone())?;
                    self.done.push(change);
                    to
                }
            };
            self.tags.insert(id, to);
        }

        let root = self.clip.root();
        let stencil = self.stencil_of(root).textures_carried(&placement);
        let children = self.instances_of(scene, root, placement)?;
        let guides = self.guides_of(root, placement);
        let mut inserted = Inserted::default();
        let change = scene.edit(self.parent, |m| {
            let before: BTreeSet<FaceId> = m.faces().map(|(id, _)| id).collect();
            let mut faces = m.draw_stencil(&stencil, &[placement]);
            let mut edges = BTreeSet::new();
            for (a, b) in stencil.segments() {
                let (a, b) = (
                    placement.transform_point3(*a),
                    placement.transform_point3(*b),
                );
                edges.extend(m.edges_along(a, b));
            }
            // A face that was there already and that the clip's edges wholly bound is one the
            // clip's face was drawn onto.
            for e in &edges {
                let v = m.edge(*e).expect("edge along the clip").v[0];
                for f in m.faces_at(v).filter(|f| before.contains(f)) {
                    let outer = &m.face(f).expect("face at a vertex").outer;
                    let n = outer.len();
                    let bounded = (0..n).all(|i| {
                        m.edge_between(outer[i], outer[(i + 1) % n])
                            .is_some_and(|s| edges.contains(&s))
                    });
                    if bounded {
                        faces.insert(f);
                    }
                }
            }
            inserted.edges = edges.into_iter().collect();
            inserted.faces = faces.into_iter().collect();
            inserted.instances = children.into_iter().map(|i| m.insert_instance(i)).collect();
            inserted.guides = guides.into_iter().map(|g| m.insert_guide(g)).collect();
            Ok(m.take_changes())
        })?;
        self.done.push(change);
        Ok(inserted)
    }

    /// The target definition clip definition `id` maps to, copying it (and what it places)
    /// unless it is a component `reuse` maps to an existing component that does not hold the
    /// mesh being inserted into.
    fn definition(
        &mut self,
        scene: &mut Scene,
        id: DefinitionId,
    ) -> Result<DefinitionId, KernelError> {
        if let Some(to) = self.definitions.get(&id) {
            return Ok(*to);
        }
        let clip = self.clip;
        let original = clip.definition(id).ok_or(KernelError::DefinitionNotFound)?;
        if original.kind == DefinitionKind::Component
            && let Some(to) = (self.reuse)(id)
            && scene
                .definition(to)
                .is_some_and(|d| d.kind == DefinitionKind::Component)
            && !matches!(self.parent, MeshId::Definition(p) if scene.reaches(to, p))
        {
            self.definitions.insert(id, to);
            return Ok(to);
        }
        if !self.open.insert(id) {
            return Err(KernelError::InvalidImport(format!(
                "definition {id} of the clip places itself"
            )));
        }
        let stencil = self.stencil_of(&original.mesh);
        let children = self.instances_of(scene, &original.mesh, DAffine3::IDENTITY)?;
        let guides = self.guides_of(&original.mesh, DAffine3::IDENTITY);
        let (copy, added) = scene.add_definition(&original.name, original.kind);
        self.done.push(added.into());
        let filled = scene.edit(MeshId::Definition(copy), |m| {
            m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
            for i in children {
                m.insert_instance(i);
            }
            for g in guides {
                m.insert_guide(g);
            }
            Ok(m.take_changes())
        })?;
        self.done.push(filled);
        self.open.remove(&id);
        self.definitions.insert(id, copy);
        Ok(copy)
    }

    /// Every edge and face of a clip mesh, with the target's materials and tags.
    fn stencil_of(&self, m: &Model) -> crate::transform::Stencil {
        whole(m).remapped(
            |id| self.materials.get(&id).copied(),
            |id| self.tags.get(&id).copied(),
        )
    }

    /// A clip mesh's instances placed by `placement`, with the target's definitions,
    /// materials and tags.
    fn instances_of(
        &mut self,
        scene: &mut Scene,
        m: &Model,
        placement: DAffine3,
    ) -> Result<Vec<Instance>, KernelError> {
        let mut out = Vec::new();
        for (_, i) in m.instances() {
            out.push(Instance {
                definition: self.definition(scene, i.definition)?,
                transform: placement * i.transform,
                material: i.material.and_then(|id| self.materials.get(&id).copied()),
                tag: i.tag.and_then(|id| self.tags.get(&id).copied()),
            });
        }
        Ok(out)
    }

    /// A clip mesh's guides placed by `placement`, on the target's tags.
    fn guides_of(&self, m: &Model, placement: DAffine3) -> Vec<Guide> {
        m.guides()
            .map(|(_, g)| {
                g.transformed(placement)
                    .with_tag(g.tag().and_then(|id| self.tags.get(&id).copied()))
            })
            .collect()
    }
}
