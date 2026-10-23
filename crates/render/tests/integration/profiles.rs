//! Profile, sketchy and endpoint edges: what the edge buffers carry (scene building) and what
//! the pixels show.

use std::collections::BTreeSet;
use std::ops::Range;

use glam::{DVec2, DVec3};
use kernel::{EdgeStyle, Model, Scene, Sketchy, StyleSettings};
use render::Snapshotter;
use render::scene::{EdgeColors, GlassView, scene_base_styled};
use session::{Camera, Command, Lighting, Projection, SectionDisplay};

use super::golden::{add_box, created_face, doc, raise, run};

const W: u32 = 128;
const H: u32 = 96;

fn style(edges: EdgeStyle) -> StyleSettings {
    StyleSettings {
        edges,
        ..StyleSettings::default()
    }
}

fn profiles(width: f32) -> StyleSettings {
    style(EdgeStyle {
        profiles: Some(width),
        ..EdgeStyle::default()
    })
}

/// The box of `add_box` (x 2-4, y 1-2.5, z 0-1, off the axes) with an inner edge up the middle of
/// its front face (y = 1), which splits that face in two coplanar faces.
fn split_box() -> Scene {
    let mut d = doc();
    add_box(&mut d, [2.0, 1.0]);
    run(
        &mut d,
        Command::AddEdge {
            start: [3.0, 1.0, 0.0],
            end: [3.0, 1.0, 1.0],
        },
    );
    d.scene().clone()
}

fn camera(eye: [f64; 3], target: [f64; 3], projection: Projection) -> Camera {
    let mut c = Camera::new(DVec2::new(W as f64, H as f64));
    c.eye = DVec3::from(eye);
    c.target = DVec3::from(target);
    c.projection = projection;
    if projection == Projection::Parallel {
        // Size the view like the perspective one at that distance.
        c.fov_y = 35f64.to_radians();
    }
    c
}

/// Looking at the front face of the box, level with its middle.
fn front(projection: Projection) -> Camera {
    camera([3.0, -5.0, 0.5], [3.0, 1.0, 0.5], projection)
}

struct Shot(Vec<u8>);

impl Shot {
    fn lum(&self, x: u32, y: u32) -> u32 {
        let i = ((y * W + x) * 4) as usize;
        (u32::from(self.0[i]) + u32::from(self.0[i + 1]) + u32::from(self.0[i + 2])) / 3
    }

    fn dark(&self, x: u32, y: u32) -> bool {
        self.lum(x, y) < 160
    }

    /// The runs of dark pixels along row `y`.
    fn row_runs(&self, y: u32) -> Vec<Range<u32>> {
        runs((0..W).map(|x| self.dark(x, y)))
    }

    /// The runs of dark pixels down column `x`, above row `limit` (below the box the red axis
    /// crosses).
    fn column_runs(&self, x: u32, limit: u32) -> Vec<Range<u32>> {
        runs((0..limit).map(|y| self.dark(x, y)))
    }
}

