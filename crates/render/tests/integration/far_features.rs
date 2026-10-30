//! Features drawn from the scene's frame far from the world origin: section cuts and caps,
//! guides, dimensions, the sun's shadows, the preview, and a renderer whose origin moves.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{Model, Scene, StyleSettings};
use render::{Renderer, Viewport};
use session::Camera;

use crate::headless::{H, Offscreen, W};

/// 5e6 m east, 4.9e6 m north: UTM-like numbers.
const FAR: DVec3 = DVec3::new(5.0e6, 4.9e6, 0.0);

fn differing(a: &[u8], b: &[u8], levels: u8) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..3).any(|i| p[i].abs_diff(q[i]) > levels))
        .count()
}

/// A 10 x 10 x 5 m box at `at` with, as asked, a guide line, a dimension and an active section
/// plane through it.
fn box_scene(at: DVec3, guide: bool, dimension: bool, section: bool) -> Scene {
    let mut m = Model::new();
    let p = |x: f64, y: f64, z: f64| at + DVec3::new(x, y, z);
    let quad = |a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]| {
        [a, b, c, d].map(|[x, y, z]| p(x, y, z))
    };
    for q in [
        quad([0., 0., 0.], [0., 10., 0.], [10., 10., 0.], [10., 0., 0.]),
        quad([0., 0., 5.], [10., 0., 5.], [10., 10., 5.], [0., 10., 5.]),
        quad([0., 0., 0.], [10., 0., 0.], [10., 0., 5.], [0., 0., 5.]),
        quad([10., 0., 0.], [10., 10., 0.], [10., 10., 5.], [10., 0., 5.]),
        quad([10., 10., 0.], [0., 10., 0.], [0., 10., 5.], [10., 10., 5.]),
        quad([0., 10., 0.], [0., 0., 0.], [0., 0., 5.], [0., 10., 5.]),
    ] {
        m.add_polygon(&q).unwrap();
    }
    if guide {
        m.add_guide(kernel::Guide::Line {
            point: p(5.0, 5.0, 8.0),
            direction: DVec3::X,
            tag: None,
            hidden: false,
        })
        .unwrap();
    }
    if dimension {
        m.add_dimension(kernel::Dimension::Linear {
            a: kernel::Anchor::Point(p(0.0, 0.0, 5.0)),
            b: kernel::Anchor::Point(p(10.0, 0.0, 5.0)),
            offset: DVec3::new(0.0, -2.0, 0.0),
            tag: None,
        })
        .unwrap();
    }
    if section {
        m.add_section(kernel::SectionPlane::new(p(5.0, 5.0, 2.0), DVec3::X), true)
            .unwrap();
    }
    Scene::from_root(m)
}

fn box_camera(at: DVec3) -> Camera {
    let mut cam = Camera::new(DVec2::new(W as f64, H as f64));
    cam.target = at + DVec3::new(5.0, 5.0, 2.0);
    cam.eye = cam.target + DVec3::new(-14.0, -22.0, 14.0);
    cam
}

fn renderer(screen: &Offscreen, setup: &dyn Fn(&mut Renderer)) -> Renderer {
    let mut r = screen.renderer();
    r.set_model_style(Some(&StyleSettings::default()));
    // The world's grid lies behind a model at the origin and is not drawn far from it.
    r.set_hide_ground(true);
    setup(&mut r);
    r
}

/// The picture of `scene` seen by `camera`, after `setup` configured the renderer.
fn picture(
    screen: &Offscreen,
    scene: &Scene,
    camera: &Camera,
    setup: &dyn Fn(&mut Renderer),
) -> Vec<u8> {
    let mut r = renderer(screen, setup);
    r.set_scene(scene, &[], &BTreeSet::new());
    screen.draw(&mut r, camera, Viewport::full((W, H)), &[])
}

/// Pictures of a scene at the origin and `FAR` away agree, and the one at the origin differs
/// from the plain scene's (so the feature is in the picture).
fn same_far_out(
    make: impl Fn(DVec3) -> Scene,
    plain: impl Fn(DVec3) -> Scene,
    setup: &dyn Fn(&mut Renderer),
    plain_setup: &dyn Fn(&mut Renderer),
) {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let near = picture(&screen, &make(DVec3::ZERO), &box_camera(DVec3::ZERO), setup);
    let far = picture(&screen, &make(FAR), &box_camera(FAR), setup);
    let bare = picture(
        &screen,
        &plain(DVec3::ZERO),
        &box_camera(DVec3::ZERO),
        plain_setup,
    );
    assert!(differing(&near, &bare, 3) > 30, "the feature is drawn");
    let off = differing(&near, &far, 3);
    assert!(
        off * 1000 <= W as usize * H as usize,
        "{off} pixels differ far out"
    );
}

#[test]
fn section_cuts_and_caps_draw_the_same_far_out() {
    let none: &dyn Fn(&mut Renderer) = &|_| {};
    let base = render::scene::scene_base(&box_scene(FAR, false, false, true), &[]);
    assert!(!base.cut_fill.is_empty(), "the cut is capped");
    assert!(
        base.cut_fill
            .iter()
            .all(|v| v.pos.iter().all(|c| c.abs() < 100.0))
    );
    same_far_out(
        |at| box_scene(at, false, false, true),
        |at| box_scene(at, false, false, false),
        none,
        none,
    );
}

#[test]
fn guides_and_annotations_draw_the_same_far_out() {
    let none: &dyn Fn(&mut Renderer) = &|_| {};
    same_far_out(
        |at| box_scene(at, true, true, false),
        |at| box_scene(at, false, false, false),
        none,
        none,
    );
}

