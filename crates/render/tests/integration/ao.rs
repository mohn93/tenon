//! Ambient occlusion: a style setting that darkens creases and inside corners by what the
//! shading's ambient and diffuse light reach, and leaves open flat faces, emission and edges
//! as they are.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{AmbientOcclusion, StyleSettings};
use render::Snapshotter;
use session::{Camera, Command, Document, Lighting};

use crate::golden::{created_face, doc, look, raise, run};

const W: u32 = 320;
const H: u32 = 240;

fn gpu() -> Option<Snapshotter> {
    let snap = Snapshotter::new_headless();
    if snap.is_none() {
        eprintln!("no GPU adapter, skipping");
    }
    snap
}

/// A 6 x 6 m floor face with a 2 m high, 0.2 m thick wall standing on it along the X axis.
fn room() -> Document {
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [-3.0, -3.0, 0.0],
            opposite: [3.0, 3.0, 0.0],
            normal: None,
        },
    );
    let wall = run(
        &mut d,
        Command::AddRectangle {
            corner: [-1.0, 0.0, 0.0],
            opposite: [1.0, 0.2, 0.0],
            normal: None,
        },
    );
    raise(&mut d, created_face(&wall), 2.0);
    look(&mut d, [0.5, -5.0, 3.0], [0.0, 0.0, 0.8]);
    d
}

fn style(on: bool, radius: f32, strength: f32) -> StyleSettings {
    StyleSettings {
        ambient_occlusion: AmbientOcclusion {
            on,
            radius,
            strength,
        },
        ..StyleSettings::default()
    }
}

struct Shot {
    pixels: Vec<u8>,
    camera: Camera,
}

impl Shot {
    /// The brightness (mean of the channels) of the pixel `p` projects to.
    fn at(&self, p: DVec3) -> f64 {
        let s = self.camera.project(p).expect("in front of the camera");
        assert!(s.x >= 0.0 && s.y >= 0.0 && s.x < f64::from(W) && s.y < f64::from(H));
        let i = ((s.y as u32 * W + s.x as u32) * 4) as usize;
        (0..3).map(|k| f64::from(self.pixels[i + k])).sum::<f64>() / 3.0
    }
}

fn shoot(snap: &mut Snapshotter, d: &Document, style: &StyleSettings, lighting: Lighting) -> Shot {
    snap.set_model_style(Some(style));
    let pixels = snap
        .capture(
            d.scene(),
            d.context(),
            &BTreeSet::new(),
            &d.camera,
            lighting,
            W,
            H,
        )
        .expect("capture");
    let mut camera = d.camera.clone();
    camera.viewport = DVec2::new(f64::from(W), f64::from(H));
    Shot { pixels, camera }
}

impl Shot {
    /// The darkest brightness within two pixels above and below the pixel `p` projects to.
    fn darkest_near(&self, p: DVec3) -> f64 {
        let s = self.camera.project(p).expect("in front of the camera");
        (-2..=2)
            .map(|dy| {
                let y = (s.y as i32 + dy) as u32;
                let i = ((y * W + s.x as u32) * 4) as usize;
                (0..3).map(|k| f64::from(self.pixels[i + k])).sum::<f64>() / 3.0
            })
            .fold(f64::MAX, f64::min)
    }
}

/// On the floor in front of the wall's foot, on the wall just above it, and on open parts of
/// the floor and the wall.
const FLOOR_CREASE: DVec3 = DVec3::new(0.0, -0.12, 0.0);
const WALL_CREASE: DVec3 = DVec3::new(0.0, 0.0, 0.12);
const OPEN_FLOOR: [DVec3; 2] = [DVec3::new(1.8, -1.2, 0.0), DVec3::new(-2.2, 2.0, 0.0)];
const OPEN_WALL: DVec3 = DVec3::new(0.0, 0.0, 1.1);

