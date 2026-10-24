//! Sun shading and the sun's shadow map: shading levels, shadows on the ground and on faces,
//! no acne or peter-panning, what casts, the map kept between frames, night and the horizon.

use glam::{DVec2, DVec3, Vec3};
use render::grid::BACKGROUND;
use render::{Snapshotter, SunLight};
use session::{Camera, Command, Document, Lighting, Side};

use crate::golden::{add_box, all_faces, doc, look, raise, run, top_face};

const W: u32 = 320;
const H: u32 = 240;

/// The unit vector toward a sun at `azimuth` (degrees clockwise from +y) and `elevation`.
fn toward(azimuth: f32, elevation: f32) -> Vec3 {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(e.cos() * a.sin(), e.cos() * a.cos(), e.sin())
}

fn sun(direction: Vec3, light: f32, dark: f32) -> SunLight {
    SunLight {
        direction,
        light,
        dark,
        on_ground: true,
    }
}

/// The sun from the west at 45 degrees: shadows fall east by the height of what casts them.
fn west() -> Vec3 {
    toward(270.0, 45.0)
}

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
    fn at(&self, p: [f64; 3]) -> [i32; 3] {
        let s = self
            .camera
            .project(DVec3::from(p))
            .expect("in front of the camera");
        let i = ((s.y as u32 * W + s.x as u32) * 4) as usize;
        [0, 1, 2].map(|k| i32::from(self.pixels[i + k]))
    }

    /// The brightest pixel within a pixel of `p`.
    fn brightest_near(&self, p: [f64; 3]) -> i32 {
        let s = self
            .camera
            .project(DVec3::from(p))
            .expect("in front of the camera");
        let (x, y) = (s.x as i32, s.y as i32);
        (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| (x + dx, y + dy)))
            .map(|(x, y)| {
                let i = ((y as u32 * W + x as u32) * 4) as usize;
                (0..3).map(|k| i32::from(self.pixels[i + k])).sum::<i32>()
            })
            .max()
            .unwrap()
    }

    fn brightness(&self, p: [f64; 3]) -> i32 {
        self.at(p).iter().sum()
    }
}

