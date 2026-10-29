//! Tags written as layers and read back by `skp::legacy`: each tag's name, colour and
//! hidden flag, and every entity's layer.

use crate::read;
use skp::geom::NO_LAYER;
use skp::model::{LayerDef, SceneIndex};
use skpwrite::{
    Definition, Doc, Edge, Face, Geometry, Guide, Instance, Material, Tag, WriteError, write,
};

const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A unit square in the z = 0 plane, its four edges and its face.
fn square() -> Geometry {
    let edge = |a, b| Edge {
        a,
        b,
        ..Edge::default()
    };
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

fn tag(name: &str, rgb: [u8; 3], hidden: bool) -> Tag {
    Tag {
        name: name.into(),
        rgb,
        hidden,
    }
}

fn solid(name: String) -> Material {
    Material {
        name,
        rgba: [10, 20, 30, 255],
        opacity: 1.0,
        texture: None,
    }
}

/// The layer a reader's layer reference names.
fn layer(index: &SceneIndex, id: u32) -> &LayerDef {
    assert_ne!(id, NO_LAYER, "every legacy entity names a layer");
    index
        .layers
        .iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("no layer {id}: {:?}", index.layers))
}

/// Three tags (one hidden) on faces, edges, instances and guides of the root and of a
/// definition, after `materials` materials.
fn tagged_doc(materials: usize) -> Doc {
    let mut inner = square();
    inner.faces[0].tag = Some(2);
    inner.guides = vec![Guide::Point {
        position: [0.5, 0.5, 1.0],
        tag: Some(1),
        hidden: false,
    }];
    let mut root = square();
    root.faces[0].tag = Some(0);
    root.edges[1].tag = Some(2);
    root.instances = vec![
        Instance {
            definition: 0,
            rotation: IDENTITY,
            translation: [5.0, 0.0, 0.0],
            material: None,
            tag: Some(1),
            hidden: false,
        },
        Instance {
            definition: 0,
            rotation: IDENTITY,
            translation: [9.0, 0.0, 0.0],
            material: None,
            tag: None,
            hidden: false,
        },
    ];
    root.guides = vec![
        Guide::Line {
            point: [0.0, 0.0, 2.0],
            direction: [1.0, 0.0, 0.0],
            tag: Some(0),
            hidden: false,
        },
        Guide::Point {
            position: [3.0, 3.0, 3.0],
            tag: None,
            hidden: false,
        },
    ];
    Doc {
        materials: (0..materials).map(|i| solid(format!("M{i}"))).collect(),
        tags: vec![
            tag("Walls", [200, 30, 40], false),
            tag("Roof", [0, 128, 255], true),
            tag("Möbel 家具", [1, 2, 3], false),
        ],
        pages: Vec::new(),
        definitions: vec![Definition {
            name: "Panel".into(),
            is_group: false,
            geometry: inner,
        }],
        root,
    }
}

/// The tagged document reads back with its tags and every entity on its layer.
fn check_tagged(materials: usize) {
    let doc = tagged_doc(materials);
    let (m, index, _) = read(&write(&doc).expect("writes"));
    let names: Vec<&str> = index.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Layer0", "Walls", "Roof", "Möbel 家具"]);
    for (l, t) in index.layers[1..].iter().zip(&doc.tags) {
        assert_eq!(l.hidden, t.hidden, "{}", t.name);
        assert_eq!(l.color[..3], t.rgb, "{}", t.name);
    }
    assert!(!index.layers[0].hidden);

    let name = |id: u32| layer(&index, id).name.as_str();
    let root = &m.root;
    assert_eq!(name(root.face_layer[0]), "Walls");
    let mut edge_layers: Vec<&str> = root.edge_layer.iter().map(|l| name(*l)).collect();
    edge_layers.sort();
    assert_eq!(edge_layers, ["Layer0", "Layer0", "Layer0", "Möbel 家具"]);
    let instance_layers: Vec<&str> = root
        .instances
        .iter()
        .map(|i| name(i.layer as u32))
        .collect();
    assert_eq!(instance_layers, ["Roof", "Layer0"]);
    let guide_layers: Vec<&str> = root.guide_layer.iter().map(|l| name(*l)).collect();
    assert_eq!(guide_layers, ["Walls", "Layer0"]);

    let def = &m.definitions[0].geom;
    assert_eq!(name(def.face_layer[0]), "Möbel 家具");
    assert!(def.edge_layer.iter().all(|l| name(*l) == "Layer0"));
    let guide_layers: Vec<&str> = def.guide_layer.iter().map(|l| name(*l)).collect();
    assert_eq!(guide_layers, ["Roof"]);
}

#[test]
fn tags_round_trip() {
    check_tagged(0);
    // Materials move the layers' slots.
    check_tagged(3);
}