#[test]
fn a_crease_is_darker_with_it_on_and_open_faces_are_not() {
    let Some(mut snap) = gpu() else { return };
    let d = room();
    for lighting in [Lighting::Day, Lighting::Studio] {
        let off = shoot(&mut snap, &d, &style(false, 0.5, 1.0), lighting);
        let on = shoot(&mut snap, &d, &style(true, 0.5, 1.0), lighting);
        for crease in [FLOOR_CREASE, WALL_CREASE] {
            let (a, b) = (off.at(crease), on.at(crease));
            assert!(
                b < a * 0.9,
                "{lighting:?} crease {crease}: {b} with, {a} without"
            );
        }
        for open in OPEN_FLOOR.into_iter().chain([OPEN_WALL]) {
            let (a, b) = (off.at(open), on.at(open));
            assert!(
                (a - b).abs() <= 2.0,
                "{lighting:?} open face {open}: {b} with, {a} without"
            );
        }
    }
}

#[test]
fn strength_scales_the_darkening_and_zero_changes_nothing() {
    let Some(mut snap) = gpu() else { return };
    let d = room();
    let off = shoot(&mut snap, &d, &style(false, 0.5, 1.0), Lighting::Day);
    let none = shoot(&mut snap, &d, &style(true, 0.5, 0.0), Lighting::Day);
    let mild = shoot(&mut snap, &d, &style(true, 0.5, 0.5), Lighting::Day);
    let full = shoot(&mut snap, &d, &style(true, 0.5, 1.0), Lighting::Day);
    let double = shoot(&mut snap, &d, &style(true, 0.5, 2.0), Lighting::Day);
    let at = |s: &Shot| s.at(FLOOR_CREASE);
    assert!((at(&none) - at(&off)).abs() <= 1.0);
    assert!(at(&mild) < at(&off) - 3.0);
    assert!(at(&full) < at(&mild) - 3.0);
    assert!(at(&double) < at(&full) - 3.0);
}

#[test]
fn the_radius_sets_how_far_from_the_crease_it_reaches() {
    let Some(mut snap) = gpu() else { return };
    let d = room();
    // 0.3 m out from the wall's foot: far for a 0.1 m radius, near for a 1.5 m one.
    let p = DVec3::new(0.0, -0.3, 0.0);
    let off = shoot(&mut snap, &d, &style(false, 0.1, 1.0), Lighting::Day).at(p);
    let small = shoot(&mut snap, &d, &style(true, 0.1, 1.0), Lighting::Day).at(p);
    let large = shoot(&mut snap, &d, &style(true, 1.5, 1.0), Lighting::Day).at(p);
    assert!((small - off).abs() <= 2.0, "{small} vs {off}");
    assert!(large < off * 0.9, "{large} vs {off}");
}

#[test]
fn off_draws_exactly_as_before_whatever_the_other_settings() {
    let Some(mut snap) = gpu() else { return };
    let d = room();
    let a = shoot(&mut snap, &d, &style(false, 0.25, 1.0), Lighting::Day);
    let b = shoot(&mut snap, &d, &style(false, 7.0, 2.0), Lighting::Day);
    assert_eq!(a.pixels, b.pixels);
    // And turning it on and off again leaves no trace.
    shoot(&mut snap, &d, &style(true, 0.5, 1.0), Lighting::Day);
    let c = shoot(&mut snap, &d, &style(false, 0.25, 1.0), Lighting::Day);
    assert_eq!(a.pixels, c.pixels);
}

#[test]
fn it_follows_a_moved_camera() {
    let Some(mut snap) = gpu() else { return };
    let mut d = room();
    let on = style(true, 0.5, 1.0);
    let before = shoot(&mut snap, &d, &on, Lighting::Day);
    let unlit = shoot(&mut snap, &d, &style(false, 0.5, 1.0), Lighting::Day);
    assert!(before.at(FLOOR_CREASE) < unlit.at(FLOOR_CREASE) * 0.9);
    // A camera on the other side of the wall: the crease it sees is darkened too.
    look(&mut d, [-0.5, 5.0, 3.0], [0.0, 0.0, 0.8]);
    let off = shoot(&mut snap, &d, &style(false, 0.5, 1.0), Lighting::Day);
    let shot = shoot(&mut snap, &d, &on, Lighting::Day);
    let behind = DVec3::new(0.0, 0.32, 0.0);
    assert!(shot.at(behind) < off.at(behind) * 0.9);
}

