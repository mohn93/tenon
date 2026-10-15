//! Timing of headless renders of a lit night scene: a 10 x 10 grid of walled rooms, each
//! with a glowing lamp (100 lights). Ignored by default; run with
//! `cargo test --release -p render --test perf -- --ignored --nocapture`.

use std::time::Instant;

use kernel::EntityId;
use render::Snapshotter;
use session::{Command, Document, Lighting, Side};

const ROOMS: u32 = 10;
const ROOM: f64 = 4.0;

fn run(doc: &mut Document, command: Command) -> session::Outcome {
    doc.execute(command.clone())
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
}

fn created(outcome: &session::Outcome, face: bool) -> Vec<EntityId> {
    outcome
        .changes
        .created
        .iter()
        .copied()
        .filter(|e| {
            if face {
                matches!(e, EntityId::Face(_))
            } else {
                matches!(e, EntityId::Instance(_))
            }
        })
        .collect()
}

/// A wall box on the ground from `a` to `b` (x, y), 2.5 m tall.
fn wall(doc: &mut Document, a: [f64; 2], b: [f64; 2]) {
    let base = run(
        doc,
        Command::AddRectangle {
            corner: [a[0], a[1], 0.0],
            opposite: [b[0], b[1], 0.0],
            normal: None,
        },
    );
    let face = created(&base, true)[0];
    let EntityId::Face(id) = face else {
        unreachable!()
    };
    let up = doc.model().face(id).unwrap().normal.z > 0.0;
    run(
        doc,
        Command::PushPull {
            face,
            distance: if up { 2.5 } else { -2.5 },
        },
    );
}

/// The rooms, lamps and night lighting; the camera looks over the grid.
fn scene() -> Document {
    let mut d = Document::new(glam::DVec2::new(1400.0, 900.0));
    let size = ROOM * ROOMS as f64;
    run(
        &mut d,
        Command::AddRectangle {
            corner: [-1.0, -1.0, -0.01],
            opposite: [size + 1.0, size + 1.0, -0.01],
            normal: None,
        },
    );
    // Walls on the south and west side of each room, a door gap at the corner.
    for i in 0..ROOMS {
        for j in 0..ROOMS {
            let (x, y) = (i as f64 * ROOM, j as f64 * ROOM);
            wall(&mut d, [x + 0.2, y], [x + 2.8, y + 0.2]);
            wall(&mut d, [x, y + 0.2], [x + 0.2, y + 2.8]);
        }
    }
    let shade = run(
        &mut d,
        Command::AddRectangle {
            corner: [1.7, 1.7, 2.2],
            opposite: [2.3, 2.3, 2.2],
            normal: None,
        },
    );
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: created(&shade, true),
            name: "Lamp".into(),
        },
    );
    let lamp = created(&made, false)[0];
    let glow = run(
        &mut d,
        Command::AddMaterial {
            name: Some("Glow".into()),
            color: Some([255, 220, 160]),
            opacity: None,
            texture: None,
            emission: Some(3.0),
        },
    )
    .changes
    .materials[0];
    run(
        &mut d,
        Command::Paint {
            ids: vec![lamp],
            material: Some(glow),
            side: Side::Front,
        },
    );
    let row = run(
        &mut d,
        Command::Copy {
            ids: vec![lamp],
            offset: [ROOM, 0.0, 0.0],
            count: Some(ROOMS - 1),
        },
    );
    let mut first_row = created(&row, false);
    first_row.push(lamp);
    run(
        &mut d,
        Command::Copy {
            ids: first_row,
            offset: [0.0, ROOM, 0.0],
            count: Some(ROOMS - 1),
        },
    );
    run(
        &mut d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
        },
    );
    run(
        &mut d,
        Command::SetCamera {
            eye: [-6.0, -10.0, 18.0],
            target: [size / 2.0, size / 2.0, 0.0],
            fov_y_deg: None,
            up: None,
            projection: None,
        },
    );
    d
}

