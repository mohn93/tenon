//! Importing SketchUp files (2017 and newer) through JSON commands, over the CC BY 4.0
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
    // Its layers (if any but Layer0) are tags now, with no warning.
    assert_eq!(
        report["tags"].as_u64().unwrap_or(0) as usize,
        tags_in(&path),
        "{report}"
    );
    assert!(!warnings.iter().any(|w| w.contains("tag")), "{warnings:?}");
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
        warnings.contains(
            &"1 texture was not imported (the image is over 16 MiB or 4096 pixels a side); \
              its material keeps its colour"
                .into()
        ),
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
        let result = run(&mut d, json!({"type": "import", "path": path}));
        let _ = std::fs::remove_file(&path);
        match result {
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
fn older_than_2017_is_refused() {
    let mut d = doc();
    for version in ["{16.1.1451}", "{13.0.3689}", ""] {
        let mut bytes = utf16_tag("SketchUp Model");
        if !version.is_empty() {
            bytes.extend(utf16_tag(version));
        }
        bytes.extend([0x01, 0x00, 0x00, 0x00, b'C', b'V', b'e', b'r']);
        bytes.extend([0u8; 300]);
        for name in ["old.skp", "old.model"] {
            // Without an extension it knows, the header says SketchUp.
            let path = write(name, &bytes);
            let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
            assert_eq!(kind, ErrorKind::InvalidInput, "{version}: {message}");
            assert!(message.contains("older than 2017"), "{message}");
        }
    }
    assert_eq!(d.scene(), doc().scene());

    // A 2020 header over bytes that are no model: a file error, nothing changes.
    let mut bytes = utf16_tag("SketchUp Model");
    bytes.extend(utf16_tag("{20.0.373}"));
    bytes.extend([0x01, 0x00, 0x00, 0x00, b'C', b'V', b'e', b'r']);
    bytes.extend([0u8; 300]);
    let path = write("broken-2020.skp", &bytes);
    let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::File, "{message}");
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

/// The cupboard as SketchUp 2024 saved it in the 2017 to 2020 formats.
const LEGACY_CUPBOARDS: [&str; 4] = [
    "legacy/cupboard_v2017.skp",
    "legacy/cupboard_v2018.skp",
    "legacy/cupboard_v2019.skp",
    "legacy/cupboard_v2020.skp",
];

/// The report's counts the legacy copies must share with the original.
const COUNTS: [&str; 9] = [
    "definitions",
    "instances",
    "polygons",
    "faces",
    "vertices",
    "materials",
    "skipped",
    "guides",
    "textures",
];

/// Imports `path` into a new document: the document and the report.
fn import(path: &str) -> (Document, Value) {
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path}))
        .unwrap_or_else(|e| panic!("{path}: {e:?}"));
    d.scene().validate().unwrap();
    (d, r["import"].clone())
}

/// The bounds of the one instance an import placed in the root.
fn import_bounds(d: &Document) -> [[f64; 3]; 2] {
    let roots = root_instances(d);
    assert_eq!(roots.len(), 1);
    let b = &roots[0]["bounds"];
    ["min", "max"].map(|k| [0, 1, 2].map(|i| b[k][i].as_f64().unwrap()))
}

fn close(a: [[f64; 3]; 2], b: [[f64; 3]; 2], tolerance: f64) -> bool {
    (0..2).all(|k| (0..3).all(|i| (a[k][i] - b[k][i]).abs() <= tolerance))
}

#[test]
fn legacy_cupboards_import_like_the_original() {
    let (original, want) = import(&fixture("cupboard.skp"));
    let want_bounds = import_bounds(&original);
    for name in LEGACY_CUPBOARDS {
        let (d, report) = import(&fixture(name));
        for k in COUNTS {
            assert_eq!(report[k], want[k], "{name}: {k}: {report} against {want}");
        }
        assert!(
            close(import_bounds(&d), want_bounds, 1e-3),
            "{name}: {:?} against {want_bounds:?}",
            import_bounds(&d)
        );
        // The knob and the hinge are each still one component placed twice.
        let defs = definitions(&d);
        for part in ["knob", "Hinge"] {
            let found: Vec<&Value> = defs.iter().filter(|x| x["name"] == part).collect();
            assert_eq!(found.len(), 1, "{name}: {part}");
            assert_eq!(found[0]["kind"], "component", "{name}: {}", found[0]);
            assert_eq!(found[0]["instances"], 2, "{name}: {}", found[0]);
        }
        let mut names: Vec<Value> = materials_of(&d).iter().map(|m| m["name"].clone()).collect();
        let mut want_names: Vec<Value> = materials_of(&original)
            .iter()
            .map(|m| m["name"].clone())
            .collect();
        names.sort_by_key(Value::to_string);
        want_names.sort_by_key(Value::to_string);
        assert_eq!(names, want_names, "{name}");
    }
}

