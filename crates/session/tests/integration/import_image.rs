//! `import_image`: a picture as a textured rectangle in its own group.

use std::path::{Path, PathBuf};

use base64::Engine;
use glam::{DVec2, DVec3};
use kernel::{MeshId, back_texture_position};
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

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-import-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A `w` x `h` pixel PNG whose pixels differ along x, as a file.
fn png(name: &str, w: u32, h: u32) -> PathBuf {
    let path = scratch(name);
    image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x * 7) as u8, (y * 9) as u8, 90, 255])
    })
    .save(&path)
    .unwrap();
    path
}

fn pic(path: &Path, extra: Value) -> Value {
    let mut v = json!({"type": "import_image", "path": path.to_str().unwrap(),
                       "origin": [0, 0, 0], "width": 4});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

/// The group's definition mesh: its one face with the corners' positions, the face's front
/// texture coordinates and its normal.
fn corners(d: &Document, out: &Value) -> (Vec<DVec3>, Vec<DVec2>, Vec<DVec2>, DVec3) {
    let def: kernel::DefinitionId = out["definitions"][0].as_str().unwrap().parse().unwrap();
    let mesh = d.scene().mesh(MeshId::Definition(def)).unwrap();
    let faces: Vec<_> = mesh.faces().collect();
    assert_eq!(faces.len(), 1);
    let (_, face) = faces[0];
    let points: Vec<DVec3> = face
        .outer
        .iter()
        .map(|v| mesh.position(*v).unwrap())
        .collect();
    let front: Vec<DVec2> = points.iter().map(|p| face.texture_position(*p)).collect();
    let back: Vec<DVec2> = front.iter().map(|f| back_texture_position(*f)).collect();
    (points, front, back, face.normal)
}

fn near(a: DVec2, b: DVec2) -> bool {
    (a - b).length() < 1e-9
}

#[test]
fn a_picture_is_one_group_with_one_face_of_the_images_proportions() {
    let mut d = doc();
    let file = png("wide.png", 8, 4);
    let out = run(&mut d, pic(&file, json!({"origin": [1, 2, 0]}))).unwrap();
    assert_eq!(out["placed"].as_array().unwrap().len(), 1);
    assert_eq!(out["definitions"].as_array().unwrap().len(), 1);
    let (points, front, back, normal) = corners(&d, &out);
    assert!((normal - DVec3::Z).length() < 1e-9);
    assert_eq!(points.len(), 4);
    // Four by two meters, lifted by the default millimeter, lower left corner at the origin.
    let lo = points.iter().fold(DVec3::MAX, |a, p| a.min(*p));
    let hi = points.iter().fold(DVec3::MIN, |a, p| a.max(*p));
    assert!((lo - DVec3::new(1.0, 2.0, 0.001)).length() < 1e-9, "{lo:?}");
    assert!((hi - DVec3::new(5.0, 4.0, 0.001)).length() < 1e-9, "{hi:?}");
    // The image fills the face: its corners are the face's corners, front and back.
    for ((p, f), b) in points.iter().zip(&front).zip(&back) {
        let at = DVec2::new(p.x - 1.0, p.y - 2.0);
        assert!(near(*f, at), "front {f:?} at {at:?}");
        // The back reads as the image seen from behind: the same image, mirrored in x.
        assert!(near(*b, DVec2::new(-at.x, at.y)), "back {b:?}");
    }

    let materials = ask(&d, json!({"type": "materials"}));
    let m = &materials["materials"][0];
    assert_eq!(m["name"], "wide");
    assert_eq!(m["texture"]["width"], 4.0);
    assert_eq!(m["texture"]["height"], 2.0);
    assert_eq!(m["texture"]["pixel_width"], 8);
    assert_eq!(m["faces"], 1);
    let groups = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    assert_eq!(groups["instances"].as_array().unwrap().len(), 1);
    // One undo step takes the group and its material away.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn x_axis_turns_the_picture_and_the_corners_stay_the_images() {
    let mut d = doc();
    let file = png("turned.png", 4, 2);
    let out = run(
        &mut d,
        pic(&file, json!({"width": 2, "x_axis": [0, 1, 0], "lift": 0})),
    )
    .unwrap();
    let (points, front, _, _) = corners(&d, &out);
    // Width along +y: the image's x runs along y, its y along -x (the normal is z).
    for (p, f) in points.iter().zip(&front) {
        assert!(near(*f, DVec2::new(p.y, -p.x)), "{p:?} {f:?}");
    }
}

#[test]
fn a_picture_on_a_tilted_face_lies_in_its_plane_slightly_in_front() {
    let mut d = doc();
    let file = png("tilt.png", 4, 4);
    let n = DVec3::new(0.0, -1.0, 1.0).normalize();
    let origin = DVec3::new(1.0, 1.0, 1.0);
    let out = run(
        &mut d,
        pic(
            &file,
            json!({"origin": origin.to_array(), "normal": n.to_array(), "width": 1}),
        ),
    )
    .unwrap();
    let (points, front, _, normal) = corners(&d, &out);
    assert!((normal - n).length() < 1e-9);
    for p in &points {
        let distance = (*p - origin).dot(n);
        assert!((distance - 0.001).abs() < 1e-9, "{distance}");
    }
    // The four corners are the image's four corners, whatever the tilt.
    let mut seen: Vec<(i32, i32)> = front
        .iter()
        .map(|f| ((f.x * 2.0).round() as i32, (f.y * 2.0).round() as i32))
        .collect();
    seen.sort();
    assert_eq!(seen, [(0, 0), (0, 2), (2, 0), (2, 2)]);
}

#[test]
fn data_and_name_work_and_one_of_path_or_data_is_needed() {
    let mut d = doc();
    let file = png("bytes.png", 2, 2);
    let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&file).unwrap());
    let out = run(
        &mut d,
        json!({"type": "import_image", "data": data, "origin": [0, 0, 0], "width": 1,
               "name": "Poster"}),
    )
    .unwrap();
    assert_eq!(out["placed"].as_array().unwrap().len(), 1);
    assert_eq!(
        ask(&d, json!({"type": "materials"}))["materials"][0]["name"],
        "Poster"
    );
    // A second picture of the same name gets its own material.
    run(
        &mut d,
        json!({"type": "import_image", "data": data, "origin": [3, 0, 0], "width": 1,
               "name": "Poster"}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "materials"}))["materials"][1]["name"],
        "Poster 2"
    );

    for bad in [
        json!({"type": "import_image", "origin": [0, 0, 0], "width": 1}),
        json!({"type": "import_image", "data": data, "path": file.to_str().unwrap(),
               "origin": [0, 0, 0], "width": 1}),
        json!({"type": "import_image", "data": "not base64!", "origin": [0, 0, 0], "width": 1}),
        json!({"type": "import_image", "data": data, "origin": [0, 0, 0], "width": 0}),
        json!({"type": "import_image", "data": data, "origin": [0, 0, 0], "width": 1,
               "name": " "}),
        json!({"type": "import_image", "data": data, "origin": [0, 0, 0], "width": 1,
               "x_axis": [0, 0, 1]}),
    ] {
        let revision = ask(&d, json!({"type": "history"}))["revision"].clone();
        let (kind, message) = run(&mut d, bad.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{bad} {message}");
        assert_eq!(
            ask(&d, json!({"type": "history"}))["revision"],
            revision,
            "{bad}"
        );
    }
}

