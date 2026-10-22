//! The PDF writer and the print layout.

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::pdf::Pdf;
use session::print::{Paper, PrintSpec, layout};
use session::{Command, Document, ErrorKind};

fn run(d: &mut Document, v: Value) {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd).expect("command runs");
}

/// A box `x` x 10 x 5 m with a corner at the origin, seen from the front in a parallel view.
fn building(x: f64) -> Document {
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [x, 10, 0]}),
    );
    run(
        &mut d,
        json!({"type": "push_pull", "face": "f8", "distance": 5.0}),
    );
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [x / 2.0, -100.0, 2.5], "target": [x / 2.0, 5.0, 2.5],
               "projection": "parallel"}),
    );
    d
}

fn spec(scale: Option<f64>) -> PrintSpec {
    PrintSpec {
        paper: Paper::A4,
        landscape: false,
        scale,
        dpi: 150,
    }
}

fn err(r: Result<session::print::Layout, session::CommandError>) -> (ErrorKind, String) {
    let e = r.expect_err("refused");
    (e.kind, e.message)
}

/// The world rectangle a camera sees, as (right min, right max, up min, up max).
fn visible(c: &session::Camera) -> [f64; 4] {
    let (r, u) = (c.right(), c.up());
    let (x, y) = (c.target.dot(r), c.target.dot(u));
    let h = c.view_height();
    let w = h * c.aspect();
    [x - w / 2.0, x + w / 2.0, y - h / 2.0, y + h / 2.0]
}

#[test]
fn fit_is_one_sheet_with_the_view_aspect() {
    let d = building(10.0);
    let l = layout(&d, &d.camera, &spec(None)).unwrap();
    assert_eq!(l.sheets.len(), 1);
    let s = &l.sheets[0];
    assert_eq!(s.page_mm, [210.0, 297.0]);
    assert!(!s.trim && s.scale_bar.is_none() && s.footer.is_none());
    let [x, y, w, h] = s.area_mm;
    assert!(x >= 10.0 - 1e-9 && y >= 10.0 - 1e-9);
    assert!(x + w <= 200.0 + 1e-9 && y + h <= 287.0 + 1e-9);
    assert!((w / h - 800.0 / 600.0).abs() < 0.01);
    // Centred in the printable area.
    assert!(((x - 10.0) - (200.0 - (x + w))).abs() < 0.2);
    assert_eq!(s.tile_camera.eye, d.camera.eye);
    assert_eq!(s.tile_camera.viewport.x as u32, s.tile_px[0]);
    assert_eq!(s.tile_px[0] as f64, (w * 150.0 / 25.4).round());
}

#[test]
fn landscape_swaps_the_paper() {
    let d = building(10.0);
    let mut sp = spec(None);
    sp.landscape = true;
    sp.paper = Paper::Letter;
    let l = layout(&d, &d.camera, &sp).unwrap();
    assert_eq!(l.sheets[0].page_mm, [279.4, 215.9]);
}

#[test]
fn fifty_metres_at_1_100_tiles_exactly() {
    let d = building(50.0);
    let l = layout(&d, &d.camera, &spec(Some(100.0))).unwrap();
    // A4 portrait printable 190 x 277 mm = 19 x 27.7 m: 50 m is 3 columns, 10 m is 1 row.
    assert_eq!(l.sheets.len(), 3);
    for (i, s) in l.sheets.iter().enumerate() {
        assert!(s.trim);
        assert_eq!(
            s.footer.as_deref(),
            Some(format!("Sheet 1,{} of 1 x 3", i + 1).as_str())
        );
        assert!(s.scale_bar.is_some());
        let h = s.tile_camera.view_height();
        assert!((h - s.area_mm[3] / 1000.0 * 100.0).abs() < 1e-9);
        assert!(s.tile_camera.is_parallel());
    }
    for pair in l.sheets.windows(2) {
        let (a, b) = (visible(&pair[0].tile_camera), visible(&pair[1].tile_camera));
        assert!((a[1] - b[0]).abs() < 1e-9, "tiles abut: {a:?} {b:?}");
        assert!((a[2] - b[2]).abs() < 1e-9 && (a[3] - b[3]).abs() < 1e-9);
    }
    // The model is covered.
    let first = visible(&l.sheets[0].tile_camera);
    let last = visible(&l.sheets[2].tile_camera);
    assert!(first[0] <= 1e-9 && last[1] >= 50.0 - 1e-9);
}

