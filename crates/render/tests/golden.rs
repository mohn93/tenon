//! Golden-image tests: canonical scenes rendered headless at a fixed size and compared with
//! the PNGs in `tests/golden/`, within a tolerance for GPU and driver differences.
//!
//! `UPDATE_GOLDEN=1 cargo test -p render --test golden` rewrites the stored images from this
//! machine's renders (check them by eye before committing). On a mismatch the actual image
//! and a diff image are written under the target's tmp directory and their paths reported.

use std::path::{Path, PathBuf};

use kernel::EntityId;
use render::Snapshotter;
use session::{Command, Document, MaterialId, Projection, Side, StandardView};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;

/// A pixel differs when one of its color channels differs by more than this.
const CHANNEL_TOLERANCE: u8 = 24;

/// An image matches when at most this fraction of its pixels differ.
const MAX_DIFFERING: f64 = 0.005;

/// How two images compare.
#[derive(Debug, PartialEq)]
enum Comparison {
    Match { differing: usize },
    SizeMismatch,
    Differ { differing: usize, total: usize },
}

/// Compares two tightly packed RGBA8 images of `width` x `height` on their color channels
/// (alpha is ignored), returning the outcome and a mask of the differing pixels.
fn compare(expected: &[u8], actual: &[u8], width: u32, height: u32) -> (Comparison, Vec<bool>) {
    let total = (width * height) as usize;
    if expected.len() != total * 4 || actual.len() != total * 4 {
        return (Comparison::SizeMismatch, Vec::new());
    }
    let mask: Vec<bool> = expected
        .chunks(4)
        .zip(actual.chunks(4))
        .map(|(e, a)| (0..3).any(|c| e[c].abs_diff(a[c]) > CHANNEL_TOLERANCE))
        .collect();
    let differing = mask.iter().filter(|d| **d).count();
    let outcome = if differing as f64 <= MAX_DIFFERING * total as f64 {
        Comparison::Match { differing }
    } else {
        Comparison::Differ { differing, total }
    };
    (outcome, mask)
}

/// The actual image dimmed to gray, with differing pixels in red.
fn diff_image(actual: &[u8], mask: &[bool]) -> Vec<u8> {
    actual
        .chunks(4)
        .zip(mask)
        .flat_map(|(p, &d)| {
            if d {
                [255, 0, 0, 255]
            } else {
                let gray = ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 6 + 64) as u8;
                [gray, gray, gray, 255]
            }
        })
        .collect()
}

fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create image folder");
    }
    let file = std::fs::File::create(path).expect("create image");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(rgba).expect("png data");
}

/// Reads a PNG as RGBA8 pixels with its size; None if it cannot be read.
fn read_png(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf,
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    Some((rgba, info.width, info.height))
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"))
}

fn updating() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1")
}

/// Renders `doc` with its camera, selection and open context, and checks the image against
/// `tests/golden/<name>.png` (or rewrites that file with `UPDATE_GOLDEN=1`). Skips without
/// a GPU adapter.
fn check(name: &str, doc: &Document) {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let actual = snap
        .capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            &doc.camera,
            WIDTH,
            HEIGHT,
        )
        .expect("capture");
    let golden = golden_path(name);
    if updating() {
        write_png(&golden, &actual, WIDTH, HEIGHT);
        eprintln!("wrote {}", golden.display());
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden");
    let actual_path = out.join(format!("{name}.actual.png"));
    let Some((expected, w, h)) = read_png(&golden) else {
        write_png(&actual_path, &actual, WIDTH, HEIGHT);
        panic!(
            "{name}: no readable golden image at {}; actual written to {}; \
             run with UPDATE_GOLDEN=1 to store it",
            golden.display(),
            actual_path.display()
        );
    };
    if (w, h) != (WIDTH, HEIGHT) {
        write_png(&actual_path, &actual, WIDTH, HEIGHT);
        panic!(
            "{name}: golden image is {w}x{h}, expected {WIDTH}x{HEIGHT}; actual written to {}",
            actual_path.display()
        );
    }
    match compare(&expected, &actual, WIDTH, HEIGHT) {
        (Comparison::Match { .. }, _) => {}
        (outcome, mask) => {
            let diff_path = out.join(format!("{name}.diff.png"));
            write_png(&actual_path, &actual, WIDTH, HEIGHT);
            if !mask.is_empty() {
                write_png(&diff_path, &diff_image(&actual, &mask), WIDTH, HEIGHT);
            }
            panic!(
                "{name}: render differs from {} ({outcome:?}); actual: {}, diff: {}; \
                 if the change is intended, rerun with UPDATE_GOLDEN=1",
                golden.display(),
                actual_path.display(),
                diff_path.display()
            );
        }
    }
}

fn run(doc: &mut Document, command: Command) -> session::Outcome {
    doc.execute(command.clone())
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
}

