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

/// The signed volume (mm^3) of each object in a model part, from its vertices and triangles.
fn volumes(xml: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    for e in elements(xml) {
        match e.0.as_str() {
            "object" => {
                out.push(0.0);
                vertices.clear();
            }
            "vertex" => {
                vertices.push(["x", "y", "z"].map(|k| attr(&e, k).unwrap().parse().unwrap()));
            }
            "triangle" => {
                let [a, b, c] = ["v1", "v2", "v3"].map(|k| {
                    glam::DVec3::from(vertices[attr(&e, k).unwrap().parse::<usize>().unwrap()])
                });
                *out.last_mut().unwrap() += a.dot(b.cross(c)) / 6.0;
            }
            _ => {}
        }
    }
    out
}

#[test]
fn a_mirrored_part_exports_closed_with_positive_volume() {
    let mut d = doc();
    add_box(&mut d, [1.0, 2.0, 3.0], [0.0, 0.0, 0.0], "Plain", None);
    add_box(&mut d, [1.0, 2.0, 3.0], [5.0, 0.0, 0.0], "Flipped", None);
    let q: session::Query = serde_json::from_value(json!({"type": "selection"})).unwrap();
    let selected = serde_json::to_value(d.query(&q).unwrap()).unwrap();
    let ids = selected["ids"].clone();
    run(
        &mut d,
        json!({"type": "flip", "ids": ids, "axis": "red", "frame": "own"}),
    )
    .unwrap();
    // Nested in a plain group, the mirror is still in the placement.
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let (bytes, report) = export(&mut d, "mirrored.3mf", json!({}));
    let objects = report["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 2, "{report}");
    assert!(objects.iter().all(|o| o["closed"] == true), "{report}");
    // The unnamed outer group is named for its kind.
    assert_eq!(objects[1]["name"], "Group", "{report}");
    for v in volumes(&entry(&bytes, "3D/3dmodel.model")) {
        assert!((v - 6.0e9).abs() < 1.0, "volume {v}");
    }
}

/// A 1 x 2 x 3 m box made a component named "Post", copied to `copies` more placements 3 m
/// apart along x. Returns the ids of the placements.
fn posts(d: &mut Document, copies: usize) -> Vec<Value> {
    add_box(d, [1.0, 2.0, 3.0], [0.0, 0.0, 0.0], "Post", None);
    let q: session::Query = serde_json::from_value(json!({"type": "selection"})).unwrap();
    let first = serde_json::to_value(d.query(&q).unwrap()).unwrap()["ids"][0].clone();
    run(
        d,
        json!({"type": "make_component", "ids": [first], "name": "Post"}),
    )
    .unwrap();
    let first = serde_json::to_value(d.query(&q).unwrap()).unwrap()["ids"][0].clone();
    for k in 1..=copies {
        run(
            d,
            json!({"type": "copy", "ids": [first], "offset": [3.0 * k as f64, 0, 0]}),
        )
        .unwrap();
    }
    let q: session::Query =
        serde_json::from_value(json!({"type": "entities", "kinds": ["instance"]})).unwrap();
    let all = serde_json::to_value(d.query(&q).unwrap()).unwrap();
    let ids: Vec<Value> = all["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].clone())
        .collect();
    assert_eq!(ids.len(), copies + 1);
    ids
}

fn model(bytes: &[u8]) -> Vec<Element> {
    elements(&entry(bytes, "3D/3dmodel.model"))
}

/// Every id used by a reference resolves to a resource or object of the model.
fn assert_ids_resolve(els: &[Element]) {
    let ids: std::collections::BTreeSet<&str> = els
        .iter()
        .filter(|e| {
            ["object", "basematerials", "m:texture2d", "m:texture2dgroup"].contains(&e.0.as_str())
        })
        .map(|e| attr(e, "id").unwrap())
        .collect();
    let ids_count = els
        .iter()
        .filter(|e| {
            ["object", "basematerials", "m:texture2d", "m:texture2dgroup"].contains(&e.0.as_str())
        })
        .count();
    assert_eq!(ids.len(), ids_count, "ids are unique");
    for e in els {
        for key in ["objectid", "pid", "texid"] {
            if let Some(v) = attr(e, key) {
                assert!(ids.contains(v), "{} {key}={v} resolves in {ids:?}", e.0);
            }
        }
    }
}

#[test]
fn a_component_placed_twice_is_one_object_with_two_build_items() {
    let mut d = doc();
    posts(&mut d, 1);
    let (bytes, report) = export(&mut d, "shared.3mf", json!({}));
    let els = model(&bytes);
    assert_ids_resolve(&els);
    assert_eq!(els.iter().filter(|e| e.0 == "object").count(), 1);
    assert_eq!(els.iter().filter(|e| e.0 == "triangle").count(), 12);
    let items: Vec<&Element> = els.iter().filter(|e| e.0 == "item").collect();
    assert_eq!(items.len(), 2);
    assert_eq!(attr(items[0], "objectid"), attr(items[1], "objectid"));
    // One placement is at the origin, the other 3 m along x; each says where it is.
    let moves: Vec<f64> = items
        .iter()
        .map(|i| {
            attr(i, "transform").map_or(0.0, |t| t.split(' ').nth(9).unwrap().parse().unwrap())
        })
        .collect();
    assert!(moves.contains(&0.0) && moves.contains(&3000.0), "{moves:?}");
    assert_eq!(
        report["objects"],
        json!([{"name": "Post", "vertices": 8, "triangles": 12, "closed": true}])
    );
}

#[test]
fn placements_with_different_paint_or_a_mirror_are_not_shared() {
    let mut d = doc();
    let ids = posts(&mut d, 2);
    let red = run(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [255, 0, 0]}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    run(
        &mut d,
        json!({"type": "paint", "ids": [ids[1]], "material": red}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "flip", "ids": [ids[2]], "axis": "red", "frame": "own"}),
    )
    .unwrap();
    let (bytes, report) = export(&mut d, "unshared.3mf", json!({}));
    let els = model(&bytes);
    assert_ids_resolve(&els);
    // The plain one, the painted one and the mirrored one each stand alone.
    assert_eq!(els.iter().filter(|e| e.0 == "object").count(), 3);
    assert!(
        els.iter()
            .all(|e| e.0 != "item" || attr(e, "transform").is_none())
    );
    assert!(
        report["objects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["closed"] == true)
    );
    for v in volumes(&entry(&bytes, "3D/3dmodel.model")) {
        assert!((v - 6.0e9).abs() < 1.0, "volume {v}");
    }
}

#[test]
fn a_shared_component_inside_a_group_is_referenced_by_a_component_object() {
    let mut d = doc();
    let ids = posts(&mut d, 1);
    run(
        &mut d,
        json!({"type": "make_group", "ids": ids, "name": "Pair"}),
    )
    .unwrap();
    let (bytes, report) = export(&mut d, "nested.3mf", json!({}));
    let els = model(&bytes);
    assert_ids_resolve(&els);
    // The post's mesh, and the group placing it twice.
    assert_eq!(els.iter().filter(|e| e.0 == "object").count(), 2);
    let components: Vec<&Element> = els.iter().filter(|e| e.0 == "component").collect();
    assert_eq!(components.len(), 2);
    assert!(components.iter().all(|c| attr(c, "transform").is_some()));
    let group = els.iter().filter(|e| e.0 == "object").nth(1).unwrap();
    assert_eq!(attr(group, "name"), Some("Pair"));
    assert_eq!(els.iter().filter(|e| e.0 == "item").count(), 1);
    assert_eq!(report["objects"].as_array().unwrap().len(), 1, "{report}");
}

/// The text of the package's entry names.
fn names(bytes: &[u8]) -> Vec<String> {
    let zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    zip.file_names().map(|n| n.unwrap().into_owned()).collect()
}

#[test]
fn a_textured_material_writes_its_image_coordinates_and_package_parts() {
    let mut d = doc();
    let ids = posts(&mut d, 1);
    let png = scratch("tiles.png");
    image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([60 * x as u8, 0, 200, 255]))
        .save(&png)
        .unwrap();
    let tiles = run(
        &mut d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": png.to_str().unwrap(), "width": 2}}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    run(
        &mut d,
        json!({"type": "paint", "ids": [ids[0]], "material": tiles}),
    )
    .unwrap();
    let (bytes, _) = export(&mut d, "textured.3mf", json!({}));
    let els = model(&bytes);
    assert_ids_resolve(&els);
    assert!(
        attr(&els[0], "xmlns:m")
            .unwrap()
            .ends_with("material/2015/02")
    );
    let texture = els.iter().find(|e| e.0 == "m:texture2d").unwrap();
    assert_eq!(attr(texture, "contenttype"), Some("image/png"));
    let path = attr(texture, "path").unwrap();
    assert!(path.starts_with("/3D/"), "{path}");
    // The part is in the zip, byte for byte the stored PNG, with its content type and a
    // relationship from the model part.
    assert!(names(&bytes).contains(&path[1..].to_string()));
    let mut stored = Vec::new();
    zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))
        .unwrap()
        .by_name(&path[1..])
        .unwrap()
        .read_to_end(&mut stored)
        .unwrap();
    assert_eq!(stored, std::fs::read(&png).unwrap());
    let types = elements(&entry(&bytes, "[Content_Types].xml"));
    assert!(types.iter().any(|e| {
        attr(e, "Extension") == Some("png") && attr(e, "ContentType") == Some("image/png")
    }));
    let rels = elements(&entry(&bytes, "3D/_rels/3dmodel.model.rels"));
    let rel = rels.iter().find(|e| e.0 == "Relationship").unwrap();
    assert_eq!(attr(rel, "Target"), Some(path));
    assert!(attr(rel, "Type").unwrap().ends_with("/3dtexture"));
    // Triangles of the textured crate point into the group's coordinates; the plain crate's
    // triangles keep their base colors.
    let group = els.iter().find(|e| e.0 == "m:texture2dgroup").unwrap();
    assert_eq!(attr(group, "texid"), attr(texture, "id"));
    let coords: Vec<(f64, f64)> = els
        .iter()
        .filter(|e| e.0 == "m:tex2coord")
        .map(|e| {
            (
                attr(e, "u").unwrap().parse().unwrap(),
                attr(e, "v").unwrap().parse().unwrap(),
            )
        })
        .collect();
    let textured: Vec<&Element> = els
        .iter()
        .filter(|e| e.0 == "triangle" && attr(e, "pid") == attr(group, "id"))
        .collect();
    assert_eq!(textured.len(), 12);
    for t in &textured {
        for k in ["p1", "p2", "p3"] {
            assert!(attr(t, k).unwrap().parse::<usize>().unwrap() < coords.len());
        }
    }
    let plain = els
        .iter()
        .filter(|e| e.0 == "triangle" && attr(e, "pid") == Some("1"))
        .count();
    assert_eq!(plain, 12);
    // The 2 x 1 m texture over a 1 m wide wall gives half a repeat across: u spans 0.5.
    assert!(
        coords.iter().any(|(u, _)| (u - 0.5).abs() < 1e-9),
        "{coords:?}"
    );
}