#[test]
fn sun_shadows_fall_the_same_far_out() {
    let sun: &dyn Fn(&mut Renderer) = &|r| {
        r.set_sun(Some(render::SunLight {
            direction: glam::Vec3::new(-0.5, -0.5, 0.7).normalize(),
            light: 1.0,
            dark: 0.45,
            on_ground: true,
        }));
    };
    let none: &dyn Fn(&mut Renderer) = &|_| {};
    // The plain picture is the headlight's, without the sun.
    same_far_out(
        |at| box_scene(at, false, false, false),
        |at| box_scene(at, false, false, false),
        sun,
        none,
    );
}

#[test]
fn the_suns_shadow_on_the_ground_falls_in_the_same_place_far_out() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let sun: &dyn Fn(&mut Renderer) = &|r| {
        r.set_hide_ground(false);
        r.set_sun(Some(render::SunLight {
            direction: glam::Vec3::new(-0.6, -0.3, 0.6).normalize(),
            light: 1.0,
            dark: 0.45,
            on_ground: true,
        }));
    };
    let near = picture(
        &screen,
        &box_scene(DVec3::ZERO, false, false, false),
        &box_camera(DVec3::ZERO),
        sun,
    );
    let far = picture(
        &screen,
        &box_scene(FAR, false, false, false),
        &box_camera(FAR),
        sun,
    );
    // The grid is not drawn far out, so compare the mean brightness around a ground point in
    // the box's shadow (east of it) and one in the sun.
    let around = |px: &[u8], at: DVec3, p: DVec3| -> f64 {
        let s = box_camera(at).project(at + p).expect("in view");
        let (x, y) = (s.x as i64, s.y as i64);
        let mut sum = 0.0;
        for dy in -3..=3 {
            for dx in -3..=3 {
                let i = (((y + dy) as u32 * W + (x + dx) as u32) * 4) as usize;
                sum += f64::from(px[i]) + f64::from(px[i + 1]) + f64::from(px[i + 2]);
            }
        }
        sum / 49.0
    };
    let shade = DVec3::new(12.0, 3.0, 0.0);
    let lit = DVec3::new(-3.0, -1.0, 0.0);
    let (dark_near, bright_near) = (
        around(&near, DVec3::ZERO, shade),
        around(&near, DVec3::ZERO, lit),
    );
    let (dark_far, bright_far) = (around(&far, FAR, shade), around(&far, FAR, lit));
    assert!(
        dark_near < bright_near - 15.0,
        "{dark_near} {bright_near}: a shadow at the origin"
    );
    // Far out the ground has no grid, so only the shadow is compared, with some room for
    // the lines near the origin.
    assert!(
        dark_far < bright_far - 15.0,
        "{dark_far} {bright_far}: a shadow far out"
    );
    assert!(
        (dark_near - dark_far).abs() < 25.0,
        "{dark_near} {dark_far}: the same shadow"
    );
}

#[test]
fn a_preview_relative_to_the_origin_lands_on_the_model_far_out() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let full = Viewport::full((W, H));
    let draw = |at: DVec3, with: bool| {
        let mut r = renderer(&screen, &|_| {});
        r.set_scene(&box_scene(at, false, false, false), &[], &BTreeSet::new());
        if with {
            // A slab over the box, subtracted in f64 as the app does.
            let o = r.origin();
            let v = |x: f64, y: f64| render::PreviewVertex {
                pos: (at + DVec3::new(x, y, 6.0) - o).as_vec3().to_array(),
                normal: [0.0, 0.0, 1.0],
                color: [0.9, 0.3, 0.2, 0.7],
            };
            r.set_preview(&[v(-2.0, -2.0), v(12.0, -2.0), v(12.0, 12.0)]);
        }
        screen.draw(&mut r, &box_camera(at), full, &[])
    };
    let near = draw(DVec3::ZERO, true);
    let far = draw(FAR, true);
    let bare = draw(DVec3::ZERO, false);
    assert!(differing(&near, &bare, 3) > 100, "the preview is drawn");
    assert!(differing(&near, &far, 3) * 1000 <= W as usize * H as usize);
}

#[test]
fn a_renderer_moved_to_another_far_model_draws_it_as_a_new_one_does() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let full = Viewport::full((W, H));
    let other = DVec3::new(-6.0e6, 3.0e6, 0.0);
    let mut moved = renderer(&screen, &|_| {});
    moved.set_scene(&box_scene(FAR, true, true, true), &[], &BTreeSet::new());
    let _ = screen.draw(&mut moved, &box_camera(FAR), full, &[]);
    assert_ne!(moved.origin(), DVec3::ZERO);
    moved.set_scene(&box_scene(other, true, true, true), &[], &BTreeSet::new());
    let after = screen.draw(&mut moved, &box_camera(other), full, &[]);
    let fresh = picture(
        &screen,
        &box_scene(other, true, true, true),
        &box_camera(other),
        &|_| {},
    );
    assert_eq!(
        differing(&after, &fresh, 0),
        0,
        "the same as a fresh renderer"
    );
    // And back to the origin.
    moved.set_scene(
        &box_scene(DVec3::ZERO, true, true, true),
        &[],
        &BTreeSet::new(),
    );
    assert_eq!(moved.origin(), DVec3::ZERO);
    let home = screen.draw(&mut moved, &box_camera(DVec3::ZERO), full, &[]);
    let fresh = picture(
        &screen,
        &box_scene(DVec3::ZERO, true, true, true),
        &box_camera(DVec3::ZERO),
        &|_| {},
    );
    assert_eq!(differing(&home, &fresh, 0), 0);
}
