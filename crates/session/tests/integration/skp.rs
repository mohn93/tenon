//! Importing SketchUp files (2021 and newer) through JSON commands, over the CC BY 4.0
//! fixtures of the `skp` crate.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use glam::DVec2;
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
        .join("../skp/tests/fixtures")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-skp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn write(name: &str, bytes: &[u8]) -> String {
    let path = scratch(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_string()
}

/// The root's instances.
fn root_instances(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["instance"]}))["instances"]
        .as_array()
        .unwrap()
        .clone()
}

/// The size of an instance's box along x, y and z.
fn size(instance: &Value) -> [f64; 3] {
    let b = &instance["bounds"];
    [0, 1, 2].map(|i| b["max"][i].as_f64().unwrap() - b["min"][i].as_f64().unwrap())
}

fn definitions(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "definitions"}))["definitions"]
        .as_array()
        .unwrap()
        .clone()
}

fn materials_of(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .clone()
}

fn warnings(report: &Value) -> Vec<String> {
    report["warnings"]
        .as_array()
        .map(|w| w.iter().map(|s| s.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

/// Tags in a fixture besides the default one, as the `skp` reader sees them.
fn tags_in(path: &str) -> usize {
    use skp::container::Container;
    use skp::scene::{ReadOptions, read_model};
    let src = skp::source::MemorySource::new(std::fs::read(path).unwrap());
    let c = Container::open(&src).unwrap();
    let m = read_model(&src, &c, &ReadOptions::default()).unwrap();
    m.index(&src, &c)
        .layers
        .iter()
        .filter(|l| l.name != "Layer0")
        .count()
}

#[test]
fn cupboard_imports_as_a_group_with_shared_components() {
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": fixture("cupboard.skp")}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    let count = |k: &str| report[k].as_u64().unwrap_or(0);
    assert!(count("definitions") >= 2, "{report}");
    assert!(count("instances") > count("definitions"), "{report}");
    assert_eq!(count("skipped"), 0, "{report}");
    // 609 faces in the file; 4 that are not quite flat come back cut into more.
    assert_eq!((count("polygons"), count("faces")), (609, 616), "{report}");

    // One group in the root, named after the file, selected.
    let roots = root_instances(&d);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0]["name"], "cupboard");
    assert_eq!(roots[0]["kind"], "group");
    assert_eq!(roots[0]["id"], report["instance"]);
    assert_eq!(
        ask(&d, json!({"type": "selection"}))["ids"],
        json!([report["instance"]])
    );

    // The knob and the hinge are each one component placed twice: editing one edits both.
    let defs = definitions(&d);
    for name in ["knob", "Hinge"] {
        let def = defs.iter().find(|x| x["name"] == name).unwrap();
        assert_eq!(def["kind"], "component", "{def}");
        assert_eq!(def["instances"], 2, "{def}");
        assert!(def["faces"].as_u64().unwrap() > 0, "{def}");
    }
    assert_eq!(
        defs.iter().filter(|x| x["name"] == "knob").count(),
        1,
        "one knob definition"
    );

    // Inches become meters: the cupboard with the scale figure beside it.
    let measured = size(&roots[0]);
    for (got, want) in measured.iter().zip([1.0248, 3.2635, 2.0315]) {
        assert!((got - want).abs() < 1e-3, "{measured:?}");
    }

    // One undo step takes it all back.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(root_instances(&d).is_empty());
    assert!(definitions(&d).is_empty());
    assert!(materials_of(&d).is_empty());
    d.scene().validate().unwrap();
}

#[test]
fn desk_imports_its_placed_definitions_and_says_what_it_left_out() {
    let path = fixture("desk.skp");
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    let warnings = warnings(report);
    // The unplaced scale figure (and its unplaced part) are not built.
    assert!(
        warnings.contains(&"2 definitions placed nowhere were not imported".to_string()),
        "{warnings:?}"
    );
    assert!(definitions(&d).iter().all(|x| x["name"] != "Derrick"));
    // Its materials are only on the figure: none is added.
    assert_eq!(report["materials"], 0);
    assert!(materials_of(&d).is_empty());
    let tags = tags_in(&path);
    assert_eq!(
        warnings.iter().any(|w| w.contains("tag")),
        tags > 0,
        "{warnings:?}"
    );
    let measured = size(&root_instances(&d)[0]);
    for (got, want) in measured.iter().zip([0.78, 2.0, 1.0]) {
        assert!((got - want).abs() < 1e-3, "{measured:?}");
    }
}

/// The cupboard with a texture image added to its glass material: `image` stored as
/// `materials/[Translucent_Glass_Blue]/glass.png`, its tile 10 x 20 inches.
fn textured_cupboard(image: &[u8]) -> Vec<u8> {
    let bytes = std::fs::read(fixture("cupboard.skp")).unwrap();
    let start = bytes
        .windows(4)
        .position(|w| w == b"PK\x03\x04")
        .expect("zip start");
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    let mut out = Cursor::new(bytes[..start].to_vec());
    out.set_position(start as u64);
    let mut zip = zip::ZipWriter::new(out);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let folder = "materials/[Translucent_Glass_Blue]/";
    for i in 0..archive.len() {
        let file = archive.by_index_raw(i).unwrap();
        if file.name().unwrap() == format!("{folder}material.xml") {
            drop(file);
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut archive.by_index(i).unwrap(), &mut xml).unwrap();
            let xml = xml
                .replace("hasTexture=\"0\" />", "hasTexture=\"1\">")
                .replace(
                    "</materialDocument>",
                    "<mat:texture textureFilename=\"glass.png\" xScale=\"10\" yScale=\"20\" />\
                     </mat:material>\n</materialDocument>",
                );
            zip.start_file(format!("{folder}material.xml"), stored)
                .unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        } else {
            zip.raw_copy_file(file).unwrap();
        }
    }
    zip.start_file(format!("{folder}glass.png"), stored)
        .unwrap();
    zip.write_all(image).unwrap();
    zip.finish().unwrap().into_inner()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

#[test]
fn materials_and_textures_are_imported() {
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": fixture("cupboard.skp")}),
    )
    .unwrap();
    let report = &r["import"];
    let all = materials_of(&d);
    assert_eq!(report["materials"], all.len());
    assert!(!all.is_empty());
    for m in &all {
        let used = m["faces"].as_u64().unwrap() + m["instances"].as_u64().unwrap();
        assert!(used > 0, "unused material {m}");
    }
    let glass = all
        .iter()
        .find(|m| m["name"] == "[Translucent_Glass_Blue]")
        .unwrap();
    assert_eq!(glass["color"], json!([100, 149, 237]));
    assert_eq!(glass["opacity"], 0.5);
    assert!(report.get("textures").is_none(), "no textures: {report}");

    // With an image the glass gets a texture, its tile in meters.
    let path = write("textured.skp", &textured_cupboard(&png(4, 2)));
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    d.scene().validate().unwrap();
    assert_eq!(r["import"]["textures"], 1, "{}", r["import"]);
    let textured: Vec<Value> = materials_of(&d)
        .into_iter()
        .filter(|m| m["texture"].is_object())
        .collect();
    assert_eq!(textured.len(), 1);
    let t = &textured[0]["texture"];
    assert_eq!(textured[0]["name"], "[Translucent_Glass_Blue]");
    assert!((t["width"].as_f64().unwrap() - 0.254).abs() < 1e-9, "{t}");
    assert!((t["height"].as_f64().unwrap() - 0.508).abs() < 1e-9, "{t}");
    assert_eq!(
        (&t["pixel_width"], &t["pixel_height"]),
        (&json!(4), &json!(2))
    );
    assert!(
        warnings(&r["import"])
            .iter()
            .all(|w| !w.contains("texture"))
    );

    // An image over the pixel limit leaves the material its colour, with a warning.
    let path = write("too-wide.skp", &textured_cupboard(&png(4097, 1)));
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    d.scene().validate().unwrap();
    assert!(r["import"].get("textures").is_none());
    let glass = materials_of(&d)
        .into_iter()
        .find(|m| m["name"] == "[Translucent_Glass_Blue]")
        .unwrap();
    assert!(glass.get("texture").is_none_or(Value::is_null), "{glass}");
    assert_eq!(glass["color"], json!([100, 149, 237]));
    let warnings = warnings(&r["import"]);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("[Translucent_Glass_Blue]")
                && w.contains("texture")
                && w.contains("4096")),
        "{warnings:?}"
    );
}

