use glam::DVec3;
use kernel::{DefinitionKind, EdgeId, MeshId, MeshPolygon, Model, Scene, SoftenReport};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 1 m cube: a ground square pulled up.
fn cube() -> Model {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let (base, _) = m.faces().next().unwrap();
    let up = if m.face(base).unwrap().normal.z > 0.0 {
        1.0
    } else {
        -1.0
    };
    m.push_pull(base, up).unwrap();
    m
}

fn all_edges(m: &Model) -> Vec<EdgeId> {
    m.edges().map(|(id, _)| id).collect()
}

fn soft_count(m: &Model) -> usize {
    m.edges().filter(|(_, e)| e.soft).count()
}

#[test]
fn only_edges_between_two_faces_are_softened() {
    let mut m = cube();
    m.add_edge(p(3.0, 0.0, 0.0), p(4.0, 0.0, 0.0)).unwrap();
    let edges = all_edges(&m);
    assert_eq!(edges.len(), 13);
    let (changes, report) = m.soften_edges(&edges, true);
    assert_eq!(
        report,
        SoftenReport {
            changed: 12,
            skipped: 1
        }
    );
    assert_eq!(soft_count(&m), 12);
    assert_eq!(changes.summary().modified.len(), 12);
    m.validate().unwrap();

    // Softening again changes nothing and skips only the lone edge.
    let (again, report) = m.soften_edges(&edges, true);
    assert!(again.is_empty());
    assert_eq!(
        report,
        SoftenReport {
            changed: 0,
            skipped: 1
        }
    );

    // Undo takes the flags back off; hardening does the same as a new edit.
    m.apply(&changes.inverse());
    assert_eq!(soft_count(&m), 0);
    m.apply(&changes);
    let (_, report) = m.soften_edges(&edges, false);
    assert_eq!(
        report,
        SoftenReport {
            changed: 12,
            skipped: 0
        }
    );
    assert_eq!(soft_count(&m), 0);
}

#[test]
fn soften_by_angle_takes_edges_whose_faces_meet_at_most_at_the_angle() {
    let mut m = cube();
    let edges = all_edges(&m);
    let (changes, report) = m.soften_by_angle(&edges, 89f64.to_radians());
    assert!(changes.is_empty());
    assert_eq!(
        report,
        SoftenReport {
            changed: 0,
            skipped: 12
        }
    );
    let (_, report) = m.soften_by_angle(&edges, 90f64.to_radians());
    assert_eq!(
        report,
        SoftenReport {
            changed: 12,
            skipped: 0
        }
    );
    m.validate().unwrap();

    // A square split by a diagonal: the two halves lie in one plane, the sides bound one face.
    let mut flat = Model::new();
    flat.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    flat.add_edge(p(0.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(flat.faces().count(), 2);
    let edges = all_edges(&flat);
    let (_, report) = flat.soften_by_angle(&edges, 0.0);
    assert_eq!(
        report,
        SoftenReport {
            changed: 1,
            skipped: 4
        }
    );
}

#[test]
fn face_edges_and_the_faces_beside_an_edge() {
    let m = cube();
    for (f, _) in m.faces() {
        let edges = m.face_edges(f);
        assert_eq!(edges.len(), 4);
        for e in edges {
            let beside = m.faces_beside(e).unwrap();
            assert!(beside.contains(&f));
            let angle = m.fold_angle(e).unwrap();
            assert!((angle - 90f64.to_radians()).abs() < 1e-9, "{angle}");
        }
    }
}

#[test]
fn the_fold_angle_ignores_how_the_faces_are_oriented() {
    // Two triangles folded 90 degrees along the x axis, the second wound against the first.
    let points = [
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(0.5, 1.0, 0.0),
        p(0.5, 0.0, 1.0),
    ];
    for second in [vec![0, 1, 3], vec![1, 0, 3]] {
        let polygons = [
            MeshPolygon {
                corners: vec![0, 1, 2],
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
        let m = scene.mesh(MeshId::Definition(def)).unwrap();
        let fold = m
            .edges()
            .map(|(id, _)| id)
            .find(|e| m.faces_beside(*e).is_some())
            .unwrap();
        let angle = m.fold_angle(fold).unwrap();
        assert!((angle - 90f64.to_radians()).abs() < 1e-9, "{angle}");
    }
}
