//! Documents written by `skpwrite` and read back by `skp::legacy`.

use crate::{hex, png_2x2, read, sha256};
use skp::geom::DefGeometry;
use skp::model::TextureLocation;
use skp::model::edge_flag::{HIDDEN, SMOOTH, SOFT};
use skpwrite::{
    Definition, Doc, Edge, Face, Geometry, Guide, Instance, Material, Owner, Texture, WriteError,
    write,
};

/// The scaffold's tail, from offset 3515 to its end.
const TAIL_LEN: usize = 5796 - 3515;

/// The first of the Iso camera's two tail patches.
const ISO_TAIL_PATCH: [u8; 16] = [
    208, 168, 105, 97, 60, 68, 45, 71, 153, 164, 102, 125, 26, 223, 168, 54,
];

const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn edge(a: u32, b: u32) -> Edge {
    Edge {
        a,
        b,
        ..Edge::default()
    }
}

fn face(loops: Vec<Vec<u32>>) -> Face {
    Face {
        loops,
        ..Face::default()
    }
}

/// Every edge the faces' loops use, once each, in loop order.
fn edges_of(faces: &[Face]) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::new();
    for f in faces {
        for ring in &f.loops {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                if !out
                    .iter()
                    .any(|e| (e.a, e.b) == (a, b) || (e.a, e.b) == (b, a))
                {
                    out.push(edge(a, b));
                }
            }
        }
    }
    out
}

/// A box from the origin to `s`, every face's outer loop counter-clockwise seen from
/// outside.
fn boxed(s: [f64; 3]) -> Geometry {
    let vertices = (0..8)
        .map(|i| {
            [
                if i & 1 != 0 { s[0] } else { 0.0 },
                if i & 2 != 0 { s[1] } else { 0.0 },
                if i & 4 != 0 { s[2] } else { 0.0 },
            ]
        })
        .collect();
    let faces: Vec<Face> = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ]
    .iter()
    .map(|r| face(vec![r.to_vec()]))
    .collect();
    Geometry {
        vertices,
        edges: edges_of(&faces),
        faces,
        ..Geometry::default()
    }
}