#[test]
fn rows_run_top_to_bottom_and_footer_counts() {
    let d = building(50.0);
    let mut sp = spec(Some(200.0));
    sp.landscape = true;
    // A4 landscape printable 277 x 190 mm = 55.4 x 38 m: 1 x 1.
    let l = layout(&d, &d.camera, &sp).unwrap();
    assert_eq!(l.sheets.len(), 1);
    assert!(!l.sheets[0].trim && l.sheets[0].footer.is_none());
    assert!(l.sheets[0].scale_bar.is_some());
    // 1:25 -> 6.925 x 4.75 m: 8 columns, and the 5 m height needs 2 rows.
    let l = layout(
        &d,
        &d.camera,
        &PrintSpec {
            scale: Some(25.0),
            ..sp
        },
    )
    .unwrap();
    assert_eq!(l.sheets.len(), 16);
    assert_eq!(l.sheets[0].footer.as_deref(), Some("Sheet 1,1 of 2 x 8"));
    assert_eq!(l.sheets[7].footer.as_deref(), Some("Sheet 1,8 of 2 x 8"));
    assert_eq!(l.sheets[8].footer.as_deref(), Some("Sheet 2,1 of 2 x 8"));
    let (top, below) = (
        visible(&l.sheets[0].tile_camera),
        visible(&l.sheets[8].tile_camera),
    );
    assert!((below[3] - top[2]).abs() < 1e-9, "rows abut");
    assert!(top[2] > below[2]);
}

#[test]
fn oblique_views_project_along_the_view() {
    let mut d = building(50.0);
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [-80.0, -80.0, 60.0], "target": [25.0, 5.0, 2.5],
               "projection": "parallel"}),
    );
    let l = layout(&d, &d.camera, &spec(Some(100.0))).unwrap();
    // All 8 corners lie inside the tiles' union.
    let rects: Vec<_> = l.sheets.iter().map(|s| visible(&s.tile_camera)).collect();
    let c = &l.sheets[0].tile_camera;
    let (r, u) = (c.right(), c.up());
    for k in 0..8 {
        let p = DVec3::new(
            if k & 1 == 0 { 0.0 } else { 50.0 },
            if k & 2 == 0 { 0.0 } else { 10.0 },
            if k & 4 == 0 { 0.0 } else { 5.0 },
        );
        let (x, y) = (p.dot(r), p.dot(u));
        assert!(
            rects.iter().any(|v| x >= v[0] - 1e-9
                && x <= v[1] + 1e-9
                && y >= v[2] - 1e-9
                && y <= v[3] + 1e-9),
            "corner {k} covered"
        );
        assert!(l.sheets.iter().all(|s| s.tile_camera.sees_depth(p)));
    }
}

#[test]
fn too_many_sheets_perspective_and_bad_values_are_refused() {
    let d = building(50.0);
    let (kind, msg) = err(layout(&d, &d.camera, &spec(Some(1.0))));
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(msg.contains("sheets") && msg.contains("100"), "{msg}");

    let mut persp = d.camera.clone();
    persp.projection = session::Projection::Perspective;
    let (kind, msg) = err(layout(&d, &persp, &spec(Some(100.0))));
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(msg, "Switch to parallel projection to print at scale");
    // Fit works in perspective.
    assert!(layout(&d, &persp, &spec(None)).is_ok());

    for s in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            err(layout(&d, &d.camera, &spec(Some(s)))).0,
            ErrorKind::InvalidInput
        );
    }
    for dpi in [0, 71, 301] {
        let sp = PrintSpec { dpi, ..spec(None) };
        assert_eq!(err(layout(&d, &d.camera, &sp)).0, ErrorKind::InvalidInput);
    }
    for dpi in [72, 300] {
        let sp = PrintSpec { dpi, ..spec(None) };
        assert!(layout(&d, &d.camera, &sp).is_ok());
    }
}

#[test]
fn empty_model_cannot_print_at_scale() {
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    d.camera.projection = session::Projection::Parallel;
    assert_eq!(
        err(layout(&d, &d.camera, &spec(Some(100.0)))).0,
        ErrorKind::InvalidInput
    );
}

