//! Golden-image tests: canonical scenes rendered headless at a fixed size and compared with
//! the PNGs in `tests/golden/`, within a tolerance for GPU and driver differences.
//!
//! `UPDATE_GOLDEN=1 cargo test -p render --test golden` rewrites the stored images from this
//! machine's renders (check them by eye before committing). On a mismatch the actual image
//! and a diff image are written under the target's tmp directory and their paths reported.

use std::path::{Path, PathBuf};

use kernel::EntityId;
use render::Snapshotter;
use session::{Command, Document, ImportAs, MaterialId, Projection, Side, StandardView, UpAxis};

pub(crate) const WIDTH: u32 = 160;
pub(crate) const HEIGHT: u32 = 120;

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

pub(crate) fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
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

/// Renders `doc` with its camera, lighting, selection and open context, and checks the image
/// against `tests/golden/<name>.png` (or rewrites that file with `UPDATE_GOLDEN=1`). Skips
/// without a GPU adapter, and on a software adapter (Mesa lavapipe, WARP) with a printed note:
/// the goldens come from a real GPU and stay strict there; a software rasterizer differs in
/// edges, antialiasing and shadow filtering by more than any tolerance worth having.
fn check(name: &str, doc: &Document) {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    if snap.is_software_adapter() {
        // Neither compared nor rewritten: a software render must not become the reference.
        eprintln!("{name}: software GPU adapter, skipping golden comparison");
        return;
    }
    snap.set_section_display(doc.section_display);
    snap.set_sun(render::SunLight::of(doc));
    snap.set_model_style(doc.chosen_style());
    let actual = snap
        .capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            &doc.camera,
            doc.lighting,
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

pub(crate) fn run(doc: &mut Document, command: Command) -> session::Outcome {
    doc.execute(command.clone())
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
}

pub(crate) fn doc() -> Document {
    Document::new(glam::DVec2::new(WIDTH as f64, HEIGHT as f64))
}

pub(crate) fn look(doc: &mut Document, eye: [f64; 3], target: [f64; 3]) {
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

pub(crate) fn created_face(outcome: &session::Outcome) -> EntityId {
    *outcome
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Face(_)))
        .expect("a face was made")
}