fn one_edge_root() -> Geometry {
    Geometry {
        vertices: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        edges: vec![edge(0, 1)],
        ..Geometry::default()
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn close3(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

/// The positions of one loop's ring, in order.
fn ring_positions(g: &DefGeometry, l: usize) -> Vec<[f64; 3]> {
    let mut ring = Vec::new();
    assert!(g.loop_ring(l, &mut ring), "loop {l} has a ring");
    ring.iter().map(|v| g.vertex(*v)).collect()
}

/// The 13 terms an instance's transform is stored as.
fn terms(i: &Instance) -> [f64; 13] {
    let r = i.rotation;
    let t = i.translation;
    [
        r[0][0], r[0][1], r[0][2], r[1][0], r[1][1], r[1][2], r[2][0], r[2][1], r[2][2], t[0],
        t[1], t[2], 1.0,
    ]
}

#[test]
fn a_blank_document_reads() {
    let doc = Doc {
        root: one_edge_root(),
        ..Doc::default()
    };
    let (m, index, _) = read(&write(&doc).expect("writes"));
    assert_eq!(m.root.edges(), 1);
    assert_eq!(m.root.vertices(), 2);
    assert_eq!(m.root.faces(), 0);
    assert!(m.definitions.is_empty());
    assert!(index.materials.is_empty());
    assert_eq!(index.layers.len(), 1);
    assert_eq!(index.layers[0].name, "Layer0");
    let (a, b) = (m.root.edge_v[0], m.root.edge_v[1]);
    assert_eq!(m.root.vertex(a), [0.0, 0.0, 0.0]);
    assert_eq!(m.root.vertex(b), [10.0, 0.0, 0.0]);
}

#[test]
fn a_face_with_a_hole() {
    // A tilted square with a square hole: the outer ring counter-clockwise around the
    // normal, the hole clockwise.
    let tilt = |x: f64, y: f64| [x, y * 0.6, 2.0 + y * 0.8];
    let vertices = vec![
        tilt(0.0, 0.0),
        tilt(10.0, 0.0),
        tilt(10.0, 10.0),
        tilt(0.0, 10.0),
        tilt(3.0, 3.0),
        tilt(7.0, 3.0),
        tilt(7.0, 7.0),
        tilt(3.0, 7.0),
    ];
    let faces = vec![face(vec![vec![0, 1, 2, 3], vec![7, 6, 5, 4]])];
    let doc = Doc {
        root: Geometry {
            vertices: vertices.clone(),
            edges: edges_of(&faces),
            faces,
            ..Geometry::default()
        },
        ..Doc::default()
    };
    let (m, _, stats) = read(&write(&doc).expect("writes"));
    let g = &m.root;
    assert_eq!(g.faces(), 1);
    assert_eq!(g.edges(), 8);
    assert_eq!(g.vertices(), 8);
    assert_eq!(g.face_loop_ofs, vec![0, 2]);
    assert_eq!(stats.inner_loops, 1);

    let n = [0.0, -0.8, 0.6];
    assert!(close3(g.normal(0), n, 1e-9), "normal {:?}", g.normal(0));
    let centroid = [5.0, 3.0, 6.0];
    // SketchUp's own files store the plane as n.p + d = 0.
    assert!((g.face_plane_d[0] + dot(n, centroid)).abs() <= 1e-9);

    let outer = ring_positions(g, 0);
    let hole = ring_positions(g, 1);
    let want_outer: Vec<[f64; 3]> = [0, 1, 2, 3].iter().map(|i| vertices[*i]).collect();
    let want_hole: Vec<[f64; 3]> = [7, 6, 5, 4].iter().map(|i| vertices[*i]).collect();
    assert_eq!(outer, want_outer);
    assert_eq!(hole, want_hole);
}

#[test]
fn edges_keep_their_flags() {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 10.0, 0.0],
        [0.0, 10.0, 0.0],
        [20.0, 0.0, 0.0],
        [20.0, 5.0, 3.0],
    ];
    let f = |a, b, soft, smooth, hidden| Edge {
        a,
        b,
        soft,
        smooth,
        hidden,
        tag: None,
    };
    let edges = vec![
        f(0, 1, true, false, false),
        f(1, 2, false, true, false),
        f(2, 3, false, false, true),
        f(3, 0, true, true, true),
        // Loose: no face uses it.
        f(4, 5, true, false, false),
    ];
    let doc = Doc {
        root: Geometry {
            vertices: vertices.clone(),
            edges: edges.clone(),
            faces: vec![face(vec![vec![0, 1, 2, 3]])],
            ..Geometry::default()
        },
        ..Doc::default()
    };
    let (m, _, _) = read(&write(&doc).expect("writes"));
    let g = &m.root;
    assert_eq!(g.edges(), 5);
    assert_eq!(g.faces(), 1);
    for i in 0..g.edges() {
        let (p, q) = (g.vertex(g.edge_v[i * 2]), g.vertex(g.edge_v[i * 2 + 1]));
        let e = edges
            .iter()
            .find(|e| vertices[e.a as usize] == p && vertices[e.b as usize] == q)
            .expect("the edge is one of ours, in its own direction");
        let mut want = 0;
        if e.hidden {
            want |= HIDDEN;
        }
        if e.soft {
            want |= SOFT;
        }
        if e.smooth {
            want |= SMOOTH;
        }
        assert_eq!(g.edge_flags[i], want, "edge {e:?}");
        let used = g.coedge_edge.contains(&(i as u32));
        assert_eq!(used, e.a != 4, "only the loose edge is unused");
    }
}

fn textured_doc() -> Doc {
    let png = png_2x2();
    // Not a decodable JPEG; the writer only looks at the magic bytes.
    let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4, 0xff, 0xd9];
    let materials = vec![
        Material {
            name: "Red".into(),
            rgba: [200, 10, 20, 255],
            opacity: 0.5,
            texture: None,
        },
        Material {
            name: "Brick".into(),
            rgba: [120, 60, 30, 255],
            opacity: 1.0,
            texture: Some(Texture {
                bytes: png,
                width: 10.0,
                height: 20.0,
            }),
        },
        Material {
            name: "Photo".into(),
            rgba: [90, 90, 90, 255],
            opacity: 0.25,
            texture: Some(Texture {
                bytes: jpeg,
                width: 48.0,
                height: 36.0,
            }),
        },
    ];
    // Face 0 lies flat facing up; face 1 is tilted and carries pinned textures on both
    // sides.
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 10.0, 0.0],
        [0.0, 10.0, 0.0],
        [30.0, 0.0, 0.0],
        [40.0, 0.0, 5.0],
        [40.0, 10.0, 5.0],
        [30.0, 10.0, 0.0],
    ];
    let mut faces = vec![face(vec![vec![0, 1, 2, 3]]), face(vec![vec![4, 5, 6, 7]])];
    faces[0].front = Some(0);
    faces[0].back = Some(1);
    faces[1].front = Some(1);
    faces[1].back = Some(2);
    faces[1].front_uv = Some([
        ([30.0, 0.0, 0.0], [0.0, 0.0]),
        ([40.0, 0.0, 5.0], [1.5, 0.25]),
        ([30.0, 10.0, 0.0], [-0.5, 2.0]),
    ]);
    faces[1].back_uv = Some([
        ([30.0, 0.0, 0.0], [0.0, 0.0]),
        ([40.0, 10.0, 5.0], [1.0, 1.0]),
        ([30.0, 10.0, 0.0], [0.0, 1.0]),
    ]);
    // A horizontal face looking up with front pins, the basis both readers agree on.
    let k = vertices.len() as u32;
    let mut more = vertices.clone();
    more.extend_from_slice(&[
        [50.0, 0.0, 1.0],
        [60.0, 0.0, 1.0],
        [60.0, 10.0, 1.0],
        [50.0, 10.0, 1.0],
    ]);
    let mut up = face(vec![vec![k, k + 1, k + 2, k + 3]]);
    up.front = Some(1);
    up.front_uv = Some([
        ([50.0, 0.0, 1.0], [0.25, 0.5]),
        ([60.0, 0.0, 1.0], [1.25, 0.5]),
        ([50.0, 10.0, 1.0], [0.25, 1.0]),
    ]);
    faces.push(up);
    Doc {
        materials,
        tags: Vec::new(),
        definitions: Vec::new(),
        root: Geometry {
            vertices: more,
            edges: edges_of(&faces),
            faces,
            ..Geometry::default()
        },
    }
}

