//! Where a section plane is drawn: a square outline centred on the plane's point. The renderer
//! adds the arrows along the normal; tools pick a plane by the same outline.

use glam::{DAffine3, DVec3};
use kernel::geom::Plane;
use kernel::{Model, SectionId, SectionPlane};

/// The smallest side of a plane's square, in meters.
pub const MIN_SECTION_SIDE: f64 = 1.0;
/// How far past the diagonal of the shown bounds the square reaches.
pub const SECTION_SIDE_SCALE: f64 = 1.2;

/// A section plane's outline placed in some coordinates (a mesh's own, or another by
/// `transformed`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionShape {
    /// The plane's point, in the middle of the square.
    pub center: DVec3,
    /// The square's corners in order around it.
    pub corners: [DVec3; 4],
    /// The unit normal, toward the kept side.
    pub normal: DVec3,
}

impl SectionShape {
    /// The square of `side` meters centred on `plane`'s point, in the plane.
    pub fn new(plane: &SectionPlane, side: f64) -> SectionShape {
        let (u, v) = Plane {
            normal: plane.normal,
            d: 0.0,
        }
        .basis();
        let (u, v) = (u * side / 2.0, v * side / 2.0);
        let c = plane.point;
        SectionShape {
            center: c,
            corners: [c - u - v, c + u - v, c + u + v, c - u + v],
            normal: plane.normal,
        }
    }

    /// The side of the square for a context showing `bounds` (as `Scene::shown_bounds`): 1.2
    /// times the bounds' diagonal, and at least 1 m.
    pub fn side(bounds: Option<[DVec3; 2]>) -> f64 {
        let diagonal = bounds.map_or(0.0, |[lo, hi]| lo.distance(hi));
        (SECTION_SIDE_SCALE * diagonal).max(MIN_SECTION_SIDE)
    }

    /// The outline of plane `id` of `model` for a square of `side` meters, in the model's
    /// coordinates. None when there is no such plane.
    pub fn of(model: &Model, id: SectionId, side: f64) -> Option<SectionShape> {
        Some(SectionShape::new(model.section(id)?, side))
    }

    /// The shape placed by `t`: its corners move with it, the normal by the inverse transpose
    /// so it stays across the plane (and toward the same material) under a stretch or a mirror.
    pub fn transformed(&self, t: DAffine3) -> SectionShape {
        let n = t.matrix3.inverse().transpose() * self.normal;
        let normal = n
            .try_normalize()
            .filter(|n| n.is_finite())
            .unwrap_or(self.normal);
        SectionShape {
            center: t.transform_point3(self.center),
            corners: self.corners.map(|p| t.transform_point3(p)),
            normal,
        }
    }

    /// The four sides of the square, in order.
    pub fn segments(&self) -> [[DVec3; 2]; 4] {
        let c = self.corners;
        [[c[0], c[1]], [c[1], c[2]], [c[2], c[3]], [c[3], c[0]]]
    }

    /// The middle of each side, where the arrows along the normal start.
    pub fn arrow_bases(&self) -> [DVec3; 4] {
        self.segments().map(|[a, b]| a.midpoint(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_square_is_centred_in_the_plane_with_the_given_side() {
        let plane = SectionPlane::new(DVec3::new(1.0, 2.0, 3.0), DVec3::Z);
        let s = SectionShape::new(&plane, 4.0);
        assert_eq!(s.center, plane.point);
        for seg in s.segments() {
            assert!((seg[0].distance(seg[1]) - 4.0).abs() < 1e-12);
            assert!((seg[0].z - 3.0).abs() < 1e-12);
        }
        let mid = s.corners.iter().copied().sum::<DVec3>() / 4.0;
        assert!(mid.distance(plane.point) < 1e-12);
        assert_eq!(s.arrow_bases().len(), 4);
    }

    #[test]
    fn the_side_is_a_meter_at_least_and_grows_with_the_bounds() {
        assert_eq!(SectionShape::side(None), 1.0);
        assert_eq!(
            SectionShape::side(Some([DVec3::ZERO, DVec3::splat(0.1)])),
            1.0
        );
        let side = SectionShape::side(Some([DVec3::ZERO, DVec3::new(3.0, 4.0, 0.0)]));
        assert!((side - 6.0).abs() < 1e-12);
    }

    #[test]
    fn a_transformed_shape_keeps_its_normal_across_a_stretch() {
        let plane = SectionPlane::new(DVec3::ZERO, DVec3::new(1.0, 0.0, 1.0).normalize());
        let s = SectionShape::new(&plane, 2.0);
        let t = DAffine3::from_scale(DVec3::new(2.0, 1.0, 1.0));
        let moved = s.transformed(t);
        for seg in moved.segments() {
            assert!(
                (seg[1] - seg[0]).dot(moved.normal).abs() < 1e-9,
                "the sides lie in the plane"
            );
        }
        assert!((moved.normal.length() - 1.0).abs() < 1e-12);
    }
}
