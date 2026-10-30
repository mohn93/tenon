use glam::{DAffine3, DMat3, DVec3};
use kernel::geom::{LENGTH_TOL, Plane};
use kernel::intersect::{FaceShape, face_segments, face_segments_cost, face_shape};
use kernel::{FaceId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn shape(outer: &[DVec3], holes: &[&[DVec3]]) -> FaceShape {
    let plane = Plane::from_points(outer[0], outer[1], outer[2]).unwrap();
    FaceShape {
        outer: outer.to_vec(),
        holes: holes.iter().map(|h| h.to_vec()).collect(),
        plane,
    }
}

/// A rectangle in the plane z = `z`.
fn flat(x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> FaceShape {
    shape(
        &[p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)],
        &[],
    )
}

/// A rectangle in the plane y = `y`.
fn wall(x0: f64, z0: f64, x1: f64, z1: f64, y: f64) -> FaceShape {
    shape(
        &[p(x0, y, z0), p(x1, y, z0), p(x1, y, z1), p(x0, y, z1)],
        &[],
    )
}

fn quad(side: f64) -> [DVec3; 4] {
    [
        p(0.0, 0.0, 0.0),
        p(side, 0.0, 0.0),
        p(side, side, 0.0),
        p(0.0, side, 0.0),
    ]
}

fn length(segs: &[(DVec3, DVec3)]) -> f64 {
    segs.iter().map(|(a, b)| a.distance(*b)).sum()
}

fn only(segs: &[(DVec3, DVec3)]) -> (DVec3, DVec3) {
    assert_eq!(segs.len(), 1, "{segs:?}");
    let (a, b) = segs[0];
    if (a.x, a.y, a.z) <= (b.x, b.y, b.z) {
        (a, b)
    } else {
        (b, a)
    }
}

fn near(a: DVec3, b: DVec3) {
    assert!(a.distance(b) < 1e-9, "{a} vs {b}");
}

#[test]
fn crossing_quads_meet_along_a_chord() {
    let floor = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let w = wall(1.0, -1.0, 3.0, 1.0, 2.0);
    for (a, b) in [(&floor, &w), (&w, &floor)] {
        let (s, e) = only(&face_segments(a, b));
        near(s, p(1.0, 2.0, 0.0));
        near(e, p(3.0, 2.0, 0.0));
    }
}

#[test]
fn a_wall_wider_than_the_floor_is_clipped_by_it() {
    let floor = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let w = wall(-5.0, -1.0, 2.0, 1.0, 1.0);
    let (s, e) = only(&face_segments(&floor, &w));
    near(s, p(0.0, 1.0, 0.0));
    near(e, p(2.0, 1.0, 0.0));
}

#[test]
fn t_shaped_faces_meet_where_the_wall_stands_on_the_floor() {
    let floor = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let w = wall(1.0, 0.0, 3.0, 2.0, 2.0);
    let (s, e) = only(&face_segments(&floor, &w));
    near(s, p(1.0, 2.0, 0.0));
    near(e, p(3.0, 2.0, 0.0));
}

#[test]
fn faces_that_do_not_reach_each_other_give_nothing() {
    let floor = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    assert!(face_segments(&floor, &wall(1.0, 1.0, 3.0, 2.0, 2.0)).is_empty());
    assert!(face_segments(&floor, &wall(1.0, -1.0, 3.0, 1.0, 9.0)).is_empty());
}

#[test]
fn parallel_distinct_planes_give_nothing() {
    let a = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let b = flat(1.0, 1.0, 3.0, 3.0, 1.0);
    assert!(face_segments(&a, &b).is_empty());
}

#[test]
fn coplanar_overlap_gives_the_boundary_of_the_shared_region() {
    let a = flat(0.0, 0.0, 2.0, 2.0, 0.0);
    let b = flat(1.0, 1.0, 3.0, 3.0, 0.0);
    for segs in [face_segments(&a, &b), face_segments(&b, &a)] {
        assert_eq!(segs.len(), 4, "{segs:?}");
        assert!((length(&segs) - 4.0).abs() < 1e-9);
        for q in segs.iter().flat_map(|(s, e)| [s, e]) {
            assert!((1.0 - 1e-9..=2.0 + 1e-9).contains(&q.x), "{q}");
            assert!((1.0 - 1e-9..=2.0 + 1e-9).contains(&q.y), "{q}");
        }
    }
}

#[test]
fn coplanar_containment_gives_the_inner_outline() {
    let outer = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let inner = flat(1.0, 1.0, 2.0, 2.0, 0.0);
    let segs = face_segments(&outer, &inner);
    assert!((length(&segs) - 4.0).abs() < 1e-9, "{segs:?}");
}

#[test]
fn coplanar_faces_that_only_touch_give_nothing() {
    let a = flat(0.0, 0.0, 2.0, 2.0, 0.0);
    assert!(face_segments(&a, &flat(2.0, 0.0, 4.0, 2.0, 0.0)).is_empty());
    assert!(face_segments(&a, &flat(2.0, 2.0, 4.0, 4.0, 0.0)).is_empty());
}

#[test]
fn coplanar_faces_overlapping_along_a_band_give_its_outline() {
    let a = flat(0.0, 0.0, 2.0, 2.0, 0.0);
    let b = flat(0.0, 1.0, 2.0, 3.0, 0.0);
    // The shared region is 2 x 1 (y from 1 to 2): outline 6.
    let segs = face_segments(&a, &b);
    assert!((length(&segs) - 6.0).abs() < 1e-9, "{segs:?}");
}

fn holed_floor() -> FaceShape {
    let hole = [
        p(1.0, 1.0, 0.0),
        p(1.0, 3.0, 0.0),
        p(3.0, 3.0, 0.0),
        p(3.0, 1.0, 0.0),
    ];
    shape(
        &[
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 4.0, 0.0),
            p(0.0, 4.0, 0.0),
        ],
        &[&hole],
    )
}

