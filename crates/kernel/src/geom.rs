//! Tolerances and small geometric helpers used by the kernel.

use glam::{DVec2, DVec3};

/// Points closer than this (meters) are the same point.
pub const LENGTH_TOL: f64 = 1e-6;
/// Sine of the largest angle treated as zero (parallel or collinear).
pub const ANGLE_TOL: f64 = 1e-9;

/// Plane `normal . p = d` with a unit normal in canonical orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: DVec3,
    pub d: f64,
}

impl Plane {
    /// Plane through three points, or None if they are collinear.
    pub fn from_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Plane> {
        let ab = b - a;
        let ac = c - a;
        let n = ab.cross(ac);
        if n.length() <= ANGLE_TOL * ab.length() * ac.length() {
            return None;
        }
        let normal = canonical(n.normalize());
        Some(Plane {
            normal,
            d: normal.dot(a),
        })
    }

    pub fn distance(&self, p: DVec3) -> f64 {
        self.normal.dot(p) - self.d
    }

    pub fn contains(&self, p: DVec3) -> bool {
        self.distance(p).abs() < LENGTH_TOL
    }

    pub fn same_as(&self, other: &Plane) -> bool {
        if self.normal.cross(other.normal).length() >= ANGLE_TOL * 1e3 {
            return false;
        }
        let dot = self.normal.dot(other.normal);
        (dot > 0.0 && (self.d - other.d).abs() < LENGTH_TOL)
            || (dot < 0.0 && (self.d + other.d).abs() < LENGTH_TOL)
    }

    /// Orthonormal in-plane axes (u, v) with u x v = normal.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let helper = if self.normal.x.abs() < 0.9 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let u = helper.cross(self.normal).normalize();
        let v = self.normal.cross(u);
        (u, v)
    }

    pub fn project(&self, p: DVec3) -> DVec2 {
        let (u, v) = self.basis();
        DVec2::new(p.dot(u), p.dot(v))
    }
}

/// Flip a unit normal so its largest-magnitude component is positive.
fn canonical(n: DVec3) -> DVec3 {
    let a = n.abs();
    let major = if a.x >= a.y && a.x >= a.z {
        n.x
    } else if a.y >= a.z {
        n.y
    } else {
        n.z
    };
    if major < 0.0 { -n } else { n }
}

/// Parameter t of the point on segment a-b closest to p (unclamped) and its distance.
pub fn project_on_segment(p: DVec3, a: DVec3, b: DVec3) -> (f64, f64) {
    let ab = b - a;
    let t = (p - a).dot(ab) / ab.length_squared();
    (t, (a + ab * t).distance(p))
}

/// The point where open segments p-q and a-b cross, if they cross away from all four endpoints.
pub fn segment_crossing(p: DVec3, q: DVec3, a: DVec3, b: DVec3) -> Option<DVec3> {
    let d1 = q - p;
    let d2 = b - a;
    let r = p - a;
    let a11 = d1.dot(d1);
    let a12 = d1.dot(d2);
    let a22 = d2.dot(d2);
    let c1 = d1.dot(r);
    let c2 = d2.dot(r);
    let denom = a11 * a22 - a12 * a12;
    if denom <= ANGLE_TOL * ANGLE_TOL * a11 * a22 {
        return None; // parallel
    }
    let s = (a12 * c2 - a22 * c1) / denom;
    let t = (a11 * c2 - a12 * c1) / denom;
    if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
        return None;
    }
    let x1 = p + d1 * s;
    let x2 = a + d2 * t;
    if x1.distance(x2) >= LENGTH_TOL {
        return None;
    }
    let x = (x1 + x2) * 0.5;
    let away = |e: DVec3| e.distance(x) >= LENGTH_TOL;
    (away(p) && away(q) && away(a) && away(b)).then_some(x)
}

/// Twice the signed area of a 2D polygon (positive when counter-clockwise).
pub fn signed_area2(poly: &[DVec2]) -> f64 {
    (0..poly.len())
        .map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()]))
        .sum()
}

/// Even-odd point-in-polygon test. Points on the boundary may go either way.
pub fn point_in_polygon(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Vector area of a closed 3D polygon (direction = normal by right-hand rule, length = area).
pub fn vector_area(points: &[DVec3]) -> DVec3 {
    (0..points.len())
        .map(|i| points[i].cross(points[(i + 1) % points.len()]))
        .sum::<DVec3>()
        * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_normal_is_canonical() {
        let p = Plane::from_points(DVec3::ZERO, DVec3::Y, DVec3::X).unwrap();
        assert_eq!(p.normal, DVec3::Z);
        assert!(Plane::from_points(DVec3::ZERO, DVec3::X, DVec3::X * 2.0).is_none());
    }

    #[test]
    fn basis_is_right_handed() {
        let p = Plane::from_points(DVec3::ZERO, DVec3::new(1.0, 0.0, 1.0), DVec3::Y).unwrap();
        let (u, v) = p.basis();
        assert!(u.cross(v).abs_diff_eq(p.normal, 1e-12));
        assert!(u.dot(p.normal).abs() < 1e-12);
    }

    #[test]
    fn crossing_segments_meet_at_interior_point() {
        let x = segment_crossing(
            DVec3::new(0.0, -1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
        );
        assert!(x.unwrap().abs_diff_eq(DVec3::ZERO, 1e-12));
    }

    #[test]
    fn touching_parallel_and_skew_segments_do_not_cross() {
        let (o, x, y) = (DVec3::ZERO, DVec3::X, DVec3::Y);
        assert!(segment_crossing(o, x, o, y).is_none(), "shared endpoint");
        assert!(segment_crossing(o, x, y, x + y).is_none(), "parallel");
        assert!(
            segment_crossing(-x, x, -y + DVec3::Z, y + DVec3::Z).is_none(),
            "skew"
        );
        assert!(
            segment_crossing(-x, x, DVec3::new(2.0, -1.0, 0.0), DVec3::new(2.0, 1.0, 0.0))
                .is_none(),
            "beyond end"
        );
    }

    #[test]
    fn same_as_ignores_normal_orientation() {
        let p = Plane::from_points(DVec3::ZERO, DVec3::Z, DVec3::new(1.0, 1.0, 0.0)).unwrap();
        let q = Plane {
            normal: -p.normal,
            d: -p.d,
        };
        assert!(p.same_as(&q));
        assert!(q.same_as(&p));
        let offset = Plane {
            normal: p.normal,
            d: p.d + 1.0,
        };
        assert!(!p.same_as(&offset));
    }

    #[test]
    fn polygon_helpers() {
        let sq = [DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y];
        assert_eq!(signed_area2(&sq), 2.0);
        assert!(point_in_polygon(DVec2::splat(0.5), &sq));
        assert!(!point_in_polygon(DVec2::splat(1.5), &sq));
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        assert!(vector_area(&pts).abs_diff_eq(DVec3::Z, 1e-12));
    }
}
