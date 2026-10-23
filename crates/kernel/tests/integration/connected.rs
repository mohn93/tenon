use glam::DVec3;
use kernel::{
    DefinitionKind, EdgeId, EntityId, FaceId, Guide, GuideId, InstanceId, MeshId, Model, Scene,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// Adds an axis-aligned square of side `size` at height 0 and returns its face.
fn square(m: &mut Model, x0: f64, y0: f64, size: f64) -> FaceId {
    let c = m
        .add_polygon(&[
            p(x0, y0, 0.0),
            p(x0 + size, y0, 0.0),
            p(x0 + size, y0 + size, 0.0),
            p(x0, y0 + size, 0.0),
        ])
        .unwrap();
    c.summary()
        .created
        .iter()
        .rev()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .expect("square made a face")
}

/// A 1 m box with its corner at the origin.
fn unit_box(m: &mut Model) {
    let base = square(m, 0.0, 0.0, 1.0);
    m.push_pull(base, 1.0).unwrap();
    m.validate().unwrap();
}

fn edge_at(m: &Model, a: DVec3, b: DVec3) -> EdgeId {
    let v = |q: DVec3| {
        m.vertices()
            .find(|(_, v)| v.pos.distance(q) < 1e-9)
            .map(|(id, _)| id)
            .expect("vertex")
    };
    m.edge_between(v(a), v(b)).expect("edge")
}

fn kinds(ids: &[EntityId]) -> (usize, usize) {
    let edges = ids
        .iter()
        .filter(|id| matches!(id, EntityId::Edge(_)))
        .count();
    let faces = ids
        .iter()
        .filter(|id| matches!(id, EntityId::Face(_)))
        .count();
    (edges, faces)
}

fn assert_sorted(ids: &[EntityId]) {
    let edges_then_faces = ids.windows(2).all(|w| match (w[0], w[1]) {
        (EntityId::Edge(a), EntityId::Edge(b)) => a < b,
        (EntityId::Face(a), EntityId::Face(b)) => a < b,
        (EntityId::Edge(_), EntityId::Face(_)) => true,
        _ => false,
    });
    assert!(
        edges_then_faces,
        "not edges then faces in id order: {ids:?}"
    );
}

#[test]
fn an_edge_knows_the_faces_along_it() {
    let mut m = Model::new();
    unit_box(&mut m);
    let side = edge_at(&m, p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let faces = m.faces_of_edge(side);
    assert_eq!(faces.len(), 2);
    for f in &faces {
        assert!(m.face_edges(*f).contains(&side));
    }

    m.add_edge(p(5.0, 5.0, 0.0), p(6.0, 5.0, 0.0)).unwrap();
    let lone = edge_at(&m, p(5.0, 5.0, 0.0), p(6.0, 5.0, 0.0));
    assert!(m.faces_of_edge(lone).is_empty());

    assert!(m.faces_of_edge(EdgeId(999_999)).is_empty());
}

#[test]
fn connected_spreads_through_shared_vertices_only() {
    let mut m = Model::new();
    unit_box(&mut m);
    let far = square(&mut m, 5.0, 0.0, 1.0);
    m.add_edge(p(1.0, 1.0, 1.0), p(2.0, 2.0, 2.0)).unwrap();
    m.add_guide(Guide::Point {
        position: p(3.0, 3.0, 3.0),
        tag: None,
        hidden: false,
    })
    .unwrap();
    m.validate().unwrap();

    let top = m
        .faces()
        .find(|(_, f)| f.normal.z > 0.5 && m.position(f.outer[0]).unwrap().z > 0.5)
        .map(|(id, _)| id)
        .expect("top face");
    let from_box = m.connected(EntityId::Face(top));
    assert_eq!(kinds(&from_box), (13, 6));
    assert!(from_box.contains(&EntityId::Face(top)));
    assert!(!from_box.contains(&EntityId::Face(far)));
    for e in m.face_edges(far) {
        assert!(!from_box.contains(&EntityId::Edge(e)));
    }
    assert_sorted(&from_box);

    let from_square = m.connected(EntityId::Face(far));
    let mut expected: Vec<EntityId> = m.face_edges(far).into_iter().map(EntityId::Edge).collect();
    expected.push(EntityId::Face(far));
    assert_eq!(from_square, expected);
    assert_sorted(&from_square);

    // An edge of a piece gives the same set as a face of it.
    let lone = edge_at(&m, p(1.0, 1.0, 1.0), p(2.0, 2.0, 2.0));
    assert_eq!(m.connected(EntityId::Edge(lone)), from_box);

    let guide = m.guides().next().map(|(id, _)| id).expect("guide");
    assert!(m.connected(EntityId::Guide(guide)).is_empty());
    assert!(m.connected(EntityId::Guide(GuideId(999_999))).is_empty());
    assert!(m.connected(EntityId::Face(FaceId(999_999))).is_empty());
    assert!(m.connected(EntityId::Edge(EdgeId(999_999))).is_empty());
    let vertex = m.vertices().next().map(|(id, _)| id).unwrap();
    assert!(m.connected(EntityId::Vertex(vertex)).is_empty());

    // An instance gives nothing.
    let mut m = Model::new();
    let face = square(&mut m, 0.0, 0.0, 1.0);
    let edges = m.face_edges(face);
    let mut scene = Scene::from_root(m);
    let (instance, _) = scene
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "Group",
        )
        .unwrap();
    let root = scene.root();
    assert!(root.instance(instance).is_some());
    assert!(root.connected(EntityId::Instance(instance)).is_empty());
    assert!(
        root.connected(EntityId::Instance(InstanceId(999_999)))
            .is_empty()
    );

    // A face with a hole: its hole is joined to its outer loop through the face.
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    m.validate().unwrap();
    let (outer, _) = m
        .faces()
        .find(|(_, f)| !f.holes.is_empty())
        .expect("face with a hole");
    let inner = m
        .faces()
        .find(|(id, _)| *id != outer)
        .map(|(id, _)| id)
        .expect("inner face");

    let from_outer = m.connected(EntityId::Face(outer));
    assert_eq!(kinds(&from_outer), (8, 2));
    assert_sorted(&from_outer);

    let outer_edge = edge_at(&m, p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0));
    let hole_edge = edge_at(&m, p(1.0, 1.0, 0.0), p(2.0, 1.0, 0.0));
    assert_eq!(m.connected(EntityId::Edge(outer_edge)), from_outer);
    assert_eq!(m.connected(EntityId::Edge(hole_edge)), from_outer);
    assert_eq!(m.connected(EntityId::Face(inner)), from_outer);
}