/// A 2 x 1.5 x 1 m box with a corner at the origin; returns the top face.
pub(crate) fn add_box(doc: &mut Document, origin: [f64; 2]) -> EntityId {
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
pub(crate) fn raise(doc: &mut Document, face: EntityId, height: f64) {
    let EntityId::Face(id) = face else {
        panic!("{face} is not a face")
    };
    let up = doc.model().face(id).expect("face").normal.z > 0.0;
    let distance = if up { height } else { -height };
    run(doc, Command::PushPull { face, distance });
}

/// The face of the open context whose vertices all lie at height `z`.
pub(crate) fn top_face(doc: &Document, z: f64) -> EntityId {
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

pub(crate) fn add_material(doc: &mut Document, name: &str, color: [u8; 3]) -> MaterialId {
    let outcome = run(
        doc,
        Command::AddMaterial {
            name: Some(name.into()),
            color: Some(color),
            opacity: None,
            texture: None,
            emission: None,
            existing: Default::default(),
        },
    );
    outcome.changes.materials[0]
}

pub(crate) fn all_faces(doc: &Document) -> Vec<EntityId> {
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
    // The floor (z = 0) keeps its unpainted back, so the default back color shows inside.
    let floor = top_face(&d, 0.0);
    let walls: Vec<EntityId> = faces.iter().copied().filter(|f| *f != floor).collect();
    run(
        &mut d,
        Command::Paint {
            ids: faces.clone(),
            material: Some(brick),
            side: Side::Front,
        },
    );
    run(
        &mut d,
        Command::Paint {
            ids: walls,
            material: Some(leaf),
            side: Side::Back,
        },
    );
    // From above, into the open box: outside walls are brick, the inside walls leaf green,
    // and the floor shows the default back color.
    look(&mut d, [4.0, -3.5, 4.5], [1.0, 0.75, 0.3]);
    check("painted_box", &d);
}

#[test]
fn golden_glass_pane_in_front_of_a_box() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let brick = add_material(&mut d, "Brick", [190, 70, 50]);
    let faces = all_faces(&d);
    run(
        &mut d,
        Command::Paint {
            ids: faces,
            material: Some(brick),
            side: Side::Front,
        },
    );
    // A pale blue pane, see-through on both sides, standing between the eye and the box; its
    // edges are the black frame.
    let pane = run(
        &mut d,
        Command::AddRectangle {
            corner: [-0.5, -1.0, 0.0],
            opposite: [2.5, -1.0, 1.8],
            normal: None,
        },
    );
    let pane = created_face(&pane);
    let glass = run(
        &mut d,
        Command::AddMaterial {
            name: Some("Glass".into()),
            color: Some([170, 210, 235]),
            opacity: Some(0.35),
            texture: None,
            emission: None,
            existing: Default::default(),
        },
    )
    .changes
    .materials[0];
    run(
        &mut d,
        Command::Paint {
            ids: vec![pane],
            material: Some(glass),
            side: Side::Both,
        },
    );
    look(&mut d, [1.8, -6.0, 2.6], [1.0, 0.75, 0.5]);
    check("glass", &d);
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
            all: false,
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
    for position in [[-1.5, 1.0, 0.8], [0.0, -1.0, 1.2]] {
        run(&mut d, Command::AddGuidePoint { position });
    }
    look(&mut d, [2.5, -3.5, 2.8], [0.0, 0.0, 0.3]);
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

/// OBJ text for two UV spheres of radius 1 (16 sides, 8 bands), centered at x = -1.2 and 1.2
/// on z = 1, in meters with Z up.
fn two_spheres_obj() -> String {
    let (sides, bands) = (16, 8);
    let mut out = String::new();
    let mut count = 0;
    for cx in [-1.2, 1.2] {
        let first = count + 1;
        out.push_str(&format!("v {cx} 0 2\n"));
        for b in 1..bands {
            let polar = std::f64::consts::PI * b as f64 / bands as f64;
            for k in 0..sides {
                let a = std::f64::consts::TAU * k as f64 / sides as f64;
                let (x, y, z) = (polar.sin() * a.cos(), polar.sin() * a.sin(), polar.cos());
                out.push_str(&format!("v {} {} {}\n", cx + x, y, 1.0 + z));
            }
        }
        out.push_str(&format!("v {cx} 0 0\n"));
        let ring = |b: usize, k: usize| first + 1 + (b - 1) * sides + k % sides;
        let bottom = first + 1 + (bands - 1) * sides;
        for k in 0..sides {
            out.push_str(&format!("f {first} {} {}\n", ring(1, k), ring(1, k + 1)));
            for b in 1..bands - 1 {
                out.push_str(&format!(
                    "f {} {} {} {}\n",
                    ring(b, k),
                    ring(b + 1, k),
                    ring(b + 1, k + 1),
                    ring(b, k + 1)
                ));
            }
            out.push_str(&format!(
                "f {} {bottom} {}\n",
                ring(bands - 1, k),
                ring(bands - 1, k + 1)
            ));
        }
        count = bottom;
    }
    out
}

#[test]
fn golden_smooth_sphere_beside_a_faceted_one() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden");
    std::fs::create_dir_all(&dir).expect("create folder");
    let path = dir.join(format!("spheres_{}.obj", std::process::id()));
    std::fs::write(&path, two_spheres_obj()).expect("write spheres");
    let mut d = doc();
    run(
        &mut d,
        Command::Import {
            library_roots: None,
            path: path.to_string_lossy().into_owned(),
            format: None,
            place: ImportAs::Context,
            scale: None,
            up: Some(UpAxis::Z),
            merge_coplanar: Some(false),
        },
    );
    let _ = std::fs::remove_file(&path);
    // What an import brings in becomes the selection.
    run(
        &mut d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
            all: false,
        },
    );
    // Soften the sphere on the right only.
    let model = d.model();
    let right: Vec<EntityId> = model
        .faces()
        .filter(|(_, f)| {
            f.outer
                .iter()
                .all(|v| model.position(*v).is_some_and(|p| p.x > 0.0))
        })
        .map(|(id, _)| EntityId::Face(id))
        .collect();
    assert_eq!(right.len(), 16 * 8);
    let out = run(
        &mut d,
        Command::SoftenByAngle {
            ids: right,
            max_angle_deg: 30.0,
        },
    );
    assert_eq!(out.soften.map(|r| r.changed), Some(16 * 15));
    look(&mut d, [0.0, -6.0, 3.0], [0.0, 0.0, 1.0]);
    check("smooth_sphere", &d);
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
    run(&mut d, Command::ZoomExtents {});
    check("parallel_top", &d);
}

