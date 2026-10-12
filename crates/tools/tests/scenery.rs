//! Inference and picking across groups: geometry inside instances and outside the open
//! instance is seen and snapped to, in the open context's coordinates.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{DefinitionKind, InstanceId, MeshId, Model, Scene};
use session::Camera;
use tools::{Guides, Owner, Scenery, SnapKind, edge_under_in, face_under_in, infer_in};

const R: f64 = 10.0;

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 1 x 1 x 1 box at the origin, grouped and moved by (3, 0, 0), plus a loose root edge
/// from (0, -2, 0) to (1, -2, 0). Returns the scene and the group's instance.
fn scene() -> (Scene, InstanceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let base = m.faces().next().unwrap().0;
    m.push_pull(base, 1.0).unwrap();
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (i, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    s.edit(MeshId::Root, |m| {
        let mut c = m.add_edge(p(0.0, -2.0, 0.0), p(1.0, -2.0, 0.0))?;
        c.ops.extend(m.move_instances(&[i], p(3.0, 0.0, 0.0)).ops);
        Ok(c)
    })
    .unwrap();
    s.validate().unwrap();
    (s, i)
}

/// The default camera, moved into the coordinates of an instance placed by `shift`.
fn camera(shift: DVec3) -> Camera {
    let mut c = Camera::new(DVec2::new(800.0, 600.0));
    c.eye -= shift;
    c.target -= shift;
    c
}

fn px(c: &Camera, q: DVec3) -> DVec2 {
    c.project(q).unwrap()
}

#[test]
fn from_the_root_a_groups_corner_is_an_endpoint() {
    let (s, _) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    let corner = p(4.0, 0.0, 1.0);
    let hit = infer_in(
        &view,
        &c,
        px(&c, corner) + DVec2::new(3.0, 2.0),
        &Guides::default(),
        R,
    );
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(hit.point.abs_diff_eq(corner, 1e-9), "{:?}", hit.point);
}

#[test]
fn a_groups_face_is_seen_but_not_given_as_the_open_contexts_face() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    let top = p(3.5, 0.5, 1.0);
    let hit = infer_in(&view, &c, px(&c, top), &Guides::default(), R);
    assert_eq!(hit.kind, SnapKind::OnFace);
    assert!(hit.point.abs_diff_eq(top, 1e-9));
    assert_eq!(hit.face, None);
    let (owner, _, at) = face_under_in(&view, &c, px(&c, top)).unwrap();
    assert_eq!(owner, Owner::Instance(i));
    assert!(at.abs_diff_eq(top, 1e-9));
}

#[test]
fn a_groups_faces_hide_what_is_behind_them() {
    let (mut s, _) = scene();
    let c = camera(DVec3::ZERO);
    // A root edge starting right behind the middle of the group's box, as seen from the eye.
    let hidden = c.eye + (p(3.5, 0.5, 0.5) - c.eye) * 1.2;
    s.edit(MeshId::Root, |m| {
        m.add_edge(hidden, hidden + p(0.5, 0.0, 0.0))
    })
    .unwrap();
    let view = Scenery::of(&s, &[]).unwrap();
    let hit = infer_in(&view, &c, px(&c, hidden), &Guides::default(), R);
    assert_ne!(hit.kind, SnapKind::Endpoint, "{hit:?}");
    // From the root alone (no group), the same end is snapped to.
    let open = Scenery::single(s.root());
    let bare = infer_in(&open, &c, px(&c, hidden), &Guides::default(), R);
    assert_eq!(bare.kind, SnapKind::Endpoint);
}

#[test]
fn edges_under_the_cursor_report_whose_they_are() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    let (owner, _, _, _) = edge_under_in(&view, &c, px(&c, p(3.5, 0.0, 0.0)), R).unwrap();
    assert_eq!(owner, Owner::Instance(i));
    let (owner, _, _, _) = edge_under_in(&view, &c, px(&c, p(0.5, -2.0, 0.0)), R).unwrap();
    assert_eq!(owner, Owner::Open);
}