fn runs(dark: impl Iterator<Item = bool>) -> Vec<Range<u32>> {
    let mut out: Vec<Range<u32>> = Vec::new();
    let mut start = None;
    let mut n = 0;
    for (k, d) in dark.enumerate() {
        n = k as u32 + 1;
        match (d, start) {
            (true, None) => start = Some(k as u32),
            (false, Some(s)) => {
                out.push(s..k as u32);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(s..n);
    }
    out
}

fn len(r: &Range<u32>) -> u32 {
    r.end - r.start
}

fn snapshotter(style: Option<&StyleSettings>) -> Option<Snapshotter> {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    snap.set_model_style(style);
    Some(snap)
}

fn shoot(scene: &Scene, style: Option<&StyleSettings>, camera: &Camera) -> Option<Shot> {
    let mut snap = snapshotter(style)?;
    let shot = snap
        .capture(scene, &[], &BTreeSet::new(), camera, Lighting::Day, W, H)
        .map(Shot);
    shot.inspect(dump)
}

/// Writes the image to the folder named by DUMP_DIR, when it is set, to look at.
fn dump(shot: &Shot) {
    if let Some(dir) = std::env::var_os("DUMP_DIR") {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::path::Path::new(&dir).join(format!("{n}.png"));
        super::golden::write_png(&path, &shot.0, W, H);
    }
}

fn edge_extras(scene: &Scene, style: &StyleSettings) -> render::scene::SceneBase {
    scene_base_styled(
        scene,
        &[],
        false,
        GlassView::default(),
        SectionDisplay::default(),
        &EdgeColors::new(&style.edges, glam::DAffine3::IDENTITY),
    )
}

// ---- what the buffers carry ----

#[test]
fn edges_carry_no_extras_without_edge_effects() {
    let base = edge_extras(&split_box(), &StyleSettings::default());
    assert!(base.edge_extras.is_empty());
    assert_eq!(base.edges.len(), 2 * 15);
}

#[test]
fn soft_edges_are_in_the_buffer_only_with_profiles() {
    let mut d = doc();
    let circle = run(
        &mut d,
        Command::AddCircle {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
            normal: None,
            segments: Some(12),
        },
    );
    raise(&mut d, created_face(&circle), 1.0);
    let off = edge_extras(d.scene(), &StyleSettings::default());
    let on = edge_extras(d.scene(), &profiles(4.0));
    // The two rims; with profiles the 12 swept soft edges too.
    assert_eq!(off.edges.len(), 2 * 24);
    assert_eq!(on.edges.len(), 2 * 36);
    assert_eq!(on.edge_extras.len(), on.edges.len());
    let soft = on.edge_extras.iter().filter(|e| e.meta[0] == 2.0).count();
    assert_eq!(soft, 2 * 12);
}

/// Checks every edge's side directions against the box the edges bound: each lies in a face of
/// the box (the point a little way into it from the edge's middle is on the box's surface),
/// leads away from the edge, and is a unit vector.
fn assert_sides_lie_in_the_box(base: &render::scene::SceneBase) {
    let all: Vec<glam::Vec3> = base.edges.iter().map(|v| glam::Vec3::from(v.pos)).collect();
    let lo = all.iter().copied().reduce(glam::Vec3::min).unwrap();
    let hi = all.iter().copied().reduce(glam::Vec3::max).unwrap();
    assert_eq!(base.edge_extras.len(), base.edges.len());
    for (pair, extras) in base.edges.chunks(2).zip(base.edge_extras.chunks(2)) {
        let (a, b) = (glam::Vec3::from(pair[0].pos), glam::Vec3::from(pair[1].pos));
        assert_eq!(extras[0].other, pair[1].pos);
        assert_eq!(extras[1].other, pair[0].pos);
        assert_eq!(extras[0].side_a, extras[1].side_a);
        assert_eq!(extras[0].side_b, extras[1].side_b);
        for side in [extras[0].side_a, extras[0].side_b] {
            assert_eq!(side[3], 1.0, "a box edge has two faces");
            let d = glam::Vec3::new(side[0], side[1], side[2]);
            assert!((d.length() - 1.0).abs() < 1e-4, "unit: {d:?}");
            assert!(
                d.dot(b - a).abs() < 1e-4 * (b - a).length().max(1.0),
                "across the edge"
            );
            let p = (a + b) / 2.0 + 0.05 * d;
            let outside = (p.cmplt(lo - 1e-4).any()) || p.cmpgt(hi + 1e-4).any();
            assert!(!outside, "{p:?} leaves the box {lo:?} {hi:?}");
            let on_surface =
                (0..3).any(|k| (p[k] - lo[k]).abs() < 1e-4 || (p[k] - hi[k]).abs() < 1e-4);
            assert!(on_surface, "{p:?} is inside the box, not in a face");
        }
    }
}

#[test]
fn an_edge_carries_the_directions_into_its_two_faces_and_its_other_end() {
    assert_sides_lie_in_the_box(&edge_extras(&split_box(), &profiles(4.0)));
}

#[test]
fn a_free_edge_has_no_faces() {
    let mut m = Model::new();
    m.add_edge(DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)).unwrap();
    let base = edge_extras(&Scene::from_root(m), &profiles(4.0));
    assert_eq!(base.edge_extras.len(), 2);
    assert_eq!(base.edge_extras[0].side_a[3], 0.0);
    assert_eq!(base.edge_extras[0].side_b[3], 0.0);
}

/// The box of `add_box` as a component, with `commands` applied to its instance.
fn placed_box(commands: impl FnOnce(kernel::EntityId) -> Vec<Command>) -> Scene {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = super::golden::all_faces(&d);
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: faces,
            name: "Box".into(),
        },
    );
    let instance = *made
        .changes
        .created
        .iter()
        .find(|e| matches!(e, kernel::EntityId::Instance(_)))
        .expect("an instance");
    for command in commands(instance) {
        run(&mut d, command);
    }
    d.scene().clone()
}

