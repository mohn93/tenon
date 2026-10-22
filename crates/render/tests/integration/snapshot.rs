use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{Model, Scene};
use render::Snapshotter;
use render::grid::{BACKGROUND, NIGHT_BACKGROUND};
use session::{Camera, Lighting};

fn ground_square() -> Model {
    let mut m = Model::new();
    let c = [(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        m.add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    m
}

#[test]
fn capture_returns_tightly_packed_rows_for_any_width() {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // 100 * 4 = 400 bytes per row: not a multiple of 256, so padding must be stripped.
    let (w, h) = (100, 60);
    // A level camera: the top row sees only sky, the ground square is below the center.
    let mut camera = Camera::new(DVec2::new(10.0, 10.0));
    camera.eye = DVec3::new(0.0, -6.0, 1.5);
    camera.target = DVec3::new(0.0, 0.0, 1.5);
    let px = snap
        .capture(
            &Scene::from_root(ground_square()),
            &[],
            &BTreeSet::new(),
            &camera,
            Lighting::Day,
            w,
            h,
        )
        .expect("capture");
    assert_eq!(px.len(), (w * h * 4) as usize);

    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let sky = BACKGROUND.map(|c| (c * 255.0).round() as u8);
    let sky = [sky[0], sky[1], sky[2]];
    assert_eq!(
        at(w - 1, 0),
        sky,
        "last column of the first row must be sky"
    );
    // The capture uses the image size as the viewport, so projecting with it finds the face.
    camera.viewport = DVec2::new(w as f64, h as f64);
    let face = camera.project(DVec3::new(0.3, 0.3, 0.0)).unwrap();
    assert_ne!(
        at(face.x as u32, face.y as u32),
        sky,
        "face pixel must not be sky"
    );
}

#[test]
fn capture_handles_padding_edge_widths() {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut camera = Camera::new(DVec2::new(10.0, 10.0));
    camera.eye = DVec3::new(0.0, -6.0, 1.5);
    camera.target = DVec3::new(0.0, 0.0, 1.5);
    let sky = BACKGROUND.map(|c| (c * 255.0).round() as u8);
    let sky = [sky[0], sky[1], sky[2]];
    let h = 8;
    // Widths around the 64-pixel (256-byte) row alignment boundary.
    for w in [1u32, 63, 64, 65] {
        let px = snap
            .capture(
                &Scene::new(),
                &[],
                &BTreeSet::new(),
                &camera,
                Lighting::Day,
                w,
                h,
            )
            .expect("capture");
        assert_eq!(px.len(), (w * h * 4) as usize, "width {w}");
        // At width 1 the only column is the image center, where the renderer draws the
        // axis lines, so the sky check only applies to wider images.
        if w > 1 {
            let i = ((w - 1) * 4) as usize;
            assert_eq!(&px[i..i + 3], &sky, "top-right pixel at width {w}");
        }
    }
}

/// A 20 x 20 m ground face with a 0.5 m glowing white square hanging 1 m above (-5, 0), its
/// both sides painted with a material glowing at 5.
fn lamp_over_ground() -> Scene {
    let mut m = Model::new();
    let ground = [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)];
    m.add_polygon(&ground.map(|(x, y)| DVec3::new(x, y, 0.0)))
        .unwrap();
    let lamp = [(-5.25, -0.25), (-4.75, -0.25), (-4.75, 0.25), (-5.25, 0.25)];
    m.add_polygon(&lamp.map(|(x, y)| DVec3::new(x, y, 1.0)))
        .unwrap();
    let lamp_face = m
        .faces()
        .find(|(_, f)| m.position(f.outer[0]).unwrap().z == 1.0)
        .unwrap()
        .0;
    let mut s = Scene::from_root(m);
    let (glow, _) = s
        .add_material_from(kernel::Material {
            name: "Lamp".into(),
            color: [255, 255, 255],
            opacity: 1.0,
            texture: None,
            emission: 5.0,
        })
        .unwrap();
    s.paint(
        kernel::MeshId::Root,
        &[lamp_face],
        &[],
        kernel::Side::Both,
        Some(glow),
    )
    .unwrap();
    s
}

#[test]
fn night_is_dark_except_near_glowing_faces_which_show_unshaded() {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let (w, h) = (160, 120);
    let mut camera = Camera::new(DVec2::new(w as f64, h as f64));
    camera.eye = DVec3::new(0.0, -14.0, 12.0);
    camera.target = DVec3::ZERO;
    let scene = lamp_over_ground();
    let shot = |snap: &mut Snapshotter, lighting| {
        snap.capture(&scene, &[], &BTreeSet::new(), &camera, lighting, w, h)
            .expect("capture")
    };
    let night = shot(&mut snap, Lighting::Night);
    let day = shot(&mut snap, Lighting::Day);
    let at = |px: &[u8], p: DVec3| {
        let s = camera.project(p).unwrap();
        let i = ((s.y as u32 * w + s.x as u32) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let gray = |c: [u8; 3]| c.iter().map(|v| u32::from(*v)).sum::<u32>() / 3;
    // Looking level, the sky is the night background.
    let mut level = camera.clone();
    level.eye = DVec3::new(0.0, -6.0, 1.5);
    level.target = DVec3::new(0.0, 0.0, 1.5);
    let sky = snap
        .capture(&scene, &[], &BTreeSet::new(), &level, Lighting::Night, w, h)
        .expect("capture");
    let background = NIGHT_BACKGROUND.map(|c| (c * 255.0).round() as u8);
    assert_eq!(&sky[..3], &background[..3]);
    // The lamp shows its own white, unshaded.
    assert_eq!(at(&night, DVec3::new(-5.0, 0.0, 1.0)), [255, 255, 255]);
    // The ground under the lamp is lit; far from it, it is dark, darker than by day.
    let near = gray(at(&night, DVec3::new(-5.0, -1.0, 0.0)));
    let far = gray(at(&night, DVec3::new(6.0, -1.0, 0.0)));
    assert!(near > far + 60, "near {near}, far {far}");
    assert!(far < 60, "far {far}");
    assert!(gray(at(&day, DVec3::new(6.0, -1.0, 0.0))) > 180);
}

#[test]
fn print_mode_draws_white_paper_without_ground_grid_or_axes() {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let scene = Scene::from_root(Model::new());
    let mut camera = Camera::new(DVec2::new(64.0, 64.0));
    camera.eye = DVec3::new(0.0, -6.0, 3.0);
    camera.target = DVec3::ZERO;
    let capture = |snap: &mut Snapshotter, lighting| {
        snap.prepare(&scene, &[], &BTreeSet::new());
        snap.capture_prepared(&camera, lighting, 64, 64)
            .expect("capture")
    };
    let normal = capture(&mut snap, Lighting::Day);
    assert!(
        normal.chunks(4).any(|p| p[..3] != [255, 255, 255]),
        "the normal view has a background and axes"
    );
    snap.set_print_mode(true);
    for lighting in [Lighting::Day, Lighting::Night] {
        let printed = capture(&mut snap, lighting);
        assert!(
            printed.chunks(4).all(|p| p == [255, 255, 255, 255]),
            "{lighting:?}: pure white"
        );
    }
    snap.set_print_mode(false);
    assert_eq!(capture(&mut snap, Lighting::Day), normal, "normal again");
}
