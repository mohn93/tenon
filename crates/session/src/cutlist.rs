//! A cut list as text: CSV (RFC 4180) and the lengths a table shows, in the model's display
//! units.

use crate::query::CutRowInfo;
use crate::units::{LengthUnit, Units, decimals, format_length};

/// Meters in an inch.
const INCH: f64 = 0.0254;

/// The columns of the CSV and of the table: the header names, without the unit.
pub const COLUMNS: [&str; 8] = [
    "Name",
    "Material",
    "Length",
    "Width",
    "Thickness",
    "Quantity",
    "Solid",
    "Note",
];

/// What the length columns are written in: `mm` for millimeters, `in` for inches and for feet
/// and inches (whose CSV cells are decimal inches), and so on.
pub fn unit_label(units: &Units) -> &'static str {
    match units.length {
        LengthUnit::Meters => "m",
        LengthUnit::Centimeters => "cm",
        LengthUnit::Millimeters => "mm",
        LengthUnit::Inches | LengthUnit::FeetInches => "in",
        LengthUnit::Feet => "ft",
    }
}

/// `meters` as a plain number in the unit of `unit_label`, with the model's precision
/// (three decimals of an inch for feet and inches), trailing zeros dropped.
pub fn csv_length(meters: f64, units: &Units) -> String {
    let (scale, precision) = match units.length {
        LengthUnit::Meters => (1.0, units.precision),
        LengthUnit::Centimeters => (100.0, units.precision),
        LengthUnit::Millimeters => (1000.0, units.precision),
        LengthUnit::Inches => (1.0 / INCH, units.precision),
        LengthUnit::Feet => (1.0 / (12.0 * INCH), units.precision),
        LengthUnit::FeetInches => (1.0 / INCH, 3),
    };
    decimals(meters * scale, precision.min(crate::units::MAX_PRECISION))
}

/// `text` as a CSV field: quoted when it holds a comma, a quote or a line break, with quotes
/// doubled; and, as a spreadsheet would run a text that starts with `=`, `+`, `-` or `@` as a
/// formula, led by an apostrophe.
pub fn csv_text(text: &str) -> String {
    let guarded;
    let text = if text.starts_with(['=', '+', '-', '@']) {
        guarded = format!("'{text}");
        &guarded
    } else {
        text
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// The header line's cells: the columns with the length unit on the sizes.
pub fn header(units: &Units) -> Vec<String> {
    let unit = unit_label(units);
    COLUMNS
        .iter()
        .enumerate()
        .map(|(i, c)| match i {
            2..=4 => format!("{c} ({unit})"),
            _ => (*c).to_string(),
        })
        .collect()
}

/// The cut list as CSV: a header, then a line per row, each line ended by CRLF. Sizes are plain
/// numbers in the display unit named in the header, `Solid` is `yes` or `no`.
pub fn to_csv(rows: &[CutRowInfo], units: &Units) -> String {
    let mut out = String::new();
    let line = |cells: Vec<String>| cells.join(",") + "\r\n";
    out += &line(header(units).iter().map(|c| csv_text(c)).collect());
    for r in rows {
        out += &line(vec![
            csv_text(&r.name),
            csv_text(r.material.as_deref().unwrap_or("")),
            csv_length(r.length, units),
            csv_length(r.width, units),
            csv_length(r.thickness, units),
            r.quantity.to_string(),
            if r.solid { "yes" } else { "no" }.to_string(),
            csv_text(r.note.as_deref().unwrap_or("")),
        ]);
    }
    out
}

/// A row's cells as a table shows them: name, material (blank when none), the three sizes as
/// the model writes lengths, and the quantity.
pub fn table_cells(row: &CutRowInfo, units: &Units) -> [String; 6] {
    [
        row.name.clone(),
        row.material.clone().unwrap_or_default(),
        format_length(row.length, units),
        format_length(row.width, units),
        format_length(row.thickness, units),
        row.quantity.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use kernel::EntityId;

    use super::*;

    fn row(name: &str, material: Option<&str>) -> CutRowInfo {
        CutRowInfo {
            name: name.to_string(),
            material: material.map(str::to_string),
            length: 1.2,
            width: 0.0254,
            thickness: 0.019,
            quantity: 3,
            solid: true,
            note: None,
            ids: vec![EntityId::Instance(kernel::InstanceId(1))],
        }
    }

    fn mm() -> Units {
        Units {
            length: LengthUnit::Millimeters,
            precision: 1,
            ..Units::default()
        }
    }

    #[test]
    fn fields_with_commas_quotes_and_line_breaks_are_quoted_and_doubled() {
        assert_eq!(csv_text("Shelf"), "Shelf");
        assert_eq!(csv_text("Leg, front"), "\"Leg, front\"");
        assert_eq!(csv_text("5\" dowel"), "\"5\"\" dowel\"");
        assert_eq!(csv_text("a\nb"), "\"a\nb\"");
        assert_eq!(csv_text("a\rb"), "\"a\rb\"");
        assert_eq!(csv_text("say \"hi\", ok"), "\"say \"\"hi\"\", ok\"");
    }

    #[test]
    fn a_text_that_would_run_as_a_formula_is_led_by_an_apostrophe() {
        assert_eq!(csv_text("=1+1"), "'=1+1");
        assert_eq!(csv_text("-x"), "'-x");
        assert_eq!(csv_text("@sum"), "'@sum");
        assert_eq!(csv_text("+1,2"), "\"'+1,2\"");
    }

    #[test]
    fn the_csv_has_a_header_with_the_unit_and_crlf_lines() {
        let csv = to_csv(&[row("Leg, front", Some("Oak")), row("Top", None)], &mm());
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(
            lines[0],
            "Name,Material,Length (mm),Width (mm),Thickness (mm),Quantity,Solid,Note"
        );
        assert_eq!(lines[1], "\"Leg, front\",Oak,1200,25.4,19,3,yes,");
        assert_eq!(lines[2], "Top,,1200,25.4,19,3,yes,");
        assert_eq!(lines[3], "");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_part_that_is_not_solid_is_marked_with_its_note() {
        let mut r = row("Sheet", None);
        r.solid = false;
        r.note = Some("edge e3 bounds 1 face, open".to_string());
        let csv = to_csv(&[r], &mm());
        assert!(
            csv.ends_with(",no,\"edge e3 bounds 1 face, open\"\r\n"),
            "{csv}"
        );
    }

    #[test]
    fn sizes_follow_the_models_units() {
        let r = row("Board", None);
        let inches = Units {
            length: LengthUnit::Inches,
            precision: 2,
            ..Units::default()
        };
        assert_eq!(header(&inches)[2], "Length (in)");
        assert_eq!(csv_length(r.width, &inches), "1");
        assert_eq!(csv_length(r.length, &inches), "47.24");
        let feet_inches = Units {
            length: LengthUnit::FeetInches,
            precision: 4,
            ..Units::default()
        };
        assert_eq!(header(&feet_inches)[3], "Width (in)");
        assert_eq!(csv_length(r.length, &feet_inches), "47.244");
        let metres = Units::default();
        assert_eq!(header(&metres)[4], "Thickness (m)");
        assert_eq!(csv_length(r.thickness, &metres), "0.019");
        let feet = Units {
            length: LengthUnit::Feet,
            precision: 2,
            ..Units::default()
        };
        assert_eq!(header(&feet)[2], "Length (ft)");
        assert_eq!(csv_length(0.3048, &feet), "1");
        assert_eq!(table_cells(&r, &inches)[2], "47.24\"");
    }
}