#[test]
fn tag_slots_past_the_escape() {
    // The materials take the slots up to just short of 0x7fff, so the tags' slots, and the
    // references to them, take the escaped form.
    check_tagged(0x7fff - 9 - 2);
}

#[test]
fn an_untagged_document_writes_only_layer0() {
    let doc = Doc {
        root: square(),
        ..Doc::default()
    };
    let (m, index, _) = read(&write(&doc).expect("writes"));
    assert_eq!(index.layers.len(), 1);
    assert_eq!(index.layers[0].name, "Layer0");
    assert!(
        m.root
            .face_layer
            .iter()
            .chain(&m.root.edge_layer)
            .all(|l| *l == index.layers[0].id)
    );
}

#[test]
fn bad_tags_are_errors() {
    let with = |tags: Vec<Tag>, edit: fn(&mut Geometry)| {
        let mut root = square();
        edit(&mut root);
        Doc {
            tags,
            root,
            ..Doc::default()
        }
    };
    let one = || vec![tag("A", [0; 3], false)];
    let out_of_range = |e: Result<Vec<u8>, WriteError>| {
        assert_eq!(e, Err(WriteError::TagOutOfRange { index: 1 }));
    };
    out_of_range(write(&with(one(), |g| g.faces[0].tag = Some(1))));
    out_of_range(write(&with(one(), |g| g.edges[2].tag = Some(1))));
    out_of_range(write(&with(one(), |g| {
        g.guides.push(Guide::Point {
            position: [0.0; 3],
            tag: Some(1),
            hidden: false,
        })
    })));
    let mut placed = with(one(), |_| {});
    placed.definitions.push(Definition {
        name: "D".into(),
        is_group: true,
        geometry: square(),
    });
    placed.root.instances.push(Instance {
        definition: 0,
        rotation: IDENTITY,
        translation: [0.0; 3],
        material: None,
        tag: Some(1),
        hidden: false,
    });
    out_of_range(write(&placed));

    for (tags, name) in [
        (vec![tag("A", [0; 3], false), tag("A", [1; 3], true)], "A"),
        (vec![tag("Layer0", [0; 3], false)], "Layer0"),
    ] {
        assert_eq!(
            write(&with(tags, |_| {})),
            Err(WriteError::DuplicateName {
                what: "tag",
                name: name.into()
            })
        );
    }
    // "Layer_" and the name fit a string record.
    let long = "x".repeat(254 - 6);
    assert!(write(&with(vec![tag(&long, [0; 3], false)], |_| {})).is_ok());
    let longer = "x".repeat(254 - 5);
    assert!(matches!(
        write(&with(vec![tag(&longer, [0; 3], false)], |_| {})),
        Err(WriteError::StringTooLong {
            what: "tag name",
            ..
        })
    ));
}

/// A string as the archive writes it: the marker, a length byte and UTF-16.
fn archive_string(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut out = vec![0xff, 0xfe, 0xff, units.len() as u8];
    out.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    out
}

/// The last `u32` of a definition's 43-byte behaviour block, found after its name, its two
/// empty strings and its timestamp.
fn definition_group_word(bytes: &[u8], name: &str) -> u32 {
    let pat = archive_string(name);
    let empty = archive_string("");
    let mut tail = pat.clone();
    tail.extend(&empty);
    tail.extend(&empty);
    let at = bytes
        .windows(tail.len())
        .position(|w| w == tail)
        .unwrap_or_else(|| panic!("no definition named {name}"));
    let block = at + tail.len() + 4;
    u32::from_le_bytes(bytes[block + 39..block + 43].try_into().unwrap())
}

#[test]
fn group_definitions_carry_the_group_word_and_components_do_not() {
    let def = |name: &str, is_group| Definition {
        name: name.into(),
        is_group,
        geometry: square(),
    };
    let place = |definition| Instance {
        definition,
        rotation: IDENTITY,
        translation: [0.0; 3],
        material: None,
        tag: None,
        hidden: false,
    };
    let doc = Doc {
        pages: Vec::new(),
        definitions: vec![
            def("Red box", true),
            def("Blue box", false),
            def("Hidden box", true),
        ],
        root: Geometry {
            instances: vec![place(0), place(1), place(2)],
            ..Geometry::default()
        },
        ..Doc::default()
    };
    let bytes = write(&doc).unwrap();
    assert_eq!(definition_group_word(&bytes, "Red box"), 1);
    assert_eq!(definition_group_word(&bytes, "Hidden box"), 1);
    assert_eq!(definition_group_word(&bytes, "Blue box"), 0);
    // The reader still tells them apart by the instances' class.
    let (_, index, _) = read(&bytes);
    assert_eq!(index.layers[0].name, "Layer0");
    assert_eq!(index.layers[0].color, [255, 255, 255, 255]);
}
