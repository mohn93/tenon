//! Culling a big model by spatial chunk: what is out of view or too small to see is not drawn,
//! and the picture is the one of drawing it whole.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{DefinitionKind, EdgeStyle, EntityId, MeshId, Model, Scene, StyleSettings};
use render::{HoverLook, Viewport};
use session::Camera;

use crate::headless::{H, Offscreen, W};

/// How many squares lie in the row, `STEP` apart along x.
const SQUARES: u32 = 300;
const STEP: f64 = 6.0;

/// A row of 4 m square components on the ground, and the first instance's id.
fn row() -> (Scene, EntityId) {
    row_of(SQUARES)
}

/// `row` with `count` squares.
fn row_of(count: u32) -> (Scene, EntityId) {
    let mut m = Model::new();
    let p = |x: f64, y: f64| DVec3::new(x, y, 0.0);
    m.add_polygon(&[p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)])
        .unwrap();
    let edges: Vec<_> = m.edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    let mut s = Scene::from_root(m);
    let (first, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Square",
        )
        .unwrap();
    s.copy_instances(
        MeshId::Root,
        &[first],
        DVec3::new(STEP, 0.0, 0.0),
        count - 1,
    )
    .unwrap();
    (s, EntityId::Instance(first))
}

fn camera_at(eye: DVec3, target: DVec3) -> Camera {
    let mut cam = Camera::new(DVec2::new(W as f64, H as f64));
    cam.eye = eye;
    cam.target = target;
    cam
}

/// Looking down the row from the side at its first dozen squares.
fn near_start() -> Camera {
    camera_at(DVec3::new(30.0, -60.0, 40.0), DVec3::new(30.0, 0.0, 0.0))
}

const HOVER: HoverLook = HoverLook {
    face: [1.0, 0.5, 0.0],
    face_mix: 0.7,
    edge: [1.0, 0.0, 0.0, 1.0],
};

fn coloured(px: &[u8]) -> usize {
    px.chunks(4)
        .filter(|p| p[0].abs_diff(p[2]) > 20 || p[0] < 200)
        .count()
}

#[test]
fn chunks_out_of_view_are_not_drawn_and_the_picture_is_the_same() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let cam = near_start();
    let full = Viewport::full((W, H));
    // Small chunks, so a model this size has many.
    let mut culled = screen.renderer();
    culled.set_culling_from(100);
    culled.set_scene(&scene, &[], &BTreeSet::new());
    let mut whole = screen.renderer();
    whole.set_culling_from(usize::MAX);
    whole.set_scene(&scene, &[], &BTreeSet::new());

    let (drawn, all) = culled.visible_triangles(&cam).expect("culled by chunk");
    assert_eq!(all, 2 * SQUARES);
    assert!(drawn > 0 && drawn < all / 2, "drew {drawn} of {all}");
    assert!(whole.visible_triangles(&cam).is_none());

    let a = screen.draw(&mut culled, &cam, full, &[]);
    let b = screen.draw(&mut whole, &cam, full, &[]);
    assert!(coloured(&b) > 500, "the squares are in the picture");
    assert!(a == b, "culling changed the picture");
}

#[test]
fn a_view_of_the_whole_model_draws_all_of_it() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    // From above the middle of the row, high enough to see all of it.
    let cam = camera_at(
        DVec3::new(0.5 * SQUARES as f64 * STEP, -1200.0, 1500.0),
        DVec3::new(0.5 * SQUARES as f64 * STEP, 0.0, 0.0),
    );
    let mut culled = screen.renderer();
    culled.set_culling_from(100);
    culled.set_scene(&scene, &[], &BTreeSet::new());
    let (drawn, all) = culled.visible_triangles(&cam).unwrap();
    assert_eq!(drawn, all);
    let mut whole = screen.renderer();
    whole.set_culling_from(usize::MAX);
    whole.set_scene(&scene, &[], &BTreeSet::new());
    let full = Viewport::full((W, H));
    assert!(screen.draw(&mut culled, &cam, full, &[]) == screen.draw(&mut whole, &cam, full, &[]));
}