#[test]
fn opening_a_legacy_file_works() {
    let open = |path: &str| {
        let mut d = doc();
        let r = run(&mut d, json!({"type": "open", "path": path})).unwrap();
        d.scene().validate().unwrap();
        (d, r["import"].clone())
    };
    let (want, want_report) = open(&fixture("cupboard.skp"));
    let path = fixture("legacy/cupboard_v2017.skp");
    let (d, report) = open(&path);
    for k in COUNTS {
        assert_eq!(report[k], want_report[k], "{k}: {report}");
    }
    assert!(
        report.get("instance").is_none_or(Value::is_null),
        "{report}"
    );
    let file = ask(&d, json!({"type": "file"}));
    assert_eq!(file["path"], Value::Null);
    assert_eq!(file["modified"], false);
    assert_eq!(d.imported_from(), Some(Path::new(&path)));
    assert_eq!(root_instances(&d).len(), root_instances(&want).len());
    assert_eq!(root_faces(&d), root_faces(&want));
    assert_eq!(definitions(&d).len(), definitions(&want).len());
}

#[test]
fn guides_are_imported() {
    // The cupboards have none, in either container.
    for name in ["cupboard.skp"].into_iter().chain(LEGACY_CUPBOARDS) {
        let (d, report) = import(&fixture(name));
        assert!(
            report.get("guides").is_none_or(|g| g == 0),
            "{name}: {report}"
        );
        assert!(
            !warnings(&report).iter().any(|w| w.contains("guide")),
            "{name}: {report}"
        );
        d.scene().validate().unwrap();
    }

    // Guides of a crafted 2023 file: in the root and in a definition placed 10 inches along x.
    use craft::{Guide, file_with_guides, scaled};
    let root_guides = [
        Guide::Line([1.0, 2.0, 3.0], [0.0, 0.0, 5.0]),
        Guide::Point([10.0, 20.0, 30.0]),
        // Not imported: no direction, not finite, beyond the coordinate limit.
        Guide::Line([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        Guide::Point([f64::NAN, 0.0, 0.0]),
        Guide::Line([0.0, 0.0, 0.0], [f64::INFINITY, 0.0, 0.0]),
        Guide::Point([1e9, 0.0, 0.0]),
    ];
    let bytes = file_with_guides(
        &[(vec![], vec![Guide::Point([1.0, 0.0, 0.0])])],
        &[(0, scaled(1.0, 10.0), None)],
        &root_guides,
    );
    let path = write("guides.skp", &bytes);
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": path, "as": "context"}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    assert_eq!(report["guides"], 3, "{report}");
    assert!(
        warnings(report).contains(&"4 guides could not be read and were skipped".to_string()),
        "{report}"
    );
    let guides = ask(&d, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
        .as_array()
        .unwrap()
        .clone();
    let near =
        |v: &Value, want: [f64; 3]| (0..3).all(|i| (v[i].as_f64().unwrap() - want[i]).abs() < 1e-9);
    assert_eq!(guides.len(), 2, "{guides:?}");
    let line = guides.iter().find(|g| g["kind"] == "line").unwrap();
    assert!(near(&line["point"], [0.0254, 0.0508, 0.0762]), "{line}");
    assert!(near(&line["direction"], [0.0, 0.0, 1.0]), "{line}");
    let point = guides.iter().find(|g| g["kind"] == "point").unwrap();
    assert!(near(&point["point"], [0.254, 0.508, 0.762]), "{point}");

    // The definition's guide point, in the definition's own coordinates.
    let instance = root_instances(&d)[0]["id"].clone();
    run(&mut d, json!({"type": "open_instance", "id": instance})).unwrap();
    let inner = ask(&d, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(inner.len(), 1, "{inner:?}");
    assert!(near(&inner[0]["point"], [0.0254, 0.0, 0.0]), "{inner:?}");

    // A guide point is part of the model's size: one far out in a scaled-up definition makes
    // the model too large.
    let bytes = file_with_guides(
        &[(vec![], vec![Guide::Point([1e7, 0.0, 0.0])])],
        &[(0, scaled(1000.0, 0.0), None)],
        &[],
    );
    let (d, r, _) = import_timed("far-guide.skp", &bytes);
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert!(message.contains("beyond"), "{message}");
    assert_eq!(d.scene(), doc().scene());
}

#[test]
fn mutated_legacy_files_give_errors_not_panics() {
    for (n, name) in LEGACY_CUPBOARDS.iter().enumerate() {
        let original = std::fs::read(fixture(name)).unwrap();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64 ^ n as u64;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % below as u64) as usize
        };
        let (mut ok, mut failed) = (0, 0);
        // 52 mutants over the four versions, past the header (which would only refuse).
        for k in 0..13 {
            let mut bytes = original.clone();
            let body = 64 + next(bytes.len() - 64);
            match k % 4 {
                0 => bytes.truncate(body),
                1 => {
                    for _ in 0..1 + next(8) {
                        let at = 64 + next(bytes.len() - 64);
                        bytes[at] ^= 1 << next(8);
                    }
                }
                2 => {
                    let end = (body + 1 + next(64)).min(bytes.len());
                    for b in &mut bytes[body..end] {
                        *b = next(256) as u8;
                    }
                }
                _ => {
                    let end = (body + 8).min(bytes.len());
                    bytes[body..end].fill(0xff);
                }
            }
            let path = write(&format!("legacy-mutant-{n}-{k}.skp"), &bytes);
            let mut d = doc();
            run(
                &mut d,
                json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
            )
            .unwrap();
            let before = d.scene().clone();
            let result = run(&mut d, json!({"type": "import", "path": path}));
            let _ = std::fs::remove_file(&path);
            match result {
                Ok(_) => ok += 1,
                Err((kind, message)) => {
                    failed += 1;
                    assert!(
                        matches!(kind, ErrorKind::File | ErrorKind::InvalidInput),
                        "{kind:?}: {message}"
                    );
                    assert_eq!(d.scene(), &before, "{name} mutant {k}: {message}");
                }
            }
            d.scene().validate().unwrap();
        }
        println!("{name} mutants: {ok} imported, {failed} refused");
    }
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

    /// The file's layers and which of them entities are on (by pid; `None`: no layer
    /// reference).
    #[derive(Default)]
    pub struct Layering {
        /// Pid, name and hidden flag of each layer.
        pub layers: Vec<(u64, String, bool)>,
        /// Every definition's edge.
        pub edge: Option<u64>,
        /// The root's placements.
        pub instances: Option<u64>,
        /// The root's guides.
        pub guides: Option<u64>,
    }

    fn layer_ref(layer: Option<u64>) -> Vec<u8> {
        layer.map_or_else(Vec::new, |l| id(t::DRAWING_ELEMENT_LAYER_PID_REF, l))
    }

    fn instances(places: &[Place], layer: Option<u64>) -> Vec<u8> {
        let mut list = Vec::new();
        for (k, (def, m, material)) in places.iter().enumerate() {
            let mut body = id(t::ENTITY_PID, 100_000 + k as u64);
            body.extend(id(t::COMPONENT_INSTANCE_DEF_PID_REF, def_pid(*def)));
            let m: Vec<u8> = m.iter().flat_map(|x| x.to_le_bytes()).collect();
            body.extend(rec(t::COMPONENT_INSTANCE_TRANSFORM, &m));
            if let Some(material) = material {
                body.extend(id(t::DRAWING_ELEMENT_MATERIAL_PID_REF, *material));
            }
            body.extend(layer_ref(layer));
            list.extend(rec(t::COMPONENT_INSTANCE, &body));
        }
        rec(t::COMPONENT_INSTANCES, &list)
    }

    fn def_pid(i: usize) -> u64 {
        10 + i as u64
    }

    /// A one-inch edge along x, on `layer`.
    fn edge(layer: Option<u64>) -> Vec<u8> {
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
        e.extend(layer_ref(layer));
        out.extend(rec(t::COMPONENT_EDGES, &rec(t::EDGE, &e)));
        out
    }

    /// A guide as a file states it, in inches: a line (point, direction) or a point.
    pub enum Guide {
        Line([f64; 3], [f64; 3]),
        Point([f64; 3]),
    }

    fn doubles(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A component's guide lists (only those it has), every guide on `layer`.
    fn guide_lists(guides: &[Guide], layer: Option<u64>) -> Vec<u8> {
        let (mut lines, mut points) = (Vec::new(), Vec::new());
        let element = || match layer {
            Some(_) => rec(
                t::CONSTRUCTION_GEOMETRY,
                &rec(t::DRAWING_ELEMENT, &layer_ref(layer)),
            ),
            None => Vec::new(),
        };
        for g in guides {
            match g {
                Guide::Line(p, d) => {
                    // Point, direction, then the start and end parameters.
                    let body = doubles(&[p[0], p[1], p[2], d[0], d[1], d[2], 0.0, 0.0]);
                    let mut line = element();
                    line.extend(rec(t::CONSTRUCTION_LINE_GEOM_LINE, &body));
                    lines.extend(rec(t::CONSTRUCTION_LINE, &line));
                }
                Guide::Point(p) => {
                    let mut point = element();
                    point.extend(rec(t::CONSTRUCTION_POINT_POSITION, &doubles(p)));
                    points.extend(rec(t::CONSTRUCTION_POINT, &point));
                }
            }
        }
        let mut out = Vec::new();
        if !lines.is_empty() {
            out.extend(rec(t::COMPONENT_CONSTRUCTION_LINES, &lines));
        }
        if !points.is_empty() {
            out.extend(rec(t::COMPONENT_CONSTRUCTION_POINTS, &points));
        }
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
        let definitions: Vec<(Vec<Place>, Vec<Guide>)> =
            definitions.iter().map(|p| (p.clone(), vec![])).collect();
        build(
            &definitions,
            root,
            &[],
            materials,
            entries,
            &Layering::default(),
        )
    }

    /// A SketchUp 2023 file whose definitions (placements, guides) and root have guides.
    pub fn file_with_guides(
        definitions: &[(Vec<Place>, Vec<Guide>)],
        root: &[Place],
        root_guides: &[Guide],
    ) -> Vec<u8> {
        build(
            definitions,
            root,
            root_guides,
            &[],
            &[],
            &Layering::default(),
        )
    }

    /// A SketchUp 2023 file with layers, its entities on them as `layering` says.
    pub fn file_with_layers(
        definitions: &[(Vec<Place>, Vec<Guide>)],
        root: &[Place],
        root_guides: &[Guide],
        layering: &Layering,
    ) -> Vec<u8> {
        build(definitions, root, root_guides, &[], &[], layering)
    }

    fn build(
        definitions: &[(Vec<Place>, Vec<Guide>)],
        root: &[Place],
        root_guides: &[Guide],
        materials: &[(u64, String)],
        entries: &[(String, Vec<u8>)],
        layering: &Layering,
    ) -> Vec<u8> {
        let mut defs = Vec::new();
        for (i, (places, guides)) in definitions.iter().enumerate() {
            let mut component = id(t::ENTITY_PID, def_pid(i));
            component.extend(edge(layering.edge));
            component.extend(instances(places, None));
            component.extend(guide_lists(guides, None));
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
        let mut root = instances(root, layering.instances);
        root.extend(guide_lists(root_guides, layering.guides));
        let root = rec(t::SKETCH_UP_MODEL_ROOT, &rec(t::COMPONENT, &root));
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
        let mut layers = Vec::new();
        for (pid, name, hidden) in &layering.layers {
            let mut l = rec(t::ENTITY, &id(t::ENTITY_PID, *pid));
            l.extend(rec(t::LAYER_NAME, name.as_bytes()));
            l.extend(rec(t::LAYER_HIDDEN, &[u8::from(*hidden)]));
            layers.extend(rec(t::LAYER, &l));
        }
        let layers = rec(
            t::SKETCH_UP_MODEL_LAYERS,
            &rec(t::LAYER_MANAGER, &rec(t::LAYER_MANAGER_LAYERS, &layers)),
        );
        let model = rec(
            t::SKETCH_UP_MODEL,
            &[layers, materials, components, root].concat(),
        );

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
    let took = start.elapsed();
    let _ = std::fs::remove_file(&path);
    (d, r, took)
}

#[test]
fn many_placements_are_read_without_walking_them() {
    use craft::{file, scaled};
    // `levels` definitions, each placing the next 10 times, the last an edge.
    let fan = |levels: usize| {
        let mut defs: Vec<Vec<craft::Place>> = (0..levels - 1)
            .map(|i| {
                (0..10)
                    .map(|k| (i + 1, scaled(1.0, k as f64), None))
                    .collect()
            })
            .collect();
        defs.push(vec![]);
        file(&defs, &[(0, scaled(1.0, 0.0), None)], &[], &[])
    };
    // 31 levels: 10^30 placements through the nesting from 301 instances, refused at once.
    let (d, r, took) = import_timed("deep.skp", &fan(31));
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::File, "{message}");
    assert!(message.contains("placements"), "{message}");
    assert_eq!(d.scene(), doc().scene());
    assert!(took.as_secs_f64() < 2.0, "took {took:?}");

    // 6 levels: 111,111 placements, under the cap; it imports and opens quickly.
    let (d, r, took) = import_timed("fan.skp", &fan(6));
    let r = r.unwrap();
    d.scene().validate().unwrap();
    assert_eq!(r["import"]["definitions"], 6, "{}", r["import"]);
    assert_eq!(r["import"]["instances"], 51);
    assert!(took.as_secs_f64() < 2.0, "took {took:?}");
    let path = write("fan-open.skp", &fan(6));
    let mut d = doc();
    let start = std::time::Instant::now();
    run(&mut d, json!({"type": "open", "path": path})).unwrap();
    assert!(start.elapsed().as_secs_f64() < 2.0, "{:?}", start.elapsed());
    d.scene().validate().unwrap();

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
fn a_model_claiming_to_unpack_too_large_is_refused_unread() {
    use craft::{file, scaled};
    let mut bytes = file(&[vec![]], &[(0, scaled(1.0, 0.0), None)], &[], &[]);
    // model.dat is the first entry: claim 600 MiB in its local and central headers.
    let local = bytes.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
    let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    for at in [local + 22, central + 24] {
        bytes[at..at + 4].copy_from_slice(&(600u32 << 20).to_le_bytes());
    }
    let (d, r, took) = import_timed("bomb.skp", &bytes);
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert!(message.contains("512 MiB"), "{message}");
    assert_eq!(d.scene(), doc().scene());
    assert!(took.as_secs_f64() < 1.0, "took {took:?}");
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

    // Two levels each turned 45 degrees and scaled about 1e200: the inner one's box is
    // finite, the outer one's runs out of f64 (inf - inf, NaN) on the way.
    let turn = [
        1e200, -1e200, 0.0, 1e200, 1e200, 0.0, 0.0, 0.0, 1e200, 0.0, 0.0, 0.0, 1.0,
    ];
    let defs = vec![vec![(1, turn, None)], vec![(2, turn, None)], vec![]];
    let bytes = file(&defs, &[(0, scaled(1.0, 0.0), None)], &[], &[]);
    let (d, r, _) = import_timed("overflow.skp", &bytes);
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert!(message.contains("beyond"), "{message}");
    assert_eq!(d.scene(), doc().scene());

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
        warnings.contains(
            &"1 material reference could not be matched to a material and was left off".into()
        ),
        "{warnings:?}"
    );
}

#[test]
fn a_file_naming_very_many_materials_is_read_quickly() {
    use craft::{file, scaled};
    let n = 5000;
    // Every material the same name, the hardest case for unique naming.
    let materials: Vec<(u64, String)> = (0..n).map(|k| (1000 + k as u64, "Same".into())).collect();
    let root: Vec<craft::Place> = (0..n)
        .map(|k| (0, scaled(1.0, k as f64), Some(1000 + k as u64)))
        .collect();
    let bytes = file(&[vec![]], &root, &materials, &[]);
    let (d, r, took) = import_timed("5000-materials.skp", &bytes);
    let r = r.unwrap();
    d.scene().validate().unwrap();
    assert!(took.as_secs_f64() < 5.0, "took {took:?}");
    assert_eq!(r["import"]["materials"], session::MAX_IMPORT_MATERIALS);
    let names: Vec<Value> = materials_of(&d).iter().map(|m| m["name"].clone()).collect();
    assert!(names.contains(&json!("Same")) && names.contains(&json!("Same 1024")));
    let dropped = n - session::MAX_IMPORT_MATERIALS;
    let warnings = warnings(&r["import"]);
    assert!(
        warnings.contains(&format!(
            "{dropped} materials past the 1024 an import may add were left off"
        )),
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
        warnings.contains(
            &"1 texture was not imported (the model holds at most 256 textures); its material \
              keeps its colour"
                .into()
        ),
        "{warnings:?}"
    );
}

/// Imports every `*.skp` under `$SKP_CORPUS`: files from SketchUp 2021 and newer import
/// without error, or (empty templates) say there is nothing to import; files from 2017 to
/// 2020 import too or fail with a `file` error (listed); older ones are refused with the 2017
/// message. Prints each report and the warnings over all files.
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
    let (mut imported, mut empty, mut old, mut failed) = (0, 0, 0, Vec::new());
    let mut legacy_failed = Vec::new();
    let is_legacy = |path: &Path| {
        let src = skp::source::MemorySource::new(std::fs::read(path).unwrap());
        skp::container::Container::open(&src)
            .is_ok_and(|c| c.generation == skp::container::Generation::Legacy)
    };
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
            Err((ErrorKind::InvalidInput, m)) if m.contains("older than 2017") => old += 1,
            Err((ErrorKind::File, m)) if is_legacy(path) => {
                assert_eq!(d.scene(), doc().scene(), "{p}");
                legacy_failed.push(format!("{p}: {m}"));
            }
            Err((kind, m)) => failed.push(format!("{p}: {kind:?}: {m}")),
        }
    }
    println!(
        "{} files: {imported} imported, {empty} empty, {old} older than 2017 refused, {} from \
         2017-2020 could not be read, {} failed",
        files.len(),
        legacy_failed.len(),
        failed.len()
    );
    for f in &legacy_failed {
        println!("2017-2020 file not read: {f}");
    }
    for (w, (files, total)) in &warned {
        println!("warning in {files} files, {total} in all: {w}");
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));
}

