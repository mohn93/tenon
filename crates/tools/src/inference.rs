//! Snapping the cursor to meaningful 3D points.

use glam::{DVec2, DVec3};
use kernel::Model;
use kernel::geom::{Plane, point_in_polygon};
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
}

/// Finds the 3D point under `cursor` (physical pixels). `anchor` is the point a tool is drawing
/// from, enabling axis inference. Snap candidates count only within `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > on-edge > axis > on-face >
/// ground > free.
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let ray = camera.ray(cursor);
    let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
    let within = |p: DVec3| screen_dist(p).filter(|d| *d <= radius_px);

    // Points: vertices, edge midpoints, origin.
    let mut best: Option<(f64, Inference)> = None;
    let offer = |p: DVec3, kind: SnapKind, best: &mut Option<(f64, Inference)>| {
        if let Some(d) = within(p)
            && best.as_ref().is_none_or(|(bd, _)| d < *bd)
        {
            *best = Some((d, Inference { point: p, kind }));
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
        };
    }

    // Along an axis from the anchor.
    if let Some(anchor) = anchor {
        let mut on_axis: Option<(f64, Inference)> = None;
        for axis in Axis::ALL {
            let p = closest_on_line_to_ray(anchor, axis.dir(), &ray, false);
            if let Some(d) = within(p)
                && p.distance(anchor) > 0.0
                && on_axis.is_none_or(|(bd, _)| d < bd)
            {
                on_axis = Some((
                    d,
                    Inference {
                        point: p,
                        kind: SnapKind::Axis(axis),
                    },
                ));
            }
        }
        if let Some((_, hit)) = on_axis {
            return hit;
        }
    }

    // On a face (nearest along the ray).
    if let Some(point) = nearest_face_hit(model, &ray) {
        return Inference {
            point,
            kind: SnapKind::OnFace,
        };
    }

    match ray.hit_ground() {
        Some(point) if point.distance(ray.origin) <= camera.distance() * 50.0 => Inference {
            point,
            kind: SnapKind::Ground,
        },
        _ => Inference {
            point: camera.anchor_at(cursor),
            kind: SnapKind::Free,
        },
    }
}

/// Point on the line `origin + t * dir` closest to the ray; `t` is clamped to [0, 1] when
/// `segment` is true (dir is then the segment vector).
fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segment: bool) -> DVec3 {
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

fn nearest_face_hit(model: &Model, ray: &Ray) -> Option<DVec3> {
    let mut best: Option<(f64, DVec3)> = None;
    for (_, f) in model.faces() {
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
        if t <= 0.0 || best.is_some_and(|(bt, _)| t >= bt) {
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
            best = Some((t, p));
        }
    }
    best.map(|(_, p)| p)
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
                kind: SnapKind::Endpoint
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
}
