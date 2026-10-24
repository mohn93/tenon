//! The `interference` and `clearance` queries: which solid parts overlap and how near parts
//! come. Read-only: the overlap volumes come from booleans on copies.

use kernel::interfere::{Checker, Part, SWEEP_REPORT_WITHIN, sweep_check};
use kernel::{EntityId, InstanceId, JointId, KernelError};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::{
    ClearancePair, FailedPair, InterferencePair, MAX_CLEARANCE_IDS, MAX_SWEEP_STEPS, QueryResult,
    SkippedPart, SweepPair, SweepStop, TouchingPair,
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

impl Document {
    /// The `sweep_check` query: drives `joint` through the range on a copy of the scene.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sweep_check_query(
        &self,
        joint: JointId,
        from: f64,
        to: f64,
        steps: u32,
        ids: &[EntityId],
        slide: Option<f64>,
        report_within: Option<f64>,
    ) -> Result<QueryResult, CommandError> {
        let Some(j) = self.scene.joint(joint) else {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no joint {joint}"),
            ));
        };
        let angular = j.kind.is_angular();
        if !from.is_finite() || !to.is_finite() || slide.is_some_and(|s| !s.is_finite()) {
            return Err(invalid("from, to and slide must be finite numbers"));
        }
        if !(2..=MAX_SWEEP_STEPS).contains(&steps) {
            return Err(invalid(format!(
                "steps must be from 2 to {MAX_SWEEP_STEPS}"
            )));
        }
        let within = report_within.unwrap_or(SWEEP_REPORT_WITHIN);
        if !within.is_finite() || within < 0.0 {
            return Err(invalid("report_within must be a distance of 0 or more"));
        }
        let root = self.scene.root();
        let mut parts = Vec::new();
        for id in ids {
            let EntityId::Instance(i) = *id else {
                return Err(
                    invalid(format!("{id} is not a group or component")).with_ids(vec![*id])
                );
            };
            if root.instance(i).is_none() {
                return Err(CommandError::new(
                    ErrorKind::NotFound,
                    format!("no group or component {id} in the root"),
                )
                .with_ids(vec![*id]));
            }
            if !parts.contains(&i) {
                parts.push(i);
            }
        }
        let to_kernel = |v: f64| if angular { v.to_radians() } else { v };
        let shown = |v: f64| if angular { v.to_degrees() } else { v };
        let report = sweep_check(
            &self.scene,
            joint,
            to_kernel(from),
            to_kernel(to),
            steps as usize,
            slide,
            &parts,
            within,
        )
        .map_err(|e| match e {
            KernelError::JointNotFound => {
                CommandError::new(ErrorKind::NotFound, format!("no joint {joint}"))
            }
            e => invalid(e.to_string()),
        })?;
        let e = EntityId::Instance;
        Ok(QueryResult::SweepCheck {
            pairs: report
                .pairs
                .into_iter()
                .map(|q| SweepPair {
                    a: e(q.a),
                    b: e(q.b),
                    first: q.interferes.map(|(f, _)| shown(f)),
                    last: q.interferes.map(|(_, l)| shown(l)),
                    volume: q.volume,
                    volume_at: q.volume_at.map(shown),
                    clearance: q.clearance,
                    clearance_at: shown(q.clearance_at),
                    failed: q.failed.map(|(value, reason)| SweepStop {
                        value: shown(value),
                        reason,
                    }),
                })
                .collect(),
            skipped: skipped(report.skipped),
            steps: report.steps as u32,
            stopped: report.stopped.map(|(value, reason)| SweepStop {
                value: shown(value),
                reason,
            }),
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