#[test]
fn materials_and_textures() {
    let doc = textured_doc();
    let bytes = write(&doc).expect("writes");
    let (m, index, _) = read(&bytes);
    assert_eq!(index.materials.len(), 3);
    let by_name = |name: &str| {
        index
            .materials
            .iter()
            .position(|x| x.name == name)
            .expect("listed")
    };
    let (red, brick, photo) = (by_name("Red"), by_name("Brick"), by_name("Photo"));

    let r = &index.materials[red];
    assert_eq!(r.rgba, [200, 10, 20, 255]);
    assert!((r.opacity - 0.5).abs() < 1e-6);
    assert!(r.texture.is_none());

    for (i, want) in [(brick, &doc.materials[1]), (photo, &doc.materials[2])] {
        let got = &index.materials[i];
        assert_eq!(got.rgba[..3], want.rgba[..3], "{}", want.name);
        assert!(!got.colorized, "{}", want.name);
        assert!(
            (got.opacity as f64 - want.opacity).abs() < 1e-6,
            "{}",
            want.name
        );
        let t = got.texture.as_ref().expect("textured");
        let TextureLocation::Inline { offset, len } = t.location else {
            panic!("inline texture expected");
        };
        let tex = want.texture.as_ref().expect("textured input");
        assert_eq!(
            &bytes[offset as usize..(offset + len) as usize],
            &tex.bytes[..]
        );
        assert!((t.tile_mm[0] - tex.width * 25.4).abs() < 1e-9);
        assert!((t.tile_mm[1] - tex.height * 25.4).abs() < 1e-9);
    }

    let g = &m.root;
    assert_eq!(g.faces(), 3);
    let mat = |f: usize| (g.face_material[f], g.face_back_material[f]);
    assert_eq!(mat(0), (red as u32, brick as u32));
    assert_eq!(mat(1), (brick as u32, photo as u32));
    assert_eq!(mat(2), (brick as u32, skp::model::NO_MATERIAL));

    // Face 0 is unpinned; faces 1 and 2 map their pins to the pinned coordinates, in
    // tiles of the front texture.
    assert!(g.uv_matrix(0).is_none());
    for f in [1, 2] {
        let map = g.uv_map(f);
        assert!(!map.is_default());
        for (p, uv) in doc.root.faces[f].front_uv.expect("pinned") {
            let got = map.at(p);
            let tiles = [got[0] / 10.0, got[1] / 20.0];
            assert!(
                (tiles[0] - uv[0]).abs() <= 1e-9 && (tiles[1] - uv[1]).abs() <= 1e-9,
                "face {f}: {p:?} maps to {tiles:?}, not {uv:?}"
            );
        }
    }
}

