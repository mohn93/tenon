//! Inference and picking across groups: geometry inside instances and outside the open
//! instance is seen and snapped to, in the open context's coordinates.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    DefinitionKind, EntityId, Guide, InstanceId, MeshId, Model, Scene, Tag, TagEdit, TagId,
};
use session::{Camera, Ray};
use tools::{
    Guides, Owner, Reach, Scenery, SnapKind, edge_under_in, face_under_in, guide_under_where,
    infer_in, pick, select_box,
};

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

fn add_tag(s: &mut Scene, name: &str) -> TagId {
    s.add_tag(Tag {
        name: name.into(),
        color: [1, 2, 3],
        visible: true,
    })
    .unwrap()
    .0
}

fn hide(s: &mut Scene, tag: TagId) {
    s.change_tag(
        tag,
        TagEdit {
            visible: Some(false),
            ..TagEdit::default()
        },
    )
    .unwrap();
}

/// Every edge, face and instance a crossing box over the whole view selects.
fn boxed(view: &Scenery, c: &Camera) -> Vec<EntityId> {
    select_box(view, c, DVec2::ZERO, DVec2::new(800.0, 600.0), true)
}

#[test]
fn a_hidden_group_is_not_drawn_on_picked_or_snapped_to_even_where_its_contents_show() {
    let (mut s, i) = scene();
    let (shown, hidden) = (add_tag(&mut s, "Shown"), add_tag(&mut s, "Hidden"));
    let d = s.root().instance(i).unwrap().definition;
    let inside: Vec<EntityId> = {
        let m = &s.definition(d).unwrap().mesh;
        m.edges()
            .map(|(e, _)| EntityId::Edge(e))
            .chain(m.faces().map(|(f, _)| EntityId::Face(f)))
            .collect()
    };
    s.set_tags(MeshId::Definition(d), &inside, Some(shown))
        .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(i)], Some(hidden))
        .unwrap();
    // An edge behind the group's box, as seen from the eye.
    let c = camera(DVec3::ZERO);
    let behind = c.eye + (p(3.5, 0.5, 0.5) - c.eye) * 1.2;
    s.edit(MeshId::Root, |m| {
        m.add_edge(behind, behind + p(0.5, 0.0, 0.0))
    })
    .unwrap();
    let corner = p(4.0, 0.0, 1.0);
    let top = p(3.5, 0.5, 1.0);
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.len(), 2, "shown, the group is a part");
    assert!(boxed(&view, &c).contains(&EntityId::Instance(i)));
    hide(&mut s, hidden);
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.len(), 1, "hidden, it is left out");
    let snap = infer_in(
        &view,
        &c,
        px(&c, corner) + DVec2::new(3.0, 2.0),
        &Guides::default(),
        R,
    );
    assert!(!snap.point.abs_diff_eq(corner, 1e-6), "{snap:?}");
    assert!(face_under_in(&view, &c, px(&c, top)).is_none());
    let down = Ray {
        origin: p(3.5, 0.5, 10.0),
        dir: DVec3::NEG_Z,
    };
    assert!(pick(&view, &down, Reach::Around { radius: 0.01 }).is_empty());
    assert!(!boxed(&view, &c).contains(&EntityId::Instance(i)));
    assert_eq!(view.instance_box(i), None);
    // Its faces no longer hide the edge behind it.
    let hit = infer_in(&view, &c, px(&c, behind), &Guides::default(), R);
    assert_eq!(hit.kind, SnapKind::Endpoint);
    // Opened, the group is still seen and its geometry snapped to.
    let opened = Scenery::of(&s, &[i]).unwrap();
    assert_eq!(opened.part(0).owner, Owner::Open);
}

