//! Timings for saving and opening a big model, and for purging many definitions. Ignored by default (they assert only loose
//! bounds); run them in release to see the numbers:
//! `cargo test --release -p session --test perf -- --ignored --nocapture`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document};

fn run(d: &mut Document, v: Value) {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd).expect("command runs");
}

fn timed<T>(what: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    let took = t.elapsed();
    eprintln!("{what}: {took:?}");
    (out, took)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// An OBJ of an `n` x `n` terrain of bent quads.
fn terrain_obj(n: usize) -> String {
    let mut s = String::new();
    for y in 0..=n {
        for x in 0..=n {
            let (fx, fy) = (x as f64, y as f64);
            let z = (fx * 0.37).sin() * (fy * 0.23).cos() * 2.0;
            writeln!(s, "v {fx} {fy} {z}").unwrap();
        }
    }
    let at = |x: usize, y: usize| y * (n + 1) + x + 1;
    for y in 0..n {
        for x in 0..n {
            let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1));
            writeln!(s, "f {a} {b} {c} {d}").unwrap();
        }
    }
    s
}

/// A unit cube's OBJ.
const CUBE: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

#[test]
#[ignore]
fn time_saving_and_opening_a_big_model() {
    let terrain = scratch("terrain.obj");
    std::fs::write(&terrain, terrain_obj(140)).unwrap();
    let cube = scratch("cube.obj");
    std::fs::write(&cube, CUBE).unwrap();
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    run(&mut d, json!({"type": "import", "path": terrain}));
    // About as many definitions as a detailed model: each import is a group of its own.
    for _ in 0..600 {
        run(&mut d, json!({"type": "import", "path": cube}));
    }

    let path = scratch("big.tnn");
    let (_, saved) = timed("save", || {
        run(&mut d, json!({"type": "save", "path": path}));
    });
    let bytes = std::fs::read(&path).unwrap();
    eprintln!("file: {} bytes", bytes.len());
    let mut e = Document::new(DVec2::new(800.0, 600.0));
    let (_, opened) = timed("open", || {
        run(&mut e, json!({"type": "open", "path": path}));
    });
    // The same model saves to the same bytes.
    let again = scratch("again.tnn");
    run(&mut e, json!({"type": "save", "path": again}));
    assert!(std::fs::read(&again).unwrap() == bytes);

    assert!(saved < Duration::from_millis(500), "save took {saved:?}");
    assert!(opened < Duration::from_millis(1000), "open took {opened:?}");
}

#[test]
#[ignore]
fn time_purging_many_unused_definitions() {
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    let mut instances = Vec::new();
    for k in 0..2000 {
        let x = k as f64 * 2.0;
        let out = d
            .execute(
                serde_json::from_value(
                    json!({"type": "add_rectangle", "corner": [x, 0, 0], "opposite": [x + 1.0, 1, 0]}),
                )
                .unwrap(),
            )
            .unwrap();
        let ids: Vec<String> = out
            .changes
            .created
            .iter()
            .map(|id| id.to_string())
            .filter(|id| id.starts_with(['e', 'f']))
            .collect();
        let out = d
            .execute(
                serde_json::from_value(json!({"type": "make_component", "ids": ids, "name": "R"}))
                    .unwrap(),
            )
            .unwrap();
        let instance = out.changes.created.iter().map(|id| id.to_string());
        instances.extend(instance.filter(|id| id.starts_with('i')));
    }
    run(&mut d, json!({"type": "erase", "ids": instances}));

    // The old purge walked every instance once per definition per removal.
    let (out, took) = timed("purge 2000", || {
        d.execute(Command::PurgeDefinitions {}).unwrap()
    });
    assert_eq!(out.purged, Some(2000));
    assert!(took < Duration::from_secs(2), "purge took {took:?}");
}

#[test]
#[ignore]
fn time_intersecting_every_one_of_22500_placed_boxes() {
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    );
    run(&mut d, json!({"type": "select", "all": true}));
    let faces = d.selection().iter().copied().collect::<Vec<_>>();
    let base = faces
        .iter()
        .find(|id| matches!(id, kernel::EntityId::Face(_)))
        .copied()
        .unwrap();
    run(
        &mut d,
        json!({"type": "push_pull", "face": base, "distance": 1}),
    );
    run(&mut d, json!({"type": "select", "all": true}));
    let ids: Vec<_> = d.selection().iter().copied().collect();
    run(
        &mut d,
        json!({"type": "make_component", "ids": ids, "name": "box"}),
    );
    run(&mut d, json!({"type": "select", "all": true}));
    let ids: Vec<_> = d.selection().iter().copied().collect();
    run(
        &mut d,
        json!({"type": "copy", "ids": ids, "offset": [2, 0, 0], "count": 149}),
    );
    run(&mut d, json!({"type": "select", "all": true}));
    let ids: Vec<_> = d.selection().iter().copied().collect();
    run(
        &mut d,
        json!({"type": "copy", "ids": ids, "offset": [0, 2, 0], "count": 149}),
    );
    run(&mut d, json!({"type": "select", "all": true}));
    assert_eq!(d.selection().len(), 22_500);
    let (out, took) = timed(
        "intersect 22,500 placed boxes, Select All, With Model",
        || {
            d.execute(
                serde_json::from_value::<Command>(
                    json!({"type": "intersect_faces", "with": "model"}),
                )
                .unwrap(),
            )
            .unwrap()
        },
    );
    assert!(out.intersect.unwrap().added_edges.is_empty());
    assert!(took < Duration::from_secs(1), "{took:?}");
}
