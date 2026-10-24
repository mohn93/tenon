//! Material finish (metalness and roughness) rendered: smooth surfaces show highlights, metal
//! reflects the sky above and the ground below, nothing goes black or NaN, and a matte finish
//! is exactly the shading faces had before materials had one.

use std::path::Path;

use glam::DVec3;
use kernel::EntityId;
use render::Snapshotter;
use session::{Command, Document, ImportAs, MaterialId, UpAxis};

use super::golden::{add_material, check, doc, look, run, two_spheres_obj};

const W: u32 = 160;
const H: u32 = 120;

/// The two spheres of the smooth-sphere golden, both softened, the left one (x < 0) painted
/// `left` and the right one `right`; the camera looks at them from the front.
fn spheres() -> Document {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("finish");
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
    d
}

/// Softens every face and paints the left sphere `left`, the right one `right`.
fn finish_spheres(d: &mut Document, left: MaterialId, right: MaterialId) {
    let model = d.model();
    let side = |left_side: bool| -> Vec<EntityId> {
        model
            .faces()
            .filter(|(_, f)| {
                f.outer
                    .iter()
                    .all(|v| model.position(*v).is_some_and(|p| (p.x < 0.0) == left_side))
            })
            .map(|(id, _)| EntityId::Face(id))
            .collect()
    };
    let (l, r) = (side(true), side(false));
    let all: Vec<EntityId> = l.iter().chain(&r).copied().collect();
    run(
        d,
        Command::SoftenByAngle {
            ids: all,
            max_angle_deg: 30.0,
        },
    );
    for (ids, material) in [(l, left), (r, right)] {
        run(
            d,
            Command::Paint {
                ids,
                material: Some(material),
                side: Default::default(),
            },
        );
    }
    run(
        d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
            all: false,
        },
    );
    look(d, [0.0, -6.0, 3.0], [0.0, 0.0, 1.0]);
}

fn set_finish(d: &mut Document, material: MaterialId, metalness: f32, roughness: f32) {
    run(
        d,
        Command::ChangeMaterial {
            material,
            name: None,
            color: None,
            opacity: None,
            texture: None,
            emission: None,
            metalness: Some(metalness),
            roughness: Some(roughness),
        },
    );
}

/// A doc with a gray sphere on the left (finish `left`: metalness and roughness) and one on the
/// right (`right`), rendered; None without a GPU.
fn render_spheres(left: (f32, f32), right: (f32, f32)) -> Option<(Document, Vec<u8>)> {
    let mut d = spheres();
    let a = add_material(&mut d, "Left", [150, 150, 150]);
    let b = add_material(&mut d, "Right", [150, 150, 150]);
    set_finish(&mut d, a, left.0, left.1);
    set_finish(&mut d, b, right.0, right.1);
    finish_spheres(&mut d, a, b);
    let px = capture(&d)?;
    Some((d, px))
}

fn capture(d: &Document) -> Option<Vec<u8>> {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    snap.set_model_style(d.chosen_style());
    snap.capture(
        d.scene(),
        d.context(),
        d.selection(),
        &d.camera,
        d.lighting,
        W,
        H,
    )
}

fn at(px: &[u8], d: &Document, p: DVec3) -> [i32; 3] {
    let s = d.camera.project(p).expect("on screen");
    let i = ((s.y as u32 * W + s.x as u32) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]].map(i32::from)
}

fn sum(c: [i32; 3]) -> i32 {
    c.iter().sum()
}

#[test]
fn a_smooth_sphere_has_a_highlight_a_matte_one_does_not() {
    // Left: plastic (smooth, not metal); right: the default matte finish.
    let Some((d, px)) = render_spheres((0.0, 0.2), (0.0, 1.0)) else {
        return;
    };
    // The headlight's highlight is where the surface faces the eye.
    let facing_eye = |center: DVec3| center + (d.camera.eye - center).normalize();
    let plastic = at(&px, &d, facing_eye(DVec3::new(-1.2, 0.0, 1.0)));
    let matte = at(&px, &d, facing_eye(DVec3::new(1.2, 0.0, 1.0)));
    assert!(
        sum(plastic) > sum(matte) + 60,
        "plastic {plastic:?}, matte {matte:?}"
    );
}

#[test]
fn a_metal_sphere_reflects_the_sky_above_and_the_ground_below() {
    let Some((d, px)) = render_spheres((1.0, 0.1), (0.0, 1.0)) else {
        return;
    };
    let top = at(&px, &d, DVec3::new(-1.2, -0.3, 1.95));
    let bottom = at(&px, &d, DVec3::new(-1.2, -0.3, 0.05));
    assert!(top[2] > top[0] + 15, "top {top:?}: bluish sky");
    assert!(bottom[0] >= bottom[2], "bottom {bottom:?}: warm ground");
    assert!(
        sum(top) > sum(bottom),
        "sky {top:?} brighter than ground {bottom:?}"
    );
}

#[test]
fn a_mirror_metal_has_no_black_or_broken_pixels_inside_its_silhouette() {
    // Roughness 0: the sharpest highlight and the strongest reflection, down to grazing angles.
    let Some((d, px)) = render_spheres((1.0, 0.0), (0.0, 0.0)) else {
        return;
    };
    for center in [-1.2, 1.2] {
        let c = d.camera.project(DVec3::new(center, -1.0, 1.0)).unwrap();
        let edge = d
            .camera
            .project(DVec3::new(center + 1.0, 0.0, 1.0))
            .unwrap();
        let radius = (edge - c).length() * 0.97;
        let mut checked = 0;
        for y in 0..H {
            for x in 0..W {
                let (dx, dy) = (f64::from(x) - c.x, f64::from(y) - c.y);
                if (dx * dx + dy * dy).sqrt() > radius {
                    continue;
                }
                let i = ((y * W + x) * 4) as usize;
                let p = [px[i], px[i + 1], px[i + 2]].map(i32::from);
                assert!(sum(p) > 45, "sphere at {center}: pixel ({x}, {y}) is {p:?}");
                checked += 1;
            }
        }
        assert!(checked > 500, "{checked} pixels checked");
    }
}

#[test]
fn a_matte_finish_is_the_shading_faces_had_before() {
    // The default finish and an explicit metalness 0 and roughness 1 give the same pixels.
    let Some((_, with)) = render_spheres((0.0, 1.0), (0.0, 1.0)) else {
        return;
    };
    let mut plain = spheres();
    let a = add_material(&mut plain, "Left", [150, 150, 150]);
    let b = add_material(&mut plain, "Right", [150, 150, 150]);
    finish_spheres(&mut plain, a, b);
    let Some(without) = capture(&plain) else {
        return;
    };
    assert_eq!(with, without);
}

/// A brushed-looking steel sphere beside a red plastic one, in the default lighting.
#[test]
fn golden_finish_spheres() {
    let mut d = spheres();
    let steel = add_material(&mut d, "Steel", [200, 205, 215]);
    let plastic = add_material(&mut d, "Plastic", [200, 30, 30]);
    set_finish(&mut d, steel, 1.0, 0.3);
    set_finish(&mut d, plastic, 0.0, 0.25);
    finish_spheres(&mut d, steel, plastic);
    check("finish_spheres", &d);
}
