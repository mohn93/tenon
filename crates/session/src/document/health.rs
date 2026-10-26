//! The `model_health` query and the `make_solid` command: which parts are not solids and
//! why, and closing what can be closed safely, each definition once, in one undo step.

use std::collections::BTreeMap;

use kernel::health::{Health, check, repair};
use kernel::{DefinitionId, EntityId, InstanceId, MeshId, SceneChange};

use super::Document;
use crate::command::{CommandError, MakeSolidEntry, MakeSolidReport};
use crate::query::{HealthPart, HealthProblem, ProblemKind, QueryResult};

/// `n` with `one` or `many` after it.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The problems in `health`, of `definition`'s own geometry.
fn problems(definition: DefinitionId, health: &Health) -> Vec<HealthProblem> {
    let mut out = Vec::new();
    let mut add = |kind, ids: Vec<EntityId>, text: String| {
        out.push(HealthProblem {
            definition,
            kind,
            ids,
            text,
        })
    };
    let edges = |l: &[kernel::EdgeId]| l.iter().copied().map(EntityId::Edge).collect::<Vec<_>>();
    let faces = |l: &[kernel::FaceId]| l.iter().copied().map(EntityId::Face).collect::<Vec<_>>();
    let h = health;
    if !h.open_edges.is_empty() {
        let n = h.open_edges.len();
        add(
            ProblemKind::OpenEdges,
            edges(&h.open_edges),
            count(n, "open edge", "open edges"),
        );
    }
    if !h.shared_edges.is_empty() {
        let n = h.shared_edges.len();
        add(
            ProblemKind::SharedEdges,
            edges(&h.shared_edges),
            format!(
                "{} by more than two faces",
                count(n, "edge shared", "edges shared")
            ),
        );
    }
    if !h.stray_edges.is_empty() {
        let n = h.stray_edges.len();
        add(
            ProblemKind::StrayEdges,
            edges(&h.stray_edges),
            count(n, "stray edge", "stray edges"),
        );
    }
    if !h.reversed_faces.is_empty() {
        let n = h.reversed_faces.len();
        add(
            ProblemKind::ReversedFaces,
            faces(&h.reversed_faces),
            format!("{} inside out", count(n, "face", "faces")),
        );
    }
    if !h.gaps.is_empty() {
        let n = h.gaps.len();
        add(
            ProblemKind::Gaps,
            h.gaps
                .iter()
                .flat_map(|(a, b)| [EntityId::Vertex(*a), EntityId::Vertex(*b)])
                .collect(),
            count(n, "tiny gap", "tiny gaps"),
        );
    }
    if !h.internal_faces.is_empty() {
        let n = h.internal_faces.len();
        add(
            ProblemKind::InternalFaces,
            faces(&h.internal_faces),
            count(n, "internal face", "internal faces"),
        );
    }
    if !h.self_crossing.is_empty() {
        let n = h.self_crossing.len();
        add(
            ProblemKind::SelfCrossing,
            faces(&h.self_crossing),
            format!("{} cross each other", count(n, "face", "faces")),
        );
    }
    if h.no_volume {
        add(
            ProblemKind::NoVolume,
            Vec::new(),
            "closed but encloses no volume".into(),
        );
    }
    out
}

impl Document {
    /// The definitions `definition` places at any depth, itself first, each once.
    fn definitions_in(&self, definition: DefinitionId) -> Vec<DefinitionId> {
        let mut out = vec![definition];
        let mut k = 0;
        while k < out.len() {
            if let Some(d) = self.scene.definition(out[k]) {
                for (_, i) in d.mesh.instances() {
                    if !out.contains(&i.definition) {
                        out.push(i.definition);
                    }
                }
            }
            k += 1;
        }
        out
    }

    /// The parts `ids` (instances of the open context; every one when empty) with the
    /// definition each places.
    fn health_parts(
        &self,
        ids: &[EntityId],
    ) -> Result<Vec<(InstanceId, DefinitionId)>, CommandError> {
        let m = self.model();
        let ids = if ids.is_empty() {
            m.instances().map(|(id, _)| id).collect()
        } else {
            self.part_ids(ids)?
        };
        Ok(ids
            .into_iter()
            .map(|id| (id, m.instance(id).expect("checked").definition))
            .collect())
    }

    /// Whether `definition` has edges or faces of its own.
    fn has_geometry(&self, definition: DefinitionId) -> bool {
        self.scene
            .definition(definition)
            .is_some_and(|d| d.mesh.edges().next().is_some())
    }

    fn has_faces(&self, definition: DefinitionId) -> bool {
        self.scene
            .definition(definition)
            .is_some_and(|d| d.mesh.faces().next().is_some())
    }

