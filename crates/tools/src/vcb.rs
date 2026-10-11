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

/// A typed array of copies: "5x", "x5" or "*5" make 5 copies at the last copy's spacing;
/// "/5" makes 5 copies evenly between the original and the last copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Array {
    Times(u32),
    Divide(u32),
}

/// Parses array syntax; None if it is not an array or the count is 0.
pub fn parse_array(s: &str) -> Option<Array> {
    let s = s.trim().to_ascii_lowercase();
    let count = |n: &str| n.trim().parse::<u32>().ok().filter(|n| *n > 0);
    if let Some(n) = s.strip_prefix('/') {
        return count(n).map(Array::Divide);
    }
    let n = s
        .strip_suffix('x')
        .or_else(|| s.strip_prefix('x'))
        .or_else(|| s.strip_prefix('*'))?;
    count(n).map(Array::Times)
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
    fn array_syntax() {
        assert_eq!(parse_array("5x"), Some(Array::Times(5)));
        assert_eq!(parse_array(" X3 "), Some(Array::Times(3)));
        assert_eq!(parse_array("*2"), Some(Array::Times(2)));
        assert_eq!(parse_array("/4"), Some(Array::Divide(4)));
        for bad in ["5", "0x", "/0", "x", "/", "2.5x", "-3x", "5m"] {
            assert_eq!(parse_array(bad), None, "{bad:?}");
        }
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