#[test]
fn the_side_directions_follow_a_mirroring_placement() {
    let scene = placed_box(|instance| {
        vec![Command::Flip {
            ids: Some(vec![instance]),
            axis: session::FlipAxis::Red,
            frame: session::FlipFrame::default(),
        }]
    });
    assert_sides_lie_in_the_box(&edge_extras(&scene, &profiles(4.0)));
}

#[test]
fn the_side_directions_follow_a_turned_and_stretched_placement() {
    let scene = placed_box(|instance| {
        vec![
            Command::Scale {
                ids: vec![instance],
                origin: [0.0, 0.0, 0.0],
                factors: [1.0, 0.5, 1.5],
            },
            Command::Rotate {
                ids: vec![instance],
                center: [0.0, 0.0, 0.0],
                axis: Some([0.0, 0.0, 1.0]),
                angle: 30.0,
                copies: None,
            },
        ]
    });
    // The turned box is not axis aligned, so look at the directions in its own frame: every
    // side is across its edge and the point just inside the face is still on the box's faces.
    let base = edge_extras(&scene, &profiles(4.0));
    for (pair, extras) in base.edges.chunks(2).zip(base.edge_extras.chunks(2)) {
        let along = glam::Vec3::from(pair[1].pos) - glam::Vec3::from(pair[0].pos);
        for side in [extras[0].side_a, extras[0].side_b] {
            let d = glam::Vec3::new(side[0], side[1], side[2]);
            assert!(
                d.dot(along).abs() < 1e-3 * along.length(),
                "across the edge"
            );
        }
    }
    // Rotated back by the same turn, the sides must lie in the stretched box's faces.
    let turn = glam::Mat3::from_rotation_z(-30f32.to_radians());
    let mut unturned = edge_extras(&scene, &profiles(4.0));
    for v in &mut unturned.edges {
        v.pos = (turn * glam::Vec3::from(v.pos)).to_array();
    }
    for e in &mut unturned.edge_extras {
        e.other = (turn * glam::Vec3::from(e.other)).to_array();
        for side in [&mut e.side_a, &mut e.side_b] {
            let d = turn * glam::Vec3::new(side[0], side[1], side[2]);
            *side = [d.x, d.y, d.z, side[3]];
        }
    }
    // Directions are unit vectors only up to the placement's stretch: normalize happened
    // in the builder, so the check is on the point 5 cm in.
    assert_sides_lie_in_the_box(&unturned);
}

#[test]
fn sketchy_values_differ_between_edges_and_repeat_between_builds() {
    let sketchy = style(EdgeStyle {
        sketchy: Some(Sketchy {
            jitter: 4.0,
            extension: 10.0,
        }),
        ..EdgeStyle::default()
    });
    let a = edge_extras(&split_box(), &sketchy);
    let b = edge_extras(&split_box(), &sketchy);
    assert_eq!(a.edge_extras, b.edge_extras);
    let jitters: BTreeSet<u32> = a.edge_extras.iter().map(|e| e.meta[1].to_bits()).collect();
    assert!(jitters.len() >= 10, "{} distinct", jitters.len());
    for e in &a.edge_extras {
        assert!((-1.0..=1.0).contains(&e.meta[1]));
        assert!((0.0..=1.0).contains(&e.meta[2]) && (0.0..=1.0).contains(&e.meta[3]));
    }
}

// ---- pixels ----

