use glam::DVec3;
use kernel::{EdgeId, EntityId, FaceId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn rect(m: &mut Model, x0: f64, y0: f64, x1: f64, y1: f64) -> FaceId {
    let c = m
        .add_polygon(&[
            p(x0, y0, 0.0),
            p(x1, y0, 0.0),
            p(x1, y1, 0.0),
            p(x0, y1, 0.0),
        ])
        .unwrap();
    c.summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap()
}

fn edge_at(m: &Model, a: DVec3, b: DVec3) -> EdgeId {
    m.edges()
        .find(|(_, e)| {
            let (pa, pb) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
            (pa.distance(a) < 1e-9 && pb.distance(b) < 1e-9)
                || (pa.distance(b) < 1e-9 && pb.distance(a) < 1e-9)
        })
        .map(|(id, _)| id)
        .expect("edge")
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

#[test]
fn erasing_a_face_keeps_its_edges() {
    let mut m = Model::new();
    let f = rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    let c = m.erase(&[], &[f]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (4, 4, 0));
    assert_eq!(c.summary().deleted, vec![EntityId::Face(f)]);
}

#[test]
fn erasing_a_box_edge_removes_the_two_faces_it_bounds() {
    let mut m = Model::new();
    let f = rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    m.push_pull(f, 1.0).unwrap();
    let e = edge_at(&m, p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    m.erase(&[e], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 11, 4));
}

#[test]
fn erasing_the_seam_between_coplanar_faces_merges_and_heals() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 4.0, 2.0);
    m.add_edge(p(2.0, 0.0, 0.0), p(2.0, 2.0, 0.0)).unwrap();
    assert_eq!(counts(&m), (6, 7, 2));
    let seam = edge_at(&m, p(2.0, 0.0, 0.0), p(2.0, 2.0, 0.0));
    m.erase(&[seam], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (4, 4, 1));
    let (id, f) = m.faces().next().unwrap();
    assert_eq!(f.outer.len(), 4);
    assert_eq!(m.area(id), Some(8.0));
}

#[test]
fn a_merged_face_keeps_the_orientation_of_the_faces_it_joins() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    m.push_pull(base, 1.0).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0)).unwrap();
    let seam = edge_at(&m, p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0));
    m.erase(&[seam], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    let bottom: Vec<_> = m
        .faces()
        .filter(|(_, f)| f.outer.iter().all(|v| m.position(*v).unwrap().z == 0.0))
        .collect();
    assert_eq!(bottom.len(), 1);
    assert_eq!(bottom[0].1.normal, -DVec3::Z);
}

#[test]
fn erasing_a_side_of_an_inner_square_opens_it_into_the_outer_face() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 4.0, 4.0);
    rect(&mut m, 1.0, 1.0, 3.0, 3.0);
    assert_eq!(m.faces().count(), 2);
    let side = edge_at(&m, p(1.0, 1.0, 0.0), p(3.0, 1.0, 0.0));
    m.erase(&[side], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 7, 1));
    let (id, f) = m.faces().next().unwrap();
    assert!(f.holes.is_empty());
    assert_eq!(m.area(id), Some(16.0));
}

#[test]
fn erasing_a_loose_edge_removes_its_free_ends() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    let e = m.edges().next().unwrap().0;
    m.erase(&[e], &[]);
    assert_eq!(m, Model::new());
}

#[test]
fn erasing_a_spur_heals_the_side_it_split() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(counts(&m), (6, 6, 1));
    let spur = edge_at(&m, p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0));
    m.erase(&[spur], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (4, 4, 1));
    assert_eq!(m.faces().next().unwrap().1.outer.len(), 4);
}

#[test]
fn erase_undoes_and_redoes_exactly() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 4.0, 2.0);
    m.push_pull(base, 1.0).unwrap();
    m.add_edge(p(2.0, 0.0, 1.0), p(2.0, 2.0, 1.0)).unwrap();
    let before = m.clone();
    let seam = edge_at(&m, p(2.0, 0.0, 1.0), p(2.0, 2.0, 1.0));
    let corner = edge_at(&m, p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let c = m.erase(&[seam, corner], &[]);
    m.validate().unwrap();
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn missing_ids_are_ignored() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    let before = m.clone();
    assert!(m.erase(&[EdgeId(999)], &[FaceId(998)]).is_empty());
    assert_eq!(m, before);
}
