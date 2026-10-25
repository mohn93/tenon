//! Exporting a 3MF package, driven through JSON commands and parsed back.

use std::io::Read;
use std::path::PathBuf;

use glam::DVec2;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::{Value, json};
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
        .join(format!("session-3mf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn add_box(d: &mut Document, size: [f64; 3], at: [f64; 3], name: &str, material: Option<&Value>) {
    let mut c = json!({"type": "add_box", "corner": at, "size": size, "as": "group", "name": name});
    if let Some(m) = material {
        c["material"] = m.clone();
    }
    run(d, c).unwrap();
}

/// One parsed element: its name and attributes.
type Element = (String, Vec<(String, String)>);

/// Every element of an XML text, in order; panics when it is not well formed.
fn elements(xml: &str) -> Vec<Element> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) | Event::Empty(e) => {
                let attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            String::from_utf8(a.key.as_ref().to_vec()).unwrap(),
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect();
                out.push((
                    String::from_utf8(e.name().as_ref().to_vec()).unwrap(),
                    attrs,
                ));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn attr<'a>(e: &'a Element, key: &str) -> Option<&'a str> {
    e.1.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// The package's named entry as text.
fn entry(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a zip");
    let mut text = String::new();
    zip.by_name(name)
        .unwrap_or_else(|_| panic!("{name} in the package"))
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn export(d: &mut Document, name: &str, extra: Value) -> (Vec<u8>, Value) {
    let path = scratch(name);
    let mut c = json!({"type": "export", "path": path.to_str().unwrap()});
    for (k, v) in extra.as_object().into_iter().flatten() {
        c[k] = v.clone();
    }
    let out = run(d, c).unwrap();
    (std::fs::read(&path).unwrap(), out["export"].clone())
}

#[test]
fn a_box_is_8_vertices_and_12_triangles_in_millimeters() {
    let mut d = doc();
    add_box(&mut d, [1.0, 2.0, 3.0], [0.0, 0.0, 0.0], "Block", None);
    let (bytes, report) = export(&mut d, "box.3mf", json!({}));
    let model = entry(&bytes, "3D/3dmodel.model");
    let els = elements(&model);
    assert_eq!(attr(&els[0], "unit"), Some("millimeter"));
    assert_eq!(els.iter().filter(|e| e.0 == "vertex").count(), 8);
    assert_eq!(els.iter().filter(|e| e.0 == "triangle").count(), 12);
    let max = |k: &str| {
        els.iter()
            .filter(|e| e.0 == "vertex")
            .map(|e| attr(e, k).unwrap().parse::<f64>().unwrap())
            .fold(f64::MIN, f64::max)
    };
    assert_eq!((max("x"), max("y"), max("z")), (1000.0, 2000.0, 3000.0));
    let object = els.iter().find(|e| e.0 == "object").unwrap();
    assert_eq!(attr(object, "name"), Some("Block"));
    assert_eq!(attr(object, "type"), Some("model"));
    assert_eq!(report["triangles"], 12);
    assert_eq!(
        report["objects"],
        json!([{"name": "Block", "vertices": 8, "triangles": 12, "closed": true}])
    );
    assert!(report.get("warnings").is_none(), "{report}");
}

#[test]
fn the_package_has_its_three_parts_and_valid_xml() {
    let mut d = doc();
    add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], "A & <B>", None);
    let (bytes, _) = export(&mut d, "package.3mf", json!({}));
    let zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
    assert_eq!(names[0], "[Content_Types].xml");
    for n in ["[Content_Types].xml", "_rels/.rels", "3D/3dmodel.model"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    drop(zip);
    let types = elements(&entry(&bytes, "[Content_Types].xml"));
    assert!(types.iter().any(|e| attr(e, "Extension") == Some("model")));
    let rels = elements(&entry(&bytes, "_rels/.rels"));
    let rel = rels.iter().find(|e| e.0 == "Relationship").unwrap();
    assert_eq!(attr(rel, "Target"), Some("/3D/3dmodel.model"));
    let els = elements(&entry(&bytes, "3D/3dmodel.model"));
    let object = els.iter().find(|e| e.0 == "object").unwrap();
    assert_eq!(attr(object, "name"), Some("A & <B>"));
    let item = els.iter().find(|e| e.0 == "item").unwrap();
    assert_eq!(attr(item, "objectid"), attr(object, "id"));
}

