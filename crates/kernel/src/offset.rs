//! Offsetting loops and chains of points sideways in their plane (the Offset tool).

use glam::{DVec2, DVec3};

use crate::curves::chain;
use crate::edit::{KernelError, check_segment};
use crate::geom::{LENGTH_TOL, Plane, point_in_polygon, segment_distance_2d};
use crate::ids::{EdgeId, VertexId};
use crate::model::{ChangeSet, Model};

/// Why an offset cannot be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffsetError {
    /// The offset crosses or touches itself.
    SelfCrossing,
    /// A side reverses or shrinks to nothing: the shape is too small for the distance.
    Collapsed,
}

impl std::fmt::Display for OffsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OffsetError::SelfCrossing => write!(f, "the offset would cross itself"),
            OffsetError::Collapsed => write!(f, "the shape is too small for the offset distance"),
        }
    }
}

impl std::error::Error for OffsetError {}

/// A line of points to draw (see `Model::add_polylines`).
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub points: Vec<DVec3>,
    /// The last point joins the first.
    pub closed: bool,
    /// Some: the edges go on one new curve, as polygon sides when true (see `Model::add_curve`).
    pub curve: Option<bool>,
}

impl Polyline {
    fn sides(&self) -> impl Iterator<Item = (DVec3, DVec3)> + '_ {
        let n = self.points.len();
        let count = if self.closed { n } else { n.saturating_sub(1) };
        (0..count).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }
}

impl Model {
    /// Draws the polylines with the rules of `add_edge`, as one ChangeSet; those with `curve`
    /// put their edges on a new curve each, as `add_curve` does. Every side is checked first,
    /// so nothing is drawn when one is invalid.
    pub fn add_polylines(&mut self, lines: &[Polyline]) -> Result<ChangeSet, KernelError> {
        for l in lines {
            for (a, b) in l.sides() {
                check_segment(a, b)?;
            }
        }
        let mut changes = ChangeSet::default();
        for l in lines {
            let part = match l.curve {
                Some(polygon) => self.add_curve(&l.points, l.closed, polygon)?,
                None => {
                    let segments: Vec<(DVec3, DVec3)> = l.sides().collect();
                    self.draw_segments(&segments);
                    self.take_changes()
                }
            };
            changes.ops.extend(part.ops);
        }
        Ok(changes)
    }

    /// The edges as one unbranched chain: its vertices in order and whether it closes (the
    /// first vertex is then not repeated). None when the edges branch, fall apart, are none, or
    /// one does not exist.
    pub fn edge_chain(&self, edges: &[EdgeId]) -> Option<(Vec<VertexId>, bool)> {
        let members: Vec<(EdgeId, [VertexId; 2])> = edges
            .iter()
            .map(|e| self.edge(*e).map(|edge| (*e, edge.v)))
            .collect::<Option<_>>()?;
        chain(&members).map(|c| (c.vertices, c.closed))
    }
}

/// The unit normal of the plane through `points`, or None when they lie on one line (within
/// `LENGTH_TOL`). Whether every point lies in that plane is not checked.
pub fn plane_normal(points: &[DVec3]) -> Option<DVec3> {
    let a = *points.first()?;
    let b = *points
        .iter()
        .max_by(|p, q| p.distance(a).total_cmp(&q.distance(a)))?;
    let axis = (b - a).try_normalize()?;
    let off = |p: &DVec3| (*p - a).reject_from_normalized(axis).length();
    let c = *points.iter().max_by(|p, q| off(p).total_cmp(&off(q)))?;
    if off(&c) < LENGTH_TOL {
        return None;
    }
    axis.cross(c - a).try_normalize()
}

