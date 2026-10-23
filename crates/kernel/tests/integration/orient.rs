use std::collections::BTreeSet;

use glam::DVec3;
use kernel::{FaceId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 1 m cube: a ground square pulled up, every face pointing out.
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

fn face_ids(m: &Model) -> Vec<FaceId> {
    m.faces().map(|(id, _)| id).collect()
}

/// True if every pair of faces sharing a side runs it in opposite directions.
fn consistent(m: &Model) -> bool {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (_, f) in m.faces() {
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                if !seen.insert((format!("{a:?}"), format!("{b:?}"))) {
                    return false;
                }
            }
        }
    }
    true
}

#[test]
fn reverse_faces_turns_each_over_and_keeps_ids_and_paint() {
    let mut m = cube();
    let ids = face_ids(&m);
    let before: Vec<DVec3> = ids.iter().map(|id| m.face(*id).unwrap().normal).collect();
    let (changes, changed) = m.reverse_faces(&ids[..2]);
    assert_eq!(changed, ids[..2].to_vec());
    assert!(!changes.is_empty());
    for (i, id) in ids.iter().enumerate() {
        let n = m.face(*id).unwrap().normal;
        let want = if i < 2 { -before[i] } else { before[i] };
        assert!((n - want).length() < 1e-12);
    }
    m.validate().unwrap();
    m.apply(&changes.inverse());
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(m.face(*id).unwrap().normal, before[i]);
    }
    // Unknown ids and no faces change nothing.
    let (none, changed) = m.reverse_faces(&[]);
    assert!(none.is_empty() && changed.is_empty());
}

#[test]
fn orient_fixes_a_box_with_one_flipped_face() {
    let mut m = cube();
    let ids = face_ids(&m);
    assert!(consistent(&m));
    let start = ids[0];
    m.reverse_faces(&[ids[3]]);
    assert!(!consistent(&m));
    let before = m.face(start).unwrap().normal;
    let (changes, report) = m.orient_faces(start, &[]);
    assert_eq!(report.flipped, vec![ids[3]]);
    assert_eq!(report.conflicts, 0);
    assert_eq!(m.face(start).unwrap().normal, before);
    assert!(consistent(&m));
    m.validate().unwrap();
    m.apply(&changes.inverse());
    assert!(!consistent(&m));
}

#[test]
fn orient_follows_the_start_face_when_it_is_the_odd_one() {
    let mut m = cube();
    let ids = face_ids(&m);
    m.reverse_faces(&[ids[1]]);
    let (_, report) = m.orient_faces(ids[1], &[]);
    // Everything else turns to match the start face.
    assert_eq!(report.flipped.len(), 5);
    assert!(!report.flipped.contains(&ids[1]));
    assert!(consistent(&m));
}

#[test]
fn orient_fixes_a_box_with_several_flipped_faces() {
    let mut m = cube();
    let ids = face_ids(&m);
    m.reverse_faces(&[ids[1], ids[2], ids[5]]);
    let (_, report) = m.orient_faces(ids[0], &[]);
    assert_eq!(report.flipped, vec![ids[1], ids[2], ids[5]]);
    assert_eq!(report.conflicts, 0);
    assert!(consistent(&m));
}

#[test]
fn consistent_surface_has_nothing_to_flip() {
    let mut m = cube();
    let start = face_ids(&m)[0];
    let (changes, report) = m.orient_faces(start, &[]);
    assert!(changes.is_empty());
    assert!(report.flipped.is_empty());
    assert_eq!(report.conflicts, 0);
}

#[test]
fn orient_stays_within_the_given_faces() {
    let mut m = cube();
    let ids = face_ids(&m);
    let shares_side = |a: FaceId, b: FaceId| {
        let (fa, fb) = (m.face(a).unwrap(), m.face(b).unwrap());
        fa.outer.iter().filter(|v| fb.outer.contains(v)).count() == 2
    };
    let near = ids[1..]
        .iter()
        .copied()
        .find(|&f| shares_side(ids[0], f))
        .unwrap();
    let far: Vec<FaceId> = ids[1..]
        .iter()
        .copied()
        .filter(|&f| f != near && shares_side(ids[0], f))
        .collect();
    // A neighbour inside the scope and another outside it are both turned over.
    m.reverse_faces(&[near, far[0]]);
    let (_, report) = m.orient_faces(ids[0], &[ids[0], near]);
    assert_eq!(report.flipped, vec![near]);
}

#[test]
fn orient_works_on_an_open_surface() {
    let mut m = Model::new();
    // A row of three squares in the ground plane sharing edges.
    for i in 0..3 {
        let x = i as f64;
        m.add_polygon(&[
            p(x, 0.0, 0.0),
            p(x + 1.0, 0.0, 0.0),
            p(x + 1.0, 1.0, 0.0),
            p(x, 1.0, 0.0),
        ])
        .unwrap();
    }
    let ids = face_ids(&m);
    assert_eq!(ids.len(), 3);
    // Squares drawn next to each other already agree; turn the middle and last over.
    let ordered: Vec<FaceId> = {
        let mut v = ids.clone();
        v.sort_by(|a, b| {
            let cx = |f: &FaceId| {
                m.face(*f)
                    .unwrap()
                    .outer
                    .iter()
                    .map(|v| m.position(*v).unwrap().x)
                    .sum::<f64>()
            };
            cx(a).partial_cmp(&cx(b)).unwrap()
        });
        v
    };
    m.reverse_faces(&[ordered[1], ordered[2]]);
    let (_, report) = m.orient_faces(ordered[0], &[]);
    assert_eq!(report.flipped.len(), 2);
    assert_eq!(report.conflicts, 0);
    assert!(consistent(&m));
}

#[test]
fn orient_stops_at_a_non_manifold_edge() {
    let mut m = Model::new();
    // Three squares meeting along the edge (0,0,0)-(0,0,1): a T junction (three faces on one edge).
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 0.0, 1.0),
        p(0.0, 0.0, 1.0),
    ])
    .unwrap();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(-1.0, 0.0, 0.0),
        p(-1.0, 0.0, 1.0),
        p(0.0, 0.0, 1.0),
    ])
    .unwrap();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 1.0, 1.0),
        p(0.0, 0.0, 1.0),
    ])
    .unwrap();
    let ids = face_ids(&m);
    assert_eq!(ids.len(), 3);
    for &start in &ids {
        let (changes, report) = m.orient_faces(start, &[]);
        assert!(changes.is_empty(), "no edge has exactly two faces");
        assert!(report.flipped.is_empty());
    }
}

/// A triangulated Moebius strip of `n` segments.
fn moebius(n: usize) -> Model {
    let mut m = Model::new();
    let at = |i: usize, w: f64| {
        let t = std::f64::consts::TAU * i as f64 / n as f64;
        let r = 3.0 + w * (t / 2.0).cos();
        p(r * t.cos(), r * t.sin(), w * (t / 2.0).sin())
    };
    for i in 0..n {
        let (a0, a1) = (at(i, -0.5), at(i + 1, -0.5));
        let (b0, b1) = (at(i, 0.5), at(i + 1, 0.5));
        m.add_polygon(&[a0, a1, b1]).unwrap();
        m.add_polygon(&[a0, b1, b0]).unwrap();
    }
    m
}

#[test]
fn orient_terminates_on_a_moebius_strip_and_reports_the_conflict() {
    let mut m = moebius(12);
    let ids = face_ids(&m);
    assert_eq!(ids.len(), 24);
    let (_, report) = m.orient_faces(ids[0], &[]);
    assert!(report.conflicts >= 1, "{report:?}");
    m.validate().unwrap();
}
