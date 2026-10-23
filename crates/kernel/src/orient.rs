//! Reversing faces and orienting a connected surface so its faces agree.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::ids::{FaceId, VertexId};
use crate::model::{ChangeSet, Model};

/// What an orient edit did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrientReport {
    /// The faces turned over, in id order.
    pub flipped: Vec<FaceId>,
    /// Shared edges whose two faces could not be made to agree: the surface cannot be oriented
    /// (a Moebius strip). The first orientation reached stands.
    pub conflicts: usize,
}

impl Model {
    /// Turns each of `ids` over as one ChangeSet, with the faces turned in id order. Faces keep
    /// their ids and each side keeps its material; unknown ids are skipped.
    pub fn reverse_faces(&mut self, ids: &[FaceId]) -> (ChangeSet, Vec<FaceId>) {
        let mut changed = Vec::new();
        for &id in ids.iter().collect::<BTreeSet<_>>() {
            if let Some(f) = self.face(id) {
                let turned = f.reversed();
                self.replace_face(id, turned);
                changed.push(id);
            }
        }
        (self.take_changes(), changed)
    }

    /// Orients the faces connected to `start` as one ChangeSet, breadth first from it: across
    /// each side used by exactly two faces the neighbour is turned over if it runs the side the
    /// same way round as the face before it. Sides used by three or more faces are not
    /// crossed. With `within` not empty only those faces (and `start`) are walked. A face
    /// reached again from another direction that disagrees is counted in `conflicts` and keeps
    /// the orientation it was given first.
    pub fn orient_faces(&mut self, start: FaceId, within: &[FaceId]) -> (ChangeSet, OrientReport) {
        let mut report = OrientReport::default();
        if self.face(start).is_none() {
            return (self.take_changes(), report);
        }
        let scope: BTreeSet<FaceId> = within.iter().copied().chain([start]).collect();
        let in_scope = |f: FaceId| within.is_empty() || scope.contains(&f);

        // Each side with the faces using it and whether they run it from the lower vertex.
        let mut by_side: BTreeMap<(VertexId, VertexId), Vec<(FaceId, bool)>> = BTreeMap::new();
        for (fid, f) in self.faces() {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    by_side
                        .entry((a.min(b), a.max(b)))
                        .or_default()
                        .push((fid, a < b));
                }
            }
        }

        // Whether each reached face is to be turned over.
        let mut turn: BTreeMap<FaceId, bool> = BTreeMap::from([(start, false)]);
        let mut conflicting: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        let mut queue = VecDeque::from([start]);
        while let Some(fid) = queue.pop_front() {
            let f = self.face(fid).expect("walked faces exist");
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    let key = (a.min(b), a.max(b));
                    let [(g, g_up), (h, h_up)] = by_side[&key][..] else {
                        continue;
                    };
                    let (next, next_up) = if g == fid { (h, h_up) } else { (g, g_up) };
                    if next == fid || !in_scope(next) {
                        continue;
                    }
                    // The face before it, as it will run the side once turned.
                    let here_up = (a < b) ^ turn[&fid];
                    // It must run the side the other way round.
                    let want = next_up == here_up;
                    match turn.get(&next) {
                        None => {
                            turn.insert(next, want);
                            queue.push_back(next);
                        }
                        Some(&now) if now != want => {
                            conflicting.insert(key);
                        }
                        Some(_) => {}
                    }
                }
            }
        }

        for (id, flip) in turn {
            if flip {
                let turned = self.face(id).expect("walked faces exist").reversed();
                self.replace_face(id, turned);
                report.flipped.push(id);
            }
        }
        report.conflicts = conflicting.len();
        (self.take_changes(), report)
    }
}
