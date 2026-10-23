//! Model styles in the renderer: edge colors (scene building), and the background, sky,
//! ground and fog (pixels).

use std::collections::BTreeSet;

use glam::{DAffine3, DMat3, DVec2, DVec3};
use kernel::{EdgeColorMode, EdgeStyle, Fog, FogColor, Ground, Model, Scene, StyleSettings};
use render::Snapshotter;
use render::grid::{AXIS_X, AXIS_Y, AXIS_Z, NIGHT_BACKGROUND};
use render::scene::{EDGE_COLOR, EdgeColors, FADED_EDGE_COLOR, GlassView, scene_base_styled};
use session::{Camera, Command, Lighting, Projection, SectionDisplay, Side};

use super::golden::{add_box, add_material, all_faces, doc, run, top_face, write_png};

/// The edge ends the scene draws, with their colors.
fn edges(
    scene: &Scene,
    context: &[kernel::InstanceId],
    colors: &EdgeColors,
) -> Vec<([f32; 3], [f32; 4])> {
    let base = scene_base_styled(
        scene,
        context,
        false,
        GlassView::default(),
        SectionDisplay::default(),
        colors,
    );
    base.edges.iter().map(|v| (v.pos, v.color)).collect()
}

fn mode(mode: EdgeColorMode) -> EdgeColors {
    EdgeColors::new(
        &EdgeStyle {
            color_mode: mode,
            color: [255, 128, 0],
            ..EdgeStyle::default()
        },
        DAffine3::IDENTITY,
    )
}

const ORANGE: [f32; 4] = [1.0, 128.0 / 255.0, 0.0, 1.0];

/// Four edges from the origin: along x, y and z, and a diagonal.
fn four_edges() -> Scene {
    let mut m = Model::new();
    let o = DVec3::ZERO;
    m.add_edge(o, DVec3::new(2.0, 0.0, 0.0)).unwrap();
    m.add_edge(o, DVec3::new(0.0, 2.0, 0.0)).unwrap();
    m.add_edge(o, DVec3::new(0.0, 0.0, 2.0)).unwrap();
    m.add_edge(o, DVec3::new(1.0, 1.0, 0.0)).unwrap();
    Scene::from_root(m)
}

/// The color of each edge (both ends are alike).
fn colors_of(list: &[([f32; 3], [f32; 4])]) -> Vec<[f32; 4]> {
    list.chunks(2).map(|pair| pair[0].1).collect()
}

#[test]
fn default_edge_colors_are_the_classic_edge_color() {
    let list = edges(&four_edges(), &[], &EdgeColors::default());
    assert_eq!(list.len(), 8);
    assert!(list.iter().all(|(_, c)| *c == EDGE_COLOR));
}

#[test]
fn fixed_edge_color_replaces_the_default() {
    let list = edges(&four_edges(), &[], &mode(EdgeColorMode::Fixed));
    assert!(list.iter().all(|(_, c)| *c == ORANGE), "{list:?}");
}

#[test]
fn edges_by_axis_take_the_axis_colors() {
    let list = edges(&four_edges(), &[], &mode(EdgeColorMode::ByAxis));
    assert_eq!(colors_of(&list), [AXIS_X, AXIS_Y, AXIS_Z, ORANGE]);
}

#[test]
fn edges_by_axis_follow_the_drawing_axes() {
    // Axes turned 30 degrees about z: the edge along world x is no longer along an axis.
    let turned = DAffine3::from_mat3(DMat3::from_rotation_z(30f64.to_radians()));
    let colors = EdgeColors::new(
        &EdgeStyle {
            color_mode: EdgeColorMode::ByAxis,
            color: [255, 128, 0],
            ..EdgeStyle::default()
        },
        turned,
    );
    let mut m = Model::new();
    let o = DVec3::ZERO;
    m.add_edge(o, DVec3::new(2.0, 0.0, 0.0)).unwrap();
    m.add_edge(o, turned.transform_vector3(DVec3::X) * 2.0)
        .unwrap();
    m.add_edge(o, DVec3::new(0.0, 0.0, 2.0)).unwrap();
    let list = edges(&Scene::from_root(m), &[], &colors);
    assert_eq!(colors_of(&list), [ORANGE, AXIS_X, AXIS_Z]);
}

