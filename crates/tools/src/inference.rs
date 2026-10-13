//! Snapping the cursor to meaningful 3D points.

use std::cell::Cell;

use glam::{DVec2, DVec3};
use kernel::{EdgeId, FaceId, Guide, GuideId, Model, Shape, VertexId};
use session::{Camera, Ray};

use crate::scenery::{Owner, Part, Scenery};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn dir(self) -> DVec3 {
        match self {
            Axis::X => DVec3::X,
            Axis::Y => DVec3::Y,
            Axis::Z => DVec3::Z,
        }
    }
}

/// What the cursor snapped to, from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Origin,
    /// A guide point.
    GuidePoint,
    /// Where two edges or guide lines cross on screen (or in space): the point on the nearer
    /// one.
    Intersection,
    OnEdge,
    /// On a guide line.
    OnGuide,
    /// Along a red/green/blue axis through the point the tool is drawing from.
    Axis(Axis),
    /// Parallel to the reference edge, through the point the tool is drawing from.
    Parallel,
    /// Perpendicular to the reference edge, through the point the tool is drawing from.
    Perpendicular,
    OnFace,
    Ground,
    /// Nothing to snap to: a point in front of the camera under the cursor.
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inference {
    pub point: DVec3,
    pub kind: SnapKind,
    /// The nearest face under the cursor, whatever the point snapped to, if it belongs to the
    /// open context (a face inside an instance or outside the open instance is not given).
    pub face: Option<FaceId>,
}

/// What inference can follow from the point a tool is drawing from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Guides {
    /// The point a tool is drawing from; directions are measured from it.
    pub anchor: Option<DVec3>,
    /// Unit direction of the last edge the cursor rested on: snaps parallel and perpendicular
    /// to it are offered from the anchor.
    pub reference: Option<DVec3>,
    /// A locked direction (Shift or an arrow key): the point stays on the line through the
    /// anchor, and points the cursor snaps to are projected onto it.
    pub lock: Option<Lock>,
}

/// A direction inference is held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lock {
    /// Unit direction.
    pub dir: DVec3,
    /// How the locked point is reported (an axis, parallel or perpendicular).
    pub kind: SnapKind,
}

/// Finds the 3D point under `cursor` (physical pixels) on one mesh, with no guides but
/// `anchor`, which enables axis inference. See [`infer_in`].
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let guides = Guides {
        anchor,
        ..Guides::default()
    };
    infer_with(model, camera, cursor, &guides, radius_px)
}

/// [`infer_in`] on one mesh.
pub fn infer_with(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    guides: &Guides,
    radius_px: f64,
) -> Inference {
    infer_in(&Scenery::single(model), camera, cursor, guides, radius_px)
}

