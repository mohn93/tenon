use std::f64::consts::TAU;

use glam::{DAffine3, DVec3};
use kernel::{DefinitionKind, MeshId, MeshPolygon, Model, Scene};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A cylinder of radius 1 and height 2 with `n` sides: a circle pulled up, its sides soft.
fn cylinder(n: usize) -> Model {
    let ring: Vec<DVec3> = (0..n)
        .map(|k| {
            let a = TAU * k as f64 / n as f64;
            p(a.cos(), a.sin(), 0.0)
        })
        .collect();
    let mut m = Model::new();
    m.add_curve(&ring, true).unwrap();
    let (base, _) = m.faces().next().unwrap();
    let up = if m.face(base).unwrap().normal.z > 0.0 {
        2.0
    } else {
        -2.0
    };
    m.push_pull(base, up).unwrap();
    m
}

/// The vertex at `p`.
fn vertex_at(m: &Model, p: DVec3) -> kernel::VertexId {
    m.vertices()
        .find(|(_, v)| v.pos == p)
        .map(|(id, _)| id)
        .unwrap()
}

fn near(a: DVec3, b: DVec3) -> bool {
    a.distance(b) < 1e-9
}

#[test]
fn faces_without_soft_edges_keep_their_own_normal() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let smooth = m.smooth_normals();
    assert!(smooth.is_empty());
    for (f, face) in m.faces() {
        for t in m.shaded_triangles(f, &smooth).unwrap() {
            assert_eq!(t.normals, [face.normal; 3]);
        }
        assert_eq!(
            m.shaded_triangles(f, &smooth)
                .unwrap()
                .iter()
                .map(|t| t.corners)
                .collect::<Vec<_>>(),
            m.triangulate(f).unwrap()
        );
    }
}

#[test]
fn a_cylinders_sides_are_shaded_round_and_its_caps_flat() {
    let m = cylinder(12);
    let smooth = m.smooth_normals();
    let mut sides = 0;
    for (f, face) in m.faces() {
        for t in m.shaded_triangles(f, &smooth).unwrap() {
            for (c, n) in t.corners.into_iter().zip(t.normals) {
                assert_eq!(m.corner_normal(f, vertex_at(&m, c)).unwrap(), n);
                if face.normal.z.abs() > 0.5 {
                    assert_eq!(n, face.normal, "a cap stays flat");
                } else {
                    // Two equal sides meet at each soft edge: the normal there points straight
                    // out from the axis.
                    assert!(near(n, p(c.x, c.y, 0.0).normalize()), "{c} {n}");
                    sides += 1;
                }
            }
        }
    }
    assert_eq!(sides, 12 * 6);
}

#[test]
fn larger_faces_weigh_more() {
    // Two sides of a box bent 90 degrees, one twice as wide, with the fold soft.
    let points = [
        p(0.0, 0.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 1.0, 1.0),
        p(0.0, 0.0, 1.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 0.0, 1.0),
    ];
    let polygons = [
        MeshPolygon {
            corners: vec![0, 1, 2, 3],
            material: None,
        },
        MeshPolygon {
            corners: vec![0, 3, 5, 4],
            material: None,
        },
    ];
    let mut scene = Scene::new();
    let (instance, _, _) = scene
        .import_mesh(
            MeshId::Root,
            &points,
            &polygons,
            false,
            DefinitionKind::Group,
            "fold",
        )
        .unwrap();
    let def = scene.root().instance(instance).unwrap().definition;
    let mesh = MeshId::Definition(def);
    scene
        .edit(mesh, |m| {
            let all: Vec<_> = m.edges().map(|(id, _)| id).collect();
            Ok(m.soften_by_angle(&all, TAU / 4.0).0)
        })
        .unwrap();
    let m = scene.mesh(mesh).unwrap();
    let corner = m
        .vertices()
        .find(|(_, v)| near(v.pos, DVec3::ZERO))
        .map(|(id, _)| id)
        .unwrap();
    // Areas 1 and 2, normals +x and +y: (1, 2, 0) normalized, for both faces.
    let expected = p(1.0, 2.0, 0.0).normalize();
    let smooth = m.smooth_normals();
    for (f, _) in m.faces() {
        assert!(near(m.corner_normal(f, corner).unwrap(), expected));
        assert!(near(smooth.get(f, corner).unwrap(), expected));
    }
}