#[test]
fn edges_by_axis_allow_a_small_angle_only() {
    let mut m = Model::new();
    // 0.5 degrees off x, then 5 degrees off x.
    let off = |deg: f64| DVec3::new(deg.to_radians().cos(), deg.to_radians().sin(), 0.0) * 2.0;
    m.add_edge(DVec3::ZERO, off(0.5)).unwrap();
    let up = DVec3::new(0.0, 0.0, 1.0);
    m.add_edge(up, up + off(5.0)).unwrap();
    let list = edges(&Scene::from_root(m), &[], &mode(EdgeColorMode::ByAxis));
    assert_eq!(colors_of(&list), [AXIS_X, ORANGE]);
}

#[test]
fn edges_by_axis_see_the_placement_of_a_component() {
    // A group turned 45 degrees about z holds an edge along its own x: not along world x.
    let mut d = doc();
    run(
        &mut d,
        Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [2.0, 0.0, 0.0],
        },
    );
    let edge = d
        .model()
        .edges()
        .map(|(id, _)| kernel::EntityId::Edge(id))
        .collect::<Vec<_>>();
    run(&mut d, Command::MakeGroup { ids: edge });
    let group = d
        .model()
        .instances()
        .map(|(id, _)| kernel::EntityId::Instance(id))
        .next()
        .expect("a group");
    run(
        &mut d,
        Command::Rotate {
            ids: vec![group],
            center: [0.0, 0.0, 0.0],
            axis: None,
            angle: 45.0,
            copies: None,
        },
    );
    let list = edges(d.scene(), &[], &mode(EdgeColorMode::ByAxis));
    assert!(list.iter().all(|(_, c)| *c == ORANGE), "{list:?}");
}

#[test]
fn edges_by_material_take_the_front_color_of_the_first_shown_face() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let brick = add_material(&mut d, "Brick", [190, 70, 50]);
    let floor = top_face(&d, 0.0);
    run(
        &mut d,
        Command::Paint {
            ids: vec![floor],
            material: Some(brick),
            side: Side::Front,
        },
    );
    let list = edges(d.scene(), &[], &mode(EdgeColorMode::ByMaterial));
    let brick_color = [190.0 / 255.0, 70.0 / 255.0, 50.0 / 255.0, 1.0];
    let brick_ends = list.iter().filter(|(_, c)| *c == brick_color).count();
    let fixed_ends = list.iter().filter(|(_, c)| *c == ORANGE).count();
    assert_eq!(brick_ends + fixed_ends, list.len(), "only those two colors");
    // The floor's four edges bound a wall each; a vertical edge touches no floor.
    assert!(brick_ends > 0, "an edge takes the painted face's color");
    assert!(
        fixed_ends > 0,
        "edges not on the painted face keep the fixed color"
    );
    // Every edge not at z = 0 on both ends (the verticals and the top) is on no painted face.
    assert!(
        list.chunks(2)
            .filter(|p| p[0].0[2] > 0.5 && p[1].0[2] > 0.5)
            .all(|p| p[0].1 == ORANGE),
        "the top's edges"
    );
}

#[test]
fn an_edge_on_no_painted_face_keeps_the_fixed_color() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let list = edges(d.scene(), &[], &mode(EdgeColorMode::ByMaterial));
    assert!(list.iter().all(|(_, c)| *c == ORANGE));
    // A free edge has no face at all.
    let list = edges(&four_edges(), &[], &mode(EdgeColorMode::ByMaterial));
    assert!(list.iter().all(|(_, c)| *c == ORANGE));
}

#[test]
fn edges_by_material_use_a_texture_average() {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("style-texture-{}", std::process::id()))
        .join("two.png");
    // Two pixels: red 200 and black; the average is red 100.
    write_png(&path, &[200, 0, 0, 255, 0, 0, 0, 255], 2, 1);
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        },
    );
    let tex = run(
        &mut d,
        Command::AddMaterial {
            name: Some("Two".into()),
            color: Some([10, 10, 10]),
            opacity: None,
            texture: Some(session::TextureSource {
                path: Some(path.display().to_string()),
                width: Some(1.0),
                height: Some(1.0),
            }),
            emission: None,
        },
    )
    .changes
    .materials[0];
    let faces = all_faces(&d);
    run(
        &mut d,
        Command::Paint {
            ids: faces,
            material: Some(tex),
            side: Side::Both,
        },
    );
    let list = edges(d.scene(), &[], &mode(EdgeColorMode::ByMaterial));
    let want = [100.0 / 255.0, 0.0, 0.0, 1.0];
    assert!(
        list.iter()
            .all(|(_, c)| (0..4).all(|k| (c[k] - want[k]).abs() < 0.01)),
        "{list:?}"
    );
}