#[test]
fn scale_and_context() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "import", "path": fixture("desk.skp")}),
    )
    .unwrap();
    let once = size(&root_instances(&d)[0]);
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "import", "path": fixture("desk.skp"), "scale": 2}),
    )
    .unwrap();
    let twice = size(&root_instances(&d)[0]);
    for (a, b) in once.iter().zip(twice) {
        assert!((2.0 * a - b).abs() < 1e-9, "{once:?} {twice:?}");
    }

    // `up` and `merge_coplanar` are ignored; a component wraps it.
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": fixture("desk.skp"), "as": "component", "up": "y",
               "merge_coplanar": false}),
    )
    .unwrap();
    assert_eq!(root_instances(&d)[0]["kind"], "component");
    assert_eq!(size(&root_instances(&d)[0]), once);
    assert_eq!(r["import"]["faces"], 147);

    // Into the context: no wrapper, the file's own instances land in the root.
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": fixture("cupboard.skp"), "as": "context"}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    assert!(r["import"].get("instance").is_none());
    assert!(definitions(&d).iter().all(|x| x["name"] != "cupboard"));
    let names: Vec<Value> = root_instances(&d)
        .iter()
        .map(|i| i["name"].clone())
        .collect();
    assert!(names.contains(&json!("Derrick")), "{names:?}");
    assert!(names.len() > 1, "{names:?}");
    let selected = ask(&d, json!({"type": "selection"}))["ids"]
        .as_array()
        .unwrap()
        .len();
    assert!(selected >= names.len());
}

