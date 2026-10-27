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

// ---- M20c: vertex colors, texture coordinate sets, color space ----

fn source(id: &str, values: &[f64], stride: usize) -> String {
    let list: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!(
        "<source id=\"{id}\"><float_array id=\"{id}-a\" count=\"{}\">{}</float_array>\
         <technique_common><accessor source=\"#{id}-a\" count=\"{}\" stride=\"{stride}\">\
         </accessor></technique_common></source>",
        values.len(),
        list.join(" "),
        values.len() / stride,
    )
}

/// A unit square of two triangles (Z up), with `inputs` besides the vertices in its
/// `<triangles>`, `sources` before them, and `p` the corner indices.
#[allow(clippy::too_many_arguments)]
fn square_dae(
    name: &str,
    asset_extra: &str,
    diffuse: &str,
    image: &str,
    bind: &str,
    sources: &str,
    inputs: &str,
    p: &str,
) -> String {
    let xml = format!(
        r##"<?xml version="1.0"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
<asset>{asset_extra}<unit meter="1"/><up_axis>Z_UP</up_axis></asset>
<library_images><image id="img"><init_from>{image}</init_from></image></library_images>
<library_effects><effect id="fx"><profile_COMMON>
<newparam sid="surf"><surface type="2D"><init_from>img</init_from></surface></newparam>
<newparam sid="samp"><sampler2D><source>surf</source></sampler2D></newparam>
<technique sid="common"><lambert><diffuse>{diffuse}</diffuse></lambert></technique>
</profile_COMMON></effect></library_effects>
<library_materials><material id="mat" name="Paint"><instance_effect url="#fx"/></material></library_materials>
<library_geometries><geometry id="g"><mesh>
{pos}{sources}
<vertices id="v"><input semantic="POSITION" source="#pos"/></vertices>
<triangles count="2" material="sym"><input semantic="VERTEX" source="#v" offset="0"/>{inputs}<p>{p}</p></triangles>
</mesh></geometry></library_geometries>
<library_visual_scenes><visual_scene id="s"><node id="n">
<instance_geometry url="#g"><bind_material><technique_common>
<instance_material symbol="sym" target="#mat">{bind}</instance_material>
</technique_common></bind_material></instance_geometry>
</node></visual_scene></library_visual_scenes>
<scene><instance_visual_scene url="#s"/></scene>
</COLLADA>"##,
        pos = source("pos", &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.], 3),
    );
    let path = scratch(name);
    std::fs::write(&path, xml).unwrap();
    path.to_str().unwrap().to_string()
}

fn colored_dae(name: &str, tool: &str, asset_extra: &str, diffuse: &str) -> String {
    square_dae(
        name,
        &format!("<contributor><authoring_tool>{tool}</authoring_tool></contributor>{asset_extra}"),
        diffuse,
        "none.png",
        "",
        &source("col", &[0.5, 0.5, 0.5, 1., 0., 0.], 3),
        r##"<input semantic="COLOR" source="#col" offset="1" set="0"/>"##,
        "0 0 1 0 2 0 0 1 2 1 3 1",
    )
}

fn colors_of(d: &Document) -> Vec<Vec<u64>> {
    let mut all: Vec<Vec<u64>> = ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            m["color"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_u64().unwrap())
                .collect()
        })
        .collect();
    all.sort();
    all
}

fn import_path(path: &str) -> (Document, Value) {
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    d.scene().validate().unwrap();
    (d, r["import"].clone())
}

#[test]
fn collada_vertex_colors_paint_faces_in_the_files_color_space() {
    let white = "<color>1 1 1 1</color>";
    // sRGB by default: 0.5 is byte 128 (127.5 rounded up) times the white material.
    let (d, report) = import_path(&colored_dae("srgb.dae", "SketchUp", "", white));
    assert_eq!(report["faces"], 2, "{report}");
    assert_eq!(colors_of(&d), vec![vec![128, 128, 128], vec![255, 0, 0]]);
    let names = ask(&d, json!({"type": "materials"}))["materials"].to_string();
    assert!(names.contains("Paint #ff0000"), "{names}");

    // Blender's exporter writes linear values: 0.5 is the sRGB byte 188.
    let (d, _) = import_path(&colored_dae("blender.dae", "Blender 4.2", "", white));
    assert_eq!(colors_of(&d), vec![vec![188, 188, 188], vec![255, 0, 0]]);
    // So does a file that says so, whatever its tool; and a diffuse color follows suit.
    let (d, _) = import_path(&colored_dae(
        "declared.dae",
        "Other",
        "<extra><technique profile=\"x\"><color_space>linear</color_space></technique></extra>",
        "<color>0.5 0.5 0.5 1</color>",
    ));
    // 0.5 diffuse times 0.5 vertex color, both linear, then to sRGB; the red keeps 0.
    let quarter = 0.25f64;
    let want = (1.055 * quarter.powf(1.0 / 2.4) - 0.055) * 255.0;
    assert_eq!(colors_of(&d)[0][0], want.round() as u64);
    // sRGB declared over a Blender tool wins.
    let (d, _) = import_path(&colored_dae(
        "srgb-declared.dae",
        "Blender 4.2",
        "<extra><technique profile=\"x\"><color_space>sRGB</color_space></technique></extra>",
        white,
    ));
    assert_eq!(colors_of(&d)[0], vec![128, 128, 128]);
}

#[test]
fn a_diffuse_texture_reads_the_coordinate_set_its_channel_is_bound_to() {
    use base64::Engine;
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png)
    );
    let corner_uv = |dx: f64| {
        // v counts up in COLLADA: the corners' (x + dx, y).
        [0., 0., 1., 0., 1., 1., 0., 1.]
            .chunks(2)
            .flat_map(|c| [c[0] + dx, c[1]])
            .collect::<Vec<_>>()
    };
    let shift = |bind: &str, name: &str| {
        let sources = format!(
            "{}{}",
            source("uv0", &corner_uv(0.0), 2),
            source("uv1", &corner_uv(0.25), 2)
        );
        let path = square_dae(
            name,
            "",
            "<texture texture=\"samp\" texcoord=\"CHANNEL1\"/>",
            &uri,
            bind,
            &sources,
            r##"<input semantic="TEXCOORD" source="#uv0" offset="1" set="0"/><input semantic="TEXCOORD" source="#uv1" offset="1" set="1"/>"##,
            "0 0 1 1 2 2 0 0 2 2 3 3",
        );
        let (d, report) = import_path(&path);
        assert_eq!(report["textures"], 1, "{report}");
        let t = ask(&d, json!({"type": "materials"}))["materials"][0]["texture"].clone();
        let size = DVec2::new(t["width"].as_f64().unwrap(), t["height"].as_f64().unwrap());
        let mut shifts = Vec::new();
        for tri in d.scene().triangles() {
            for (c, uv) in tri.corners.iter().zip(tri.uvs) {
                shifts.push(((uv.x / size.x) - c.x).rem_euclid(1.0));
            }
        }
        let first = shifts[0];
        for s in &shifts {
            let diff = (s - first).abs();
            assert!(diff < 1e-6 || 1.0 - diff < 1e-6, "{shifts:?}");
        }
        first
    };
    let bound = shift(
        r##"<bind_vertex_input semantic="CHANNEL1" input_semantic="TEXCOORD" input_set="1"/>"##,
        "bound.dae",
    );
    assert!((bound - 0.25).abs() < 1e-6, "{bound}");
    // Without a binding the lowest set is read, as before.
    let unbound = shift("", "unbound.dae");
    assert!(unbound < 1e-6 || unbound > 1.0 - 1e-6, "{unbound}");
}
