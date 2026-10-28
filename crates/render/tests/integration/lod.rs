//! Level of detail: chunks whose detail is about a pixel are drawn simplified, and the picture
//! is the one of drawing them whole.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EdgeStyle, EntityId, Model, Scene, StyleSettings};
use render::{HoverLook, Renderer, Viewport};
use session::Camera;

use crate::headless::{H, Offscreen, W};

/// Squares along a side of the plane, and how long a square is: a fine mesh of one flat color.
const SIDE: usize = 30;
const STEP: f64 = 0.5;

/// A flat plane of `SIDE` x `SIDE` squares of `STEP` m, the faces' ids.
fn plane() -> (Scene, Vec<EntityId>) {
    let mut m = Model::new();
    for i in 0..SIDE {
        for j in 0..SIDE {
            let (x, y) = (i as f64 * STEP, j as f64 * STEP);
            let p = |x: f64, y: f64| DVec3::new(x, y, 0.0);
            m.add_polygon(&[
                p(x, y),
                p(x + STEP, y),
                p(x + STEP, y + STEP),
                p(x, y + STEP),
            ])
            .unwrap();
        }
    }
    let faces = m.faces().map(|(id, _)| EntityId::Face(id)).collect();
    (Scene::from_root(m), faces)
}

/// The look without edges (they are never simplified).
fn style() -> StyleSettings {
    StyleSettings {
        edges: EdgeStyle {
            show_edges: false,
            ..EdgeStyle::default()
        },
        background: [90, 120, 200],
        ..StyleSettings::default()
    }
}

fn renderer(screen: &Offscreen, scene: &Scene) -> Renderer {
    let mut r = screen.renderer();
    // Small chunks, so a model this size has many.
    r.set_culling_from(100);
    r.set_model_style(Some(&style()));
    r.set_scene(scene, &[], &BTreeSet::new());
    r
}

/// Looking down at the plane from `height` m.
fn camera(height: f64) -> Camera {
    let mut cam = Camera::new(DVec2::new(W as f64, H as f64));
    let mid = SIDE as f64 * STEP / 2.0;
    cam.eye = DVec3::new(mid, mid - 0.2 * height, height);
    cam.target = DVec3::new(mid, mid, 0.0);
    cam
}

const FAR: f64 = 400.0;
const NEAR: f64 = 12.0;

/// How many pixels differ by more than `levels` in a channel.
fn differing(a: &[u8], b: &[u8], levels: u8) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..3).any(|i| p[i].abs_diff(q[i]) > levels))
        .count()
}

#[test]
fn a_far_view_draws_fewer_triangles_and_the_picture_is_the_same() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let mut r = renderer(&screen, &scene);
    let cam = camera(FAR);
    let full = Viewport::full((W, H));
    let (kept, all) = r.visible_triangles(&cam).unwrap();
    let (drawn, _) = r.drawn_triangles(&cam).unwrap();
    assert_eq!(all, (2 * SIDE * SIDE) as u32);
    assert!(kept > 0);
    assert!(drawn * 3 < kept, "drew {drawn} of {kept}");
    let simple = screen.draw(&mut r, &cam, full, &[]);
    r.set_lod(false);
    assert_eq!(r.drawn_triangles(&cam), Some((kept, all)));
    let whole = screen.draw(&mut r, &cam, full, &[]);
    let background = [90u8, 120, 200];
    let seen = simple
        .chunks(4)
        .filter(|p| p[..3] != background[..])
        .count();
    assert!(seen > 100, "the plane is in the picture");
    let off = differing(&simple, &whole, 6);
    assert!(
        off * 100 <= W as usize * H as usize,
        "{off} pixels differ by more than 6 levels"
    );
}

#[test]
fn a_near_view_draws_full_detail_and_the_picture_is_identical() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let mut r = renderer(&screen, &scene);
    let cam = camera(NEAR);
    let full = Viewport::full((W, H));
    let (kept, _) = r.visible_triangles(&cam).unwrap();
    assert_eq!(r.drawn_triangles(&cam).map(|d| d.0), Some(kept));
    let on = screen.draw(&mut r, &cam, full, &[]);
    r.set_lod(false);
    assert!(on == screen.draw(&mut r, &cam, full, &[]));
}

#[test]
fn selection_and_hover_light_a_chunk_that_would_be_simplified() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, faces) = plane();
    let cam = camera(FAR);
    let full = Viewport::full((W, H));
    let look = HoverLook {
        face: [1.0, 0.5, 0.0],
        face_mix: 0.8,
        edge: [1.0, 0.0, 0.0, 1.0],
    };
    let mut on = renderer(&screen, &scene);
    let mut off = renderer(&screen, &scene);
    off.set_lod(false);
    let plain = screen.draw(&mut on, &cam, full, &[]);
    let (base, _) = on.drawn_triangles(&cam).unwrap();

    // Faces spread over the plane, so the highlight covers pixels.
    let chosen: BTreeSet<EntityId> = faces.iter().copied().step_by(7).collect();
    on.set_selection(&chosen);
    off.set_selection(&chosen);
    let (more, _) = on.drawn_triangles(&cam).unwrap();
    assert!(more > base, "the chunks with a selection are drawn whole");
    let a = screen.draw(&mut on, &cam, full, &[]);
    let b = screen.draw(&mut off, &cam, full, &[]);
    assert!(differing(&a, &plain, 6) > 50, "the selection shows");
    assert!(differing(&a, &b, 6) * 100 <= W as usize * H as usize);

    on.set_selection(&BTreeSet::new());
    off.set_selection(&BTreeSet::new());
    let first = faces[faces.len() / 2];
    on.set_hover(Some((first, look)));
    off.set_hover(Some((first, look)));
    let (hovered, _) = on.drawn_triangles(&cam).unwrap();
    assert!(hovered > base, "the chunk with the hover is drawn whole");
    let a = screen.draw(&mut on, &cam, full, &[]);
    let b = screen.draw(&mut off, &cam, full, &[]);
    let d = differing(&a, &b, 6);
    assert!(d * 100 <= W as usize * H as usize, "hover: {d} differ");
    on.set_hover(None);
    assert_eq!(on.drawn_triangles(&cam).map(|d| d.0), Some(base));
}