/// Finds the 3D point under `cursor` (physical pixels) among everything `scenery` holds, in
/// the open context's coordinates (`camera` must be in them too). Snap candidates count only
/// within `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint, origin and guide point (nearest on screen wins) > intersection
/// (of edges and guide lines) > on-edge > on-guide-line > directions from the anchor (axes,
/// parallel or perpendicular to the reference edge; nearest wins) > on-face > ground > free.
/// Points and edges hidden behind a face are skipped. With a lock, the result is always on
/// the locked line.
pub fn infer_in(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    guides: &Guides,
    radius_px: f64,
) -> Inference {
    let ray = camera.ray(cursor);
    let face_hit = nearest_face_hit(scenery, &ray);
    let face = face_hit.and_then(|(owner, f, _)| (owner == Owner::Open).then_some(f));
    let projector = camera.projector();
    let screen_dist = |p: DVec3| projector.project(p).map(|s| s.distance(cursor));
    let snapped = snap_point(scenery, camera, cursor, radius_px);

    if let (Some(anchor), Some(lock)) = (guides.anchor, guides.lock) {
        let point = match snapped {
            Some((p, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
            None => closest_on_line_to_ray(anchor, lock.dir, &ray, false),
        };
        return Inference {
            point,
            kind: lock.kind,
            face,
        };
    }
    if let Some((point, kind)) = snapped {
        return Inference { point, kind, face };
    }

    // Along a direction from the anchor. Direction lines are guides, so faces do not hide them.
    if let Some(anchor) = guides.anchor {
        let mut along: Option<(f64, Inference)> = None;
        for (dir, kind) in directions(guides.reference) {
            let p = closest_on_line_to_ray(anchor, dir, &ray, false);
            if let Some(d) = screen_dist(p).filter(|d| *d <= radius_px)
                && p.distance(anchor) > 0.0
                && along.is_none_or(|(bd, _)| d < bd)
            {
                along = Some((
                    d,
                    Inference {
                        point: p,
                        kind,
                        face,
                    },
                ));
            }
        }
        if let Some((_, hit)) = along {
            return hit;
        }
    }

    // On a face (nearest along the ray).
    if let Some((_, _, point)) = face_hit {
        return Inference {
            point,
            kind: SnapKind::OnFace,
            face,
        };
    }

    match camera.ground_at(cursor) {
        Some(point) => Inference {
            point,
            kind: SnapKind::Ground,
            face,
        },
        None => Inference {
            point: camera.target_depth_at(cursor),
            kind: SnapKind::Free,
            face,
        },
    }
}

/// The visible point snap under the cursor, if any: endpoint, midpoint, origin or guide point
/// (nearest on screen), else the nearest apparent intersection of two edges or guide lines
/// both visible there, else the nearest point on an edge, else on a guide line.
fn snap_point(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
) -> Option<(DVec3, SnapKind)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    // How far from the cursor a point is on screen, if within the radius.
    let near_enough = |p: DVec3| {
        projector
            .project(p)
            .map(|s| s.distance(cursor))
            .filter(|d| *d <= radius_px)
    };
    let sight = Sight::new(scenery, camera);
    let seen = |p: &DVec3| sight.sees(*p);
    let near = Near::around(scenery, camera, cursor, radius_px);
    let mut points: Vec<(f64, (DVec3, SnapKind))> = Vec::new();
    let mut offer = |p: DVec3, kind: SnapKind| {
        if let Some(d) = near_enough(p) {
            points.push((d, (p, kind)));
        }
    };
    for &(_, _, p) in &near.points {
        offer(p, SnapKind::Endpoint);
    }
    for &(_, _, a, b) in &near.segments {
        offer((a + b) * 0.5, SnapKind::Midpoint);
    }
    offer(DVec3::ZERO, SnapKind::Origin);
    let guides = scenery.guides();
    for &(_, _, g) in guides {
        if let Guide::Point { position } = g {
            offer(position, SnapKind::GuidePoint);
        }
    }
    if let Some((_, (p, kind))) = nearest_visible(points, |(p, _)| seen(p)) {
        return Some((p, kind));
    }

    // Guide lines passing near the cursor, as the part of them in view.
    let guide_lines: Vec<(DVec3, DVec3)> = guides
        .iter()
        .filter_map(|&(_, _, g)| match g {
            Guide::Line { point, direction } => Some((point, direction)),
            Guide::Point { .. } => None,
        })
        .filter(|&(point, dir)| {
            let p = closest_on_line_to_ray(point, dir, &ray, false);
            projector
                .project(p)
                .is_some_and(|s| s.distance(cursor) <= radius_px)
        })
        .collect();
    // Two lines cross within the radius only where both pass that near the cursor on screen,
    // so only lines passing within twice the radius (a margin far above any rounding) are
    // paired, each projected once.
    let lines: Vec<ScreenLine> = near
        .segments
        .iter()
        .map(|&(_, _, a, b)| (a, b))
        .chain(
            guide_lines
                .iter()
                .filter_map(|&(point, dir)| camera.line_span(point, dir))
                .map(|[a, b]| (a, b)),
        )
        .filter_map(|(a, b)| {
            let (sa, sb) = (projector.project(a)?, projector.project(b)?);
            (distance_to_segment(cursor, sa, sb) <= 2.0 * radius_px).then_some(ScreenLine {
                ends: (a, b),
                screen: (sa, sb),
            })
        })
        .collect();
    let mut crossings = Vec::new();
    for (i, ab) in lines.iter().enumerate() {
        for cd in &lines[i + 1..] {
            if let Some((p, far)) = apparent_crossing(camera, ab, cd)
                && let Some(dist) = near_enough(p)
            {
                crossings.push((dist, (p, far)));
            }
        }
    }
    if let Some((_, (p, _))) = nearest_visible(crossings, |(p, far)| seen(far) && seen(p)) {
        return Some((p, SnapKind::Intersection));
    }

    let on_edge = near
        .segments
        .iter()
        .filter_map(|&(_, _, a, b)| {
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            near_enough(p).map(|d| (d, p))
        })
        .collect();
    if let Some((_, p)) = nearest_visible(on_edge, seen) {
        return Some((p, SnapKind::OnEdge));
    }

    let on_guide = guide_lines
        .iter()
        .filter_map(|&(point, dir)| {
            let p = closest_on_line_to_ray(point, dir, &ray, false);
            near_enough(p).map(|d| (d, p))
        })
        .collect();
    nearest_visible(on_guide, seen).map(|(_, p)| (p, SnapKind::OnGuide))
}

/// The nearest of `candidates` (each with its distance from the cursor on screen) that `seen`
/// holds for: the least distance, the first given among equally near ones. Seeing is the
/// dearest test, so candidates are tried nearest first and those after the answer never are.
fn nearest_visible<T>(
    mut candidates: Vec<(f64, T)>,
    mut seen: impl FnMut(&T) -> bool,
) -> Option<(f64, T)> {
    // A stable sort: equally near candidates stay in the order given.
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.into_iter().find(|(_, c)| seen(c))
}

/// The visible guide nearest the cursor on screen whose owner passes `keep`, if one lies
/// within `radius_px`: whose it is, the guide (an id in its own mesh) and the guide placed in
/// the open context's coordinates.
pub fn guide_under_where(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
    keep: impl Fn(Owner) -> bool,
) -> Option<(Owner, GuideId, Guide)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    // Guides are few, so they are scanned, not indexed.
    let candidates = scenery
        .guides()
        .iter()
        .filter_map(|&(part, id, g)| {
            let owner = scenery.part(part).owner;
            if !keep(owner) {
                return None;
            }
            let p = match g {
                Guide::Point { position } => position,
                Guide::Line { point, direction } => {
                    closest_on_line_to_ray(point, direction, &ray, false)
                }
            };
            let d = projector.project(p)?.distance(cursor);
            (d <= radius_px).then_some((d, (owner, id, g, p)))
        })
        .collect();
    let sight = Sight::new(scenery, camera);
    nearest_visible(candidates, |&(.., p)| sight.sees(p))
        .map(|(_, (owner, id, g, _))| (owner, id, g))
}

/// A segment and where its ends are on screen.
struct ScreenLine {
    ends: (DVec3, DVec3),
    screen: (DVec2, DVec2),
}

/// Distance on screen from `p` to the segment `a`-`b`.
fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Where segments a-b and c-d cross on screen away from their ends: the points under the
/// crossing on the nearer and the farther segment, nearer first. Edges that meet in space share
/// a vertex (the kernel splits crossing edges), so their crossing is an endpoint, not this.
fn apparent_crossing(camera: &Camera, ab: &ScreenLine, cd: &ScreenLine) -> Option<(DVec3, DVec3)> {
    let ((a, b), (sa, sb)) = (ab.ends, ab.screen);
    let ((c, d), (sc, sd)) = (cd.ends, cd.screen);
    let (r, q) = (sb - sa, sd - sc);
    let denom = r.perp_dot(q);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (sc - sa).perp_dot(q) / denom;
    let u = (sc - sa).perp_dot(r) / denom;
    let inside = |k: f64| k > 1e-6 && k < 1.0 - 1e-6;
    if !inside(t) || !inside(u) {
        return None;
    }
    let ray = camera.ray(sa + r * t);
    let on_ab = closest_on_line_to_ray(a, b - a, &ray, true);
    let on_cd = closest_on_line_to_ray(c, d - c, &ray, true);
    Some(
        if on_ab.distance(ray.origin) <= on_cd.distance(ray.origin) {
            (on_ab, on_cd)
        } else {
            (on_cd, on_ab)
        },
    )
}

