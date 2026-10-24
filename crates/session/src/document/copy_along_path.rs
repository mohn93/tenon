//! Copy along a path: copies of a selection spaced along a chain of edges.

use std::collections::BTreeSet;

use glam::{DAffine3, DQuat, DVec3};
use kernel::{EdgeId, EntityId, SceneChange};

use super::{Document, check_coordinates};
use crate::command::{CommandError, ErrorKind};

/// Most copies one `copy_along_path` command may make.
const MAX_PATH_COPIES: usize = 10_000;
/// Slack when a position falls on the end of a segment or of the path.
const EPS: f64 = 1e-9;

/// A path as a polyline with its arc lengths.
struct Walk {
    points: Vec<DVec3>,
    closed: bool,
    /// `cum[i]` is the length up to `points[i]`; one more entry than segments.
    cum: Vec<f64>,
}

impl Walk {
    fn segments(&self) -> usize {
        self.cum.len() - 1
    }

    fn length(&self) -> f64 {
        *self.cum.last().unwrap()
    }

    fn end(&self, i: usize) -> DVec3 {
        self.points[(i + 1) % self.points.len()]
    }

    /// The segment a position at `t` belongs to: at a vertex the one leaving it, except at the
    /// end of an open path.
    fn segment_at(&self, t: f64) -> usize {
        (0..self.segments())
            .find(|i| t < self.cum[i + 1] - EPS)
            .unwrap_or(self.segments() - 1)
    }

    fn direction(&self, i: usize) -> DVec3 {
        (self.end(i) - self.points[i]).normalize()
    }

    /// The smallest rotation taking `from` to `to`; for a straight-back turn, a half turn
    /// about the path's turning plane normal (not an arbitrary axis).
    fn turn_between(&self, from: DVec3, to: DVec3) -> DQuat {
        if from.dot(to) < -1.0 + EPS {
            let axis = (0..self.segments())
                .map(|i| from.cross(self.direction(i)))
                .find(|c| c.length() > 1e-6)
                .and_then(|c| c.try_normalize());
            if let Some(axis) = axis {
                return DQuat::from_axis_angle(axis, std::f64::consts::PI);
            }
        }
        DQuat::from_rotation_arc(from, to)
    }

    fn point_at(&self, t: f64) -> DVec3 {
        let i = self.segment_at(t);
        let along = (t - self.cum[i]).clamp(0.0, self.cum[i + 1] - self.cum[i]);
        self.points[i] + self.direction(i) * along
    }
}

impl Document {
    /// Copies `ids` along the chain of edges `path`, by `count` or by `spacing`.
    pub(super) fn copy_along_path_changes(
        &mut self,
        ids: &[EntityId],
        path: &[EntityId],
        (count, spacing): (Option<u32>, Option<f64>),
        align: bool,
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if ids.is_empty() {
            return Err(invalid(
                "ids must list at least one edge, face, instance, guide, dimension, note or section plane",
            ));
        }
        if path.is_empty() {
            return Err(invalid("path must list edges forming one chain"));
        }
        match (count, spacing) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(invalid("give exactly one of count and spacing"));
            }
            (Some(0), _) => return Err(invalid("count must be at least 1")),
            (_, Some(s)) if !(s.is_finite() && s > 0.0) => {
                return Err(invalid("spacing must be a positive number"));
            }
            _ => {}
        }
        let missing: Vec<EntityId> = ids
            .iter()
            .chain(path)
            .copied()
            .filter(|id| !self.exists(*id))
            .collect();
        if !missing.is_empty() {
            return Err(self.no_such_entities(missing));
        }
        let others: Vec<EntityId> = path
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Edge(_)))
            .collect();
        if !others.is_empty() {
            return Err(invalid("path must list only edges").with_ids(others));
        }
        let parts = self.parts(ids)?;
        let edges: Vec<EdgeId> = path
            .iter()
            .filter_map(|id| match id {
                EntityId::Edge(e) => Some(*e),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let m = self.model();
        let Some((vertices, closed)) = m.edge_chain(&edges) else {
            let ids = edges.iter().map(|e| EntityId::Edge(*e)).collect();
            return Err(
                invalid("the path edges must form one connected chain without branches")
                    .with_ids(ids),
            );
        };
        let points: Vec<DVec3> = vertices.iter().map(|v| m.position(*v).unwrap()).collect();
        let anchor = self.selection_center(ids, &parts);
        let walk = start_walk(points, closed, anchor);
        let length = walk.length();
        if !(length > 0.0 && length.is_finite()) {
            return Err(invalid("the path has no length"));
        }
        let times = positions(&walk, count, spacing)?;
        let p0 = walk.points[0];
        let t0 = walk.direction(0);
        let places: Vec<DAffine3> = times
            .iter()
            .map(|t| {
                let at = walk.point_at(*t);
                let turn = if align {
                    walk.turn_between(t0, walk.direction(walk.segment_at(*t)))
                } else {
                    DQuat::IDENTITY
                };
                DAffine3::from_translation(at)
                    * DAffine3::from_quat(turn)
                    * DAffine3::from_translation(-p0)
            })
            .collect();
        if places.is_empty() {
            return Ok(SceneChange::default());
        }
        check_coordinates(&[p0.to_array()])?;
        self.check_placed(&self.vertex_ids(ids), &parts, &places, true)?;
        self.copy_placed_changes(ids, &parts, &places)
    }

    /// The centre of the box around the geometry, instances, guides and planes of `ids`
    /// (the origin when they have no extent, as with annotations alone).
    fn selection_center(&self, ids: &[EntityId], parts: &super::Parts) -> DVec3 {
        let m = self.model();
        let mut points: Vec<DVec3> = self
            .vertex_ids(ids)
            .iter()
            .filter_map(|v| m.position(*v))
            .collect();
        for i in &parts.instances {
            if let Some(b) = self.instance_box(*i) {
                points.extend(b);
            }
        }
        points.extend(
            parts
                .guides
                .iter()
                .filter_map(|g| m.guide(*g))
                .map(|g| g.anchor()),
        );
        points.extend(
            parts
                .sections
                .iter()
                .filter_map(|p| m.section(*p))
                .map(|p| p.point),
        );
        let Some(first) = points.first().copied() else {
            return DVec3::ZERO;
        };
        let (lo, hi) = points
            .iter()
            .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        (lo + hi) / 2.0
    }
}

