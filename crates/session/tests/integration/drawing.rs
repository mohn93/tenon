//! Hidden-line drawings of the view and their SVG export.

use std::path::PathBuf;

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::drawing::line_drawing;
use session::{Command, Document, ErrorKind};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-drawing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A 2 x 2 x 1 box with a corner at the origin.
fn box_(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
}

fn camera(d: &mut Document, eye: [f64; 3], target: [f64; 3], parallel: bool) {
    let projection = if parallel { "parallel" } else { "perspective" };
    run(
        d,
        json!({"type": "set_camera", "eye": eye, "target": target, "projection": projection}),
    )
    .unwrap();
}

/// The box seen from the front left above, in a parallel view.
fn iso(d: &mut Document) {
    camera(d, [-8.0, -12.0, 7.0], [1.0, 1.0, 0.5], true);
}

fn front(d: &mut Document) {
    camera(d, [1.0, -10.0, 0.5], [1.0, 1.0, 0.5], true);
}

fn draw(d: &Document) -> session::drawing::Drawing {
    line_drawing(d, &d.camera, d.camera.viewport).unwrap()
}

fn export(d: &mut Document, name: &str, scale: Option<f64>) -> String {
    let path = scratch(name);
    let mut v = json!({"type": "export", "path": path.to_str().unwrap()});
    if let Some(s) = scale {
        v["scale"] = json!(s);
    }
    run(d, v).unwrap();
    String::from_utf8(std::fs::read(&path).unwrap()).unwrap()
}

/// The points of the `<polyline>`s of the group `id`.
fn polylines(svg: &str, id: &str) -> Vec<Vec<DVec2>> {
    let start = svg
        .find(&format!("<g id=\"{id}\""))
        .unwrap_or_else(|| panic!("no group {id} in {svg}"));
    let group = &svg[start..];
    let group = &group[..group.find("</g>").unwrap()];
    group
        .split("<polyline points=\"")
        .skip(1)
        .map(|p| {
            p[..p.find('"').unwrap()]
                .split(' ')
                .map(|pt| {
                    let (x, y) = pt.split_once(',').unwrap();
                    DVec2::new(x.parse().unwrap(), y.parse().unwrap())
                })
                .collect()
        })
        .collect()
}

#[test]
fn an_iso_box_has_its_nine_visible_edges_and_not_the_three_behind() {
    let mut d = doc();
    box_(&mut d);
    iso(&mut d);
    let drawing = draw(&d);
    assert_eq!(drawing.lines.len(), 9, "{:?}", drawing.lines);
    assert!(drawing.fills.is_empty() && drawing.cut_lines.is_empty());
    // None of them ends at the hidden corner (2, 2, 0).
    let hidden = d.camera.project(DVec3::new(2.0, 2.0, 0.0)).unwrap();
    for line in &drawing.lines {
        assert!(line.iter().all(|p| p.distance(hidden) > 1.0), "{line:?}");
    }
    let svg = export(&mut d, "iso.svg", None);
    assert_eq!(polylines(&svg, "lines").len(), 9);
    assert!(svg.starts_with("<?xml"), "{svg}");
    assert!(svg.contains("width=\"800\" height=\"600\""), "{svg}");
}

#[test]
fn a_line_drawn_on_a_face_stays_visible() {
    let mut d = doc();
    box_(&mut d);
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 1, 1], "end": [2, 1, 1]}),
    )
    .unwrap();
    iso(&mut d);
    let drawing = draw(&d);
    // Nine box edges, the new one, and the two top edges it splits in two.
    assert_eq!(drawing.lines.len(), 12, "{:?}", drawing.lines);
    let mid = d.camera.project(DVec3::new(1.0, 1.0, 1.0)).unwrap();
    assert!(
        drawing
            .lines
            .iter()
            .any(|l| l.windows(2).any(|w| point_on(mid, w[0], w[1])))
    );
}

fn point_on(p: DVec2, a: DVec2, b: DVec2) -> bool {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    (a + ab * t).distance(p) < 0.5
}