#[test]
fn a_silhouette_is_thicker_than_an_inner_edge() {
    let scene = split_box();
    let wide = profiles(8.0);
    for projection in [Projection::Perspective, Projection::Parallel] {
        let camera = front(projection);
        let Some(with) = shoot(&scene, Some(&wide), &camera) else {
            return;
        };
        let plain = shoot(&scene, None, &camera).unwrap();
        let row = with.row_runs(H / 2 - 4);
        assert_eq!(
            row.len(),
            3,
            "{projection:?}: left, inner and right: {row:?}"
        );
        assert!(len(&row[0]) >= 6, "{projection:?}: left {row:?}");
        assert!(len(&row[2]) >= 6, "{projection:?}: right {row:?}");
        assert!(len(&row[1]) <= 3, "{projection:?}: inner {row:?}");
        let flat = plain.row_runs(H / 2 - 4);
        assert!(
            flat.iter().all(|r| len(r) <= 3),
            "{projection:?}: plain {flat:?}"
        );
    }
}

#[test]
fn orbiting_changes_which_edges_are_profiles_without_a_rebuild() {
    let scene = split_box();
    let wide = profiles(8.0);
    let Some(mut snap) = snapshotter(Some(&wide)) else {
        return;
    };
    snap.prepare(&scene, &[], &BTreeSet::new());
    let mut shot = |eye: [f64; 3]| {
        let c = camera(eye, [3.0, 1.75, 0.5], Projection::Perspective);
        let shot = Shot(
            snap.capture_prepared(&c, Lighting::Day, W, H)
                .expect("pixels"),
        );
        dump(&shot);
        shot
    };
    // Below the top face and the bottom face seen from above: the front's top and bottom edges
    // are both silhouettes.
    let level = shot([3.0, -2.5, 0.5]);
    let column = level.column_runs(W / 2 - 14, 76);
    assert_eq!(column.len(), 2, "{column:?}");
    assert!(column.iter().all(|r| len(r) >= 6), "{column:?}");
    // From above, the top face shows: its back edge is the silhouette, the front-top edge turns
    // inner (thin) and the front-bottom one stays a silhouette.
    let above = shot([3.0, -2.5, 2.5]);
    let column = above.column_runs(W / 2 - 14, H);
    assert_eq!(column.len(), 3, "{column:?}");
    // (The top face, near-grazing, hides the inner part of the back edge's width.)
    assert!(len(&column[0]) >= 5 && len(&column[2]) >= 5, "{column:?}");
    assert!(len(&column[1]) <= 3, "{column:?}");
    // And back again, from the same upload.
    let again = shot([3.0, -2.5, 0.5]);
    assert_eq!(again.0, level.0);
}

#[test]
fn a_cylinders_soft_silhouette_is_drawn_with_profiles_only() {
    let mut d = doc();
    let circle = run(
        &mut d,
        Command::AddCircle {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
            normal: None,
            segments: Some(24),
        },
    );
    raise(&mut d, created_face(&circle), 1.5);
    let camera = camera([0.3, -6.0, 0.75], [0.0, 0.0, 0.75], Projection::Perspective);
    let Some(with) = shoot(d.scene(), Some(&profiles(8.0)), &camera) else {
        return;
    };
    let without = shoot(d.scene(), None, &camera).unwrap();
    assert!(
        without.row_runs(H / 2).is_empty(),
        "soft edges are hidden: {:?}",
        without.row_runs(H / 2)
    );
    // The left and right silhouettes only: the soft edges across the front stay hidden.
    let row = with.row_runs(H / 2);
    assert_eq!(row.len(), 2, "{row:?}");
    assert!(row.iter().all(|r| len(r) >= 5), "{row:?}");
}

#[test]
fn show_edges_off_hides_all_but_profiles() {
    let scene = split_box();
    let camera = front(Projection::Perspective);
    let hidden = style(EdgeStyle {
        show_edges: false,
        ..EdgeStyle::default()
    });
    let Some(none) = shoot(&scene, Some(&hidden), &camera) else {
        return;
    };
    assert!(none.row_runs(H / 2 - 4).is_empty());
    let only = style(EdgeStyle {
        show_edges: false,
        profiles: Some(8.0),
        ..EdgeStyle::default()
    });
    let shot = shoot(&scene, Some(&only), &camera).unwrap();
    let row = shot.row_runs(H / 2 - 4);
    assert_eq!(row.len(), 2, "the silhouette only: {row:?}");
}

fn one_edge() -> Scene {
    let mut m = Model::new();
    m.add_edge(DVec3::new(-1.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0))
        .unwrap();
    Scene::from_root(m)
}

fn edge_camera() -> Camera {
    camera([0.0, -6.0, 0.0], [0.0, 0.0, 0.0], Projection::Parallel)
}

