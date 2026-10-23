//! The model's display units: how lengths and angles are written and typed, and the
//! snapping steps the tools round cursor measurements to.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Meters in an inch.
const INCH: f64 = 0.0254;

/// Meters in a foot.
const FOOT: f64 = 12.0 * INCH;

/// The most `precision` may be.
pub const MAX_PRECISION: u8 = 6;

/// The most `angle_precision` may be.
pub const MAX_ANGLE_PRECISION: u8 = 3;

/// The most `length_snap` may be, in meters.
pub const MAX_LENGTH_SNAP: f64 = 1000.0;

/// The smallest nonzero `length_snap`, in meters (a micrometer).
pub const MIN_LENGTH_SNAP: f64 = 1e-6;

/// The most `angle_snap` may be, in degrees.
pub const MAX_ANGLE_SNAP: f64 = 90.0;

/// The unit lengths are written and typed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    #[default]
    Meters,
    Centimeters,
    Millimeters,
    Inches,
    /// Decimal feet (engineering): `4.1'`.
    Feet,
    /// Feet and inches with fractions of an inch (architectural): `5' 3 1/2"`.
    FeetInches,
}

/// How the model writes and types lengths and angles, and what its tools snap to:
/// - `length`, and `precision`, the decimals kept (0 to 6), or for `feet_inches` the finest
///   fraction of an inch, 1/2^precision (0: whole inches, 6: 1/64);
/// - `length_snap`, in meters: a length taken from the cursor rounds to a multiple of it
///   (0: off, up to 1000);
/// - `angle_precision`, the decimals of an angle (0 to 3);
/// - `angle_snap`, in degrees: the step angles from the cursor round to (0: free, up to 90).
///
/// Saved with the model; the default is meters with 3 decimals, no length snapping, angles
/// with 1 decimal and a 15 degree step.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Units {
    pub length: LengthUnit,
    pub precision: u8,
    #[serde(default)]
    pub length_snap: f64,
    #[serde(default = "default_angle_precision")]
    pub angle_precision: u8,
    #[serde(default = "default_angle_snap")]
    pub angle_snap: f64,
}

/// [`Units`] as the file stores them: the settings added after version 12 are left out
/// while they are the defaults, so files of default-snapping units read as before.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredUnits {
    length: LengthUnit,
    precision: u8,
    #[serde(default, skip_serializing_if = "is_zero")]
    length_snap: f64,
    #[serde(
        default = "default_angle_precision",
        skip_serializing_if = "is_default_angle_precision"
    )]
    angle_precision: u8,
    #[serde(
        default = "default_angle_snap",
        skip_serializing_if = "is_default_angle_snap"
    )]
    angle_snap: f64,
}

impl From<Units> for StoredUnits {
    fn from(u: Units) -> Self {
        StoredUnits {
            length: u.length,
            precision: u.precision,
            length_snap: u.length_snap,
            angle_precision: u.angle_precision,
            angle_snap: u.angle_snap,
        }
    }
}

impl From<StoredUnits> for Units {
    fn from(u: StoredUnits) -> Self {
        Units {
            length: u.length,
            precision: u.precision,
            length_snap: u.length_snap,
            angle_precision: u.angle_precision,
            angle_snap: u.angle_snap,
        }
    }
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn default_angle_precision() -> u8 {
    1
}

fn is_default_angle_precision(v: &u8) -> bool {
    *v == default_angle_precision()
}

fn default_angle_snap() -> f64 {
    15.0
}

fn is_default_angle_snap(v: &f64) -> bool {
    *v == default_angle_snap()
}

impl Default for Units {
    fn default() -> Self {
        Units {
            length: LengthUnit::Meters,
            precision: 3,
            length_snap: 0.0,
            angle_precision: default_angle_precision(),
            angle_snap: default_angle_snap(),
        }
    }
}

impl Units {
    /// Refuses a setting outside its range.
    pub fn check(&self) -> Result<(), String> {
        if self.precision > MAX_PRECISION {
            return Err(format!(
                "precision must be from 0 to {MAX_PRECISION} (for feet_inches: fractions down to 1/64 inch)"
            ));
        }
        let snap = self.length_snap;
        if !(snap.is_finite()
            && (snap == 0.0 || (MIN_LENGTH_SNAP..=MAX_LENGTH_SNAP).contains(&snap)))
        {
            return Err(format!(
                "length_snap must be 0 (off) or from {MIN_LENGTH_SNAP} to {MAX_LENGTH_SNAP} meters"
            ));
        }
        if self.angle_precision > MAX_ANGLE_PRECISION {
            return Err(format!(
                "angle_precision must be from 0 to {MAX_ANGLE_PRECISION}"
            ));
        }
        if !(self.angle_snap.is_finite() && (0.0..=MAX_ANGLE_SNAP).contains(&self.angle_snap)) {
            return Err(format!(
                "angle_snap must be from 0 (free angles) to {MAX_ANGLE_SNAP} degrees"
            ));
        }
        Ok(())
    }