fn doc() -> Document {
    Document::new(glam::DVec2::new(WIDTH as f64, HEIGHT as f64))
}

fn look(doc: &mut Document, eye: [f64; 3], target: [f64; 3]) {
    run(
        doc,
        Command::SetCamera {
            eye,
            target,
            fov_y_deg: None,
            up: None,
            projection: None,
        },
    );
}

fn created_face(outcome: &session::Outcome) -> EntityId {
    *outcome
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Face(_)))
        .expect("a face was made")
}

/// A 2 x 1.5 x 1 m box with a corner at the origin; returns the top face.
fn add_box(doc: &mut Document, origin: [f64; 2]) -> EntityId {
    let [x, y] = origin;
    let base = run(
        doc,
        Command::AddRectangle {
            corner: [x, y, 0.0],
            opposite: [x + 2.0, y + 1.5, 0.0],
            normal: None,
        },
    );
    let face = created_face(&base);
    raise(doc, face, 1.0);
    top_face(doc, 1.0)
}

/// Push-pulls a ground face of the open context up by `height`, whichever way it faces.
fn raise(doc: &mut Document, face: EntityId, height: f64) {
    let EntityId::Face(id) = face else {
        panic!("{face} is not a face")
    };
    let up = doc.model().face(id).expect("face").normal.z > 0.0;
    let distance = if up { height } else { -height };
    run(doc, Command::PushPull { face, distance });
}

/// The face of the open context whose vertices all lie at height `z`.
fn top_face(doc: &Document, z: f64) -> EntityId {
    let model = doc.model();
    let (id, _) = model
        .faces()
        .find(|(_, f)| {
            f.outer
                .iter()
                .all(|v| model.position(*v).is_some_and(|p| (p.z - z).abs() < 1e-9))
        })
        .expect("top face");
    EntityId::Face(id)
}

fn add_material(doc: &mut Document, name: &str, color: [u8; 3]) -> MaterialId {
    let outcome = run(
        doc,
        Command::AddMaterial {
            name: Some(name.into()),
            color,
        },
    );
    outcome.changes.materials[0]
}

fn all_faces(doc: &Document) -> Vec<EntityId> {
    doc.model()
        .faces()
        .map(|(id, _)| EntityId::Face(id))
        .collect()
}

#[test]
fn compare_accepts_small_differences_and_rejects_large_ones() {
    let (w, h) = (20, 10);
    let base = vec![100u8; (w * h * 4) as usize];
    assert_eq!(
        compare(&base, &base, w, h).0,
        Comparison::Match { differing: 0 }
    );
    // Every channel off by the tolerance still matches.
    let near: Vec<u8> = base.iter().map(|v| v + CHANNEL_TOLERANCE).collect();
    assert_eq!(
        compare(&base, &near, w, h).0,
        Comparison::Match { differing: 0 }
    );
    // Alpha is ignored.
    let mut alpha = base.clone();
    alpha[3] = 0;
    assert_eq!(
        compare(&base, &alpha, w, h).0,
        Comparison::Match { differing: 0 }
    );
    // One pixel of 200 is 0.5%: allowed; two are not.
    let mut one = base.clone();
    one[0] = 200;
    assert_eq!(
        compare(&base, &one, w, h).0,
        Comparison::Match { differing: 1 }
    );
    let mut two = one.clone();
    two[4 * 57 + 2] = 0;
    let (outcome, mask) = compare(&base, &two, w, h);
    assert_eq!(
        outcome,
        Comparison::Differ {
            differing: 2,
            total: 200
        }
    );
    assert!(mask[0] && mask[57] && mask.iter().filter(|d| **d).count() == 2);
    assert_eq!(compare(&base, &base[4..], w, h).0, Comparison::SizeMismatch);
}

#[test]
fn diff_image_marks_differing_pixels_red() {
    let actual = [10, 20, 30, 255, 200, 200, 200, 255];
    let diff = diff_image(&actual, &[false, true]);
    assert_eq!(&diff[4..], &[255, 0, 0, 255]);
    assert_eq!(diff[0], diff[1]);
    assert_ne!(&diff[..3], &[255, 0, 0]);
}

#[test]
fn png_round_trip_keeps_pixels() {
    let (w, h) = (3, 2);
    let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 11) as u8).collect();
    let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("golden")
        .join(format!("round_trip_{}.png", std::process::id()));
    write_png(&path, &rgba, w, h);
    assert_eq!(read_png(&path), Some((rgba, w, h)));
    let _ = std::fs::remove_file(&path);
    assert_eq!(read_png(&path.with_extension("missing")), None);
}

#[test]
fn golden_box() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    look(&mut d, [5.0, -4.5, 3.5], [1.0, 0.75, 0.4]);
    check("box", &d);
}

