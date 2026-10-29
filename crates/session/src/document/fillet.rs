//! `chamfer_edges` and `fillet_edges`: bevelling or rounding the straight edges of a solid, in
//! the open context's own geometry or in an instance's definition.

use std::collections::BTreeSet;

use kernel::boolean::{BooleanError, NOT_CLOSED, TOO_MUCH};
use kernel::fillet::{BevelError, DEFAULT_SEGMENTS, MAX_SEGMENTS, MIN_SIZE, Profile};
use kernel::solid::instance_volume;
use kernel::{EdgeId, EntityId, MeshId, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind};

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

impl Document {
    /// `chamfer_edges`: `edges` of the open context, or of `instance`'s definition.
    pub(super) fn chamfer_edges_changes(
        &mut self,
        instance: Option<EntityId>,
        edges: &[EntityId],
        distance: f64,
        distances: Option<&[f64]>,
    ) -> Result<SceneChange, CommandError> {
        if !(distance.is_finite() && distance > MIN_SIZE) {
            return Err(invalid(format!("distance must be more than {MIN_SIZE} m")));
        }
        self.bevel_changes(instance, edges, Profile::Chamfer { distance }, distances)
    }

    /// `fillet_edges`: as `chamfer_edges`, rounding with `radius` in `segments` (default 8)
    /// faces per quarter turn.
    pub(super) fn fillet_edges_changes(
        &mut self,
        instance: Option<EntityId>,
        edges: &[EntityId],
        radius: f64,
        segments: Option<u32>,
        radii: Option<&[f64]>,
    ) -> Result<SceneChange, CommandError> {
        if !(radius.is_finite() && radius > MIN_SIZE) {
            return Err(invalid(format!("radius must be more than {MIN_SIZE} m")));
        }
        let segments = segments.unwrap_or(DEFAULT_SEGMENTS);
        if !(1..=MAX_SEGMENTS).contains(&segments) {
            return Err(invalid(format!("segments must be 1 to {MAX_SEGMENTS}")));
        }
        self.bevel_changes(instance, edges, Profile::Fillet { radius, segments }, radii)
    }

    /// Bevels `edges` (see `chamfer_edges`) with `profile`; on an error nothing changes.
    fn bevel_changes(
        &mut self,
        instance: Option<EntityId>,
        edges: &[EntityId],
        profile: Profile,
        sizes: Option<&[f64]>,
    ) -> Result<SceneChange, CommandError> {
        if edges.is_empty() {
            return Err(invalid("edges must name at least 1 edge"));
        }
        let (noun, plural) = match profile {
            Profile::Chamfer { .. } => ("distance", "distances"),
            Profile::Fillet { .. } => ("radius", "radii"),
        };
        if let Some(sizes) = sizes {
            if sizes.len() != edges.len() {
                return Err(invalid(format!(
                    "{plural} must have one value for each of the {} edges, not {}",
                    edges.len(),
                    sizes.len()
                )));
            }
            if let Some(bad) = sizes.iter().find(|s| !(s.is_finite() && **s > MIN_SIZE)) {
                return Err(invalid(format!(
                    "{noun} must be more than {MIN_SIZE} m, not {bad}"
                )));
            }
        }
        let (mesh, owner) = match instance {
            None => (self.active(), None),
            Some(id) => {
                if !self.exists(id) {
                    return Err(self.no_such_entities(vec![id]));
                }
                let EntityId::Instance(i) = id else {
                    return Err(
                        invalid(format!("{id} is not a group or component")).with_ids(vec![id])
                    );
                };
                let placed = self.model().instance(i).expect("exists").clone();
                instance_volume(&self.scene, &placed).map_err(|why| {
                    invalid(format!("{id} is not a solid: {why}")).with_ids(vec![id])
                })?;
                (MeshId::Definition(placed.definition), Some(id))
            }
        };
        let m = self.scene.mesh(mesh).expect("the mesh exists");
        let mut seen = BTreeSet::new();
        let mut ids: Vec<EdgeId> = Vec::new();
        let mut each: Vec<(EdgeId, f64)> = Vec::new();
        let mut missing = Vec::new();
        for (at, id) in edges.iter().enumerate() {
            let EntityId::Edge(e) = *id else {
                if owner.is_none() && !self.exists(*id) {
                    if !missing.contains(id) {
                        missing.push(*id);
                    }
                    continue;
                }
                return Err(invalid(format!("{id} is not an edge")).with_ids(vec![*id]));
            };
            if m.edge(e).is_none() {
                if !missing.contains(id) {
                    missing.push(*id);
                }
                continue;
            }
            if !seen.insert(e) {
                return Err(invalid(format!("{id} is named twice")).with_ids(vec![*id]));
            }
            ids.push(e);
            each.push((e, sizes.map_or(profile.size(), |s| s[at])));
        }
        if !missing.is_empty() {
            return Err(match owner {
                None => self.no_such_entities(missing),
                Some(owner) => {
                    let list: Vec<String> = missing.iter().map(|id| id.to_string()).collect();
                    CommandError::new(
                        ErrorKind::NotFound,
                        format!("no edge {} in the definition of {owner}", list.join(", ")),
                    )
                    .with_ids(missing)
                }
            });
        }
        self.scene
            .bevel_edges_each(mesh, &each, profile)
            .map_err(|e| match e {
                BevelError::NotSolid(why) => {
                    invalid(format!("the open context is not a solid: {why}"))
                }
                BevelError::Invalid(why) => invalid(why),
                BevelError::Edge(e, why) => invalid(why).with_ids(vec![EntityId::Edge(e)]),
                BevelError::Edges(pair, why) => {
                    invalid(why).with_ids(pair.map(EntityId::Edge).to_vec())
                }
                BevelError::Boolean(BooleanError::TooMuch) => invalid(TOO_MUCH),
                BevelError::Boolean(_) => invalid(NOT_CLOSED).with_ids(edges.to_vec()),
            })
    }
}