#[test]
fn a_chord_through_a_hole_is_split_around_it() {
    let mut segs = face_segments(&holed_floor(), &wall(-1.0, -1.0, 5.0, 1.0, 2.0));
    assert_eq!(segs.len(), 2, "{segs:?}");
    let span = |s: &(DVec3, DVec3)| (s.0.x.min(s.1.x), s.0.x.max(s.1.x));
    segs.sort_by(|a, b| span(a).0.total_cmp(&span(b).0));
    assert_eq!(
        [span(&segs[0]), span(&segs[1])].map(|(a, b)| (a.round(), b.round())),
        [(0.0, 1.0), (3.0, 4.0)]
    );
}

#[test]
fn a_line_along_a_hole_edge_stays_inside() {
    let segs = face_segments(&holed_floor(), &wall(-1.0, -1.0, 5.0, 1.0, 1.0));
    assert!((length(&segs) - 4.0).abs() < 1e-9, "{segs:?}");
}

#[test]
fn faces_of_a_box_meet_along_their_shared_edge() {
    let a = flat(0.0, 0.0, 2.0, 2.0, 0.0);
    let b = wall(0.0, 0.0, 2.0, 2.0, 0.0);
    let (s, e) = only(&face_segments(&a, &b));
    near(s, p(0.0, 0.0, 0.0));
    near(e, p(2.0, 0.0, 0.0));
}

#[test]
fn slivers_are_dropped() {
    let floor = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let w = wall(2.0, -1.0, 2.0 + LENGTH_TOL * 0.5, 1.0, 2.0);
    assert!(face_segments(&floor, &w).is_empty());
    let w = wall(2.0, 0.0, 2.0 + LENGTH_TOL * 0.5, 1.0, 2.0);
    assert!(face_segments(&floor, &w).is_empty());
}

fn all_finite(segs: &[(DVec3, DVec3)]) -> bool {
    segs.iter().all(|(a, b)| a.is_finite() && b.is_finite())
}