#[test]
fn a_face_oriented_against_its_neighbour_is_turned_over_first() {
    // The same bent pair, the second side wound the other way.
    let points = [
        p(0.0, 0.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 1.0, 1.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 0.0, 1.0),
    ];
    let mut normals = Vec::new();
    for second in [vec![0, 3, 5, 4], vec![0, 4, 5, 3]] {
        let polygons = [
            MeshPolygon {
                corners: vec![0, 1, 2, 3],
                material: None,
            },
            MeshPolygon {
                corners: second,
                material: None,
            },
        ];
        let mut scene = Scene::new();
        let (instance, _, _) = scene
            .import_mesh(
                MeshId::Root,
                &points,
                &polygons,
                false,
                DefinitionKind::Group,
                "fold",
            )
            .unwrap();
        let def = scene.root().instance(instance).unwrap().definition;
        let mesh = MeshId::Definition(def);
        scene
            .edit(mesh, |m| {
                let all: Vec<_> = m.edges().map(|(id, _)| id).collect();
                Ok(m.soften_by_angle(&all, TAU / 4.0).0)
            })
            .unwrap();
        let m = scene.mesh(mesh).unwrap();
        let corner = m
            .vertices()
            .find(|(_, v)| near(v.pos, DVec3::ZERO))
            .map(|(id, _)| id)
            .unwrap();
        let (first, _) = m.faces().next().unwrap();
        normals.push(m.corner_normal(first, corner).unwrap());
    }
    assert!(
        near(normals[0], p(1.0, 1.0, 0.0).normalize()),
        "{normals:?}"
    );
    assert!(near(normals[0], normals[1]), "{normals:?}");
}

#[test]
fn scene_triangles_carry_corner_normals_turned_with_their_placement() {
    // The cylinder's faces imported as a group (hard edges), softened up to 45 degrees (its
    // eight sides), then the group turned a quarter about x.
    let model = cylinder(8);
    let mut points = Vec::new();
    let mut polygons = Vec::new();
    for (_, face) in model.faces() {
        let start = points.len();
        points.extend(face.outer.iter().map(|v| model.position(*v).unwrap()));
        polygons.push(MeshPolygon {
            corners: (start..points.len()).collect(),
            material: None,
        });
    }
    let mut scene = Scene::new();
    let (instance, _, _) = scene
        .import_mesh(
            MeshId::Root,
            &points,
            &polygons,
            false,
            DefinitionKind::Group,
            "cylinder",
        )
        .unwrap();
    let flat = scene.triangles();
    assert!(flat.iter().all(|t| t.normals == [t.normal; 3]));
    let def = scene.root().instance(instance).unwrap().definition;
    let mut report = None;
    scene
        .edit(MeshId::Definition(def), |m| {
            let all: Vec<_> = m.edges().map(|(id, _)| id).collect();
            let (changes, r) = m.soften_by_angle(&all, TAU / 8.0);
            report = Some(r);
            Ok(changes)
        })
        .unwrap();
    assert_eq!(report.unwrap().changed, 8);
    let turn = DAffine3::from_rotation_x(TAU / 4.0);
    scene
        .edit(MeshId::Root, |m| {
            Ok(m.transform_instances(&[instance], turn))
        })
        .unwrap();
    let mut sides = 0;
    for t in scene.triangles() {
        if t.normal.y.abs() > 0.5 {
            assert_eq!(t.normals, [t.normal; 3], "a cap, now facing along y");
            continue;
        }
        for (c, n) in t.corners.into_iter().zip(t.normals) {
            // Before the turn: straight out from the z axis; after it, from the y axis.
            assert!(near(n, p(c.x, 0.0, c.z).normalize()), "{c} {n}");
            sides += 1;
        }
    }
    assert_eq!(sides, 8 * 6);
}