    /// Whether a file of these units needs schema 17: a unit or a setting that builds
    /// before it refuse (`feet`, a length snap, an angle precision or an angle step that is
    /// not the default).
    pub fn needs_snap_schema(&self) -> bool {
        let d = Units::default();
        self.length == LengthUnit::Feet
            || self.length_snap != d.length_snap
            || self.angle_precision != d.angle_precision
            || self.angle_snap != d.angle_snap
    }
}

/// `meters` written in `units`: rounded to the precision, trailing zeros of the decimals
/// dropped ("1.5 m", "12 cm", `12.25"`, `4.1'`, `5' 3 1/2"`). Zero and values that round to
/// zero have no sign. Values that are not finite are written as Rust writes them, with the
/// unit.
pub fn format_length(meters: f64, units: &Units) -> String {
    let precision = units.precision.min(MAX_PRECISION);
    let (scale, suffix) = match units.length {
        LengthUnit::Meters => (1.0, " m"),
        LengthUnit::Centimeters => (100.0, " cm"),
        LengthUnit::Millimeters => (1000.0, " mm"),
        LengthUnit::Inches => (1.0 / INCH, "\""),
        LengthUnit::Feet => (1.0 / FOOT, "'"),
        LengthUnit::FeetInches => return feet_inches(meters / INCH, precision),
    };
    format!("{}{suffix}", decimals(meters * scale, precision))
}

/// `degrees` written with the angle precision of `units`, trailing zeros dropped, no sign
/// on zero: "22.5°".
pub fn format_angle(degrees: f64, units: &Units) -> String {
    let precision = units.angle_precision.min(MAX_ANGLE_PRECISION);
    format!("{}°", decimals(degrees, precision))
}

/// Parses a typed length into meters. A bare number is in the model's unit (inches for
/// `feet_inches`); a suffix always wins: `m`, `cm`, `mm`, `in` or `"`, `ft` or `'`. Feet and
/// inches may be written `3'6"`, `3' 6"`, `3'6`, `3'-6"`, and inches may be a fraction or a
/// mixed number: `1/2"`, `6 1/2"`, `3'6 1/2"`, `3' 6-1/2"`. A leading `-` negates; spaces
/// around are ignored. Anything else is an error naming the text.
pub fn parse_length(text: &str, units: &Units) -> Result<f64, String> {
    let bad = || format!("cannot read a length from {text:?}");
    let s = text.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest.trim_start()),
        None => (1.0, s),
    };
    let meters = match s.split_once('\'') {
        Some((feet, inches)) => {
            let feet = quantity(feet).ok_or_else(bad)?;
            feet * FOOT + inches_after_feet(inches, true).ok_or_else(bad)? * INCH
        }
        None => {
            let split = s
                .find(|c: char| c.is_alphabetic() || c == '"')
                .unwrap_or(s.len());
            let (value, suffix) = s.split_at(split);
            let suffix = suffix.trim().to_ascii_lowercase();
            match (suffix.as_str(), suffix.strip_prefix("ft")) {
                // Feet then inches in words: `3ft 6in`.
                (_, Some(inches)) if !inches.trim().is_empty() => {
                    let inches = inches_after_feet(inches, false).ok_or_else(bad)?;
                    quantity(value).ok_or_else(bad)? * FOOT + inches * INCH
                }
                _ => {
                    let scale = match suffix.as_str() {
                        "" => match units.length {
                            LengthUnit::Meters => 1.0,
                            LengthUnit::Centimeters => 0.01,
                            LengthUnit::Millimeters => 0.001,
                            LengthUnit::Inches | LengthUnit::FeetInches => INCH,
                            LengthUnit::Feet => FOOT,
                        },
                        "m" => 1.0,
                        "cm" => 0.01,
                        "mm" => 0.001,
                        "in" | "\"" => INCH,
                        "ft" => FOOT,
                        _ => return Err(bad()),
                    };
                    quantity(value).ok_or_else(bad)? * scale
                }
            }
        }
    };
    if meters.is_finite() {
        // `-0` is 0, not a negative zero.
        Ok(if meters == 0.0 { 0.0 } else { sign * meters })
    } else {
        Err(bad())
    }
}