#[test]
fn sketchy_jitter_is_stable_between_frames() {
    let sketchy = style(EdgeStyle {
        sketchy: Some(Sketchy {
            jitter: 6.0,
            extension: 0.0,
        }),
        ..EdgeStyle::default()
    });
    let scene = split_box();
    let camera = front(Projection::Perspective);
    let Some(mut snap) = snapshotter(Some(&sketchy)) else {
        return;
    };
    snap.prepare(&scene, &[], &BTreeSet::new());
    let a = snap.capture_prepared(&camera, Lighting::Day, W, H).unwrap();
    let b = snap.capture_prepared(&camera, Lighting::Day, W, H).unwrap();
    assert_eq!(a, b);
    let plain = shoot(&scene, None, &camera).unwrap();
    assert_ne!(a, plain.0, "the edges moved");
}

#[test]
fn sketchy_extension_lengthens_both_ends() {
    let camera = edge_camera();
    let Some(plain) = shoot(&one_edge(), None, &camera) else {
        return;
    };
    let sketchy = style(EdgeStyle {
        sketchy: Some(Sketchy {
            jitter: 0.0,
            extension: 20.0,
        }),
        ..EdgeStyle::default()
    });
    let longer = shoot(&one_edge(), Some(&sketchy), &camera).unwrap();
    let row = |s: &Shot| {
        (0..H)
            .map(|y| s.row_runs(y))
            .max_by_key(|r| r.iter().map(len).sum::<u32>())
            .unwrap()
    };
    let (a, b) = (row(&plain), row(&longer));
    assert_eq!((a.len(), b.len()), (1, 1), "{a:?} {b:?}");
    // Each end grows by between 0.3 and 1 times the extension.
    let (growth, grown_left) = (len(&b[0]) - len(&a[0]), a[0].start - b[0].start);
    assert!((12..=42).contains(&growth), "{a:?} {b:?}");
    assert!(grown_left >= 6, "the left end moved out: {a:?} {b:?}");
}

#[test]
fn endpoints_mark_each_end_and_not_the_middle() {
    let camera = edge_camera();
    let Some(plain) = shoot(&one_edge(), None, &camera) else {
        return;
    };
    let marked = style(EdgeStyle {
        endpoints: Some(12.0),
        ..EdgeStyle::default()
    });
    let with = shoot(&one_edge(), Some(&marked), &camera).unwrap();
    let y = (0..H)
        .find(|&y| !plain.row_runs(y).is_empty())
        .expect("the edge");
    let line = plain.row_runs(y).remove(0);
    // 3 px wide marks grow upward from the line: rows y-2 and y-1 are dark near the ends only.
    let above = |s: &Shot, x: u32| s.dark(x, y - 1) || s.dark(x, y - 2);
    let near_left = line.start + 4;
    let near_right = line.end - 5;
    let middle = (line.start + line.end) / 2;
    assert!(above(&with, near_left), "left mark");
    assert!(above(&with, near_right), "right mark");
    assert!(!above(&with, middle), "no mark mid-edge");
    assert!(!above(&plain, near_left), "no marks without endpoints");
}

// ---- the effects in use ----

fn doc_split_box() -> session::Document {
    let mut d = doc();
    add_box(&mut d, [2.0, 1.0]);
    run(
        &mut d,
        Command::AddEdge {
            start: [3.0, 1.0, 0.0],
            end: [3.0, 1.0, 1.0],
        },
    );
    d
}

#[test]
fn a_reversed_face_does_not_move_the_silhouette() {
    let mut d = doc_split_box();
    let top = super::golden::top_face(&d, 1.0);
    // Both sides pale, so the face reads the same whichever way it points.
    let paper = super::golden::add_material(&mut d, "Paper", [236, 236, 232]);
    run(
        &mut d,
        Command::Paint {
            ids: vec![top],
            material: Some(paper),
            side: kernel::Side::Both,
        },
    );
    let wide = profiles(8.0);
    let above = camera([3.0, -2.5, 2.5], [3.0, 1.75, 0.5], Projection::Perspective);
    let Some(plain) = shoot(d.scene(), Some(&wide), &above) else {
        return;
    };
    run(&mut d, Command::ReverseFaces { ids: vec![top] });
    let reversed = shoot(d.scene(), Some(&wide), &above).unwrap();
    let column = reversed.column_runs(W / 2 - 14, H);
    assert_eq!(column, plain.column_runs(W / 2 - 14, H), "{column:?}");
    assert_eq!(column.len(), 3, "{column:?}");
}