#[test]
fn degenerate_faces_do_not_panic() {
    let line = FaceShape {
        outer: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)],
        holes: vec![],
        plane: Plane::through(DVec3::ZERO, DVec3::Z),
    };
    let f = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let w = wall(1.0, -1.0, 3.0, 1.0, 2.0);
    for s in [
        face_segments(&line, &f),
        face_segments(&line, &w),
        face_segments(&w, &line),
    ] {
        assert!(all_finite(&s));
    }
    let nan = FaceShape {
        outer: vec![p(f64::NAN, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)],
        holes: vec![],
        plane: Plane::through(DVec3::ZERO, DVec3::Z),
    };
    assert!(all_finite(&face_segments(&nan, &w)));
}

#[test]
fn near_parallel_planes_stay_finite() {
    let a = flat(0.0, 0.0, 4.0, 4.0, 0.0);
    let t = 2e-6;
    let b = shape(
        &[
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, t),
            p(4.0, 4.0, t),
            p(0.0, 4.0, 0.0),
        ],
        &[],
    );
    assert!(all_finite(&face_segments(&a, &b)));
}

fn model_with_quad(pts: [DVec3; 4]) -> (Model, FaceId) {
    let mut m = Model::new();
    m.add_polygon(&pts).unwrap();
    let id = m.faces().next().unwrap().0;
    (m, id)
}

#[test]
fn face_shape_applies_scaled_mirrored_transforms() {
    let (m, id) = model_with_quad(quad(2.0));
    // Mirror in x, stretch z, then lift: the face stays in z = 1 and spans x in -2..0.
    let t = DAffine3 {
        matrix3: DMat3::from_diagonal(p(-1.0, 1.0, 3.0)),
        translation: p(0.0, 0.0, 1.0),
    };
    let s = face_shape(&m, id, t).unwrap();
    assert!(s.plane.same_as(&Plane::through(p(0.0, 0.0, 1.0), DVec3::Z)));
    assert!(
        s.outer
            .iter()
            .all(|q| (q.z - 1.0).abs() < 1e-12 && q.x <= 0.0)
    );

    // A non-uniform scale tilts a slanted face; the plane follows the moved points.
    let (m, id) = model_with_quad([
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 2.0),
        p(0.0, 2.0, 2.0),
    ]);
    let s = face_shape(&m, id, DAffine3::from_scale(p(1.0, 1.0, 5.0))).unwrap();
    assert!(s.outer.iter().all(|q| s.plane.contains(*q)));
    let want = p(0.0, 5.0, -1.0).normalize();
    assert!(s.plane.normal.cross(want).length() < 1e-9, "{:?}", s.plane);

    // Collapsing the face to a line gives no shape; neither does a missing face.
    assert!(face_shape(&m, id, DAffine3::from_scale(p(1.0, 0.0, 0.0))).is_none());
    assert!(face_shape(&m, FaceId(9999), DAffine3::IDENTITY).is_none());
}

#[test]
fn a_transformed_face_crosses_like_the_world_one() {
    let (m, id) = model_with_quad(quad(2.0));
    let s = face_shape(&m, id, DAffine3::from_scale(p(2.0, 2.0, 1.0))).unwrap();
    let (a, b) = only(&face_segments(&s, &wall(1.0, -1.0, 3.0, 1.0, 2.0)));
    near(a, p(1.0, 2.0, 0.0));
    near(b, p(3.0, 2.0, 0.0));
}

#[test]
fn add_segments_splits_a_face_with_a_chord() {
    let (mut m, _) = model_with_quad(quad(4.0));
    let c = m
        .add_segments(&[(p(0.0, 2.0, 0.0), p(4.0, 2.0, 0.0))])
        .unwrap();
    assert!(!c.is_empty());
    m.validate().unwrap();
    let mut areas: Vec<f64> = m.faces().map(|(id, _)| m.area(id).unwrap()).collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 2);
    assert!((areas[0] - 8.0).abs() < 1e-9 && (areas[1] - 8.0).abs() < 1e-9);
}