#[test]
fn two_parts_are_two_objects_and_loose_geometry_is_one_more() {
    let mut d = doc();
    add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], "Left", None);
    add_box(&mut d, [1.0, 1.0, 1.0], [3.0, 0.0, 0.0], "Right", None);
    let (bytes, report) = export(&mut d, "two.3mf", json!({}));
    let els = elements(&entry(&bytes, "3D/3dmodel.model"));
    assert_eq!(els.iter().filter(|e| e.0 == "object").count(), 2);
    assert_eq!(els.iter().filter(|e| e.0 == "item").count(), 2);
    let names: Vec<&str> = report["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Left", "Right"]);
    // The second box sits 3 m along x: its world transform is baked in.
    let xs: Vec<f64> = els
        .iter()
        .filter(|e| e.0 == "vertex")
        .map(|e| attr(e, "x").unwrap().parse().unwrap())
        .collect();
    assert_eq!(xs.iter().cloned().fold(f64::MIN, f64::max), 4000.0);

    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [10, 0, 0], "opposite": [11, 1, 0]}),
    )
    .unwrap();
    let (_, report) = export(&mut d, "three.3mf", json!({}));
    assert_eq!(report["objects"].as_array().unwrap().len(), 3);
    assert_eq!(report["objects"][0]["name"], "Loose geometry");
}

#[test]
fn a_material_color_lands_in_basematerials() {
    let mut d = doc();
    let red = run(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [255, 0, 16]}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    add_box(
        &mut d,
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        "Painted",
        Some(&red),
    );
    add_box(&mut d, [1.0, 1.0, 1.0], [3.0, 0.0, 0.0], "Plain", None);
    let (bytes, _) = export(&mut d, "color.3mf", json!({}));
    let els = elements(&entry(&bytes, "3D/3dmodel.model"));
    let group = els.iter().find(|e| e.0 == "basematerials").unwrap();
    let bases: Vec<&Element> = els.iter().filter(|e| e.0 == "base").collect();
    assert_eq!(bases.len(), 2);
    assert_eq!(attr(bases[0], "name"), Some("Red"));
    assert_eq!(attr(bases[0], "displaycolor"), Some("#FF0010FF"));
    assert_eq!(attr(bases[1], "name"), Some("default"));
    let objects: Vec<&Element> = els.iter().filter(|e| e.0 == "object").collect();
    assert_eq!(attr(objects[0], "pid"), attr(group, "id"));
    assert_eq!(attr(objects[0], "pindex"), Some("0"));
    assert_eq!(attr(objects[1], "pindex"), Some("1"));
    let p1: std::collections::BTreeSet<&str> = els
        .iter()
        .filter(|e| e.0 == "triangle")
        .map(|e| attr(e, "p1").unwrap())
        .collect();
    assert_eq!(p1.len(), 2);
}

#[test]
fn an_open_part_is_reported_not_closed() {
    let mut d = doc();
    add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], "Solid", None);
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [6, 1, 0]}),
    )
    .unwrap();
    let (_, report) = export(&mut d, "open.3mf", json!({}));
    let objects = report["objects"].as_array().unwrap();
    assert_eq!(objects[0]["name"], "Loose geometry");
    assert_eq!(objects[0]["closed"], false);
    assert!(
        objects[0]["why"]
            .as_str()
            .unwrap()
            .contains("bounded by one")
    );
    assert_eq!(objects[1]["closed"], true);
    let warnings = report["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .starts_with("Loose geometry is not a closed mesh")
    );
}

#[test]
fn selection_exports_only_the_selected_parts() {
    let mut d = doc();
    add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], "Left", None);
    add_box(&mut d, [1.0, 1.0, 1.0], [3.0, 0.0, 0.0], "Right", None);
    run(&mut d, json!({"type": "select", "ids": []})).unwrap();
    let path = scratch("none.3mf");
    let err = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "selection": true}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput, "{}", err.1);
    run(&mut d, json!({"type": "select", "all": true})).unwrap();
    let q: session::Query = serde_json::from_value(json!({"type": "selection"})).unwrap();
    let selected = serde_json::to_value(d.query(&q).unwrap()).unwrap();
    let first = selected["ids"][0].clone();
    run(&mut d, json!({"type": "select", "ids": [first]})).unwrap();
    let (_, report) = export(&mut d, "selected.3mf", json!({"selection": true}));
    assert_eq!(report["objects"].as_array().unwrap().len(), 1);
    let path = scratch("bad.stl");
    let err = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "selection": true}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
}

#[test]
fn an_existing_foreign_file_is_refused_without_overwrite() {
    let mut d = doc();
    add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], "Cube", None);
    let path = scratch("foreign.3mf");
    std::fs::write(&path, b"not a package").unwrap();
    let err = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput, "{}", err.1);
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    // A package of ours is replaced without asking.
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
}

#[test]
fn an_empty_model_has_nothing_to_export() {
    let mut d = doc();
    let path = scratch("empty.3mf");
    let err = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
}
