//! Scenes written as `CViewPage` records and read back by `skp::legacy`: name, camera,
//! projection and hidden tags, with the model after them still intact.

use crate::read;
use skp::model::{SceneDef, SceneIndex};
use skpwrite::{Doc, Edge, Face, Geometry, Material, Page, PageView, Tag, WriteError, write};

const INCH_TO_MM: f64 = 25.4;

fn edge(a: u32, b: u32) -> Edge {
    Edge {
        a,
        b,
        ..Edge::default()
    }
}

fn square() -> Geometry {
    Geometry {
        vertices: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        edges: vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 0)],
        faces: vec![Face {
            loops: vec![vec![0, 1, 2, 3]],
            ..Face::default()
        }],
        ..Geometry::default()
    }
}

fn tag(name: &str, hidden: bool) -> Tag {
    Tag {
        name: name.into(),
        rgb: [10, 20, 30],
        hidden,
    }
}

fn view(eye: [f64; 3], parallel: bool) -> PageView {
    PageView {
        eye,
        target: [0.5, 0.5, 0.0],
        up: [0.0, 0.0, 1.0],
        fov_deg: 40.0,
        parallel,
        height: 120.0,
    }
}

fn doc(materials: usize, pages: Vec<Page>) -> Doc {
    Doc {
        materials: (0..materials)
            .map(|i| Material {
                name: format!("M{i}"),
                rgba: [1, 2, 3, 255],
                opacity: 1.0,
                texture: None,
            })
            .collect(),
        tags: vec![tag("Walls", false), tag("Roof", true), tag("Möbel", false)],
        pages,
        definitions: Vec::new(),
        root: square(),
    }
}

fn three_pages() -> Vec<Page> {
    vec![
        Page {
            name: "Front".into(),
            view: Some(view([10.0, -20.0, 30.0], false)),
            hidden_tags: vec![1, 2],
        },
        Page {
            name: "Plan 平面".into(),
            view: Some(view([0.5, 0.5, 100.0], true)),
            hidden_tags: vec![],
        },
        Page {
            name: "Tags only".into(),
            view: None,
            hidden_tags: vec![0],
        },
    ]
}

/// Tags hidden by a scene, by name.
fn hidden(index: &SceneIndex, s: &SceneDef) -> Vec<String> {
    s.hidden_layer_ids
        .iter()
        .map(|id| {
            index
                .layers
                .iter()
                .find(|l| l.id == *id)
                .unwrap()
                .name
                .clone()
        })
        .collect()
}

fn check_pages(materials: usize) {
    let d = doc(materials, three_pages());
    let (m, index, _) = read(&write(&d).expect("writes"));
    assert_eq!(index.scenes.len(), 3);
    let [front, plan, tags_only] = &index.scenes[..] else {
        unreachable!()
    };
    assert_eq!(front.name, "Front");
    assert_eq!(front.flags, 0x1ff);
    assert!(front.perspective);
    assert_eq!(
        front.eye.unwrap(),
        [10.0 * INCH_TO_MM, -20.0 * INCH_TO_MM, 30.0 * INCH_TO_MM]
    );
    assert_eq!(
        front.target.unwrap(),
        [0.5 * INCH_TO_MM, 0.5 * INCH_TO_MM, 0.0]
    );
    assert_eq!(front.up.unwrap(), [0.0, 0.0, 1.0]);
    assert_eq!(front.fov_deg, 40.0);
    assert_eq!(hidden(&index, front), ["Roof", "Möbel"]);

    assert_eq!(plan.name, "Plan 平面");
    assert!(!plan.perspective);
    assert_eq!(plan.ortho_height, 120.0 * INCH_TO_MM);
    assert_eq!(hidden(&index, plan), Vec::<String>::new());

    assert_eq!(tags_only.name, "Tags only");
    // No view: the camera bit is clear.
    assert_eq!(tags_only.flags, 0x1fe);
    assert_eq!(hidden(&index, tags_only), ["Walls"]);

    // The model after the scenes reads as before.
    assert_eq!(m.root.faces(), 1);
    assert_eq!(index.layers.len(), 4);
    assert_eq!(index.styles.len(), 1);
    assert_eq!(index.active_style, Some(0));
}