#[test]
fn edges_outside_the_open_context_stay_faded() {
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [1.0, 1.0, 0.0],
            normal: None,
        },
    );
    let all = all_faces(&d);
    run(&mut d, Command::MakeGroup { ids: all });
    let group = d
        .model()
        .instances()
        .map(|(id, _)| id)
        .next()
        .expect("a group");
    run(
        &mut d,
        Command::AddRectangle {
            corner: [5.0, 5.0, 0.0],
            opposite: [6.0, 6.0, 0.0],
            normal: None,
        },
    );
    for colors in [mode(EdgeColorMode::Fixed), mode(EdgeColorMode::ByAxis)] {
        let list = edges(d.scene(), &[group], &colors);
        // The group's square is in the context, the other outside it, then the context box.
        assert!(list.iter().any(|(_, c)| *c == FADED_EDGE_COLOR));
        assert!(list.iter().any(|(_, c)| *c == ORANGE || *c == AXIS_X));
    }
}

// ---- pixels ----

const W: u32 = 64;
const H: u32 = 48;

fn settings() -> StyleSettings {
    StyleSettings {
        background: [200, 220, 40],
        ..StyleSettings::default()
    }
}

fn level_camera() -> Camera {
    let mut c = Camera::new(DVec2::new(W as f64, H as f64));
    c.eye = DVec3::new(0.0, -6.0, 1.5);
    c.target = DVec3::new(0.0, 0.0, 1.5);
    c
}

struct Shot(Vec<u8>);

impl Shot {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * W + x) * 4) as usize;
        [self.0[i], self.0[i + 1], self.0[i + 2]]
    }

    /// The most pixels of any one row in `rows` within 2 of `color`: a grid line may run along
    /// a row, so a single row can be off.
    fn row_matches(&self, rows: std::ops::Range<u32>, color: [u8; 3]) -> u32 {
        rows.map(|y| (0..W).filter(|&x| near(self.at(x, y), color, 2)).count() as u32)
            .max()
            .unwrap_or(0)
    }

    /// How many pixels of `rows` are within 2 of `color`. The grid's far lines run together near
    /// the horizon, and a software rasterizer (lavapipe, WARP) loses the near ones in the middle
    /// of the view, so only the corners of the lower half show the canvas there.
    fn count_in(&self, rows: std::ops::Range<u32>, color: [u8; 3]) -> u32 {
        rows.map(|y| (0..W).filter(|&x| near(self.at(x, y), color, 2)).count() as u32)
            .sum()
    }

    /// The most common color of the image.
    fn dominant(&self) -> [u8; 3] {
        let mut counts = std::collections::HashMap::new();
        for k in 0..W * H {
            *counts.entry(self.at(k % W, k / W)).or_insert(0u32) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .expect("pixels")
            .0
    }

    /// How many pixels of the whole image are within 2 of `color`.
    fn matches(&self, color: [u8; 3]) -> u32 {
        (0..H).map(|y| self.row_matches(y..y + 1, color)).sum()
    }
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    (0..3).all(|k| a[k].abs_diff(b[k]) <= tolerance)
}

/// Renders `scene` with `style` (None: the default look), or None without a GPU.
fn shoot(
    scene: &Scene,
    style: Option<&StyleSettings>,
    camera: &Camera,
    lighting: Lighting,
    print: bool,
) -> Option<Shot> {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    snap.set_model_style(style);
    snap.set_print_mode(print);
    snap.capture(scene, &[], &BTreeSet::new(), camera, lighting, W, H)
        .map(Shot)
}

fn nothing() -> Scene {
    Scene::default()
}

#[test]
fn the_style_background_replaces_the_theme_background() {
    let s = settings();
    let Some(shot) = shoot(&nothing(), Some(&s), &level_camera(), Lighting::Day, false) else {
        return;
    };
    assert!(near(shot.at(1, 1), s.background, 1), "{:?}", shot.at(1, 1));
    // Without a style it is the classic background.
    let classic = shoot(&nothing(), None, &level_camera(), Lighting::Day, false).unwrap();
    assert_ne!(classic.at(1, 1), shot.at(1, 1));
}