#[test]
fn golden_selection_highlight() {
    let mut d = doc();
    let top = add_box(&mut d, [0.0, 0.0]);
    // The top face and its four boundary edges.
    let model = d.model();
    let mut ids = vec![top];
    ids.extend(
        model
            .edges()
            .filter(|(_, e)| {
                e.v.iter()
                    .all(|v| model.position(*v).is_some_and(|p| (p.z - 1.0).abs() < 1e-9))
            })
            .map(|(id, _)| EntityId::Edge(id)),
    );
    assert_eq!(ids.len(), 5, "top face and four edges");
    run(
        &mut d,
        Command::Select {
            ids,
            mode: Default::default(),
            all: false,
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
    let group = run(
        &mut d,
        Command::MakeGroup {
            ids: faces,
            name: None,
        },
    );
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

#[test]
fn golden_section_cut_through_a_box() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: None,
        },
    );
    // From the cut-away side, so the filled cut shows.
    look(&mut d, [-3.5, -3.5, 3.0], [1.0, 0.75, 0.4]);
    check("section_cut", &d);
}

#[test]
fn golden_section_plane_in_a_group_cuts_only_the_group() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    let group = run(
        &mut d,
        Command::MakeGroup {
            ids: faces,
            name: None,
        },
    );
    let instance = *group
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("an instance");
    add_box(&mut d, [3.0, 0.5]);
    run(&mut d, Command::OpenInstance { id: instance });
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: None,
        },
    );
    run(&mut d, Command::CloseInstance {});
    look(&mut d, [-3.0, -5.0, 4.0], [2.5, 1.0, 0.4]);
    check("section_group", &d);
}

/// A 16 x 16 pixel PNG in four quadrants, red top left, yellow top right, green bottom left and
/// blue bottom right, with a white line along its top and left edges; returns its path.
pub(crate) fn quadrants_png() -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("golden-textures-{}", std::process::id()))
        .join("quadrants.png");
    let mut rgba = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            let color = match (x < 8, y < 8) {
                _ if x == 0 || y == 0 => [255, 255, 255],
                (true, true) => [220, 40, 40],
                (false, true) => [235, 200, 40],
                (true, false) => [40, 170, 70],
                (false, false) => [40, 80, 220],
            };
            rgba.extend(color);
            rgba.push(255);
        }
    }
    write_png(&path, &rgba, 16, 16);
    path
}

#[test]
fn golden_textured_box_and_ground() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    // A ground square beside the box, to show the texture on a level face.
    run(
        &mut d,
        Command::AddRectangle {
            corner: [2.5, 0.0, 0.0],
            opposite: [4.5, 2.0, 0.0],
            normal: None,
        },
    );
    let tiles = run(
        &mut d,
        Command::AddMaterial {
            name: Some("Tiles".into()),
            color: None,
            opacity: None,
            texture: Some(session::TextureSource {
                path: Some(quadrants_png().display().to_string()),
                width: Some(1.0),
                height: Some(1.0),
            }),
            emission: None,
            existing: Default::default(),
        },
    )
    .changes
    .materials[0];
    let faces = all_faces(&d);
    run(
        &mut d,
        Command::Paint {
            ids: faces,
            material: Some(tiles),
            side: Side::Both,
        },
    );
    // Each wall shows whole 1 m copies upright (white lines along their tops and left sides);
    // the top and the ground square show them with north up.
    look(&mut d, [5.5, -5.0, 4.5], [2.0, 0.9, 0.3]);
    check("textured", &d);
}

/// A free-standing 1.5 x 1.5 m wall at `x` in the plane y = 0, turned so its front faces -y
/// (toward the camera of `golden_positioned_textures`) or +y; returns its face.
fn wall(doc: &mut Document, x: f64, front_to_camera: bool) -> EntityId {
    let made = run(
        doc,
        Command::AddRectangle {
            corner: [x, 0.0, 0.0],
            opposite: [x + 1.5, 0.0, 1.5],
            normal: None,
        },
    );
    let face = created_face(&made);
    let EntityId::Face(id) = face else {
        unreachable!()
    };
    let toward = doc.model().face(id).expect("face").normal.y < 0.0;
    if toward != front_to_camera {
        // Half a turn about its middle shows the other side.
        run(
            doc,
            Command::Rotate {
                ids: vec![face],
                center: [x + 0.75, 0.0, 0.0],
                axis: None,
                angle: 180.0,
                copies: None,
            },
        );
    }
    face
}

