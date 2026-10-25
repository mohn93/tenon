//! Import trees: definitions with faces, holes, soft and loose edges, guides and nested
//! instances, built into the scene with shared definitions.

use glam::{DAffine3, DMat3, DQuat, DVec3};
use kernel::{
    DefinitionId, DefinitionKind, Guide, ImportDefinition, ImportEdge, ImportFace, ImportGeometry,
    ImportInstance, ImportTree, KernelError, MAX_IMPORT_DEPTH, MAX_IMPORT_PLACED_FACES,
    MAX_IMPORT_PLACEMENTS, MeshId, MeshPolygon, Model, Scene, TexturePlacement, polygon_geometry,
};

fn face(loops: &[&[usize]]) -> ImportFace {
    ImportFace {
        loops: loops.iter().map(|l| l.to_vec()).collect(),
        front: None,
        back: None,
        texture: None,
        tag: None,
        hidden: false,
    }
}

fn edge(a: usize, b: usize, soft: bool) -> ImportEdge {
    ImportEdge {
        a,
        b,
        soft,
        tag: None,
        hidden: false,
    }
}

fn place(definition: usize, transform: DAffine3) -> ImportInstance {
    ImportInstance {
        definition,
        transform,
        material: None,
        tag: None,
        hidden: false,
    }
}

fn definition(name: &str, geometry: ImportGeometry) -> ImportDefinition {
    ImportDefinition {
        name: name.to_string(),
        kind: DefinitionKind::Component,
        geometry,
    }
}

fn import(s: &mut Scene, tree: &ImportTree) -> Result<kernel::TreeReport, KernelError> {
    s.import_tree(MeshId::Root, tree, DefinitionKind::Group, "Tree")
        .map(|(_, report, _)| report)
}

/// Our definition named `name`.
fn ours(s: &Scene, name: &str) -> DefinitionId {
    s.definitions()
        .find(|(_, d)| d.name == name)
        .unwrap_or_else(|| panic!("no definition {name}"))
        .0
}

fn mesh<'a>(s: &'a Scene, name: &str) -> &'a Model {
    &s.definition(ours(s, name)).unwrap().mesh
}

/// A unit box: 8 corners, 6 quads wound outward.
fn unit_box() -> ImportGeometry {
    let points = (0..8)
        .map(|i| DVec3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64))
        .collect();
    let quads: [&[usize]; 6] = [
        &[0, 2, 3, 1],
        &[4, 5, 7, 6],
        &[0, 1, 5, 4],
        &[2, 6, 7, 3],
        &[0, 4, 6, 2],
        &[1, 3, 7, 5],
    ];
    ImportGeometry {
        points,
        faces: quads.iter().map(|q| face(&[q])).collect(),
        ..Default::default()
    }
}

/// A root that places definition 0 once.
fn root_placing_0() -> ImportGeometry {
    ImportGeometry {
        instances: vec![place(0, DAffine3::IDENTITY)],
        ..Default::default()
    }
}