#[test]
fn add_segments_of_existing_edges_changes_nothing() {
    let (mut m, _) = model_with_quad(quad(4.0));
    let before = m.clone();
    let c = m
        .add_segments(&[
            (p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0)),
            (p(4.0, 4.0, 0.0), p(4.0, 0.0, 0.0)),
        ])
        .unwrap();
    assert!(c.is_empty(), "{c:?}");
    assert_eq!(m, before);
    assert!(m.add_segments(&[]).unwrap().is_empty());
}

#[test]
fn add_segments_skips_short_ones_and_rejects_non_finite() {
    let (mut m, _) = model_with_quad(quad(4.0));
    let before = m.clone();
    let tiny = (p(1.0, 1.0, 0.0), p(1.0 + LENGTH_TOL * 0.1, 1.0, 0.0));
    assert!(m.add_segments(&[tiny]).unwrap().is_empty());
    assert_eq!(m, before);
    let bad = (p(1.0, f64::NAN, 0.0), p(2.0, 1.0, 0.0));
    assert!(m.add_segments(&[tiny, bad]).is_err());
    assert_eq!(m, before);
}

#[test]
fn a_face_shape_segment_from_a_neighbour_splits_the_face() {
    // A wall crossing the whole floor gives a chord edge to edge, which splits it.
    let (mut m, floor_id) = model_with_quad(quad(4.0));
    let a = face_shape(&m, floor_id, DAffine3::IDENTITY).unwrap();
    let segs = face_segments(&a, &wall(-1.0, -1.0, 5.0, 1.0, 2.0));
    m.add_segments(&segs).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
}

#[test]
fn faces_near_box_finds_candidates() {
    let (mut m, _) = model_with_quad(quad(4.0));
    m.add_polygon(&[
        p(10.0, 0.0, 0.0),
        p(12.0, 0.0, 0.0),
        p(12.0, 2.0, 0.0),
        p(10.0, 2.0, 0.0),
    ])
    .unwrap();
    assert_eq!(
        m.faces_near_box(p(1.0, 1.0, -1.0), p(2.0, 2.0, 1.0)).len(),
        1
    );
    assert_eq!(
        m.faces_near_box(p(-1.0, -1.0, -1.0), p(20.0, 20.0, 1.0))
            .len(),
        2
    );
    assert!(
        m.faces_near_box(p(5.0, 5.0, 5.0), p(6.0, 6.0, 6.0))
            .is_empty()
    );
}

#[test]
fn crossing_triangles_meet_along_a_segment() {
    let a = shape(&[p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0), p(0.0, 4.0, 0.0)], &[]);
    let b = shape(
        &[p(1.0, 1.0, -1.0), p(2.0, 1.0, 1.0), p(1.0, 2.0, 1.0)],
        &[],
    );
    let segs = face_segments(&a, &b);
    assert_eq!(segs.len(), 1, "{segs:?}");
    assert!(segs[0].0.z.abs() < 1e-9 && segs[0].1.z.abs() < 1e-9);
    assert!(segs[0].0.distance(segs[0].1) > 0.1);
}

fn square_at(lo: f64, hi: f64) -> [DVec3; 4] {
    [
        p(lo, lo, 0.0),
        p(hi, lo, 0.0),
        p(hi, hi, 0.0),
        p(lo, hi, 0.0),
    ]
}

#[test]
fn coplanar_faces_with_holes_on_both_give_the_outlines_inside_the_other() {
    // b's outline (16) lies in a's area; a's hole outline (8) lies in b's area; b's hole
    // lies inside a's hole, so its outline (4) is in no area.
    let (ah, bh) = (square_at(2.0, 4.0), square_at(2.5, 3.5));
    let a = shape(&square_at(0.0, 6.0), &[&ah]);
    let b = shape(&square_at(1.0, 5.0), &[&bh]);
    for segs in [face_segments(&a, &b), face_segments(&b, &a)] {
        assert!((length(&segs) - 24.0).abs() < 1e-9, "{segs:?}");
    }
}

