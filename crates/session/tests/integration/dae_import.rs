//! Importing COLLADA (`.dae`) files: units and up axis, library nodes as components,
//! materials, textures and polygons with holes, driven through JSON commands.

use std::path::PathBuf;

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dae")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-dae-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn definition(d: &Document, name: &str) -> Value {
    ask(d, json!({"type": "definitions"}))["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("no definition {name}"))
        .clone()
}

fn material(d: &Document, name: &str) -> Value {
    ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("no material {name}"))
        .clone()
}

#[test]
fn a_sketchup_like_file_keeps_its_units_axes_components_and_materials() {
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": fixture("chairs.dae")}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    // The ground (one face with a hole), the seat's 6 faces once, the window pane.
    assert_eq!(report["faces"], 8);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["materials"], 3);
    assert_eq!(report["textures"], 1);
    // Chair (component), SketchUp and Window (groups); the camera node holds nothing.
    assert_eq!(report["definitions"], 3);
    assert_eq!(report["instances"], 4);
    let warnings = report["warnings"].to_string();
    assert!(warnings.contains("1 camera was not imported"), "{warnings}");
    assert!(
        warnings.contains("1 set of lines was not imported"),
        "{warnings}"
    );

    let chair = definition(&d, "Chair");
    assert_eq!(chair["kind"], "component");
    assert_eq!(chair["instances"], 2);
    assert_eq!(chair["faces"], 6);
    assert_eq!(definition(&d, "SketchUp")["kind"], "group");

    // Centimeters and Y up: the 2 m ground lies on z = 0 running toward +y, with its hole.
    let sketchup = definition(&d, "SketchUp");
    let mesh = &d
        .scene()
        .definition(serde_json::from_value(sketchup["id"].clone()).unwrap())
        .unwrap()
        .mesh;
    let ground = mesh
        .faces()
        .map(|(_, f)| f)
        .find(|f| !f.holes.is_empty())
        .expect("the ground keeps its hole");
    assert!(ground.normal.distance(DVec3::Z) < 1e-9);
    let corners: Vec<DVec3> = ground
        .outer
        .iter()
        .map(|v| mesh.vertex(*v).unwrap().pos)
        .collect();
    let (lo, hi) = corners.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    assert!(lo.distance(DVec3::ZERO) < 1e-9 && hi.distance(DVec3::new(2.0, 2.0, 0.0)) < 1e-9);
    // The seat is 0.5 m a side and stands on the ground; the copies are 1 m away.
    let (lo, hi) = d.scene().triangles().iter().flat_map(|t| t.corners).fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    assert!(lo.z.abs() < 1e-9 && (hi.z - 1.1).abs() < 1e-9, "{lo} {hi}");

    // Colors as the file gives them; the brick texture repeats every 25 cm; glass is half
    // see-through.
    assert_eq!(material(&d, "Grass")["color"], json!([51, 153, 51]));
    let brick = material(&d, "Brick");
    assert_eq!(brick["texture"]["width"], 0.25);
    assert_eq!(brick["texture"]["height"], 0.25);
    assert_eq!(material(&d, "Glass")["opacity"], 0.5);
}

#[test]
fn units_and_up_can_be_overridden_and_scale_multiplies_the_files_unit() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "import", "path": fixture("chairs.dae"), "scale": 2, "up": "z"}),
    )
    .unwrap();
    // Z up: file y stays model y (the window 1.1 m up the file's y); centimeters doubled.
    let (lo, hi) = d.scene().triangles().iter().flat_map(|t| t.corners).fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    assert!((hi.y - lo.y - 2.2).abs() < 1e-9, "{lo} {hi}");
    assert!((hi.z - lo.z - 4.0).abs() < 1e-9, "{lo} {hi}");
}

#[test]
fn broken_and_hostile_files_are_refused_and_change_nothing() {
    let good = std::fs::read_to_string(fixture("chairs.dae")).unwrap();
    let cases: Vec<(&str, String, &str)> = vec![
        ("xml", "<COLLADA><asset>".to_string(), "not a COLLADA file"),
        ("root", "<model/>".to_string(), "not a COLLADA file"),
        (
            "dtd",
            format!(
                "<!DOCTYPE x [<!ENTITY a \"aaaa\">]>{}",
                good.replace("<?xml version=\"1.0\" encoding=\"utf-8\"?>", "")
            ),
            "not a COLLADA file",
        ),
        (
            "unit",
            good.replace("meter=\"0.01\"", "meter=\"-1\""),
            "unit",
        ),
        (
            "number",
            good.replace("0 0 0 50 0 0", "0 0 0 fifty 0 0"),
            "not a number",
        ),
        (
            "vcount",
            good.replace(
                "<vcount>4 4 4 4 4 4</vcount>",
                "<vcount>4 4 4 4 4 400</vcount>",
            ),
            "vcount",
        ),
        (
            "cycle",
            good.replace(
                "<node id=\"chair\" name=\"Chair\">",
                "<node id=\"chair\" name=\"Chair\"><instance_node url=\"#chair\"/>",
            ),
            "places itself",
        ),
        (
            "matrix",
            good.replace("1 0 0 100 0 1 0 0 0 0 1 0 0 0 0 1", "1 0 0 100"),
            "16 numbers",
        ),
    ];
    for (name, text, wanted) in cases {
        let path = scratch(&format!("bad-{name}.dae"));
        std::fs::write(&path, text).unwrap();
        let mut d = doc();
        let err = run(
            &mut d,
            json!({"type": "import", "path": path.to_str().unwrap()}),
        )
        .unwrap_err();
        assert_eq!(err.0, ErrorKind::File, "{name}: {}", err.1);
        assert!(err.1.contains(wanted), "{name}: {}", err.1);
        assert!(d.scene().triangles().is_empty(), "{name}");
        assert!(
            ask(&d, json!({"type": "materials"}))["materials"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{name}"
        );
    }
}

#[test]
fn an_index_out_of_range_skips_the_polygon_and_an_escaping_image_keeps_the_color() {
    let good = std::fs::read_to_string(fixture("chairs.dae")).unwrap();
    let text = good
        .replace("<p>0 1 2 0 2 3</p>", "<p>0 1 2 0 2 9</p>")
        .replace("textures/brick.png", "../dae/textures/brick.png");
    let path = scratch("odd.dae");
    std::fs::write(&path, text).unwrap();
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let report = &r["import"];
    assert_eq!(report["skipped"], 1);
    assert_eq!(report["faces"], 8);
    assert_eq!(report.get("textures"), None);
    assert!(
        report["warnings"]
            .to_string()
            .contains("1 texture was not imported"),
        "{report}"
    );
}
