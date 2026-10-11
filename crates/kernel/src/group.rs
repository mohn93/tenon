//! Making groups and components out of geometry.

use std::collections::BTreeSet;

use glam::DAffine3;

use crate::edit::{KernelError, loop_has_side};
use crate::ids::{EdgeId, FaceId, InstanceId, VertexId};
use crate::model::{Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};

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
        let source = self.mesh(parent).expect("group in a missing mesh");
        let stencil = source.stencil(edges, faces);
        let moved: Vec<(InstanceId, Instance)> = instances
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| source.instance(*id).map(|i| (*id, i.clone())))
            .collect();

        let (definition, added) = self.add_definition(name, kind);
        let mut placed = None;
        let left = self.edit(parent, |m| {
            for (id, _) in &moved {
                m.remove_instance(*id);
            }
            m.detach(edges, faces);
            placed = Some(m.insert_instance(Instance {
                definition,
                transform: DAffine3::IDENTITY,
            }));
            Ok(m.take_changes())
        })?;
        let filled = self.edit(MeshId::Definition(definition), |m| {
            m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
            for (id, i) in moved {
                m.insert_instance_as(id, i);
            }
            Ok(m.take_changes())
        })?;
        let change = SceneChange::join([SceneChange::from(added), left, filled]);
        Ok((placed.expect("instance placed"), change))
    }
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
            let bounds_a_face = self.faces().any(|(_, f)| {
                std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .any(|l| loop_has_side(l, a, b))
            });
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
