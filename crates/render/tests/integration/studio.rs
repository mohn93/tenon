//! Studio lighting: no headlight, a key, fill and rim light that follow the camera, a low
//! ambient light, the style's background as the backdrop, and the key light casting the shadows.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3, Vec3};
use kernel::StyleSettings;
use render::{Snapshotter, SunLight};
use session::{Camera, Command, Document, Lighting, Projection};

use crate::golden::{add_box, doc, look, run};

const W: u32 = 320;
const H: u32 = 240;

const KEY: [f64; 3] = [-0.5, 0.6, 0.6];
const FILL: [f64; 3] = [0.6, 0.1, 0.5];
const RIM: [f64; 3] = [0.0, 0.5, -0.8];

fn gpu() -> Option<Snapshotter> {
    let snap = Snapshotter::new_headless();
    if snap.is_none() {
        eprintln!("no GPU adapter, skipping");
    }
    snap
}

struct Shot {
    pixels: Vec<u8>,
    camera: Camera,
}

impl Shot {
    fn at(&self, p: DVec3) -> [f64; 3] {
        let s = self.camera.project(p).expect("in front of the camera");
        self.pixel(s.x as u32, s.y as u32)
    }

    fn pixel(&self, x: u32, y: u32) -> [f64; 3] {
        let i = ((y * W + x) * 4) as usize;
        [0, 1, 2].map(|k| f64::from(self.pixels[i + k]))
    }
}

fn shoot(
    snap: &mut Snapshotter,
    d: &Document,
    style: Option<&StyleSettings>,
    sun: Option<SunLight>,
    lighting: Lighting,
) -> Shot {
    snap.set_model_style(style);
    snap.set_sun(sun);
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

/// A 2 x 1.5 x 1 m box seen in a parallel view from `eye`.
fn parallel_box(eye: [f64; 3]) -> Document {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::SetProjection {
            projection: Projection::Parallel,
        },
    );
    look(&mut d, eye, [1.0, 0.75, 0.5]);
    d
}

/// The unit vector toward a light given in camera space (x right, y up, z toward the viewer).
fn toward(c: &Camera, v: [f64; 3]) -> DVec3 {
    (c.right() * v[0] + c.up() * v[1] - c.forward() * v[2]).normalize()
}

fn lambert(n: DVec3, l: DVec3) -> f64 {
    n.dot(l).max(0.0)
}

#[test]
fn faces_take_the_ambient_plus_key_fill_and_rim_by_the_angle_to_each() {
    let Some(mut snap) = gpu() else { return };
    // Two cameras: the lights follow the camera, so the same faces shade differently.
    for eye in [[3.0, -4.0, 5.0], [-6.0, -5.0, 4.0]] {
        let d = parallel_box(eye);
        let day = shoot(&mut snap, &d, None, None, Lighting::Day);
        let studio = shoot(&mut snap, &d, None, None, Lighting::Studio);
        let c = &studio.camera;
        let to_eye = -c.forward();
        let (key, fill, rim) = (toward(c, KEY), toward(c, FILL), toward(c, RIM));
        let mut checked = 0;
        for (n, p) in [
            (DVec3::Z, DVec3::new(1.0, 0.75, 1.0)),
            (-DVec3::Y, DVec3::new(1.0, 0.0, 0.5)),
            (DVec3::X, DVec3::new(2.0, 0.75, 0.5)),
            (-DVec3::X, DVec3::new(0.0, 0.75, 0.5)),
        ] {
            if n.dot(to_eye) <= 0.05 {
                continue;
            }
            // The face's color from its day shading: ambient 0.6 + headlight 0.4 * cosine.
            let day_shade = 0.6 + 0.4 * n.dot(to_eye);
            let shade =
                0.25 + 0.75 * lambert(n, key) + 0.35 * lambert(n, fill) + 0.5 * lambert(n, rim);
            let (got, seen) = (studio.at(p), day.at(p));
            for k in 0..3 {
                let expected = (seen[k] / day_shade * shade).min(255.0);
                assert!(
                    (got[k] - expected).abs() <= 4.0,
                    "eye {eye:?} face {n}: {got:?} vs expected {expected} (day {seen:?})"
                );
            }
            checked += 1;
        }
        assert!(checked >= 2, "{checked} faces checked");
    }
}

fn backdrop() -> StyleSettings {
    StyleSettings {
        background: [200, 120, 40],
        ..StyleSettings::default()
    }
}

#[test]
fn the_backdrop_is_the_style_background_with_no_ground() {
    let Some(mut snap) = gpu() else { return };
    let mut d = doc();
    look(&mut d, [0.0, -6.0, 1.5], [0.0, 0.0, 1.5]);
    let mut style = backdrop();
    style.ground = Some(kernel::Ground {
        color: [255, 0, 0],
        opacity: 1.0,
        from_below: true,
    });
    let shot = shoot(&mut snap, &d, Some(&style), None, Lighting::Studio);
    // The upper half is above the horizon; below it the ground quad of the view is drawn as in
    // the day (but not the style's ground color).
    for (x, y) in [(1, 1), (W - 2, 1), (1, H / 3), (W - 2, H / 3)] {
        let p = shot.pixel(x, y);
        assert!(
            (0..3).all(|k| (p[k] - f64::from(style.background[k])).abs() <= 1.0),
            "({x}, {y}) is {p:?}"
        );
    }
}