/// Unit directions offered from the anchor: the three axes, plus, for a reference direction
/// not along an axis, the direction itself (parallel) and two perpendiculars (one horizontal,
/// one in the vertical plane through the reference) when they are not along an axis either.
pub fn directions(reference: Option<DVec3>) -> Vec<(DVec3, SnapKind)> {
    let mut out: Vec<(DVec3, SnapKind)> = Axis::ALL
        .into_iter()
        .map(|a| (a.dir(), SnapKind::Axis(a)))
        .collect();
    let along_axis = |d: DVec3| Axis::ALL.iter().any(|a| d.cross(a.dir()).length() < 1e-6);
    let Some(r) = reference.and_then(|r| r.try_normalize()) else {
        return out;
    };
    if along_axis(r) {
        return out;
    }
    out.push((r, SnapKind::Parallel));
    let horizontal = r.cross(DVec3::Z).try_normalize();
    let upright = horizontal.and_then(|h| r.cross(h).try_normalize());
    for p in [horizontal, upright].into_iter().flatten() {
        if !along_axis(p) {
            out.push((p, SnapKind::Perpendicular));
        }
    }
    out
}

/// The nearest face of one mesh under the cursor and the point where the cursor's ray meets it.
pub fn face_under(model: &Model, camera: &Camera, cursor: DVec2) -> Option<(FaceId, DVec3)> {
    face_under_in(&Scenery::single(model), camera, cursor).map(|(_, f, p)| (f, p))
}

/// The nearest face under the cursor among everything `scenery` holds: whose it is, the face
/// (an id in its own mesh) and where the cursor's ray meets it.
pub fn face_under_in(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
) -> Option<(Owner, FaceId, DVec3)> {
    nearest_face_hit(scenery, &camera.ray(cursor))
}

/// The visible edge of one mesh nearest the cursor on screen, if one lies within `radius_px`.
pub fn edge_under(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Option<EdgeId> {
    edge_under_in(&Scenery::single(model), camera, cursor, radius_px).map(|(_, e, _, _)| e)
}

/// The visible edge nearest the cursor on screen among everything `scenery` holds, if one
/// lies within `radius_px`: whose it is, the edge (an id in its own mesh) and its ends.
pub fn edge_under_in(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
    edge_under_where(scenery, camera, cursor, radius_px, |_| true)
}

/// Like `edge_under_in`, but only edges whose owner passes `keep` compete. Visibility is still
/// tested against every part, so an edge hidden behind other geometry is not found.
pub fn edge_under_where(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
    keep: impl Fn(Owner) -> bool,
) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    let candidates = Near::around(scenery, camera, cursor, radius_px)
        .segments
        .into_iter()
        .filter_map(|(part, id, a, b)| {
            let owner = scenery.part(part).owner;
            if !keep(owner) {
                return None;
            }
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            let d = projector.project(p)?.distance(cursor);
            (d <= radius_px).then_some((d, (owner, id, a, b, p)))
        })
        .collect();
    let sight = Sight::new(scenery, camera);
    nearest_visible(candidates, |&(.., p)| sight.sees(p))
        .map(|(_, (owner, id, a, b, _))| (owner, id, a, b))
}

/// The vertices and edges that may lie within `radius_px` of the cursor on screen, with the
/// index of the part each comes from and positions in the open context's coordinates, in
/// part and id order so that ties between equally near candidates always go the same way.
/// Soft edges are hidden, so they are left out: nothing snaps to them or picks them (their
/// ends are vertices of the edges around them and still snap).
struct Near {
    points: Vec<(usize, VertexId, DVec3)>,
    segments: Vec<(usize, EdgeId, DVec3, DVec3)>,
}

impl Near {
    fn around(scenery: &Scenery, camera: &Camera, cursor: DVec2, radius_px: f64) -> Near {
        let ray = camera.ray(cursor);
        // The rays through the cursor and a point `radius_px` beside it spread apart by the
        // cone's slope (perspective) or lie that far apart (parallel); doubling covers the
        // diagonal of the pixel square around the cursor.
        let beside = camera.ray(cursor + DVec2::new(radius_px, 0.0));
        let slope = 2.0 * beside.dir.distance(ray.dir);
        let r0 = 2.0 * beside.origin.distance(ray.origin);
        let mut near = Near {
            points: Vec::new(),
            segments: Vec::new(),
        };
        for (k, part) in scenery.parts().enumerate() {
            // Whatever lies near the cursor on screen lies in the cone, so a part whose box
            // misses it has nothing to offer.
            if !part.may_meet_cone(ray.origin, ray.dir, r0, slope) {
                continue;
            }
            let (origin, dir) = (
                part.back.transform_point3(ray.origin),
                part.back.transform_vector3(ray.dir),
            );
            let at = |p: DVec3| part.place.transform_point3(p);
            part.model
                .index()
                .near_ray(origin, dir, r0, slope, |s| match *s {
                    Shape::Point(v, p) => near.points.push((k, v, at(p))),
                    Shape::Segment(e, a, b) => {
                        if part.model.edge(e).is_some_and(|edge| !edge.soft) {
                            near.segments.push((k, e, at(a), at(b)));
                        }
                    }
                    Shape::Triangle(..) => {}
                });
        }
        near.points.sort_by_key(|(k, v, _)| (*k, *v));
        near.segments.sort_by_key(|(k, e, _, _)| (*k, *e));
        near
    }
}

/// Tells which points the camera sees.
struct Sight<'s> {
    scenery: &'s Scenery<'s>,
    camera: &'s Camera,
    /// The part that last hid a point. Points near the cursor are often hidden by the same
    /// thing, so it is tried first.
    blocker: Cell<Option<usize>>,
}

impl<'s> Sight<'s> {
    fn new(scenery: &'s Scenery<'s>, camera: &'s Camera) -> Self {
        Sight {
            scenery,
            camera,
            blocker: Cell::new(None),
        }
    }

    /// True if no face lies between where the camera's sight of `p` starts (the eye, in
    /// perspective) and `p` (faces through `p` itself do not hide it).
    fn sees(&self, p: DVec3) -> bool {
        let eye = self.camera.sight_origin(p);
        let to = p - eye;
        let dist = to.length();
        if dist < 1e-12 {
            return true;
        }
        let dir = to / dist;
        let reach = dist * (1.0 - 1e-6);
        let hides = |part: Part| {
            part.may_meet_ray(eye, dir, reach)
                && part
                    .model
                    .index()
                    .ray_hit(
                        part.back.transform_point3(eye),
                        part.back.transform_vector3(dir),
                        0.0,
                        reach,
                    )
                    .is_some()
        };
        if let Some(k) = self.blocker.get()
            && hides(self.scenery.part(k))
        {
            return false;
        }
        match self.scenery.parts().position(hides) {
            Some(k) => {
                self.blocker.set(Some(k));
                false
            }
            None => true,
        }
    }
}

