//! Print layout: paper sizes, fit or exact 1:N scale, tiling across sheets, trim marks, the
//! scale bar, the footer and the label positions. The GPU renders each sheet's tile from its
//! camera and `crate::pdf` assembles the pages; this module only decides what goes where.

use glam::{DVec2, DVec3};
use kernel::MeshId;

use crate::camera::{Camera, Projection};
use crate::command::{CommandError, ErrorKind};
use crate::document::Document;
use crate::drawing::{LabelAnchor, line_drawing};
use crate::pdf::{PageBuilder, text_width_mm};
use crate::units::format_length_in;

/// Blank border of every sheet, in millimetres.
pub const MARGIN_MM: f64 = 10.0;
/// Most sheets one print may span.
pub const MAX_SHEETS: usize = 100;
/// Largest side of one rendered image, in pixels.
pub const MAX_CHUNK_PX: u32 = 4096;
pub const DEFAULT_DPI: u32 = 150;
pub const MIN_DPI: u32 = 72;
pub const MAX_DPI: u32 = 300;
/// Length of a trim mark, in millimetres.
pub const TRIM_MM: f64 = 5.0;
/// Height of the scale bar, in millimetres.
pub const BAR_HEIGHT_MM: f64 = 4.0;
/// Text size of the scale bar, footer and labels, in points.
pub const TEXT_PT: f64 = 8.5;
const BAR_MIN_MM: f64 = 20.0;
const BAR_MAX_MM: f64 = 60.0;
const LINE_MM: f64 = 0.15;
/// Pixels per millimetre of the drawing used to place labels.
const LABEL_PX_PER_MM: f64 = 2.0;
const LABEL_MAX_PX: f64 = 8192.0;
const PT_MM: f64 = 25.4 / 72.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paper {
    A4,
    A3,
    Letter,
    Legal,
    Tabloid,
}

impl Paper {
    /// Portrait size in millimetres (width, height).
    pub fn size_mm(self) -> [f64; 2] {
        match self {
            Paper::A4 => [210.0, 297.0],
            Paper::A3 => [297.0, 420.0],
            Paper::Letter => [215.9, 279.4],
            Paper::Legal => [215.9, 355.6],
            Paper::Tabloid => [279.4, 431.8],
        }
    }

    /// "a4", "a3", "letter", "legal" or "tabloid" (any case).
    pub fn from_name(name: &str) -> Option<Paper> {
        Some(match name.to_ascii_lowercase().as_str() {
            "a4" => Paper::A4,
            "a3" => Paper::A3,
            "letter" => Paper::Letter,
            "legal" => Paper::Legal,
            "tabloid" => Paper::Tabloid,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrintSpec {
    pub paper: Paper,
    pub landscape: bool,
    /// 1:N when given (parallel camera only); absent fits the current view on one sheet.
    pub scale: Option<f64>,
    pub dpi: u32,
}

/// A bar of `length_mm` on paper standing for `length_m`, drawn in the bottom margin: top
/// left at (`x_mm`, `y_mm`), `height_mm` tall, in four alternating black and white quarters.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleBar {
    pub x_mm: f64,
    pub y_mm: f64,
    pub length_m: f64,
    pub length_mm: f64,
    pub height_mm: f64,
    /// "0".
    pub zero_label: String,
    /// The length in display units.
    pub end_label: String,
    /// "1:N".
    pub scale_label: String,
}

#[derive(Clone, Debug)]
pub struct Sheet {
    pub page_mm: [f64; 2],
    /// Where the rendered image goes: [x, y, width, height] from the page's top left. Its
    /// size is `tile_px` at the print resolution.
    pub area_mm: [f64; 4],
    /// Looks at the area's part of the model; its viewport is `tile_px`.
    pub tile_camera: Camera,
    pub tile_px: [u32; 2],
    /// Trim marks at the area's corners (see `trim_lines`): when the print spans sheets.
    pub trim: bool,
    pub scale_bar: Option<ScaleBar>,
    /// "Sheet r,c of R x C" when the print spans sheets.
    pub footer: Option<String>,
}

/// One image of a sheet: at most `MAX_CHUNK_PX` per side.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub camera: Camera,
    pub px: [u32; 2],
    /// Where it goes on the page: [x, y, width, height] in millimetres.
    pub rect_mm: [f64; 4],
}

/// Label text at a position on a sheet, in page millimetres from its top left.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetLabel {
    pub text: String,
    pub at_mm: [f64; 2],
    pub anchor: LabelAnchor,
}

#[derive(Clone, Debug)]
pub struct Layout {
    /// Rows top to bottom, columns left to right.
    pub sheets: Vec<Sheet>,
    pub rows: usize,
    pub cols: usize,
    /// Per sheet (same order as `sheets`): dimension and note text.
    pub labels: Vec<Vec<SheetLabel>>,
    pub warnings: Vec<String>,
}

impl Sheet {
    /// Trim marks as line segments [from, to] in page millimetres: at each corner of the
    /// area, one horizontal and one vertical `TRIM_MM` long, outside the area. Empty unless
    /// `trim`.
    pub fn trim_lines(&self) -> Vec<[[f64; 2]; 2]> {
        if !self.trim {
            return Vec::new();
        }
        let [x, y, w, h] = self.area_mm;
        let mut lines = Vec::with_capacity(8);
        for (cx, dx) in [(x, -1.0), (x + w, 1.0)] {
            for (cy, dy) in [(y, -1.0), (y + h, 1.0)] {
                lines.push([[cx, cy], [cx + dx * TRIM_MM, cy]]);
                lines.push([[cx, cy], [cx, cy + dy * TRIM_MM]]);
            }
        }
        lines
    }