/// A box definition placed three times by a second definition, which the root places
/// twice as a group. The second definition is listed first.
fn nested_doc() -> Doc {
    let a = Definition {
        name: "Box".into(),
        is_group: false,
        geometry: boxed([4.0, 5.0, 6.0]),
    };
    let place = |rotation, translation, material| Instance {
        definition: 1,
        rotation,
        translation,
        material,
        tag: None,
        hidden: false,
    };
    let b = Definition {
        name: "Shelf".into(),
        is_group: true,
        geometry: Geometry {
            instances: vec![
                place(
                    [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                    [10.0, 0.0, 0.0],
                    None,
                ),
                place(
                    [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                    [0.0, 20.0, 0.0],
                    Some(0),
                ),
                place(
                    [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 0.5]],
                    [0.0, 0.0, 30.0],
                    None,
                ),
            ],
            ..Geometry::default()
        },
    };
    let group = |translation| Instance {
        definition: 0,
        rotation: IDENTITY,
        translation,
        material: None,
        tag: None,
        hidden: false,
    };
    Doc {
        materials: vec![Material {
            name: "Oak".into(),
            rgba: [150, 110, 60, 255],
            opacity: 1.0,
            texture: None,
        }],
        tags: Vec::new(),
        definitions: vec![b, a],
        root: Geometry {
            instances: vec![group([0.0, 0.0, 0.0]), group([100.0, 0.0, 0.0])],
            ..Geometry::default()
        },
    }
}

#[test]
fn components_groups_and_nesting() {
    let doc = nested_doc();
    let (m, index, _) = read(&write(&doc).expect("writes"));
    assert_eq!(m.definitions.len(), 2, "the box is written once");
    let def = |name: &str| {
        m.definitions
            .iter()
            .find(|d| d.name == name)
            .expect("written")
    };
    let (boxd, shelf) = (def("Box"), def("Shelf"));
    assert_eq!(boxd.geom.faces(), 6);
    assert_eq!(boxd.geom.edges(), 12);
    assert_eq!(boxd.geom.vertices(), 8);
    assert!(boxd.geom.instances.is_empty());
    assert_eq!(shelf.geom.faces(), 0);
    assert_ne!(boxd.guid, shelf.guid);

    assert_eq!(shelf.geom.instances.len(), 3);
    for (got, want) in shelf
        .geom
        .instances
        .iter()
        .zip(&doc.definitions[0].geometry.instances)
    {
        assert_eq!(got.def_pid, boxd.pid);
        assert!(!got.is_group);
        assert_eq!(got.m, terms(want));
        assert_eq!(got.has_material, want.material.is_some());
    }
    let oak = index
        .materials
        .iter()
        .position(|x| x.name == "Oak")
        .expect("listed");
    assert_eq!(shelf.geom.instances[1].material, oak as u64);

    assert_eq!(m.root.instances.len(), 2);
    for (got, want) in m.root.instances.iter().zip(&doc.root.instances) {
        assert_eq!(got.def_pid, shelf.pid);
        assert!(got.is_group);
        assert_eq!(got.m, terms(want));
    }
    // Every box face still faces out.
    let mut ring = Vec::new();
    let g = &boxd.geom;
    for f in 0..g.faces() {
        assert!(g.loop_ring(g.face_loop_ofs[f] as usize, &mut ring));
        let c = ring.iter().fold([0.0; 3], |acc, v| {
            let p = g.vertex(*v);
            [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]]
        });
        let c = [c[0] / 4.0 - 2.0, c[1] / 4.0 - 2.5, c[2] / 4.0 - 3.0];
        assert!(dot(g.normal(f), c) > 0.0, "face {f} faces out");
    }
}