/// Point on the line `origin + t * dir` closest to the ray; `t` is clamped to [0, 1] when
/// `segment` is true (dir is then the segment vector).
pub(crate) fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segment: bool) -> DVec3 {
    let w0 = origin - ray.origin;
    let a = dir.dot(dir);
    let b = dir.dot(ray.dir);
    let c = ray.dir.dot(ray.dir);
    let d = dir.dot(w0);
    let e = ray.dir.dot(w0);
    let denom = a * c - b * b;
    let mut t = if denom.abs() < 1e-12 * a * c {
        0.0
    } else {
        (b * e - c * d) / denom
    };
    if segment {
        t = t.clamp(0.0, 1.0);
    }
    origin + dir * t
}

/// The nearest face the ray meets in front of its origin: whose it is, the face, and where.
fn nearest_face_hit(scenery: &Scenery, ray: &Ray) -> Option<(Owner, FaceId, DVec3)> {
    let mut best: Option<(f64, Owner, FaceId)> = None;
    for part in scenery.parts() {
        let reach = best.map_or(f64::INFINITY, |(t, _, _)| t);
        if !part.may_meet_ray(ray.origin, ray.dir, reach) {
            continue;
        }
        if let Some((f, t)) = part.model.index().ray_hit(
            part.back.transform_point3(ray.origin),
            part.back.transform_vector3(ray.dir),
            0.0,
            reach,
        ) && best.is_none_or(|(bt, _, _)| t < bt)
        {
            best = Some((t, part.owner, f));
        }
    }
    best.map(|(t, owner, f)| (owner, f, ray.origin + ray.dir * t))
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 10.0;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    fn px(c: &Camera, p: DVec3) -> DVec2 {
        c.project(p).unwrap()
    }

    fn square(m: &mut Model, z: f64) {
        let c = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, z), DVec3::new(b.0, b.1, z))
                .unwrap();
        }
    }

    #[test]
    fn snaps_to_a_nearby_endpoint_exactly() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(3.0, 1.0, 0.0)) + DVec2::new(4.0, -3.0),
            None,
            R,
        );
        assert_eq!(
            hit,
            Inference {
                point: DVec3::new(3.0, 1.0, 0.0),
                kind: SnapKind::Endpoint,
                face: None,
            }
        );
    }

    #[test]
    fn snaps_to_midpoints_and_the_origin() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let mid = infer(&m, &c, px(&c, DVec3::new(2.0, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert!(mid.point.abs_diff_eq(DVec3::new(2.0, 1.0, 0.0), 1e-12));
        let origin = infer(&m, &c, px(&c, DVec3::ZERO) + DVec2::new(2.0, 2.0), None, R);
        assert_eq!(origin.kind, SnapKind::Origin);
    }

    #[test]
    fn snaps_onto_an_edge_between_its_points() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(5.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(4.2, 1.0, 0.0)) + DVec2::new(0.0, 3.0),
            None,
            R,
        );
        assert_eq!(hit.kind, SnapKind::OnEdge);
        assert!((hit.point.y - 1.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
        assert!((hit.point.x - 4.2).abs() < 0.1);
    }

    #[test]
    fn snaps_where_edges_cross_on_screen_to_the_nearer_edge() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(0.0, 1.0, 0.0), DVec3::new(4.0, 1.0, 0.0))
            .unwrap();
        // A second edge hovering between the eye and the first, across it, crossing it on
        // screen away from both midpoints.
        let behind = DVec3::new(2.7, 1.0, 0.0);
        let front = behind + (c.eye - behind) * 0.3;
        let across = DVec3::Z.cross(c.eye - behind).normalize() + DVec3::Z;
        m.add_edge(front - across * 0.5, front + across * 1.5)
            .unwrap();
        let hit = infer(&m, &c, px(&c, behind) + DVec2::new(1.0, 2.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(front) < 1e-6, "{:?}", hit.point);
        // Next to an end, the endpoint wins.
        let end = infer(&m, &c, px(&c, DVec3::new(4.0, 1.0, 0.0)), None, R);
        assert_eq!(end.kind, SnapKind::Endpoint);
    }

    #[test]
    fn snaps_find_their_edge_among_many() {
        let (c, mut m) = (cam(), Model::new());
        for i in -10..10 {
            for j in -10..10 {
                let a = DVec3::new(i as f64, j as f64, 0.0);
                m.add_edge(a, a + DVec3::new(0.4, 0.0, 0.0)).unwrap();
            }
        }
        let end = DVec3::new(2.4, 1.0, 0.0);
        let hit = infer(&m, &c, px(&c, end) + DVec2::new(2.0, -1.0), None, R);
        assert_eq!((hit.point, hit.kind), (end, SnapKind::Endpoint));
        let mid = infer(&m, &c, px(&c, DVec3::new(2.2, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        let e = edge_under(&m, &c, px(&c, DVec3::new(2.1, 1.0, 0.0)), R).unwrap();
        let ends = m.edge(e).unwrap().v.map(|v| m.position(v).unwrap());
        assert_eq!(ends, [DVec3::new(2.0, 1.0, 0.0), end]);
    }

    #[test]
    fn follows_an_axis_from_the_anchor() {
        let c = cam();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let target = DVec3::new(1.0, 1.0, 2.5); // straight up (blue)
        let hit = infer(
            &Model::new(),
            &c,
            px(&c, target) + DVec2::new(5.0, 0.0),
            Some(anchor),
            R,
        );
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
        assert!(hit.point.x == 1.0 && hit.point.y == 1.0);
        assert!((hit.point.z - 2.5).abs() < 0.2);
    }

    #[test]
    fn points_beat_axes() {
        let (c, mut m) = (cam(), Model::new());
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        m.add_edge(DVec3::new(2.0, 3.0, 0.0), DVec3::new(2.0, 5.0, 0.0))
            .unwrap();
        let hit = infer(&m, &c, px(&c, DVec3::new(2.0, 3.0, 0.0)), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
    }

    #[test]
    fn lands_on_faces_then_ground_then_free() {
        let (c, mut m) = (cam(), Model::new());
        square(&mut m, 1.0);
        let on_face = infer(&m, &c, px(&c, DVec3::new(1.3, 0.7, 1.0)), None, R);
        assert_eq!(on_face.kind, SnapKind::OnFace);
        assert!((on_face.point.z - 1.0).abs() < 1e-9);

        let ground = infer(
            &Model::new(),
            &c,
            px(&c, DVec3::new(-3.0, 4.0, 0.0)),
            None,
            R,
        );
        assert_eq!(ground.kind, SnapKind::Ground);
        assert!(ground.point.z.abs() < 1e-9);

        let mut level = cam();
        level.eye = DVec3::new(0.0, -10.0, 1.0);
        level.target = DVec3::new(0.0, 0.0, 1.0);
        let sky = infer(&Model::new(), &level, DVec2::new(400.0, 0.0), None, R);
        assert_eq!(sky.kind, SnapKind::Free);
    }

    /// A parallel view from the front (looking along +Y), zoomed in so the eye is half a
    /// meter in front of the target.
    fn parallel_front() -> Camera {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        c.target = DVec3::new(1.0, 0.0, 1.0);
        c.eye = DVec3::new(1.0, -0.5, 1.0);
        c
    }

    #[test]
    fn a_parallel_view_lands_on_the_ground() {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        c.look_from(session::StandardView::Top);
        let at = DVec3::new(-3.0, 4.0, 0.0);
        let hit = infer(&Model::new(), &c, px(&c, at), None, R);
        assert_eq!(hit.kind, SnapKind::Ground);
        assert!(hit.point.abs_diff_eq(at, 1e-9), "{}", hit.point);
    }

    #[test]
    fn a_parallel_view_sees_and_hides_with_what_is_behind_the_eye() {
        let c = parallel_front();
        let mut m = Model::new();
        // A wall 2 m in front of the target: behind the eye, but in view.
        let w = [(-1.0, -1.0), (3.0, -1.0), (3.0, 3.0), (-1.0, 3.0)];
        for i in 0..4 {
            let (a, b) = (w[i], w[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, -2.0, a.1), DVec3::new(b.0, -2.0, b.1))
                .unwrap();
        }
        // An edge behind the target, hidden by the wall.
        let end = DVec3::new(0.5, 1.0, 0.5);
        m.add_edge(end, DVec3::new(1.5, 1.0, 0.5)).unwrap();
        let hit = infer(&m, &c, px(&c, end), None, R);
        assert_eq!(hit.kind, SnapKind::OnFace, "{hit:?}");
        assert!((hit.point.y + 2.0).abs() < 1e-9);
        // The wall's own corner, behind the eye, snaps (seen 6 m wide).
        let corner = DVec3::new(3.0, -2.0, 3.0);
        let mut wide = c.clone();
        wide.eye = DVec3::new(1.0, -1.5, 1.0);
        wide.target = DVec3::new(1.0, 4.5, 1.0);
        let hit = infer(&m, &wide, px(&wide, corner) + DVec2::new(3.0, 2.0), None, R);
        assert_eq!((hit.kind, hit.point), (SnapKind::Endpoint, corner));
    }

    #[test]
    fn parallel_snaps_reach_as_far_on_screen_as_perspective_ones() {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        c.look_from(session::StandardView::Top);
        // A millimeter edge: the index's box around it is far smaller than the snap radius.
        let mut m = Model::new();
        let end = DVec3::new(2.001, 1.0, 0.0);
        m.add_edge(DVec3::new(2.0, 1.0, 0.0), end).unwrap();
        let hit = infer(&m, &c, px(&c, end) + DVec2::new(6.0, -5.0), None, R);
        assert_eq!((hit.point, hit.kind), (end, SnapKind::Endpoint));
        let e = edge_under(&m, &c, px(&c, end) + DVec2::new(0.0, 6.0), R);
        assert!(e.is_some());
    }

    #[test]
    fn inner_face_inside_another_faces_hole_is_hit() {
        let (c, mut m) = (cam(), Model::new());
        let pts = [(-2.0, -2.0), (4.0, -2.0), (4.0, 4.0), (-2.0, 4.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, 1.0), DVec3::new(b.0, b.1, 1.0))
                .unwrap();
        }
        square(&mut m, 1.0); // inner square (0,0)-(2,2): a face and a hole of the outer face
        let inside_inner = infer(&m, &c, px(&c, DVec3::new(1.2, 0.8, 1.0)), None, R);
        assert_eq!(inside_inner.kind, SnapKind::OnFace);
        assert!((inside_inner.point.z - 1.0).abs() < 1e-9);
    }

    /// A 2 m box on the ground at the origin, built through push/pull.
    fn cube() -> Model {
        let mut m = Model::new();
        let pts = [
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
        ];
        m.add_polygon(&pts).unwrap();
        let base = m.faces().next().unwrap().0;
        m.push_pull(base, 2.0).unwrap();
        m
    }

    #[test]
    fn hidden_corners_are_not_snapped() {
        let (c, m) = (cam(), cube());
        // The camera looks from (-8, -12, 7): the far bottom corner is behind the box.
        let hidden = DVec3::new(2.0, 2.0, 0.0);
        let hit = infer(&m, &c, px(&c, hidden), None, R);
        assert_ne!(hit.kind, SnapKind::Endpoint);
        assert_eq!(hit.kind, SnapKind::OnFace);
        assert!(hit.face.is_some());

        let near_top = DVec3::new(2.0, 2.0, 2.0);
        let hit = infer(&m, &c, px(&c, near_top), None, R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
        assert_eq!(hit.point, near_top);
    }

    #[test]
    fn hidden_edges_make_no_intersections() {
        let (c, m) = (cam(), cube());
        // The hidden back edge (2,2,z) crosses the visible top-front edge on screen.
        let front = (DVec3::new(0.0, 0.0, 2.0), DVec3::new(2.0, 0.0, 2.0));
        let back = (DVec3::new(2.0, 2.0, 0.0), DVec3::new(2.0, 2.0, 2.0));
        let (sa, sb) = (px(&c, front.0), px(&c, front.1));
        let (sc, sd) = (px(&c, back.0), px(&c, back.1));
        let (r, q) = (sb - sa, sd - sc);
        let t = (sc - sa).perp_dot(q) / r.perp_dot(q);
        assert!(t > 0.1 && t < 0.4, "{t}");
        let hit = infer(&m, &c, sa + r * t, None, R);
        assert_eq!(hit.kind, SnapKind::OnEdge, "{:?}", hit.point);
        assert!((hit.point.y).abs() < 1e-9 && (hit.point.z - 2.0).abs() < 1e-9);
    }

    #[test]
    fn reports_the_face_under_the_cursor() {
        let (c, m) = (cam(), cube());
        let top = m
            .faces()
            .find(|(_, f)| f.normal == DVec3::Z)
            .map(|(id, _)| id)
            .unwrap();
        let cursor = px(&c, DVec3::new(1.0, 1.2, 2.0));
        let hit = infer(&m, &c, cursor, None, R);
        assert_eq!((hit.kind, hit.face), (SnapKind::OnFace, Some(top)));
        let (face, point) = face_under(&m, &c, cursor).unwrap();
        assert_eq!(face, top);
        assert!((point.z - 2.0).abs() < 1e-9);
        assert!(face_under(&m, &c, px(&c, DVec3::new(-3.0, 4.0, 0.0))).is_none());
    }

    #[test]
    fn axis_guides_show_through_faces() {
        let (c, m) = (cam(), cube());
        let anchor = DVec3::new(3.0, 3.0, 0.0);
        let behind = DVec3::new(3.0, 3.0, 1.0);
        let hit = infer(&m, &c, px(&c, behind), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
    }

    #[test]
    fn edge_under_picks_the_nearest_visible_edge() {
        let (c, m) = (cam(), cube());
        let edge_at = |a: DVec3, b: DVec3| {
            m.edges()
                .find(|(_, e)| {
                    let (p, q) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
                    (p == a && q == b) || (p == b && q == a)
                })
                .unwrap()
                .0
        };
        let near_top = edge_at(DVec3::new(0.0, 0.0, 2.0), DVec3::new(2.0, 0.0, 2.0));
        let cursor = px(&c, DVec3::new(1.0, 0.0, 2.0)) + DVec2::new(0.0, 3.0);
        assert_eq!(edge_under(&m, &c, cursor, R), Some(near_top));
        // The far bottom edge lies behind the box.
        let far_bottom = edge_at(DVec3::new(0.0, 2.0, 0.0), DVec3::new(2.0, 2.0, 0.0));
        let behind = px(&c, DVec3::new(1.0, 2.0, 0.0));
        assert_ne!(edge_under(&m, &c, behind, R), Some(far_bottom));
        assert_eq!(
            edge_under(&m, &c, px(&c, DVec3::new(-3.0, 4.0, 0.0)), R),
            None
        );
    }

    fn guides(anchor: DVec3, reference: Option<DVec3>, lock: Option<Lock>) -> Guides {
        Guides {
            anchor: Some(anchor),
            reference,
            lock,
        }
    }

    #[test]
    fn follows_parallel_and_perpendicular_to_the_reference() {
        let c = cam();
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        let r = DVec3::new(2.0, 1.0, 0.0).normalize();
        let g = guides(anchor, Some(r), None);
        let along = anchor + r * 2.0;
        let hit = infer_with(&Model::new(), &c, px(&c, along), &g, R);
        assert_eq!(hit.kind, SnapKind::Parallel);
        assert!(hit.point.abs_diff_eq(along, 1e-6));
        let across = anchor + DVec3::new(1.0, -2.0, 0.0).normalize() * 2.0;
        let hit = infer_with(&Model::new(), &c, px(&c, across), &g, R);
        assert_eq!(hit.kind, SnapKind::Perpendicular);
        assert!(hit.point.abs_diff_eq(across, 1e-6));
    }

    #[test]
    fn a_reference_along_an_axis_adds_nothing() {
        assert_eq!(directions(Some(DVec3::new(0.0, 3.0, 0.0))).len(), 3);
        let tilted = directions(Some(DVec3::new(1.0, 1.0, 0.0)));
        // Parallel and the horizontal perpendicular; the other perpendicular is vertical (Z).
        assert_eq!(tilted.len(), 5);
        assert_eq!(directions(None).len(), 3);
    }

    #[test]
    fn a_lock_holds_the_point_on_its_line() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(3.0, 3.0, 2.0), DVec3::new(4.0, 3.0, 2.0))
            .unwrap();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let lock = Lock {
            dir: DVec3::Z,
            kind: SnapKind::Axis(Axis::Z),
        };
        let g = guides(anchor, None, Some(lock));
        let far = infer_with(&m, &c, px(&c, DVec3::new(-4.0, 5.0, 0.0)), &g, R);
        assert_eq!(far.kind, SnapKind::Axis(Axis::Z));
        assert!(far.point.x == 1.0 && far.point.y == 1.0);
        // An endpoint under the cursor sets the height.
        let snapped = infer_with(&m, &c, px(&c, DVec3::new(3.0, 3.0, 2.0)), &g, R);
        assert_eq!(snapped.point, DVec3::new(1.0, 1.0, 2.0));
    }

    fn with_guides() -> Model {
        let mut m = Model::new();
        m.add_guide(Guide::Point {
            position: DVec3::new(2.0, 1.0, 0.0),
        })
        .unwrap();
        m.add_guide(Guide::Line {
            point: DVec3::new(0.0, 3.0, 0.0),
            direction: DVec3::X,
        })
        .unwrap();
        m.add_guide(Guide::Line {
            point: DVec3::new(5.0, 0.0, 0.0),
            direction: DVec3::Y,
        })
        .unwrap();
        m
    }

    #[test]
    fn snaps_to_guide_points_and_onto_guide_lines() {
        let (c, m) = (cam(), with_guides());
        let point = DVec3::new(2.0, 1.0, 0.0);
        let hit = infer(&m, &c, px(&c, point) + DVec2::new(3.0, -2.0), None, R);
        assert_eq!((hit.point, hit.kind), (point, SnapKind::GuidePoint));
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(2.6, 3.0, 0.0)) + DVec2::new(0.0, 3.0),
            None,
            R,
        );
        assert_eq!(hit.kind, SnapKind::OnGuide);
        assert!((hit.point.y - 3.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
        assert!((hit.point.x - 2.6).abs() < 0.1);
    }

    #[test]
    fn guide_lines_cross_each_other_and_edges() {
        let (c, mut m) = (cam(), with_guides());
        let both = DVec3::new(5.0, 3.0, 0.0);
        let hit = infer(&m, &c, px(&c, both) + DVec2::new(2.0, 1.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(both) < 1e-6, "{:?}", hit.point);
        m.add_edge(DVec3::new(1.0, -3.0, 0.0), DVec3::new(1.0, 7.0, 0.0))
            .unwrap();
        let cross = DVec3::new(1.0, 3.0, 0.0);
        let hit = infer(&m, &c, px(&c, cross) + DVec2::new(-1.0, 2.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(cross) < 1e-6, "{:?}", hit.point);
        // Along the edge away from the guide, the edge wins over nothing.
        let hit = infer(&m, &c, px(&c, DVec3::new(1.0, 0.5, 0.0)), None, R);
        assert_eq!(hit.kind, SnapKind::OnEdge);
    }

    #[test]
    fn finds_the_guide_under_the_cursor() {
        let (c, m) = (cam(), with_guides());
        let scenery = Scenery::single(&m);
        let ids: Vec<GuideId> = m.guides().map(|(id, _)| id).collect();
        let under = |p: DVec3| {
            guide_under_where(&scenery, &c, px(&c, p) + DVec2::new(2.0, 2.0), R, |_| true)
                .map(|(_, id, _)| id)
        };
        assert_eq!(under(DVec3::new(2.0, 1.0, 0.0)), Some(ids[0]));
        assert_eq!(under(DVec3::new(-2.0, 3.0, 0.0)), Some(ids[1]));
        assert_eq!(under(DVec3::new(5.0, -4.0, 0.0)), Some(ids[2]));
        assert_eq!(under(DVec3::new(-3.0, -3.0, 0.0)), None);
        // Guides behind a face are not found.
        let mut hidden = with_guides();
        let lid = [(-9.0, -9.0), (9.0, -9.0), (9.0, 9.0), (-9.0, 9.0)]
            .map(|(x, y)| DVec3::new(x, y, 1.0));
        hidden.add_polygon(&lid).unwrap();
        let scenery = Scenery::single(&hidden);
        let p = px(&c, DVec3::new(2.0, 1.0, 0.0));
        assert!(guide_under_where(&scenery, &c, p, R, |_| true).is_none());
    }

    /// A scene whose only instance, a group holding a unit square at the origin, also holds a
    /// guide point at (3, 1, 0).
    fn scene_with_a_guide_in_a_group() -> kernel::Scene {
        let mut m = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
            .unwrap();
        let mut s = kernel::Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(
                kernel::MeshId::Root,
                &edges,
                &faces,
                &[],
                kernel::DefinitionKind::Group,
                "",
            )
            .unwrap();
        let def = s.root().instance(i).unwrap().definition;
        s.edit(kernel::MeshId::Definition(def), |m| {
            m.add_guide(Guide::Point {
                position: p(3.0, 1.0),
            })
        })
        .unwrap();
        s
    }

    #[test]
    fn a_guide_inside_a_closed_group_is_snapped_to() {
        let (c, s) = (cam(), scene_with_a_guide_in_a_group());
        let scenery = Scenery::of(&s, &[]).unwrap();
        let g = Guides::default();
        let at = DVec3::new(3.0, 1.0, 0.0);
        let hit = infer_in(&scenery, &c, px(&c, at) + DVec2::new(2.0, 2.0), &g, R);
        assert_eq!((hit.point, hit.kind), (at, SnapKind::GuidePoint));
    }

    /// A deterministic stream of numbers in [0, 1).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn within(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }
    }

    /// Adds a box `size` wide with a corner at `at` to a mesh of `s`.
    fn add_box(s: &mut kernel::Scene, mesh: kernel::MeshId, at: DVec3, size: f64) {
        s.edit(mesh, |m| {
            let p = |x: f64, y: f64| at + DVec3::new(x, y, 0.0) * size;
            let mut c = m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])?;
            let f = m.faces().map(|(id, _)| id).max().unwrap();
            c.ops.extend(m.push_pull(f, size)?.ops);
            Ok(c)
        })
        .unwrap();
    }

    /// A crowd of placed boxes: a component (a box, a nested group with a smaller box, guides)
    /// copied 28 times turned, moved, mirrored and scaled, some of the copies gathered in a
    /// group, over a ground face, with crossing loose edges and a guide line. Returns the
    /// scene and the open contexts to look from: the root, the group, the nested group in a
    /// component in the group, and a component at the root.
    fn crowd() -> (kernel::Scene, Vec<Vec<kernel::InstanceId>>) {
        use kernel::{DefinitionKind, MeshId};
        let mut s = kernel::Scene::new();
        add_box(&mut s, MeshId::Root, DVec3::ZERO, 0.5);
        let all = |s: &kernel::Scene, mesh: MeshId| {
            let m = s.mesh(mesh).unwrap();
            let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
            let faces: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
            (edges, faces)
        };
        let (edges, faces) = all(&s, MeshId::Root);
        let (comp, _) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Component,
                "C",
            )
            .unwrap();
        let def = MeshId::Definition(s.root().instance(comp).unwrap().definition);
        let (old_edges, old_faces) = all(&s, def);
        add_box(&mut s, def, DVec3::new(0.1, 0.15, 0.5), 0.2);
        let (edges, faces) = all(&s, def);
        let edges: Vec<EdgeId> = edges
            .into_iter()
            .filter(|e| !old_edges.contains(e))
            .collect();
        let faces: Vec<FaceId> = faces
            .into_iter()
            .filter(|f| !old_faces.contains(f))
            .collect();
        let (nested, _) = s
            .make_group(def, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(def, |m| {
            let mut c = m.add_guide(Guide::Point {
                position: DVec3::new(0.25, 0.25, 0.9),
            })?;
            c.ops.extend(
                m.add_guide(Guide::Line {
                    point: DVec3::new(0.0, 0.5, 0.25),
                    direction: DVec3::X,
                })?
                .ops,
            );
            Ok(c)
        })
        .unwrap();

        let mut rng = Lcg(7);
        let places: Vec<glam::DAffine3> = (0..28)
            .map(|k| {
                let axis = DVec3::new(rng.within(-1.0, 1.0), rng.within(-1.0, 1.0), rng.next())
                    .normalize();
                let turn = glam::DQuat::from_axis_angle(axis, rng.within(-3.0, 3.0));
                let scale = match k % 7 {
                    0 => DVec3::new(-1.0, 1.0, 1.0),
                    3 => DVec3::splat(rng.within(0.5, 2.0)),
                    5 => DVec3::new(1.0, 1.5, 0.7),
                    _ => DVec3::ONE,
                };
                let at = DVec3::new(
                    rng.within(-5.0, 5.0),
                    rng.within(-5.0, 5.0),
                    rng.within(0.0, 2.0),
                );
                glam::DAffine3::from_scale_rotation_translation(scale, turn, at)
            })
            .collect();
        s.copy_instances_placed(MeshId::Root, &[comp], &places)
            .unwrap();
        let copies: Vec<kernel::InstanceId> = s
            .root()
            .instances()
            .map(|(id, _)| id)
            .filter(|id| *id != comp)
            .collect();
        let (group, _) = s
            .make_group(
                MeshId::Root,
                &[],
                &[],
                &copies[..8],
                DefinitionKind::Group,
                "",
            )
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_polygon(&[
                DVec3::new(-6.0, -6.0, 0.0),
                DVec3::new(6.0, -6.0, 0.0),
                DVec3::new(6.0, 6.0, 0.0),
                DVec3::new(-6.0, 6.0, 0.0),
            ])?;
            for k in 0..6 {
                let y = -3.0 + k as f64;
                c.ops.extend(
                    m.add_edge(DVec3::new(-4.0, y, 1.0), DVec3::new(4.0, y + 1.5, 2.5))?
                        .ops,
                );
                c.ops.extend(
                    m.add_edge(DVec3::new(y, -4.0, 1.5), DVec3::new(y + 0.5, 4.0, 0.5))?
                        .ops,
                );
            }
            c.ops.extend(
                m.add_guide(Guide::Line {
                    point: DVec3::new(0.0, 0.0, 0.3),
                    direction: DVec3::new(1.0, 1.0, 0.0).normalize(),
                })?
                .ops,
            );
            Ok(c)
        })
        .unwrap();
        let group_mesh = MeshId::Definition(s.root().instance(group).unwrap().definition);
        let inner = s.mesh(group_mesh).unwrap().instances().next().unwrap().0;
        let contexts = vec![
            vec![],
            vec![group],
            vec![group, inner, nested],
            vec![copies[20]],
        ];
        (s, contexts)
    }

    /// Cameras in root coordinates: perspective and parallel, from around and above.
    fn views() -> Vec<Camera> {
        let mut out = Vec::new();
        for (eye, target) in [
            (DVec3::new(-8.0, -12.0, 7.0), DVec3::ZERO),
            (DVec3::new(0.5, -2.0, 9.0), DVec3::new(0.5, 0.0, 0.0)),
            (DVec3::new(-1.0, 0.2, 1.2), DVec3::new(3.0, 1.0, 1.0)),
        ] {
            for projection in [
                session::Projection::Perspective,
                session::Projection::Parallel,
            ] {
                let mut c = Camera::new(DVec2::new(800.0, 600.0));
                c.eye = eye;
                c.target = target;
                c.projection = projection;
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn skipping_parts_by_their_boxes_changes_no_result() {
        let (s, contexts) = crowd();
        let mut kinds = std::collections::BTreeSet::new();
        let mut skipped = 0;
        let mut checked = 0;
        for context in &contexts {
            let culled = Scenery::of(&s, context).unwrap();
            let full = culled.unculled();
            let to_root = s.resolve(context).unwrap().1;
            for view in views() {
                let c = view.placed(to_root.inverse());
                // A grid over the view, plus spots next to placed corners so points snap.
                let mut cursors: Vec<DVec2> = (0..7)
                    .flat_map(|i| {
                        (0..5).map(move |j| {
                            DVec2::new(40.0 + 120.0 * i as f64, 30.0 + 135.0 * j as f64)
                        })
                    })
                    .collect();
                for part in culled.parts().step_by(4) {
                    for (_, v) in part.model.vertices().step_by(7) {
                        if let Some(p) = c.project(part.place.transform_point3(v.pos)) {
                            cursors.push(p + DVec2::new(2.0, -1.5));
                        }
                    }
                }
                for (n, cursor) in cursors.into_iter().enumerate() {
                    let ray = c.ray(cursor);
                    skipped += culled
                        .parts()
                        .filter(|p| !p.may_meet_ray(ray.origin, ray.dir, f64::INFINITY))
                        .count();
                    checked += culled.len();
                    let anchor = c.ray(DVec2::new(400.0, 300.0)).origin
                        + c.ray(DVec2::new(400.0, 300.0)).dir * 5.0;
                    // Every other cursor follows directions from an anchor.
                    let g = Guides {
                        anchor: (n % 2 == 1).then_some(anchor),
                        reference: Some(DVec3::new(1.0, 2.0, 0.5)),
                        lock: None,
                    };
                    let hit = infer_in(&culled, &c, cursor, &g, R);
                    assert_eq!(
                        hit,
                        infer_in(&full, &c, cursor, &g, R),
                        "{context:?} {cursor}"
                    );
                    kinds.insert(format!("{:?}", hit.kind));
                    assert_eq!(
                        edge_under_in(&culled, &c, cursor, R),
                        edge_under_in(&full, &c, cursor, R)
                    );
                    let open = |o: Owner| o == Owner::Open;
                    assert_eq!(
                        edge_under_where(&culled, &c, cursor, R, open),
                        edge_under_where(&full, &c, cursor, R, open)
                    );
                    assert_eq!(
                        face_under_in(&culled, &c, cursor),
                        face_under_in(&full, &c, cursor)
                    );
                    assert_eq!(
                        guide_under_where(&culled, &c, cursor, R, |_| true),
                        guide_under_where(&full, &c, cursor, R, |_| true)
                    );
                }
            }
        }
        // The check means something only if parts were skipped and many kinds of snap came up.
        assert!(skipped * 2 > checked, "{skipped} of {checked} skipped");
        for kind in [
            "Endpoint",
            "Midpoint",
            "OnEdge",
            "OnFace",
            "Intersection",
            "GuidePoint",
            "OnGuide",
        ] {
            assert!(kinds.contains(kind), "no {kind} in {kinds:?}");
        }
    }
}