/// Which side of the chain through `points` (closed: the last point joins the first) `toward`
/// lies on, looking down `normal`: 1 to the left of the walking direction, -1 to the right,
/// as the nearest side sees it. None when `toward` lies on the chain (within `LENGTH_TOL`) or
/// straight ahead of an end of it.
pub fn side_of(points: &[DVec3], closed: bool, normal: DVec3, toward: DVec3) -> Option<f64> {
    let n = points.len();
    let count = if closed { n } else { n.saturating_sub(1) };
    // Each side's distance to toward and which way toward lies from it.
    let seen: Vec<(f64, f64)> = (0..count)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let ab = b - a;
            let t = ((toward - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            let q = a + ab * t;
            let dist = q.distance(toward);
            let s = (toward - q).dot(normal.cross(ab).normalize_or_zero());
            (dist, s / dist.max(f64::MIN_POSITIVE))
        })
        .collect();
    let best = seen.iter().map(|s| s.0).fold(f64::INFINITY, f64::min);
    // Sides equally near (meeting at the nearest corner) each have a say.
    let tie = 1e-9 * best.max(1.0);
    let side: f64 = seen.iter().filter(|s| s.0 <= best + tie).map(|s| s.1).sum();
    (best >= LENGTH_TOL && side.abs() > 1e-9).then(|| side.signum())
}

/// A corner whose miter would reach farther than this many times the distance is bevelled.
const MITER_LIMIT: f64 = 4.0;

/// Offsets the closed loop through `points` by `distance` in its plane (with unit `normal`):
/// every side moves `distance` to its left looking down `normal` (inward for a loop running
/// counter-clockwise around `normal`; negative: to its right). Corners are mitred, except that a
/// corner on the outside of its turn whose miter would reach farther than `4 * |distance|` is
/// bevelled (two points). Points closer than `LENGTH_TOL` to the one before are merged.
pub fn offset_loop(
    points: &[DVec3],
    normal: DVec3,
    distance: f64,
) -> Result<Vec<DVec3>, OffsetError> {
    offset_line(points, normal, distance, true, None)
}

/// Offsets the open chain through `points` by `distance` in its plane, to the left of the
/// walking direction looking down `normal` (negative: to the right). The ends move square to
/// their own side; corners are made as in `offset_loop`.
pub fn offset_chain(
    points: &[DVec3],
    normal: DVec3,
    distance: f64,
) -> Result<Vec<DVec3>, OffsetError> {
    offset_line(points, normal, distance, false, None)
}

/// Offsets an open chain whose corners lie on a circle around `center` (an arc) as
/// `offset_chain` does, except that each end moves along the line from `center` through it, to
/// where its side's offset line meets that line, so the result lies on a circle around `center`
/// too.
pub fn offset_arc(
    points: &[DVec3],
    normal: DVec3,
    distance: f64,
    center: DVec3,
) -> Result<Vec<DVec3>, OffsetError> {
    offset_line(points, normal, distance, false, Some(center))
}

/// Offsets a face's loops (`rings[0]` its outer loop, counter-clockwise around `normal`, the
/// rest its holes, clockwise) by `distance`. Inward (positive) every loop is offset: the outer
/// loop shrinks and the holes grow, and every new loop must lie inside the face (inside its
/// outer loop, outside its holes) without touching another loop. Outward (negative) only the
/// outer loop is offset (the holes stay as they are) and the face must lie inside it. Else
/// `Collapsed`. Returns the new loops: all of them inward, the outer one only outward.
pub fn offset_face(
    rings: &[Vec<DVec3>],
    normal: DVec3,
    distance: f64,
) -> Result<Vec<Vec<DVec3>>, OffsetError> {
    let offset: &[Vec<DVec3>] = if distance > 0.0 { rings } else { &rings[..1] };
    let new: Vec<Vec<DVec3>> = offset
        .iter()
        .map(|r| offset_loop(r, normal, distance))
        .collect::<Result<_, _>>()?;
    let plane = Plane::through(rings[0][0], normal.normalize());
    let flat = |r: &Vec<DVec3>| -> Vec<DVec2> { r.iter().map(|p| plane.project(*p)).collect() };
    let old: Vec<Vec<DVec2>> = rings.iter().map(flat).collect();
    let made: Vec<Vec<DVec2>> = new.iter().map(flat).collect();
    // No new loop may touch another new loop or an old one.
    let all: Vec<&[DVec2]> = made.iter().chain(&old).map(Vec::as_slice).collect();
    for i in 0..made.len() {
        for j in i + 1..all.len() {
            if rings_meet(all[i], all[j]) {
                return Err(OffsetError::Collapsed);
            }
        }
    }
    // Loops that do not meet nest as one point of theirs does.
    let inside = |a: &[DVec2], b: &[DVec2]| point_in_polygon(a[0], b);
    if distance < 0.0 {
        return if inside(&old[0], &made[0]) {
            Ok(new)
        } else {
            Err(OffsetError::Collapsed)
        };
    }
    if !inside(&made[0], &old[0]) {
        return Err(OffsetError::Collapsed);
    }
    let holes = old.len() - 1;
    for k in 1..=holes {
        if !inside(&old[k], &made[k]) || !inside(&made[k], &made[0]) {
            return Err(OffsetError::Collapsed);
        }
        if (1..=holes).any(|j| j != k && inside(&made[k], &made[j])) {
            return Err(OffsetError::Collapsed);
        }
    }
    Ok(new)
}