/// Imports each model of `$SKP_CORPUS/../legacy_src` and its copies saved by SketchUp 2024 in
/// the 2017 to 2020 formats (`$SKP_CORPUS/../legacy_out/<model>_v20xx.skp`). Every copy
/// imports or fails with a `file` or `invalid_input` message, leaving the model as it was.
/// Prints, per version, how many copies import like their original: the same definitions,
/// instances, materials and guides (the top level's at the same places, within 1 micrometer),
/// faces made within 1% and bounds within 1 mm; and how many give the very same report.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_legacy_imports() {
    let Some(dir) = std::env::var_os("SKP_CORPUS") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let base = Path::new(&dir).join("..");
    let mut originals = Vec::new();
    collect(&base.join("legacy_src"), &mut originals);
    originals.sort();
    let versions = ["2017", "2018", "2019", "2020"];
    let mut matched = [0usize; 4];
    let mut imported = [0usize; 4];
    let mut identical = [0usize; 4];
    let mut problems = Vec::new();
    let try_import = |path: &str| {
        let mut d = doc();
        let r = run(&mut d, json!({"type": "import", "path": path}));
        if r.is_err() {
            assert_eq!(d.scene(), doc().scene(), "{path}");
        }
        d.scene().validate().unwrap();
        r.map(|r| {
            let bounds = import_bounds(&d);
            // The top level's guides, in the wrapper's coordinates, in a stable order.
            let wrapper = root_instances(&d)[0]["id"].clone();
            run(&mut d, json!({"type": "open_instance", "id": wrapper})).unwrap();
            let mut guides: Vec<Vec<f64>> =
                ask(&d, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|g| {
                        let mut v: Vec<f64> = g["point"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_f64().unwrap())
                            .collect();
                        if let Some(dir) = g["direction"].as_array() {
                            v.extend(dir.iter().map(|x| x.as_f64().unwrap()));
                        }
                        v
                    })
                    .collect();
            guides.sort_by(|a, b| a.partial_cmp(b).unwrap());
            (r["import"].clone(), bounds, guides)
        })
    };
    let count = |r: &Value, k: &str| r[k].as_u64().unwrap_or(0);
    for original in &originals {
        let stem = original.file_stem().unwrap().to_str().unwrap();
        let want = try_import(original.to_str().unwrap());
        let Ok((want, want_bounds, want_guides)) = want else {
            println!("{stem}: the original does not import: {want:?}");
            continue;
        };
        println!("{stem} original: {want}");
        if !want_guides.is_empty() {
            println!("{stem} top-level guides: {want_guides:?}");
        }
        for (k, v) in versions.iter().enumerate() {
            let path = base.join("legacy_out").join(format!("{stem}_v{v}.skp"));
            if !path.exists() {
                problems.push(format!("{stem} {v}: missing"));
                continue;
            }
            match try_import(path.to_str().unwrap()) {
                Ok((got, bounds, guides)) => {
                    imported[k] += 1;
                    let mut differ = Vec::new();
                    for key in ["definitions", "instances", "materials", "guides"] {
                        if count(&got, key) != count(&want, key) {
                            differ.push(format!(
                                "{key} {} (original {})",
                                count(&got, key),
                                count(&want, key)
                            ));
                        }
                    }
                    let (f, wf) = (count(&got, "faces") as f64, count(&want, "faces") as f64);
                    if (f - wf).abs() > 0.01 * wf {
                        differ.push(format!("faces {f} (original {wf})"));
                    }
                    let same_guides = guides.len() == want_guides.len()
                        && guides.iter().zip(&want_guides).all(|(a, b)| {
                            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
                        });
                    if !same_guides {
                        differ.push(format!("guides {guides:?} (original {want_guides:?})"));
                    }
                    if got == want {
                        identical[k] += 1;
                    }
                    if !close(bounds, want_bounds, 1e-3) {
                        differ.push(format!("bounds {bounds:?} (original {want_bounds:?})"));
                    }
                    if differ.is_empty() {
                        matched[k] += 1;
                    } else {
                        problems.push(format!("{stem} {v}: {}", differ.join(", ")));
                    }
                    println!("{stem} {v}: {got}");
                }
                Err((kind, message)) => {
                    assert!(
                        matches!(kind, ErrorKind::File | ErrorKind::InvalidInput),
                        "{stem} {v}: {kind:?}: {message}"
                    );
                    problems.push(format!("{stem} {v}: refused: {kind:?}: {message}"));
                }
            }
        }
    }
    println!("version | imported | match their original | identical report | of");
    for (k, v) in versions.iter().enumerate() {
        println!(
            "{v} | {} | {} | {} | {}",
            imported[k],
            matched[k],
            identical[k],
            originals.len()
        );
    }
    for p in &problems {
        println!("differs: {p}");
    }
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

