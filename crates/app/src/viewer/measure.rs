//! Measuring in the viewer: tap a first point, tap a second, read the distance between them;
//! tap again to start over. Points snap as inference does, but within a finger's reach.

use glam::DVec3;
use session::{Units, format_length};
use tools::SnapKind;

/// How far, in logical pixels, a tap may be from a vertex or edge and still snap to it: a
/// finger covers much more than a mouse pointer. Multiply by the display scale factor for the
/// physical pixels inference works in.
pub const TOUCH_RADIUS_PX: f64 = 24.0;

/// The snap radius in physical pixels on a display with `scale_factor` physical pixels per
/// logical one.
pub fn snap_radius(scale_factor: f64) -> f64 {
    TOUCH_RADIUS_PX * scale_factor
}

/// Whether a snap is a point on the model: a corner, middle, edge, guide or face. The origin
/// (a mark, not geometry), directions from nowhere, the ground and open air are not measured.
pub fn on_model(kind: SnapKind) -> bool {
    matches!(
        kind,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::GuidePoint
            | SnapKind::Intersection
            | SnapKind::OnEdge
            | SnapKind::OnGuide
            | SnapKind::OnFace
    )
}

/// The measurement in progress: no point, one, or both.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measure {
    first: Option<DVec3>,
    second: Option<DVec3>,
}

impl Measure {
    /// A tapped point: the first when none is set (or a measurement was complete, which it
    /// starts over), else the second.
    pub fn tap(&mut self, point: DVec3) {
        match (self.first, self.second) {
            (Some(_), None) => self.second = Some(point),
            _ => {
                self.first = Some(point);
                self.second = None;
            }
        }
    }

    pub fn first(&self) -> Option<DVec3> {
        self.first
    }

    pub fn second(&self) -> Option<DVec3> {
        self.second
    }

    /// The distance between the two points in meters, once both are set.
    pub fn distance(&self) -> Option<f64> {
        Some(self.first?.distance(self.second?))
    }

    /// What the bar says: the distance in `units`, or what to tap next.
    pub fn text(&self, units: &Units) -> String {
        match (self.first, self.second) {
            (None, _) => "Tap the first point".into(),
            (Some(_), None) => "Tap the second point".into(),
            (Some(_), Some(_)) => format_length(self.distance().expect("both points"), units),
        }
    }
}

#[cfg(test)]
mod tests {
    use session::LengthUnit;

    use super::*;

    #[test]
    fn two_taps_measure_a_third_starts_over() {
        let mut m = Measure::default();
        assert_eq!(m.distance(), None);
        assert_eq!(m.text(&Units::default()), "Tap the first point");
        m.tap(DVec3::ZERO);
        assert_eq!(m.distance(), None);
        assert_eq!(m.text(&Units::default()), "Tap the second point");
        m.tap(DVec3::new(3.0, 4.0, 0.0));
        assert_eq!(m.distance(), Some(5.0));
        assert_eq!(m.text(&Units::default()), "5 m");
        m.tap(DVec3::new(1.0, 1.0, 1.0));
        assert_eq!(m.first(), Some(DVec3::new(1.0, 1.0, 1.0)));
        assert_eq!(m.second(), None);
        assert_eq!(m.distance(), None);
        m.tap(DVec3::new(1.0, 1.0, 3.0));
        assert_eq!(m.distance(), Some(2.0));
    }

    #[test]
    fn the_distance_is_shown_in_the_documents_units() {
        let mut m = Measure::default();
        m.tap(DVec3::ZERO);
        m.tap(DVec3::new(1.2, 0.0, 1.6));
        assert_eq!(m.text(&Units::default()), "2 m");
        let cm = Units {
            length: LengthUnit::Centimeters,
            precision: 1,
            ..Units::default()
        };
        assert_eq!(m.text(&cm), "200 cm");
        let inches = Units {
            length: LengthUnit::Inches,
            precision: 2,
            ..Units::default()
        };
        assert_eq!(m.text(&inches), "78.74\"");
    }

    #[test]
    fn the_touch_radius_is_24_logical_pixels_whatever_the_display() {
        assert_eq!(snap_radius(1.0), 24.0);
        assert_eq!(snap_radius(3.0), 72.0);
        assert_eq!(snap_radius(1.5), 36.0);
    }

    #[test]
    fn only_points_on_the_model_count() {
        for kind in [
            SnapKind::Endpoint,
            SnapKind::Midpoint,
            SnapKind::OnEdge,
            SnapKind::OnFace,
            SnapKind::Intersection,
        ] {
            assert!(on_model(kind), "{kind:?}");
        }
        for kind in [
            SnapKind::Free,
            SnapKind::Ground,
            SnapKind::Parallel,
            SnapKind::Origin,
        ] {
            assert!(!on_model(kind), "{kind:?}");
        }
    }
}