fn shoot(snap: &mut Snapshotter, d: &Document, sun: Option<SunLight>, lighting: Lighting) -> Shot {
    snap.set_sun(sun);
    snap.set_glass_view(render::scene::GlassView {
        x_ray: d.x_ray,
        hidden_geometry: d.hidden_geometry,
    });
    snap.set_section_display(d.section_display);
    let pixels = snap
        .capture(
            d.scene(),
            d.context(),
            d.selection(),
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

fn background() -> [i32; 3] {
    [0, 1, 2].map(|k| (BACKGROUND[k] * 255.0).round() as i32)
}

fn close(a: [i32; 3], b: [i32; 3], tolerance: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
}

fn scaled(c: [i32; 3], by: f32) -> [i32; 3] {
    c.map(|v| (v as f32 * by).round() as i32)
}

/// A 2 x 1.5 x 1 m box at the origin seen from above, and its top and south face centers.
fn one_box() -> Document {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    look(&mut d, [3.0, -4.0, 5.0], [1.0, 0.75, 0.5]);
    d
}
const TOP: [f64; 3] = [1.0, 0.75, 1.0];
const SOUTH: [f64; 3] = [1.0, 0.0, 0.5];

#[test]
fn faces_take_the_dark_level_plus_the_light_level_by_the_angle_to_the_sun() {
    let Some(mut snap) = gpu() else { return };
    let d = one_box();
    let up = Vec3::Z;
    let mut color = |light, dark| shoot(&mut snap, &d, Some(sun(up, light, dark)), Lighting::Day);
    // Full dark level and no light shows each face's color as it is.
    let full = color(0.0, 1.0);
    let half = color(0.0, 0.5);
    for face in [TOP, SOUTH] {
        assert!(
            close(half.at(face), scaled(full.at(face), 0.5), 3),
            "{face:?}"
        );
    }
    // The sun overhead lights the top square-on and the walls not at all.
    let lit = color(1.0, 0.0);
    assert!(
        close(lit.at(TOP), full.at(TOP), 3),
        "top {:?} {:?}",
        lit.at(TOP),
        full.at(TOP)
    );
    let walls = color(1.0, 0.5);
    assert!(close(walls.at(SOUTH), scaled(full.at(SOUTH), 0.5), 3));
    // And it is not the headlight: the same view without the sun shades differently.
    let headlight = shoot(&mut snap, &d, None, Lighting::Day);
    assert!(!close(headlight.at(SOUTH), half.at(SOUTH), 6));
}

#[test]
fn a_box_shades_the_ground_where_the_sun_does_not_reach() {
    let Some(mut snap) = gpu() else { return };
    let d = one_box();
    let (light, dark) = (0.8, 0.45);
    let shot = shoot(&mut snap, &d, Some(sun(west(), light, dark)), Lighting::Day);
    let shadowed = [2.5, 0.75, 0.0];
    let sunny = [4.6, 0.75, 0.0];
    assert!(
        close(shot.at(sunny), background(), 2),
        "{:?}",
        shot.at(sunny)
    );
    // The ground in shadow keeps the dark level of the light it has in the sun.
    let ratio = dark / (dark + light * 45f32.to_radians().sin());
    let expected = scaled(background(), ratio);
    assert!(
        close(shot.at(shadowed), expected, 10),
        "{:?} vs {expected:?}",
        shot.at(shadowed)
    );
    // Off the ground there is no shadow drawn there.
    let mut off = sun(west(), light, dark);
    off.on_ground = false;
    let shot = shoot(&mut snap, &d, Some(off), Lighting::Day);
    assert!(
        close(shot.at(shadowed), background(), 2),
        "{:?}",
        shot.at(shadowed)
    );
}

#[test]
fn the_shadow_starts_at_the_foot_of_the_box() {
    let Some(mut snap) = gpu() else { return };
    let d = {
        let mut d = doc();
        add_box(&mut d, [0.0, 0.0]);
        look(&mut d, [6.0, -5.0, 4.0], [1.0, 0.75, 0.5]);
        d
    };
    // Sun from the west at 60 degrees: the shadow reaches 0.58 m past the east wall; 3 cm off the wall is
    // a few texels, so a light gap there would be peter-panning.
    let shot = shoot(
        &mut snap,
        &d,
        Some(sun(toward(270.0, 60.0), 0.8, 0.45)),
        Lighting::Day,
    );
    let foot = shot.brightness([2.03, 0.75, 0.0]);
    let sunny = shot.brightness([3.5, 0.75, 0.0]);
    assert!(
        foot * 10 < sunny * 6,
        "foot {foot}, sunny {sunny}: no peter-panning"
    );
    // The lit top is even: acne would stripe it.
    let samples: Vec<i32> = [0.3, 0.8, 1.3, 1.7]
        .iter()
        .flat_map(|&x| [0.3, 0.75, 1.2].map(|y| shot.brightness([x, y, 1.0])))
        .collect();
    let (lo, hi) = (samples.iter().min().unwrap(), samples.iter().max().unwrap());
    assert!(hi - lo <= 6, "top face varies {lo}..{hi}: {samples:?}");
}

/// A 3 m wall box at the origin and a 1 m box east of it; returns the document looking from
/// above at the low box's top, which the wall's shadow covers from x = 3 to 4 of 3 to 5.
fn wall_and_box() -> Document {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let top = top_face(&d, 1.0);
    raise(&mut d, top, 2.0);
    add_box(&mut d, [3.0, 0.0]);
    look(&mut d, [3.5, -6.0, 8.0], [3.5, 0.75, 1.0]);
    d
}

#[test]
fn a_shadow_falls_on_the_faces_it_reaches() {
    let Some(mut snap) = gpu() else { return };
    let d = wall_and_box();
    let (light, dark) = (0.8, 0.45);
    let shot = shoot(&mut snap, &d, Some(sun(west(), light, dark)), Lighting::Day);
    let dark_only = shoot(&mut snap, &d, Some(sun(west(), 0.0, dark)), Lighting::Day);
    let shadowed = [3.4, 0.75, 1.0];
    let sunny = [4.6, 0.75, 1.0];
    assert!(
        close(shot.at(shadowed), dark_only.at(shadowed), 4),
        "{:?}",
        shot.at(shadowed)
    );
    assert!(shot.brightness(sunny) > dark_only.brightness(sunny) + 60);
}

#[test]
fn the_shadow_ends_where_the_wall_shadow_ends() {
    let Some(mut snap) = gpu() else { return };
    let d = wall_and_box();
    let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    // The edge lies at x = 4 on the top of the low box, to within a few texels and the filter.
    let inside = shot.brightness([3.8, 0.75, 1.0]);
    let outside = shot.brightness([4.2, 0.75, 1.0]);
    assert!(outside > inside + 60, "inside {inside}, outside {outside}");
}

/// Paints every face made so far with a material of `opacity`.
fn paint_all(d: &mut Document, opacity: f32) {
    let made = run(
        d,
        Command::AddMaterial {
            name: Some("Pane".into()),
            color: Some([200, 200, 255]),
            opacity: Some(opacity),
            texture: None,
            emission: None,
            metalness: None,
            roughness: None,
            existing: Default::default(),
        },
    );
    let material = made.changes.materials[0];
    let faces = all_faces(d);
    run(
        d,
        Command::Paint {
            ids: faces,
            material: Some(material),
            side: Side::Both,
        },
    );
}

#[test]
fn only_nearly_opaque_faces_cast_the_sun() {
    let Some(mut snap) = gpu() else { return };
    let shadowed = [2.5, 0.75, 0.0];
    for (opacity, casts) in [(0.5, false), (0.69, false), (0.75, true), (1.0, true)] {
        let mut d = doc();
        add_box(&mut d, [0.0, 0.0]);
        paint_all(&mut d, opacity);
        look(&mut d, [3.0, -6.0, 5.0], [1.0, 0.75, 0.5]);
        let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
        let dark = shot.brightness(shadowed) < shot.brightness([4.6, 0.75, 0.0]) * 8 / 10;
        assert_eq!(dark, casts, "opacity {opacity}");
    }
}

#[test]
fn ghosts_and_hidden_faces_do_not_cast() {
    let Some(mut snap) = gpu() else { return };
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    run(&mut d, Command::Hide { ids: faces });
    look(&mut d, [3.0, -6.0, 5.0], [1.0, 0.75, 0.5]);
    let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    assert!(close(shot.at([2.5, 0.75, 0.0]), background(), 2));
}

#[test]
fn the_map_is_drawn_again_only_when_the_sun_or_the_casters_change() {
    let Some(mut snap) = gpu() else { return };
    let mut d = one_box();
    let drawn = |snap: &Snapshotter| snap.sun_maps_drawn();
    shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    assert_eq!(drawn(&snap), 1);
    // The camera moving, or the levels changing, leaves the map as it is.
    for k in 0..3 {
        look(&mut d, [3.0 + f64::from(k), -4.0, 5.0], [1.0, 0.75, 0.5]);
        shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    }
    shoot(&mut snap, &d, Some(sun(west(), 0.5, 0.3)), Lighting::Day);
    assert_eq!(drawn(&snap), 1);
    // Another sun draws it again; the same one again does not.
    shoot(
        &mut snap,
        &d,
        Some(sun(toward(250.0, 40.0), 0.8, 0.45)),
        Lighting::Day,
    );
    shoot(
        &mut snap,
        &d,
        Some(sun(toward(250.0, 40.0), 0.8, 0.45)),
        Lighting::Day,
    );
    assert_eq!(drawn(&snap), 2);
    // So do other casters.
    add_box(&mut d, [4.0, 0.0]);
    shoot(
        &mut snap,
        &d,
        Some(sun(toward(250.0, 40.0), 0.8, 0.45)),
        Lighting::Day,
    );
    assert_eq!(drawn(&snap), 3);
}

#[test]
fn the_sun_casters_are_held_only_while_the_sun_shades() {
    let Some(mut snap) = gpu() else { return };
    let d = one_box();
    // Without a sun, nothing is uploaded for it, however the model changes.
    shoot(&mut snap, &d, None, Lighting::Day);
    assert_eq!(snap.sun_caster_bytes(), 0);
    // Turning the sun on after the scene was prepared still shades and shadows.
    let lit = sun(west(), 0.8, 0.45);
    snap.set_sun(Some(lit));
    let shot = snap
        .capture_prepared(&d.camera, Lighting::Day, W, H)
        .unwrap();
    assert!(snap.sun_caster_bytes() > 0);
    assert_eq!(snap.sun_maps_drawn(), 1);
    let reference = shoot(&mut snap, &d, Some(lit), Lighting::Day);
    assert_eq!(shot, reference.pixels);
    // Turning it off drops them; on again draws the map again.
    snap.set_sun(None);
    assert_eq!(snap.sun_caster_bytes(), 0);
    let again = shoot(&mut snap, &d, Some(lit), Lighting::Day);
    assert_eq!(again.pixels, reference.pixels);
    assert_eq!(snap.sun_maps_drawn(), 2);
    // Off and on again with the scene untouched, as the window does: the shadow returns.
    snap.set_sun(None);
    snap.set_sun(Some(lit));
    let back = snap
        .capture_prepared(&d.camera, Lighting::Day, W, H)
        .unwrap();
    assert_eq!(back, reference.pixels);
    assert_eq!(snap.sun_maps_drawn(), 3);
}

#[test]
fn night_and_no_sun_draw_no_map_and_night_ignores_the_sun() {
    let Some(mut snap) = gpu() else { return };
    let d = one_box();
    shoot(&mut snap, &d, None, Lighting::Day);
    let night = shoot(&mut snap, &d, None, Lighting::Night);
    let night_sun = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Night);
    assert_eq!(night.pixels, night_sun.pixels);
    assert_eq!(snap.sun_maps_drawn(), 0);
}

#[test]
fn a_sun_below_the_horizon_leaves_the_dark_level_and_no_shadows() {
    let Some(mut snap) = gpu() else { return };
    let d = one_box();
    let set = Vec3::new(-0.8, 0.0, -0.3).normalize();
    let full = shoot(&mut snap, &d, Some(sun(Vec3::Z, 0.0, 1.0)), Lighting::Day);
    let shot = shoot(&mut snap, &d, Some(sun(set, 0.8, 0.5)), Lighting::Day);
    for face in [TOP, SOUTH] {
        assert!(
            close(shot.at(face), scaled(full.at(face), 0.5), 3),
            "{face:?}"
        );
    }
    assert!(close(shot.at([2.5, 0.75, 0.0]), background(), 2));
}

#[test]
fn the_ground_shows_no_shadow_to_a_camera_below_it() {
    let Some(mut snap) = gpu() else { return };
    let mut d = one_box();
    look(&mut d, [3.0, -4.0, -3.0], [1.0, 0.75, 0.0]);
    // Compared with the same view without the sun: a grid line may cross the sampled pixel.
    let plain = shoot(&mut snap, &d, None, Lighting::Day);
    let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    let at = [2.5, 0.75, 0.0];
    assert!(
        close(shot.at(at), plain.at(at), 2),
        "{:?} {:?}",
        shot.at(at),
        plain.at(at)
    );
}

#[test]
fn the_sun_of_a_document_follows_its_shadow_settings() {
    let mut d = doc();
    assert_eq!(SunLight::of(&d), None, "shadows are off by default");
    run(
        &mut d,
        Command::SetShadows {
            on: Some(true),
            date: None,
            time: None,
            light: Some(70),
            dark: Some(30),
            on_ground: Some(false),
        },
    );
    let sun = SunLight::of(&d).expect("shadows on");
    assert_eq!((sun.light, sun.dark, sun.on_ground), (0.7, 0.3, false));
    assert!(sun.is_up(), "Boulder at 13:30 in November");
    let want = d.sun().direction.as_vec3();
    assert!((sun.direction - want).length() < 1e-6);
}

#[test]
fn grid_lines_in_a_shadow_darken_with_the_ground() {
    let Some(mut snap) = gpu() else { return };
    let d = one_box();
    let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    // The 1 m grid line y = 1 crosses the shadow (x from 2 to 3): no pixel near it may be
    // lighter than the shadowed ground (about 0.44 of the background), as it was when the
    // grid was drawn over the shadow.
    let bright: i32 = background().iter().sum();
    let in_shadow = shot.brightest_near([2.5, 1.0, 0.0]);
    assert!(
        in_shadow * 10 < bright * 6,
        "grid in shadow {in_shadow}, background {bright}"
    );
}

/// Whether the ground at (2.5, 0.75) is in the box's shadow.
fn shadowed(shot: &Shot) -> bool {
    shot.brightness([2.5, 0.75, 0.0]) * 10 < shot.brightness([4.6, 0.75, 0.0]) * 8
}

#[test]
fn ghosts_under_hidden_geometry_do_not_cast() {
    let Some(mut snap) = gpu() else { return };
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    run(&mut d, Command::Hide { ids: faces });
    run(&mut d, Command::SetHiddenGeometry { on: true });
    look(&mut d, [3.0, -6.0, 5.0], [1.0, 0.75, 0.5]);
    let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    assert!(!shadowed(&shot));
}

#[test]
fn x_ray_does_not_change_what_casts() {
    let Some(mut snap) = gpu() else { return };
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(&mut d, Command::SetXRay { on: true });
    look(&mut d, [3.0, -6.0, 5.0], [1.0, 0.75, 0.5]);
    let shot = shoot(&mut snap, &d, Some(sun(west(), 0.8, 0.45)), Lighting::Day);
    assert!(shadowed(&shot));
}

#[test]
fn what_a_section_plane_cuts_away_does_not_cast() {
    let Some(mut snap) = gpu() else { return };
    // The east wall (x = 2) and its shadow, with the plane at x = 1 keeping one half.
    let cut = |normal: [f64; 3]| {
        let mut d = doc();
        add_box(&mut d, [0.0, 0.0]);
        run(
            &mut d,
            Command::AddSectionPlane {
                point: [1.0, 0.0, 0.0],
                normal,
                activate: Some(true),
            },
        );
        look(&mut d, [3.0, -6.0, 5.0], [1.0, 0.75, 0.5]);
        d
    };
    let both: Vec<bool> = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]]
        .map(|n| {
            shadowed(&shoot(
                &mut snap,
                &cut(n),
                Some(sun(west(), 0.8, 0.45)),
                Lighting::Day,
            ))
        })
        .to_vec();
    // One side keeps the east half, whose wall still shades x = 2.5; the other keeps the west
    // half, whose shadow ends at x = 2.
    assert_eq!(both.iter().filter(|&&s| s).count(), 1, "{both:?}");
}