#[test]
fn a_face_with_a_hole_and_soft_edges() {
    let p = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
    let points = vec![
        // Outer square, clockwise seen from +z.
        p(0.0, 0.0, 0.0),
        p(0.0, 2.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(2.0, 0.0, 0.0),
        // Hole, clockwise seen from +z too.
        p(0.5, 0.5, 0.0),
        p(0.5, 1.5, 0.0),
        p(1.5, 1.5, 0.0),
        p(1.5, 0.5, 0.0),
        // The folded face's top.
        p(2.0, 2.0, 2.0),
        p(2.0, 0.0, 2.0),
        // A loose edge.
        p(5.0, 5.0, 5.0),
        p(6.0, 5.0, 5.0),
    ];
    let root = ImportGeometry {
        points,
        faces: vec![
            face(&[&[0, 1, 2, 3], &[4, 5, 6, 7]]),
            face(&[&[3, 2, 8, 9]]),
        ],
        edges: vec![
            edge(2, 3, true),   // between the two faces
            edge(3, 2, false),  // the same edge again
            edge(10, 11, true), // loose: stays hard
            edge(0, 1, false),
        ],
        guides: vec![
            Guide::Point {
                position: DVec3::ONE,
                tag: None,
                hidden: false,
            },
            Guide::Line {
                point: DVec3::ZERO,
                direction: DVec3::ZERO,
                tag: None,
                hidden: false,
            },
        ],
        instances: vec![],
    };
    let tree = ImportTree {
        definitions: vec![],
        root,
    };
    let mut s = Scene::new();
    let (instance, report, change) = s
        .import_tree(MeshId::Root, &tree, DefinitionKind::Group, "Tree")
        .unwrap();
    s.validate().unwrap();
    // Undo and redo give back the same scene.
    let built = s.clone();
    s.apply(&change.inverse());
    assert_eq!(s, Scene::new());
    s.apply(&change);
    s.validate().unwrap();
    assert_eq!(s, built);
    assert!(s.root().instance(instance).is_some());
    assert_eq!(report.faces, 2);
    assert_eq!(report.vertices, 12);
    assert_eq!((report.skipped, report.split, report.hard), (0, 0, 1));
    assert_eq!(report.skipped_guides, 1);
    let m = mesh(&s, "Tree");
    m.validate().unwrap();
    let (_, holed) = m.faces().find(|(_, f)| f.holes.len() == 1).unwrap();
    // The outer loop was given clockwise seen from +z, so the face looks down.
    assert_eq!(holed.normal, -DVec3::Z);
    let pts: Vec<DVec3> = holed
        .outer
        .iter()
        .map(|v| m.position(*v).unwrap())
        .collect();
    assert!(kernel::geom::vector_area(&pts).dot(holed.normal) > 0.0);
    let hole: Vec<DVec3> = holed.holes[0]
        .iter()
        .map(|v| m.position(*v).unwrap())
        .collect();
    assert!(kernel::geom::vector_area(&hole).dot(holed.normal) < 0.0);
    // The shared edge is soft, the others hard; the loose edge exists.
    let at = |q: DVec3| {
        m.vertices()
            .find(|(_, v)| v.pos.distance(q) < 1e-9)
            .unwrap()
            .0
    };
    let shared = m
        .edge_between(at(p(2.0, 2.0, 0.0)), at(p(2.0, 0.0, 0.0)))
        .unwrap();
    assert!(m.edge(shared).unwrap().soft);
    assert_eq!(m.edges().filter(|(_, e)| e.soft).count(), 1);
    assert!(
        m.edge_between(at(p(5.0, 5.0, 5.0)), at(p(6.0, 5.0, 5.0)))
            .is_some()
    );
    // 4 + 4 + 3 face sides and the loose edge.
    assert_eq!(m.edges().count(), 12);
    assert_eq!(m.guides().count(), 1);
}

#[test]
fn components_are_shared_and_nested() {
    let turn = DAffine3::from_rotation_translation(
        DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
        DVec3::new(3.0, 0.0, 0.0),
    );
    let mirror = DAffine3::from_scale_rotation_translation(
        DVec3::new(-1.0, 1.0, 1.0),
        DQuat::IDENTITY,
        DVec3::new(6.0, 0.0, 0.0),
    );
    let stretch = DAffine3::from_scale(DVec3::new(2.0, 1.0, 1.0));
    let row = ImportGeometry {
        instances: vec![place(0, turn), place(0, mirror), place(0, stretch)],
        ..Default::default()
    };
    let up = DAffine3::from_translation(DVec3::new(0.0, 0.0, 5.0));
    let tree = ImportTree {
        definitions: vec![definition("Box", unit_box()), definition("Row", row)],
        root: ImportGeometry {
            instances: vec![place(1, DAffine3::IDENTITY), place(1, up)],
            ..Default::default()
        },
    };
    let mut s = Scene::new();
    let (_, report, change) = s
        .import_tree(MeshId::Root, &tree, DefinitionKind::Group, "Tree")
        .unwrap();
    s.validate().unwrap();
    assert_eq!(s.definitions().count(), 3);
    assert_eq!((report.definitions, report.instances), (2, 5));
    assert_eq!((report.faces, report.vertices, report.unused), (6, 8, 0));
    let (boxed, row) = (ours(&s, "Box"), ours(&s, "Row"));
    assert_eq!(s.instance_count(boxed), 3);
    assert_eq!(s.instance_count(row), 2);
    assert_eq!(s.instance_count(ours(&s, "Tree")), 1);
    assert_eq!(s.definition(boxed).unwrap().kind, DefinitionKind::Component);
    // One box mesh, as given: six faces facing out of the unit cube.
    let m = mesh(&s, "Box");
    assert_eq!((m.faces().count(), m.edges().count()), (6, 12));
    let centre = DVec3::splat(0.5);
    for (id, f) in m.faces() {
        let out = m.position(f.outer[0]).unwrap() - centre;
        assert!(f.normal.dot(out) > 0.0, "{id:?} faces in");
        assert_eq!(f.outer.len(), 4);
    }
    // The row places the box with the transforms as given; the mirrored one keeps det < 0.
    let placed: Vec<DAffine3> = mesh(&s, "Row")
        .instances()
        .map(|(_, i)| {
            assert_eq!(i.definition, boxed);
            i.transform
        })
        .collect();
    assert_eq!(placed, vec![turn, mirror, stretch]);
    assert!(mirror.matrix3.determinant() < 0.0);
    let wrapper = mesh(&s, "Tree");
    assert_eq!(wrapper.instances().count(), 2);
    assert!(wrapper.instances().all(|(_, i)| i.definition == row));
    // One step undoes it all.
    s.apply(&change.inverse());
    assert_eq!(s, Scene::new());
}

#[test]
fn bad_trees_change_nothing() {
    let invalid = |tree: ImportTree| {
        let mut s = Scene::new();
        let r = import(&mut s, &tree);
        assert!(matches!(r, Err(KernelError::InvalidImport(_))), "{r:?}");
        assert_eq!(s, Scene::new());
    };
    // An instance past the definitions.
    invalid(ImportTree {
        definitions: vec![definition("Box", unit_box())],
        root: ImportGeometry {
            instances: vec![place(1, DAffine3::IDENTITY)],
            ..Default::default()
        },
    });
    // A cycle: 0 places 1, 1 places 0.
    let placing = |d: usize| ImportGeometry {
        instances: vec![place(d, DAffine3::IDENTITY)],
        ..unit_box()
    };
    invalid(ImportTree {
        definitions: vec![definition("A", placing(1)), definition("B", placing(0))],
        root: root_placing_0(),
    });
    // A chain deeper than the limit, and one at the limit.
    let chain = |n: usize| ImportTree {
        definitions: (0..n)
            .map(|i| {
                let mut g = unit_box();
                if i + 1 < n {
                    g.instances.push(place(i + 1, DAffine3::IDENTITY));
                }
                definition(&format!("D{i}"), g)
            })
            .collect(),
        root: root_placing_0(),
    };
    invalid(chain(MAX_IMPORT_DEPTH + 1));
    let mut s = Scene::new();
    let report = import(&mut s, &chain(MAX_IMPORT_DEPTH)).unwrap();
    assert_eq!(report.definitions, MAX_IMPORT_DEPTH);
    s.validate().unwrap();
    // A singular transform and a transform that is not finite.
    for t in [
        DAffine3::from_scale(DVec3::new(1.0, 1.0, 0.0)),
        DAffine3::from_mat3(DMat3::from_diagonal(DVec3::new(1.0, f64::INFINITY, 1.0))),
    ] {
        invalid(ImportTree {
            definitions: vec![definition("Box", unit_box())],
            root: ImportGeometry {
                instances: vec![place(0, t)],
                ..Default::default()
            },
        });
    }
    // A point that is not finite.
    let mut bad = unit_box();
    bad.points[3].x = f64::NAN;
    let mut s = Scene::new();
    let r = import(
        &mut s,
        &ImportTree {
            definitions: vec![],
            root: bad,
        },
    );
    assert_eq!(r.unwrap_err(), KernelError::NotFinite);
    assert_eq!(s, Scene::new());
    // A material the scene lacks.
    let mut painted = unit_box();
    painted.faces[0].front = Some(kernel::MaterialId(99));
    let r = import(
        &mut s,
        &ImportTree {
            definitions: vec![],
            root: painted,
        },
    );
    assert_eq!(r.unwrap_err(), KernelError::MaterialNotFound);
    assert_eq!(s, Scene::new());
    // Nothing at all.
    let r = import(&mut s, &ImportTree::default());
    assert_eq!(r.unwrap_err(), KernelError::NothingToImport);
    assert_eq!(s, Scene::new());
}

#[test]
fn unused_definitions_are_not_built() {
    let tree = ImportTree {
        definitions: vec![
            definition("Box", unit_box()),
            definition("Spare", unit_box()),
        ],
        root: root_placing_0(),
    };
    let mut s = Scene::new();
    let report = import(&mut s, &tree).unwrap();
    s.validate().unwrap();
    assert_eq!((report.definitions, report.unused), (1, 1));
    let names: Vec<&str> = s.definitions().map(|(_, d)| d.name.as_str()).collect();
    assert_eq!(names, vec!["Box", "Tree"]);
}

#[test]
fn invalid_faces_are_split_or_skipped() {
    let points = vec![
        DVec3::ZERO,
        DVec3::X,
        DVec3::new(1.0, 1.0, 0.3),
        DVec3::Y,
        // A hole outside the square below.
        DVec3::new(5.0, 5.0, 0.0),
        DVec3::new(6.0, 5.0, 0.0),
        DVec3::new(6.0, 6.0, 0.0),
        // A flat square at z = -1.
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(1.0, 0.0, -1.0),
        DVec3::new(1.0, 1.0, -1.0),
        DVec3::new(0.0, 1.0, -1.0),
    ];
    let root = ImportGeometry {
        points,
        faces: vec![
            face(&[&[0, 1, 2, 3]]),                 // bent: two triangles
            face(&[&[0, 1, 0]]),                    // two distinct points
            face(&[&[0, 1, 99]]),                   // a corner out of range
            face(&[]),                              // no loop at all
            face(&[&[7, 8, 9, 10], &[4, 5, 6]]),    // a hole outside: dropped
            face(&[&[7, 8, 9, 10], &[4, 5, 6, 4]]), // the same face again
        ],
        ..Default::default()
    };
    let mut s = Scene::new();
    let report = import(
        &mut s,
        &ImportTree {
            definitions: vec![],
            root,
        },
    )
    .unwrap();
    s.validate().unwrap();
    assert_eq!(report.faces, 3);
    assert_eq!(report.split, 2);
    assert_eq!(report.skipped, 4);
    let m = mesh(&s, "Tree");
    assert_eq!(m.faces().filter(|(_, f)| f.outer.len() == 3).count(), 2);
    assert!(m.faces().all(|(_, f)| f.holes.is_empty()));
}

#[test]
fn into_root_needs_an_empty_mesh() {
    let tree = ImportTree {
        definitions: vec![definition("Box", unit_box())],
        root: ImportGeometry {
            instances: vec![place(0, DAffine3::from_translation(DVec3::X * 3.0))],
            ..unit_box()
        },
    };
    let mut s = Scene::new();
    let (report, change) = s.import_tree_into_root(MeshId::Root, &tree).unwrap();
    s.validate().unwrap();
    assert_eq!(report.definitions, 1);
    assert_eq!(s.root().faces().count(), 6);
    assert_eq!(s.root().instances().count(), 1);
    assert_eq!(s.definitions().count(), 1);
    s.apply(&change.inverse());
    assert_eq!(s, Scene::new());

    let mut s = Scene::new();
    s.edit(MeshId::Root, |m| m.add_edge(DVec3::ZERO, DVec3::X))
        .unwrap();
    let before = s.clone();
    let r = s.import_tree_into_root(MeshId::Root, &tree);
    assert!(matches!(r, Err(KernelError::InvalidImport(_))), "{r:?}");
    assert_eq!(s, before);
}

/// Definitions 0..levels-1 each placing the next `fan` times; the last holds `leaf`.
fn fan_tree(levels: usize, fan: usize, leaf: ImportGeometry) -> ImportTree {
    let mut definitions: Vec<ImportDefinition> = (0..levels - 1)
        .map(|i| {
            definition(
                &format!("L{i}"),
                ImportGeometry {
                    instances: (0..fan)
                        .map(|k| {
                            place(
                                i + 1,
                                DAffine3::from_translation(DVec3::new(k as f64, 0.0, 0.0)),
                            )
                        })
                        .collect(),
                    ..Default::default()
                },
            )
        })
        .collect();
    definitions.push(definition("Leaf", leaf));
    ImportTree {
        definitions,
        root: root_placing_0(),
    }
}

#[test]
fn trees_resolving_to_too_many_placements_are_refused() {
    let refused = |tree: &ImportTree, what: &str| {
        let mut s = Scene::new();
        match import(&mut s, tree) {
            Err(KernelError::InvalidImport(m)) => assert!(m.contains(what), "{m}"),
            r => panic!("{r:?}"),
        }
        assert_eq!(s, Scene::new());
    };
    // 1 + 10 + ... + 10^6 placements: past MAX_IMPORT_PLACEMENTS, with only 70 instances.
    let tree = fan_tree(8, 10, unit_box());
    refused(&tree, &MAX_IMPORT_PLACEMENTS.to_string());
    // A thousand-face leaf placed 10^5 times: 10^8 faces shown, 111,111 placements.
    let triangle = ImportGeometry {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::Y],
        faces: vec![face(&[&[0, 1, 2]]); 1000],
        ..Default::default()
    };
    let tree = fan_tree(6, 10, triangle);
    refused(&tree, &MAX_IMPORT_PLACED_FACES.to_string());
    // Just under both: 1 + 10 + ... + 10^5 placements of a box.
    let mut s = Scene::new();
    let report = import(&mut s, &fan_tree(6, 10, unit_box())).unwrap();
    assert_eq!(report.instances, 51);
    s.validate().unwrap();
}