#[test]
fn existing_material_names_are_kept() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_material", "name": "[Translucent_Glass_Blue]", "color": [1, 2, 3]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "import", "path": fixture("cupboard.skp")}),
    )
    .unwrap();
    let all = materials_of(&d);
    let mine = all
        .iter()
        .find(|m| m["name"] == "[Translucent_Glass_Blue]")
        .unwrap();
    assert_eq!(mine["color"], json!([1, 2, 3]));
    assert_eq!(mine["opacity"], 1.0);
    assert_eq!(mine["faces"], 0);
    let theirs = all
        .iter()
        .find(|m| m["name"] == "[Translucent_Glass_Blue] 2")
        .unwrap();
    assert_eq!(theirs["color"], json!([100, 149, 237]));
    assert!(theirs["faces"].as_u64().unwrap() > 0);
}

#[test]
fn mutated_files_give_errors_not_panics() {
    let original = std::fs::read(fixture("desk.skp")).unwrap();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let (mut ok, mut failed) = (0, 0);
    for k in 0..100 {
        let mut bytes = original.clone();
        match k % 4 {
            0 => bytes.truncate(next(bytes.len())),
            1 => {
                for _ in 0..1 + next(8) {
                    let at = next(bytes.len());
                    bytes[at] ^= 1 << next(8);
                }
            }
            2 => {
                let at = next(bytes.len());
                let end = (at + 1 + next(64)).min(bytes.len());
                for b in &mut bytes[at..end] {
                    *b = next(256) as u8;
                }
            }
            _ => {
                // A run of 0xff: lengths and counts read from it are huge.
                let at = next(bytes.len());
                let end = (at + 8).min(bytes.len());
                bytes[at..end].fill(0xff);
            }
        }
        let path = write(&format!("mutant-{k}.skp"), &bytes);
        let mut d = doc();
        run(
            &mut d,
            json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
        )
        .unwrap();
        let before = d.scene().clone();
        match run(&mut d, json!({"type": "import", "path": path})) {
            Ok(_) => ok += 1,
            Err((kind, message)) => {
                failed += 1;
                assert!(
                    matches!(kind, ErrorKind::File | ErrorKind::InvalidInput),
                    "{kind:?}: {message}"
                );
                assert_eq!(d.scene(), &before, "mutant {k}: {message}");
            }
        }
        d.scene().validate().unwrap();
    }
    println!("mutants: {ok} imported, {failed} refused");
    assert_eq!(ok + failed, 100);
    assert!(failed > 0);
}