#[test]
fn oversize_and_broken_images_are_refused_naming_the_limit() {
    let mut d = doc();
    // 8192 pixels wide: refused before decoding.
    let big = scratch("big.png");
    image::RgbaImage::new(8192, 1).save(&big).unwrap();
    let (kind, message) = run(&mut d, pic(&big, json!({}))).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(
        message.contains("8192") && message.contains("4096"),
        "{message}"
    );

    let text = scratch("notes.png");
    std::fs::write(&text, b"hello").unwrap();
    let (kind, _) = run(&mut d, pic(&text, json!({}))).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    let (kind, _) = run(&mut d, pic(&scratch("missing.png"), json!({}))).unwrap_err();
    assert_eq!(kind, ErrorKind::File);

    // The same image as base64 data is invalid input, and a long thin image whose height
    // would fall under the least texture size is refused with why.
    let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&big).unwrap());
    let (kind, message) = run(
        &mut d,
        json!({"type": "import_image", "data": data, "origin": [0, 0, 0], "width": 1}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("4096"), "{message}");
    let thin = png("thin.png", 4000, 1);
    let (kind, message) = run(&mut d, pic(&thin, json!({"width": 0.001}))).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("height"), "{message}");
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_image_survives_saving_and_exports_as_a_textured_face() {
    let mut d = doc();
    let file = png("kept.png", 4, 2);
    run(&mut d, pic(&file, json!({"width": 2}))).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    let skr = scratch("kept.skr");
    run(
        &mut d,
        json!({"type": "save", "path": skr.to_str().unwrap()}),
    )
    .unwrap();
    std::fs::remove_file(&file).unwrap();
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": skr.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(e.scene(), d.scene());
    let texture = e
        .scene()
        .materials()
        .next()
        .unwrap()
        .1
        .texture
        .clone()
        .unwrap();
    assert_eq!(texture.image.encoded, bytes, "the file as it was read");

    // glTF: the image is embedded and the face names texture coordinates.
    let glb = scratch("kept.glb");
    run(
        &mut e,
        json!({"type": "export", "path": glb.to_str().unwrap()}),
    )
    .unwrap();
    let raw = std::fs::read(&glb).unwrap();
    let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;
    let doc: Value = serde_json::from_slice(&raw[20..20 + word(12)]).unwrap();
    assert_eq!(doc["images"][0]["mimeType"], "image/png");
    assert_eq!(
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"],
        json!({"index": 0})
    );
    assert!(doc["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"].is_u64());
}