#[test]
#[ignore]
fn time_night_renders() {
    let t = Instant::now();
    let d = scene();
    eprintln!("scene built in {:.2} s", t.elapsed().as_secs_f64());
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let capture = |snap: &mut Snapshotter, camera: &session::Camera| {
        let t = Instant::now();
        snap.capture(
            d.scene(),
            d.context(),
            d.selection(),
            camera,
            Lighting::Night,
            1400,
            900,
        )
        .expect("capture");
        t.elapsed().as_secs_f64()
    };
    let first = capture(&mut snap, &d.camera);
    let same: Vec<f64> = (0..10).map(|_| capture(&mut snap, &d.camera)).collect();
    // The camera moving a little each frame (the shadowed set stays the same).
    let mut moving = Vec::new();
    for k in 0..10 {
        let mut camera = d.camera.clone();
        camera.eye += glam::DVec3::new(0.1 * k as f64, 0.0, 0.0);
        moving.push(capture(&mut snap, &camera));
    }
    // Switching between this scene and one with a wall more: the casters change each frame,
    // so every shadowed light's map is drawn again.
    let mut edited = scene();
    wall(&mut edited, [1.0, 1.0], [1.2, 2.0]);
    let mut changed = Vec::new();
    for k in 0..6 {
        let doc = if k % 2 == 0 { &edited } else { &d };
        let t = Instant::now();
        snap.capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            &doc.camera,
            Lighting::Night,
            1400,
            900,
        )
        .expect("capture");
        changed.push(t.elapsed().as_secs_f64());
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    eprintln!(
        "first {:.3} s, same view {:.3} s, moving camera {:.3} s, casters changed {:.3} s",
        first,
        mean(&same),
        mean(&moving),
        mean(&changed)
    );
}

/// The same timings for a saved model named by `HOUSE_SKR`, its materials named "Lamp" made
/// to glow at 3, seen from its saved camera at night.
#[test]
#[ignore]
fn time_house_renders() {
    let Some(path) = std::env::var_os("HOUSE_SKR") else {
        eprintln!("HOUSE_SKR not set, skipping");
        return;
    };
    let mut d = Document::new(glam::DVec2::new(1400.0, 900.0));
    run(
        &mut d,
        Command::Open {
            path: path.to_string_lossy().into_owned(),
            discard: true,
        },
    );
    let lamps: Vec<_> = d
        .scene()
        .materials()
        .filter(|(_, m)| m.name == "Lamp")
        .map(|(id, _)| id)
        .collect();
    for material in lamps {
        run(
            &mut d,
            Command::ChangeMaterial {
                material,
                name: None,
                color: None,
                opacity: None,
                texture: None,
                emission: Some(3.0),
            },
        );
    }
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut capture = |camera: &session::Camera| {
        let t = Instant::now();
        snap.capture(
            d.scene(),
            d.context(),
            d.selection(),
            camera,
            Lighting::Night,
            1400,
            900,
        )
        .expect("capture");
        t.elapsed().as_secs_f64()
    };
    let first = capture(&d.camera);
    let same: Vec<f64> = (0..10).map(|_| capture(&d.camera)).collect();
    let mut moving = Vec::new();
    for k in 0..10 {
        let mut camera = d.camera.clone();
        camera.eye += glam::DVec3::new(0.05 * k as f64, 0.0, 0.0);
        moving.push(capture(&camera));
    }
    drop(capture);
    // An edit and its undo, each frame: the casters change, so every map is drawn again.
    let mut edited = Vec::new();
    for k in 0..6 {
        if k % 2 == 0 {
            run(
                &mut d,
                Command::AddRectangle {
                    corner: [100.0, 100.0, 0.0],
                    opposite: [101.0, 101.0, 0.0],
                    normal: None,
                },
            );
        } else {
            run(&mut d, Command::Undo);
        }
        let t = Instant::now();
        snap.capture(
            d.scene(),
            d.context(),
            d.selection(),
            &d.camera,
            Lighting::Night,
            1400,
            900,
        )
        .expect("capture");
        edited.push(t.elapsed().as_secs_f64());
    }
    let lights = render::scene::scene_base(d.scene(), &[]).lights.len();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    eprintln!(
        "{lights} lights: first {:.3} s, same view {:.3} s, moving camera {:.3} s, \
         edited {:.3} s",
        first,
        mean(&same),
        mean(&moving),
        mean(&edited)
    );
}
