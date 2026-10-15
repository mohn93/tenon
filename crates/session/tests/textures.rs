//! Textures on materials, driven through JSON commands and queries.

use std::io::Write;
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

fn materials(d: &Document) -> Vec<Value> {
    let q: Query = serde_json::from_value(json!({"type": "materials"})).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()["materials"]
        .as_array()
        .unwrap()
        .clone()
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-textures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A 4 x 2 pixel PNG, its left half red and its right half blue.
fn red_blue_png(name: &str) -> PathBuf {
    let path = scratch(name);
    image::RgbaImage::from_fn(4, 2, |x, _| {
        if x < 2 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    })
    .save(&path)
    .unwrap();
    path
}

fn add_textured(d: &mut Document, path: &Path, width: f64) -> String {
    let out = run(
        d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": path.to_str().unwrap(), "width": width}}),
    )
    .unwrap();
    out["changes"]["materials"][0].as_str().unwrap().to_string()
}

#[test]
fn a_material_reads_a_texture_and_reports_it() {
    let png = red_blue_png("tiles.png");
    let mut d = doc();
    let id = add_textured(&mut d, &png, 0.5);
    let listed = materials(&d);
    // No color given: the image's average. No height: the image's proportions.
    assert_eq!(listed[0]["color"], json!([128, 0, 128]));
    assert_eq!(
        listed[0]["texture"],
        json!({"width": 0.5, "height": 0.25, "pixel_width": 4, "pixel_height": 2,
               "format": "png"})
    );

    // Resizing keeps the image; a size left out stays.
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "texture": {"height": 1.0}}),
    )
    .unwrap();
    assert_eq!(
        materials(&d)[0]["texture"],
        json!({"width": 0.5, "height": 1.0, "pixel_width": 4, "pixel_height": 2,
               "format": "png"})
    );
    // Renaming leaves the texture; null removes it and keeps the color.
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "name": "Plain"}),
    )
    .unwrap();
    assert!(materials(&d)[0]["texture"].is_object());
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "texture": null}),
    )
    .unwrap();
    let m = &materials(&d)[0];
    assert!(m.get("texture").is_none(), "{m}");
    assert_eq!(m["color"], json!([128, 0, 128]));

    // Undo puts the texture back as it was, a step at a time.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["texture"]["height"], 1.0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["texture"]["height"], 0.25);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(materials(&d)[0]["texture"]["height"], 1.0);
}

#[test]
fn a_new_image_takes_the_average_color_unless_one_is_given() {
    let png = red_blue_png("average.png");
    let mut d = doc();
    let out = run(&mut d, json!({"type": "add_material", "color": [1, 2, 3]})).unwrap();
    let id = out["changes"]["materials"][0].clone();
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "color": [9, 9, 9],
               "texture": {"path": png.to_str().unwrap()}}),
    )
    .unwrap();
    let m = &materials(&d)[0];
    assert_eq!(m["color"], json!([9, 9, 9]));
    // 1 m wide by default, as tall as the image's proportions.
    assert_eq!(
        (
            m["texture"]["width"].clone(),
            m["texture"]["height"].clone()
        ),
        (json!(1.0), json!(0.5))
    );
    run(
        &mut d,
        json!({"type": "change_material", "material": id,
               "texture": {"path": png.to_str().unwrap(), "height": 3.0}}),
    )
    .unwrap();
    let m = &materials(&d)[0];
    assert_eq!(m["color"], json!([128, 0, 128]));
    // A new image keeps the width of the one it replaces.
    assert_eq!(
        (
            m["texture"]["width"].clone(),
            m["texture"]["height"].clone()
        ),
        (json!(1.0), json!(3.0))
    );
}

#[test]
fn bad_textures_are_refused() {
    let png = red_blue_png("bad.png");
    let mut d = doc();
    let mut fails = |v: Value, kind: ErrorKind, text: &str| {
        let (k, message) = run(&mut d, v.clone()).unwrap_err();
        assert_eq!(k, kind, "{v}: {message}");
        assert!(message.contains(text), "{v}: {message}");
    };
    let missing = scratch("missing.png");
    fails(
        json!({"type": "add_material", "texture": {"path": missing.to_str().unwrap()}}),
        ErrorKind::File,
        "missing.png",
    );
    let text = scratch("not-an-image.png");
    std::fs::write(&text, "hello").unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": text.to_str().unwrap()}}),
        ErrorKind::File,
        "not a PNG or JPEG image",
    );
    let wide = scratch("wide.png");
    image::RgbaImage::new(4097, 1).save(&wide).unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": wide.to_str().unwrap()}}),
        ErrorKind::File,
        "at most 4096",
    );
    let big = scratch("big.png");
    let mut f = std::fs::File::create(&big).unwrap();
    f.write_all(&vec![0; 16 * 1024 * 1024 + 1]).unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": big.to_str().unwrap()}}),
        ErrorKind::File,
        "file too large",
    );
    let path = png.to_str().unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": path, "width": 0}}),
        ErrorKind::InvalidInput,
        "width must be from 0.001 to 1000 m",
    );
    fails(
        json!({"type": "add_material", "texture": {"width": 1}}),
        ErrorKind::InvalidInput,
        "needs a path",
    );
    fails(
        json!({"type": "add_material", "name": "Nothing"}),
        ErrorKind::InvalidInput,
        "a color or a texture",
    );
    // Unknown texture fields are not commands at all.
    let unknown = json!({"type": "add_material", "texture": {"path": path, "size": 1}});
    assert!(serde_json::from_value::<Command>(unknown).is_err());
    let out = run(&mut d, json!({"type": "add_material", "color": [1, 2, 3]})).unwrap();
    let id = out["changes"]["materials"][0].clone();
    let (kind, message) = run(
        &mut d,
        json!({"type": "change_material", "material": id, "texture": {"width": 2}}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("no texture"), "{message}");
    assert_eq!(d.revision(), 1, "nothing else changed the model");
}

