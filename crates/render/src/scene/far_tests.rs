//! Scenes far from the world origin: positions are made relative to `SceneBase::origin` in f64
//! before they are rounded to f32.

use kernel::{Guide, Model};

use super::*;

/// A 10 m wall panel, a guide point and an active section plane, `shift` away from where they
/// were drawn.
fn model_at(shift: DVec3) -> Model {
    let mut m = Model::new();
    let p = |x: f64, y: f64, z: f64| shift + DVec3::new(x + 0.123_456_7, y + 0.765_432_1, z);
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(10.0, 0.0, 0.0),
        p(10.0, 0.0, 3.0),
        p(0.0, 0.0, 3.0),
    ])
    .unwrap();
    m.add_guide(Guide::Point {
        position: p(5.0, 5.0, 0.0),
        tag: None,
        hidden: false,
    })
    .unwrap();
    m.add_section(SectionPlane::new(p(5.0, 0.0, 0.0), DVec3::X), true)
        .unwrap();
    m
}

fn base_at(shift: DVec3) -> SceneBase {
    let s = Scene::from_root(model_at(shift));
    let display = SectionDisplay {
        cuts: true,
        ..SectionDisplay::default()
    };
    scene_base_shown(&s, &[], false, GlassView::default(), display)
}

fn world(b: &SceneBase, pos: [f32; 3]) -> DVec3 {
    b.origin + DVec3::from(pos.map(f64::from))
}

#[test]
fn a_model_near_the_origin_is_not_moved() {
    for shift in [DVec3::ZERO, DVec3::new(300.0, -2000.0, 0.0)] {
        let b = base_at(shift);
        assert_eq!(b.origin, DVec3::ZERO);
        // The cut keeps one end of the panel; its end is where the world has it.
        let ends = [0.123_456_7, 10.123_456_7].map(|x| (shift.x + x) as f32);
        assert!(b.faces.iter().any(|v| ends.contains(&v.pos[0])));
    }
}

#[test]
fn the_origin_is_a_whole_number_of_blocks_and_never_in_height() {
    let o = origin_for(DVec3::new(5.0e6 + 100.0, -4.9e6, 8000.0));
    assert_eq!(o.z, 0.0);
    assert_eq!(o.x % ORIGIN_SNAP, 0.0);
    assert!((o.x - 5.0e6 - 100.0).abs() <= ORIGIN_SNAP);
    assert_eq!(origin_for(DVec3::new(4000.0, -4000.0, 0.0)), DVec3::ZERO);
    // A small edit does not move it.
    assert_eq!(
        origin_for(DVec3::new(5.0e6 + 100.0, 0.0, 0.0)),
        origin_for(DVec3::new(5.0e6 + 103.0, 0.0, 0.0))
    );
}

#[test]
fn positions_5e6_m_out_are_kept_to_a_micrometer_relative_to_the_origin() {
    let shift = DVec3::new(5.0e6, 4.9e6, 12.0);
    let b = base_at(shift);
    assert_ne!(b.origin, DVec3::ZERO);
    let near = base_at(DVec3::new(0.0, 0.0, 12.0));
    assert_eq!(b.faces.len(), near.faces.len());
    let moved = DVec3::new(shift.x, shift.y, 0.0);
    // The panel is 10 m across and the origin within 8 m of its center: the steps of its f32
    // positions are about 2e-6 m.
    for (v, n) in b.faces.iter().zip(&near.faces) {
        let expected = DVec3::from(n.pos.map(f64::from)) + moved;
        let off = (world(&b, v.pos) - expected).abs().max_element();
        assert!(off < 2.0e-6, "{off}");
        assert!(v.pos.iter().all(|c| c.abs() < 40.0), "{:?}", v.pos);
    }
    // The edges are in the same frame.
    assert_eq!(b.edges.len(), near.edges.len());
    for (v, n) in b.edges.iter().zip(&near.edges) {
        let expected = DVec3::from(n.pos.map(f64::from)) + moved;
        assert!((world(&b, v.pos) - expected).abs().max_element() < 1.0e-5);
    }
    // So are the f64 shapes and the planes.
    assert!(!b.cut_planes.is_empty());
    assert!(b.cut_planes[0].point.abs().max_element() < 40.0);
    let guide = b.guides[0].point;
    assert!(guide.abs().max_element() < 40.0);
    let at = shift + DVec3::new(5.123_456_7, 5.765_432_1, 0.0);
    assert!((guide + b.origin - at).abs().max_element() < 1.0e-9);
    // The casters' bounds are in the frame of their corners.
    assert!(!b.caster_chunks.is_empty());
    for c in &b.caster_chunks {
        assert!(c.lo.iter().chain(&c.hi).all(|v| v.abs() < 40.0));
    }
}

#[test]
fn a_point_shared_by_two_triangles_gets_the_same_f32_each_time() {
    // The corners of different chunks are rounded from the same f64 with the same origin.
    let b = base_at(DVec3::new(5.0e6, 4.9e6, 0.0));
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let first = key(b.faces[0].pos);
    assert!(b.faces.iter().filter(|v| key(v.pos) == first).count() >= 2);
}
