use glam::DVec3;
use kernel::{FaceId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn polygon(m: &mut Model, pts: &[DVec3]) {
    for i in 0..pts.len() {
        m.add_edge(pts[i], pts[(i + 1) % pts.len()]).unwrap();
    }
}

fn square(m: &mut Model, x0: f64, y0: f64, s: f64) {
    polygon(
        m,
        &[
            p(x0, y0, 0.0),
            p(x0 + s, y0, 0.0),
            p(x0 + s, y0 + s, 0.0),
            p(x0, y0 + s, 0.0),
        ],
    );
}

/// Sum of triangle areas, asserting each triangle winds counter-clockwise around `normal`.
fn covered_area(tris: &[[DVec3; 3]], normal: DVec3) -> f64 {
    tris.iter()
        .map(|t| {
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            assert!(n.dot(normal) > 0.0, "triangle winds the wrong way");
            n.length() * 0.5
        })
        .sum()
}

#[test]
fn square_face_triangulates_to_its_area() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let (id, f) = m.faces().next().unwrap();
    let tris = m.triangulate(id).unwrap();
    assert_eq!(tris.len(), 2);
    assert!((covered_area(&tris, f.normal) - 4.0).abs() < 1e-9);
}

#[test]
fn face_with_hole_triangulates_around_the_hole() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    let (id, f) = m.faces().find(|(_, f)| !f.holes.is_empty()).unwrap();
    let tris = m.triangulate(id).unwrap();
    assert!((covered_area(&tris, f.normal) - 15.0).abs() < 1e-9);
    // No triangle covers the hole's center.
    let c = p(1.5, 1.5, 0.0);
    for t in &tris {
        let inside = (0..3).all(|i| (t[(i + 1) % 3] - t[i]).cross(c - t[i]).dot(f.normal) > 0.0);
        assert!(!inside, "a triangle covers the hole");
    }
}

#[test]
fn tilted_and_concave_faces_triangulate() {
    let mut m = Model::new();
    // L-shaped (concave) face on a tilted plane z = x.
    polygon(
        &mut m,
        &[
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 2.0),
            p(2.0, 1.0, 2.0),
            p(1.0, 1.0, 1.0),
            p(1.0, 2.0, 1.0),
            p(0.0, 2.0, 0.0),
        ],
    );
    let (id, f) = m.faces().next().unwrap();
    let tris = m.triangulate(id).unwrap();
    assert!((covered_area(&tris, f.normal) - m.area(id).unwrap()).abs() < 1e-9);
}

#[test]
fn missing_face_triangulates_to_none() {
    assert!(Model::new().triangulate(FaceId(5)).is_none());
}