#[test]
fn selection_and_hover_light_the_same_triangles_after_chunking() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let instance = |n: usize| {
        scene
            .root()
            .instances()
            .map(|(id, _)| EntityId::Instance(id))
            .nth(n)
            .unwrap()
    };
    let (first, second) = (instance(4), instance(5));
    let cam = near_start();
    let full = Viewport::full((W, H));
    let mut culled = screen.renderer();
    culled.set_culling_from(100);
    culled.set_scene(&scene, &[], &BTreeSet::new());
    let mut whole = screen.renderer();
    whole.set_culling_from(usize::MAX);
    whole.set_scene(&scene, &[], &BTreeSet::new());

    let plain = screen.draw(&mut whole, &cam, full, &[]);
    let selection = BTreeSet::from([first]);
    culled.set_selection(&selection);
    whole.set_selection(&selection);
    let a = screen.draw(&mut culled, &cam, full, &[]);
    let b = screen.draw(&mut whole, &cam, full, &[]);
    assert!(a != plain, "the selection shows");
    assert!(a == b, "selection lights other triangles");

    culled.set_hover(Some((second, HOVER)));
    whole.set_hover(Some((second, HOVER)));
    let a = screen.draw(&mut culled, &cam, full, &[]);
    let b = screen.draw(&mut whole, &cam, full, &[]);
    assert!(a == b, "hover lights other triangles");
    whole.set_hover(None);
    assert!(a != screen.draw(&mut whole, &cam, full, &[]), "hover shows");
}

#[test]
fn chunks_too_small_to_see_are_left_out_and_visible_ones_are_kept() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let mut culled = screen.renderer();
    culled.set_culling_from(100);
    culled.set_scene(&scene, &[], &BTreeSet::new());
    // Far enough that the whole row is under a pixel across.
    let far = camera_at(
        DVec3::new(900.0, -2.0e6, 1.0e6),
        DVec3::new(900.0, 0.0, 0.0),
    );
    assert_eq!(culled.visible_triangles(&far).map(|v| v.0), Some(0));
    // Near enough that a chunk is a few pixels: it is drawn.
    let near = camera_at(
        DVec3::new(900.0, -9000.0, 4000.0),
        DVec3::new(900.0, 0.0, 0.0),
    );
    let (drawn, _) = culled.visible_triangles(&near).unwrap();
    assert!(drawn > 0);
    let full = Viewport::full((W, H));
    let mut whole = screen.renderer();
    whole.set_culling_from(usize::MAX);
    whole.set_scene(&scene, &[], &BTreeSet::new());
    let a = screen.draw(&mut culled, &near, full, &[]);
    assert!(a == screen.draw(&mut whole, &near, full, &[]));
}

#[test]
fn a_model_below_the_threshold_is_drawn_whole() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let mut r = screen.renderer();
    r.set_scene(&scene, &[], &BTreeSet::new());
    assert!(r.visible_triangles(&near_start()).is_none());
}

#[test]
fn see_through_faces_are_culled_and_sorted_by_chunk_and_look_the_same() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let cam = near_start();
    let full = Viewport::full((W, H));
    let see_through = render::scene::GlassView {
        x_ray: true,
        hidden_geometry: false,
    };
    let make = |from: usize| {
        let mut r = screen.renderer();
        r.set_glass_view(see_through);
        r.set_culling_from(from);
        r.set_scene(&scene, &[], &BTreeSet::new());
        r
    };
    let (mut culled, mut whole) = (make(100), make(usize::MAX));
    // Every triangle is see-through, so none is in the opaque list.
    assert_eq!(culled.visible_triangles(&cam), Some((0, 0)));
    let a = screen.draw(&mut culled, &cam, full, &[]);
    let b = screen.draw(&mut whole, &cam, full, &[]);
    assert!(coloured(&b) > 500, "the squares are in the picture");
    assert!(a == b, "culling changed the picture");
    // Looking from the other end draws the other squares, sorted again.
    let cam = camera_at(
        DVec3::new(1750.0, -60.0, 40.0),
        DVec3::new(1750.0, 0.0, 0.0),
    );
    assert!(screen.draw(&mut culled, &cam, full, &[]) == screen.draw(&mut whole, &cam, full, &[]));
}

