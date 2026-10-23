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

#[test]
fn an_edge_carries_the_normals_of_its_two_faces_and_its_other_end() {
    let base = edge_extras(&split_box(), &profiles(4.0));
    assert_eq!(base.edge_extras.len(), base.edges.len());
    for (pair, extras) in base.edges.chunks(2).zip(base.edge_extras.chunks(2)) {
        assert_eq!(extras[0].other, pair[1].pos);
        assert_eq!(extras[1].other, pair[0].pos);
        assert_eq!(extras[0].normal_a, extras[1].normal_a);
        assert_eq!(extras[0].normal_b, extras[1].normal_b);
        // A closed box: every edge has two faces, with unit normals.
        for n in [extras[0].normal_a, extras[0].normal_b] {
            assert_eq!(n[3], 1.0, "present");
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4);
        }
    }
}

#[test]
fn a_free_edge_has_no_faces() {
    let mut m = Model::new();
    m.add_edge(DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)).unwrap();
    let base = edge_extras(&Scene::from_root(m), &profiles(4.0));
    assert_eq!(base.edge_extras.len(), 2);
    assert_eq!(base.edge_extras[0].normal_a[3], 0.0);
    assert_eq!(base.edge_extras[0].normal_b[3], 0.0);
}

#[test]
fn the_normals_follow_a_mirroring_placement() {
    // A component of the box, mirrored along the red axis: the normals are those of the
    // placed faces, so the left face's normal points right-handed after the flip.
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
    run(
        &mut d,
        Command::Flip {
            ids: Some(vec![instance]),
            axis: session::FlipAxis::Red,
            frame: session::FlipFrame::default(),
        },
    );
    let base = edge_extras(d.scene(), &profiles(4.0));
    // Every normal is a unit vector along an axis, and each pair of an edge's two faces is
    // perpendicular (a box stays a box), mirrored or not.
    assert_eq!(base.edges.len(), 2 * 12);
    for extras in base.edge_extras.chunks(2) {
        let (a, b) = (extras[0].normal_a, extras[0].normal_b);
        let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert!(dot.abs() < 1e-4, "{a:?} {b:?}");
    }
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