fn offset_line(
    points: &[DVec3],
    normal: DVec3,
    distance: f64,
    closed: bool,
    center: Option<DVec3>,
) -> Result<Vec<DVec3>, OffsetError> {
    let normal = normal.normalize();
    let pts = merged(points.iter().copied(), closed);
    let n = pts.len();
    if n < if closed { 3 } else { 2 } {
        return Err(OffsetError::Collapsed);
    }
    let sides = if closed { n } else { n - 1 };
    let dirs: Vec<DVec3> = (0..sides)
        .map(|i| (pts[(i + 1) % n] - pts[i]).normalize())
        .collect();
    let left = |d: DVec3| normal.cross(d);
    // An open end: square to its side, or along the radius through it for an arc (when the
    // radius is not along the side).
    let end = |p: DVec3, d: DVec3| {
        let l = left(d);
        let radial = center.and_then(|c| (p - c).try_normalize());
        match radial {
            Some(u) if u.dot(l).abs() > 1e-6 => p + u * (distance / u.dot(l)),
            _ => p + l * distance,
        }
    };
    // The new point or points at each corner, in order.
    let corners: Vec<Vec<DVec3>> = (0..n)
        .map(|i| {
            if !closed && i == 0 {
                vec![end(pts[0], dirs[0])]
            } else if !closed && i == n - 1 {
                vec![end(pts[i], dirs[i - 1])]
            } else {
                corner(
                    pts[i],
                    dirs[(i + sides - 1) % sides],
                    dirs[i],
                    normal,
                    distance,
                )
            }
        })
        .collect();
    // Each side must keep its direction and at least the merge tolerance of length.
    for (i, d) in dirs.iter().enumerate() {
        let a = *corners[i].last().unwrap();
        let b = corners[(i + 1) % n][0];
        if (b - a).dot(*d) < LENGTH_TOL {
            return Err(OffsetError::Collapsed);
        }
    }
    let out = merged(corners.into_iter().flatten(), closed);
    if out.len() < if closed { 3 } else { 2 } {
        return Err(OffsetError::Collapsed);
    }
    let plane = Plane::through(out[0], normal);
    let flat: Vec<DVec2> = out.iter().map(|p| plane.project(*p)).collect();
    if crosses_itself(&flat, closed) {
        return Err(OffsetError::SelfCrossing);
    }
    Ok(out)
}

/// The new point (or the two points of a bevel) at corner `p`, entered along `din` and left
/// along `dout`.
fn corner(p: DVec3, din: DVec3, dout: DVec3, normal: DVec3, distance: f64) -> Vec<DVec3> {
    let (lin, lout) = (normal.cross(din), normal.cross(dout));
    let turn = din.cross(dout).dot(normal);
    let bevel = vec![p + lin * distance, p + lout * distance];
    let Some(m) = (lin + lout).try_normalize() else {
        // The chain turns straight back.
        return bevel;
    };
    // The miter reaches |distance| / cos, cos being that of half the turn.
    let cos = m.dot(lout);
    let outside = turn * distance < 0.0;
    if outside && cos * MITER_LIMIT < 1.0 {
        return bevel;
    }
    vec![p + m * (distance / cos)]
}