#[test]
fn the_image_corners_land_on_the_face_corners_for_any_normal_and_x_axis() {
    let cases: [([f64; 3], Option<[f64; 3]>); 7] = [
        ([0.0, 0.0, -1.0], None),
        ([0.0, 0.0, -1.0], Some([0.0, 1.0, 0.0])),
        ([-1.0, 0.0, 0.0], None),
        ([0.0, -1.0, 1.0], Some([1.0, 1.0, 0.0])),
        ([0.3, -0.5, -0.8], Some([1.0, 2.0, 3.0])),
        ([0.0, 0.0, 1.0], Some([1.0, 1e-12, 0.0])),
        ([1e-12, 0.0, 1.0], None),
    ];
    let file = png("corners.png", 4, 2);
    let origin = DVec3::new(3.0, -2.0, 5.0);
    for (n, given) in cases {
        let mut d = doc();
        let mut extra = json!({"origin": origin.to_array(), "normal": n, "width": 2, "lift": 0});
        if let Some(x) = given {
            extra["x_axis"] = json!(x);
        }
        let out = run(&mut d, pic(&file, extra)).unwrap();
        let (points, front, _, normal) = corners(&d, &out);
        let n = DVec3::from(n).normalize();
        assert!((normal - n).length() < 1e-6, "{n:?} {normal:?}");
        // Measured in the picture's own frame: width along x_axis, height along normal x x_axis.
        let x = points
            .iter()
            .zip(&front)
            .find(|(_, f)| near(**f, DVec2::new(2.0, 0.0)))
            .map(|(p, _)| (*p - origin) / 2.0)
            .expect("an image corner at (2, 0)");
        let y = n.cross(x);
        for (p, f) in points.iter().zip(&front) {
            let rel = *p - origin;
            let at = DVec2::new(rel.dot(x), rel.dot(y));
            assert!((*f - at).length() < 1e-6, "{n:?}: {f:?} at {at:?}");
        }
        if let Some(given) = given {
            let given = DVec3::from(given);
            let flat = (given - n * given.dot(n)).normalize();
            assert!((x - flat).length() < 1e-6, "{n:?}: width runs along {x:?}");
        }
    }
}

#[test]
fn a_photo_kept_on_its_side_is_placed_upright() {
    // A 16 x 8 pixel JPEG whose EXIF orientation (6) says to turn it a quarter clockwise.
    let mut jpeg = Vec::new();
    image::RgbImage::new(16, 8)
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let mut app1 =
        b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
    let mut photo = vec![0xFF, 0xD8, 0xFF, 0xE1];
    photo.extend(((app1.len() + 2) as u16).to_be_bytes());
    photo.append(&mut app1);
    photo.extend(&jpeg[2..]);
    let data = base64::engine::general_purpose::STANDARD.encode(photo);
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "import_image", "data": data, "origin": [0, 0, 0], "width": 1}),
    )
    .unwrap();
    let m = &ask(&d, json!({"type": "materials"}))["materials"][0];
    assert_eq!(m["texture"]["pixel_width"], 8, "{m}");
    assert_eq!(m["texture"]["height"], 2.0, "{m}");
}