    /// The images to render for this sheet. A parallel tile that exceeds `MAX_CHUNK_PX` is
    /// split in a grid; each chunk keeps the tile's pixel size in the world and its camera's
    /// distance (so the depth range is unchanged), with the field of view set to its height.
    pub fn chunks(&self) -> Vec<Chunk> {
        let [tw, th] = self.tile_px;
        let whole = Chunk {
            camera: self.tile_camera.clone(),
            px: self.tile_px,
            rect_mm: self.area_mm,
        };
        if !self.tile_camera.is_parallel() || (tw <= MAX_CHUNK_PX && th <= MAX_CHUNK_PX) {
            return vec![whole];
        }
        let c = &self.tile_camera;
        let (nx, ny) = (tw.div_ceil(MAX_CHUNK_PX), th.div_ceil(MAX_CHUNK_PX));
        let pixel = c.view_height() / th as f64;
        let (mm_x, mm_y) = (self.area_mm[2] / tw as f64, self.area_mm[3] / th as f64);
        let (right, up, dist) = (c.right(), c.up(), c.distance());
        let mut out = Vec::new();
        for j in 0..ny {
            let (y0, y1) = ((j * th) / ny, ((j + 1) * th) / ny);
            for i in 0..nx {
                let (x0, x1) = ((i * tw) / nx, ((i + 1) * tw) / nx);
                let (w, h) = (x1 - x0, y1 - y0);
                let dx = ((x0 + x1) as f64 * 0.5 - tw as f64 * 0.5) * pixel;
                let dy = -((y0 + y1) as f64 * 0.5 - th as f64 * 0.5) * pixel;
                let mut cam = c.clone();
                let pan = right * dx + up * dy;
                cam.eye += pan;
                cam.target += pan;
                cam.viewport = DVec2::new(w as f64, h as f64);
                cam.fov_y = 2.0 * (pixel * h as f64 / (2.0 * dist)).atan();
                out.push(Chunk {
                    camera: cam,
                    px: [w, h],
                    rect_mm: [
                        self.area_mm[0] + x0 as f64 * mm_x,
                        self.area_mm[1] + y0 as f64 * mm_y,
                        w as f64 * mm_x,
                        h as f64 * mm_y,
                    ],
                });
            }
        }
        out
    }
}

/// Draws a sheet's trim marks, scale bar, footer and labels onto its page (the image is the
/// caller's). Text widths are estimated, so centring and right alignment are approximate.
pub fn decorate(page: &mut PageBuilder, sheet: &Sheet, labels: &[SheetLabel]) {
    for [a, b] in sheet.trim_lines() {
        page.line(a, b, LINE_MM);
    }
    let base = sheet.page_mm[1] - 2.0;
    if let Some(bar) = &sheet.scale_bar {
        let quarter = bar.length_mm / 4.0;
        for i in 0..4 {
            let r = [
                bar.x_mm + quarter * i as f64,
                bar.y_mm,
                quarter,
                bar.height_mm,
            ];
            page.rect_fill(r, if i % 2 == 0 { 0.0 } else { 1.0 });
        }
        let (x0, y0, x1, y1) = (
            bar.x_mm,
            bar.y_mm,
            bar.x_mm + bar.length_mm,
            bar.y_mm + bar.height_mm,
        );
        for (a, b) in [
            ([x0, y0], [x1, y0]),
            ([x1, y0], [x1, y1]),
            ([x1, y1], [x0, y1]),
            ([x0, y1], [x0, y0]),
        ] {
            page.line(a, b, LINE_MM);
        }
        let half = |t: &str| text_width_mm(t, TEXT_PT) / 2.0;
        page.text([x0 - half(&bar.zero_label), base], TEXT_PT, &bar.zero_label);
        page.text([x1 - half(&bar.end_label), base], TEXT_PT, &bar.end_label);
        let after = x1 + half(&bar.end_label) + 3.0;
        page.text([after, base], TEXT_PT, &bar.scale_label);
    }
    if let Some(footer) = &sheet.footer {
        let right = sheet.area_mm[0] + sheet.area_mm[2];
        page.text(
            [right - text_width_mm(footer, TEXT_PT), base],
            TEXT_PT,
            footer,
        );
    }
    let step = TEXT_PT * 1.2 * PT_MM;
    for label in labels {
        let lines: Vec<&str> = label.text.split('\n').collect();
        let width = lines
            .iter()
            .map(|l| text_width_mm(l, TEXT_PT))
            .fold(0.0, f64::max);
        let (x, top) = match label.anchor {
            LabelAnchor::Center => (
                label.at_mm[0] - width / 2.0,
                label.at_mm[1] - step * lines.len() as f64 / 2.0,
            ),
            LabelAnchor::TopLeft => (label.at_mm[0], label.at_mm[1]),
        };
        for (i, line) in lines.iter().enumerate() {
            page.text([x, top + step * (i as f64 + 0.8)], TEXT_PT, line);
        }
    }
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

/// The nice bar length (1, 2 or 5 times a power of ten metres) for 1:`n`: the largest whose
/// length on paper is within 20 to 60 mm, else the one closest to that range. Returns the
/// length in metres and on paper in millimetres.
fn bar_length(n: f64) -> (f64, f64) {
    let mut best: Option<(f64, f64, f64)> = None;
    for k in -12..=12 {
        for m in [1.0, 2.0, 5.0] {
            let metres = m * 10f64.powi(k);
            let mm = metres * 1000.0 / n;
            let miss = if mm < BAR_MIN_MM {
                BAR_MIN_MM - mm
            } else if mm > BAR_MAX_MM {
                mm - BAR_MAX_MM
            } else {
                0.0
            };
            if best.is_none_or(|(b, bm, _)| miss < b || (miss == b && metres > bm)) {
                best = Some((miss, metres, mm));
            }
        }
    }
    let (_, metres, mm) = best.expect("candidates exist");
    (metres, mm)
}

/// Lays `camera`'s view of `doc` out on sheets as `spec` says.
pub fn layout(doc: &Document, camera: &Camera, spec: &PrintSpec) -> Result<Layout, CommandError> {
    if !(MIN_DPI..=MAX_DPI).contains(&spec.dpi) {
        return Err(invalid(format!(
            "dpi must be between {MIN_DPI} and {MAX_DPI}"
        )));
    }
    if let Some(n) = spec.scale
        && !(n.is_finite() && n > 0.0)
    {
        return Err(invalid("scale must be a positive number"));
    }
    let [mut pw, mut ph] = spec.paper.size_mm();
    if spec.landscape {
        std::mem::swap(&mut pw, &mut ph);
    }
    let page_mm = [pw, ph];
    let (avail_w, avail_h) = (pw - 2.0 * MARGIN_MM, ph - 2.0 * MARGIN_MM);
    let ppm = spec.dpi as f64 / 25.4;
    let mut warnings = Vec::new();

    let (sheets, rows, cols, whole) = match spec.scale {
        None => {
            let aspect = camera.viewport.x / camera.viewport.y.max(1.0);
            let (mut w, mut h) = (avail_w, avail_w / aspect);
            if h > avail_h {
                (w, h) = (avail_h * aspect, avail_h);
            }
            let mut px = [(w * ppm).floor().max(1.0), (h * ppm).floor().max(1.0)];
            if !camera.is_parallel() {
                let big = px[0].max(px[1]);
                if big > MAX_CHUNK_PX as f64 {
                    let f = MAX_CHUNK_PX as f64 / big;
                    px = [(px[0] * f).floor().max(1.0), (px[1] * f).floor().max(1.0)];
                    warnings.push(format!(
                        "a perspective view is rendered in one image: resolution reduced to {} x {} px",
                        px[0], px[1]
                    ));
                }
            }
            let eff = px[0] / w;
            let (aw, ah) = (px[0] / eff, px[1] / eff);
            let mut tile = camera.clone();
            tile.viewport = DVec2::new(px[0], px[1]);
            let sheet = Sheet {
                page_mm,
                area_mm: [
                    MARGIN_MM + (avail_w - aw) / 2.0,
                    MARGIN_MM + (avail_h - ah) / 2.0,
                    aw,
                    ah,
                ],
                tile_camera: tile.clone(),
                tile_px: [px[0] as u32, px[1] as u32],
                trim: false,
                scale_bar: None,
                footer: None,
            };
            (vec![sheet], 1, 1, tile)
        }
        Some(n) => exact(doc, camera, n, page_mm, ppm)?,
    };

    let labels = place_labels(doc, &sheets, rows, cols, &whole, &mut warnings);
    Ok(Layout {
        sheets,
        rows,
        cols,
        labels,
        warnings,
    })
}

type Tiles = (Vec<Sheet>, usize, usize, Camera);

fn exact(
    doc: &Document,
    camera: &Camera,
    n: f64,
    page_mm: [f64; 2],
    ppm: f64,
) -> Result<Tiles, CommandError> {
    if camera.projection != Projection::Parallel {
        return Err(invalid("Switch to parallel projection to print at scale"));
    }
    let Some([lo, hi]) = doc.scene().shown_bounds(MeshId::Root, doc.context()) else {
        return Err(invalid("nothing to print: the model shows no geometry"));
    };
    let (right, up, forward) = (camera.right(), camera.up(), camera.forward());
    let (mut x0, mut x1, mut y0, mut y1, mut z0, mut z1) =
        (f64::MAX, f64::MIN, f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for k in 0..8 {
        let p = DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        );
        (x0, x1) = (x0.min(p.dot(right)), x1.max(p.dot(right)));
        (y0, y1) = (y0.min(p.dot(up)), y1.max(p.dot(up)));
        (z0, z1) = (z0.min(p.dot(forward)), z1.max(p.dot(forward)));
    }
    let [pw, ph] = page_mm;
    let (avail_w, avail_h) = (pw - 2.0 * MARGIN_MM, ph - 2.0 * MARGIN_MM);
    // The tile's size is a whole number of pixels, so its camera shows exactly its world size.
    let px = [
        (avail_w * ppm).floor().max(1.0),
        (avail_h * ppm).floor().max(1.0),
    ];
    let (aw, ah) = (px[0] / ppm, px[1] / ppm);
    let (tile_w, tile_h) = (aw / 1000.0 * n, ah / 1000.0 * n);
    let cols_f = ((x1 - x0) / tile_w - 1e-9).ceil().max(1.0);
    let rows_f = ((y1 - y0) / tile_h - 1e-9).ceil().max(1.0);
    if cols_f * rows_f > MAX_SHEETS as f64 {
        return Err(invalid(format!(
            "printing at 1:{n} needs {} x {} = {} sheets; the limit is {MAX_SHEETS}",
            rows_f as u64,
            cols_f as u64,
            (rows_f * cols_f) as u64
        )));
    }
    let (cols, rows) = (cols_f as usize, rows_f as usize);
    let depth = forward * ((z0 + z1) / 2.0);
    let camera_at = |x: f64, y: f64, height: f64, viewport: DVec2| {
        let mut c = camera.clone();
        c.target = right * x + up * y + depth;
        c.viewport = viewport;
        let d = height / (2.0 * (camera.fov_y * 0.5).tan());
        c.eye = c.target - forward * d;
        c
    };
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let left = cx - cols as f64 * tile_w / 2.0;
    let top = cy + rows as f64 * tile_h / 2.0;
    let tiled = rows * cols > 1;
    let (bar_m, bar_mm) = bar_length(n);
    let units = doc.units();
    let scale_label = format!("1:{n}");
    let mut sheets = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let x = left + (c as f64 + 0.5) * tile_w;
            let y = top - (r as f64 + 0.5) * tile_h;
            sheets.push(Sheet {
                page_mm,
                area_mm: [MARGIN_MM, MARGIN_MM, aw, ah],
                tile_camera: camera_at(x, y, tile_h, DVec2::new(px[0], px[1])),
                tile_px: [px[0] as u32, px[1] as u32],
                trim: tiled,
                scale_bar: Some(ScaleBar {
                    x_mm: MARGIN_MM,
                    y_mm: ph - MARGIN_MM + 0.5,
                    length_m: bar_m,
                    length_mm: bar_mm,
                    height_mm: BAR_HEIGHT_MM,
                    zero_label: "0".to_string(),
                    end_label: format_length_in(bar_m, &units),
                    scale_label: scale_label.clone(),
                }),
                footer: tiled.then(|| format!("Sheet {},{} of {rows} x {cols}", r + 1, c + 1)),
            });
        }
    }
    let (gw, gh) = (cols as f64 * aw, rows as f64 * ah);
    let s = (LABEL_PX_PER_MM).min(LABEL_MAX_PX / gw.max(gh));
    let whole = camera_at(
        cx,
        cy,
        rows as f64 * tile_h,
        DVec2::new((gw * s).max(1.0), (gh * s).max(1.0)),
    );
    Ok((sheets, rows, cols, whole))
}

