//! The `interference` and `clearance` queries: which solid parts overlap and how near parts
//! come. Read-only: the overlap volumes come from booleans on copies.

use kernel::interfere::{Checker, Part};
use kernel::{EntityId, InstanceId};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::{
    ClearancePair, FailedPair, InterferencePair, MAX_CLEARANCE_IDS, QueryResult, SkippedPart,
    TouchingPair,
};

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

fn skipped(list: Vec<(InstanceId, String)>) -> Vec<SkippedPart> {
    list.into_iter()
        .map(|(id, reason)| SkippedPart {
            id: EntityId::Instance(id),
            reason,
        })
        .collect()
}

impl Document {
    /// `ids` as instances of the open context, each once in the order first named: an id
    /// that is not an instance is `invalid_input`, an unknown one `not_found`.
    fn part_ids(&self, ids: &[EntityId]) -> Result<Vec<InstanceId>, CommandError> {
        let m = self.model();
        let mut out = Vec::new();
        for id in ids {
            let EntityId::Instance(i) = *id else {
                return Err(
                    invalid(format!("{id} is not a group or component")).with_ids(vec![*id])
                );
            };
            if m.instance(i).is_none() {
                return Err(self.missing_error(vec![*id], format!("no entity {id}")));
            }
            if !out.contains(&i) {
                out.push(i);
            }
        }
        Ok(out)
    }

    pub(super) fn interference_query(
        &self,
        ids: &[EntityId],
        touching: bool,
    ) -> Result<QueryResult, CommandError> {
        let ids = self.part_ids(ids)?;
        let mut checker = Checker::new();
        let (parts, skip) = checker.parts(&self.scene, self.active(), &ids);
        let report = checker.interference(&self.scene, &parts);
        let e = EntityId::Instance;
        Ok(QueryResult::Interference {
            pairs: report
                .overlaps
                .iter()
                .map(|o| InterferencePair {
                    a: e(o.a),
                    b: e(o.b),
                    volume: o.volume,
                    point: o.point.to_array(),
                })
                .collect(),
            touching: touching.then(|| {
                report
                    .touching
                    .iter()
                    .map(|t| TouchingPair {
                        a: e(t.a),
                        b: e(t.b),
                        point: t.point.to_array(),
                    })
                    .collect()
            }),
            skipped: skipped(skip),
            failed: report
                .failed
                .into_iter()
                .map(|(a, b, reason)| FailedPair {
                    a: e(a),
                    b: e(b),
                    reason,
                })
                .collect(),
            revision: self.revision,
        })
    }

    pub(super) fn clearance_query(
        &self,
        a: Option<EntityId>,
        b: Option<EntityId>,
        ids: &[EntityId],
    ) -> Result<QueryResult, CommandError> {
        let named: Vec<EntityId> = match (a, b, ids.is_empty()) {
            (Some(a), Some(b), true) => {
                if a == b {
                    return Err(invalid("a and b must be different parts").with_ids(vec![a]));
                }
                vec![a, b]
            }
            (None, None, false) => ids.to_vec(),
            _ => {
                return Err(invalid(
                    "give either a and b, or ids (at least 2 groups or components)",
                ));
            }
        };
        let ids = self.part_ids(&named)?;
        if ids.len() < 2 {
            return Err(invalid("ids must name at least 2 groups or components"));
        }
        if ids.len() > MAX_CLEARANCE_IDS {
            return Err(invalid(format!(
                "ids may name at most {MAX_CLEARANCE_IDS} groups or components"
            )));
        }
        let mut checker = Checker::new();
        let (parts, skip) = checker.parts(&self.scene, self.active(), &ids);
        let mut pairs = Vec::new();
        for (k, pa) in parts.iter().enumerate() {
            for pb in &parts[k + 1..] {
                if let Some(pair) = clearance_pair(&mut checker, self, pa, pb) {
                    pairs.push(pair);
                }
            }
        }
        Ok(QueryResult::Clearance {
            pairs,
            skipped: skipped(skip),
            revision: self.revision,
        })
    }
}

fn clearance_pair(
    checker: &mut Checker,
    d: &Document,
    a: &Part,
    b: &Part,
) -> Option<ClearancePair> {
    let c = checker.clearance(&d.scene, a, b)?;
    Some(ClearancePair {
        a: EntityId::Instance(a.id),
        b: EntityId::Instance(b.id),
        distance: c.distance,
        point_a: c.a.to_array(),
        point_b: c.b.to_array(),
        overlap: c.overlap,
    })
}