#[test]
fn golden_painted_box_front_and_back() {
    let mut d = doc();
    let top = add_box(&mut d, [0.0, 0.0]);
    run(&mut d, Command::Erase { ids: vec![top] });
    let brick = add_material(&mut d, "Brick", [190, 70, 50]);
    let leaf = add_material(&mut d, "Leaf", [60, 170, 80]);
    let faces = all_faces(&d);
    for (material, side) in [(brick, Side::Front), (leaf, Side::Back)] {
        run(
            &mut d,
            Command::Paint {
                ids: faces.clone(),
                material: Some(material),
                side,
            },
        );
    }
    // From above, into the open box: outside walls are brick, the inside leaf green.
    look(&mut d, [4.0, -3.5, 4.5], [1.0, 0.75, 0.3]);
    check("painted_box", &d);
}

#[test]
fn golden_rotated_component() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: faces,
            name: "Crate".into(),
        },
    );
    let first = *made
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("an instance");
    let copied = run(
        &mut d,
        Command::Copy {
            ids: vec![first],
            offset: [3.0, 0.5, 0.0],
            count: None,
        },
    );
    let second = *copied
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("a copy");
    run(
        &mut d,
        Command::Rotate {
            ids: vec![second],
            center: [4.0, 1.25, 0.0],
            axis: None,
            angle: 35.0,
            copies: None,
        },
    );
    run(
        &mut d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
        },
    );
    look(&mut d, [7.0, -6.0, 5.0], [2.5, 1.0, 0.4]);
    check("rotated_component", &d);
}

#[test]
fn golden_guides_are_dashed() {
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [-1.5, -1.0, 0.0],
            opposite: [1.5, 1.0, 0.0],
            normal: None,
        },
    );
    run(
        &mut d,
        Command::AddGuideLine {
            point: [0.0, 1.0, 0.0],
            direction: [1.0, 0.0, 0.0],
        },
    );
    run(
        &mut d,
        Command::AddGuideLine {
            point: [-1.5, 0.0, 0.0],
            direction: [1.0, 1.0, 0.0],
        },
    );
    run(
        &mut d,
        Command::AddGuidePoint {
            position: [1.5, -1.0, 0.5],
        },
    );
    look(&mut d, [3.0, -4.5, 3.5], [0.0, 0.0, 0.0]);
    check("guides", &d);
}

#[test]
fn golden_cylinder_hides_soft_edges() {
    let mut d = doc();
    let circle = run(
        &mut d,
        Command::AddCircle {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
            normal: None,
            segments: Some(24),
        },
    );
    let face = created_face(&circle);
    raise(&mut d, face, 1.5);
    look(&mut d, [3.5, -4.0, 3.0], [0.0, 0.0, 0.6]);
    check("soft_cylinder", &d);
}

#[test]
fn golden_parallel_top_view() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::AddCircle {
            center: [3.5, 0.75, 0.0],
            radius: 0.75,
            normal: None,
            segments: Some(24),
        },
    );
    run(
        &mut d,
        Command::StandardView {
            view: StandardView::Top,
        },
    );
    run(
        &mut d,
        Command::SetProjection {
            projection: Projection::Parallel,
        },
    );
    run(&mut d, Command::ZoomExtents);
    check("parallel_top", &d);
}

#[test]
fn golden_selection_highlight() {
    let mut d = doc();
    let top = add_box(&mut d, [0.0, 0.0]);
    // One vertical edge: the one standing at the box's corner nearest the camera.
    let model = d.model();
    let (edge, _) = model
        .edges()
        .find(|(_, e)| {
            let [a, b] = e.v.map(|v| model.position(v).unwrap());
            (a.x - 2.0).abs() < 1e-9
                && a.y.abs() < 1e-9
                && (b.x - 2.0).abs() < 1e-9
                && b.y.abs() < 1e-9
        })
        .expect("corner edge");
    run(
        &mut d,
        Command::Select {
            ids: vec![top, EntityId::Edge(edge)],
            mode: Default::default(),
        },
    );
    look(&mut d, [5.0, -4.5, 3.5], [1.0, 0.75, 0.4]);
    check("selection", &d);
}

#[test]
fn golden_open_group_fades_the_rest() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    let group = run(&mut d, Command::MakeGroup { ids: faces });
    let instance = *group
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("an instance");
    add_box(&mut d, [3.0, 0.5]);
    run(&mut d, Command::OpenInstance { id: instance });
    look(&mut d, [6.5, -5.5, 4.5], [2.5, 1.0, 0.4]);
    check("open_group", &d);
}

/// Every stored golden image belongs to a test here, so stale ones do not pile up.
#[test]
fn every_golden_image_has_a_test() {
    if updating() {
        return; // the other tests are writing the images
    }
    let names = [
        "box",
        "painted_box",
        "rotated_component",
        "guides",
        "soft_cylinder",
        "parallel_top",
        "selection",
        "open_group",
    ];
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut stored: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter_map(|n| n.strip_suffix(".png").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    stored.sort();
    let mut expected: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(stored, expected, "golden images in {}", dir.display());
}