#[test]
fn golden_positioned_textures() {
    let mut d = doc();
    // Left: a wall's front with the texture turned a quarter counter-clockwise and halved.
    // Right: a wall's back with the texture moved a quarter along: it reads as seen from
    // behind, not mirrored (white lines along the tops and left sides of the copies).
    let front = wall(&mut d, 0.0, true);
    let back = wall(&mut d, 2.0, false);
    let tiles = run(
        &mut d,
        Command::AddMaterial {
            name: Some("Tiles".into()),
            color: None,
            opacity: None,
            texture: Some(session::TextureSource {
                path: Some(quadrants_png().display().to_string()),
                width: Some(1.0),
                height: Some(1.0),
            }),
            emission: None,
            existing: Default::default(),
        },
    )
    .changes
    .materials[0];
    run(
        &mut d,
        Command::Paint {
            ids: vec![front, back],
            material: Some(tiles),
            side: Side::Both,
        },
    );
    run(
        &mut d,
        Command::PositionTexture {
            ids: vec![front],
            offset: None,
            rotation_deg: Some(90.0),
            rotate_by: None,
            scale: Some([0.5, 0.5]),
            reset: false,
        },
    );
    run(
        &mut d,
        Command::PositionTexture {
            ids: vec![back],
            offset: Some([0.25, 0.0]),
            rotation_deg: None,
            rotate_by: None,
            scale: None,
            reset: false,
        },
    );
    look(&mut d, [1.75, -4.5, 0.75], [1.75, 0.0, 0.75]);
    check("positioned", &d);
}

/// A material glowing at `emission`.
pub(crate) fn add_lamp_material(
    doc: &mut Document,
    name: &str,
    color: [u8; 3],
    emission: f32,
) -> MaterialId {
    let outcome = run(
        doc,
        Command::AddMaterial {
            name: Some(name.into()),
            color: Some(color),
            opacity: None,
            texture: None,
            emission: Some(emission),
            existing: Default::default(),
        },
    );
    outcome.changes.materials[0]
}

/// The instance of the open context an outcome made (a copied group's insides are new too).
fn created_instance(doc: &Document, outcome: &session::Outcome) -> EntityId {
    *outcome
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(i) if doc.model().instance(*i).is_some()))
        .expect("an instance was made")
}

#[test]
fn golden_night_lit_by_two_lamps() {
    let mut d = doc();
    // A floor with a box on it.
    run(
        &mut d,
        Command::AddRectangle {
            corner: [-4.0, -3.0, 0.0],
            opposite: [6.0, 4.0, 0.0],
            normal: None,
        },
    );
    add_box(&mut d, [0.0, 0.0]);
    // A lamp: a level square component inside a "Fixture" group, hanging left of the box.
    let shade = run(
        &mut d,
        Command::AddRectangle {
            corner: [-1.8, -1.3, 2.2],
            opposite: [-1.2, -0.7, 2.2],
            normal: None,
        },
    );
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: vec![created_face(&shade)],
            name: "Lamp".into(),
        },
    );
    let lamp = created_instance(&d, &made);
    let grouped = run(
        &mut d,
        Command::MakeGroup {
            ids: vec![lamp],
            name: None,
        },
    );
    let fixture = created_instance(&d, &grouped);
    // A copy right of the box.
    let copied = run(
        &mut d,
        Command::Copy {
            ids: vec![fixture],
            offset: [5.0, 0.0, 0.0],
            count: None,
        },
    );
    let copy = created_instance(&d, &copied);
    // Warm on the left, cool on the right: the fixtures' paint reaches the lamps' faces.
    let warm = add_lamp_material(&mut d, "Warm", [255, 200, 120], 6.0);
    let cool = add_lamp_material(&mut d, "Cool", [120, 170, 255], 6.0);
    for (instance, material) in [(fixture, warm), (copy, cool)] {
        run(
            &mut d,
            Command::Paint {
                ids: vec![instance],
                material: Some(material),
                side: Side::Front,
            },
        );
    }
    run(
        &mut d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
            all: false,
        },
    );
    run(
        &mut d,
        Command::SetLighting {
            mode: session::Lighting::Night,
        },
    );
    look(&mut d, [1.0, -7.5, 4.5], [1.0, 0.75, 0.6]);
    check("night", &d);
}

