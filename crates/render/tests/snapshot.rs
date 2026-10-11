use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::Model;
use render::Snapshotter;
use render::grid::BACKGROUND;
use session::Camera;

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
        .capture(&ground_square(), &BTreeSet::new(), &camera, w, h)
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
            .capture(&Model::new(), &BTreeSet::new(), &camera, w, h)
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
