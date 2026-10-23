//! The measurements box: parsing typed lengths and formatting live ones.

use kernel::geom::LENGTH_TOL;

const INCH: f64 = 0.0254;

/// Parses a length in meters: "2.5" (meters), "250mm", "25cm", "2m", "10in", "10\"", "3ft",
/// "3'", "5'6\"" or "5'6". A leading "-" negates it. None if it is not a length.
pub fn parse_length(s: &str) -> Option<f64> {
    let s = s.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest.trim_start()),
        None => (1.0, s),
    };
    let meters = match s.split_once('\'') {
        Some((feet, inches)) => {
            let inches = inches.trim();
            let inches = inches.strip_suffix('"').unwrap_or(inches).trim();
            let inches = if inches.is_empty() {
                0.0
            } else {
                number(inches)?
            };
            (number(feet.trim())? * 12.0 + inches) * INCH
        }
        None => {
            let split = s
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(s.len());
            let (value, unit) = s.split_at(split);
            number(value)? * unit_scale(unit.trim())?
        }
    };
    meters.is_finite().then_some(sign * meters)
}

/// Comma-separated lengths, each with its own unit: "2,3" or "200mm, 1.5m".
pub fn parse_lengths(s: &str) -> Option<Vec<f64>> {
    s.split(',').map(parse_length).collect()
}

/// One typed scale value: a plain number is a factor, a number with a unit a length (meters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScaleValue {
    Factor(f64),
    Length(f64),
}

/// Comma-separated scale values, all factors ("2", "-1", "1.5, 2") or all lengths ("3m",
/// "250mm, 1m"). None if one is neither or the kinds are mixed.
pub fn parse_scale(s: &str) -> Option<Vec<ScaleValue>> {
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
                None => parse_length(v).map(ScaleValue::Length),
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
pub fn parse_slope(s: &str) -> Option<f64> {
    let (rise, run) = s.split_once(':')?;
    let (rise, run) = (parse_length(rise)?, parse_length(run)?);
    (run != 0.0).then(|| (rise / run).atan().to_degrees())
}

/// An angle for display, in degrees to a tenth: "22.5°".
pub fn format_angle(degrees: f64) -> String {
    let s = format!("{degrees:.1}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let s = if s == "-0" { "0" } else { s };
    format!("{s}°")
}

/// A length for display, in meters with up to millimeter precision: "2.5 m".
pub fn format_length(meters: f64) -> String {
    let s = format!("{meters:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let s = if s == "-0" { "0" } else { s };
    format!("{s} m")
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

fn unit_scale(unit: &str) -> Option<f64> {
    Some(match unit.to_ascii_lowercase().as_str() {
        "" | "m" => 1.0,
        "mm" => 0.001,
        "cm" => 0.01,
        "in" | "\"" => INCH,
        "ft" => 12.0 * INCH,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