#[test]
fn golden_wall_shading_the_floor_from_a_lamp() {
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [-5.0, -4.0, 0.0],
            opposite: [7.0, 4.0, 0.0],
            normal: None,
        },
    );
    // A wall 4 m long and 2 m tall across the floor; the lamp hangs left of it, lower than
    // its top, so the floor right of it is dark.
    let base = run(
        &mut d,
        Command::AddRectangle {
            corner: [0.0, -2.0, 0.0],
            opposite: [0.2, 2.0, 0.0],
            normal: None,
        },
    );
    raise(&mut d, created_face(&base), 2.0);
    let shade = run(
        &mut d,
        Command::AddRectangle {
            corner: [-1.8, -0.3, 1.6],
            opposite: [-1.2, 0.3, 1.6],
            normal: None,
        },
    );
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: vec![created_face(&shade)],
            name: "Lamp".into(),
        },
    );
    let lamp = created_instance(&d, &made);
    let warm = add_lamp_material(&mut d, "Warm", [255, 210, 150], 6.0);
    run(
        &mut d,
        Command::Paint {
            ids: vec![lamp],
            material: Some(warm),
            side: Side::Front,
        },
    );
    run(
        &mut d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
            all: false,
        },
    );
    run(
        &mut d,
        Command::SetLighting {
            mode: session::Lighting::Night,
        },
    );
    look(&mut d, [-5.0, -8.0, 7.0], [1.0, 0.0, 0.0]);
    check("shadow", &d);
}

#[test]
fn golden_sun_shadow() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::SetShadows {
            on: Some(true),
            date: None,
            time: None,
            light: None,
            dark: None,
            on_ground: None,
        },
    );
    look(&mut d, [4.5, -3.5, 4.0], [1.0, 1.0, 0.3]);
    check("sun_shadow", &d);
}

/// Adds a style with `patch`, chooses it and returns nothing else: the document draws with it.
fn choose_style(doc: &mut Document, patch: session::StylePatch) {
    let id = run(
        doc,
        Command::AddStyle {
            name: None,
            from: None,
        },
    )
    .changes
    .styles[0];
    run(
        doc,
        Command::ChangeStyle {
            style: id,
            name: None,
            settings: Some(patch),
        },
    );
    run(doc, Command::SetStyle { style: Some(id) });
}

/// A row of boxes receding from the camera, under a sky, a translucent ground and fog in the
/// background color: the far boxes melt into the haze.
#[test]
fn golden_style_sky_ground_fog() {
    let mut d = doc();
    for k in 0..5 {
        add_box(&mut d, [0.0, 4.0 * k as f64]);
    }
    choose_style(
        &mut d,
        session::StylePatch {
            background: Some([225, 232, 238]),
            sky: Some(Some([70, 130, 220])),
            ground: Some(Some(kernel::Ground {
                color: [120, 150, 90],
                opacity: 0.6,
                from_below: false,
            })),
            fog: Some(Some(kernel::Fog {
                start: 6.0,
                end: 22.0,
                color: kernel::FogColor::Background,
            })),
            ..Default::default()
        },
    );
    look(&mut d, [3.5, -5.0, 1.6], [0.8, 8.0, 1.2]);
    check("style_sky_ground_fog", &d);
}

/// A box whose edges take the axis colors: red along x, green along y, blue along z, and the
/// fixed color (orange) for a diagonal.
#[test]
fn golden_style_edges_by_axis() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::AddEdge {
            start: [3.0, 0.0, 0.0],
            end: [4.5, 1.5, 0.0],
        },
    );
    choose_style(
        &mut d,
        session::StylePatch {
            edges: Some(session::EdgesPatch {
                color_mode: Some(kernel::EdgeColorMode::ByAxis),
                color: Some([230, 120, 20]),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    look(&mut d, [5.0, -4.5, 3.5], [1.0, 0.75, 0.4]);
    check("style_edges_by_axis", &d);
}

/// A box and a cylinder with profiles, sketchy edges and endpoints: the silhouettes are heavy,
/// the cylinder's soft edges show only where it turns away, and each edge wanders and overshoots
/// its corners with a short mark at both ends.
#[test]
fn golden_style_profiles_sketchy() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let circle = run(
        &mut d,
        Command::AddCircle {
            center: [4.0, 0.7, 0.0],
            radius: 0.8,
            normal: None,
            segments: Some(16),
        },
    );
    raise(&mut d, created_face(&circle), 1.4);
    choose_style(
        &mut d,
        session::StylePatch {
            edges: Some(session::EdgesPatch {
                profiles: Some(Some(4.0)),
                sketchy: Some(Some(kernel::Sketchy {
                    jitter: 2.0,
                    extension: 10.0,
                })),
                endpoints: Some(Some(8.0)),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    look(&mut d, [2.6, -7.5, 3.6], [2.4, 0.7, 0.6]);
    check("style_profiles_sketchy", &d);
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
        "smooth_sphere",
        "parallel_top",
        "selection",
        "open_group",
        "glass",
        "textured",
        "night",
        "positioned",
        "shadow",
        "sun_shadow",
        "section_cut",
        "section_group",
        "style_sky_ground_fog",
        "style_edges_by_axis",
        "style_profiles_sketchy",
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