/// A SketchUp header (`ff fe ff`, a length, UTF-16LE text) for `text`.
fn utf16_tag(text: &str) -> Vec<u8> {
    let mut out = vec![0xff, 0xfe, 0xff, text.encode_utf16().count() as u8];
    out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    out
}

#[test]
fn legacy_files_are_refused_with_a_message() {
    let mut bytes = utf16_tag("SketchUp Model");
    bytes.extend(utf16_tag("{20.0.373}"));
    bytes.extend([0x01, 0x00, 0x00, 0x00, b'C', b'V', b'e', b'r']);
    bytes.extend([0u8; 300]);
    let mut d = doc();
    for name in ["old.skp", "old.model"] {
        // Without an extension it knows, the header says SketchUp.
        let path = write(name, &bytes);
        let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
        assert!(message.contains("2020"), "{message}");
        assert!(message.contains("2021 or newer"), "{message}");
    }
    assert_eq!(d.scene(), doc().scene());

    // A 2021+ file under another name imports by its header too; other files do not.
    let path = write("desk.model", &std::fs::read(fixture("desk.skp")).unwrap());
    run(&mut d, json!({"type": "import", "path": path})).unwrap();
    let path = write("note.model", b"hello");
    let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains(".skp"), "{message}");
    let path = write("not-sketchup.skp", b"hello");
    let (kind, _) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
}

/// Building small SketchUp files record by record (`[u16 tag][u32 size][payload]`), so tests
/// can make what no fixture has: many placements, deep nesting, many materials.
mod craft {
    use std::io::{Cursor, Write};

    use skp::tags as t;

    fn rec(tag: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = tag.to_le_bytes().to_vec();
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend(payload);
        out
    }

    /// An id record: the id in 8 bytes (an id's width is its record's size).
    fn id(tag: u16, v: u64) -> Vec<u8> {
        rec(tag, &v.to_le_bytes())
    }

    /// A row-major transform: `scale` on the diagonal and translation `x` inches along x.
    pub fn scaled(scale: f64, x: f64) -> [f64; 13] {
        [
            scale, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, scale, x, 0.0, 0.0, 1.0,
        ]
    }

    /// A placement: the definition's index, its transform and its material's pid.
    pub type Place = (usize, [f64; 13], Option<u64>);

    fn instances(places: &[Place]) -> Vec<u8> {
        let mut list = Vec::new();
        for (k, (def, m, material)) in places.iter().enumerate() {
            let mut body = id(t::ENTITY_PID, 100_000 + k as u64);
            body.extend(id(t::COMPONENT_INSTANCE_DEF_PID_REF, def_pid(*def)));
            let m: Vec<u8> = m.iter().flat_map(|x| x.to_le_bytes()).collect();
            body.extend(rec(t::COMPONENT_INSTANCE_TRANSFORM, &m));
            if let Some(material) = material {
                body.extend(id(t::DRAWING_ELEMENT_MATERIAL_PID_REF, *material));
            }
            list.extend(rec(t::COMPONENT_INSTANCE, &body));
        }
        rec(t::COMPONENT_INSTANCES, &list)
    }