#[test]
fn a_sky_makes_the_backdrop_a_vertical_gradient_from_the_background() {
    let Some(mut snap) = gpu() else { return };
    let mut d = doc();
    look(&mut d, [0.0, -6.0, 1.5], [0.0, 0.0, 1.5]);
    let mut style = backdrop();
    style.sky = Some([20, 40, 220]);
    let shot = shoot(&mut snap, &d, Some(&style), None, Lighting::Studio);
    let near = |p: [f64; 3], c: [u8; 3]| (0..3).all(|k| (p[k] - f64::from(c[k])).abs() <= 3.0);
    assert!(
        near(shot.pixel(1, 0), [20, 40, 220]),
        "{:?}",
        shot.pixel(1, 0)
    );
    // Halfway down the view the backdrop is halfway from the sky to the background, and the
    // same at both sides: a gradient down the screen, not a horizon.
    let row = H / 3;
    let t = (f64::from(row) + 0.5) / f64::from(H);
    let expected =
        [0, 1, 2].map(|k| [20.0, 40.0, 220.0][k] * (1.0 - t) + f64::from(style.background[k]) * t);
    for x in [1, W - 2] {
        let p = shot.pixel(x, row);
        assert!(
            (0..3).all(|k| (p[k] - expected[k]).abs() <= 4.0),
            "{p:?} vs {expected:?}"
        );
    }
}

/// A thin 3 m post at the origin and a camera above and to the side.
fn post() -> Document {
    let mut d = doc();
    let base = run(
        &mut d,
        Command::AddRectangle {
            corner: [-0.1, -0.1, 0.0],
            opposite: [0.1, 0.1, 0.0],
            normal: None,
        },
    );
    crate::golden::raise(&mut d, crate::golden::created_face(&base), 3.0);
    look(&mut d, [4.0, -6.0, 6.0], [0.0, 0.0, 0.5]);
    d
}

fn day_sun(direction: Vec3) -> SunLight {
    SunLight {
        direction,
        light: 0.8,
        dark: 0.45,
        on_ground: true,
    }
}

#[test]
fn the_key_light_casts_the_shadow_instead_of_the_sun() {
    let Some(mut snap) = gpu() else { return };
    let d = post();
    let key = toward(&d.camera, KEY);
    assert!(key.z > 0.3, "{key}");
    // A sun from elsewhere (east, high): its shadow would fall west of the post.
    let sun = Some(day_sun(Vec3::new(0.6, 0.0, 0.8)));
    let shadowed = shoot(&mut snap, &d, None, sun, Lighting::Studio);
    let unshadowed = shoot(&mut snap, &d, None, None, Lighting::Studio);
    // The key's shadow runs from the post's foot away from the key, 3 m high over key.z.
    let run_out = DVec3::new(-key.x, -key.y, 0.0) / key.z;
    let in_key_shadow = run_out * 1.5;
    let in_sun_shadow = DVec3::new(-0.6, 0.0, 0.0) / 0.8 * 1.5;
    let ratio = 0.25 / (0.25 + 0.75 * key.z);
    let (off, on) = (unshadowed.at(in_key_shadow), shadowed.at(in_key_shadow));
    for k in 0..3 {
        assert!(
            (on[k] - off[k] * ratio).abs() <= 12.0,
            "key shadow {on:?} vs unshadowed {off:?} * {ratio}"
        );
    }
    let (off, on) = (unshadowed.at(in_sun_shadow), shadowed.at(in_sun_shadow));
    assert_eq!(on, off, "no shadow of the sun");
}

#[test]
fn the_key_map_is_drawn_again_only_when_the_camera_moves() {
    let Some(mut snap) = gpu() else { return };
    let mut d = post();
    let sun = Some(day_sun(Vec3::new(0.6, 0.0, 0.8)));
    shoot(&mut snap, &d, None, sun, Lighting::Studio);
    shoot(&mut snap, &d, None, sun, Lighting::Studio);
    assert_eq!(snap.sun_maps_drawn(), 1);
    look(&mut d, [-4.0, -6.0, 6.0], [0.0, 0.0, 0.5]);
    shoot(&mut snap, &d, None, sun, Lighting::Studio);
    assert_eq!(snap.sun_maps_drawn(), 2);
    // Without shadows there is no map.
    let mut plain = gpu().unwrap();
    shoot(&mut plain, &d, None, None, Lighting::Studio);
    assert_eq!(plain.sun_maps_drawn(), 0);
}