#[test]
fn night_and_print_ignore_the_style_background() {
    let mut s = settings();
    s.sky = Some([0, 0, 255]);
    s.ground = Some(Ground {
        color: [255, 0, 0],
        opacity: 1.0,
        from_below: true,
    });
    let night = [
        NIGHT_BACKGROUND[0],
        NIGHT_BACKGROUND[1],
        NIGHT_BACKGROUND[2],
    ]
    .map(|c| (c * 255.0).round() as u8);
    let Some(shot) = shoot(
        &nothing(),
        Some(&s),
        &level_camera(),
        Lighting::Night,
        false,
    ) else {
        return;
    };
    assert!(
        shot.row_matches(0..8, night) >= W / 2,
        "{:?}",
        shot.at(1, 0)
    );
    assert!(shot.count_in(H / 2..H, night) >= W / 2);
    assert_eq!(shot.matches([255, 0, 0]), 0, "no ground");
    let print = shoot(&nothing(), Some(&s), &level_camera(), Lighting::Day, true).unwrap();
    for y in [0, H - 1] {
        assert_eq!(print.at(1, y), [255, 255, 255], "print {y}");
    }
}

fn sky_and_ground() -> StyleSettings {
    StyleSettings {
        background: [200, 200, 200],
        sky: Some([40, 80, 220]),
        ground: Some(Ground {
            color: [60, 160, 60],
            opacity: 1.0,
            from_below: false,
        }),
        ..StyleSettings::default()
    }
}

#[test]
fn the_sky_runs_from_the_background_at_the_horizon_to_the_sky_color_above() {
    let s = sky_and_ground();
    let Some(shot) = shoot(&nothing(), Some(&s), &level_camera(), Lighting::Day, false) else {
        return;
    };
    // The horizon is the middle row: just above it, nearly the background; at the top, bluer.
    let near_horizon = shot.at(1, H / 2 - 2);
    let top = shot.at(1, 0);
    assert!(near(near_horizon, s.background, 40), "{near_horizon:?}");
    assert!(top[2] > top[0] + 20, "bluer at the top: {top:?}");
    assert!(top[0] < near_horizon[0], "darker toward the zenith");
    // Monotonic: each row up is at least as far from the background.
    let reds: Vec<u8> = (0..H / 2).map(|y| shot.at(1, y)[0]).collect();
    assert!(reds.windows(2).all(|w| w[0] <= w[1] + 1), "{reds:?}");
}

#[test]
fn the_ground_covers_the_view_below_the_horizon() {
    let s = sky_and_ground();
    let Some(shot) = shoot(&nothing(), Some(&s), &level_camera(), Lighting::Day, false) else {
        return;
    };
    // Below the horizon is the ground color wherever the grid's lines leave it, and above the
    // horizon there is none.
    let ground = s.ground.unwrap().color;
    assert!(
        shot.count_in(H / 2..H, ground) >= W / 2,
        "{:?}",
        shot.at(1, H - 1)
    );
    assert_eq!(shot.count_in(0..H / 2 - 2, ground), 0);
}

#[test]
fn the_ground_mixes_over_the_background_by_its_opacity() {
    let mut s = sky_and_ground();
    s.ground.as_mut().unwrap().opacity = 0.5;
    let Some(shot) = shoot(&nothing(), Some(&s), &level_camera(), Lighting::Day, false) else {
        return;
    };
    let want = [130, 180, 130];
    assert!(
        shot.count_in(H / 2..H, want) >= W / 2,
        "{:?}",
        shot.at(1, H - 1)
    );
    assert_eq!(shot.count_in(0..H / 2 - 2, want), 0);
}

#[test]
fn the_ground_is_seen_from_below_only_when_asked() {
    // Below the ground, looking up: the ground plane is above the horizon.
    let mut camera = level_camera();
    camera.eye.z = -1.5;
    camera.target = DVec3::new(0.0, 0.0, -0.5);
    let mut s = sky_and_ground();
    let ground = s.ground.unwrap().color;
    let Some(without) = shoot(&nothing(), Some(&s), &camera, Lighting::Day, false) else {
        return;
    };
    assert_eq!(without.matches(ground), 0, "no ground from below");
    s.ground.as_mut().unwrap().from_below = true;
    let with = shoot(&nothing(), Some(&s), &camera, Lighting::Day, false).unwrap();
    assert!(
        with.row_matches(0..8, ground) >= W / 2,
        "{:?}",
        with.at(1, 0)
    );
    // Below the horizon there is no ground plane to see: the background.
    assert!(with.row_matches(H - 8..H, s.background) >= W / 2);
}

