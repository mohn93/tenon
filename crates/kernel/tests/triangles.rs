//! A scene's faces as triangles in root coordinates.

use glam::DVec3;
use kernel::{DefinitionKind, MeshId, Model, Scene};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn box_model() -> Model {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let base = m.faces().next().unwrap().0;
    m.push_pull(base, 1.0).unwrap();
    m
}

#[test]
fn a_box_gives_twelve_triangles_facing_out() {
    let s = Scene::from_root(box_model());
    let tris = s.triangles();
    assert_eq!(tris.len(), 12);
    let centre = p(0.5, 0.5, 0.5);
    for t in &tris {
        let [a, b, c] = t.corners;
        let winding = (b - a).cross(c - a).normalize();
        assert!(
            winding.abs_diff_eq(t.normal, 1e-9),
            "wound around its normal"
        );
        let mid = (a + b + c) / 3.0;
        assert!((mid - centre).dot(t.normal) > 0.0, "faces out of the box");
    }
}

#[test]
fn instances_are_placed_and_components_repeat() {
    let mut s = Scene::from_root(box_model());
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (crate_, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Crate",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[crate_], p(5.0, 0.0, 0.0), 1)
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_edge(p(0.0, -3.0, 0.0), p(1.0, -3.0, 0.0))
    })
    .unwrap();
    let tris = s.triangles();
    assert_eq!(tris.len(), 24, "two boxes; a loose edge adds nothing");
    let max_x = tris
        .iter()
        .flat_map(|t| t.corners)
        .map(|c| c.x)
        .fold(f64::MIN, f64::max);
    assert_eq!(max_x, 6.0);
}
