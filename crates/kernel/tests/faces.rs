use glam::{DVec2, DVec3};
use kernel::geom::point_in_polygon;
use kernel::{ChangeSet, EntityId, Model, VertexId};

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

// ---- regression tests for the planar-region rebuild ----

fn assert_areas(m: &Model, expected: &[f64]) {
    let got = face_areas(m);
    assert_eq!(
        got.len(),
        expected.len(),
        "face areas {got:?}, expected {expected:?}"
    );
    for (g, e) in got.iter().zip(expected) {
        assert!(
            (g - e).abs() < 1e-9,
            "face areas {got:?}, expected {expected:?}"
        );
    }
}

/// Asserts that no point of the z = 0 plane lies inside more than one face.
fn assert_no_overlap(m: &Model) {
    let xy = |v: VertexId| {
        let q = m.vertex(v).unwrap().pos;
        DVec2::new(q.x, q.y)
    };
    let polys: Vec<(Vec<DVec2>, Vec<Vec<DVec2>>)> = m
        .faces()
        .filter(|(_, f)| {
            f.normal.z.abs() > 1.0 - 1e-9
                && f.outer
                    .iter()
                    .all(|v| m.vertex(*v).unwrap().pos.z.abs() < 1e-9)
        })
        .map(|(_, f)| {
            (
                f.outer.iter().map(|v| xy(*v)).collect(),
                f.holes
                    .iter()
                    .map(|h| h.iter().map(|v| xy(*v)).collect())
                    .collect(),
            )
        })
        .collect();
    if polys.len() < 2 {
        return;
    }
    let pts = polys.iter().flat_map(|(o, _)| o.iter().copied());
    let lo = pts.clone().fold(DVec2::INFINITY, DVec2::min);
    let hi = pts.fold(DVec2::NEG_INFINITY, DVec2::max);
    // Irrational offsets keep samples off edges drawn on simple coordinates.
    let off = DVec2::new(2f64.sqrt() * 1e-3, std::f64::consts::PI * 1e-3);
    let step = 0.05;
    let (nx, ny) = (
        ((hi.x - lo.x) / step).ceil() as usize,
        ((hi.y - lo.y) / step).ceil() as usize,
    );
    for i in 0..=nx {
        for j in 0..=ny {
            let s = lo + off + DVec2::new(i as f64, j as f64) * step;
            let n = polys
                .iter()
                .filter(|(o, hs)| {
                    point_in_polygon(s, o) && !hs.iter().any(|h| point_in_polygon(s, h))
                })
                .count();
            assert!(n <= 1, "point {s} lies inside {n} faces");
        }
    }
}

#[test]
fn cut_through_a_hole_splits_the_hole_with_its_pieces() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    polygon(
        &mut m,
        &[
            p(1.0, 2.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(2.0, 1.0, 0.0),
            p(1.0, 1.0, 0.0),
        ],
    );
    m.add_edge(p(0.0, 1.7, 0.0), p(4.0, 1.7, 0.0)).unwrap();
    m.validate().unwrap();
    assert_areas(&m, &[0.3, 0.7, 6.1, 8.9]);
    assert_no_overlap(&m);
}

#[test]
fn split_beside_a_dangling_edge_keeps_both_halves() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    m.add_edge(p(0.0, 0.0, 0.0), p(0.5, 0.2, 0.0)).unwrap();
    m.add_edge(p(2.0, 0.0, 0.0), p(0.0, 2.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_areas(&m, &[2.0, 2.0]);
    assert_no_overlap(&m);
}

#[test]
fn split_through_a_hole_corner_keeps_both_halves() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    m.add_edge(p(4.0, 0.0, 0.0), p(0.0, 4.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_areas(&m, &[1.0, 7.0, 8.0]);
    assert_no_overlap(&m);
}

#[test]
fn loop_closed_around_a_dangling_edge_becomes_a_face() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    square(&mut m, 0.0, 0.0, 2.0);
    assert_areas(&m, &[4.0]);
}

#[test]
fn loop_touching_an_inner_face_at_a_vertex_surrounds_it() {
    let mut m = Model::new();
    polygon(
        &mut m,
        &[
            p(2.0, 0.0, 0.0),
            p(3.0, 1.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(1.0, 1.0, 0.0),
        ],
    );
    square(&mut m, 0.0, 0.0, 4.0);
    assert_areas(&m, &[2.0, 14.0]);
    assert_no_overlap(&m);
}

#[test]
fn window_in_a_45_degree_wall_keeps_one_orientation() {
    let mut m = Model::new();
    let q = |s: f64, z: f64| p(s, s + 0.3, z);
    polygon(
        &mut m,
        &[q(0.0, 0.0), q(3.0, 0.0), q(3.0, 3.0), q(0.0, 3.0)],
    );
    polygon(
        &mut m,
        &[q(1.0, 1.0), q(2.0, 1.0), q(2.0, 2.0), q(1.0, 2.0)],
    );
    let r = 2f64.sqrt();
    assert_areas(&m, &[r, 8.0 * r]);
    let normals: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    assert!(normals[0].abs_diff_eq(normals[1], 1e-12));
}

#[test]
fn loop_drawn_around_an_inner_face_takes_it_as_a_hole() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 10.0);
    square(&mut m, 4.0, 4.0, 1.0);
    square(&mut m, 2.0, 2.0, 5.0);
    assert_areas(&m, &[1.0, 24.0, 75.0]);
    assert_no_overlap(&m);
}

#[test]
fn loop_around_a_face_with_a_hole_takes_only_its_outer() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 2.0);
    square(&mut m, -1.0, -1.0, 6.0);
    assert_areas(&m, &[4.0, 12.0, 20.0]);
    assert_no_overlap(&m);
}

#[test]
fn thin_sliver_beside_a_face_keeps_the_face() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 10.0);
    m.add_edge(p(0.0, 0.0, 0.0), p(5.0, -0.0005, 0.0)).unwrap();
    m.add_edge(p(5.0, -0.0005, 0.0), p(10.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_areas(&m, &[0.0025, 100.0]);
    assert_no_overlap(&m);
}

/// Same edit generator as `random_edits_keep_invariants_and_fully_undo` with another seed,
/// checking after every edit that faces on the ground never overlap.
#[test]
fn random_edits_never_overlap_faces() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut faces_seen = 0;
    for _round in 0..20 {
        let mut m = Model::new();
        for _ in 0..25 {
            let r: Vec<f64> = (0..6).map(|_| (next() % 4) as f64).collect();
            let z = if r[4] < 1.0 { r[5] } else { 0.0 };
            let a = p(r[0], r[1], z);
            let b = p(r[2], r[3], if r[5] < 2.0 { z } else { 0.0 });
            if m.add_edge(a, b).is_ok() {
                if let Err(e) = m.validate() {
                    panic!("after adding {a} -> {b}: {e}");
                }
                assert_no_overlap(&m);
            }
        }
        faces_seen += m.faces().count();
    }
    assert!(faces_seen > 0, "random edits never formed a face");
}