#[test]
fn a_parallel_top_view_is_all_ground() {
    let s = sky_and_ground();
    let mut camera = Camera::new(DVec2::new(W as f64, H as f64));
    camera.eye = DVec3::new(0.0, 0.0, 20.0);
    camera.target = DVec3::ZERO;
    camera.up = DVec3::Y;
    camera.projection = Projection::Parallel;
    let Some(shot) = shoot(&nothing(), Some(&s), &camera, Lighting::Day, false) else {
        return;
    };
    // The grid's lines cross many pixels; most of the rest is ground.
    assert_eq!(shot.dominant(), s.ground.unwrap().color);
}

#[test]
fn a_parallel_side_view_has_a_straight_horizon() {
    let s = sky_and_ground();
    let mut camera = level_camera();
    camera.projection = Projection::Parallel;
    let Some(shot) = shoot(&nothing(), Some(&s), &camera, Lighting::Day, false) else {
        return;
    };
    // Every pixel of the top row is alike, and bluish; the bottom row is the ground.
    assert_eq!(shot.at(1, 0), shot.at(W - 2, 0));
    assert!(
        shot.at(1, 0)[2] > shot.at(1, 0)[0] + 20,
        "{:?}",
        shot.at(1, 0)
    );
    assert!(shot.row_matches(H - 8..H, s.ground.unwrap().color) >= W / 2);
    // The eye and target are 1.5 m up: the ground plane's row is below the middle, at
    // 24 + 24 * 1.5 / (6 * tan(17.5 degrees)) = about 43. Above it is no ground.
    let ground = s.ground.unwrap().color;
    for y in H / 2 + 2..H / 2 + 14 {
        assert_eq!(
            shot.row_matches(y..y + 1, ground),
            0,
            "row {y} is above the ground"
        );
    }
}

#[test]
fn a_parallel_view_has_no_eye_height_below_the_ground() {
    // Level, with the target 1.5 m below the ground: the ground plane's row is about 19 rows
    // above the middle and everything under it is ground; the arbitrary height of a parallel
    // view's eye does not make this a view from below.
    let s = sky_and_ground();
    let mut camera = level_camera();
    camera.projection = Projection::Parallel;
    camera.eye.z = -1.5;
    camera.target.z = -1.5;
    let Some(shot) = shoot(&nothing(), Some(&s), &camera, Lighting::Day, false) else {
        return;
    };
    let ground = s.ground.unwrap().color;
    assert!(shot.row_matches(H / 2..H, ground) >= W / 2);
    assert_eq!(
        shot.row_matches(0..2, ground),
        0,
        "sky above the plane's row"
    );
}