/// The faces of the open context whose normal is `n`, at the plane `along` of `n`.
fn faces_facing(d: &Document, n: DVec3, along: f64) -> Vec<kernel::EntityId> {
    d.model()
        .faces()
        .filter(|(_, f)| f.normal.dot(n) > 0.99)
        .filter(|(_, f)| {
            f.outer
                .iter()
                .all(|v| (d.model().position(*v).unwrap().dot(n) - along).abs() < 1e-6)
        })
        .map(|(id, _)| kernel::EntityId::Face(id))
        .collect()
}

#[test]
fn a_glowing_face_and_the_edges_keep_their_color() {
    let Some(mut snap) = gpu() else { return };
    let mut d = room();
    // The wall's front face glows; its foot (the crease) is a drawn edge.
    let lamp = crate::golden::add_lamp_material(&mut d, "Lamp", [200, 220, 120], 1.0);
    let front = faces_facing(&d, -DVec3::Y, 0.0);
    assert_eq!(front.len(), 1);
    run(
        &mut d,
        Command::Paint {
            ids: front,
            material: Some(lamp),
            side: Default::default(),
        },
    );
    let off = shoot(&mut snap, &d, &style(false, 0.5, 1.0), Lighting::Day);
    let on = shoot(&mut snap, &d, &style(true, 0.5, 1.0), Lighting::Day);
    for dz in [0.02, 0.06, 0.12] {
        let p = DVec3::new(0.0, 0.0, dz);
        assert!((on.at(p) - off.at(p)).abs() <= 1.0, "glow at {dz}");
    }
    // The line itself: the darkest pixel on the wall's foot is as dark as the edge color.
    let foot = DVec3::new(0.3, 0.0, 0.0);
    let (a, b) = (off.darkest_near(foot), on.darkest_near(foot));
    assert!(a < 150.0 && (a - b).abs() <= 1.0, "{a} without, {b} with");
}

#[test]
fn a_parallel_view_and_a_viewport_inside_the_target_draw_it_too() {
    use render::Viewport;

    let Some(screen) = crate::headless::Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut d = room();
    run(
        &mut d,
        Command::SetProjection {
            projection: session::Projection::Parallel,
        },
    );
    look(&mut d, [0.5, -8.0, 4.0], [0.0, 0.0, 0.8]);
    // A view 160 x 160 at an odd offset in the 256 x 256 target.
    let (x, y, size) = (63u32, 47u32, 160u32);
    let mut camera = d.camera.clone();
    camera.viewport = DVec2::new(f64::from(size), f64::from(size));
    let viewport = Viewport {
        x,
        y,
        width: size,
        height: size,
    };
    let mut renderer = screen.renderer();
    renderer.set_scene(d.scene(), d.context(), &BTreeSet::new());
    let mut shot = |on: bool| {
        renderer.set_model_style(Some(&style(on, 0.5, 1.0)));
        screen.draw(&mut renderer, &camera, Viewport { ..viewport }, &[])
    };
    let (off, on) = (shot(false), shot(true));
    let at = |px: &[u8], p: DVec3| {
        let s = camera.project(p).unwrap();
        let (sx, sy) = (s.x as u32 + x, s.y as u32 + y);
        let i = ((sy * crate::headless::W + sx) * 4) as usize;
        (0..3).map(|k| f64::from(px[i + k])).sum::<f64>() / 3.0
    };
    for crease in [FLOOR_CREASE, WALL_CREASE] {
        assert!(at(&on, crease) < at(&off, crease) * 0.9, "{crease}");
    }
    for open in [OPEN_FLOOR[0], OPEN_WALL] {
        assert!((at(&on, open) - at(&off, open)).abs() <= 2.0, "{open}");
    }
    // Outside the view nothing changes.
    for (px, py) in [(2u32, 2u32), (250, 250), (10, 200)] {
        let i = ((py * crate::headless::W + px) * 4) as usize;
        assert_eq!(on[i..i + 4], off[i..i + 4]);
    }
}