/// Dimension and note text per sheet: one drawing of the whole print, low resolution, its
/// label positions cut into the sheets they fall on. A label lies on the sheet of its anchor.
fn place_labels(
    doc: &Document,
    sheets: &[Sheet],
    rows: usize,
    cols: usize,
    whole: &Camera,
    warnings: &mut Vec<String>,
) -> Vec<Vec<SheetLabel>> {
    let mut out = vec![Vec::new(); sheets.len()];
    // Fit mode's camera is the tile's: scale it to the label drawing's size.
    let area = sheets[0].area_mm;
    let s = if rows * cols == 1 {
        (LABEL_PX_PER_MM).min(LABEL_MAX_PX / area[2].max(area[3]))
    } else {
        whole.viewport.x / (cols as f64 * area[2])
    };
    let mut camera = whole.clone();
    camera.viewport = DVec2::new(
        (cols as f64 * area[2] * s).max(1.0),
        (rows as f64 * area[3] * s).max(1.0),
    );
    let drawing = match line_drawing(doc, &camera, camera.viewport) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("labels not placed: {}", e.message));
            return out;
        }
    };
    let (tw, th) = (area[2] * s, area[3] * s);
    for label in &drawing.labels {
        let (c, r) = ((label.at.x / tw).floor(), (label.at.y / th).floor());
        if c < 0.0 || r < 0.0 || c as usize >= cols || r as usize >= rows {
            continue;
        }
        let sheet = &sheets[r as usize * cols + c as usize];
        out[r as usize * cols + c as usize].push(SheetLabel {
            text: label.text.clone(),
            at_mm: [
                sheet.area_mm[0] + (label.at.x - c * tw) / s,
                sheet.area_mm[1] + (label.at.y - r * th) / s,
            ],
            anchor: label.anchor,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_lengths() {
        for (n, m) in [
            (100.0, 5.0),
            (1.0, 0.05),
            (50.0, 2.0),
            (500.0, 20.0),
            (3.0, 0.1),
        ] {
            let (metres, mm) = bar_length(n);
            assert!((metres - m).abs() < 1e-12, "1:{n}: {metres}");
            assert!((mm - metres * 1000.0 / n).abs() < 1e-9);
        }
        // 1:3 -> 0.1 m = 33.3 mm; 0.2 m = 66.7 mm is out of range.
    }

    #[test]
    fn bar_with_nothing_in_range_takes_the_closest() {
        // Candidates are 1/2/5: 20..60 mm always holds one for a ratio of 3, so exercise the
        // fallback with an absurd scale where the candidate grid ends.
        let (_, mm) = bar_length(1e30);
        assert!(mm < 20.0);
    }

    #[test]
    fn paper_names() {
        assert_eq!(Paper::from_name("A3"), Some(Paper::A3));
        assert_eq!(Paper::from_name("tabloid"), Some(Paper::Tabloid));
        assert_eq!(Paper::from_name("a5"), None);
        assert_eq!(Paper::Letter.size_mm(), [215.9, 279.4]);
    }
}