#[test]
fn scale_bar_measures_on_paper() {
    let d = building(10.0);
    for (n, metres) in [(100.0, 5.0), (50.0, 2.0), (1000.0, 50.0), (20.0, 1.0)] {
        let l = layout(&d, &d.camera, &spec(Some(n))).unwrap();
        let bar = l.sheets[0].scale_bar.as_ref().unwrap();
        assert!(
            (bar.length_m - metres).abs() < 1e-12,
            "1:{n} -> {}",
            bar.length_m
        );
        assert!((bar.length_mm - metres * 1000.0 / n).abs() < 1e-9);
        assert!((20.0..=60.0).contains(&bar.length_mm));
        assert_eq!(bar.height_mm, 4.0);
        // In the bottom margin.
        assert!(bar.y_mm >= 287.0 && bar.y_mm + bar.height_mm <= 297.0);
        assert_eq!(bar.scale_label, format!("1:{n}"));
        assert_eq!(bar.zero_label, "0");
    }
    let l = layout(&d, &d.camera, &spec(Some(100.0))).unwrap();
    assert_eq!(l.sheets[0].scale_bar.as_ref().unwrap().end_label, "5 m");
}

#[test]
fn chunks_cover_the_tile_within_4096() {
    let d = building(10.0);
    let sp = PrintSpec {
        paper: Paper::A3,
        landscape: true,
        scale: Some(100.0),
        dpi: 300,
    };
    let l = layout(&d, &d.camera, &sp).unwrap();
    let s = &l.sheets[0];
    assert!(s.tile_px[0] > 4096);
    let chunks = s.chunks();
    assert!(chunks.len() >= 2);
    let total: u64 = chunks.iter().map(|c| c.px[0] as u64 * c.px[1] as u64).sum();
    assert_eq!(total, s.tile_px[0] as u64 * s.tile_px[1] as u64);
    assert!(chunks.iter().all(|c| c.px[0] <= 4096 && c.px[1] <= 4096));
    let whole = visible(&s.tile_camera);
    let mut x = f64::MAX;
    let mut xe = f64::MIN;
    for c in &chunks {
        let v = visible(&c.camera);
        assert_eq!(
            c.camera.viewport,
            DVec2::new(c.px[0] as f64, c.px[1] as f64)
        );
        x = x.min(v[0]);
        xe = xe.max(v[1]);
        assert!(v[2] >= whole[2] - 1e-9 && v[3] <= whole[3] + 1e-9);
        // Same pixel scale as the tile.
        let scale = c.camera.view_height() / c.px[1] as f64;
        assert!((scale - s.tile_camera.view_height() / s.tile_px[1] as f64).abs() < 1e-12);
    }
    assert!((x - whole[0]).abs() < 1e-9 && (xe - whole[1]).abs() < 1e-9);
}

#[test]
fn labels_land_on_their_sheets() {
    let mut d = building(50.0);
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [40.0, 0.0, 2.5], "position": [41.0, -1.0, 3.0], "text": "Hall"}),
    );
    let l = layout(&d, &d.camera, &spec(Some(100.0))).unwrap();
    assert_eq!(l.labels.len(), l.sheets.len());
    let found: Vec<_> = l
        .labels
        .iter()
        .enumerate()
        .flat_map(|(i, v)| v.iter().map(move |t| (i, t)))
        .filter(|(_, t)| t.text == "Hall")
        .collect();
    assert_eq!(found.len(), 1, "{:?}", l.labels);
    let (i, t) = found[0];
    assert_eq!(i, 2);
    let [x, y, w, h] = l.sheets[i].area_mm;
    assert!(t.at_mm[0] >= x && t.at_mm[0] <= x + w && t.at_mm[1] >= y && t.at_mm[1] <= y + h);
}

#[test]
fn a_laid_out_sheet_makes_a_valid_pdf() {
    let d = building(50.0);
    let l = layout(&d, &d.camera, &spec(Some(100.0))).unwrap();
    let mut pdf = Pdf::new();
    for (s, labels) in l.sheets.iter().zip(&l.labels) {
        let mut page = pdf.page(s.page_mm);
        let rgb = vec![200u8; 4 * 4 * 3];
        page.image(s.area_mm, 4, 4, &rgb);
        session::print::decorate(&mut page, s, labels);
    }
    let a = pdf.finish();
    assert!(a.starts_with(b"%PDF-1.4"));
    assert!(String::from_utf8_lossy(&a).contains("Sheet 1,2 of 1 x 3"));
}
