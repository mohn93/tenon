//! Level of detail: chunks whose detail is about a pixel are drawn simplified, and the picture
//! is the one of drawing them whole.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EdgeStyle, EntityId, Model, Scene, SectionPlane, StyleSettings};
use render::{HoverLook, Renderer, Viewport};
use session::{Camera, SectionDisplay};

use crate::headless::{H, Offscreen, W};

/// Squares along a side of the plane, and how long a square is: a fine mesh of one flat color.
const SIDE: usize = 30;
const STEP: f64 = 1.0;
/// The faces are made block by block, a block a chunk's piece (see `CHUNK`), so a chunk is a
/// compact patch and not a strip.
const BLOCK: usize = 15;
/// Triangles to a chunk: two blocks.
const CHUNK: usize = 900;

/// A flat plane of `SIDE` x `SIDE` squares of `STEP` m, the faces' ids.
fn plane() -> (Scene, Vec<EntityId>) {
    plane_cut(None)
}

/// The same, with `cut` as its active section plane.
fn plane_cut(cut: Option<SectionPlane>) -> (Scene, Vec<EntityId>) {
    let mut m = Model::new();
    if let Some(cut) = cut {
        m.add_section(cut, true).unwrap();
    }
    let mut order: Vec<(usize, usize)> = Vec::new();
    for i in 0..SIDE {
        for j in 0..SIDE {
            order.push((i, j));
        }
    }
    order.sort_by_key(|&(i, j)| (i / BLOCK, j / BLOCK, i, j));
    {
        for (i, j) in order {
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
    r.set_culling_from(CHUNK);
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
    r.set_culling_from(CHUNK);
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
    r.set_culling_from(CHUNK);
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

#[test]
fn chunks_near_an_active_section_plane_stay_whole_in_a_far_view() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    // The plane through the middle keeps the half with x >= 15. The cut's fill and outline are
    // made from the whole surface, so chunks within the largest cell (8 m) of it are not
    // simplified.
    let mid = SIDE as f64 * STEP / 2.0;
    let cut = SectionPlane::new(DVec3::new(mid, 0.0, 0.0), DVec3::X);
    let (scene, _) = plane_cut(Some(cut));
    let cam = camera(FAR);
    let on = renderer(&screen, &scene, true);
    let (kept, all) = on.visible_triangles(&cam).unwrap();
    let (drawn, _) = on.drawn_triangles(&cam).unwrap();
    assert_eq!(
        all,
        (SIDE * SIDE) as u32,
        "the plane keeps half the triangles"
    );
    assert_eq!(drawn, kept, "the chunks at the plane are drawn whole");
    // With the cuts off the same far view is simplified all over (nothing to protect).
    let mut r = screen.renderer();
    r.set_culling_from(CHUNK);
    r.set_model_style(Some(&style()));
    r.set_lod(true);
    r.set_section_display(SectionDisplay {
        cuts: false,
        ..SectionDisplay::default()
    });
    r.set_scene(&scene, &[], &BTreeSet::new());
    r.wait_for_lod();
    let (kept_off, _) = r.visible_triangles(&cam).unwrap();
    let (drawn_off, _) = r.drawn_triangles(&cam).unwrap();
    assert!(drawn_off * 3 < kept_off, "drew {drawn_off} of {kept_off}");
    assert!(drawn > drawn_off);
}

/// A patch of 1 m squares (made block by block, so chunks are compact) with one big square
/// beside it painted `paint` (a material of the document), seen from far above.
fn patch_with_feature(
    paint: impl FnOnce(&mut session::Document) -> session::MaterialId,
) -> session::Document {
    use crate::golden::{created_face, doc, run};
    use session::{Command, Side};
    let mut d = doc();
    // The big square first, so it shares a chunk with simplifiable squares.
    let big = run(
        &mut d,
        Command::AddRectangle {
            corner: [20.0, 0.0, 0.0],
            opposite: [44.0, 24.0, 0.0],
            normal: None,
        },
    );
    let mut order: Vec<(i32, i32)> = Vec::new();
    for i in 0..16 {
        for j in 0..16 {
            order.push((i, j));
        }
    }
    order.sort_by_key(|&(i, j)| (i / 8, j / 8, i, j));
    for (i, j) in order {
        let (x, y) = (f64::from(i), f64::from(j));
        run(
            &mut d,
            Command::AddRectangle {
                corner: [x, y, 0.0],
                opposite: [x + 1.0, y + 1.0, 0.0],
                normal: None,
            },
        );
    }
    let material = paint(&mut d);
    run(
        &mut d,
        Command::Paint {
            ids: vec![created_face(&big)],
            material: Some(material),
            side: Side::Both,
        },
    );
    d
}

/// How many pixels of a far view of `d` satisfy `colored`, drawn simplified and whole.
fn colored_pixels(d: &session::Document, colored: impl Fn(&[u8]) -> bool) -> (usize, usize) {
    let Some(screen) = Offscreen::new() else {
        return (0, 0);
    };
    let mut r = screen.renderer();
    r.set_culling_from(256);
    r.set_model_style(Some(&style()));
    r.set_lod(true);
    r.set_scene(d.scene(), &[], &BTreeSet::new());
    r.wait_for_lod();
    let mut cam = Camera::new(DVec2::new(W as f64, H as f64));
    cam.eye = DVec3::new(22.0, 12.0 - 140.0, 700.0);
    cam.target = DVec3::new(22.0, 12.0, 0.0);
    let full = Viewport::full((W, H));
    let simple = screen.draw(&mut r, &cam, full, &[]);
    let (kept, _) = r.visible_triangles(&cam).unwrap();
    let (drawn, _) = r.drawn_triangles(&cam).unwrap();
    assert!(drawn < kept, "the view is simplified: {drawn} of {kept}");
    r.set_lod(false);
    let whole = screen.draw(&mut r, &cam, full, &[]);
    let count = |p: &[u8]| p.chunks(4).filter(|c| colored(c)).count();
    (count(&simple), count(&whole))
}

#[test]
fn a_glowing_face_in_a_simplified_view_glows_as_it_does_in_full_detail() {
    let d = patch_with_feature(|d| crate::golden::add_lamp_material(d, "Lamp", [255, 0, 0], 1.0));
    let red = |c: &[u8]| c[0] > 200 && c[1] < 80 && c[2] < 80;
    let (simple, whole) = colored_pixels(&d, red);
    if whole == 0 {
        return; // no GPU
    }
    assert!(whole > 10, "the lamp shows: {whole} pixels");
    assert!(
        simple * 10 >= whole * 8,
        "{simple} glowing pixels of {whole}"
    );
}

#[test]
fn a_textured_face_in_a_simplified_view_shows_its_texture() {
    let d = patch_with_feature(|d| {
        crate::golden::run(
            d,
            session::Command::AddMaterial {
                name: Some("Tiles".into()),
                color: None,
                opacity: None,
                texture: Some(session::TextureSource {
                    path: Some(crate::golden::quadrants_png().display().to_string()),
                    width: Some(24.0),
                    height: Some(24.0),
                }),
                emission: None,
                metalness: None,
                roughness: None,
                density: None,
                existing: Default::default(),
            },
        )
        .changes
        .materials[0]
    });
    // The picture's colors are far from the cream of the patch and the blue of the background.
    let saturated = |c: &[u8]| {
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        hi - lo > 125 && !(c[2] > c[1] && c[1] > c[0] && c[2] > 180)
    };
    let (simple, whole) = colored_pixels(&d, saturated);
    if whole == 0 {
        return;
    }
    assert!(whole > 10, "the texture shows: {whole} pixels");
    assert!(
        simple * 10 >= whole * 8,
        "{simple} textured pixels of {whole}"
    );
}