#[test]
fn narrow_coplanar_faces_touching_give_nothing() {
    let w = 2.0 * LENGTH_TOL;
    let a = flat(0.0, 0.0, 2.0, w, 0.0);
    assert!(face_segments(&a, &flat(0.0, w, 2.0, 2.0 * w, 0.0)).is_empty());
    // Overlapping instead of touching: the shared strip's outline is found.
    let segs = face_segments(&a, &flat(0.0, 0.5 * w, 2.0, 1.5 * w, 0.0));
    assert!(length(&segs) >= 4.0 - 1e-9, "{segs:?}");
}

#[test]
fn a_mirrored_face_crosses_a_skewed_one() {
    let (m, id) = model_with_quad(quad(2.0));
    // x -> 1 - x: the face spans x in -1..1.
    let t = DAffine3 {
        matrix3: DMat3::from_diagonal(p(-1.0, 1.0, 1.0)),
        translation: p(1.0, 0.0, 0.0),
    };
    let s = face_shape(&m, id, t).unwrap();
    let skew = shape(
        &[
            p(-2.0, 0.5, -1.0),
            p(2.0, 0.5, -1.0),
            p(2.0, 1.5, 1.0),
            p(-2.0, 1.5, 1.0),
        ],
        &[],
    );
    for (a, b) in [(&s, &skew), (&skew, &s)] {
        let (u, v) = only(&face_segments(a, b));
        near(u, p(-1.0, 1.0, 0.0));
        near(v, p(1.0, 1.0, 0.0));
    }
}

#[test]
fn add_segments_merges_overlaps_and_splits_at_crossings() {
    let (mut m, _) = model_with_quad(quad(4.0));
    m.add_segments(&[
        (p(0.0, 2.0, 0.0), p(3.0, 2.0, 0.0)),
        (p(1.0, 2.0, 0.0), p(4.0, 2.0, 0.0)),
        (p(2.0, 0.0, 0.0), p(2.0, 4.0, 0.0)),
    ])
    .unwrap();
    m.validate().unwrap();
    // The corners, the four edge midpoints, the centre, and the ends (1,2) and (3,2) of the
    // overlapping segments.
    assert_eq!(m.vertices().count(), 11);
    assert_eq!(m.edges().count(), 14);
    assert_eq!(m.faces().count(), 4);
    assert!(
        m.faces()
            .all(|(id, _)| (m.area(id).unwrap() - 4.0).abs() < 1e-9)
    );
}

#[test]
fn the_cost_estimate_grows_with_loop_sizes() {
    // Coplanar: product of the edge counts.
    let a = flat(0.0, 0.0, 2.0, 2.0, 0.0);
    let b = flat(1.0, 1.0, 3.0, 3.0, 0.0);
    assert_eq!(face_segments_cost(&a, &b), 16);
    // Crossing planes: bounded by the edges of both times the possible windows.
    let wall = shape(
        &[
            p(1.0, 0.0, -1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 2.0, 1.0),
            p(1.0, 2.0, -1.0),
        ],
        &[],
    );
    assert_eq!(face_segments_cost(&a, &wall), 8 * 5);
    // A face with a hole costs more than the same without.
    let holed = shape(
        &[
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 4.0, 0.0),
            p(0.0, 4.0, 0.0),
        ],
        &[&[
            p(1.0, 1.0, 0.0),
            p(1.0, 3.0, 0.0),
            p(3.0, 3.0, 0.0),
            p(3.0, 1.0, 0.0),
        ]],
    );
    assert_eq!(face_segments_cost(&holed, &b), 8 * 4);
}