#[test]
fn scenes_round_trip() {
    check_pages(0);
    // Materials move the layers' slots.
    check_pages(3);
}

#[test]
fn a_model_without_scenes_has_none() {
    let (_, index, _) = read(&write(&doc(0, Vec::new())).expect("writes"));
    assert!(index.scenes.is_empty());
    assert_eq!(index.styles.len(), 1);
}

#[test]
fn scenes_cross_the_slot_escape() {
    // Slots past 0x7fff: the scenes' own references and the tail's use the escape.
    const N: usize = 11_000;
    let mut root = Geometry::default();
    for i in 0..N {
        let (x, y) = ((i % 200) as f64 * 3.0, (i / 200) as f64 * 3.0);
        let k = root.vertices.len() as u32;
        root.vertices
            .extend_from_slice(&[[x, y, 0.0], [x + 1.0, y, 0.0], [x, y + 1.0, 0.0]]);
        root.edges
            .extend_from_slice(&[edge(k, k + 1), edge(k + 1, k + 2), edge(k + 2, k)]);
        root.faces.push(Face {
            loops: vec![vec![k, k + 1, k + 2]],
            ..Face::default()
        });
    }
    let mut d = doc(0, three_pages());
    d.root = root;
    let (m, index, _) = read(&write(&d).expect("writes"));
    assert_eq!(m.root.faces(), N);
    assert_eq!(index.scenes.len(), 3);
    assert_eq!(index.scenes[0].name, "Front");
    assert_eq!(index.scenes[2].name, "Tags only");
    assert_eq!(index.styles.len(), 1);
}

#[test]
fn hidden_tags_with_layer_slots_past_the_limit_are_left_out_and_counted() {
    // Textured materials take two slots each: 16,378 of them end the materials' own slots
    // just under 0x7fff and put every tag's layer slot past it.
    let mut d = doc(0, three_pages());
    d.materials = (0..16_378)
        .map(|i| Material {
            name: format!("M{i}"),
            rgba: [1, 2, 3, 255],
            opacity: 1.0,
            texture: Some(skpwrite::Texture {
                bytes: crate::png_2x2(),
                width: 1.0,
                height: 1.0,
            }),
        })
        .collect();
    let (bytes, notes) = skpwrite::write_noted(&d).expect("writes");
    // "Front" hides two tags, "Tags only" one.
    assert_eq!(notes.skipped_hidden_tags, 3);
    let (_, index, _) = read(&bytes);
    assert_eq!(index.scenes.len(), 3);
    assert!(hidden(&index, &index.scenes[0]).is_empty());
    // Within the limit nothing is left out.
    let (_, notes) = skpwrite::write_noted(&doc(0, three_pages())).unwrap();
    assert_eq!(notes.skipped_hidden_tags, 0);
}

#[test]
fn the_same_scenes_give_the_same_bytes() {
    let a = write(&doc(0, three_pages())).unwrap();
    let b = write(&doc(0, three_pages())).unwrap();
    assert_eq!(a, b);
}

#[test]
fn bad_scenes_are_errors() {
    let bad = |page: Page| write(&doc(0, vec![page])).unwrap_err();
    let ok = Page {
        name: "A".into(),
        view: Some(view([1.0, 2.0, 3.0], false)),
        hidden_tags: vec![],
    };
    assert!(matches!(
        bad(Page {
            hidden_tags: vec![9],
            ..ok.clone()
        }),
        WriteError::TagOutOfRange { index: 9 }
    ));
    assert!(matches!(
        bad(Page {
            name: "x".repeat(300),
            ..ok.clone()
        }),
        WriteError::StringTooLong { .. }
    ));
    let mut v = view([1.0, 2.0, 3.0], false);
    v.eye[0] = f64::NAN;
    assert!(matches!(
        bad(Page {
            view: Some(v),
            ..ok.clone()
        }),
        WriteError::NonFinite { .. }
    ));
    let mut v = view([1.0, 2.0, 3.0], false);
    v.fov_deg = 0.0;
    assert!(matches!(
        bad(Page {
            view: Some(v),
            ..ok.clone()
        }),
        WriteError::Degenerate { .. }
    ));
    let many = vec![ok; 10_001];
    assert!(matches!(
        write(&doc(0, many)).unwrap_err(),
        WriteError::TooLarge { .. }
    ));
}
