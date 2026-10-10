//! Snapping the cursor to meaningful 3D points.

use glam::{DVec2, DVec3};
use kernel::geom::{Plane, point_in_polygon};
use kernel::{FaceId, Model};
use session::{Camera, Ray};

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
    OnEdge,
    /// Along a red/green/blue axis through the point the tool is drawing from.
    Axis(Axis),
    OnFace,
    Ground,
    /// Nothing to snap to: a point in front of the camera under the cursor.
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inference {
    pub point: DVec3,
    pub kind: SnapKind,
    /// The nearest face under the cursor, whatever the point snapped to.
    pub face: Option<FaceId>,
}

/// Finds the 3D point under `cursor` (physical pixels). `anchor` is the point a tool is drawing
/// from, enabling axis inference. Snap candidates count only within `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > on-edge > axis > on-face >
/// ground > free. Points and edges hidden behind a face are skipped.
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let ray = camera.ray(cursor);
    let face_hit = nearest_face_hit(model, &ray);
    let face = face_hit.map(|(f, _)| f);
    let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
    let within = |p: DVec3| {
        screen_dist(p)
            .filter(|d| *d <= radius_px)
            .filter(|_| visible(model, camera.eye, p))
    };

    // Points: vertices, edge midpoints, origin.
    let mut best: Option<(f64, Inference)> = None;
    let offer = |p: DVec3, kind: SnapKind, best: &mut Option<(f64, Inference)>| {
        if let Some(d) = within(p)
            && best.as_ref().is_none_or(|(bd, _)| d < *bd)
        {
            *best = Some((
                d,
                Inference {
                    point: p,
                    kind,
                    face,
                },
            ));
        }
    };
    for (_, v) in model.vertices() {
        offer(v.pos, SnapKind::Endpoint, &mut best);
    }
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            offer((a + b) * 0.5, SnapKind::Midpoint, &mut best);
        }
    }
    offer(DVec3::ZERO, SnapKind::Origin, &mut best);
    if let Some((_, hit)) = best {
        return hit;
    }

    // On an edge.
    let mut on_edge: Option<(f64, DVec3)> = None;
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            if let Some(d) = within(p)
                && on_edge.is_none_or(|(bd, _)| d < bd)
            {
                on_edge = Some((d, p));
            }
        }
    }
    if let Some((_, point)) = on_edge {
        return Inference {
            point,
            kind: SnapKind::OnEdge,
            face,
        };
    }

    // Along an axis from the anchor.
    if let Some(anchor) = anchor {
        let mut on_axis: Option<(f64, Inference)> = None;
        for axis in Axis::ALL {
            let p = closest_on_line_to_ray(anchor, axis.dir(), &ray, false);
            // Axis lines are guides, so they are not hidden by faces.
            if let Some(d) = screen_dist(p).filter(|d| *d <= radius_px)
                && p.distance(anchor) > 0.0
                && on_axis.is_none_or(|(bd, _)| d < bd)
            {
                on_axis = Some((
                    d,
                    Inference {
                        point: p,
                        kind: SnapKind::Axis(axis),
                        face,
                    },
                ));
            }
        }
        if let Some((_, hit)) = on_axis {
            return hit;
        }
    }

    // On a face (nearest along the ray).
    if let Some((_, point)) = face_hit {
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

/// The nearest face under the cursor and the point where the cursor's ray meets it.
pub fn face_under(model: &Model, camera: &Camera, cursor: DVec2) -> Option<(FaceId, DVec3)> {
    nearest_face_hit(model, &camera.ray(cursor))
}

/// True if no face lies between the eye and `p` (faces through `p` itself do not hide it).
fn visible(model: &Model, eye: DVec3, p: DVec3) -> bool {
    let to = p - eye;
    let dist = to.length();
    if dist < 1e-12 {
        return true;
    }
    let ray = Ray {
        origin: eye,
        dir: to / dist,
    };
    nearest_face_hit(model, &ray).is_none_or(|(_, q)| q.distance(eye) >= dist * (1.0 - 1e-6))
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

fn nearest_face_hit(model: &Model, ray: &Ray) -> Option<(FaceId, DVec3)> {
    let mut best: Option<(f64, FaceId, DVec3)> = None;
    for (id, f) in model.faces() {
        let Some(origin) = f.outer.first().and_then(|v| model.position(*v)) else {
            continue;
        };
        let plane = Plane {
            normal: f.normal,
            d: f.normal.dot(origin),
        };
        let denom = plane.normal.dot(ray.dir);
        if denom.abs() < 1e-12 {
            continue;
        }
        let t = -plane.distance(ray.origin) / denom;
        if t <= 0.0 || best.is_some_and(|(bt, _, _)| t >= bt) {
            continue;
        }
        let p = ray.origin + ray.dir * t;
        let project = |l: &Vec<kernel::VertexId>| -> Vec<DVec2> {
            l.iter()
                .filter_map(|v| model.position(*v))
                .map(|q| plane.project(q))
                .collect()
        };
        let q = plane.project(p);
        if point_in_polygon(q, &project(&f.outer))
            && !f.holes.iter().any(|h| point_in_polygon(q, &project(h)))
        {
            best = Some((t, id, p));
        }
    }
    best.map(|(_, id, p)| (id, p))
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
}
