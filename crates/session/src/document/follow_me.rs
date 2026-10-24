//! Follow Me: a face swept along a chain of edges.

use std::collections::BTreeSet;

use kernel::sweep::SweepError;
use kernel::{EdgeId, EntityId, FaceId, SceneChange};

use super::{Document, check_coordinates};
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// Sweeps face `face` along the chain of edges `path`. Refusals change nothing.
    pub(super) fn follow_me_changes(
        &mut self,
        face: EntityId,
        path: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if path.is_empty() {
            return Err(invalid("path must list edges forming one chain"));
        }
        let missing: Vec<EntityId> = std::iter::once(face)
            .chain(path.iter().copied())
            .filter(|id| !self.exists(*id))
            .collect();
        if !missing.is_empty() {
            return Err(self.no_such_entities(missing));
        }
        let EntityId::Face(profile) = face else {
            return Err(invalid("face must be a face").with_ids(vec![face]));
        };
        let others: Vec<EntityId> = path
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Edge(_)))
            .collect();
        if !others.is_empty() {
            return Err(invalid("path must list only edges").with_ids(others));
        }
        let edges: Vec<EdgeId> = path
            .iter()
            .filter_map(|id| match id {
                EntityId::Edge(e) => Some(*e),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let ids: Vec<EntityId> = edges.iter().map(|e| EntityId::Edge(*e)).collect();
        let m = self.model();
        let Some((vertices, closed)) = m.edge_chain(&edges) else {
            return Err(
                invalid("the path edges must form one connected chain without branches")
                    .with_ids(ids),
            );
        };
        let own: BTreeSet<EdgeId> = m.face_edges(profile).into_iter().collect();
        let shared: Vec<EntityId> = edges
            .iter()
            .filter(|e| own.contains(e))
            .map(|e| EntityId::Edge(*e))
            .collect();
        if !shared.is_empty() && !self.is_face_loop(profile, &edges) {
            return Err(invalid(
                "the path must not include the profile's own edges, unless it is the outer loop of another face",
            )
            .with_ids(shared));
        }
        let least = kernel::sweep::MIN_LENGTH;
        let short: Vec<EntityId> = edges
            .iter()
            .filter(|e| {
                let [a, b] = m.edge(**e).unwrap().v;
                m.position(a).unwrap().distance(m.position(b).unwrap()) < least
            })
            .map(|e| EntityId::Edge(*e))
            .collect();
        if !short.is_empty() {
            return Err(
                invalid(&format!("path segments must be at least {least} m long")).with_ids(short),
            );
        }
        let culprits: Vec<EntityId> = std::iter::once(face).chain(ids).collect();
        let plan = m
            .plan_follow_me(profile, &vertices, closed)
            .map_err(|e| sweep_error(e).with_ids(culprits))?;
        let points: Vec<[f64; 3]> = plan
            .sweep
            .sections
            .iter()
            .flatten()
            .flatten()
            .map(|p| p.to_array())
            .collect();
        check_coordinates(&points)?;
        Ok(self
            .edit_mesh(|m| Ok(m.follow_me(&plan)))
            .expect("drawing a planned sweep cannot fail"))
    }

    /// True when `edges` are exactly the outer loop of a face other than `profile`.
    fn is_face_loop(&self, profile: FaceId, edges: &[EdgeId]) -> bool {
        let m = self.model();
        let set: BTreeSet<EdgeId> = edges.iter().copied().collect();
        m.faces().any(|(id, f)| {
            let n = f.outer.len();
            id != profile
                && (0..n)
                    .map(|i| m.edge_between(f.outer[i], f.outer[(i + 1) % n]))
                    .collect::<Option<BTreeSet<EdgeId>>>()
                    .is_some_and(|outer| outer == set)
        })
    }
}

/// The refusal of a sweep the kernel cannot make.
fn sweep_error(e: SweepError) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, e.to_string())
}