/// Faces in the root.
fn root_faces(d: &Document) -> usize {
    ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn opening_a_skp_makes_a_new_untitled_model() {
    let path = fixture("cupboard.skp");
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    )
    .unwrap();

    // Unsaved changes: refused, nothing changes.
    let err = run(&mut d, json!({"type": "open", "path": path})).unwrap_err();
    assert_eq!(err.0, ErrorKind::UnsavedChanges, "{}", err.1);
    assert_eq!(ask(&d, json!({"type": "file"}))["modified"], true);
    assert!(root_instances(&d).is_empty());

    let r = run(
        &mut d,
        json!({"type": "open", "path": path, "discard": true}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    assert_eq!(report["polygons"], 609, "{report}");
    assert!(
        report.get("instance").is_none_or(Value::is_null),
        "{report}"
    );
    let file = ask(&d, json!({"type": "file"}));
    assert_eq!(file["path"], Value::Null);
    assert_eq!(file["modified"], false);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    assert_eq!(d.imported_from(), Some(Path::new(&path)));

    // No wrapper: the root holds what importing into the context gives.
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "import", "path": path, "as": "context"}),
    )
    .unwrap();
    let names = |d: &Document| {
        let mut n: Vec<String> = root_instances(d)
            .iter()
            .map(|i| i["name"].as_str().unwrap_or("").to_string())
            .collect();
        n.sort();
        n
    };
    assert!(root_instances(&d).len() > 1);
    assert_eq!(names(&d), names(&e));
    assert_eq!(root_faces(&d), root_faces(&e));
    assert_eq!(definitions(&d).len(), definitions(&e).len());
    assert!(definitions(&d).iter().all(|x| x["name"] != "cupboard"));
    assert_eq!(materials_of(&d).len(), materials_of(&e).len());

    // The camera shows the model.
    let camera = ask(&d, json!({"type": "camera"}));
    assert_ne!(camera, ask(&doc(), json!({"type": "camera"})));

    // Saving asks for a path; it never writes over the .skp.
    let err = run(&mut d, json!({"type": "save"})).unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput, "{}", err.1);
    assert!(err.1.contains("give a path"), "{}", err.1);
    let saved = scratch("opened-cupboard.skr");
    run(
        &mut d,
        json!({"type": "save", "path": saved.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(d.path(), Some(saved.as_path()));
    assert_eq!(d.imported_from(), None);
    let mut f = doc();
    run(
        &mut f,
        json!({"type": "open", "path": saved.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(names(&f), names(&d));
    assert_eq!(root_faces(&f), root_faces(&d));
    assert_eq!(definitions(&f).len(), definitions(&d).len());
}

#[test]
fn a_skp_is_opened_by_its_header_and_a_bad_one_changes_nothing() {
    let bytes = std::fs::read(fixture("cupboard.skp")).unwrap();
    let renamed = write("cupboard-renamed.model", &bytes);
    let mut d = doc();
    run(&mut d, json!({"type": "open", "path": renamed})).unwrap();
    assert!(!root_instances(&d).is_empty());
    assert_eq!(ask(&d, json!({"type": "file"}))["path"], Value::Null);

    // A truncated file: an error, and the model open before stays as it was.
    let cut = write("cupboard-cut.skp", &bytes[..bytes.len() / 2]);
    let before = (root_instances(&d).len(), root_faces(&d));
    let revision = ask(&d, json!({"type": "file"}))["revision"].clone();
    let err = run(
        &mut d,
        json!({"type": "open", "path": cut, "discard": true}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::File, "{}", err.1);
    assert_eq!((root_instances(&d).len(), root_faces(&d)), before);
    assert_eq!(ask(&d, json!({"type": "file"}))["revision"], revision);
    d.scene().validate().unwrap();
}

fn tags_of(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "tags"}))["tags"]
        .as_array()
        .unwrap()
        .clone()
}

/// A tag's name, visibility and count, by name.
fn tag_summary(d: &Document) -> Vec<(String, bool, u64)> {
    tags_of(d)
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["visible"].as_bool().unwrap(),
                t["count"].as_u64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn fixtures_with_only_layer0_import_untagged() {
    for name in ["cupboard.skp", "desk.skp"]
        .into_iter()
        .chain(LEGACY_CUPBOARDS)
    {
        let (d, report) = import(&fixture(name));
        assert!(report.get("tags").is_none(), "{name}: {report}");
        assert!(tags_of(&d).is_empty(), "{name}");
        assert!(
            !warnings(&report).iter().any(|w| w.contains("tag")),
            "{name}: {report}"
        );
    }
}

/// A crafted 2023 file with layers: Layer0, "Walls", a hidden "Roof", one named "Untagged"
/// and one with a blank name; the definition's edge and the root's guides on "Walls", the
/// root's placement on "Roof".
fn layered_file() -> Vec<u8> {
    use craft::{Guide, Layering, file_with_layers, scaled};
    let layering = Layering {
        layers: vec![
            (500, "Layer0".into(), false),
            (501, "Walls".into(), false),
            (502, "Roof".into(), true),
            (503, "Untagged".into(), false),
            (504, "  ".into(), false),
        ],
        edge: Some(501),
        instances: Some(502),
        guides: Some(501),
    };
    file_with_layers(
        &[(vec![], vec![])],
        &[(0, scaled(1.0, 0.0), None)],
        &[
            Guide::Point([1.0, 2.0, 3.0]),
            Guide::Line([0.0; 3], [1.0, 0.0, 0.0]),
        ],
        &layering,
    )
}

#[test]
fn layers_import_as_tags() {
    let path = write("layered.skp", &layered_file());
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_tag", "name": "Walls", "color": [1, 2, 3]}),
    )
    .unwrap();
    let before = tags_of(&d);
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    d.scene().validate().unwrap();
    let report = &r["import"];
    assert_eq!(report["tags"], 4, "{report}");
    assert!(
        !warnings(report).iter().any(|w| w.contains("tag")),
        "{report}"
    );
    // Names made unique against the model's tags (and "Untagged"); a blank one is "Tag".
    // The edge (in the definition) and both guides are on the file's "Walls"; the placement
    // on "Roof", which stays hidden.
    assert_eq!(
        tag_summary(&d),
        [
            ("Walls".to_string(), true, 0),
            ("Walls 2".to_string(), true, 3),
            ("Roof".to_string(), false, 1),
            ("Untagged 2".to_string(), true, 0),
            ("Tag".to_string(), true, 0),
        ]
    );
    // The file has no colours for them: each takes the next free one.
    let colors: Vec<Value> = tags_of(&d)[1..]
        .iter()
        .map(|t| t["color"].clone())
        .collect();
    assert_eq!(
        colors,
        kernel::TAG_COLORS[..4]
            .iter()
            .map(|c| json!(c))
            .collect::<Vec<_>>()
    );

    // One undo step takes the tags back with the rest.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(tags_of(&d), before);
    assert!(root_instances(&d).is_empty());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(tag_summary(&d).len(), 5);
    d.scene().validate().unwrap();

    // Opened as a new model, the names need no number.
    let mut opened = doc();
    run(&mut opened, json!({"type": "open", "path": path})).unwrap();
    opened.scene().validate().unwrap();
    assert_eq!(
        tag_summary(&opened),
        [
            ("Walls".to_string(), true, 3),
            ("Roof".to_string(), false, 1),
            ("Untagged 2".to_string(), true, 0),
            ("Tag".to_string(), true, 0),
        ]
    );
    // The root's placement is on "Roof"; the root's guides on "Walls".
    let roof = tags_of(&opened)[1]["id"].clone();
    let walls = tags_of(&opened)[0]["id"].clone();
    let placed = root_instances(&opened)[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        ask(&opened, json!({"type": "entity", "id": placed}))["tag"],
        roof
    );
    let guides = ask(&opened, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(guides.len(), 2);
    for g in &guides {
        let id = g["id"].as_str().unwrap();
        assert_eq!(
            ask(&opened, json!({"type": "entity", "id": id}))["tag"],
            walls
        );
    }
}

#[test]
fn a_failed_import_leaves_no_tags() {
    // The layers are read and added, but the definitions nest too deep to build: refused.
    use craft::{Layering, file_with_layers, scaled};
    let n = kernel::MAX_IMPORT_DEPTH + 2;
    let definitions: Vec<_> = (0..n)
        .map(|i| {
            let places = if i + 1 < n {
                vec![(i + 1, scaled(1.0, 0.0), None)]
            } else {
                vec![]
            };
            (places, vec![])
        })
        .collect();
    let bytes = file_with_layers(
        &definitions,
        &[(0, scaled(1.0, 0.0), None)],
        &[],
        &Layering {
            layers: vec![(501, "Walls".into(), false)],
            edge: Some(501),
            ..Layering::default()
        },
    );
    let (d, r, _) = import_timed("layered-deep.skp", &bytes);
    let (kind, message) = r.unwrap_err();
    assert_eq!(kind, ErrorKind::File, "{message}");
    assert!(message.contains("nested"), "{message}");
    assert_eq!(d.scene(), doc().scene());
}

/// The tutorial (`$SKP_CORPUS/su2024/su007_Tutorial01_.skp`) has seven tags besides Layer0,
/// six of them hidden. Opened from its SketchUp 2024 file and from its 2017-2020 copies
/// (`$SKP_CORPUS/../legacy_out/tutorial01_v20xx.skp`), each gives the same tags (name,
/// colour, visibility, entities on them); exported and opened again, the same.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_tutorial_tags() {
    let Some(dir) = std::env::var_os("SKP_CORPUS") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let dir = Path::new(&dir);
    let summary = |d: &Document| -> Vec<(String, Value, bool, u64)> {
        tags_of(d)
            .iter()
            .map(|t| {
                (
                    t["name"].as_str().unwrap().to_string(),
                    t["color"].clone(),
                    t["visible"].as_bool().unwrap(),
                    t["count"].as_u64().unwrap(),
                )
            })
            .collect()
    };
    let open = |path: &Path| {
        let mut d = doc();
        let r = run(
            &mut d,
            json!({"type": "open", "path": path.to_str().unwrap()}),
        )
        .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        d.scene().validate().unwrap();
        (d, r)
    };
    let (d, r) = open(&dir.join("su2024/su007_Tutorial01_.skp"));
    let want = summary(&d);
    println!("tutorial: {want:?}");
    assert_eq!(r["import"]["tags"], 7, "{}", r["import"]);
    assert!(!warnings(&r["import"]).iter().any(|w| w.contains("tag")));
    let mut names: Vec<&str> = want.iter().map(|t| t.0.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Scene 01-03",
            "Scene 02",
            "Scene 03",
            "Scene 04",
            "Scene 04-12",
            "Scene 05",
            "Scene 07"
        ]
    );
    assert_eq!(want.iter().filter(|t| !t.2).count(), 6, "{want:?}");
    assert!(want.iter().any(|t| t.3 > 0), "{want:?}");

    for year in 2017..=2020 {
        let (d, _) = open(&dir.join(format!("../legacy_out/tutorial01_v{year}.skp")));
        let got = summary(&d);
        println!("{year}: {got:?}");
        assert_eq!(got, want, "{year}");
    }

    let mut d = d;
    let path = scratch("tutorial-tags.skp");
    let _ = std::fs::remove_file(&path);
    let out = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    println!("exported: {}", out["export"]);
    let (back, _) = open(&path);
    let used: Vec<_> = want.iter().filter(|t| t.3 > 0).cloned().collect();
    assert_eq!(summary(&back), used);
}