    fn def_pid(i: usize) -> u64 {
        10 + i as u64
    }

    /// A one-inch edge along x.
    fn edge() -> Vec<u8> {
        let vertex = |pid: u64, x: f64| {
            let mut body = id(t::ENTITY_PID, pid);
            let p: Vec<u8> = [x, 0.0, 0.0].iter().flat_map(|c| c.to_le_bytes()).collect();
            body.extend(rec(t::VERTEX_POSITION, &p));
            rec(t::VERTEX, &body)
        };
        let mut out = rec(
            t::COMPONENT_VERTICES,
            &[vertex(1, 0.0), vertex(2, 1.0)].concat(),
        );
        let mut e = id(t::ENTITY_PID, 3);
        e.extend(id(t::EDGE_START_VERTEX_PID_REF, 1));
        e.extend(id(t::EDGE_END_VERTEX_PID_REF, 2));
        out.extend(rec(t::COMPONENT_EDGES, &rec(t::EDGE, &e)));
        out
    }

    /// A SketchUp 2023 file: definitions (each a one-inch edge plus its placements), the
    /// root's placements, materials (pid, name) and extra zip entries (stored).
    pub fn file(
        definitions: &[Vec<Place>],
        root: &[Place],
        materials: &[(u64, String)],
        entries: &[(String, Vec<u8>)],
    ) -> Vec<u8> {
        let mut defs = Vec::new();
        for (i, places) in definitions.iter().enumerate() {
            let mut component = id(t::ENTITY_PID, def_pid(i));
            component.extend(edge());
            component.extend(instances(places));
            let mut d = rec(t::COMPONENT, &component);
            d.extend(rec(
                t::COMPONENT_DEFINITION_NAME,
                format!("D{i}").as_bytes(),
            ));
            defs.extend(rec(t::COMPONENT_DEFINITION, &d));
        }
        let components = rec(
            t::SKETCH_UP_MODEL_COMPONENTS,
            &rec(t::DEFINITION_LIST, &rec(t::DEFINITION_LIST_DEFS, &defs)),
        );
        let root = rec(
            t::SKETCH_UP_MODEL_ROOT,
            &rec(t::COMPONENT, &instances(root)),
        );
        let mut list = Vec::new();
        for (pid, name) in materials {
            let mut m = rec(t::ENTITY, &id(t::ENTITY_PID, *pid));
            m.extend(rec(t::MATERIAL_XML_PATH, name.as_bytes()));
            list.extend(rec(t::MATERIAL, &m));
        }
        let materials = rec(
            t::SKETCH_UP_MODEL_MATERIALS,
            &rec(
                t::MATERIAL_MANAGER,
                &rec(t::MATERIAL_MANAGER_MATERIALS, &list),
            ),
        );
        let model = rec(t::SKETCH_UP_MODEL, &[materials, components, root].concat());

        let mut header = super::utf16_tag("SketchUp Model");
        header.extend(super::utf16_tag("{23.0.0}"));
        let start = header.len() as u64;
        let mut out = Cursor::new(header);
        out.set_position(start);
        let mut zip = zip::ZipWriter::new(out);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("model.dat", deflated).unwrap();
        zip.write_all(&model).unwrap();
        for (name, bytes) in entries {
            zip.start_file(name.as_str(), stored).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
}

/// Imports `bytes` (written as `name`) into a new document, timing it.
fn import_timed(
    name: &str,
    bytes: &[u8],
) -> (
    Document,
    Result<Value, (ErrorKind, String)>,
    std::time::Duration,
) {
    let path = write(name, bytes);
    let mut d = doc();
    let start = std::time::Instant::now();
    let r = run(&mut d, json!({"type": "import", "path": path}));
    (d, r, start.elapsed())
}

#[test]
fn many_placements_are_read_without_walking_them() {
    use craft::{file, scaled};
    // 31 levels, each placing the next 10 times: 10^31 placements through the nesting, 310
    // instances in the file.
    let mut defs: Vec<Vec<craft::Place>> = (0..30)
        .map(|i| {
            (0..10)
                .map(|k| (i + 1, scaled(1.0, k as f64), None))
                .collect()
        })
        .collect();
    defs.push(vec![]);
    let bytes = file(&defs, &[(0, scaled(1.0, 0.0), None)], &[], &[]);
    let (d, r, took) = import_timed("deep.skp", &bytes);
    let r = r.unwrap();
    d.scene().validate().unwrap();
    assert_eq!(r["import"]["definitions"], 31, "{}", r["import"]);
    assert_eq!(r["import"]["instances"], 301);
    assert!(took.as_secs_f64() < 2.0, "took {took:?}");

    // A definition placing itself 50,000 times.
    let defs = vec![vec![(0, scaled(1.0, 0.0), None); 50_000]];
    let bytes = file(&defs, &[(0, scaled(1.0, 0.0), None)], &[], &[]);
    let (d, r, took) = import_timed("self.skp", &bytes);
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::File, "{message}");
    assert!(message.contains("places itself"), "{message}");
    assert_eq!(d.scene(), doc().scene());
    assert!(took.as_secs_f64() < 2.0, "took {took:?}");
}

#[test]
fn a_model_larger_than_the_world_is_refused() {
    use craft::{file, scaled};
    // One instance scaled a billion times.
    let bytes = file(&[vec![]], &[(0, scaled(1e9, 0.0), None)], &[], &[]);
    let (d, r, _) = import_timed("huge.skp", &bytes);
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert!(message.contains("beyond"), "{message}");
    assert_eq!(d.scene(), doc().scene());

    // Ten levels each scaled 100 times: no one transform is large, the model is.
    let mut defs: Vec<Vec<craft::Place>> = (0..10)
        .map(|i| vec![(i + 1, scaled(100.0, 0.0), None)])
        .collect();
    defs.push(vec![]);
    let bytes = file(&defs, &[(0, scaled(1.0, 0.0), None)], &[], &[]);
    let (_, r, _) = import_timed("growing.skp", &bytes);
    assert_eq!(r.unwrap_err().0, ErrorKind::InvalidInput);

    // Three such levels fit, and `scale` counts.
    let mut defs: Vec<Vec<craft::Place>> = (0..3)
        .map(|i| vec![(i + 1, scaled(100.0, 0.0), None)])
        .collect();
    defs.push(vec![]);
    let bytes = file(&defs, &[(0, scaled(1.0, 0.0), None)], &[], &[]);
    let (_, r, _) = import_timed("big.skp", &bytes);
    r.unwrap();
    let path = write("big.skp", &bytes);
    let (kind, _) = run(
        &mut doc(),
        json!({"type": "import", "path": path, "scale": 1e3}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn materials_past_the_limit_are_left_off_with_a_warning() {
    use craft::{file, scaled};
    let n = session::MAX_IMPORT_MATERIALS + 6;
    let mut materials: Vec<(u64, String)> =
        (0..n).map(|k| (1000 + k as u64, format!("M{k}"))).collect();
    let mut root: Vec<craft::Place> = (0..n)
        .map(|k| (0, scaled(1.0, k as f64), Some(1000 + k as u64)))
        .collect();
    // An id too large for the reader's 32-bit material numbering, and an instance naming it.
    materials.push((1 << 33, "Far".into()));
    root.push((0, scaled(1.0, 0.0), Some(1 << 33)));
    let bytes = file(&[vec![]], &root, &materials, &[]);
    let (d, r, _) = import_timed("many-materials.skp", &bytes);
    let r = r.unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    assert_eq!(report["materials"], session::MAX_IMPORT_MATERIALS);
    let names: Vec<Value> = materials_of(&d).iter().map(|m| m["name"].clone()).collect();
    assert!(names.contains(&json!("M0")) && names.contains(&json!("M1023")));
    assert!(!names.contains(&json!("M1024")));
    assert!(materials_of(&d).iter().all(|m| m["instances"] == 1));
    let warnings = warnings(report);
    assert!(
        warnings.contains(&"6 materials past the 1024 an import may add were left off".into()),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("2 material references could not be matched")),
        "{warnings:?}"
    );
}

#[test]
fn textures_past_the_model_limit_keep_their_colour() {
    use craft::{file, scaled};
    let n = kernel::MAX_TEXTURES + 1;
    let materials: Vec<(u64, String)> =
        (0..n).map(|k| (1000 + k as u64, format!("T{k}"))).collect();
    let root: Vec<craft::Place> = (0..n)
        .map(|k| (0, scaled(1.0, k as f64), Some(1000 + k as u64)))
        .collect();
    let mut entries = Vec::new();
    for k in 0..n {
        let xml = format!(
            "<materialDocument><mat:material name=\"T{k}\" colorRed=\"200\" colorGreen=\"10\" \
             colorBlue=\"20\" hasTexture=\"1\"><mat:texture textureFilename=\"t.png\" \
             xScale=\"10\" yScale=\"10\" /></mat:material></materialDocument>"
        );
        entries.push((format!("materials/T{k}/material.xml"), xml.into_bytes()));
        entries.push((format!("materials/T{k}/t.png"), png(1, 1)));
    }
    let bytes = file(&[vec![]], &root, &materials, &entries);
    let (d, r, _) = import_timed("many-textures.skp", &bytes);
    let r = r.unwrap();
    d.scene().validate().unwrap();
    assert_eq!(r["import"]["textures"], kernel::MAX_TEXTURES);
    assert_eq!(r["import"]["materials"], n);
    let plain: Vec<Value> = materials_of(&d)
        .into_iter()
        .filter(|m| m.get("texture").is_none_or(Value::is_null))
        .collect();
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0]["color"], json!([200, 10, 20]));
    let warnings = warnings(&r["import"]);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains(plain[0]["name"].as_str().unwrap()) && w.contains("texture")),
        "{warnings:?}"
    );
}