fn polygon(corners: &[usize]) -> MeshPolygon {
    MeshPolygon {
        corners: corners.to_vec(),
        material: None,
    }
}

#[test]
fn polygon_geometry_merges_triangles_with_one_texture_placement() {
    let points = [
        DVec3::ZERO,
        DVec3::X,
        DVec3::new(1.0, 1.0, 0.0),
        // The second triangle's corners again, as an exporter writes each triangle's own.
        DVec3::ZERO,
        DVec3::new(1.0, 1.0, 0.0),
        DVec3::Y,
    ];
    let polygons = [polygon(&[0, 1, 2]), polygon(&[3, 4, 5])];
    let turned = TexturePlacement {
        rotation_deg: 30.0,
        ..TexturePlacement::default()
    };
    let nudged = TexturePlacement {
        rotation_deg: 30.0 + 1e-7,
        ..TexturePlacement::default()
    };
    let (g, skipped) =
        polygon_geometry(&points, &polygons, &[Some(turned), Some(nudged)], &[], true).unwrap();
    assert_eq!((g.points.len(), g.faces.len(), skipped), (4, 1, 0));
    let kept = g.faces[0].texture.unwrap();
    assert!((kept.rotation_deg - 30.0).abs() < 1e-6, "{kept:?}");
    assert!(g.edges.is_empty());

    let other = TexturePlacement {
        rotation_deg: 31.0,
        ..TexturePlacement::default()
    };
    let (g, _) =
        polygon_geometry(&points, &polygons, &[Some(turned), Some(other)], &[], true).unwrap();
    assert_eq!(g.faces.len(), 2, "different placements stay apart");
    // The default placement and one out of range are left off.
    let wild = TexturePlacement {
        scale: [0.0, 1.0],
        ..TexturePlacement::default()
    };
    let (g, _) = polygon_geometry(
        &points,
        &polygons,
        &[Some(TexturePlacement::default()), Some(wild)],
        &[],
        true,
    )
    .unwrap();
    assert_eq!(g.faces.len(), 1);
    assert_eq!(g.faces[0].texture, None);

    // The placement reaches the built face.
    let (g, _) =
        polygon_geometry(&points, &polygons, &[Some(turned), Some(turned)], &[], true).unwrap();
    let mut s = Scene::new();
    import(
        &mut s,
        &ImportTree {
            definitions: vec![],
            root: g,
        },
    )
    .unwrap();
    s.validate().unwrap();
    let (_, f) = mesh(&s, "Tree").faces().next().unwrap();
    assert_eq!(f.texture, Some(turned));
}

