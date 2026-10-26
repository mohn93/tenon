//! A picture lying flush on a face (lift 0) shows over it without flicker, at any distance and
//! from a grazing angle, in perspective and parallel views. Headless: these run on whatever
//! adapter the machine has and skip without one; they were not run in a window or a browser.

use glam::DVec3;
use kernel::EntityId;
use render::Snapshotter;
use session::{Command, Document, Projection, Side};

use crate::golden::{WIDTH, add_material, created_face, doc, look, quadrants_png, run};

/// The picture's x axis: turned off the face's grid so its corners fall between the face's own
/// triangles' pixels and the two surfaces' depths differ by rounding.
const TURN: f64 = 0.5;

/// A plain face `2 * d` meters wide at the origin with the quadrants picture `d` wide centred
/// on it, turned by `TURN` radians and flush (lift 0).
fn flush_picture(d: f64) -> Document {
    let mut doc = doc();
    run(
        &mut doc,
        Command::AddRectangle {
            corner: [-d, -d, 0.0],
            opposite: [d, d, 0.0],
            normal: None,
        },
    );
    run(
        &mut doc,
        Command::ImportImage {
            path: Some(picture_file().display().to_string()),
            data: None,
            origin: (-0.5 * d * (frame().0 + frame().1)).to_array(),
            normal: None,
            x_axis: Some(frame().0.to_array()),
            width: d,
            name: None,
            lift: Some(0.0),
        },
    );
    doc
}

/// The quadrants image, written once (the tests run side by side).
fn picture_file() -> &'static std::path::Path {
    static FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    FILE.get_or_init(quadrants_png)
}

/// The picture's width and height directions on the face.
fn frame() -> (DVec3, DVec3) {
    let x = DVec3::new(TURN.cos(), TURN.sin(), 0.0);
    (x, DVec3::Z.cross(x))
}

/// The point `(u, v)` of the picture, each from 0 to 1 across it.
fn on_picture(d: f64, u: f64, v: f64) -> DVec3 {
    let (x, y) = frame();
    d * ((u - 0.5) * x + (v - 0.5) * y)
}