/// The points without those closer than `LENGTH_TOL` to the one kept before (and, closed, the
/// last one when it is that close to the first).
fn merged(points: impl Iterator<Item = DVec3>, closed: bool) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = Vec::new();
    for p in points {
        if out.last().is_none_or(|q| q.distance(p) >= LENGTH_TOL) {
            out.push(p);
        }
    }
    while closed && out.len() > 1 && out[0].distance(*out.last().unwrap()) < LENGTH_TOL {
        out.pop();
    }
    out
}

/// The sides of a 2D polyline (closed: the last point joined to the first).
fn sides(poly: &[DVec2], closed: bool) -> Vec<(DVec2, DVec2)> {
    let n = poly.len();
    let count = if closed { n } else { n.saturating_sub(1) };
    (0..count).map(|i| (poly[i], poly[(i + 1) % n])).collect()
}

/// True when two sides of the polyline cross or touch, apart from neighbours meeting at their
/// shared corner (neighbours that fold back over each other count).
fn crosses_itself(poly: &[DVec2], closed: bool) -> bool {
    let s = sides(poly, closed);
    let n = s.len();
    for i in 0..n {
        for j in i + 1..n {
            let neighbours = j == i + 1 || (closed && i == 0 && j == n - 1);
            if neighbours {
                // They share one end: they meet elsewhere only by folding back.
                let (a, b) = if j == i + 1 {
                    (s[i], s[j])
                } else {
                    (s[j], s[i])
                };
                if segment_distance_2d(a.0, b.0, b.1) < LENGTH_TOL
                    || segment_distance_2d(b.1, a.0, a.1) < LENGTH_TOL
                {
                    return true;
                }
            } else if segments_meet(s[i], s[j]) {
                return true;
            }
        }
    }
    false
}

/// True when a side of closed polygon `a` crosses or touches a side of closed polygon `b`.
fn rings_meet(a: &[DVec2], b: &[DVec2]) -> bool {
    let (sa, sb) = (sides(a, true), sides(b, true));
    sa.iter().any(|x| sb.iter().any(|y| segments_meet(*x, *y)))
}