/// The inches after a foot mark or `ft`, as a number: `6`, `6"`, `6 1/2in`. With `mark` (a
/// `'` came first) they may be empty (`3'`) and may follow a separating dash (`3'-6"`, but
/// not a dash alone); after `ft` they need an `in` or `"`.
fn inches_after_feet(text: &str, mark: bool) -> Option<f64> {
    let text = text.trim();
    let (dashed, text) = match text.strip_prefix('-') {
        Some(rest) if mark => (true, rest.trim()),
        _ => (false, text),
    };
    let (inch_mark, text) = match text.strip_suffix('"') {
        Some(rest) => (true, rest),
        None => {
            let lower = text.to_ascii_lowercase();
            match lower.strip_suffix("in") {
                Some(rest) => (true, &text[..rest.len()]),
                None => (false, text),
            }
        }
    };
    let text = text.trim();
    if text.is_empty() {
        return (mark && !dashed && !inch_mark).then_some(0.0);
    }
    (mark || inch_mark).then(|| quantity(text)).flatten()
}

/// Parses a typed angle in degrees: "90", "-45", "22.5", optionally ending in "°" or "deg".
/// Anything else is an error naming the text.
pub fn parse_angle(text: &str) -> Result<f64, String> {
    let s = text.trim();
    let s = s
        .strip_suffix('°')
        .or_else(|| s.strip_suffix("deg"))
        .unwrap_or(s)
        .trim_end();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest.trim_start()),
        None => (1.0, s),
    };
    number(s)
        .filter(|a| a.is_finite())
        .map(|a| sign * a)
        .ok_or_else(|| format!("cannot read an angle from {text:?}"))
}

/// A non-negative number, a fraction `1/2`, or a mixed number `6 1/2` or `6-1/2`.
fn quantity(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some((whole, part)) = s.split_once([' ', '-']) {
        let whole = whole.trim();
        let part = part.trim();
        return (part.contains('/') && whole.chars().all(|c| c.is_ascii_digit()))
            .then(|| Some(number(whole)? + fraction(part)?))
            .flatten();
    }
    if s.contains('/') {
        fraction(s)
    } else {
        number(s)
    }
}

/// `n/d` with whole numbers and `d` not zero (`3/2` is allowed).
fn fraction(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    fn whole(t: &str) -> Option<f64> {
        (!t.is_empty() && t.chars().all(|c| c.is_ascii_digit())).then(|| t.parse().ok())?
    }
    let (n, d) = (whole(n)?, whole(d)?);
    (d != 0.0).then(|| n / d)
}

/// A plain decimal number: digits and at most the dots `parse` takes.
fn number(s: &str) -> Option<f64> {
    let digits = !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');
    if digits { s.parse().ok() } else { None }
}

/// `value` with at most `precision` decimals, trailing zeros dropped and no "-0".
fn decimals(value: f64, precision: u8) -> String {
    let s = format!("{value:.*}", usize::from(precision));
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    match s {
        "-0" => "0".to_string(),
        s => s.to_string(),
    }
}