#[test]
fn an_edge_behind_the_box_is_split_where_the_box_covers_it() {
    let mut d = doc();
    box_(&mut d);
    run(
        &mut d,
        json!({"type": "add_edge", "start": [-2, 5, 0.5], "end": [4, 5, 0.5]}),
    )
    .unwrap();
    front(&mut d);
    let drawing = draw(&d);
    let p = |x: f64| d.camera.project(DVec3::new(x, 5.0, 0.5)).unwrap();
    let row = p(0.0).y;
    let mut across: Vec<&Vec<DVec2>> = drawing
        .lines
        .iter()
        .filter(|l| l.iter().all(|q| (q.y - row).abs() < 0.5))
        .filter(|l| l[0].distance(l[l.len() - 1]) > 10.0)
        // The box's own horizontal edges lie at other heights; keep the far line's pieces.
        .collect();
    across.sort_by(|a, b| a[0].x.total_cmp(&b[0].x));
    assert_eq!(across.len(), 2, "{across:?}");
    let ends = |l: &Vec<DVec2>| (l[0].x.min(l[l.len() - 1].x), l[0].x.max(l[l.len() - 1].x));
    // Pieces are 2 px, so the cut lands within 2.5 px of the silhouette.
    assert!((ends(across[0]).1 - p(0.0).x).abs() < 2.5, "{across:?}");
    assert!((ends(across[1]).0 - p(2.0).x).abs() < 2.5, "{across:?}");
    assert!((ends(across[0]).0 - p(-2.0).x).abs() < 0.01);
    assert!((ends(across[1]).1 - p(4.0).x).abs() < 0.01);
}

#[test]
fn a_section_cut_shows_the_fill_and_cut_edges_but_not_the_edges_behind_the_fill() {
    let mut d = doc();
    box_(&mut d);
    run(
        &mut d,
        json!({"type": "add_section_plane", "point": [1, 0, 0], "normal": [1, 0, 0]}),
    )
    .unwrap();
    camera(&mut d, [-10.0, 1.0, 0.5], [1.0, 1.0, 0.5], true);
    let drawing = draw(&d);
    assert!(drawing.fills.len() >= 2, "{:?}", drawing.fills);
    assert_eq!(drawing.cut_lines.len(), 4, "{:?}", drawing.cut_lines);
    assert!(drawing.lines.is_empty(), "{:?}", drawing.lines);
    let svg = export(&mut d, "cut.svg", None);
    assert!(svg.contains("#3a3a3a"));
    assert_eq!(polylines(&svg, "cuts").len(), 4);
    assert_eq!(polylines(&svg, "lines").len(), 0);
    // Without the fill the far end's edges show through the opening.
    run(
        &mut d,
        json!({"type": "set_section_display", "fill": false}),
    )
    .unwrap();
    let open = draw(&d);
    assert!(open.fills.is_empty());
    assert_eq!(open.lines.len(), 4, "{:?}", open.lines);
    // Without the cut it is the whole box seen end on: the far end hides behind the near.
    run(
        &mut d,
        json!({"type": "set_section_display", "cuts": false}),
    )
    .unwrap();
    let whole = draw(&d);
    assert!(whole.cut_lines.is_empty());
    assert_eq!(whole.lines.len(), 4, "{:?}", whole.lines);
}

#[test]
fn hidden_tags_and_soft_edges_are_left_out() {
    let mut d = doc();
    box_(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_edge", "start": [-2, 5, 0.5], "end": [-1, 5, 0.5]}),
    )
    .unwrap();
    let loose = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('e'))
        .unwrap()
        .clone();
    iso(&mut d);
    assert_eq!(draw(&d).lines.len(), 10);
    let out = run(&mut d, json!({"type": "add_tag", "name": "Hide"})).unwrap();
    let tag = out["changes"]["tags"][0].clone();
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [loose], "tag": tag}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "change_tag", "tag": tag, "visible": false}),
    )
    .unwrap();
    assert_eq!(draw(&d).lines.len(), 9);
    // A soft edge between two faces is not drawn.
    let q: session::Query =
        serde_json::from_value(json!({"type": "entities", "kinds": ["edge"]})).unwrap();
    let all = serde_json::to_value(d.query(&q).unwrap()).unwrap();
    let edge = all["edges"][0]["id"].clone();
    assert_ne!(edge, loose);
    run(
        &mut d,
        json!({"type": "soften_edges", "ids": [edge], "soft": true}),
    )
    .unwrap();
    assert_eq!(draw(&d).lines.len(), 8);
}

