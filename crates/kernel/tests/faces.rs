use glam::DVec3;
use kernel::{ChangeSet, EntityId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// Adds a closed polyline and checks invariants after every edge.
fn polygon(m: &mut Model, pts: &[DVec3]) -> Vec<ChangeSet> {
    (0..pts.len())
        .map(|i| {
            let c = m.add_edge(pts[i], pts[(i + 1) % pts.len()]).unwrap();
            m.validate().unwrap();
            c
        })
        .collect()
}

fn square(m: &mut Model, x0: f64, y0: f64, size: f64) -> Vec<ChangeSet> {
    polygon(
        m,
        &[
            p(x0, y0, 0.0),
            p(x0 + size, y0, 0.0),
            p(x0 + size, y0 + size, 0.0),
            p(x0, y0 + size, 0.0),
        ],
    )
}

fn face_areas(m: &Model) -> Vec<f64> {
    let mut a: Vec<f64> = m.faces().map(|(id, _)| m.face_area(id)).collect();
    a.sort_by(f64::total_cmp);
    a
}

fn count_faces(ids: &[EntityId]) -> usize {
    ids.iter()
        .filter(|id| matches!(id, EntityId::Face(_)))
        .count()
}

#[test]
fn closing_a_square_creates_one_face() {
    let mut m = Model::new();
    let changes = square(&mut m, 0.0, 0.0, 2.0);
    assert!(
        changes[..3]
            .iter()
            .all(|c| count_faces(&c.summary().created) == 0)
    );
    assert_eq!(count_faces(&changes[3].summary().created), 1);
    let (_, f) = m.faces().next().unwrap();
    assert_eq!(f.normal, DVec3::Z);
    assert_eq!(face_areas(&m), [4.0]);
}

#[test]
fn non_coplanar_loop_creates_no_face() {
    let mut m = Model::new();
    polygon(
        &mut m,
        &[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 0.0),
        ],
    );
    assert_eq!(m.faces().count(), 0);
}

#[test]
fn tilted_triangle_gets_a_face_in_its_own_plane() {
    let mut m = Model::new();
    polygon(
        &mut m,
        &[p(0.0, 0.0, 0.0), p(2.0, 0.0, 1.0), p(0.0, 2.0, 1.0)],
    );
    assert_eq!(m.faces().count(), 1);
    let (id, f) = m.faces().next().unwrap();
    assert!((f.normal.length() - 1.0).abs() < 1e-12);
    assert!((m.face_area(id) - 6f64.sqrt()).abs() < 1e-9);
}

#[test]
fn diagonal_splits_a_face_in_two() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let c = m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(face_areas(&m), [2.0, 2.0]);
    let s = c.summary();
    assert_eq!(count_faces(&s.deleted), 1);
    assert_eq!(count_faces(&s.created), 2);
}

#[test]
fn edge_between_side_midpoints_splits_the_sides_and_the_face() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 7);
    assert_eq!(face_areas(&m), [2.0, 2.0]);
}

#[test]
fn dangling_edge_inside_a_face_keeps_the_face() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(face_areas(&m), [4.0]);
}

#[test]
fn loop_drawn_inside_a_face_becomes_a_face_and_a_hole() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    assert_eq!(face_areas(&m), [1.0, 15.0]);
    let outer = m.faces().find(|(_, f)| !f.holes.is_empty()).unwrap().1;
    assert_eq!(outer.holes.len(), 1);
}

#[test]
fn loop_drawn_around_a_face_encloses_it_as_a_hole() {
    let mut m = Model::new();
    square(&mut m, 1.0, 1.0, 1.0);
    square(&mut m, 0.0, 0.0, 4.0);
    assert_eq!(face_areas(&m), [1.0, 15.0]);
}

#[test]
fn twelve_cube_edges_make_six_faces() {
    let mut m = Model::new();
    let c = |i: u8| p((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64);
    let pairs = [
        (0, 1),
        (1, 3),
        (3, 2),
        (2, 0),
        (4, 5),
        (5, 7),
        (7, 6),
        (6, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (a, b) in pairs {
        m.add_edge(c(a), c(b)).unwrap();
        m.validate().unwrap();
    }
    assert_eq!(face_areas(&m), [1.0; 6]);
}

#[test]
fn undoing_face_edits_restores_each_state() {
    let mut m = Model::new();
    let mut states = vec![m.clone()];
    let mut history = Vec::new();
    for c in square(&mut m, 0.0, 0.0, 2.0) {
        history.push(c);
    }
    history.push(m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0)).unwrap());
    let mut replay = Model::new();
    for c in &history {
        replay.apply(c);
        states.push(replay.clone());
    }
    assert_eq!(replay, m);
    for (i, c) in history.iter().enumerate().rev() {
        m.apply(&c.inverse());
        m.validate().unwrap();
        assert_eq!(m, states[i]);
    }
}

/// Deterministic pseudo-random edits on a coarse grid in a few planes; invariants must always hold
/// and undoing everything must return to an empty model.
#[test]
fn random_edits_keep_invariants_and_fully_undo() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut faces_seen = 0;
    for _round in 0..20 {
        let mut m = Model::new();
        let mut history = Vec::new();
        for _ in 0..25 {
            let r: Vec<f64> = (0..6).map(|_| (next() % 4) as f64).collect();
            let z = if r[4] < 1.0 { r[5] } else { 0.0 };
            let a = p(r[0], r[1], z);
            let b = p(r[2], r[3], if r[5] < 2.0 { z } else { 0.0 });
            if let Ok(c) = m.add_edge(a, b) {
                if let Err(e) = m.validate() {
                    panic!("after adding {a} -> {b}: {e}");
                }
                history.push(c);
            }
        }
        faces_seen += m.faces().count();
        let built = m.clone();
        for c in history.iter().rev() {
            m.apply(&c.inverse());
        }
        assert_eq!(m, Model::new());
        for c in &history {
            m.apply(c);
        }
        assert_eq!(m, built);
    }
    assert!(faces_seen > 0, "random edits never formed a face");
}
