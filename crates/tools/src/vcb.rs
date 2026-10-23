//! The measurements box: parsing typed lengths and formatting live ones.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Units;

use crate::inference::{Inference, SnapKind};
use crate::tool::ToolContext;

/// Parses a typed length in meters, a bare number being in the model's unit (see
/// `session::parse_length`). None if it is not a length.
pub fn parse_length(s: &str, units: &Units) -> Option<f64> {
    session::parse_length(s, units).ok()
}

/// Comma-separated lengths, each with its own unit: "2,3" or "200mm, 1.5m". A comma inside
/// feet and inches ("3'6 1/2", 4'") does not matter: they have none.
pub fn parse_lengths(s: &str, units: &Units) -> Option<Vec<f64>> {
    s.split(',').map(|v| parse_length(v, units)).collect()
}

/// One typed scale value: a plain number is a factor, a length (one with a unit, or in feet and
/// inches) a length in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScaleValue {
    Factor(f64),
    Length(f64),
}

/// Comma-separated scale values, all factors ("2", "-1", "1.5, 2") or all lengths ("3m",
/// "250mm, 1m"). A bare number is always a factor, whatever the model's unit. None if one is
/// neither or the kinds are mixed.
pub fn parse_scale(s: &str, units: &Units) -> Option<Vec<ScaleValue>> {
    let values: Vec<ScaleValue> = s
        .split(',')
        .map(|v| {
            let v = v.trim();
            let (sign, digits) = match v.strip_prefix('-') {
                Some(rest) => (-1.0, rest.trim_start()),
                None => (1.0, v),
            };
            match number(digits) {
                Some(f) => Some(ScaleValue::Factor(sign * f)),
                None => parse_length(v, units).map(ScaleValue::Length),
            }
        })
        .collect::<Option<_>>()?;
    let factors = values
        .iter()
        .filter(|v| matches!(v, ScaleValue::Factor(_)))
        .count();
    (factors == 0 || factors == values.len()).then_some(values)
}

/// A typed array of copies: "5x", "x5" or "*5" make 5 copies at the last copy's spacing;
/// "/5" makes 5 copies evenly between the original and the last copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Array {
    Times(u32),
    Divide(u32),
}

/// Parses array syntax; None if it is not an array or the count is 0. Surrounding spaces are
/// allowed, spaces inside are not.
pub fn parse_array(s: &str) -> Option<Array> {
    let s = s.trim().to_ascii_lowercase();
    // Digits only: no sign, no inner spaces.
    let count = |n: &str| {
        let digits = !n.is_empty() && n.chars().all(|c| c.is_ascii_digit());
        digits
            .then(|| n.parse::<u32>().ok())
            .flatten()
            .filter(|n| *n > 0)
    };
    if let Some(n) = s.strip_prefix('/') {
        return count(n).map(Array::Divide);
    }
    let n = s
        .strip_suffix('x')
        .or_else(|| s.strip_prefix('x'))
        .or_else(|| s.strip_prefix('*'))?;
    count(n).map(Array::Times)
}

/// A typed side count, "24s" (any case, spaces around allowed): the number of segments for
/// circles. None if it is not one or the count is 0.
pub fn parse_segments(s: &str) -> Option<u32> {
    let s = s.trim().to_ascii_lowercase();
    let n = s.strip_suffix('s')?;
    let digits = !n.is_empty() && n.chars().all(|c| c.is_ascii_digit());
    digits
        .then(|| n.parse::<u32>().ok())
        .flatten()
        .filter(|n| *n > 0)
}

/// True when typed text ends in "s" (any case) and so was meant as a side or segment count,
/// even if it is not a valid one ("0s", "2.5s").
pub fn meant_as_count(s: &str) -> bool {
    s.trim().to_ascii_lowercase().ends_with('s')
}

/// Parses an angle in degrees: "90", "-45", "22.5", optionally ending in "°" or "deg". None if
/// it is not an angle.
pub fn parse_angle(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = (s.strip_suffix('°'))
        .or_else(|| s.strip_suffix("deg"))
        .unwrap_or(s)
        .trim_end();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest.trim_start()),
        None => (1.0, s),
    };
    number(s).filter(|a| a.is_finite()).map(|a| sign * a)
}

