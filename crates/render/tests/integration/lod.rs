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
const STEP: f64 = 1.0;

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

/// A renderer of `scene`; with `lod`, one with the simplified copies made.
fn renderer(screen: &Offscreen, scene: &Scene, lod: bool) -> Renderer {
    let mut r = screen.renderer();
    // Small chunks, so a model this size has many.
    r.set_culling_from(100);
    r.set_model_style(Some(&style()));
    r.set_lod(lod);
    r.set_scene(scene, &[], &BTreeSet::new());
    if lod {
        r.wait_for_lod();
    }
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

const FAR: f64 = 700.0;
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
    let mut r = renderer(&screen, &scene, true);
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
    let mut r = renderer(&screen, &scene, true);
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
    let mut on = renderer(&screen, &scene, true);
    let mut off = renderer(&screen, &scene, false);
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

#[test]
fn a_renderer_that_was_not_asked_for_lod_draws_full_detail_however_far_the_view() {
    // What the snapshotter wraps: pictures made off screen are never simplified.
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let mut r = renderer(&screen, &scene, false);
    r.wait_for_lod();
    let cam = camera(FAR);
    let full = Viewport::full((W, H));
    for _ in 0..12 {
        screen.draw(&mut r, &cam, full, &[]);
    }
    assert!(r.lod_level_bytes().is_empty());
    let (kept, all) = r.visible_triangles(&cam).unwrap();
    assert_eq!(r.drawn_triangles(&cam), Some((kept, all)));
}

#[test]
fn an_idle_renderer_makes_the_copies_once_the_model_has_settled() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let mut r = screen.renderer();
    r.set_culling_from(100);
    r.set_model_style(Some(&style()));
    r.set_lod(true);
    r.set_scene(&scene, &[], &BTreeSet::new());
    // No frame is drawn and no camera asked: the copies still come, and the vertices they
    // are made from are let go.
    assert!(r.lod_busy());
    let started = std::time::Instant::now();
    let mut arrived = false;
    while r.lod_busy() {
        assert!(started.elapsed().as_secs() < 30, "the copies never came");
        arrived |= r.poll_lod();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(arrived, "poll_lod says when they arrive");
    assert!(
        started.elapsed().as_millis() >= 400,
        "not before it settled"
    );
    assert!(!r.lod_level_bytes().is_empty());
    let far = camera(FAR);
    let full = Viewport::full((W, H));
    let (kept, _) = r.visible_triangles(&far).unwrap();
    screen.draw(&mut r, &far, full, &[]);
    assert!(r.drawn_triangles(&far).unwrap().0 * 3 < kept);
    // Turning it off drops them.
    r.set_lod(false);
    assert!(r.lod_level_bytes().is_empty() && !r.lod_busy());
}

#[test]
fn a_level_over_the_buffer_limit_is_left_out() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let all = renderer(&screen, &scene, true).lod_level_bytes();
    assert!(all.len() > 1 && all.iter().all(|&b| b > 0), "{all:?}");
    let largest = *all.iter().max().unwrap();
    let mut r = screen.renderer();
    r.set_culling_from(100);
    r.set_model_style(Some(&style()));
    r.set_lod(true);
    r.set_lod_buffer_limit(largest - 1);
    r.set_scene(&scene, &[], &BTreeSet::new());
    r.wait_for_lod();
    let some: Vec<u64> = all
        .iter()
        .map(|&b| if b == largest { 0 } else { b })
        .collect();
    assert_eq!(r.lod_level_bytes(), some);
    // And with room for none, nothing is made and nothing panics.
    r.set_lod_buffer_limit(1);
    r.set_scene(&scene, &[], &BTreeSet::new());
    r.wait_for_lod();
    assert!(r.lod_level_bytes().iter().all(|&b| b == 0));
    let cam = camera(FAR);
    let (kept, _) = r.visible_triangles(&cam).unwrap();
    assert_eq!(r.drawn_triangles(&cam).map(|d| d.0), Some(kept));
}

#[test]
fn a_new_model_drops_the_copies_of_the_old_one() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let mut r = renderer(&screen, &scene, true);
    assert!(!r.lod_level_bytes().is_empty());
    let bytes = r.lod_level_bytes();
    // Many sets without a frame between, as an edit or a transition makes.
    for _ in 0..5 {
        r.set_scene(&scene, &[], &BTreeSet::new());
        assert!(r.lod_level_bytes().is_empty(), "the old copies are gone");
    }
    r.wait_for_lod();
    assert_eq!(r.lod_level_bytes(), bytes);
}

#[test]
fn the_pixel_limit_is_kept_finite_and_positive() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = plane();
    let mut r = renderer(&screen, &scene, true);
    let cam = camera(FAR);
    let (drawn, _) = r.drawn_triangles(&cam).unwrap();
    for bad in [f32::NAN, f32::INFINITY, -3.0, 0.0] {
        r.set_lod_pixels(bad);
        let (now, _) = r.drawn_triangles(&cam).unwrap();
        assert!(now > 0 && now <= 2 * (SIDE * SIDE) as u32, "{bad}: {now}");
    }
    r.set_lod_pixels(f32::NAN);
    assert_eq!(r.drawn_triangles(&cam).unwrap().0, drawn);
}
