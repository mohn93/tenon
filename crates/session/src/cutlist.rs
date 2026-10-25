//! A cut list as text: CSV (RFC 4180) and the lengths a table shows, in the model's display
//! units.

use crate::pdf::{Pdf, text_width_mm};
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
/// doubled; and, as a spreadsheet would run a text that starts with `=`, `+`, `-` or `@` (or a
/// tab or carriage return before one) as a formula, led by an apostrophe.
pub fn csv_text(text: &str) -> String {
    let guarded;
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
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

/// Text size of a table page, in points, and the title's.
const PAGE_PT: f64 = 9.0;
const TITLE_PT: f64 = 13.0;
/// Margin of a table page and the height of a row, in millimetres.
const PAGE_MARGIN_MM: f64 = 15.0;
const ROW_MM: f64 = 6.0;
const RULE_MM: f64 = 0.15;
/// Where the columns start, as fractions of the width between the margins: name, material,
/// then the right edges of length, width, thickness and quantity.
const NAME_AT: f64 = 0.0;
const MATERIAL_AT: f64 = 0.34;
const RIGHT_EDGES: [f64; 4] = [0.70, 0.82, 0.94, 1.0];

/// `text` cut with ".." so it fits `max_mm` at `PAGE_PT`.
fn fitted(text: &str, max_mm: f64) -> String {
    if text_width_mm(text, PAGE_PT) <= max_mm {
        return text.to_string();
    }
    let mut cut: Vec<char> = text.chars().collect();
    while !cut.is_empty()
        && text_width_mm(&format!("{}..", cut.iter().collect::<String>()), PAGE_PT) > max_mm
    {
        cut.pop();
    }
    format!("{}..", cut.into_iter().collect::<String>())
}

/// Adds the cut list to `pdf` as pages of `page_mm` (width, height): a title, a header row, a
/// row per part with its sizes in the model's units, and a footer with the page number. As many
/// pages as the rows need (one for an empty list). Returns the pages added.
pub fn draw_pages(pdf: &mut Pdf, rows: &[CutRowInfo], units: &Units, page_mm: [f64; 2]) -> usize {
    let [pw, ph] = page_mm;
    let width = pw - 2.0 * PAGE_MARGIN_MM;
    let top = PAGE_MARGIN_MM + 12.0;
    let per_page = (((ph - top - PAGE_MARGIN_MM - ROW_MM) / ROW_MM).floor() as usize).max(1);
    let pages = rows.len().div_ceil(per_page).max(1);
    let x = |f: f64| PAGE_MARGIN_MM + width * f;
    let name_room = x(MATERIAL_AT) - x(NAME_AT) - 2.0;
    for n in 0..pages {
        let mut page = pdf.page(page_mm);
        page.text([PAGE_MARGIN_MM, PAGE_MARGIN_MM + 4.0], TITLE_PT, "Cut list");
        let heads = header(units);
        let cell = |page: &mut crate::pdf::PageBuilder, y: f64, cells: [&str; 6]| {
            page.text([x(NAME_AT), y], PAGE_PT, &fitted(cells[0], name_room));
            let material_room = x(RIGHT_EDGES[0] - 0.14) - x(MATERIAL_AT) - 2.0;
            page.text(
                [x(MATERIAL_AT), y],
                PAGE_PT,
                &fitted(cells[1], material_room),
            );
            for (i, edge) in RIGHT_EDGES.iter().enumerate() {
                let t = cells[2 + i];
                page.text([x(*edge) - text_width_mm(t, PAGE_PT), y], PAGE_PT, t);
            }
        };
        let head_y = top;
        cell(
            &mut page,
            head_y,
            [
                &heads[0], &heads[1], &heads[2], &heads[3], &heads[4], &heads[5],
            ],
        );
        page.line(
            [PAGE_MARGIN_MM, head_y + 1.5],
            [PAGE_MARGIN_MM + width, head_y + 1.5],
            RULE_MM,
        );
        if rows.is_empty() {
            page.text([PAGE_MARGIN_MM, top + ROW_MM * 2.0], PAGE_PT, "No parts");
        }
        for (i, row) in rows.iter().skip(n * per_page).take(per_page).enumerate() {
            let y = top + ROW_MM * (i as f64 + 1.0);
            let cells = table_cells(row, units);
            // A long name is cut, not the mark after it.
            let name = if row.solid {
                cells[0].clone()
            } else {
                let mark = " (not solid)";
                let room = name_room - text_width_mm(mark, PAGE_PT);
                format!("{}{mark}", fitted(&cells[0], room))
            };
            cell(
                &mut page,
                y,
                [&name, &cells[1], &cells[2], &cells[3], &cells[4], &cells[5]],
            );
        }
        let footer = format!("Cut list, page {} of {pages}", n + 1);
        page.text(
            [
                PAGE_MARGIN_MM + width - text_width_mm(&footer, PAGE_PT),
                ph - PAGE_MARGIN_MM / 2.0,
            ],
            PAGE_PT,
            &footer,
        );
    }
    pages
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
        assert_eq!(csv_text("\t=1"), "'\t=1");
        assert_eq!(csv_text("\r=1"), "\"'\r=1\"");
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
    fn the_pdf_pages_follow_the_row_count() {
        let rows: Vec<CutRowInfo> = (0..100).map(|i| row(&format!("Part {i}"), None)).collect();
        let mut pdf = Pdf::new();
        assert_eq!(draw_pages(&mut pdf, &[], &mm(), [297.0, 210.0]), 1);
        let few = draw_pages(&mut pdf, &rows[..5], &mm(), [297.0, 210.0]);
        assert_eq!(few, 1);
        let many = draw_pages(&mut pdf, &rows, &mm(), [297.0, 210.0]);
        assert!(many > 2, "{many}");
        assert_eq!(pdf.page_count(), 1 + few + many);
        let bytes = pdf.finish();
        assert!(bytes.starts_with(b"%PDF-"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("(Cut list) Tj"));
        assert!(text.contains("(Length \\(mm\\)) Tj"), "units in the header");
        assert!(text.contains("(1200 mm) Tj"));
    }

    #[test]
    fn long_names_are_cut_to_fit_their_column() {
        let long = "x".repeat(200);
        let cut = fitted(&long, 30.0);
        assert!(
            cut.ends_with("..") && text_width_mm(&cut, PAGE_PT) <= 30.0,
            "{cut}"
        );
        assert_eq!(fitted("Leg", 30.0), "Leg");
    }

    #[test]
    fn a_long_name_that_is_not_solid_keeps_its_mark_on_the_page() {
        let mut r = row(&"x".repeat(200), None);
        r.solid = false;
        let mut pdf = Pdf::new();
        draw_pages(&mut pdf, &[r], &mm(), [297.0, 210.0]);
        let text = String::from_utf8_lossy(&pdf.finish()).into_owned();
        assert!(text.contains(".. \\(not solid\\)) Tj"), "{text}");
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