/// True when two closed 2D segments cross or come within `LENGTH_TOL` of each other.
fn segments_meet((p, q): (DVec2, DVec2), (a, b): (DVec2, DVec2)) -> bool {
    let tol = LENGTH_TOL;
    // Boxes farther apart than the tolerance cannot meet.
    if p.max(q).x + tol < a.min(b).x
        || a.max(b).x + tol < p.min(q).x
        || p.max(q).y + tol < a.min(b).y
        || a.max(b).y + tol < p.min(q).y
    {
        return false;
    }
    let side = |o: DVec2, u: DVec2, w: DVec2| (u - o).perp_dot(w - o);
    let cross = side(p, q, a) * side(p, q, b) < 0.0 && side(a, b, p) * side(a, b, q) < 0.0;
    cross
        || segment_distance_2d(p, a, b) < tol
        || segment_distance_2d(q, a, b) < tol
        || segment_distance_2d(a, p, q) < tol
        || segment_distance_2d(b, p, q) < tol
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    fn same(got: &[DVec3], want: &[DVec3]) {
        assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
        for (a, b) in got.iter().zip(want) {
            assert!(a.distance(*b) < 1e-12, "{got:?} vs {want:?}");
        }
    }

    fn square() -> Vec<DVec3> {
        vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]
    }

    #[test]
    fn a_square_shrinks_inward_and_grows_outward() {
        let inner = offset_loop(&square(), DVec3::Z, 0.1).unwrap();
        same(
            &inner,
            &[p(0.1, 0.1), p(0.9, 0.1), p(0.9, 0.9), p(0.1, 0.9)],
        );
        let outer = offset_loop(&square(), DVec3::Z, -0.1).unwrap();
        same(
            &outer,
            &[p(-0.1, -0.1), p(1.1, -0.1), p(1.1, 1.1), p(-0.1, 1.1)],
        );
    }

    #[test]
    fn points_closer_than_the_tolerance_are_merged() {
        let mut pts = square();
        pts.insert(1, p(1.0 - 1e-8, 0.0));
        let inner = offset_loop(&pts, DVec3::Z, 0.1).unwrap();
        assert_eq!(inner.len(), 4, "{inner:?}");
    }

    #[test]
    fn an_l_shape_offsets_inward_with_its_six_corners() {
        let l = [
            p(0.0, 0.0),
            p(2.0, 0.0),
            p(2.0, 1.0),
            p(1.0, 1.0),
            p(1.0, 2.0),
            p(0.0, 2.0),
        ];
        let inner = offset_loop(&l, DVec3::Z, 0.1).unwrap();
        same(
            &inner,
            &[
                p(0.1, 0.1),
                p(1.9, 0.1),
                p(1.9, 0.9),
                p(0.9, 0.9),
                p(0.9, 1.9),
                p(0.1, 1.9),
            ],
        );
        // Too far: the arms are 1 wide.
        assert!(offset_loop(&l, DVec3::Z, 0.6).is_err());
    }

    fn star() -> Vec<DVec3> {
        (0..10)
            .map(|i| {
                let a = std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
                let r = if i % 2 == 0 { 1.0 } else { 0.4 };
                p(r * a.cos(), r * a.sin())
            })
            .collect()
    }

    #[test]
    fn a_star_offsets_inward_a_little_and_refuses_a_lot() {
        let small = offset_loop(&star(), DVec3::Z, 0.02).unwrap();
        assert_eq!(small.len(), 10);
        let err = offset_loop(&star(), DVec3::Z, 0.5).unwrap_err();
        assert!(
            matches!(err, OffsetError::SelfCrossing | OffsetError::Collapsed),
            "{err:?}"
        );
    }

    #[test]
    fn a_sharp_spike_is_bevelled_on_its_outside_only() {
        // The corner at (10, 0) turns by 170 degrees.
        let h = 10.0 * 10f64.to_radians().tan();
        let tri = [p(0.0, 0.0), p(10.0, 0.0), p(0.0, h)];
        let out = offset_loop(&tri, DVec3::Z, -0.1).unwrap();
        assert_eq!(out.len(), 4, "{out:?}");
        let tip: Vec<&DVec3> = out
            .iter()
            .filter(|q| (q.distance(p(10.0, 0.0)) - 0.1).abs() < 1e-12)
            .collect();
        assert_eq!(tip.len(), 2, "{out:?}");
        // Inward the corner is a trim, not a bevel.
        let inner = offset_loop(&tri, DVec3::Z, 0.1).unwrap();
        assert_eq!(inner.len(), 3, "{inner:?}");
    }

    #[test]
    fn a_chain_offsets_left_of_its_walk_and_its_ends_move_square() {
        let u = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 1.0), p(0.0, 1.0)];
        let inside = offset_chain(&u, DVec3::Z, 0.1).unwrap();
        same(
            &inside,
            &[p(0.0, 0.1), p(1.9, 0.1), p(1.9, 0.9), p(0.0, 0.9)],
        );
        let outside = offset_chain(&u, DVec3::Z, -0.1).unwrap();
        same(
            &outside,
            &[p(0.0, -0.1), p(2.1, -0.1), p(2.1, 1.1), p(0.0, 1.1)],
        );
        // The U is 1 wide: half of it closes the gap.
        assert!(offset_chain(&u, DVec3::Z, 0.5).is_err());
    }

    #[test]
    fn a_hairpin_chain_offsets_outward_without_crossing() {
        let hairpin = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 0.1), p(0.0, 0.1)];
        let out = offset_chain(&hairpin, DVec3::Z, -0.05).unwrap();
        same(
            &out,
            &[p(0.0, -0.05), p(2.05, -0.05), p(2.05, 0.15), p(0.0, 0.15)],
        );
        assert!(offset_chain(&hairpin, DVec3::Z, 0.05).is_err());
    }

    #[test]
    fn a_face_with_a_hole_shrinks_its_outline_and_grows_its_hole() {
        let outer = vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)];
        // Holes wind clockwise around the normal.
        let hole = vec![p(1.5, 1.5), p(1.5, 2.5), p(2.5, 2.5), p(2.5, 1.5)];
        let rings = offset_face(&[outer.clone(), hole.clone()], DVec3::Z, 0.5).unwrap();
        same(
            &rings[0],
            &[p(0.5, 0.5), p(3.5, 0.5), p(3.5, 3.5), p(0.5, 3.5)],
        );
        same(
            &rings[1],
            &[p(1.0, 1.0), p(1.0, 3.0), p(3.0, 3.0), p(3.0, 1.0)],
        );
        // Outward only the outline moves.
        let rings = offset_face(&[outer.clone(), hole.clone()], DVec3::Z, -0.25).unwrap();
        assert_eq!(rings.len(), 1);
        same(
            &rings[0],
            &[
                p(-0.25, -0.25),
                p(4.25, -0.25),
                p(4.25, 4.25),
                p(-0.25, 4.25),
            ],
        );
        // The grown hole meets the shrunk outline.
        assert_eq!(
            offset_face(&[outer.clone(), hole.clone()], DVec3::Z, 1.0),
            Err(OffsetError::Collapsed)
        );
        // Outward far is fine: the hole is left alone.
        assert!(offset_face(&[outer, hole], DVec3::Z, -0.6).is_ok());
    }

    #[test]
    fn side_of_follows_the_nearest_side_also_where_the_chain_doubles_back() {
        let u = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 1.0), p(0.0, 1.0)];
        let side = |x, y| side_of(&u, false, DVec3::Z, p(x, y));
        assert_eq!(side(1.0, 0.5), Some(1.0));
        assert_eq!(side(1.0, 0.95), Some(1.0));
        assert_eq!(side(1.0, 1.5), Some(-1.0));
        assert_eq!(side(1.0, -0.5), Some(-1.0));
        // Off the corner, outside.
        assert_eq!(side(3.0, -1.0), Some(-1.0));
        // On the chain, or straight ahead of an end.
        assert_eq!(side(1.0, 0.0), None);
        assert_eq!(side(-1.0, 0.0), None);
    }

    #[test]
    fn plane_normal_of_a_chain_and_of_a_straight_one() {
        // An S has no area as a closed polygon, but a plane all the same.
        let s = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(2.0, 1.0)];
        let n = plane_normal(&s).unwrap();
        assert!((n.z.abs() - 1.0).abs() < 1e-12, "{n:?}");
        assert_eq!(plane_normal(&[p(0.0, 0.0), p(1.0, 0.0), p(3.0, 0.0)]), None);
    }

    #[test]
    fn an_arc_offsets_onto_a_circle_also_at_its_ends() {
        let arc: Vec<DVec3> = (0..=12)
            .map(|i| {
                let a = i as f64 * std::f64::consts::FRAC_PI_2 / 12.0;
                p(a.cos(), a.sin())
            })
            .collect();
        let r = 1.0 - 0.1 / (std::f64::consts::PI / 48.0).cos();
        let out = offset_arc(&arc, DVec3::Z, 0.1, DVec3::ZERO).unwrap();
        assert_eq!(out.len(), 13);
        for q in &out {
            assert!((q.length() - r).abs() < 1e-12, "{q:?}");
        }
        // Square ends miss the circle.
        let plain = offset_chain(&arc, DVec3::Z, 0.1).unwrap();
        assert!((plain[0].length() - r).abs() > 1e-6);
    }
}