/// An open cylinder band of `n` quads, each cut into two triangles with corners of their own,
/// and a top cap, with the file's normals: radial on the band, up on the cap.
fn banded(n: usize) -> (Vec<DVec3>, Vec<MeshPolygon>, Vec<DVec3>) {
    let at = |k: usize, z: f64| {
        let a = std::f64::consts::TAU * (k % n) as f64 / n as f64;
        DVec3::new(a.cos(), a.sin(), z)
    };
    let radial = |p: DVec3| DVec3::new(p.x, p.y, 0.0);
    let (mut points, mut polygons, mut normals) = (Vec::new(), Vec::new(), Vec::new());
    let mut add = |corners: [DVec3; 3], normal: &dyn Fn(DVec3) -> DVec3| {
        let start = points.len();
        for c in corners {
            points.push(c);
            normals.push(normal(c));
        }
        polygons.push(polygon(&[start, start + 1, start + 2]));
    };
    for k in 0..n {
        let (a, b, c, d) = (at(k, 0.0), at(k + 1, 0.0), at(k + 1, 1.0), at(k, 1.0));
        add([a, b, c], &radial);
        add([a, c, d], &radial);
        add([DVec3::Z, d, c], &|_| DVec3::Z);
    }
    (points, polygons, normals)
}

#[test]
fn polygon_geometry_softens_smooth_surfaces_and_keeps_creases() {
    let n = 8;
    let (points, polygons, normals) = banded(n);
    let (g, _) = polygon_geometry(&points, &polygons, &[], &normals, true).unwrap();
    // The band's quads come back whole, the cap's triangles as one face.
    assert_eq!(g.faces.len(), n + 1);
    // The seams between quads are soft; the cap's rim stays hard.
    assert_eq!(g.edges.len(), n);
    assert!(g.edges.iter().all(|e| e.soft));
    for e in &g.edges {
        let (a, b) = (g.points[e.a], g.points[e.b]);
        assert!((a.z - b.z).abs() > 0.5, "a seam runs up the band");
    }
    let mut s = Scene::new();
    let report = import(
        &mut s,
        &ImportTree {
            definitions: vec![],
            root: g,
        },
    )
    .unwrap();
    assert_eq!(report.hard, 0);
    s.validate().unwrap();
    let m = mesh(&s, "Tree");
    assert_eq!(m.edges().filter(|(_, e)| e.soft).count(), n);

    // Without normals the mesh is flat shaded: nothing is soft.
    let (g, _) = polygon_geometry(&points, &polygons, &[], &[], true).unwrap();
    assert!(g.edges.is_empty());
    // Without merging, the quads' diagonals are soft too (their triangles share normals).
    let (g, _) = polygon_geometry(&points, &polygons, &[], &normals, false).unwrap();
    assert_eq!(g.faces.len(), 3 * n);
    assert_eq!(
        g.edges.len(),
        n,
        "diagonals of flat quads are coplanar, so not soft"
    );
}

#[test]
fn polygon_geometry_with_normals_stays_quick_on_long_polygons_sharing_their_sides() {
    // A long ring that is not flat (its corners rise and fall), and a second polygon running
    // back along the same corners to an apex: thousands of sides each shared by two faces
    // from the two long polygons.
    let n = 50_000;
    let mut points: Vec<DVec3> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            DVec3::new(a.cos(), a.sin(), if k % 2 == 0 { 0.0 } else { 0.01 })
        })
        .collect();
    points.push(DVec3::new(0.0, 0.0, 1.0));
    let normals = vec![DVec3::Z; n + 1];
    let back: Vec<usize> = std::iter::once(n).chain((0..n).rev()).collect();
    let polygons = [polygon(&(0..n).collect::<Vec<_>>()), polygon(&back)];
    let started = std::time::Instant::now();
    let (g, _) = polygon_geometry(&points, &polygons, &[], &normals, false).unwrap();
    assert!(g.faces.len() > 1000);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}