#[test]
fn guides() {
    let mut root = one_edge_root();
    root.guides = vec![
        Guide::Line {
            point: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 2.0],
            tag: None,
            hidden: false,
        },
        Guide::Point {
            position: [4.0, 5.0, 6.0],
            tag: None,
            hidden: false,
        },
        Guide::Line {
            point: [-1.0, 0.5, 0.0],
            direction: [3.0, 4.0, 0.0],
            tag: None,
            hidden: false,
        },
    ];
    let (m, _, stats) = read(
        &write(&Doc {
            root,
            ..Doc::default()
        })
        .expect("writes"),
    );
    assert_eq!(stats.hidden_guides, 0);
    assert_eq!(
        m.root.guides,
        vec![
            skp::geom::RawGuide::Line {
                point: [1.0, 2.0, 3.0],
                direction: [0.0, 0.0, 1.0],
            },
            skp::geom::RawGuide::Point {
                position: [4.0, 5.0, 6.0],
            },
            skp::geom::RawGuide::Line {
                point: [-1.0, 0.5, 0.0],
                direction: [0.6, 0.8, 0.0],
            },
        ]
    );
}

#[test]
fn a_guide_line_can_end_a_definition() {
    let mut inner = one_edge_root();
    inner.guides = vec![Guide::Line {
        point: [0.0, 0.0, 1.0],
        direction: [1.0, 0.0, 0.0],
        tag: None,
        hidden: false,
    }];
    let mut root = one_edge_root();
    root.instances = vec![Instance {
        definition: 0,
        rotation: IDENTITY,
        translation: [0.0; 3],
        material: None,
        tag: None,
        hidden: false,
    }];
    root.guides = vec![
        Guide::Line {
            point: [0.0, 0.0, 2.0],
            direction: [0.0, 1.0, 0.0],
            tag: None,
            hidden: false,
        },
        Guide::Point {
            position: [1.0, 1.0, 1.0],
            tag: None,
            hidden: false,
        },
    ];
    let doc = Doc {
        materials: Vec::new(),
        tags: Vec::new(),
        definitions: vec![Definition {
            name: "Marked".into(),
            is_group: false,
            geometry: inner,
        }],
        root,
    };
    let (m, _, _) = read(&write(&doc).expect("writes"));
    assert_eq!(m.definitions[0].geom.guides.len(), 1);
    assert_eq!(m.root.guides.len(), 2);
    assert_eq!(m.root.instances.len(), 1);
}

#[test]
fn a_tiny_face_keeps_its_texture_pins() {
    // A face 0.01 inch across with its texture pinned at 1000 times its projected size:
    // the pins span 1e-5 inch of texture, and are judged by their shape, not their size.
    let (s, k) = (0.01, 0.001);
    let doc = Doc {
        materials: vec![Material {
            name: "tex".into(),
            rgba: [1, 2, 3, 255],
            opacity: 1.0,
            texture: Some(Texture {
                bytes: png_2x2(),
                width: 100.0,
                height: 100.0,
            }),
        }],
        root: Geometry {
            vertices: vec![[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0]],
            edges: vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 0)],
            faces: vec![Face {
                front: Some(0),
                front_uv: Some([
                    ([0.0, 0.0, 0.0], [0.0, 0.0]),
                    ([s, 0.0, 0.0], [k * s / 100.0, 0.0]),
                    ([s, s, 0.0], [k * s / 100.0, k * s / 100.0]),
                ]),
                ..face(vec![vec![0, 1, 2, 3]])
            }],
            ..Geometry::default()
        },
        ..Doc::default()
    };
    let (m, _, _) = read(&write(&doc).expect("writes"));
    let map = m.root.uv_map(0);
    assert!(!map.is_default());
    let got = map.at([s, s, 0.0]);
    assert!(
        (got[0] - k * s).abs() < 1e-15 && (got[1] - k * s).abs() < 1e-15,
        "{got:?}"
    );
}

