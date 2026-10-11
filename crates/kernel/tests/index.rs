use glam::DVec3;
use kernel::{EntityId, FaceId, Model, Shape};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 5 x 5 grid of boxes of different heights, with a loose edge above each.
fn town() -> Model {
    let mut m = Model::new();
    for i in 0..5 {
        for j in 0..5 {
            let (x, y) = (i as f64 * 3.0, j as f64 * 3.0);
            let c = m
                .add_polygon(&[
                    p(x, y, 0.0),
                    p(x + 2.0, y, 0.0),
                    p(x + 2.0, y + 2.0, 0.0),
                    p(x, y + 2.0, 0.0),
                ])
                .unwrap();
            let base = c
                .summary()
                .created
                .into_iter()
                .find_map(|id| match id {
                    EntityId::Face(f) => Some(f),
                    _ => None,
                })
                .unwrap();
            m.push_pull(base, 1.0 + ((i * 7 + j * 3) % 5) as f64 * 0.5)
                .unwrap();
            m.add_edge(p(x, y, 6.0), p(x + 1.0, y + 1.5, 6.5)).unwrap();
        }
    }
    m.take_changes();
    m
}

/// Rays from an eye above one corner of the town towards points spread over it.
fn rays() -> Vec<(DVec3, DVec3)> {
    let eye = p(-6.1, -7.3, 9.7);
    let mut out = Vec::new();
    for i in 0..23 {
        for j in 0..19 {
            let target = p(
                i as f64 * 0.731 - 1.0,
                j as f64 * 0.853 - 1.0,
                (i + j) as f64 * 0.05,
            );
            out.push((eye, (target - eye).normalize()));
        }
    }
    out
}

fn hit_triangle(from: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let n = (b - a).cross(c - a);
    let denom = n.dot(dir);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = n.dot(a - from) / denom;
    let q = from + dir * t;
    let inside = |u: DVec3, v: DVec3| (v - u).cross(q - u).dot(n) >= 0.0;
    (inside(a, b) && inside(b, c) && inside(c, a)).then_some(t)
}

/// The nearest face hit with t > 0, by testing every triangle.
fn brute_hit(m: &Model, from: DVec3, dir: DVec3) -> Option<(FaceId, f64)> {
    let mut best: Option<(FaceId, f64)> = None;
    for (f, _) in m.faces() {
        for tri in m.triangulate(f).unwrap() {
            if let Some(t) = hit_triangle(from, dir, tri)
                && t > 0.0
                && best.is_none_or(|(_, bt)| t < bt)
            {
                best = Some((f, t));
            }
        }
    }
    best
}

#[test]
fn ray_hits_match_testing_every_face() {
    let m = town();
    let mut hits = 0;
    for (from, dir) in rays() {
        let want = brute_hit(&m, from, dir);
        let got = m.index().ray_hit(from, dir, 0.0, f64::INFINITY);
        assert_eq!(got.map(|(f, _)| f), want.map(|(f, _)| f), "ray {dir:?}");
        if let (Some((_, a)), Some((_, b))) = (got, want) {
            assert!((a - b).abs() < 1e-9);
            hits += 1;
        }
    }
    assert!(hits > 100, "most rays hit something ({hits})");
}

#[test]
fn ray_hits_list_every_face_within_reach() {
    let m = town();
    let (from, dir) = (p(1.0, 1.0, 10.0), DVec3::NEG_Z);
    let hits = m.index().ray_hits(from, dir, 0.0, 20.0);
    // The box top and its base below.
    assert_eq!(hits.len(), 2);
    let short = m.index().ray_hits(from, dir, 0.0, 9.5);
    assert_eq!(short.len(), 1, "the base is out of reach");
}

#[test]
fn near_ray_visits_everything_inside_the_cone() {
    let m = town();
    let (r0, slope) = (0.05, 0.02);
    for (from, dir) in rays() {
        let mut seen = Vec::new();
        m.index().near_ray(from, dir, r0, slope, |s| seen.push(*s));
        let within = |q: DVec3| {
            let t = (q - from).dot(dir);
            (q - from - dir * t).length() <= r0 + slope * t.max(0.0)
        };
        for (v, vert) in m.vertices() {
            if within(vert.pos) {
                assert!(
                    seen.contains(&Shape::Point(v, vert.pos)),
                    "vertex {v:?} missed"
                );
            }
        }
        for (e, edge) in m.edges() {
            let (a, b) = (
                m.position(edge.v[0]).unwrap(),
                m.position(edge.v[1]).unwrap(),
            );
            let near = (0..=20).any(|k| within(a + (b - a) * (k as f64 / 20.0)));
            if near {
                assert!(
                    seen.iter()
                        .any(|s| matches!(s, Shape::Segment(id, ..) if *id == e)),
                    "edge {e:?} missed"
                );
            }
        }
        assert!(
            seen.len() < m.vertices().count() + m.edges().count(),
            "the cone prunes"
        );
    }
}

#[test]
fn the_index_follows_edits_undo_and_clones() {
    let mut m = town();
    let (from, dir) = (p(20.0, 20.0, 10.0), DVec3::NEG_Z);
    assert_eq!(m.index().ray_hit(from, dir, 0.0, f64::INFINITY), None);
    let c = m
        .add_polygon(&[
            p(19.0, 19.0, 0.0),
            p(21.0, 19.0, 0.0),
            p(21.0, 21.0, 0.0),
            p(19.0, 21.0, 0.0),
        ])
        .unwrap();
    let hit = m.index().ray_hit(from, dir, 0.0, f64::INFINITY);
    assert_eq!(hit.map(|(_, t)| t), Some(10.0));
    let copy = m.clone();
    assert_eq!(copy, m);
    assert_eq!(copy.index().ray_hit(from, dir, 0.0, f64::INFINITY), hit);
    m.apply(&c.inverse());
    assert_eq!(m.index().ray_hit(from, dir, 0.0, f64::INFINITY), None);
}