/// The path as a walk from its start: an open path from the end nearest `anchor`, a closed one
/// from the vertex nearest it (in chain order).
fn start_walk(mut points: Vec<DVec3>, closed: bool, anchor: DVec3) -> Walk {
    let near = |a: DVec3, b: DVec3| a.distance_squared(anchor) < b.distance_squared(anchor);
    if closed {
        let best = (0..points.len()).fold(0, |best, i| {
            if near(points[i], points[best]) {
                i
            } else {
                best
            }
        });
        points.rotate_left(best);
    } else if near(points[points.len() - 1], points[0]) {
        points.reverse();
    }
    let n = points.len();
    let segments = if closed { n } else { n - 1 };
    let mut cum = vec![0.0];
    for i in 0..segments {
        let l = points[i].distance(points[(i + 1) % n]);
        cum.push(cum[i] + l);
    }
    Walk {
        points,
        closed,
        cum,
    }
}

/// The arc lengths of the copies: the positions of the rule, without the start.
fn positions(
    walk: &Walk,
    count: Option<u32>,
    spacing: Option<f64>,
) -> Result<Vec<f64>, CommandError> {
    let too_many = || {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!("at most {MAX_PATH_COPIES} copies"),
        )
    };
    let length = walk.length();
    let all: Vec<f64> = match (count, spacing) {
        (Some(n), _) => {
            let n = n as usize;
            if n > MAX_PATH_COPIES + 1 {
                return Err(too_many());
            }
            let steps = if walk.closed { n } else { n.max(2) - 1 };
            (0..n).map(|k| length * k as f64 / steps as f64).collect()
        }
        (None, Some(s)) => {
            let reach = length / s;
            if reach + 1.0 > (MAX_PATH_COPIES + 1) as f64 + 1.0 {
                return Err(too_many());
            }
            (0..)
                .map(|k| s * k as f64)
                .take_while(|t| {
                    if walk.closed {
                        *t < length - EPS
                    } else {
                        *t <= length + EPS
                    }
                })
                .map(|t| t.min(length))
                .collect()
        }
        (None, None) => unreachable!("checked"),
    };
    if all.len() > MAX_PATH_COPIES + 1 {
        return Err(too_many());
    }
    // Copies closer than the merge tolerance would merge with the originals or each other.
    let mut steps: Vec<f64> = all.windows(2).map(|w| w[1] - w[0]).collect();
    if walk.closed {
        steps.push(length - all.last().copied().unwrap_or(0.0));
    }
    let least = steps.into_iter().fold(f64::INFINITY, f64::min);
    if least < kernel::geom::LENGTH_TOL {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "copies would be closer together than the merge tolerance",
        ));
    }
    Ok(all.into_iter().skip(1).collect())
}