/// Imports every `*.skp` under `$SKP_CORPUS`: files from SketchUp 2021 and newer import
/// without error, or (empty templates) say there is nothing to import; older ones are refused
/// with the 2020 message. Prints each report and the warnings over all files.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_imports() {
    let Some(dir) = std::env::var_os("SKP_CORPUS") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    collect(Path::new(&dir), &mut files);
    files.sort();
    let (mut imported, mut empty, mut legacy, mut failed) = (0, 0, 0, Vec::new());
    // Per warning (its number taken out): files and total count.
    let mut warned: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for path in &files {
        let mut d = doc();
        let p = path.to_str().unwrap();
        match run(&mut d, json!({"type": "import", "path": p})) {
            Ok(r) => {
                imported += 1;
                if let Err(e) = d.scene().validate() {
                    failed.push(format!("{p}: invalid scene: {e}"));
                }
                let report = &r["import"];
                for w in warnings(report) {
                    let (n, rest) = w.split_once(' ').unwrap_or(("1", &w));
                    let entry = warned.entry(rest.to_string()).or_default();
                    entry.0 += 1;
                    entry.1 += n.parse::<usize>().unwrap_or(1);
                }
                println!("ok {p}: {report}");
            }
            Err((ErrorKind::File, m)) if m.contains("nothing to import") => empty += 1,
            Err((ErrorKind::InvalidInput, m)) if m.contains("2020") => legacy += 1,
            Err((kind, m)) => failed.push(format!("{p}: {kind:?}: {m}")),
        }
    }
    println!(
        "{} files: {imported} imported, {empty} empty, {legacy} legacy refused, {} failed",
        files.len(),
        failed.len()
    );
    for (w, (files, total)) in &warned {
        println!("warning in {files} files, {total} in all: {w}");
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read corpus dir") {
        let path = entry.expect("corpus entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skp"))
        {
            out.push(path);
        }
    }
}
