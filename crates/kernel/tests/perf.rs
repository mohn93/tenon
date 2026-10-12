//! Timings for big edits. Ignored by default (they assert only loose bounds); run them in
//! release to see the numbers:
//! `cargo test --release -p kernel --test perf -- --ignored --nocapture`.

use std::collections::BTreeSet;
use std::f64::consts::TAU;
use std::time::{Duration, Instant};

use glam::DVec3;
use kernel::{DefinitionKind, EdgeId, FaceId, MeshId, MeshPolygon, Model, Scene, VertexId};

fn timed<T>(what: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    let took = t.elapsed();
    eprintln!("{what}: {took:?}");
    (out, took)
}

/// `count` circles with `sides` sides on the ground, six to a row 1 m apart, radius 0.4 m,
/// each a curve with its face.
fn circles(count: usize, sides: usize) -> Model {
    let mut m = Model::new();
    for k in 0..count {
        let c = DVec3::new((k % 6) as f64, (k / 6) as f64, 0.0);
        let pts: Vec<DVec3> = (0..sides)
            .map(|i| {
                let a = TAU * i as f64 / sides as f64;
                c + DVec3::new(a.cos(), a.sin(), 0.0) * 0.4
            })
            .collect();
        m.add_curve(&pts, true).unwrap();
    }
    m
}

/// A `n` x `n` terrain of bent quads (two triangles each, apart from a few flat ones).
fn terrain(n: usize) -> (Vec<DVec3>, Vec<MeshPolygon>) {
    let points: Vec<DVec3> = (0..=n)
        .flat_map(|y| {
            (0..=n).map(move |x| {
                let (x, y) = (x as f64, y as f64);
                DVec3::new(x, y, (x * 0.37).sin() * (y * 0.23).cos() * 2.0)
            })
        })
        .collect();
    let at = |x: usize, y: usize| y * (n + 1) + x;
    let polygons: Vec<MeshPolygon> = (0..n)
        .flat_map(|y| (0..n).map(move |x| (x, y)))
        .map(|(x, y)| MeshPolygon {
            corners: vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)],
            material: None,
        })
        .collect();
    (points, polygons)
}

fn everything(m: &Model) -> (Vec<EdgeId>, Vec<FaceId>) {
    (
        m.edges().map(|(id, _)| id).collect(),
        m.faces().map(|(id, _)| id).collect(),
    )
}

#[test]
#[ignore]
fn time_thirty_circles_copied_three_times() {
    let (mut m, _) = timed("draw 30 circles of 24 sides", || circles(30, 24));
    let (edges, faces) = everything(&m);
    let before = m.clone();
    let (copy, took) = timed("copy them 3 times", || {
        m.copy(&edges, &faces, DVec3::new(7.0, 0.0, 0.0), 3)
            .unwrap()
    });
    assert_eq!(m.faces().count(), 120);
    m.validate().unwrap();
    let after = m.clone();
    timed("undo the copy", || m.apply(&copy.inverse()));
    assert_eq!(m, before);
    timed("redo the copy", || m.apply(&copy));
    assert_eq!(m, after);
    let all: BTreeSet<VertexId> = m.vertices().map(|(id, _)| id).collect();
    timed("move all 2,880 vertices", || {
        m.move_vertices(&all, DVec3::new(0.0, 0.0, 1.0)).unwrap()
    });
    let (edges, _) = everything(&m);
    timed("erase all 2,880 edges", || m.erase(&edges, &[]));
    assert_eq!(m.vertices().count(), 0);
    assert!(took < Duration::from_secs(3), "copy took {took:?}");
}

#[test]
#[ignore]
fn time_edits_inside_a_big_imported_group() {
    let (points, polygons) = terrain(100);
    let mut scene = Scene::new();
    let ((instance, report, _), _) = timed("import 10,000 bent quads", || {
        scene
            .import_mesh(
                MeshId::Root,
                &points,
                &polygons,
                true,
                DefinitionKind::Group,
                "terrain",
            )
            .unwrap()
    });
    assert!(report.faces > 19_000, "{report:?}");
    let def = scene.root().instance(instance).unwrap().definition;
    let mesh = MeshId::Definition(def);
    let model = scene.mesh(mesh).unwrap();
    let v = model.vertices().map(|(id, _)| id).nth(5_000).unwrap();
    let e = model.edges().map(|(id, _)| id).nth(15_000).unwrap();
    let f = model.faces().map(|(id, _)| id).nth(10_000).unwrap();
    let mut worst = Duration::ZERO;
    let mut time = |what: &str, edit: &mut dyn FnMut(&mut Scene)| {
        let (_, took) = timed(what, || edit(&mut scene));
        worst = worst.max(took);
    };
    time("move one vertex up 1 cm", &mut |s| {
        s.edit(mesh, |m| {
            m.move_vertices(&BTreeSet::from([v]), DVec3::Z * 0.01)
        })
        .unwrap();
    });
    time("erase one edge", &mut |s| {
        s.edit(mesh, |m| Ok(m.erase(&[e], &[]))).unwrap();
    });
    time("push/pull one triangle 1 m", &mut |s| {
        s.edit(mesh, |m| m.push_pull(f, 1.0)).unwrap();
    });
    time("draw a line above the terrain", &mut |s| {
        s.edit(mesh, |m| {
            m.add_edge(DVec3::new(10.5, 10.5, 9.0), DVec3::new(20.5, 30.5, 9.0))
        })
        .unwrap();
    });
    scene.validate().unwrap();
    assert!(worst < Duration::from_secs(1), "an edit took {worst:?}");
}

#[test]
#[ignore]
fn time_exploding_an_imported_group() {
    let (points, polygons) = terrain(30);
    let mut scene = Scene::new();
    let (instance, report, _) = scene
        .import_mesh(
            MeshId::Root,
            &points,
            &polygons,
            true,
            DefinitionKind::Group,
            "terrain",
        )
        .unwrap();
    let (_, took) = timed(&format!("explode {} imported faces", report.faces), || {
        scene.explode(MeshId::Root, instance).unwrap()
    });
    assert_eq!(scene.root().faces().count(), report.faces);
    scene.validate().unwrap();
    assert!(took < Duration::from_secs(3), "explode took {took:?}");
}