#[test]
fn a_selected_component_does_not_show_its_soft_edges() {
    let mut d = doc();
    let circle = run(
        &mut d,
        Command::AddCircle {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
            normal: None,
            segments: Some(24),
        },
    );
    raise(&mut d, created_face(&circle), 1.5);
    let faces = super::golden::all_faces(&d);
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: faces,
            name: "Can".into(),
        },
    );
    let instance = *made
        .changes
        .created
        .iter()
        .find(|e| matches!(e, kernel::EntityId::Instance(_)))
        .expect("an instance");
    let camera = camera([0.3, -6.0, 0.75], [0.0, 0.0, 0.75], Projection::Perspective);
    let Some(mut snap) = snapshotter(Some(&profiles(8.0))) else {
        return;
    };
    let selection = BTreeSet::from([instance]);
    let picked = Shot(
        snap.capture(d.scene(), &[], &selection, &camera, Lighting::Day, W, H)
            .unwrap(),
    );
    dump(&picked);
    // Selected edges are drawn in blue; none shows across the can's front, only its silhouette.
    let blue = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        u32::from(picked.0[i + 2]) > u32::from(picked.0[i]) + 100
    };
    let row: Vec<bool> = (0..W).map(|x| blue(x, H / 2)).collect();
    let runs = runs(row.into_iter());
    assert_eq!(runs.len(), 2, "the two silhouettes only: {runs:?}");
}

#[test]
fn edges_are_the_same_in_a_stretched_turned_group_and_loose() {
    let build = |grouped: bool| {
        let mut d = doc();
        add_box(&mut d, [2.0, 1.0]);
        let mut ids = super::golden::all_faces(&d);
        if grouped {
            let made = run(&mut d, Command::MakeGroup { ids: ids.clone() });
            ids = vec![
                *made
                    .changes
                    .created
                    .iter()
                    .find(|e| matches!(e, kernel::EntityId::Instance(_)))
                    .expect("an instance"),
            ];
        }
        run(
            &mut d,
            Command::Scale {
                ids: ids.clone(),
                origin: [3.0, 1.0, 0.0],
                factors: [1.0, 0.5, 1.5],
            },
        );
        run(
            &mut d,
            Command::Rotate {
                ids,
                center: [3.0, 1.0, 0.0],
                axis: Some([0.0, 0.0, 1.0]),
                angle: 30.0,
                copies: None,
            },
        );
        d
    };
    let camera = camera([3.0, -4.5, 2.0], [3.0, 1.0, 0.5], Projection::Perspective);
    let wide = profiles(6.0);
    let Some(loose) = shoot(build(false).scene(), Some(&wide), &camera) else {
        return;
    };
    let grouped = shoot(build(true).scene(), Some(&wide), &camera).unwrap();
    let plain = shoot(build(true).scene(), None, &camera).unwrap();
    let count = |s: &Shot| (0..W * H).filter(|k| s.dark(k % W, k / W)).count() as i64;
    assert!(
        (count(&loose) - count(&grouped)).abs() * 50 < count(&loose),
        "{} against {}",
        count(&loose),
        count(&grouped)
    );
    assert!(
        2 * count(&grouped) > 3 * count(&plain),
        "silhouettes are heavy: {} against {}",
        count(&grouped),
        count(&plain)
    );
}

#[test]
fn no_effects_allocate_no_edge_extras() {
    let scene = split_box();
    let Some(mut snap) = snapshotter(None) else {
        return;
    };
    let mut bytes = |style: Option<&StyleSettings>| {
        snap.set_model_style(style);
        snap.prepare(&scene, &[], &BTreeSet::new());
        snap.edge_extra_bytes()
    };
    assert_eq!(bytes(None), 0);
    assert_eq!(bytes(Some(&StyleSettings::default())), 0);
    assert!(bytes(Some(&profiles(4.0))) > 0);
    assert_eq!(bytes(Some(&StyleSettings::default())), 0, "and back again");
}
