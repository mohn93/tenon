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

#[test]
fn a_mirrored_instance_keeps_its_triangles_wound_around_outward_normals() {
    let mut s = Scene::from_root(box_model());
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    s.make_group(
        MeshId::Root,
        &edges,
        &faces,
        &[],
        DefinitionKind::Component,
        "Crate",
    )
    .unwrap();
    // Mirror the instance across x and move it by (0, 0, 0): the box fills x in [-1, 0].
    let mut data = s.to_data();
    data.root.instances[0].transform =
        [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let s = Scene::from_data(&data).unwrap();
    let tris = s.triangles();
    assert_eq!(tris.len(), 12);
    let centre = p(-0.5, 0.5, 0.5);
    for t in &tris {
        let [a, b, c] = t.corners;
        assert!(
            a.x <= 1e-9 && b.x <= 1e-9 && c.x <= 1e-9,
            "mirrored into -x"
        );
        let winding = (b - a).cross(c - a).normalize();
        assert!(
            winding.abs_diff_eq(t.normal, 1e-9),
            "wound around its normal: {winding} vs {}",
            t.normal
        );
        let mid = (a + b + c) / 3.0;
        assert!((mid - centre).dot(t.normal) > 0.0, "faces out of the box");
    }
}

#[test]
fn triangles_carry_face_materials_and_inherit_the_nearest_painted_instance() {
    let mut s = Scene::from_root(box_model());
    let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
    let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
    let (green, _) = s.add_material("Green", [0, 255, 0]).unwrap();
    let top = s
        .root()
        .faces()
        .find(|(_, f)| f.normal == DVec3::Z)
        .unwrap()
        .0;
    s.paint(MeshId::Root, &[top], &[], kernel::Side::Front, Some(red))
        .unwrap();
    let tris = s.triangles();
    assert_eq!(
        tris.iter().filter(|t| t.front == Some(red)).count(),
        2,
        "the top's two triangles"
    );
    assert!(tris.iter().all(|t| t.back.is_none()));

    // Group the box, then group the group: paint the inner instance blue and the outer green.
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let faces: Vec<_> = s.root().faces().map(|(f, _)| f).collect();
    let (inner, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    let (outer, _) = s
        .make_group(MeshId::Root, &[], &[], &[inner], DefinitionKind::Group, "")
        .unwrap();
    let d = s.root().instance(outer).unwrap().definition;
    s.paint(
        MeshId::Root,
        &[],
        &[outer],
        kernel::Side::Front,
        Some(green),
    )
    .unwrap();
    let tris = s.triangles();
    assert_eq!(tris.iter().filter(|t| t.front == Some(green)).count(), 10);
    assert!(tris.iter().all(|t| t.back == Some(green)));
    s.paint(
        MeshId::Definition(d),
        &[],
        &[inner],
        kernel::Side::Front,
        Some(blue),
    )
    .unwrap();
    let tris = s.triangles();
    assert_eq!(tris.iter().filter(|t| t.front == Some(red)).count(), 2);
    assert_eq!(
        tris.iter().filter(|t| t.front == Some(blue)).count(),
        10,
        "the nearer instance wins"
    );
    assert!(tris.iter().all(|t| t.back == Some(blue)));
}