/// Parses a slope "rise:run" ("1:2", "6in:12in", "-1:4") as an angle in degrees, atan(rise / run).
/// None if it is not two lengths or the run is zero.
pub fn parse_slope(s: &str, units: &Units) -> Option<f64> {
    let (rise, run) = s.split_once(':')?;
    let (rise, run) = (parse_length(rise, units)?, parse_length(run, units)?);
    (run != 0.0).then(|| (rise / run).atan().to_degrees())
}

/// An angle for display in the model's angle precision: "22.5°".
pub fn format_angle(degrees: f64, units: &Units) -> String {
    session::format_angle(degrees, units)
}

/// A length for display in the model's unit and precision: "2.5 m", "7' 3 1/2\"".
pub fn format_length(meters: f64, units: &Units) -> String {
    session::format_length(meters, units)
}

/// `meters` rounded to the nearest multiple of the model's length snapping (unchanged where it
/// is off). For lengths taken from the cursor; typed lengths are never rounded.
pub fn snap_length(meters: f64, units: &Units) -> f64 {
    if units.length_snap > 0.0 {
        (meters / units.length_snap).round() * units.length_snap
    } else {
        meters
    }
}

/// Whether the cursor's length is rounded to the model's length snapping at `hit`: not when it
/// is off, and not at a point of the model (an endpoint, a midpoint...), which keeps its exact
/// position, also under a direction lock.
pub(crate) fn snaps(hit: &Inference, units: &Units) -> bool {
    units.length_snap > 0.0 && hit.point_snap().is_none()
}

/// The cursor point `p` (found with `hit`) moved along the line from `from` so its distance
/// from `from` is a multiple of the model's length snapping; unchanged where `snaps` is false.
pub(crate) fn snap_distance(from: DVec3, p: DVec3, hit: &Inference, units: &Units) -> DVec3 {
    if !snaps(hit, units) {
        return p;
    }
    let v = p - from;
    let len = v.length();
    if len < LENGTH_TOL {
        return p;
    }
    from + v * (snap_length(len, units) / len)
}

/// [`snap_distance`] for a point taken on a face or the ground as it is (not on a plane the
/// tool draws in): rounded only when the rounded point stays on that surface, which it does
/// not when `from` lies off its plane.
pub(crate) fn snap_distance_on_surface(
    from: DVec3,
    p: DVec3,
    hit: &Inference,
    ctx: &ToolContext,
) -> DVec3 {
    on_surface(p, snap_distance(from, p, hit, &ctx.units), hit, ctx)
}

/// `q`, the rounded `p`, unless it left the face or ground `hit` was taken on: then `p`.
pub(crate) fn on_surface(p: DVec3, q: DVec3, hit: &Inference, ctx: &ToolContext) -> DVec3 {
    let normal = match hit.kind {
        SnapKind::OnFace => hit.face.and_then(|f| ctx.model.face(f)).map(|f| f.normal),
        // A ground hit lies on the open context's own z = 0.
        SnapKind::Ground => Some(DVec3::Z),
        _ => None,
    };
    match normal {
        Some(n) if (q - p).dot(n).abs() > LENGTH_TOL => p,
        _ => q,
    }
}

/// A typed `value` pointed the way the cursor is: negated when `cursor` (the live measurement)
/// is clearly negative. A cursor within tolerance of zero counts as positive.
pub fn toward_cursor(cursor: f64, value: f64) -> f64 {
    if cursor <= -LENGTH_TOL { -value } else { value }
}