#[test]
fn dimension_and_note_text_is_drawn_and_escaped() {
    let mut d = doc();
    box_(&mut d);
    iso(&mut d);
    run(
        &mut d,
        json!({"type": "add_dimension", "a": [0, 0, 0], "b": [2, 0, 0], "offset": [0, -1, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_screen_note", "at": [0.1, 0.1], "text": "A & B <c>"}),
    )
    .unwrap();
    let drawing = draw(&d);
    assert!(!drawing.annotation_lines.is_empty());
    let texts: Vec<&str> = drawing.labels.iter().map(|l| l.text.as_str()).collect();
    assert!(
        texts.contains(&"2 m") && texts.contains(&"A & B <c>"),
        "{texts:?}"
    );
    let note = drawing
        .labels
        .iter()
        .find(|l| l.text.starts_with("A &"))
        .unwrap();
    assert!((note.at - DVec2::new(80.0, 60.0)).length() < 1e-9);
    let svg = export(&mut d, "dim.svg", None);
    assert!(svg.contains(">2 m</text>"), "{svg}");
    assert!(svg.contains(">A &amp; B &lt;c&gt;</text>"), "{svg}");
}

#[test]
fn a_parallel_view_exports_in_millimetres_at_scale() {
    let mut d = doc();
    box_(&mut d);
    front(&mut d);
    let svg = export(&mut d, "scaled.svg", Some(100.0));
    let mm_per_px = d.camera.view_height() / 600.0 * 1000.0 / 100.0;
    let (w, h) = (800.0 * mm_per_px, 600.0 * mm_per_px);
    assert!(
        svg.contains(&format!("width=\"{w:.3}mm\" height=\"{h:.3}mm\"")),
        "{svg}"
    );
    assert!(
        svg.contains(&format!("viewBox=\"0 0 {w:.3} {h:.3}\"")),
        "{svg}"
    );
    // The box is 2 m wide: 20 mm at 1:100.
    let widest = polylines(&svg, "lines")
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
    assert!((widest.1 - widest.0 - 20.0).abs() < 0.01, "{widest:?}");
    assert!(svg.contains("stroke-width=\"0.250\""), "{svg}");
}

#[test]
fn a_scale_needs_a_parallel_view_and_an_svg() {
    let mut d = doc();
    box_(&mut d);
    camera(&mut d, [-8.0, -12.0, 7.0], [1.0, 1.0, 0.5], false);
    let path = scratch("persp.svg");
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "scale": 50}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(message, "Switch to parallel projection to export at scale");
    assert!(!path.exists());
    // A scale for a format that has none, and a nonsense scale.
    let obj = scratch("scaled.obj");
    assert!(
        run(
            &mut d,
            json!({"type": "export", "path": obj.to_str().unwrap(), "scale": 50})
        )
        .is_err()
    );
    iso(&mut d);
    for bad in [0.0, -5.0] {
        let (kind, _) = run(
            &mut d,
            json!({"type": "export", "path": path.to_str().unwrap(), "scale": bad}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn the_same_view_gives_the_same_bytes() {
    let mut d = doc();
    box_(&mut d);
    run(
        &mut d,
        json!({"type": "add_section_plane", "point": [1, 0, 0], "normal": [1, 0, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_dimension", "a": [1, 0, 0], "b": [2, 0, 0], "offset": [0, -1, 0]}),
    )
    .unwrap();
    iso(&mut d);
    let a = export(&mut d, "a.svg", None);
    let b = export(&mut d, "b.svg", None);
    assert_eq!(a, b);
    assert!(!a.contains("-0.000"));
}

#[test]
fn a_drawing_of_too_many_pieces_is_refused() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1, "segments": 360}),
    )
    .unwrap();
    let all = {
        let q: session::Query =
            serde_json::from_value(json!({"type": "entities", "kinds": ["edge", "face"]})).unwrap();
        serde_json::to_value(d.query(&q).unwrap()).unwrap()
    };
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    run(
        &mut d,
        json!({"type": "make_component", "ids": ids, "name": "Ring"}),
    )
    .unwrap();
    let ring = {
        let q: session::Query = serde_json::from_value(json!({"type": "selection"})).unwrap();
        serde_json::to_value(d.query(&q).unwrap()).unwrap()["ids"][0].clone()
    };
    for i in 1..25 {
        run(
            &mut d,
            json!({"type": "copy", "ids": [ring], "offset": [3 * (i % 5), 3 * (i / 5), 0]}),
        )
        .unwrap();
    }
    camera(&mut d, [6.0, 6.0, 40.0], [6.0, 6.0, 0.0], true);
    // 9000 edges, each 700 px long on this huge view: 256 pieces each.
    let e = line_drawing(&d, &d.camera, DVec2::new(1e6, 1e6)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert_eq!(
        e.message,
        "the model is too large for a line drawing at this size"
    );
    // At the window's size the same model draws.
    assert!(line_drawing(&d, &d.camera, d.camera.viewport).is_ok());
}