#[test]
fn many_entities_cross_the_slot_escape() {
    const N: usize = 15_000;
    let mut root = Geometry::default();
    for i in 0..N {
        let (x, y) = ((i % 200) as f64 * 3.0, (i / 200) as f64 * 3.0);
        let k = root.vertices.len() as u32;
        root.vertices
            .extend_from_slice(&[[x, y, 0.0], [x + 1.0, y, 0.0], [x, y + 1.0, 0.0]]);
        root.edges
            .extend_from_slice(&[edge(k, k + 1), edge(k + 1, k + 2), edge(k + 2, k)]);
        root.faces.push(face(vec![vec![k, k + 1, k + 2]]));
    }
    // Classes first declared past slot 0x7fff, then referred to by the escaped form.
    root.guides = vec![
        Guide::Point {
            position: [1.0, 2.0, 3.0],
            tag: None,
            hidden: false,
        },
        Guide::Point {
            position: [4.0, 5.0, 6.0],
            tag: None,
            hidden: false,
        },
    ];
    root.instances = vec![Instance {
        definition: 0,
        rotation: IDENTITY,
        translation: [0.0; 3],
        material: None,
        tag: None,
        hidden: false,
    }];
    // 254 UTF-16 units, two of them a surrogate pair.
    let name: String = core::iter::once('\u{10400}')
        .chain(core::iter::repeat_n('x', 252))
        .collect();
    assert_eq!(name.encode_utf16().count(), 254);
    let doc = Doc {
        materials: Vec::new(),
        tags: Vec::new(),
        definitions: vec![Definition {
            name: name.clone(),
            is_group: false,
            geometry: one_edge_root(),
        }],
        root,
    };
    let bytes = write(&doc).expect("writes");
    let (m, _, _) = read(&bytes);
    assert_eq!(m.root.faces(), N);
    assert_eq!(m.root.edges(), 3 * N);
    assert_eq!(m.root.vertices(), 3 * N);
    assert_eq!(m.root.loops(), N);
    assert_eq!(m.root.guides.len(), 2);
    assert_eq!(m.root.instances.len(), 1);
    assert_eq!(m.definitions.len(), 1);
    assert_eq!(m.definitions[0].name, name);
    assert_eq!(m.root.instances[0].def_pid, m.definitions[0].pid);
    // The last triangle is where it was put.
    let mut ring = Vec::new();
    assert!(m.root.loop_ring(N - 1, &mut ring));
    let want = &doc.root.vertices[3 * (N - 1)..];
    let got: Vec<[f64; 3]> = ring.iter().map(|v| m.root.vertex(*v)).collect();
    assert_eq!(got, want);

    // Every tail reference moved past 0x7fff and widened to the escape, six times four
    // bytes, and the Iso camera patch after the first four landed where it moved to.
    let tail = &bytes[bytes.len() - (TAIL_LEN + 24)..];
    assert_eq!(&tail[409..411], &[0xff, 0x7f]);
    let big = u32::from_le_bytes([tail[411], tail[412], tail[413], tail[414]]);
    assert!(
        big & 0x8000_0000 != 0 && big & 0x7fff_ffff > 0x7fff,
        "{big:#x}"
    );
    assert_eq!(&tail[509 + 16..509 + 32], &ISO_TAIL_PATCH);
}

#[test]
fn the_same_document_gives_the_same_bytes() {
    let mut doc = nested_doc();
    let textured = textured_doc();
    doc.materials.extend(textured.materials);
    doc.root = textured.root;
    for f in &mut doc.root.faces {
        for m in [&mut f.front, &mut f.back].into_iter().flatten() {
            *m += 1;
        }
    }
    doc.root.instances = nested_doc().root.instances;
    doc.root.guides = vec![Guide::Point {
        position: [1.0, 1.0, 1.0],
        tag: None,
        hidden: false,
    }];
    let first = write(&doc).expect("writes");
    let second = write(&doc.clone()).expect("writes");
    assert_eq!(first, second);
    read(&first);
}