#[test]
fn a_parallel_view_culls_by_chunk_and_looks_the_same() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let mut cam = near_start();
    cam.projection = session::Projection::Parallel;
    let full = Viewport::full((W, H));
    let make = |from: usize| {
        let mut r = screen.renderer();
        r.set_culling_from(from);
        r.set_scene(&scene, &[], &BTreeSet::new());
        r
    };
    let (mut culled, mut whole) = (make(100), make(usize::MAX));
    let (drawn, all) = culled.visible_triangles(&cam).unwrap();
    assert!(drawn > 0 && drawn < all, "drew {drawn} of {all}");
    let a = screen.draw(&mut culled, &cam, full, &[]);
    assert!(coloured(&a) > 500, "the squares are in the picture");
    assert!(a == screen.draw(&mut whole, &cam, full, &[]));
}

#[test]
fn a_model_that_grows_past_the_threshold_is_chunked_on_the_same_renderer() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (small, _) = row_of(20);
    let (big, _) = row();
    let cam = near_start();
    let full = Viewport::full((W, H));
    let mut r = screen.renderer();
    r.set_culling_from(100);
    let mut whole = screen.renderer();
    whole.set_culling_from(usize::MAX);
    // 40 triangles: drawn whole; then 600: chunked; then back.
    for (scene, chunked) in [(&small, false), (&big, true), (&small, false), (&big, true)] {
        r.set_scene(scene, &[], &BTreeSet::new());
        whole.set_scene(scene, &[], &BTreeSet::new());
        assert_eq!(r.visible_triangles(&cam).is_some(), chunked);
        assert!(screen.draw(&mut r, &cam, full, &[]) == screen.draw(&mut whole, &cam, full, &[]));
    }
}

fn edge_effects(edges: EdgeStyle) -> StyleSettings {
    StyleSettings {
        edges,
        ..StyleSettings::default()
    }
}

#[test]
fn profile_and_endpoint_edges_out_of_view_are_not_drawn_and_the_picture_is_the_same() {
    let Some(screen) = Offscreen::new() else {
        return;
    };
    let (scene, _) = row();
    let full = Viewport::full((W, H));
    let styles = [
        edge_effects(EdgeStyle {
            profiles: Some(6.0),
            ..EdgeStyle::default()
        }),
        edge_effects(EdgeStyle {
            endpoints: Some(8.0),
            ..EdgeStyle::default()
        }),
        edge_effects(EdgeStyle {
            profiles: Some(6.0),
            endpoints: Some(8.0),
            ..EdgeStyle::default()
        }),
    ];
    let cameras = [
        near_start(),
        camera_at(
            DVec3::new(1750.0, -60.0, 40.0),
            DVec3::new(1750.0, 0.0, 0.0),
        ),
    ];
    for style in &styles {
        let make = |from: usize| {
            let mut r = screen.renderer();
            r.set_model_style(Some(style));
            r.set_culling_from(from);
            r.set_scene(&scene, &[], &BTreeSet::new());
            r
        };
        let (mut culled, mut whole) = (make(100), make(usize::MAX));
        for cam in &cameras {
            if style.edges.profiles.is_some() {
                let (drawn, all) = culled.visible_profile_edges(cam).expect("culled by chunk");
                assert!(drawn > 0 && drawn < all / 2, "drew {drawn} of {all}");
            }
            let a = screen.draw(&mut culled, cam, full, &[]);
            let b = screen.draw(&mut whole, cam, full, &[]);
            assert!(coloured(&b) > 500 || b.iter().any(|&v| v != b[0]));
            assert!(a == b, "culling changed the picture of {style:?}");
        }
    }
}