#[test]
fn textures_are_saved_inside_the_file() {
    let png = red_blue_png("saved.png");
    let mut d = doc();
    let id = add_textured(&mut d, &png, 2.0);
    let path = scratch("textured.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(
        document["schema_version"],
        session::file::TEXTURED_SCHEMA_VERSION
    );
    assert_eq!(document["schema_version"], 7);
    let file = format!("textures/{id}.png");
    assert_eq!(
        document["model"]["materials"][0]["texture"],
        json!({"file": file, "width": 2.0, "height": 1.0})
    );
    let mut stored = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name(&file).unwrap(), &mut stored).unwrap();
    assert_eq!(
        stored,
        std::fs::read(&png).unwrap(),
        "the file as it was read"
    );

    // The image file is gone: the model has its own copy.
    std::fs::remove_file(&png).unwrap();
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));
    assert_eq!(e.scene(), d.scene());
}

#[test]
fn files_with_missing_or_broken_textures_do_not_open() {
    let png = red_blue_png("broken.png");
    let mut d = doc();
    add_textured(&mut d, &png, 1.0);
    let path = scratch("good.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    let file = document["model"]["materials"][0]["texture"]["file"]
        .as_str()
        .unwrap()
        .to_string();
    let rewrite = |name: &str, image: Option<&[u8]>| {
        let out = scratch(name);
        let mut w = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        w.start_file("document.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(&serde_json::to_vec(&document).unwrap())
            .unwrap();
        if let Some(image) = image {
            w.start_file(file.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(image).unwrap();
        }
        w.finish().unwrap();
        out
    };
    for (out, expected) in [
        (rewrite("no-image.skr", None), "is missing"),
        (
            rewrite("garbled.skr", Some(b"garbled")),
            "not a PNG or JPEG",
        ),
    ] {
        let mut e = doc();
        let (kind, message) = run(
            &mut e,
            json!({"type": "open", "path": out.to_str().unwrap()}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::File);
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn a_derived_height_is_clamped_to_the_size_limits() {
    let path = scratch("sliver.png");
    image::RgbaImage::from_pixel(4096, 3, image::Rgba([9, 9, 9, 255]))
        .save(&path)
        .unwrap();
    let mut d = doc();
    let id = add_textured(&mut d, &path, 1.0);
    let m = materials(&d)
        .into_iter()
        .find(|m| m["id"] == id.as_str())
        .unwrap();
    assert_eq!(m["texture"]["height"], 0.001);

    // A given height is still checked.
    let (kind, _) = run(
        &mut d,
        json!({"type": "change_material", "material": id,
               "texture": {"height": 0.0005}}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn a_file_with_too_many_textured_materials_does_not_open() {
    let png = red_blue_png("many.png");
    let mut d = doc();
    let id = add_textured(&mut d, &png, 1.0);
    let path = scratch("many-good.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    let file = format!("textures/{id}.png");
    let mut image = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name(&file).unwrap(), &mut image).unwrap();
    let first = document["model"]["materials"][0].clone();
    let materials = (1..=257)
        .map(|k| {
            let mut m = first.clone();
            m["id"] = json!(k);
            m["name"] = json!(format!("T{k}"));
            m
        })
        .collect::<Vec<_>>();
    document["model"]["materials"] = json!(materials);
    let out = scratch("many.skr");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
    w.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    w.start_file(file.as_str(), zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(&image).unwrap();
    w.finish().unwrap();
    let mut e = doc();
    let (kind, message) = run(
        &mut e,
        json!({"type": "open", "path": out.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("more than 256 materials"), "{message}");
}

/// A PNG for an image of `width` x `height` pixels with no pixel data: enough to read its size,
/// not to decode it.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for b in bytes {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut chunk = b"IHDR".to_vec();
    chunk.extend_from_slice(&width.to_be_bytes());
    chunk.extend_from_slice(&height.to_be_bytes());
    // 8-bit RGBA, default compression, filter and no interlace.
    chunk.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(&chunk);
    png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    // An empty data chunk and the end, so the header reads as complete.
    for name in [b"IDAT", b"IEND"] {
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(name);
        png.extend_from_slice(&crc32(name).to_be_bytes());
    }
    png
}

#[test]
fn a_file_whose_textures_hold_too_many_pixels_does_not_open() {
    // Seventeen 4096 x 4096 images: each within the limits, together over the 2^28-pixel
    // budget. They are refused from their headers, before any is decoded.
    let png = red_blue_png("budget.png");
    let mut d = doc();
    add_textured(&mut d, &png, 1.0);
    let path = scratch("budget-good.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    let first = document["model"]["materials"][0].clone();
    let materials = (1..=17)
        .map(|k| {
            let mut m = first.clone();
            m["id"] = json!(k);
            m["name"] = json!(format!("T{k}"));
            m["texture"]["file"] = json!(format!("textures/{k}.png"));
            m
        })
        .collect::<Vec<_>>();
    document["model"]["materials"] = json!(materials);
    let out = scratch("budget.skr");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
    w.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    for k in 1..=17 {
        w.start_file(
            format!("textures/{k}.png"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(&png_header(4096, 4096)).unwrap();
    }
    w.finish().unwrap();
    let mut e = doc();
    let (kind, message) = run(
        &mut e,
        json!({"type": "open", "path": out.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("too many texture pixels"), "{message}");
}