#[test]
fn invalid_documents_are_errors() {
    let base = || Doc {
        root: one_edge_root(),
        ..Doc::default()
    };
    let square = || Geometry {
        vertices: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        edges: vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 0)],
        faces: vec![face(vec![vec![0, 1, 2, 3]])],
        ..Geometry::default()
    };
    let place = |definition| Instance {
        definition,
        rotation: IDENTITY,
        translation: [0.0; 3],
        material: None,
        tag: None,
        hidden: false,
    };
    let solid = |name: String| Material {
        name,
        rgba: [1, 2, 3, 255],
        opacity: 1.0,
        texture: None,
    };

    type Check = fn(&WriteError) -> bool;
    let mut cases: Vec<(&str, Doc, Check)> = Vec::new();

    let mut d = base();
    d.root = square();
    d.root.faces[0].loops[0].truncate(2);
    cases.push(("short loop", d, |e| {
        matches!(
            e,
            WriteError::ShortLoop {
                owner: Owner::Root,
                face: 0
            }
        )
    }));

    let mut d = base();
    d.root.edges[0].b = 9;
    cases.push(("vertex out of range", d, |e| {
        matches!(e, WriteError::VertexOutOfRange { index: 9, .. })
    }));

    let mut d = base();
    d.root = square();
    d.root.edges.pop();
    cases.push(("missing edge", d, |e| {
        matches!(e, WriteError::MissingEdge { a: 3, b: 0, .. })
    }));

    let mut d = base();
    d.definitions = vec![
        Definition {
            name: "A".into(),
            is_group: false,
            geometry: Geometry {
                instances: vec![place(1)],
                ..Geometry::default()
            },
        },
        Definition {
            name: "B".into(),
            is_group: false,
            geometry: Geometry {
                instances: vec![place(0)],
                ..Geometry::default()
            },
        },
    ];
    cases.push(("recursive definition", d, |e| {
        matches!(e, WriteError::RecursiveDefinition { .. })
    }));

    let mut d = base();
    d.root.instances.push(place(5));
    cases.push(("definition out of range", d, |e| {
        matches!(e, WriteError::DefinitionOutOfRange { definition: 5, .. })
    }));

    let mut d = base();
    d.definitions.push(Definition::default());
    d.root.instances.push(place(0));
    cases.push(("empty definition", d, |e| {
        matches!(e, WriteError::EmptyDefinition { definition: 0, .. })
    }));

    let mut d = base();
    d.root = square();
    d.root.faces[0].back = Some(3);
    cases.push(("material out of range", d, |e| {
        matches!(e, WriteError::MaterialOutOfRange { index: 3 })
    }));

    let mut d = base();
    d.materials.push(Material {
        texture: Some(Texture {
            bytes: b"GIF89a".to_vec(),
            width: 1.0,
            height: 1.0,
        }),
        ..solid("gif".into())
    });
    cases.push(("texture format", d, |e| {
        matches!(e, WriteError::UnknownTextureFormat { material: 0 })
    }));

    let mut d = base();
    d.materials.push(solid("x".repeat(255)));
    cases.push(("long string", d, |e| {
        matches!(e, WriteError::StringTooLong { units: 255, .. })
    }));

    let mut d = base();
    d.root.vertices[1][2] = f64::NAN;
    cases.push(("non-finite", d, |e| {
        matches!(e, WriteError::NonFinite { .. })
    }));

    let mut d = base();
    d.root.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    d.root.edges = vec![edge(0, 1), edge(1, 2), edge(2, 0)];
    d.root.faces = vec![face(vec![vec![0, 1, 2]])];
    cases.push(("degenerate face", d, |e| {
        matches!(e, WriteError::Degenerate { what: "face", .. })
    }));

    cases.push(("empty document", Doc::default(), |e| {
        matches!(e, WriteError::EmptyDocument)
    }));

    let mut d = base();
    d.materials = (0..0x7fff).map(|i| solid(format!("m{i}"))).collect();
    cases.push(("too many materials", d, |e| {
        matches!(e, WriteError::TooManyMaterials)
    }));

    let mut d = base();
    d.root = square();
    d.materials.push(Material {
        texture: Some(Texture {
            bytes: png_2x2(),
            width: 1.0,
            height: 1.0,
        }),
        ..solid("tex".into())
    });
    d.root.faces[0].front = Some(0);
    d.root.faces[0].front_uv = Some([
        ([0.0, 0.0, 0.0], [0.0, 0.0]),
        ([1.0, 0.0, 0.0], [1.0, 1.0]),
        ([1.0, 1.0, 0.0], [2.0, 2.0]),
    ]);
    cases.push(("pins on one line", d, |e| {
        matches!(
            e,
            WriteError::Degenerate {
                what: "texture pins of face",
                ..
            }
        )
    }));

    let mut d = base();
    d.root = square();
    d.root.edges.push(edge(1, 0));
    cases.push(("duplicate edge", d, |e| {
        matches!(
            e,
            WriteError::Degenerate {
                what: "duplicate edge",
                index: 4,
                ..
            }
        )
    }));

    let mut d = base();
    d.root = square();
    d.root.vertices.push([2.0, 0.0, 0.0]);
    d.root.edges = vec![
        edge(0, 1),
        edge(1, 4),
        edge(4, 2),
        edge(2, 1),
        edge(2, 3),
        edge(3, 0),
    ];
    d.root.faces = vec![face(vec![vec![0, 1, 4, 2, 1, 2, 3]])];
    cases.push(("loop repeating a vertex", d, |e| {
        matches!(
            e,
            WriteError::Degenerate {
                what: "face loop",
                ..
            }
        )
    }));

    for size in [0.0, -1.0] {
        let mut d = base();
        d.materials.push(Material {
            texture: Some(Texture {
                bytes: png_2x2(),
                width: size,
                height: 1.0,
            }),
            ..solid("tex".into())
        });
        cases.push(("texture size", d, |e| {
            matches!(e, WriteError::TextureSize { material: 0 })
        }));
    }

    let mut d = base();
    d.materials = vec![solid("same".into()), solid("same".into())];
    cases.push((
        "duplicate material name",
        d,
        |e| matches!(e, WriteError::DuplicateName { what: "material", name } if name == "same"),
    ));

    let mut d = base();
    let named = |name: &str| Definition {
        name: name.into(),
        is_group: true,
        geometry: one_edge_root(),
    };
    d.definitions = vec![named("Group"), named("Group")];
    cases.push((
        "duplicate definition name",
        d,
        |e| matches!(e, WriteError::DuplicateName { what: "definition", name } if name == "Group"),
    ));

    let mut d = base();
    d.definitions = vec![named("Flat")];
    let mut flat = place(0);
    flat.rotation[2] = [0.0; 3];
    d.root.instances.push(flat);
    cases.push(("singular instance", d, |e| {
        matches!(
            e,
            WriteError::Degenerate {
                what: "instance",
                index: 0,
                ..
            }
        )
    }));

    for (name, doc, check) in cases {
        match write(&doc) {
            Ok(_) => panic!("{name}: wrote"),
            Err(e) => {
                assert!(check(&e), "{name}: {e:?}");
                assert!(!e.to_string().is_empty());
            }
        }
    }
}