#[test]
fn a_renderer_told_it_has_none_draws_without_it() {
    let Some(screen) = crate::headless::Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let d = room();
    let mut camera = d.camera.clone();
    camera.viewport = DVec2::new(f64::from(crate::headless::W), f64::from(crate::headless::H));
    let mut renderer = screen.renderer();
    assert!(renderer.ambient_occlusion_available());
    renderer.set_scene(d.scene(), d.context(), &BTreeSet::new());
    renderer.set_model_style(Some(&style(true, 0.5, 1.0)));
    let viewport = render::Viewport::full((crate::headless::W, crate::headless::H));
    let with = screen.draw(&mut renderer, &camera, viewport, &[]);
    renderer.disable_ambient_occlusion();
    assert!(!renderer.ambient_occlusion_available());
    let without = screen.draw(&mut renderer, &camera, viewport, &[]);
    renderer.set_model_style(Some(&style(false, 0.5, 1.0)));
    let plain = screen.draw(&mut renderer, &camera, viewport, &[]);
    assert_ne!(with, plain);
    assert_eq!(without, plain);
}

#[test]
fn a_size_change_between_frames_keeps_it_working_and_off_leaves_nothing_behind() {
    let Some(mut snap) = gpu() else { return };
    let d = room();
    let on = style(true, 0.5, 1.0);
    let off = style(false, 0.5, 1.0);
    let sized = |snap: &mut Snapshotter, style: &StyleSettings, (w, h): (u32, u32)| {
        snap.set_model_style(Some(style));
        let pixels = snap
            .capture(
                d.scene(),
                d.context(),
                &BTreeSet::new(),
                &d.camera,
                Lighting::Day,
                w,
                h,
            )
            .expect("capture");
        let mut camera = d.camera.clone();
        camera.viewport = DVec2::new(f64::from(w), f64::from(h));
        let at = |p: DVec3| {
            let s = camera.project(p).unwrap();
            let i = ((s.y as u32 * w + s.x as u32) * 4) as usize;
            (0..3).map(|k| f64::from(pixels[i + k])).sum::<f64>() / 3.0
        };
        let (crease, open) = (at(FLOOR_CREASE), at(OPEN_FLOOR[0]));
        (pixels, crease, open)
    };
    let first = sized(&mut snap, &on, (W, H));
    // Smaller, an odd size, then the first size again: each frame darkens the crease and not the
    // open floor, and the last is the first again.
    for size in [(200, 150), (251, 187)] {
        let plain = sized(&mut snap, &off, size);
        let (_, crease, open) = sized(&mut snap, &on, size);
        assert!(crease < plain.1 * 0.9, "{size:?}: {crease} vs {}", plain.1);
        assert!((open - plain.2).abs() <= 2.0, "{size:?}");
    }
    let again = sized(&mut snap, &on, (W, H));
    assert_eq!(first.0, again.0);
    // Off after on draws exactly what it drew before the first on.
    let fresh = sized(&mut gpu().unwrap(), &off, (W, H));
    let after = sized(&mut snap, &off, (W, H));
    assert_eq!(fresh.0, after.0);
}

#[test]
fn see_through_faces_are_not_darkened() {
    let Some(mut snap) = gpu() else { return };
    let d = room();
    snap.set_glass_view(render::scene::GlassView {
        x_ray: true,
        hidden_geometry: false,
    });
    let off = shoot(&mut snap, &d, &style(false, 0.5, 1.0), Lighting::Day);
    let on = shoot(&mut snap, &d, &style(true, 0.5, 1.0), Lighting::Day);
    snap.set_glass_view(render::scene::GlassView::default());
    for p in [FLOOR_CREASE, WALL_CREASE, OPEN_WALL] {
        assert!((on.at(p) - off.at(p)).abs() <= 1.0, "{p}");
    }
}
