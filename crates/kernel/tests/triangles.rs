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

/// A mesh holding a 1 m triangle and a square face whose hole is its own outer loop (it
/// passes validation but covers nothing, so it gives no triangles), ids from `first`.
fn mesh_with_an_empty_face(first: u64, x: f64) -> kernel::MeshData {
    use kernel::{EdgeData, FaceData, MeshData, VertexData};
    let id = |k: u64| first + k;
    let v = |k, position| VertexData {
        id: id(k),
        position,
    };
    let e = |k, a, b| EdgeData {
        id: id(k),
        vertices: [id(a), id(b)],
        curve: None,
        soft: false,
    };
    let face = |k, outer: &[u64], holes: Vec<Vec<u64>>| FaceData {
        id: id(k),
        outer: outer.iter().map(|&a| id(a)).collect(),
        holes: holes
            .into_iter()
            .map(|h| h.into_iter().map(id).collect())
            .collect(),
        normal: [0.0, 0.0, 1.0],
        front: None,
        back: None,
        texture_position: None,
    };
    MeshData {
        vertices: vec![
            v(0, [x, 0.0, 0.0]),
            v(1, [x + 1.0, 0.0, 0.0]),
            v(2, [x + 1.0, 1.0, 0.0]),
            v(3, [x, 1.0, 0.0]),
            v(4, [x, 2.0, 0.0]),
            v(5, [x + 1.0, 2.0, 0.0]),
        ],
        edges: vec![
            e(6, 0, 1),
            e(7, 1, 2),
            e(8, 2, 3),
            e(9, 3, 0),
            e(10, 3, 5),
            e(11, 5, 4),
            e(12, 4, 3),
        ],
        faces: vec![
            face(13, &[0, 1, 2, 3], vec![vec![0, 3, 2, 1]]),
            face(14, &[3, 5, 4], vec![]),
        ],
        instances: vec![],
        guides: vec![],
    }
}

#[test]
fn faces_that_give_no_triangles_are_counted_once_per_placement() {
    let mut root = mesh_with_an_empty_face(100, 0.0);
    let place = |k: u64, x: f64| kernel::InstanceData {
        id: 200 + k,
        definition: 1,
        transform: glam::DAffine3::from_translation(p(x, 5.0, 0.0)).to_cols_array(),
        material: None,
    };
    root.instances = vec![place(0, 0.0), place(1, 3.0)];
    let data = kernel::SceneData {
        next_id: 300,
        root,
        definitions: vec![kernel::DefinitionData {
            id: 1,
            name: "Part".into(),
            kind: DefinitionKind::Component,
            mesh: mesh_with_an_empty_face(2, 0.0),
        }],
        materials: vec![],
    };
    let s = Scene::from_data(&data).unwrap();
    let (tris, skipped) = s.triangles_and_skipped();
    assert_eq!(tris.len(), 3, "one triangle per mesh placement");
    assert_eq!(
        skipped, 3,
        "the empty face in the root and in each placement"
    );
    assert_eq!(s.triangles(), tris);
    let (_, none) = Scene::from_root(box_model()).triangles_and_skipped();
    assert_eq!(none, 0);
}
