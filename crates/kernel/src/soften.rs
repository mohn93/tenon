//! Softening and hardening edges: a soft edge is hidden and the faces on its two sides are
//! shaded as one smooth surface (see `smooth`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{ChangeSet, Model};

/// What a soften or harden edit did.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct SoftenReport {
    /// Edges whose soft flag changed.
    pub changed: usize,
    /// Edges left as they were: not between exactly two faces when softening, or (by angle)
    /// between faces meeting at more than the angle. Edges already as asked are not counted.
    pub skipped: usize,
}

impl Model {
    /// The edges along a face's outer loop and holes, in id order.
    pub fn face_edges(&self, id: FaceId) -> Vec<EdgeId> {
        let Some(f) = self.face(id) else {
            return Vec::new();
        };
        let mut edges = BTreeSet::new();
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for i in 0..l.len() {
                edges.extend(self.edge_between(l[i], l[(i + 1) % l.len()]));
            }
        }
        edges.into_iter().collect()
    }

    /// The two faces an edge lies between, oldest first, when it bounds exactly two faces
    /// (each using it once).
    pub fn faces_beside(&self, id: EdgeId) -> Option<[FaceId; 2]> {
        let [a, b] = self.edge(id)?.v;
        let faces = self.faces_on_side(a, b);
        let [f, g] = faces[..] else { return None };
        let once = |f: FaceId| sides_in(self, f, a, b) == 1;
        (once(f) && once(g)).then_some([f, g])
    }

    /// The angle between the normals of the two faces beside an edge, in radians (0: they lie
    /// in one plane), with the second face turned over first if the two are oriented against
    /// each other (both loops run the same way along the edge).
    pub fn fold_angle(&self, id: EdgeId) -> Option<f64> {
        let [f, g] = self.faces_beside(id)?;
        let [a, b] = self.edge(id)?.v;
        let (nf, mut ng) = (self.face(f)?.normal, self.face(g)?.normal);
        if runs(self, f, a, b) == runs(self, g, a, b) {
            ng = -ng;
        }
        Some(nf.dot(ng).clamp(-1.0, 1.0).acos())
    }

    /// Sets (`soft`) or clears the soft flag of `edges`, as one ChangeSet. Only an edge between
    /// exactly two faces is made soft; others are left as they are and counted as skipped.
    /// Unknown ids are ignored.
    pub fn soften_edges(&mut self, edges: &[EdgeId], soft: bool) -> (ChangeSet, SoftenReport) {
        let mut report = SoftenReport::default();
        for &e in edges.iter().collect::<BTreeSet<_>>() {
            let Some(edge) = self.edge(e) else { continue };
            if edge.soft == soft {
                continue;
            }
            if soft && self.faces_beside(e).is_none() {
                report.skipped += 1;
                continue;
            }
            let (curve, polygon) = (edge.curve, edge.polygon);
            self.set_edge(e, curve, soft, polygon);
            report.changed += 1;
        }
        (self.take_changes(), report)
    }

    /// Softens each of `edges` that lies between exactly two faces meeting at no more than
    /// `max_angle` radians (see `fold_angle`), as one ChangeSet. Other edges stay as they are;
    /// hard ones are counted as skipped. Unknown ids are ignored.
    pub fn soften_by_angle(
        &mut self,
        edges: &[EdgeId],
        max_angle: f64,
    ) -> (ChangeSet, SoftenReport) {
        let mut report = SoftenReport::default();
        for &e in edges.iter().collect::<BTreeSet<_>>() {
            let Some(edge) = self.edge(e) else { continue };
            if edge.soft {
                continue;
            }
            // A small allowance, so 90 degrees takes the sides of a box.
            if self.fold_angle(e).is_some_and(|a| a <= max_angle + 1e-9) {
                let (curve, polygon) = (edge.curve, edge.polygon);
                self.set_edge(e, curve, true, polygon);
                report.changed += 1;
            } else {
                report.skipped += 1;
            }
        }
        (self.take_changes(), report)
    }
}

/// True when one of the face's loops runs from `a` straight to `b`.
fn runs(m: &Model, f: FaceId, a: VertexId, b: VertexId) -> bool {
    m.face(f).is_some_and(|f| {
        std::iter::once(&f.outer)
            .chain(&f.holes)
            .any(|l| (0..l.len()).any(|i| l[i] == a && l[(i + 1) % l.len()] == b))
    })
}

/// How many times the face's loops use the side a-b, either way round.
fn sides_in(m: &Model, f: FaceId, a: VertexId, b: VertexId) -> usize {
    m.face(f).map_or(0, |f| {
        std::iter::once(&f.outer)
            .chain(&f.holes)
            .map(|l| {
                (0..l.len())
                    .filter(|&i| {
                        let (x, y) = (l[i], l[(i + 1) % l.len()]);
                        (x, y) == (a, b) || (x, y) == (b, a)
                    })
                    .count()
            })
            .sum()
    })
}
