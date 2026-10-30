//! Models far from the world origin (geo-referenced, about 5e6 m out) draw as they do at the
//! origin: the scene's positions are relative to a point near the model, and the camera, the
//! overlay and the chunks are worked out in that frame.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{Model, Scene, StyleSettings};
use render::grid::LineVertex;
use render::{Renderer, Viewport};
use session::Camera;

use crate::headless::{H, Offscreen, W};

const SIDE: usize = 30;
const BLOCK: usize = 15;
const CHUNK: usize = 900;
/// 5e6 m east, 4.9e6 m north: UTM-like numbers.
const FAR: DVec3 = DVec3::new(5.0e6, 4.9e6, 0.0);

/// A flat plane of `SIDE` x `SIDE` squares of 1 m with the corner at `at`.
fn plane(at: DVec3) -> Scene {
    let mut m = Model::new();
    let mut order: Vec<(usize, usize)> = Vec::new();
    for i in 0..SIDE {
        for j in 0..SIDE {
            order.push((i, j));
        }
    }
    order.sort_by_key(|&(i, j)| (i / BLOCK, j / BLOCK, i, j));
    for (i, j) in order {
        let p = |x: usize, y: usize| at + DVec3::new(x as f64, y as f64, 0.0);
        m.add_polygon(&[p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1)])
            .unwrap();
    }
    Scene::from_root(m)
}

fn renderer(screen: &Offscreen, scene: &Scene, lod: bool) -> Renderer {
    let mut r = screen.renderer();
    r.set_culling_from(CHUNK);
    // The default look: edges drawn, which show any jitter of the positions.
    r.set_model_style(Some(&StyleSettings::default()));
    r.set_lod(lod);
    r.set_scene(scene, &[], &BTreeSet::new());
    if lod {
        r.wait_for_lod();
    }
    r
}

fn camera(at: DVec3, height: f64) -> Camera {
    let mut cam = Camera::new(DVec2::new(W as f64, H as f64));
    let mid = SIDE as f64 / 2.0;
    cam.eye = at + DVec3::new(mid, mid - 0.2 * height, height);
    cam.target = at + DVec3::new(mid, mid, 0.0);
    cam
}

fn differing(a: &[u8], b: &[u8], levels: u8) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..3).any(|i| p[i].abs_diff(q[i]) > levels))
        .count()
}

/// The pixels that are not the plain background's: the model is in the picture.
fn lit(picture: &[u8]) -> usize {
    let corner = &picture[..4];
    picture.chunks(4).filter(|p| p[..3] != corner[..3]).count()
}

#[test]
fn a_model_5e6_m_out_draws_the_picture_of_one_at_the_origin() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let full = Viewport::full((W, H));
    let mut here = renderer(&screen, &plane(DVec3::ZERO), false);
    let mut there = renderer(&screen, &plane(FAR), false);
    for height in [12.0, 40.0] {
        let a = screen.draw(&mut here, &camera(DVec3::ZERO, height), full, &[]);
        let b = screen.draw(&mut there, &camera(FAR, height), full, &[]);
        assert!(lit(&a) > 1000, "the plane is in the picture");
        let off = differing(&a, &b, 3);
        // Tiny differences remain: the origin is not the world's, so f32 rounds differently.
        assert!(
            off * 1000 <= W as usize * H as usize,
            "{off} pixels differ at {height} m"
        );
    }
}

#[test]
fn chunks_are_culled_and_simplified_the_same_far_out() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let here = renderer(&screen, &plane(DVec3::ZERO), true);
    let mut there = renderer(&screen, &plane(FAR), true);
    for height in [12.0, 700.0] {
        let (near_cam, far_cam) = (camera(DVec3::ZERO, height), camera(FAR, height));
        assert_eq!(
            here.visible_triangles(&near_cam),
            there.visible_triangles(&far_cam),
            "visible at {height} m"
        );
        assert_eq!(
            here.drawn_triangles(&near_cam),
            there.drawn_triangles(&far_cam),
            "drawn at {height} m"
        );
    }
    // Looking away from the plane culls all of it, as at the origin.
    let mut away = camera(FAR, 12.0);
    away.target = away.eye + (away.eye - camera(FAR, 12.0).target);
    let mut away_here = camera(DVec3::ZERO, 12.0);
    away_here.target = away_here.eye + (away_here.eye - camera(DVec3::ZERO, 12.0).target);
    assert_eq!(
        here.visible_triangles(&away_here).map(|v| v.0),
        Some(0),
        "culled at the origin"
    );
    assert_eq!(there.visible_triangles(&away).map(|v| v.0), Some(0));
    // The far view with simplified copies matches the one with the whole model.
    let full = Viewport::full((W, H));
    let simple = screen.draw(&mut there, &camera(FAR, 700.0), full, &[]);
    there.set_lod(false);
    let whole = screen.draw(&mut there, &camera(FAR, 700.0), full, &[]);
    assert!(lit(&simple) > 100);
    assert!(differing(&simple, &whole, 6) * 100 <= W as usize * H as usize);
}

#[test]
fn an_overlay_in_world_coordinates_lands_on_the_model_far_out() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let full = Viewport::full((W, H));
    // A line across the plane, at whole meters (which f32 holds at 5e6).
    let line = |at: DVec3| {
        let v = |x: f64| LineVertex {
            pos: (at + DVec3::new(x, 15.0, 0.0)).as_vec3().to_array(),
            color: [1.0, 0.0, 0.0, 1.0],
        };
        [v(3.0), v(27.0)]
    };
    let mut here = renderer(&screen, &plane(DVec3::ZERO), false);
    let mut there = renderer(&screen, &plane(FAR), false);
    let a = screen.draw(
        &mut here,
        &camera(DVec3::ZERO, 30.0),
        full,
        &line(DVec3::ZERO),
    );
    let b = screen.draw(&mut there, &camera(FAR, 30.0), full, &line(FAR));
    let plain = screen.draw(&mut there, &camera(FAR, 30.0), full, &[]);
    assert!(differing(&b, &plain, 3) > 50, "the overlay is drawn");
    assert!(differing(&a, &b, 3) * 1000 <= W as usize * H as usize);
}
