//! Snapping the cursor to meaningful 3D points.

use glam::{DVec2, DVec3};
use kernel::{EdgeId, FaceId, Guide, GuideId, Model, Shape, VertexId};
use session::{Camera, Ray};

use crate::scenery::{Owner, Scenery};

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
    let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
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

    match ray.hit_ground() {
        Some(point) if point.distance(ray.origin) <= camera.distance() * 50.0 => Inference {
            point,
            kind: SnapKind::Ground,
            face,
        },
        _ => Inference {
            point: camera.anchor_at(cursor),
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
    let within = |p: DVec3| {
        camera
            .project(p)
            .map(|s| s.distance(cursor))
            .filter(|d| *d <= radius_px)
            .filter(|_| visible(scenery, camera.eye, p))
    };
    let near = Near::around(scenery, camera, cursor, radius_px);
    let mut best: Option<(f64, DVec3, SnapKind)> = None;
    let mut offer = |p: DVec3, kind: SnapKind| {
        if let Some(d) = within(p)
            && best.is_none_or(|(bd, _, _)| d < bd)
        {
            best = Some((d, p, kind));
        }
    };
    for &(_, _, p) in &near.points {
        offer(p, SnapKind::Endpoint);
    }
    for &(_, _, a, b) in &near.segments {
        offer((a + b) * 0.5, SnapKind::Midpoint);
    }
    offer(DVec3::ZERO, SnapKind::Origin);
    let guides = placed_guides(scenery);
    for &(_, _, g) in &guides {
        if let Guide::Point { position } = g {
            offer(position, SnapKind::GuidePoint);
        }
    }
    if let Some((_, p, kind)) = best {
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
            camera
                .project(p)
                .is_some_and(|s| s.distance(cursor) <= radius_px)
        })
        .collect();
    let lines: Vec<(DVec3, DVec3)> = near
        .segments
        .iter()
        .map(|&(_, _, a, b)| (a, b))
        .chain(
            guide_lines
                .iter()
                .filter_map(|&(point, dir)| camera.line_span(point, dir))
                .map(|[a, b]| (a, b)),
        )
        .collect();
    let mut crossing: Option<(f64, DVec3)> = None;
    for (i, &(a, b)) in lines.iter().enumerate() {
        for &(c, d) in &lines[i + 1..] {
            if let Some((p, far)) = apparent_crossing(camera, (a, b), (c, d))
                && visible(scenery, camera.eye, far)
                && let Some(dist) = within(p)
                && crossing.is_none_or(|(bd, _)| dist < bd)
            {
                crossing = Some((dist, p));
            }
        }
    }
    if let Some((_, p)) = crossing {
        return Some((p, SnapKind::Intersection));
    }

    let mut on_edge: Option<(f64, DVec3)> = None;
    for &(_, _, a, b) in &near.segments {
        let p = closest_on_line_to_ray(a, b - a, &ray, true);
        if let Some(d) = within(p)
            && on_edge.is_none_or(|(bd, _)| d < bd)
        {
            on_edge = Some((d, p));
        }
    }
    if let Some((_, p)) = on_edge {
        return Some((p, SnapKind::OnEdge));
    }

    let mut on_guide: Option<(f64, DVec3)> = None;
    for &(point, dir) in &guide_lines {
        let p = closest_on_line_to_ray(point, dir, &ray, false);
        if let Some(d) = within(p)
            && on_guide.is_none_or(|(bd, _)| d < bd)
        {
            on_guide = Some((d, p));
        }
    }
    on_guide.map(|(_, p)| (p, SnapKind::OnGuide))
}

/// Every guide of every part, placed in the open context's coordinates, with the index of the
/// part it comes from, in part and id order. Guides are few, so they are scanned, not indexed.
fn placed_guides(scenery: &Scenery) -> Vec<(usize, GuideId, Guide)> {
    scenery
        .parts
        .iter()
        .enumerate()
        .flat_map(|(k, part)| {
            part.model
                .guides()
                .map(move |(id, g)| (k, id, g.transformed(part.place)))
        })
        .collect()
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
    let mut best: Option<(f64, (Owner, GuideId, Guide))> = None;
    for (part, id, g) in placed_guides(scenery) {
        let owner = scenery.parts[part].owner;
        if !keep(owner) {
            continue;
        }
        let p = match g {
            Guide::Point { position } => position,
            Guide::Line { point, direction } => {
                closest_on_line_to_ray(point, direction, &ray, false)
            }
        };
        let Some(d) = camera.project(p).map(|s| s.distance(cursor)) else {
            continue;
        };
        if d <= radius_px && best.is_none_or(|(bd, _)| d < bd) && visible(scenery, camera.eye, p) {
            best = Some((d, (owner, id, g)));
        }
    }
    best.map(|(_, hit)| hit)
}

/// Where segments a-b and c-d cross on screen away from their ends: the points under the
/// crossing on the nearer and the farther segment, nearer first. Edges that meet in space share
/// a vertex (the kernel splits crossing edges), so their crossing is an endpoint, not this.
fn apparent_crossing(
    camera: &Camera,
    (a, b): (DVec3, DVec3),
    (c, d): (DVec3, DVec3),
) -> Option<(DVec3, DVec3)> {
    let (sa, sb) = (camera.project(a)?, camera.project(b)?);
    let (sc, sd) = (camera.project(c)?, camera.project(d)?);
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
    let mut best: Option<(f64, (Owner, EdgeId, DVec3, DVec3))> = None;
    for (part, id, a, b) in Near::around(scenery, camera, cursor, radius_px).segments {
        let owner = scenery.parts[part].owner;
        if !keep(owner) {
            continue;
        }
        let p = closest_on_line_to_ray(a, b - a, &ray, true);
        let Some(d) = camera.project(p).map(|s| s.distance(cursor)) else {
            continue;
        };
        if d <= radius_px && best.is_none_or(|(bd, _)| d < bd) && visible(scenery, camera.eye, p) {
            best = Some((d, (owner, id, a, b)));
        }
    }
    best.map(|(_, hit)| hit)
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
        // cone's slope; doubling it covers the diagonal of the pixel square around the cursor.
        let beside = camera.ray(cursor + DVec2::new(radius_px, 0.0));
        let slope = 2.0 * beside.dir.distance(ray.dir);
        let mut near = Near {
            points: Vec::new(),
            segments: Vec::new(),
        };
        for (k, part) in scenery.parts.iter().enumerate() {
            let back = part.place.inverse();
            let (origin, dir) = (
                back.transform_point3(ray.origin),
                back.transform_vector3(ray.dir),
            );
            let at = |p: DVec3| part.place.transform_point3(p);
            part.model
                .index()
                .near_ray(origin, dir, 0.0, slope, |s| match *s {
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

/// True if no face lies between the eye and `p` (faces through `p` itself do not hide it).
fn visible(scenery: &Scenery, eye: DVec3, p: DVec3) -> bool {
    let to = p - eye;
    let dist = to.length();
    if dist < 1e-12 {
        return true;
    }
    let dir = to / dist;
    scenery.parts.iter().all(|part| {
        let back = part.place.inverse();
        part.model
            .index()
            .ray_hit(
                back.transform_point3(eye),
                back.transform_vector3(dir),
                0.0,
                dist * (1.0 - 1e-6),
            )
            .is_none()
    })
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
    for part in &scenery.parts {
        let back = part.place.inverse();
        let reach = best.map_or(f64::INFINITY, |(t, _, _)| t);
        if let Some((f, t)) = part.model.index().ray_hit(
            back.transform_point3(ray.origin),
            back.transform_vector3(ray.dir),
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
}