#[test]
fn a_line_leaving_a_face_across_a_shallow_edge_is_followed_to_it() {
    // The corner (0, 0.5) is 5e-7 above the line y = 0.5; the edge to (1, 0.5 - 5e-5) crosses
    // the line at x = 0.0099, and the face lies below the edge: the line is in the face from the
    // corner to there.
    let c = 0.5;
    let floor = shape(
        &[
            p(0.0, c + 5e-7, 0.0),
            p(1.0, c - 5e-5, 0.0),
            p(1.0, c - 1.0, 0.0),
            p(0.0, c - 1.0, 0.0),
        ],
        &[],
    );
    let w = wall(-1.0, -1.0, 2.0, 1.0, c);
    for segs in [face_segments(&floor, &w), face_segments(&w, &floor)] {
        let (s, e) = only(&segs);
        assert!(s.x.abs() < 1e-6, "{s}");
        assert!((e.x - 0.0099).abs() < 1e-4, "{e}");
    }
}

#[test]
fn two_faces_sharing_a_crease_cut_a_shallow_face_to_one_point_of_it() {
    // The faces meet along the y axis at (0, y, 0); the first is twisted by 4e-6, so its
    // fitted plane is off the crease by a fraction of a micron. The third face crosses the
    // crease at a shallow angle (0.076 rad), which magnifies that to over a micron along it.
    // Each face's cut ends on the crease, at the same point.
    let fitted = |q: [DVec3; 4]| {
        let n = (q[2] - q[0]).cross(q[3] - q[1]).normalize();
        FaceShape {
            outer: q.to_vec(),
            holes: Vec::new(),
            plane: Plane::through(q.iter().sum::<DVec3>() / 4.0, n),
        }
    };
    let right = fitted([
        p(0.0, -1.0, 0.0),
        p(1.0, -1.0, 0.2),
        p(1.0, 1.0, 0.2 + 4e-6),
        p(0.0, 1.0, 0.0),
    ]);
    let left = fitted([
        p(0.0, -1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(-1.0, 1.0, 0.3),
        p(-1.0, -1.0, 0.3),
    ]);
    let slope = 0.0761;
    let cut = fitted([
        p(-1.0, -0.5, 0.01 + slope * 0.5),
        p(1.0, -0.5, 0.01 + slope * 0.5),
        p(1.0, 0.5, 0.01 - slope * 0.5),
        p(-1.0, 0.5, 0.01 - slope * 0.5),
    ]);
    // The end of each face's cut on the crease (x = 0).
    let on_crease = |segs: Vec<(DVec3, DVec3)>| {
        let (s, e) = only(&segs);
        if s.x.abs() < e.x.abs() { s } else { e }
    };
    let r = on_crease(face_segments(&right, &cut));
    let l = on_crease(face_segments(&left, &cut));
    assert!(r.distance(l) < LENGTH_TOL, "{r} vs {l}");
}

#[test]
fn a_crease_end_does_not_carry_a_cut_out_of_the_other_face() {
    // The edge of a crosses the line y = 0.5 at x = 0.5 at 0.13 rad, so at 0.08 rad between
    // the planes its sine to b's plane is just over MIN_SLOPE; its vertices are 1e-6 off a's
    // plane, which slides the crossing ~1e-4 along the line. b ends at x = 0.50005, within
    // 1e-4 of the crossing: the cut must stay within b, whichever way the slide goes.
    let t = 0.08_f64;
    let v = DVec3::new(0.0, t.cos(), t.sin());
    let b = fitted_shape(&[
        p(0.499, 0.5, 0.0) - v,
        p(0.50005, 0.5, 0.0) - v,
        p(0.50005, 0.5, 0.0) + v,
        p(0.499, 0.5, 0.0) + v,
    ]);
    for dz in [1e-6, -1e-6] {
        let a = FaceShape {
            outer: vec![
                p(0.0, 0.5 + 0.065, dz),
                p(1.0, 0.5 - 0.065, dz),
                p(1.0, -1.0, 0.0),
                p(0.0, -1.0, 0.0),
            ],
            holes: Vec::new(),
            plane: Plane::through(DVec3::ZERO, DVec3::Z),
        };
        for segs in [face_segments(&a, &b), face_segments(&b, &a)] {
            for (s, e) in &segs {
                for q in [s, e] {
                    assert!(
                        (0.499 - LENGTH_TOL..=0.50005 + LENGTH_TOL).contains(&q.x),
                        "dz {dz}: {q}"
                    );
                }
            }
            assert_eq!(segs.len(), 1, "dz {dz}: {segs:?}");
        }
    }
}

#[test]
fn a_line_grazing_a_corner_within_tolerance_leaves_no_sliver_face() {
    // The corner (0, 0.5 + 5e-7) is within LENGTH_TOL of the line y = 0.5, and the edge from
    // it runs shallowly across the line at x = 0.0099: the line is inside the face from the
    // corner to there, but only within the tolerance of its edge.
    let c = 0.5;
    let pts = [
        p(0.0, c + 5e-7, 0.0),
        p(1.0, c - 5e-5, 0.0),
        p(1.0, c - 1.0, 0.0),
        p(0.0, c - 1.0, 0.0),
    ];
    let (mut m, id) = model_with_quad(pts);
    let a = face_shape(&m, id, DAffine3::IDENTITY).unwrap();
    let segs = face_segments(&a, &wall(-1.0, -1.0, 2.0, 1.0, c));
    m.add_segments(&segs).unwrap();
    m.validate().unwrap();
    for (id, _) in m.faces() {
        let s = face_shape(&m, id, DAffine3::IDENTITY).unwrap();
        let n = s.outer.len();
        let sides = || (0..n).map(|i| (s.outer[i], s.outer[(i + 1) % n]));
        let perimeter: f64 = sides().map(|(u, v)| u.distance(v)).sum();
        let area = 0.5 * sides().map(|(u, v)| u.cross(v)).sum::<DVec3>().length();
        let width = 2.0 * area / perimeter;
        assert!(width >= LENGTH_TOL, "sliver face {width:e} wide");
    }
}

/// A face through `pts` with the plane fitted to all of them (Newell), as `face_shape` does.
fn fitted_shape(pts: &[DVec3]) -> FaceShape {
    let mut n = DVec3::ZERO;
    for i in 0..pts.len() {
        n += pts[i].cross(pts[(i + 1) % pts.len()]);
    }
    FaceShape {
        outer: pts.to_vec(),
        holes: Vec::new(),
        plane: Plane::through(pts.iter().sum::<DVec3>() / pts.len() as f64, n.normalize()),
    }
}

#[test]
fn a_cut_along_a_shallow_edge_stays_inside_both_faces_whatever_the_fit_error() {
    // Face a lies in z = 0 below a shallow edge (1e-3 rad to the line y = 0.5) that crosses the
    // line at x = 0.5; its two edge vertices are `dz` off the plane. Face b contains the line
    // at 0.08 rad to a and covers x 0.499 to 2. The overlap is x 0.499 to 0.5, and the ends
    // must stay within a few LENGTH_TOL of it however the fit error slides the crossing.
    let t = 0.08_f64;
    let v = DVec3::new(0.0, t.cos(), t.sin());
    let b = fitted_shape(&[
        p(0.499, 0.5, 0.0) - v,
        p(2.0, 0.5, 0.0) - v,
        p(2.0, 0.5, 0.0) + v,
        p(0.499, 0.5, 0.0) + v,
    ]);
    for dz in [0.0, 5e-7, -5e-7] {
        let a = FaceShape {
            outer: vec![
                p(0.0, 0.5 + 5e-4, dz),
                p(1.0, 0.5 - 5e-4, dz),
                p(1.0, -1.0, 0.0),
                p(0.0, -1.0, 0.0),
            ],
            holes: Vec::new(),
            plane: Plane::through(DVec3::ZERO, DVec3::Z),
        };
        for segs in [face_segments(&a, &b), face_segments(&b, &a)] {
            let (s, e) = only(&segs);
            assert!((s.x - 0.499).abs() < 1e-5, "dz {dz}: {s}");
            assert!((e.x - 0.5).abs() < 1e-5, "dz {dz}: {e}");
        }
    }
}