#[test]
fn the_scaffold_is_openskps() {
    let blank = skpwrite::SCAFFOLD;
    assert_eq!(
        hex(&sha256(blank)),
        "809a1ab73a20a192ab13aaff197afb1c67d0e9352f6a353a9cd8030919f8a6c3"
    );
    assert_eq!(blank.len(), 5796);
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    // The patch points, as the writer uses them: an empty material count, the layer
    // count, Layer0's class declaration, the active-layer anchor (slot 10, Layer0), the
    // definition and root counts, and the persistent-id counter.
    assert_eq!(u32_at(blank, 3396), 0);
    assert_eq!(u32_at(blank, 3405), 1);
    assert_eq!(&blank[3409..3411], &[0xff, 0xff]);
    assert_eq!(&blank[3415..3421], b"CLayer");
    assert_eq!(u16::from_le_bytes([blank[3505], blank[3506]]), 10);
    assert_eq!(u32_at(blank, 3507), 0);
    assert_eq!(u32_at(blank, 3511), 0);
    assert_eq!(u32_at(blank, 1987), 18);

    // The reader walks the blank as an empty model numbered from slot 9, the base the
    // writer's material section starts at.
    let (m, index, stats) = read(blank);
    assert_eq!(stats.slot_base, 9);
    assert_eq!(m.root.edges() + m.root.faces() + m.root.instances.len(), 0);
    assert!(m.definitions.is_empty());
    assert!(index.materials.is_empty());
    assert_eq!(index.layers.len(), 1);

    // One edge and its two vertices take persistent ids 19, 20 and 21; the counter holds
    // the last one handed out.
    let one = write(&Doc {
        root: one_edge_root(),
        ..Doc::default()
    })
    .expect("writes");
    assert_eq!(u32_at(&one, 1987), 21);
    assert_eq!(u32_at(&one, 3511), 1, "one root entity");
    // The tail's first slot reference (a class reference to slot 11) moved by the five
    // slots written: CEdge, the edge, CVertex, two vertices. The Iso camera is in place.
    let tail = &one[one.len() - TAIL_LEN..];
    assert_eq!(
        u16::from_le_bytes([tail[409], tail[410]]),
        0x8000 | (11 + 5)
    );
    assert_eq!(&tail[509..525], &ISO_TAIL_PATCH);
}