#[test]
fn inside_the_group_the_root_is_snapped_to_in_the_groups_coordinates() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[i]).unwrap();
    let shift = p(3.0, 0.0, 0.0);
    let c = camera(shift);
    // The root edge's end (1, -2, 0) is (-2, -2, 0) inside the group.
    let end = p(-2.0, -2.0, 0.0);
    let hit = infer_in(
        &view,
        &c,
        px(&c, end) + DVec2::new(2.0, 2.0),
        &Guides::default(),
        R,
    );
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(hit.point.abs_diff_eq(end, 1e-9), "{:?}", hit.point);
    let (owner, _, _, _) = edge_under_in(&view, &c, px(&c, p(-2.5, -2.0, 0.0)), R).unwrap();
    assert_eq!(owner, Owner::Outside);
    // The group's own top face is the open context's face.
    let hit = infer_in(&view, &c, px(&c, p(0.5, 0.5, 1.0)), &Guides::default(), R);
    assert!(hit.face.is_some());
}

/// `scene()` with the group turned a quarter turn about Z through its corner (3, 0, 0): its
/// box now spans x 2..3, y 0..1. Returns the turn too (the group's own coordinates to the
/// root's).
fn turned_scene() -> (Scene, InstanceId, DAffine3) {
    let (mut s, i) = scene();
    let turn = DAffine3::from_translation(p(3.0, 0.0, 0.0))
        * DAffine3::from_cols(DVec3::Y, -DVec3::X, DVec3::Z, DVec3::ZERO)
        * DAffine3::from_translation(p(-3.0, 0.0, 0.0));
    s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[i], turn)))
        .unwrap();
    let place = s.root().instance(i).unwrap().transform;
    (s, i, place)
}

#[test]
fn a_turned_groups_corners_and_faces_are_where_they_are_shown() {
    let (s, i, _) = turned_scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    // The group's corner (1, 0, 1) shows at (3, 1, 1).
    let corner = p(3.0, 1.0, 1.0);
    let hit = infer_in(
        &view,
        &c,
        px(&c, corner) + DVec2::new(3.0, 2.0),
        &Guides::default(),
        R,
    );
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(hit.point.abs_diff_eq(corner, 1e-9), "{:?}", hit.point);
    let top = p(2.5, 0.5, 1.0);
    let (owner, _, at) = face_under_in(&view, &c, px(&c, top)).unwrap();
    assert_eq!(owner, Owner::Instance(i));
    assert!(at.abs_diff_eq(top, 1e-9));
    assert_eq!(
        view.instance_box(i),
        Some([p(2.0, 0.0, 0.0), p(3.0, 1.0, 1.0)])
    );
}

#[test]
fn inside_a_turned_group_the_root_is_seen_in_the_groups_coordinates() {
    let (s, i, place) = turned_scene();
    let view = Scenery::of(&s, &[i]).unwrap();
    let c = Camera::new(DVec2::new(800.0, 600.0)).placed(place.inverse());
    // The root edge's end (1, -2, 0) in the group's coordinates.
    let end = place.inverse().transform_point3(p(1.0, -2.0, 0.0));
    let hit = infer_in(
        &view,
        &c,
        px(&c, end) + DVec2::new(2.0, 2.0),
        &Guides::default(),
        R,
    );
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(hit.point.abs_diff_eq(end, 1e-9), "{:?}", hit.point);
    // The group's own top face, under the cursor where it is shown.
    let top = p(0.5, 0.5, 1.0);
    let shown = Camera::new(DVec2::new(800.0, 600.0)).project(place.transform_point3(top));
    let hit = infer_in(&view, &c, shown.unwrap(), &Guides::default(), R);
    assert!(hit.face.is_some());
    assert!(hit.point.abs_diff_eq(top, 1e-9), "{:?}", hit.point);
}