/// A square of ground 8 m across.
fn ground_face() -> Scene {
    let mut m = Model::new();
    let c = [(-4.0, -4.0), (4.0, -4.0), (4.0, 4.0), (-4.0, 4.0)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        m.add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    Scene::from_root(m)
}

fn looking_down_from(distance: f64) -> Camera {
    let mut c = Camera::new(DVec2::new(W as f64, H as f64));
    c.eye = DVec3::new(0.0, 0.0, distance);
    c.target = DVec3::ZERO;
    c.up = DVec3::Y;
    c
}

fn fogged(start: f32, end: f32, color: FogColor) -> StyleSettings {
    StyleSettings {
        background: [255, 255, 255],
        fog: Some(Fog { start, end, color }),
        ..StyleSettings::default()
    }
}

fn without_fog(mut style: StyleSettings) -> StyleSettings {
    style.fog = None;
    style
}

/// A pixel on the ground face of `looking_down_from`, away from its edges.
const ON_FACE: (u32, u32) = (W / 2 + 3, H / 2 + 3);

#[test]
fn a_face_beyond_the_fog_end_is_the_fog_color() {
    let style = fogged(1.0, 10.0, FogColor::Rgb([255, 0, 255]));
    let camera = looking_down_from(30.0);
    let Some(shot) = shoot(&ground_face(), Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    let plain = shoot(
        &ground_face(),
        Some(&without_fog(style)),
        &camera,
        Lighting::Day,
        false,
    )
    .unwrap();
    let (x, y) = ON_FACE;
    assert!(near(shot.at(x, y), [255, 0, 255], 2), "{:?}", shot.at(x, y));
    assert!(
        !near(plain.at(x, y), [255, 0, 255], 40),
        "no fog: {:?}",
        plain.at(x, y)
    );
}

#[test]
fn a_face_before_the_fog_start_is_untouched() {
    let style = fogged(40.0, 80.0, FogColor::Rgb([255, 0, 255]));
    let camera = looking_down_from(30.0);
    let Some(shot) = shoot(&ground_face(), Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    let plain = shoot(
        &ground_face(),
        Some(&without_fog(style)),
        &camera,
        Lighting::Day,
        false,
    )
    .unwrap();
    assert_eq!(shot.0, plain.0);
}

#[test]
fn fog_fades_in_between_start_and_end() {
    // The ground at 30 m, fog from 20 to 40 m: smoothstep 0.5 there, halfway to black.
    let style = fogged(20.0, 40.0, FogColor::Rgb([0, 0, 0]));
    let camera = looking_down_from(30.0);
    let Some(shot) = shoot(&ground_face(), Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    let plain = shoot(
        &ground_face(),
        Some(&without_fog(style)),
        &camera,
        Lighting::Day,
        false,
    )
    .unwrap();
    let (x, y) = ON_FACE;
    let (a, b) = (shot.at(x, y)[0] as f32, plain.at(x, y)[0] as f32);
    assert!((a - b * 0.5).abs() < 12.0, "fogged {a}, plain {b}");
}

#[test]
fn fog_in_the_background_color_fades_to_the_background() {
    let mut style = fogged(1.0, 10.0, FogColor::Background);
    style.background = [250, 100, 50];
    let camera = looking_down_from(30.0);
    let Some(shot) = shoot(&ground_face(), Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    assert!(near(shot.at(ON_FACE.0, ON_FACE.1), [250, 100, 50], 2));
    // The whole face (where no grid shows) is the background color.
    for (x, y) in [(W / 2 - 6, H / 2 - 6), (W / 2 + 6, H / 2 + 6)] {
        assert!(
            near(shot.at(x, y), [250, 100, 50], 2),
            "{:?}",
            shot.at(x, y)
        );
    }
}

#[test]
fn lines_fade_into_fog_too() {
    // Edges away from the axes, 30 m from the eye.
    let mut m = Model::new();
    m.add_edge(DVec3::new(1.0, 2.0, 0.0), DVec3::new(4.0, 3.0, 0.0))
        .unwrap();
    m.add_edge(DVec3::new(1.0, 3.0, 0.0), DVec3::new(4.0, 2.0, 0.0))
        .unwrap();
    let scene = Scene::from_root(m);
    let camera = looking_down_from(30.0);
    // Pixels of the fog color (mostly magenta over white), and dark ones (an edge as it is).
    let count = |style: &StyleSettings| {
        let shot = shoot(&scene, Some(style), &camera, Lighting::Day, false)?;
        let all = (0..W * H).map(|k| shot.at(k % W, k / W));
        Some((
            all.clone()
                .filter(|p| p[0] > 200 && p[1] < 100 && p[2] > 200)
                .count(),
            all.filter(|p| p[0] < 90 && p[1] < 90 && p[2] < 90).count(),
        ))
    };
    let style = fogged(1.0, 10.0, FogColor::Rgb([255, 0, 255]));
    let Some((plain_fog, plain_dark)) = count(&without_fog(style)) else {
        return;
    };
    let (fog_fog, fog_dark) = count(&style).unwrap();
    assert_eq!((plain_fog, fog_dark), (0, 0));
    assert!(plain_dark > 0, "an edge is dark without fog");
    assert!(fog_fog > 0, "a fogged edge is the fog color");
}

#[test]
fn fog_works_in_a_parallel_view() {
    let style = fogged(1.0, 10.0, FogColor::Rgb([255, 0, 255]));
    let mut camera = looking_down_from(30.0);
    camera.projection = Projection::Parallel;
    let Some(shot) = shoot(&ground_face(), Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    let (x, y) = ON_FACE;
    assert!(near(shot.at(x, y), [255, 0, 255], 2), "{:?}", shot.at(x, y));
}

#[test]
fn print_ignores_fog() {
    let style = fogged(1.0, 10.0, FogColor::Rgb([255, 0, 255]));
    let camera = looking_down_from(30.0);
    let Some(shot) = shoot(&ground_face(), Some(&style), &camera, Lighting::Day, true) else {
        return;
    };
    assert!(!near(shot.at(ON_FACE.0, ON_FACE.1), [255, 0, 255], 60));
}

#[test]
fn the_default_settings_look_like_no_style_but_for_the_background() {
    // Edges, faces and the grid are as without a style, with the style's background set to
    // the classic one.
    let camera = looking_down_from(8.0);
    let s = StyleSettings {
        background: [245, 245, 245],
        ..StyleSettings::default()
    };
    let (Some(a), Some(b)) = (
        shoot(&ground_face(), Some(&s), &camera, Lighting::Day, false),
        shoot(&ground_face(), None, &camera, Lighting::Day, false),
    ) else {
        return;
    };
    let differing = (0..W * H)
        .filter(|&k| !near(a.at(k % W, k / W), b.at(k % W, k / W), 8))
        .count();
    assert_eq!(differing, 0);
}

/// A long edge along x passing 2 m in front of the eye, its ends about 40 m away.
fn long_edge_near_the_eye() -> (Scene, Camera) {
    let mut m = Model::new();
    m.add_edge(DVec3::new(-40.0, 2.0, 1.5), DVec3::new(40.0, 2.0, 1.5))
        .unwrap();
    let mut camera = Camera::new(DVec2::new(W as f64, H as f64));
    camera.eye = DVec3::new(0.0, 0.0, 1.5);
    camera.target = DVec3::new(0.0, 10.0, 1.5);
    (Scene::from_root(m), camera)
}

#[test]
fn a_long_edge_is_fogged_by_each_pixels_distance() {
    let (scene, camera) = long_edge_near_the_eye();
    // Fog from 5 m: the edge's middle, 2 m away, is clear; its far ends are fogged. Distances
    // interpolated from the ends would fog the middle too.
    let style = fogged(5.0, 10.0, FogColor::Rgb([255, 0, 255]));
    let Some(shot) = shoot(&scene, Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    let dark = (0..W * H)
        .map(|k| shot.at(k % W, k / W))
        .filter(|p| p[0] < 170 && p[1] < 170 && p[2] < 170)
        .count();
    assert!(dark > 0, "the middle of the edge is unfogged");
}

#[test]
fn the_grid_and_axes_are_fogged_too() {
    let mut style = fogged(1.0, 10.0, FogColor::Background);
    style.background = [250, 100, 50];
    let camera = looking_down_from(30.0);
    let Some(shot) = shoot(&nothing(), Some(&style), &camera, Lighting::Day, false) else {
        return;
    };
    assert!(shot.matches([250, 100, 50]) >= W * H * 98 / 100);
}

#[test]
fn selected_faces_and_edges_are_not_fogged() {
    let style = fogged(1.0, 10.0, FogColor::Rgb([255, 0, 255]));
    let camera = looking_down_from(30.0);
    let scene = ground_face();
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    snap.set_model_style(Some(&style));
    let faces: BTreeSet<kernel::EntityId> = scene
        .mesh(kernel::MeshId::Root)
        .unwrap()
        .faces()
        .map(|(id, _)| kernel::EntityId::Face(id))
        .collect();
    let px = snap
        .capture(&scene, &[], &faces, &camera, Lighting::Day, W, H)
        .unwrap();
    let i = ((ON_FACE.1 * W + ON_FACE.0) * 4) as usize;
    let selected = [px[i], px[i + 1], px[i + 2]];
    assert!(!near(selected, [255, 0, 255], 60), "{selected:?}");
    // Selected edges: blue pixels (the selection color) remain.
    let edges: BTreeSet<kernel::EntityId> = scene
        .mesh(kernel::MeshId::Root)
        .unwrap()
        .edges()
        .map(|(id, _)| kernel::EntityId::Edge(id))
        .collect();
    let px = snap
        .capture(&scene, &[], &edges, &camera, Lighting::Day, W, H)
        .unwrap();
    let blue = px
        .chunks(4)
        .filter(|p| p[0] < 100 && p[1] < 150 && p[2] > 200)
        .count();
    assert!(blue > 0, "selected edges keep their color in fog");
}