/// `inches` as feet, whole inches and a reduced fraction of 1/2^precision inch, rounded to
/// the nearest fraction (so 11 63/64" at 1/16 carries to 1').
fn feet_inches(inches: f64, precision: u8) -> String {
    if !inches.is_finite() {
        return format!("{inches}\"");
    }
    let denominator = 1u64 << precision;
    let steps = (inches.abs() * denominator as f64).round() as u64;
    let (whole, mut numerator) = (steps / denominator, steps % denominator);
    let mut below = denominator;
    while numerator > 0 && numerator.is_multiple_of(2) {
        numerator /= 2;
        below /= 2;
    }
    let (feet, inch) = (whole / 12, whole % 12);
    let sign = if steps > 0 && inches < 0.0 { "-" } else { "" };
    let fraction = (numerator > 0).then(|| format!("{numerator}/{below}"));
    let inches_part = match (inch, fraction) {
        (0, None) => None,
        (0, Some(f)) => Some(format!("{f}\"")),
        (i, None) => Some(format!("{i}\"")),
        (i, Some(f)) => Some(format!("{i} {f}\"")),
    };
    match (feet, inches_part) {
        (0, None) => "0\"".to_string(),
        (0, Some(i)) => format!("{sign}{i}"),
        (f, None) => format!("{sign}{f}'"),
        (f, Some(i)) => format!("{sign}{f}' {i}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(length: LengthUnit, precision: u8) -> Units {
        Units {
            length,
            precision,
            ..Units::default()
        }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn feet_are_decimal_with_a_foot_mark() {
        use LengthUnit::Feet;
        assert_eq!(format_length(1.25, &u(Feet, 1)), "4.1'");
        assert_eq!(format_length(0.3048, &u(Feet, 3)), "1'");
        assert_eq!(format_length(-0.0001, &u(Feet, 2)), "0'");
        assert_eq!(format_length(3.048, &u(Feet, 0)), "10'");
    }

    #[test]
    fn angles_use_the_angle_precision_and_drop_zeros() {
        let d = Units::default();
        assert_eq!(format_angle(22.5, &d), "22.5°");
        assert_eq!(format_angle(90.0, &d), "90°");
        assert_eq!(format_angle(-0.01, &d), "0°");
        assert_eq!(format_angle(12.34567, &d), "12.3°");
        let p = |angle_precision| Units {
            angle_precision,
            ..Units::default()
        };
        assert_eq!(format_angle(12.34567, &p(0)), "12°");
        assert_eq!(format_angle(12.34567, &p(2)), "12.35°");
        assert_eq!(format_angle(12.34567, &p(3)), "12.346°");
        assert_eq!(format_angle(45.0, &p(3)), "45°");
    }

    #[test]
    fn bare_numbers_are_in_the_model_unit() {
        use LengthUnit::*;
        let cases = [
            (Meters, 2.5),
            (Centimeters, 0.025),
            (Millimeters, 0.0025),
            (Inches, 0.0635),
            (FeetInches, 0.0635),
            (Feet, 0.762),
        ];
        for (length, meters) in cases {
            let got = parse_length("2.5", &u(length, 3)).unwrap();
            assert!(near(got, meters), "{length:?}: {got}");
        }
    }

    #[test]
    fn explicit_suffixes_win_over_the_model_unit() {
        use LengthUnit::*;
        let cases = [
            ("2m", 2.0),
            ("250mm", 0.25),
            ("25cm", 0.25),
            ("10in", 0.254),
            ("10\"", 0.254),
            ("3ft", 0.9144),
            ("3'", 0.9144),
            ("10 IN", 0.254),
            ("1.5 M", 1.5),
        ];
        for length in [Meters, Millimeters, Inches, FeetInches, Feet] {
            for (text, meters) in cases {
                let got = parse_length(text, &u(length, 3)).unwrap();
                assert!(near(got, meters), "{length:?} {text:?}: {got}");
            }
        }
    }

    #[test]
    fn feet_and_inches_forms() {
        let d = Units::default();
        let want = (3.0 * 12.0 + 6.5) * 0.0254;
        for text in [
            "3'6 1/2\"",
            "3' 6 1/2\"",
            "3'6-1/2\"",
            "3' 6-1/2\"",
            "3'-6 1/2\"",
            "3'6.5\"",
            "3'6.5",
            "3' 6.5in",
        ] {
            let got = parse_length(text, &d).unwrap();
            assert!(near(got, want), "{text:?}: {got}");
        }
        assert!(near(parse_length("3'6\"", &d).unwrap(), 42.0 * 0.0254));
        assert!(near(parse_length("3' 6\"", &d).unwrap(), 42.0 * 0.0254));
        assert!(near(parse_length("3'6", &d).unwrap(), 42.0 * 0.0254));
        assert!(near(parse_length("3'-6\"", &d).unwrap(), 42.0 * 0.0254));
        assert!(near(parse_length("-3'6\"", &d).unwrap(), -42.0 * 0.0254));
        assert!(near(parse_length("1.5'", &d).unwrap(), 18.0 * 0.0254));
    }

    #[test]
    fn fractions() {
        let d = Units::default();
        assert!(near(parse_length("1/2\"", &d).unwrap(), 0.0127));
        assert!(near(parse_length("6 1/2\"", &d).unwrap(), 6.5 * 0.0254));
        assert!(near(parse_length("6-1/2\"", &d).unwrap(), 6.5 * 0.0254));
        assert!(near(parse_length("-6 1/2in", &d).unwrap(), -6.5 * 0.0254));
        let inches = u(LengthUnit::FeetInches, 4);
        assert!(near(parse_length("1/4", &inches).unwrap(), 0.25 * 0.0254));
        assert!(near(
            parse_length("2 3/8", &inches).unwrap(),
            2.375 * 0.0254
        ));
        assert!(near(parse_length(" 1/2 ft ", &d).unwrap(), 6.0 * 0.0254));
    }

    #[test]
    fn leading_minus_and_spaces() {
        let d = Units::default();
        assert!(near(parse_length("  -2.5 ", &d).unwrap(), -2.5));
        assert!(near(parse_length("- 2.5 cm", &d).unwrap(), -0.025));
    }

    #[test]
    fn bad_lengths_name_the_text() {
        let d = Units::default();
        for bad in [
            "",
            "abc",
            "1.2.3",
            "5 furlongs",
            "1/0\"",
            "1/",
            "/2",
            "3'x",
            "--2",
            "2 m m",
            "'",
            "3'6 1/\"",
            "1e400",
            "inf",
            "NaN",
            "1/2/3",
            "3'4'5",
        ] {
            let e = parse_length(bad, &d).unwrap_err();
            assert!(e.contains(&format!("{bad:?}")), "{bad:?}: {e}");
        }
    }

    #[test]
    fn what_is_written_reads_back_for_every_format() {
        use LengthUnit::*;
        let values = [
            0.0, 0.0254, 0.3048, 1.0, -2.75, 0.0127, 1234.5678, 3.5052, 1e-6,
        ];
        for length in [Meters, Centimeters, Millimeters, Inches, Feet, FeetInches] {
            for precision in 0..=MAX_PRECISION {
                let units = u(length, precision);
                // Half a step at the finest precision, in meters, bounds the round trip.
                let step = match length {
                    FeetInches => INCH / f64::from(1u32 << precision),
                    Meters => 10f64.powi(-i32::from(precision)),
                    Centimeters => 0.01 * 10f64.powi(-i32::from(precision)),
                    Millimeters => 0.001 * 10f64.powi(-i32::from(precision)),
                    Inches => INCH * 10f64.powi(-i32::from(precision)),
                    Feet => 0.3048 * 10f64.powi(-i32::from(precision)),
                };
                for v in values {
                    let text = format_length(v, &units);
                    let back = parse_length(&text, &units)
                        .unwrap_or_else(|e| panic!("{length:?}/{precision} {v}: {text:?}: {e}"));
                    assert!(
                        (back - v).abs() <= step / 2.0 + 1e-9,
                        "{length:?}/{precision} {v}: {text:?} -> {back}"
                    );
                }
            }
        }
    }

    #[test]
    fn fractions_reduce_and_carry_into_feet() {
        let u4 = u(LengthUnit::FeetInches, 4);
        assert_eq!(
            format_length(3.5 * 12.0 * INCH + 0.5 * INCH, &u4),
            "3' 6 1/2\""
        );
        assert_eq!(format_length(11.99 * INCH, &u4), "1'");
        assert_eq!(format_length(-(11.99 * INCH), &u4), "-1'");
        assert_eq!(format_length(0.5 * INCH, &u4), "1/2\"");
        assert_eq!(format_length(1.0e6, &u4).chars().last(), Some('"'));
    }

    #[test]
    fn angles_parse_with_sign_and_marks() {
        for (text, degrees) in [
            ("90", 90.0),
            ("-45", -45.0),
            ("22.5", 22.5),
            ("30°", 30.0),
            ("30 deg", 30.0),
            (" -12.5 ° ", -12.5),
        ] {
            assert!(near(parse_angle(text).unwrap(), degrees), "{text:?}");
        }
        for bad in ["", "abc", "1.2.3", "°", "1e999", "30 rad"] {
            let e = parse_angle(bad).unwrap_err();
            assert!(e.contains(&format!("{bad:?}")), "{bad:?}: {e}");
        }
    }

    #[test]
    fn settings_are_checked() {
        let ok = Units::default();
        assert!(ok.check().is_ok());
        let bad = [
            Units { precision: 7, ..ok },
            Units {
                length_snap: -1.0,
                ..ok
            },
            Units {
                length_snap: 1000.5,
                ..ok
            },
            Units {
                length_snap: f64::NAN,
                ..ok
            },
            Units {
                length_snap: 1e-320,
                ..ok
            },
            Units {
                length_snap: 1e-7,
                ..ok
            },
            Units {
                angle_precision: 4,
                ..ok
            },
            Units {
                angle_snap: -5.0,
                ..ok
            },
            Units {
                angle_snap: 90.5,
                ..ok
            },
            Units {
                angle_snap: f64::INFINITY,
                ..ok
            },
        ];
        for b in bad {
            assert!(b.check().is_err(), "{b:?}");
        }
        assert!(
            Units {
                length_snap: MIN_LENGTH_SNAP,
                ..ok
            }
            .check()
            .is_ok()
        );
        let edge = Units {
            length_snap: 1000.0,
            angle_snap: 90.0,
            angle_precision: 3,
            precision: 6,
            ..ok
        };
        assert!(edge.check().is_ok());
    }

    #[test]
    fn the_api_reports_every_field_and_the_file_skips_defaults() {
        let all = serde_json::to_value(Units::default()).unwrap();
        assert_eq!(
            all,
            serde_json::json!({"length": "meters", "precision": 3, "length_snap": 0.0,
                "angle_precision": 1, "angle_snap": 15.0})
        );
        let stored = |u: Units| serde_json::to_value(StoredUnits::from(u)).unwrap();
        assert_eq!(
            stored(Units::default()),
            serde_json::json!({"length": "meters", "precision": 3})
        );
        let old: StoredUnits =
            serde_json::from_str(r#"{"length":"inches","precision":2}"#).unwrap();
        let old = Units::from(old);
        assert_eq!(old.angle_snap, 15.0);
        assert_eq!(old.angle_precision, 1);
        assert_eq!(old.length_snap, 0.0);
        let new = Units {
            length_snap: 0.05,
            angle_snap: 0.0,
            angle_precision: 2,
            ..Units::default()
        };
        let json = stored(new);
        assert_eq!(
            json,
            serde_json::json!({"length": "meters", "precision": 3, "length_snap": 0.05,
                "angle_precision": 2, "angle_snap": 0.0})
        );
        assert_eq!(
            Units::from(serde_json::from_value::<StoredUnits>(json).unwrap()),
            new
        );
        assert!(
            serde_json::from_str::<StoredUnits>(r#"{"length":"meters","precision":3,"x":1}"#)
                .is_err()
        );
    }

    #[test]
    fn suffix_case_dangling_separators_and_zero() {
        let d = Units::default();
        let want = (3.0 * 12.0 + 6.0) * 0.0254;
        for text in [
            "3'6 In",
            "3'6IN",
            "3' 6 in",
            "3ft 6in",
            "3 ft 6 in",
            "3FT 6 In",
            "3ft6in",
            "3 ft 6\"",
        ] {
            let got = parse_length(text, &d).unwrap_or_else(|e| panic!("{text:?}: {e}"));
            assert!(near(got, want), "{text:?}: {got}");
        }
        assert!(near(
            parse_length("3ft 6 1/2in", &d).unwrap(),
            (36.0 + 6.5) * 0.0254
        ));
        assert!(near(parse_length("-3ft 6in", &d).unwrap(), -want));
        assert!(near(parse_length("3ft", &d).unwrap(), 3.0 * 0.3048));
        for bad in [
            "3'-",
            "3'-\"",
            "3' -",
            "3ft 6",
            "3ft 6m",
            "3ft 6ft",
            "3ft in",
            "3in 6ft",
            "3ft 6in 2in",
        ] {
            assert!(parse_length(bad, &d).is_err(), "{bad:?}");
        }
        for zero in ["-0", "-0m", "-0'", "-0.0 cm"] {
            let got = parse_length(zero, &d).unwrap();
            assert!(got == 0.0 && got.is_sign_positive(), "{zero:?}");
        }
        assert!(near(parse_length("3/2\"", &d).unwrap(), 1.5 * 0.0254));
        assert!(near(parse_length("1 3/2\"", &d).unwrap(), 2.5 * 0.0254));
    }

    #[test]
    fn only_new_settings_need_schema_17() {
        let d = Units::default();
        assert!(!d.needs_snap_schema());
        assert!(!u(LengthUnit::FeetInches, 4).needs_snap_schema());
        assert!(u(LengthUnit::Feet, 3).needs_snap_schema());
        assert!(
            Units {
                length_snap: 0.1,
                ..d
            }
            .needs_snap_schema()
        );
        assert!(
            Units {
                angle_precision: 0,
                ..d
            }
            .needs_snap_schema()
        );
        assert!(
            Units {
                angle_snap: 5.0,
                ..d
            }
            .needs_snap_schema()
        );
    }
}
