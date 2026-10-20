//! The model's display units: how dimension and note labels write lengths.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Meters in an inch.
const INCH: f64 = 0.0254;

/// The most `precision` may be.
pub const MAX_PRECISION: u8 = 6;

/// The unit labels write lengths in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    #[default]
    Meters,
    Centimeters,
    Millimeters,
    Inches,
    /// Feet and inches with fractions of an inch: `5' 3 1/2"`.
    FeetInches,
}

/// How labels write lengths: the unit, and `precision`, the decimals kept (0 to 6), or for
/// `feet_inches` the finest fraction of an inch, 1/2^precision (0: whole inches, 6: 1/64).
/// Saved with the model; the default is meters with 3 decimals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Units {
    pub length: LengthUnit,
    pub precision: u8,
}

impl Default for Units {
    fn default() -> Self {
        Units {
            length: LengthUnit::Meters,
            precision: 3,
        }
    }
}

/// `meters` written in `units`: rounded to the precision, trailing zeros of the decimals
/// dropped ("1.5 m", "12 cm", `12.25"`, `5' 3 1/2"`). Zero and values that round to zero
/// have no sign. Values that are not finite are written as Rust writes them, with the unit.
pub fn format_length_in(meters: f64, units: &Units) -> String {
    let precision = units.precision.min(MAX_PRECISION);
    let (scale, suffix) = match units.length {
        LengthUnit::Meters => (1.0, " m"),
        LengthUnit::Centimeters => (100.0, " cm"),
        LengthUnit::Millimeters => (1000.0, " mm"),
        LengthUnit::Inches => (1.0 / INCH, "\""),
        LengthUnit::FeetInches => return feet_inches(meters / INCH, precision),
    };
    format!("{}{suffix}", decimals(meters * scale, precision))
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

/// Refuses a precision past [`MAX_PRECISION`].
pub(crate) fn check_precision(precision: u8) -> Result<(), String> {
    if precision > MAX_PRECISION {
        return Err(format!(
            "precision must be from 0 to {MAX_PRECISION} (for feet_inches: fractions down to 1/64 inch)"
        ));
    }
    Ok(())
}