    pub(super) fn model_health_query(&self, ids: &[EntityId]) -> Result<QueryResult, CommandError> {
        let parts = self.health_parts(ids)?;
        // Each definition is checked, and its repair tried, once.
        let mut seen: BTreeMap<DefinitionId, (Vec<HealthProblem>, bool)> = BTreeMap::new();
        let mut out = Vec::new();
        for &(id, definition) in &parts {
            let defs = self.definitions_in(definition);
            let mut list = Vec::new();
            let mut fixable = true;
            for d in defs.iter().copied().filter(|d| self.has_geometry(*d)) {
                let (found, can) = seen.entry(d).or_insert_with(|| {
                    let mesh = &self.scene.definition(d).expect("listed").mesh;
                    let health = check(mesh);
                    let can = health.is_solid() || repair(mesh).model.is_some();
                    (problems(d, &health), can)
                });
                list.extend(found.iter().cloned());
                fixable &= *can;
            }
            if !defs.iter().any(|d| self.has_faces(*d)) {
                fixable = false;
                list.insert(
                    0,
                    HealthProblem {
                        definition,
                        kind: ProblemKind::NoFaces,
                        ids: Vec::new(),
                        text: "no faces".into(),
                    },
                );
            }
            if list.is_empty() {
                continue;
            }
            out.push(HealthPart {
                id: EntityId::Instance(id),
                name: self
                    .scene
                    .definition(definition)
                    .map(|d| d.name.clone())
                    .unwrap_or_default(),
                fixable,
                problems: list,
            });
        }
        Ok(QueryResult::ModelHealth {
            parts: out,
            checked: parts.len(),
            revision: self.revision,
        })
    }

    /// `make_solid`: repairs each definition of `ids` (see the command) that is not a solid,
    /// all in one change; the report goes to the outcome.
    pub(super) fn make_solid_changes(
        &mut self,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let parts = self.health_parts(ids)?;
        let mut order: Vec<DefinitionId> = Vec::new();
        let mut holders: BTreeMap<DefinitionId, Vec<EntityId>> = BTreeMap::new();
        for &(id, definition) in &parts {
            for d in self.definitions_in(definition) {
                let list = holders.entry(d).or_default();
                if list.is_empty() {
                    order.push(d);
                }
                if !list.contains(&EntityId::Instance(id)) {
                    list.push(EntityId::Instance(id));
                }
            }
        }
        let mut report = MakeSolidReport::default();
        let mut changes = Vec::new();
        for d in order {
            if !self.has_geometry(d) {
                continue;
            }
            if check(&self.scene.definition(d).expect("listed").mesh).is_solid() {
                report.solid += 1;
                continue;
            }
            let mut done = None;
            let change = self
                .scene
                .edit(MeshId::Definition(d), |m| {
                    let r = repair(m);
                    let fixed = r.model.is_some();
                    if let Some(closed) = r.model {
                        *m = closed;
                    }
                    done = Some((r.repair, r.left, fixed));
                    Ok(m.take_changes())
                })
                .expect("repairing cannot fail");
            changes.push(change);
            let (r, left, fixed) = done.expect("the edit ran");
            report.entries.push(if fixed {
                MakeSolidEntry {
                    definition: d,
                    parts: holders[&d].clone(),
                    fixed,
                    welded: r.welded,
                    tjunctions_mended: r.tjunctions_mended,
                    strays_removed: r.strays_removed,
                    internal_removed: r.internal_removed,
                    holes_filled: r.holes_filled,
                    faces_flipped: r.faces_flipped,
                    left: Vec::new(),
                    reasons: Vec::new(),
                }
            } else {
                MakeSolidEntry {
                    definition: d,
                    parts: holders[&d].clone(),
                    fixed,
                    welded: 0,
                    tjunctions_mended: 0,
                    strays_removed: 0,
                    internal_removed: 0,
                    holes_filled: 0,
                    faces_flipped: 0,
                    left: problems(d, &left),
                    reasons: r.reasons,
                }
            });
        }
        self.make_solid_report = Some(report);
        Ok(SceneChange::join(changes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{DefinitionId, FaceId};

    #[test]
    fn faces_that_cross_each_other_are_a_problem_of_their_own() {
        let health = Health {
            reason: Some("its faces cross each other".into()),
            self_crossing: vec![FaceId(3), FaceId(9)],
            ..Health::default()
        };
        let list = problems(DefinitionId(1), &health);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, ProblemKind::SelfCrossing);
        assert_eq!(list[0].text, "2 faces cross each other");
        assert_eq!(
            list[0].ids,
            vec![EntityId::Face(FaceId(3)), EntityId::Face(FaceId(9))]
        );
    }
}
