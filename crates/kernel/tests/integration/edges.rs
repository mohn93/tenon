use glam::DVec3;
use kernel::{KernelError, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

#[test]
fn rejects_degenerate_edges() {
    let mut m = Model::new();
    assert_eq!(
        m.add_edge(DVec3::ZERO, p(0.0, 0.0, 1e-9)),
        Err(KernelError::ZeroLength)
    );
    assert_eq!(
        m.add_edge(DVec3::ZERO, p(f64::NAN, 0.0, 0.0)),
        Err(KernelError::NotFinite)
    );
    assert_eq!(m, Model::new());
}

#[test]
fn endpoints_merge_with_nearby_vertices() {
    let mut m = Model::new();
    m.add_edge(DVec3::ZERO, p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0 + 1e-8, 0.0, 0.0), p(1.0, 1.0, 0.0))
        .unwrap();
    assert_eq!(m.vertices().count(), 3);
    assert_eq!(m.edges().count(), 2);
    m.validate().unwrap();
}

#[test]
fn adding_an_existing_edge_changes_nothing() {
    let mut m = Model::new();
    m.add_edge(DVec3::ZERO, p(1.0, 0.0, 0.0)).unwrap();
    let c = m.add_edge(p(1.0, 0.0, 0.0), DVec3::ZERO).unwrap();
    assert!(c.is_empty());
}

#[test]
fn endpoint_on_an_edge_splits_it() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 3);
    assert_eq!(m.vertices().count(), 4);
}

#[test]
fn crossing_edges_split_each_other() {
    let mut m = Model::new();
    m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(0.0, -1.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 4);
    assert_eq!(m.vertices().count(), 5);
}

#[test]
fn skew_edges_do_not_split() {
    let mut m = Model::new();
    m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(0.0, -1.0, 1.0), p(0.0, 1.0, 1.0)).unwrap();
    assert_eq!(m.edges().count(), 2);
}

#[test]
fn collinear_overlap_merges() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(3.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 3);
    assert_eq!(m.vertices().count(), 4);
}

#[test]
fn edge_covering_existing_vertices_is_split_at_them() {
    let mut m = Model::new();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(2.0, 0.0, 0.0), p(2.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(0.0, 0.0, 0.0), p(3.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 5);
}

#[test]
fn open_polyline_creates_no_face() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 1.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    assert_eq!(m.faces().count(), 0);
}

#[test]
fn edits_undo_exactly() {
    let mut m = Model::new();
    m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    let before = m.clone();
    let c = m.add_edge(p(0.0, -1.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 4);
}

#[test]
fn one_segment_crossing_three_parallel_edges_splits_each() {
    let mut m = Model::new();
    for x in [1.0, 2.0, 3.0] {
        m.add_edge(p(x, -1.0, 0.0), p(x, 1.0, 0.0)).unwrap();
    }
    m.add_edge(p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    // Three verticals split in two each, plus the new segment in four pieces.
    assert_eq!(m.edges().count(), 6 + 4);
    assert_eq!(m.vertices().count(), 6 + 2 + 3);
}

#[test]
fn segment_through_an_existing_vertex_splits_there() {
    let mut m = Model::new();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 3);
    assert_eq!(m.vertices().count(), 4);
}

#[test]
fn add_polygon_is_one_change_set_with_a_face() {
    let mut m = Model::new();
    let pts = [
        DVec3::ZERO,
        DVec3::new(2.0, 0.0, 0.0),
        DVec3::new(2.0, 1.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
    ];
    let c = m.add_polygon(&pts).unwrap();
    m.validate().unwrap();
    assert_eq!(c.summary().created.len(), 4 + 4 + 1);
    assert_eq!(m.faces().count(), 1);
    m.apply(&c.inverse());
    assert_eq!(m, Model::new());
}

#[test]
fn add_polygon_rejects_bad_points_without_changes() {
    let mut m = Model::new();
    let (a, b) = (DVec3::ZERO, DVec3::X);
    assert_eq!(
        m.add_polygon(&[a, b, b, DVec3::Y]),
        Err(KernelError::ZeroLength)
    );
    assert_eq!(
        m.add_polygon(&[a, b, DVec3::NAN]),
        Err(KernelError::NotFinite)
    );
    assert_eq!(m, Model::new());
}