#[test]
fn hidden_faces_edges_and_guides_are_not_picked_or_snapped_to() {
    let (mut s, i) = scene();
    let tag = add_tag(&mut s, "Lines");
    // The group's top face (z = 1 in its coordinates) on the tag too.
    let d = s.root().instance(i).unwrap().definition;
    let top = {
        let m = &s.definition(d).unwrap().mesh;
        m.faces()
            .find(|(_, f)| f.normal.z.abs() > 0.5 && m.position(f.outer[0]).unwrap().z > 0.5)
            .unwrap()
            .0
    };
    s.set_tags(MeshId::Definition(d), &[EntityId::Face(top)], Some(tag))
        .unwrap();
    let through_box = Ray {
        origin: p(3.5, 0.5, 10.0),
        dir: DVec3::NEG_Z,
    };
    let faces_met = |view: &Scenery| {
        pick(view, &through_box, Reach::Around { radius: 0.0 })
            .iter()
            .map(|h| h.distance)
            .collect::<Vec<f64>>()
    };
    s.edit(MeshId::Root, |m| {
        m.add_guide(Guide::point(p(0.5, -4.0, 0.0)))
    })
    .unwrap();
    let edge = s.root().edges().next().unwrap().0;
    let guide = s.root().guides().next().unwrap().0;
    s.set_tags(
        MeshId::Root,
        &[EntityId::Edge(edge), EntityId::Guide(guide)],
        Some(tag),
    )
    .unwrap();
    let c = camera(DVec3::ZERO);
    let (mid, end) = (p(0.5, -2.0, 0.0), p(1.0, -2.0, 0.0));
    let guide_at = p(0.5, -4.0, 0.0);
    let all = |_: Owner| true;
    let view = Scenery::of(&s, &[]).unwrap();
    assert!(edge_under_in(&view, &c, px(&c, mid), R).is_some());
    assert!(guide_under_where(&view, &c, px(&c, guide_at), R, all).is_some());
    assert!(boxed(&view, &c).contains(&EntityId::Edge(edge)));
    assert_eq!(faces_met(&view), vec![9.0, 10.0]);
    hide(&mut s, tag);
    let view = Scenery::of(&s, &[]).unwrap();
    assert!(edge_under_in(&view, &c, px(&c, mid), R).is_none());
    assert!(guide_under_where(&view, &c, px(&c, guide_at), R, all).is_none());
    assert!(view.guides().is_empty());
    let snap = infer_in(
        &view,
        &c,
        px(&c, end) + DVec2::new(2.0, 1.0),
        &Guides::default(),
        R,
    );
    assert_ne!(snap.kind, SnapKind::Endpoint, "its ends have no shown edge");
    let boxed = boxed(&view, &c);
    assert!(!boxed.contains(&EntityId::Edge(edge)));
    assert!(!boxed.contains(&EntityId::Guide(guide)));
    let down = Ray {
        origin: p(0.5, -2.0, 10.0),
        dir: DVec3::NEG_Z,
    };
    assert!(pick(&view, &down, Reach::Around { radius: 0.01 }).is_empty());
    assert_eq!(faces_met(&view), vec![10.0], "only the bottom face");
    let (_, _, at) = face_under_in(&view, &c, px(&c, p(3.5, 0.5, 1.0))).unwrap();
    assert!(at.z < 1.0 - 1e-6, "the top is passed through: {at}");
}

#[test]
fn faces_whose_edges_are_all_hidden_keep_their_corners_and_box() {
    let (mut s, i) = scene();
    let lines = add_tag(&mut s, "Lines");
    let d = s.root().instance(i).unwrap().definition;
    let edges: Vec<EntityId> = s
        .definition(d)
        .unwrap()
        .mesh
        .edges()
        .map(|(e, _)| EntityId::Edge(e))
        .collect();
    s.set_tags(MeshId::Definition(d), &edges, Some(lines))
        .unwrap();
    hide(&mut s, lines);
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
    assert_eq!(
        hit.kind,
        SnapKind::Endpoint,
        "the faces' corner still snaps"
    );
    assert!(hit.point.abs_diff_eq(corner, 1e-9), "{:?}", hit.point);
    assert_eq!(
        view.instance_box(i),
        Some([p(3.0, 0.0, 0.0), p(4.0, 1.0, 1.0)])
    );
    assert!(boxed(&view, &c).contains(&EntityId::Instance(i)));
    let window = select_box(&view, &c, DVec2::ZERO, DVec2::new(800.0, 600.0), false);
    assert!(window.contains(&EntityId::Instance(i)));
}