fn pixel(rgba: &[u8], at: glam::DVec2) -> [u8; 3] {
    let (x, y) = (at.x.round() as u32, at.y.round() as u32);
    let i = ((y * WIDTH + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

/// The colors sampled in the red quadrant and in the blue one.
type Quadrants = (Vec<[u8; 3]>, Vec<[u8; 3]>);

/// Renders `doc` and returns the colors at a grid of points inside the picture's red (top
/// left) quadrant and its blue (bottom right) one.
fn quadrant_colors(doc: &Document, d: f64) -> Option<Quadrants> {
    let mut snap = Snapshotter::new_headless()?;
    let rgba = snap.capture(
        doc.scene(),
        doc.context(),
        &std::collections::BTreeSet::new(),
        &doc.camera,
        doc.lighting,
        WIDTH,
        crate::golden::HEIGHT,
    )?;
    let sample = |us: [f64; 3], vs: [f64; 3]| {
        let mut out = Vec::new();
        for u in us {
            for v in vs {
                let at = doc.camera.project(on_picture(d, u, v)).unwrap();
                out.push(pixel(&rgba, at));
            }
        }
        out
    };
    Some((
        sample([0.15, 0.25, 0.35], [0.65, 0.75, 0.85]),
        sample([0.65, 0.75, 0.85], [0.15, 0.25, 0.35]),
    ))
}

fn shows_picture(colors: Quadrants, what: &str) {
    let (reds, blues) = colors;
    for red in reds {
        assert!(
            red[0] > red[1].saturating_add(50) && red[0] > red[2].saturating_add(50),
            "{what}: a point of the red quadrant shows {red:?}"
        );
    }
    for blue in blues {
        assert!(
            blue[2] > blue[0].saturating_add(50) && blue[2] > blue[1].saturating_add(30),
            "{what}: a point of the blue quadrant shows {blue:?}"
        );
    }
}

#[test]
fn a_flush_picture_shows_over_its_face_at_any_distance() {
    for d in [2.0, 20.0, 200.0, 800.0] {
        let mut doc = flush_picture(d);
        look(&mut doc, [0.25 * d, -0.9 * d, 1.1 * d], [0.0, 0.0, 0.0]);
        let Some(colors) = quadrant_colors(&doc, d) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        shows_picture(colors, &format!("perspective, picture {d} m"));
    }
}

#[test]
fn a_flush_picture_shows_from_a_grazing_angle_and_in_a_parallel_view() {
    let d = 20.0;
    let mut doc = flush_picture(d);
    look(&mut doc, [0.0, -4.0 * d, 1.2 * d], [0.0, 0.0, 0.0]);
    let Some(colors) = quadrant_colors(&doc, d) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    shows_picture(colors, "grazing");

    let mut doc = flush_picture(d);
    look(&mut doc, [0.25 * d, -0.9 * d, 1.1 * d], [0.0, 0.0, 0.0]);
    doc.camera.projection = Projection::Parallel;
    let colors = quadrant_colors(&doc, d).unwrap();
    shows_picture(colors, "parallel");
}

/// Lays a grey face `2 * d` wide `gap` meters above the plane z = 0, over the picture.
fn cover(doc: &mut Document, d: f64, gap: f64) {
    let face = created_face(&run(
        doc,
        Command::AddRectangle {
            corner: [-d, -d, gap],
            opposite: [d, d, gap],
            normal: None,
        },
    ));
    let grey = add_material(doc, "Grey", [128, 128, 128]);
    paint(doc, face, grey);
}

fn paint(doc: &mut Document, face: EntityId, material: session::MaterialId) {
    run(
        doc,
        Command::Paint {
            ids: vec![face],
            material: Some(material),
            side: Side::Both,
        },
    );
}

fn shows_grey(colors: Quadrants, what: &str) {
    for c in colors.0.into_iter().chain(colors.1) {
        let (lo, hi) = (c.iter().min().unwrap(), c.iter().max().unwrap());
        assert!(hi - lo < 30, "{what}: a point of the cover shows {c:?}");
    }
}

#[test]
fn a_flush_picture_does_not_show_through_a_face_in_front_of_it() {
    // The picture is pulled 2^-12 of its distance nearer in perspective and 2^-12 eye-to-target
    // distances in a parallel view; a face 1% of the picture's size in front of it hides it in
    // both (the parallel view's depth spans 200 distances, so a pull scaled by depth alone
    // would be a twentieth of the distance there).
    let d = 20.0;
    for projection in [Projection::Perspective, Projection::Parallel] {
        let mut doc = flush_picture(d);
        cover(&mut doc, d, 0.01 * d);
        look(&mut doc, [0.25 * d, -0.9 * d, 1.1 * d], [0.0, 0.0, 0.0]);
        doc.camera.projection = projection;
        let Some(colors) = quadrant_colors(&doc, d) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        shows_grey(colors, &format!("{projection:?}"));
    }
}

#[test]
fn a_textured_face_that_is_not_a_picture_is_drawn_where_it_lies() {
    // A root face painted with the quadrants texture, 1.5 cm under a grey face, seen from about
    // 58 m: only pictures are pulled nearer, so the grey face hides the texture.
    let d = 40.0;
    let mut doc = doc();
    let face = created_face(&run(
        &mut doc,
        Command::AddRectangle {
            corner: [-d, -d, 0.0],
            opposite: [d, d, 0.0],
            normal: None,
        },
    ));
    let tiles = run(
        &mut doc,
        Command::AddMaterial {
            name: Some("Tiles".into()),
            color: None,
            opacity: None,
            texture: Some(session::TextureSource {
                path: Some(picture_file().display().to_string()),
                width: Some(d),
                height: Some(d),
            }),
            emission: None,
            metalness: None,
            roughness: None,
            density: None,
            existing: Default::default(),
        },
    )
    .changes
    .materials[0];
    paint(&mut doc, face, tiles);
    cover(&mut doc, d, 0.015);
    look(&mut doc, [0.25 * d, -0.9 * d, 1.1 * d], [0.0, 0.0, 0.0]);
    let Some(colors) = quadrant_colors(&doc, d) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    shows_grey(colors, "textured face under a grey one");
}