fn number(s: &str) -> Option<f64> {
    let digits = !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');
    if digits { s.parse().ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::LengthUnit;

    const INCH: f64 = 0.0254;

    fn parse_length(s: &str) -> Option<f64> {
        super::parse_length(s, &Units::default())
    }

    fn parse_lengths(s: &str) -> Option<Vec<f64>> {
        super::parse_lengths(s, &Units::default())
    }

    fn parse_scale(s: &str) -> Option<Vec<ScaleValue>> {
        super::parse_scale(s, &Units::default())
    }

    fn format_length(m: f64) -> String {
        super::format_length(m, &Units::default())
    }

    fn format_angle(d: f64) -> String {
        super::format_angle(d, &Units::default())
    }

    fn feet_inches() -> Units {
        Units {
            length: LengthUnit::FeetInches,
            ..Units::default()
        }
    }

    /// Ground hits the way a tool finds them: through inference with the camera in a context
    /// turned by `to_world`, so they lie on the context's own z = 0.
    fn ground_hits(to_world: glam::DAffine3, cursors: [glam::DVec2; 2]) -> [Inference; 2] {
        let model = kernel::Model::new();
        let camera =
            session::Camera::new(glam::DVec2::new(800.0, 600.0)).placed(to_world.inverse());
        cursors.map(|c| {
            let hit = crate::inference::infer(&model, &camera, c, None, 10.0);
            assert_eq!(hit.kind, SnapKind::Ground, "{hit:?}");
            assert!(hit.point.z.abs() < 1e-9, "{hit:?}");
            hit
        })
    }

    fn round_between(to_world: glam::DAffine3, from: DVec3, hit: &Inference) -> DVec3 {
        let model = kernel::Model::new();
        let camera = session::Camera::new(glam::DVec2::new(800.0, 600.0));
        let units = Units {
            length_snap: 0.1,
            ..Units::default()
        };
        let ctx = ToolContext {
            model: &model,
            scenery: &crate::scenery::Scenery::single(&model),
            camera: &camera,
            cursor: glam::DVec2::ZERO,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            scale_factor: 1.0,
            to_world,
            quick_press: false,
            units,
            shift: false,
            selection: &Default::default(),
            material: None,
            sample: false,
            labels: &[],
        };
        snap_distance_on_surface(from, hit.point, hit, &ctx)
    }

    #[test]
    fn ground_hits_in_a_tilted_group_round_on_the_context_ground() {
        // The ground a tool sees is the context's own z = 0, whatever way the context is turned.
        let to_world = glam::DAffine3::from_rotation_x(0.4);
        let [a, b] = ground_hits(
            to_world,
            [
                glam::DVec2::new(300.0, 400.0),
                glam::DVec2::new(520.0, 380.0),
            ],
        );
        let q = round_between(to_world, a.point, &b);
        assert!(q != b.point, "not rounded: {q:?}");
        assert!(q.z.abs() < 1e-9, "{q:?}");
        let length = q.distance(a.point);
        assert!(
            ((length / 0.1).round() * 0.1 - length).abs() < 1e-9,
            "{length}"
        );
    }

    #[test]
    fn ground_rounding_does_not_leave_the_context_ground() {
        let to_world = glam::DAffine3::from_rotation_x(std::f64::consts::FRAC_PI_2);
        let [_, b] = ground_hits(
            to_world,
            [
                glam::DVec2::new(300.0, 400.0),
                glam::DVec2::new(520.0, 380.0),
            ],
        );
        // From a point off the ground the rounded point would leave it: the cursor stays.
        let from = DVec3::new(0.0, 0.0, 1.0);
        assert_eq!(round_between(to_world, from, &b), b.point);
    }

    #[test]
    fn side_counts_end_in_s() {
        assert_eq!(parse_segments("24s"), Some(24));
        assert_eq!(parse_segments(" 8S "), Some(8));
        for bad in ["s", "24", "0s", "-3s", "2 4s", "4.5s", "24ss"] {
            assert_eq!(parse_segments(bad), None, "{bad:?}");
        }
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-12)
    }

    #[test]
    fn metric_lengths() {
        assert!(close(parse_length("2.5"), 2.5));
        assert!(close(parse_length(" 2m "), 2.0));
        assert!(close(parse_length("250mm"), 0.25));
        assert!(close(parse_length("25 cm"), 0.25));
        assert!(close(parse_length(".5"), 0.5));
        assert!(close(parse_length("-1.2M"), -1.2));
    }

    #[test]
    fn imperial_lengths() {
        assert!(close(parse_length("10in"), 0.254));
        assert!(close(parse_length("10\""), 0.254));
        assert!(close(parse_length("3ft"), 0.9144));
        assert!(close(parse_length("3'"), 0.9144));
        assert!(close(parse_length("5'6\""), 66.0 * INCH));
        assert!(close(parse_length("5' 6"), 66.0 * INCH));
    }

    #[test]
    fn typed_lengths_follow_the_model_unit() {
        let fi = feet_inches();
        let ft = Units {
            length: LengthUnit::Feet,
            ..Units::default()
        };
        let p = |s: &str, u: &Units| super::parse_length(s, u);
        assert!(close(p("3'6 1/2\"", &fi), 42.5 * INCH));
        assert!(close(p("10", &fi), 10.0 * INCH));
        assert!(close(p("2", &ft), 24.0 * INCH));
        assert!(close(p("2m", &fi), 2.0));
        assert!(close(p("-3'6", &fi), -42.0 * INCH));
        assert_eq!(p("3'x", &fi), None);
    }

    #[test]
    fn feet_inches_lengths_split_only_on_the_comma_between_them() {
        let fi = feet_inches();
        assert_eq!(
            super::parse_lengths("3'6 1/2\", 4'", &fi),
            Some(vec![42.5 * INCH, 48.0 * INCH])
        );
        assert_eq!(
            super::parse_lengths("6 1/2, 3", &fi).map(|v| v.len()),
            Some(2)
        );
    }

    #[test]
    fn scale_numbers_stay_factors_whatever_the_unit() {
        use ScaleValue::{Factor, Length};
        let fi = feet_inches();
        assert_eq!(super::parse_scale("2", &fi), Some(vec![Factor(2.0)]));
        assert_eq!(
            super::parse_scale("1.5, 2", &fi),
            Some(vec![Factor(1.5), Factor(2.0)])
        );
        let Some([Length(m)]) = super::parse_scale("3'", &fi).as_deref().map(|v| [v[0]]) else {
            panic!("not a length")
        };
        assert!((m - 36.0 * INCH).abs() < 1e-12);
        assert_eq!(super::parse_scale("2,3'", &fi), None);
    }

    #[test]
    fn slope_lengths_follow_the_model_unit() {
        let deg = super::parse_slope("6:12", &feet_inches()).unwrap();
        assert!((deg - 0.5f64.atan().to_degrees()).abs() < 1e-12);
        let deg = super::parse_slope("1':2'", &feet_inches()).unwrap();
        assert!((deg - 0.5f64.atan().to_degrees()).abs() < 1e-12);
    }

    #[test]
    fn lengths_show_in_the_model_unit_and_snap() {
        let cm = Units {
            length: LengthUnit::Centimeters,
            ..Units::default()
        };
        assert_eq!(super::format_length(1.25, &cm), "125 cm");
        assert_eq!(super::format_length(1.25, &feet_inches()), "4' 1 1/4\"");
        let snap = Units {
            length_snap: 0.25,
            ..Units::default()
        };
        assert_eq!(super::snap_length(1.13, &snap), 1.25);
        assert_eq!(super::snap_length(-0.9, &snap), -1.0);
        assert_eq!(super::snap_length(1.13, &Units::default()), 1.13);
    }

    #[test]
    fn angles_show_with_the_model_precision() {
        let u = Units {
            angle_precision: 2,
            ..Units::default()
        };
        assert_eq!(super::format_angle(22.456, &u), "22.46°");
        assert_eq!(super::format_angle(22.456, &Units::default()), "22.5°");
    }

    #[test]
    fn rejects_non_lengths() {
        for bad in ["", "m", "abc", "2km", "1e3", "2..5", "--2", "5'x", "'6"] {
            assert_eq!(parse_length(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn comma_separated_lengths() {
        assert_eq!(parse_lengths("2,3"), Some(vec![2.0, 3.0]));
        assert_eq!(parse_lengths("200mm, 1.5m"), Some(vec![0.2, 1.5]));
        assert_eq!(parse_lengths("2,"), None);
    }

    #[test]
    fn typed_values_follow_the_cursor_direction() {
        assert_eq!(toward_cursor(0.4, 2.0), 2.0);
        assert_eq!(toward_cursor(-0.4, 2.0), -2.0);
        assert_eq!(toward_cursor(-0.4, -2.0), 2.0);
        assert_eq!(
            toward_cursor(-1e-12, 2.0),
            2.0,
            "rounding noise is not a direction"
        );
    }

    #[test]
    fn parse_scale_reads_factors_and_lengths() {
        use ScaleValue::{Factor, Length};
        assert_eq!(parse_scale("2"), Some(vec![Factor(2.0)]));
        assert_eq!(parse_scale("-1"), Some(vec![Factor(-1.0)]));
        assert_eq!(parse_scale("1.5, 2"), Some(vec![Factor(1.5), Factor(2.0)]));
        assert_eq!(parse_scale("3m"), Some(vec![Length(3.0)]));
        assert_eq!(
            parse_scale("250mm,1m"),
            Some(vec![Length(0.25), Length(1.0)])
        );
        for bad in ["2,3m", "", "x", "2,", "1e3"] {
            assert_eq!(parse_scale(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn array_syntax() {
        assert_eq!(parse_array("5x"), Some(Array::Times(5)));
        assert_eq!(parse_array(" X3 "), Some(Array::Times(3)));
        assert_eq!(parse_array("*2"), Some(Array::Times(2)));
        assert_eq!(parse_array("/4"), Some(Array::Divide(4)));
        for bad in [
            "5", "0x", "/0", "x", "/", "2.5x", "-3x", "5m", "+5x", "x+5", "/+5", "*+5", "5 x",
            "x 5", "/ 5", "* 5", "1 0x",
        ] {
            assert_eq!(parse_array(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn angles_are_degrees() {
        assert_eq!(parse_angle("90"), Some(90.0));
        assert_eq!(parse_angle(" -45 "), Some(-45.0));
        assert_eq!(parse_angle("22.5°"), Some(22.5));
        assert_eq!(parse_angle("30deg"), Some(30.0));
        for bad in ["", "deg", "abc", "1e3", "--5", "5m", "90x"] {
            assert_eq!(parse_angle(bad), None, "{bad:?}");
        }
        assert_eq!(format_angle(22.5), "22.5°");
        assert_eq!(format_angle(90.0), "90°");
        assert_eq!(format_angle(-0.01), "0°");
    }

    #[test]
    fn formats_meters() {
        assert_eq!(format_length(2.5), "2.5 m");
        assert_eq!(format_length(2.0), "2 m");
        assert_eq!(format_length(0.12345), "0.123 m");
        assert_eq!(format_length(-0.0001), "0 m");
        assert_eq!(format_length(-1.25), "-1.25 m");
    }
}

#[cfg(test)]
mod slope_tests {
    use super::*;

    fn parse_slope(s: &str) -> Option<f64> {
        super::parse_slope(s, &Units::default())
    }

    #[test]
    fn slopes_are_rise_over_run_in_degrees() {
        let deg = |s| parse_slope(s).unwrap();
        assert!((deg("1:1") - 45.0).abs() < 1e-12);
        assert!((deg("1:2") - 0.5f64.atan().to_degrees()).abs() < 1e-12);
        assert!((deg(" 3 : 12 ") - 0.25f64.atan().to_degrees()).abs() < 1e-12);
        assert!((deg("6in:12in") - 0.5f64.atan().to_degrees()).abs() < 1e-12);
        assert!((deg("0:5")).abs() < 1e-12);
        // Signs: either one negative tips it down, both negative cancel.
        assert!((deg("-1:2") + 0.5f64.atan().to_degrees()).abs() < 1e-12);
        assert!((deg("1:-2") + 0.5f64.atan().to_degrees()).abs() < 1e-12);
        assert!((deg("-1:-2") - 0.5f64.atan().to_degrees()).abs() < 1e-12);
    }

    #[test]
    fn bad_slopes_are_refused() {
        for bad in ["1:0", "1", "", ":", "1:", ":2", "a:2", "1:b", "1:2:3", "30"] {
            assert_eq!(parse_slope(bad), None, "{bad:?}");
        }
    }
}
