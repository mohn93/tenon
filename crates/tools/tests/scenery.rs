//! Inference and picking across groups: geometry inside instances and outside the open
//! instance is seen and snapped to, in the open context's coordinates.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    DefinitionKind, EntityId, Guide, InstanceId, MeshId, Model, Scene, Tag, TagEdit, TagId,
};
use session::{AnchorInput, Camera, Command, Ray};
use tools::{
    DimensionTool, Guides, Label, NoteRequest, Owner, Reach, Scenery, SnapKind, TextTool, Tool,
    ToolContext, ToolEvent, ToolOutput, annotation_under_where, edge_under_in, edge_under_where,
    face_under_in, guide_under_where, infer_detail, infer_in, label_under, pick, select_box,
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
fn a_part_hidden_for_viewing_is_not_seen_picked_or_snapped_to() {
    let (mut s, group) = scene();
    let c = camera(DVec3::ZERO);
    let corner = p(4.0, 0.0, 1.0);
    let at = px(&c, corner) + DVec2::new(3.0, 2.0);
    let seen = |s: &Scene| {
        let view = Scenery::of(s, &[]).unwrap();
        (
            infer_in(&view, &c, at, &Guides::default(), R),
            face_under_in(&view, &c, px(&c, p(3.5, 0.5, 1.0))),
        )
    };
    let (hit, face) = seen(&s);
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(face.is_some());
    s.set_view_hidden(std::collections::BTreeSet::from([group]));
    let (hit, face) = seen(&s);
    assert_ne!(hit.kind, SnapKind::Endpoint);
    assert!(face.is_none(), "the hidden group has no face to pick");
    s.set_view_hidden(Default::default());
    assert_eq!(seen(&s).0.kind, SnapKind::Endpoint);
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
    select_box(view, c, DVec2::ZERO, DVec2::new(800.0, 600.0), true, &[])
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
    let window = select_box(&view, &c, DVec2::ZERO, DVec2::new(800.0, 600.0), false, &[]);
    assert!(window.contains(&EntityId::Instance(i)));
}

/// The scene with a dimension inside the group, along the box's front bottom edge (its
/// vertices, so it follows them), offset by -0.5 in y.
fn dimensioned() -> (Scene, InstanceId, EntityId) {
    let (mut s, i) = scene();
    let d = MeshId::Definition(s.root().instance(i).unwrap().definition);
    let dimension = s
        .edit(d, |m| {
            let at = |q: DVec3| m.vertices().find(|(_, v)| v.pos == q).unwrap().0;
            let (a, b) = (at(p(0.0, 0.0, 0.0)), at(p(1.0, 0.0, 0.0)));
            m.add_dimension(kernel::Dimension::Linear {
                a: kernel::Anchor::Vertex(a),
                b: kernel::Anchor::Vertex(b),
                offset: p(0.0, -0.5, 0.0),
                tag: None,
            })
        })
        .unwrap()
        .summary()
        .created[0];
    (s, i, dimension)
}

#[test]
fn annotations_are_seen_where_their_group_places_them_and_pick_the_group() {
    let (mut s, i, dimension) = dimensioned();
    let view = Scenery::of(&s, &[]).unwrap();
    let [(part, id, shape)] = view.annotations() else {
        panic!("{:?}", view.annotations());
    };
    assert_eq!(
        (view.part(*part).owner, *id),
        (Owner::Instance(i), dimension)
    );
    let session::AnnotationShape::Linear { line, .. } = shape else {
        panic!("{shape:?}");
    };
    assert_eq!(*line, [p(3.0, -0.5, 0.0), p(4.0, -0.5, 0.0)]);
    let c = camera(DVec3::ZERO);
    let on_line = px(&c, p(3.5, -0.5, 0.0));
    let any = |_: Owner| true;
    assert_eq!(
        annotation_under_where(&view, &c, on_line, R, any),
        Some((Owner::Instance(i), dimension))
    );
    assert_eq!(
        annotation_under_where(&view, &c, on_line, R, |o| o == Owner::Open),
        None
    );
    // The probe meets it once, at its line.
    let ray = c.ray(on_line);
    let reach = Reach::Screen {
        camera: &c,
        cursor: on_line,
        radius_px: R,
    };
    let hits: Vec<_> = pick(&view, &ray, reach)
        .into_iter()
        .filter(|h| h.id == dimension)
        .collect();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].point.abs_diff_eq(p(3.5, -0.5, 0.0), 1e-6));
    // Opened, it is the open context's, in the group's coordinates.
    let inside = Scenery::of(&s, &[i]).unwrap();
    let c = camera(p(3.0, 0.0, 0.0));
    let on_line = px(&c, p(0.5, -0.5, 0.0));
    assert_eq!(
        annotation_under_where(&inside, &c, on_line, R, any),
        Some((Owner::Open, dimension))
    );
    // On a hidden tag it is not seen.
    let tag = add_tag(&mut s, "Notes");
    let d = MeshId::Definition(s.root().instance(i).unwrap().definition);
    s.set_tags(d, &[dimension], Some(tag)).unwrap();
    hide(&mut s, tag);
    assert!(Scenery::of(&s, &[]).unwrap().annotations().is_empty());
}

#[test]
fn the_topmost_label_under_the_cursor_is_picked() {
    let label = |owner, id: &str, min: (f64, f64), max: (f64, f64)| Label {
        owner,
        id: id.parse().unwrap(),
        min: DVec2::new(min.0, min.1),
        max: DVec2::new(max.0, max.1),
    };
    let labels = [
        label(Owner::Open, "r1", (0.0, 0.0), (50.0, 20.0)),
        label(Owner::Open, "n2", (40.0, 10.0), (90.0, 30.0)),
        label(Owner::Outside, "n3", (100.0, 0.0), (150.0, 20.0)),
    ];
    let at = |x, y| DVec2::new(x, y);
    let open = |o: Owner| o == Owner::Open;
    assert_eq!(
        label_under(&labels, at(45.0, 15.0), open).map(|l| l.1.to_string()),
        Some("n2".into())
    );
    assert_eq!(
        label_under(&labels, at(10.0, 5.0), open).map(|l| l.1.to_string()),
        Some("r1".into())
    );
    assert_eq!(label_under(&labels, at(120.0, 5.0), open), None);
    assert_eq!(label_under(&labels, at(95.0, 5.0), |_| true), None);
}

/// Clicks `tool` at each of `points` from the root of `s`, seen by the default camera moved
/// closer, and returns what the last click gave.
fn clicked(s: &Scene, tool: &mut dyn Tool, points: &[DVec3]) -> ToolOutput {
    let view = Scenery::of(s, &[]).unwrap();
    let mut c = camera(DVec3::ZERO);
    c.eye = c.target + (c.eye - c.target) * 0.3;
    let selection = Default::default();
    let mut out = ToolOutput::default();
    for &q in points {
        let cursor = px(&c, q);
        let guides = Guides {
            anchor: tool.anchor(),
            ..Guides::default()
        };
        let hit = infer_in(&view, &c, cursor, &guides, R);
        let ctx = ToolContext {
            model: s.root(),
            scenery: &view,
            camera: &c,
            cursor,
            pick_radius: R,
            drag_threshold: 4.0,
            scale_factor: 1.0,
            to_world: glam::DAffine3::IDENTITY,
            quick_press: false,
            shift: false,
            selection: &selection,
            material: None,
            sample: false,
            labels: &[],
        };
        out = tool.handle(ToolEvent::Click(hit), &ctx);
    }
    out
}

/// Whether `anchor` is a fixed point at about `q`.
fn fixed_at(anchor: AnchorInput, q: DVec3) -> bool {
    matches!(anchor, AnchorInput::Point(a) if DVec3::from(a).abs_diff_eq(q, 1e-9))
}

#[test]
fn dimensions_and_notes_on_a_group_from_the_root_hold_fixed_points() {
    let (s, _) = scene();
    // The group's front bottom edge, from (3, 0, 0) to (4, 0, 0) in the root: its ends are
    // not the root's, so the dimension measures fixed points.
    let mut tool = DimensionTool::new();
    let out = clicked(&s, &mut tool, &[p(3.3, 0.0, 0.0), p(3.5, -1.0, 0.0)]);
    let [Command::AddDimension { a, b, offset }] = out.commands[..] else {
        panic!("{:?}", out.commands);
    };
    let ends = [a, b];
    assert!(
        ends.iter().any(|e| fixed_at(*e, p(3.0, 0.0, 0.0)))
            && ends.iter().any(|e| fixed_at(*e, p(4.0, 0.0, 0.0))),
        "{ends:?}"
    );
    assert!(DVec3::from(offset).abs_diff_eq(p(0.0, -1.0, 0.0), 1e-6));
    // The root's own edge is measured by its vertices.
    let out = clicked(&s, &mut tool, &[p(0.3, -2.0, 0.0), p(0.5, -3.0, 0.0)]);
    assert!(
        matches!(out.commands[..], [Command::AddEdgeDimension { .. }]),
        "{:?}",
        out.commands
    );
    // A note on the group's corner points at the corner's place.
    let mut tool = TextTool::new();
    let out = clicked(&s, &mut tool, &[p(4.0, 0.0, 0.0), p(5.0, -1.0, 0.0)]);
    let Some(NoteRequest::Leader { anchor, .. }) = out.note else {
        panic!("{:?}", out.note);
    };
    assert!(fixed_at(anchor, p(4.0, 0.0, 0.0)), "{anchor:?}");
}

// Section planes: what the active planes cut away is not picked, inferred or box-selected.

/// `scene()` with the root's plane through x = 3.5 keeping x <= 3.5 (half of the group's box)
/// active; returns the plane too.
fn cut_scene() -> (Scene, InstanceId, kernel::SectionId) {
    let (mut s, i) = scene();
    let mut id = None;
    s.edit(MeshId::Root, |m| {
        let plane = kernel::SectionPlane::new(p(3.5, 0.0, 0.0), -DVec3::X);
        let c = m.add_section(plane, true)?;
        id = m.active_section();
        Ok(c)
    })
    .unwrap();
    (s, i, id.unwrap())
}

#[test]
fn faces_on_the_cut_away_side_are_not_picked_and_the_kept_side_is() {
    let (s, i, _) = cut_scene();
    let c = camera(DVec3::ZERO);
    let view = Scenery::of(&s, &[]).unwrap();
    let away = p(3.75, 0.5, 1.0);
    let kept = p(3.25, 0.5, 1.0);
    assert_eq!(
        face_under_in(&view, &c, px(&c, kept)).map(|(o, ..)| o),
        Some(Owner::Instance(i))
    );
    assert_eq!(face_under_in(&view, &c, px(&c, away)), None);
    // The same inference does not stand on the cut-away face.
    let hit = infer_in(&view, &c, px(&c, away), &Guides::default(), R);
    assert_ne!(hit.kind, SnapKind::OnFace, "{hit:?}");
    // Cuts off: everything is there again.
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    assert!(face_under_in(&whole, &c, px(&c, away)).is_some());
}

#[test]
fn points_and_edges_on_the_cut_away_side_are_not_snapped_to() {
    let (s, _, _) = cut_scene();
    let c = camera(DVec3::ZERO);
    let view = Scenery::of(&s, &[]).unwrap();
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    let away = p(4.0, 0.0, 1.0);
    let kept = p(3.0, 0.0, 1.0);
    let near = |q: DVec3| px(&c, q) + DVec2::new(2.0, 1.0);
    let with = infer_in(&whole, &c, near(away), &Guides::default(), R);
    assert_eq!(with.kind, SnapKind::Endpoint);
    let without = infer_in(&view, &c, near(away), &Guides::default(), R);
    assert_ne!(without.kind, SnapKind::Endpoint, "{without:?}");
    let ok = infer_in(&view, &c, near(kept), &Guides::default(), R);
    assert_eq!(ok.kind, SnapKind::Endpoint);
    assert!(ok.point.abs_diff_eq(kept, 1e-9));
    // An edge wholly cut away is not under the cursor; one that is kept is.
    let on_away_edge = p(3.8, 0.0, 0.0);
    assert!(edge_under_in(&whole, &c, px(&c, on_away_edge), R).is_some());
    assert!(edge_under_in(&view, &c, px(&c, on_away_edge), R).is_none());
    assert!(edge_under_in(&view, &c, px(&c, p(3.2, 0.0, 0.0)), R).is_some());
}

#[test]
fn a_cut_away_face_hides_nothing_behind_it() {
    // The group's own plane (local x <= 0.5) cuts away the right half of its box; a root edge
    // behind that half is unaffected by the plane and is no longer hidden.
    let (mut s, i) = scene();
    let def = s.root().instance(i).unwrap().definition;
    s.edit(MeshId::Definition(def), |m| {
        m.add_section(kernel::SectionPlane::new(p(0.5, 0.0, 0.0), -DVec3::X), true)
    })
    .unwrap();
    let c = camera(DVec3::ZERO);
    let behind = c.eye + (p(3.9, 0.3, 0.6) - c.eye) * 1.2;
    s.edit(MeshId::Root, |m| {
        m.add_edge(behind, behind + p(0.0, 0.5, 0.0))
    })
    .unwrap();
    let view = Scenery::of(&s, &[]).unwrap();
    let hit = infer_in(&view, &c, px(&c, behind), &Guides::default(), R);
    assert_eq!(hit.kind, SnapKind::Endpoint, "{hit:?}");
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    let hit = infer_in(&whole, &c, px(&c, behind), &Guides::default(), R);
    assert_ne!(hit.kind, SnapKind::Endpoint, "{hit:?}");
}

#[test]
fn the_probe_leaves_out_what_is_cut_away() {
    let (s, _, _) = cut_scene();
    let c = camera(DVec3::ZERO);
    let view = Scenery::of(&s, &[]).unwrap();
    let ray = |q: DVec3| c.ray(px(&c, q));
    let faces = |scenery: &Scenery, q: DVec3| {
        pick(scenery, &ray(q), Reach::Around { radius: 0.0 })
            .iter()
            .filter(|h| matches!(h.id, EntityId::Face(_)))
            .count()
    };
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    assert!(faces(&whole, p(3.75, 0.5, 1.0)) > 0);
    assert_eq!(faces(&view, p(3.75, 0.5, 1.0)), 0);
    assert!(faces(&view, p(3.25, 0.5, 1.0)) > 0);
}

#[test]
fn a_box_select_skips_what_is_cut_away() {
    let (mut s, _, _) = cut_scene();
    // A loose edge at x 0..1 (kept) and another at 5..6 (cut away).
    s.edit(MeshId::Root, |m| {
        m.add_edge(p(5.0, -2.0, 0.0), p(6.0, -2.0, 0.0))
    })
    .unwrap();
    let c = camera(DVec3::ZERO);
    let view = Scenery::of(&s, &[]).unwrap();
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    let lo = px(&c, p(-1.0, -2.5, 0.0)).min(px(&c, p(7.0, -1.5, 0.0))) - DVec2::splat(20.0);
    let hi = px(&c, p(-1.0, -2.5, 0.0)).max(px(&c, p(7.0, -1.5, 0.0))) + DVec2::splat(20.0);
    let edges = |scenery: &Scenery| {
        select_box(scenery, &c, lo, hi, false, &[])
            .into_iter()
            .filter(|id| matches!(id, EntityId::Edge(_)))
            .count()
    };
    assert_eq!(edges(&whole), 2);
    assert_eq!(edges(&view), 1);
}

#[test]
fn a_section_plane_is_picked_by_its_outline_and_box_selected() {
    let (s, _, id) = cut_scene();
    let c = camera(DVec3::ZERO);
    let view = Scenery::of(&s, &[]).unwrap();
    let (_, sid, shape) = view.sections()[0];
    assert_eq!(sid, EntityId::Section(id));
    let corner = shape.corners[0];
    let on_outline = (shape.corners[0] + shape.corners[1]) * 0.5;
    let found = annotation_under_where(&view, &c, px(&c, on_outline), R, |o| o == Owner::Open);
    assert_eq!(found, Some((Owner::Open, EntityId::Section(id))));
    // A crossing box touching the outline selects it; a window box needs all four corners.
    let (a, b) = (
        px(&c, corner) - DVec2::splat(5.0),
        px(&c, corner) + DVec2::splat(5.0),
    );
    assert!(select_box(&view, &c, a, b, true, &[]).contains(&EntityId::Section(id)));
    assert!(!select_box(&view, &c, a, b, false, &[]).contains(&EntityId::Section(id)));
    let all: Vec<DVec2> = shape.corners.iter().map(|q| px(&c, *q)).collect();
    let lo = all.iter().fold(all[0], |m, q| m.min(*q)) - DVec2::splat(5.0);
    let hi = all.iter().fold(all[0], |m, q| m.max(*q)) + DVec2::splat(5.0);
    assert!(select_box(&view, &c, lo, hi, false, &[]).contains(&EntityId::Section(id)));
}

#[test]
fn a_plane_in_a_group_cuts_only_the_group() {
    let (mut s, i) = scene();
    let def = s.root().instance(i).unwrap().definition;
    // The group's own plane through its local x = 0.5, keeping x <= 0.5.
    s.edit(MeshId::Definition(def), |m| {
        m.add_section(kernel::SectionPlane::new(p(0.5, 0.0, 0.0), -DVec3::X), true)
    })
    .unwrap();
    let c = camera(DVec3::ZERO);
    let view = Scenery::of(&s, &[]).unwrap();
    // The group (at x 3..4) loses x 3.5..4; the root's loose edge at x 0..1 stays.
    assert!(face_under_in(&view, &c, px(&c, p(3.75, 0.5, 1.0))).is_none());
    assert!(face_under_in(&view, &c, px(&c, p(3.25, 0.5, 1.0))).is_some());
    assert!(edge_under_in(&view, &c, px(&c, p(0.5, -2.0, 0.0)), R).is_some());
}

#[test]
fn with_planes_off_a_plane_is_not_picked_or_box_selected() {
    let (s, _, id) = cut_scene();
    let c = camera(DVec3::ZERO);
    let shape = Scenery::of(&s, &[]).unwrap().sections()[0].2;
    let on_outline = (shape.corners[0] + shape.corners[1]) * 0.5;
    let display = |planes| session::SectionDisplay {
        planes,
        cuts: true,
        fill: true,
    };
    let on = Scenery::of(&s, &[]).unwrap().section_display(display(true));
    let off = Scenery::of(&s, &[])
        .unwrap()
        .section_display(display(false));
    let pick_at =
        |v: &Scenery| annotation_under_where(v, &c, px(&c, on_outline), R, |o| o == Owner::Open);
    assert_eq!(pick_at(&on), Some((Owner::Open, EntityId::Section(id))));
    assert_eq!(pick_at(&off), None);
    let (a, b) = (
        px(&c, shape.corners[0]) - DVec2::splat(5.0),
        px(&c, shape.corners[0]) + DVec2::splat(5.0),
    );
    assert!(select_box(&on, &c, a, b, true, &[]).contains(&EntityId::Section(id)));
    assert!(!select_box(&off, &c, a, b, true, &[]).contains(&EntityId::Section(id)));
}

#[test]
fn a_box_touching_only_the_cut_away_half_of_a_sectioned_group_does_not_select_it() {
    let (mut s, i) = scene();
    let def = s.root().instance(i).unwrap().definition;
    s.edit(MeshId::Definition(def), |m| {
        m.add_section(kernel::SectionPlane::new(p(0.5, 0.0, 0.0), -DVec3::X), true)
    })
    .unwrap();
    let c = camera(DVec3::ZERO);
    let around = |q: DVec3| (px(&c, q) - DVec2::splat(2.0), px(&c, q) + DVec2::splat(2.0));
    let (a, b) = around(p(4.0, 0.0, 1.0));
    let cut = Scenery::of(&s, &[]).unwrap();
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    let has = |v: &Scenery| select_box(v, &c, a, b, true, &[]).contains(&EntityId::Instance(i));
    assert!(has(&whole), "uncut, the box touches the group");
    assert!(!has(&cut), "cut, the group's kept half lies elsewhere");
    let (a, b) = around(p(3.25, 0.0, 0.5));
    assert!(select_box(&cut, &c, a, b, true, &[]).contains(&EntityId::Instance(i)));
}

#[test]
fn a_shape_is_cut_away_when_each_corner_is_cut_by_some_plane() {
    // The root's plane keeps y <= 0.5; the group's own keeps its local x <= 0.5 (world 3.5).
    let (mut s, i) = scene();
    let def = s.root().instance(i).unwrap().definition;
    s.edit(MeshId::Definition(def), |m| {
        m.add_section(kernel::SectionPlane::new(p(0.5, 0.0, 0.0), -DVec3::X), true)
    })
    .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_section(kernel::SectionPlane::new(p(0.0, 0.5, 0.0), -DVec3::Y), true)
    })
    .unwrap();
    let view = Scenery::of(&s, &[]).unwrap();
    let part = view
        .parts()
        .find(|part| part.owner == Owner::Instance(i))
        .unwrap();
    // One end cut by the group's plane only, the other by the root's only.
    assert!(part.cuts_away(&[p(3.8, 0.2, 0.0), p(3.2, 0.8, 0.0)]));
    // One end kept.
    assert!(!part.cuts_away(&[p(3.2, 0.2, 0.0), p(3.2, 0.8, 0.0)]));
    // The root's own part is cut by its plane alone.
    assert!(
        view.part(0)
            .cuts_away(&[p(0.0, 0.8, 0.0), p(1.0, 0.9, 0.0)])
    );
    assert!(
        !view
            .part(0)
            .cuts_away(&[p(0.0, 0.2, 0.0), p(1.0, 0.9, 0.0)])
    );
    assert!(
        !Scenery::of(&s, &[])
            .unwrap()
            .cut(false)
            .part(0)
            .cuts_away(&[p(0.0, 0.8, 0.0)])
    );
}

#[test]
fn box_select_looks_only_at_the_kept_part_of_edges_and_faces() {
    let (mut s, _, _) = cut_scene();
    // A root edge and a root square across the plane at x = 3.5.
    s.edit(MeshId::Root, |m| {
        let mut c = m.add_edge(p(3.0, -2.0, 0.0), p(4.0, -2.0, 0.0))?;
        c.ops.extend(
            m.add_polygon(&[
                p(3.0, -5.0, 0.0),
                p(4.0, -5.0, 0.0),
                p(4.0, -4.0, 0.0),
                p(3.0, -4.0, 0.0),
            ])?
            .ops,
        );
        Ok(c)
    })
    .unwrap();
    let c = camera(DVec3::ZERO);
    let cut = Scenery::of(&s, &[]).unwrap();
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    let kinds = |v: &Scenery, at: DVec3| {
        let (a, b) = (
            px(&c, at) - DVec2::splat(2.0),
            px(&c, at) + DVec2::splat(2.0),
        );
        let ids = select_box(v, &c, a, b, true, &[]);
        (
            ids.iter().any(|id| matches!(id, EntityId::Edge(_))),
            ids.iter().any(|id| matches!(id, EntityId::Face(_))),
        )
    };
    // Over the cut-away half: selected uncut, nothing cut.
    assert!(kinds(&whole, p(3.8, -2.0, 0.0)).0);
    assert_eq!(kinds(&cut, p(3.8, -2.0, 0.0)), (false, false));
    assert!(kinds(&whole, p(3.8, -4.5, 0.0)).1);
    assert_eq!(kinds(&cut, p(3.8, -4.5, 0.0)), (false, false));
    // Over the kept half: still selected.
    assert!(kinds(&cut, p(3.2, -2.0, 0.0)).0);
    assert!(kinds(&cut, p(3.2, -4.5, 0.0)).1);
}

#[test]
fn the_kept_box_of_an_instance_is_its_whole_box_when_nothing_cuts_it() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.kept_instance_box(i), view.instance_box(i));
    // A plane in the root that cuts the group does not change the other answer.
    let (s, i, _) = cut_scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let [lo, hi] = view.kept_instance_box(i).unwrap();
    assert!(
        (lo.x - 3.0).abs() < 1e-9 && (hi.x - 3.5).abs() < 1e-9,
        "{lo} {hi}"
    );
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    assert_eq!(whole.kept_instance_box(i), whole.instance_box(i));
}

#[test]
fn the_fill_of_a_cut_hides_what_lies_behind_it() {
    let (s, _, _) = cut_scene();
    // Looking along -x at the cut face of the group's box (x = 3.5), from the cut-away side.
    let mut c = camera(DVec3::ZERO);
    c.eye = p(9.0, 0.5, 0.5);
    c.target = p(3.5, 0.5, 0.5);
    let view = Scenery::of(&s, &[]).unwrap();
    let whole = Scenery::of(&s, &[]).unwrap().cut(false);
    let at = px(&c, p(3.5, 0.5, 0.5));
    // Uncut the near wall (x = 4) is met; cut, the fill is in the way and nothing is met.
    assert!(face_under_in(&whole, &c, at).is_some());
    assert!(face_under_in(&view, &c, at).is_none());
    let faces = |v: &Scenery| {
        pick(v, &c.ray(at), Reach::Around { radius: 0.0 })
            .iter()
            .filter(|h| matches!(h.id, EntityId::Face(_)))
            .count()
    };
    assert!(faces(&whole) > 0);
    assert_eq!(faces(&view), 0);
}

#[test]
fn at_most_a_thousand_plane_outlines_are_drawn_and_picked() {
    let (mut s, _) = scene();
    s.edit(MeshId::Root, |m| {
        let mut change = kernel::ChangeSet::default();
        for k in 0..1005 {
            let plane = kernel::SectionPlane::new(p(k as f64, 0.0, 0.0), DVec3::X);
            change.ops.extend(m.add_section(plane, false)?.ops);
        }
        Ok(change)
    })
    .unwrap();
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.sections().len(), session::MAX_SECTION_OUTLINES);
}

#[test]
fn with_the_fill_off_the_cut_hides_nothing() {
    let (s, _, _) = cut_scene();
    let mut c = camera(DVec3::ZERO);
    c.eye = p(9.0, 0.5, 0.5);
    c.target = p(3.5, 0.5, 0.5);
    let at = px(&c, p(3.5, 0.5, 0.5));
    let display = |fill| session::SectionDisplay {
        planes: true,
        cuts: true,
        fill,
    };
    let on = Scenery::of(&s, &[]).unwrap().section_display(display(true));
    let off = Scenery::of(&s, &[])
        .unwrap()
        .section_display(display(false));
    assert!(face_under_in(&on, &c, at).is_none());
    assert!(face_under_in(&off, &c, at).is_some());
}

/// Hides entities of `mesh` by their own flag.
fn flag_hidden(s: &mut Scene, mesh: MeshId, ids: &[EntityId]) {
    s.edit(mesh, |m| Ok(m.hide_entities(ids, true).0)).unwrap();
}

#[test]
fn an_instance_hidden_by_its_flag_is_not_seen_picked_or_snapped_to_at_any_depth() {
    let (mut s, i) = scene();
    let c = camera(DVec3::ZERO);
    let behind = c.eye + (p(3.5, 0.5, 0.5) - c.eye) * 1.2;
    s.edit(MeshId::Root, |m| {
        m.add_edge(behind, behind + p(0.5, 0.0, 0.0))
    })
    .unwrap();
    let corner = p(4.0, 0.0, 1.0);
    let top = p(3.5, 0.5, 1.0);
    let down = Ray {
        origin: p(3.5, 0.5, 10.0),
        dir: DVec3::NEG_Z,
    };
    // Wrap the group in an outer group, so the box is one level down.
    let (outer, _) = s
        .make_group(MeshId::Root, &[], &[], &[i], DefinitionKind::Group, "")
        .unwrap();
    let outer_def = MeshId::Definition(s.root().instance(outer).unwrap().definition);
    let inner = s.mesh(outer_def).unwrap().instances().next().unwrap().0;
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.len(), 3, "root, outer and inner are parts");
    assert!(!pick(&view, &down, Reach::Around { radius: 0.01 }).is_empty());
    flag_hidden(&mut s, outer_def, &[EntityId::Instance(inner)]);
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.len(), 2, "the hidden instance's box is left out");
    let snap = infer_in(
        &view,
        &c,
        px(&c, corner) + DVec2::new(3.0, 2.0),
        &Guides::default(),
        R,
    );
    assert!(!snap.point.abs_diff_eq(corner, 1e-6), "{snap:?}");
    assert!(face_under_in(&view, &c, px(&c, top)).is_none());
    assert!(pick(&view, &down, Reach::Around { radius: 0.01 }).is_empty());
    // Its faces no longer hide the edge behind it.
    let hit = infer_in(&view, &c, px(&c, behind), &Guides::default(), R);
    assert_eq!(hit.kind, SnapKind::Endpoint);
    // The same at the root: the outer group hidden hides everything below it.
    s.edit(outer_def, |m| {
        Ok(m.hide_entities(&[EntityId::Instance(inner)], false).0)
    })
    .unwrap();
    flag_hidden(&mut s, MeshId::Root, &[EntityId::Instance(outer)]);
    let view = Scenery::of(&s, &[]).unwrap();
    assert_eq!(view.len(), 1);
    assert!(!boxed(&view, &c).contains(&EntityId::Instance(outer)));
    assert_eq!(view.instance_box(outer), None);
    // Opened, the hidden instance's own context is still seen.
    let opened = Scenery::of(&s, &[outer]).unwrap();
    assert_eq!(opened.part(0).owner, Owner::Open);
}

#[test]
fn faces_edges_and_guides_hidden_by_their_flag_are_not_picked_or_snapped_to() {
    let (mut s, i) = scene();
    let d = s.root().instance(i).unwrap().definition;
    let top = {
        let m = &s.definition(d).unwrap().mesh;
        m.faces()
            .find(|(_, f)| f.normal.z.abs() > 0.5 && m.position(f.outer[0]).unwrap().z > 0.5)
            .unwrap()
            .0
    };
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
    let c = camera(DVec3::ZERO);
    let (mid, end) = (p(0.5, -2.0, 0.0), p(1.0, -2.0, 0.0));
    let guide_at = p(0.5, -4.0, 0.0);
    let all = |_: Owner| true;
    let view = Scenery::of(&s, &[]).unwrap();
    assert!(edge_under_in(&view, &c, px(&c, mid), R).is_some());
    assert_eq!(faces_met(&view), vec![9.0, 10.0]);
    flag_hidden(&mut s, MeshId::Definition(d), &[EntityId::Face(top)]);
    flag_hidden(
        &mut s,
        MeshId::Root,
        &[EntityId::Edge(edge), EntityId::Guide(guide)],
    );
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
    assert!(!boxed.contains(&EntityId::Face(top)));
    assert_eq!(faces_met(&view), vec![10.0], "only the bottom face");
    let (_, _, at) = face_under_in(&view, &c, px(&c, p(3.5, 0.5, 1.0))).unwrap();
    assert!(at.z < 1.0 - 1e-6, "the top is passed through: {at}");
}

#[test]
fn faces_whose_edges_are_all_hidden_by_flag_keep_their_corners_and_box() {
    let (mut s, i) = scene();
    let d = s.root().instance(i).unwrap().definition;
    let edges: Vec<EntityId> = s
        .definition(d)
        .unwrap()
        .mesh
        .edges()
        .map(|(e, _)| EntityId::Edge(e))
        .collect();
    flag_hidden(&mut s, MeshId::Definition(d), &edges);
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
}

#[test]
fn revealed_hidden_entities_are_picked_and_box_selected_but_never_inferred() {
    let (mut s, i) = scene();
    let d = s.root().instance(i).unwrap().definition;
    let top = {
        let m = &s.definition(d).unwrap().mesh;
        m.faces()
            .find(|(_, f)| f.normal.z.abs() > 0.5 && m.position(f.outer[0]).unwrap().z > 0.5)
            .unwrap()
            .0
    };
    s.edit(MeshId::Root, |m| {
        m.add_guide(Guide::point(p(0.5, -4.0, 0.0)))
    })
    .unwrap();
    let edge = s.root().edges().next().unwrap().0;
    let guide = s.root().guides().next().unwrap().0;
    flag_hidden(&mut s, MeshId::Definition(d), &[EntityId::Face(top)]);
    flag_hidden(
        &mut s,
        MeshId::Root,
        &[EntityId::Edge(edge), EntityId::Guide(guide)],
    );
    let c = camera(DVec3::ZERO);
    let (mid, end) = (p(0.5, -2.0, 0.0), p(1.0, -2.0, 0.0));
    let guide_at = p(0.5, -4.0, 0.0);
    let all = |_: Owner| true;
    let strict = Scenery::of(&s, &[]).unwrap();
    let view = Scenery::of(&s, &[]).unwrap().reveal_hidden(true);
    assert!(view.reveals_hidden() && !strict.reveals_hidden());
    // Picking meets them.
    assert!(edge_under_where(&strict, &c, px(&c, mid), R, all).is_none());
    assert!(edge_under_where(&view, &c, px(&c, mid), R, all).is_some());
    assert!(guide_under_where(&strict, &c, px(&c, guide_at), R, all).is_none());
    assert!(guide_under_where(&view, &c, px(&c, guide_at), R, all).is_some());
    let (_, _, at) = face_under_in(&strict, &c, px(&c, p(3.5, 0.5, 1.0))).unwrap();
    assert!(at.z < 1.0 - 1e-6, "the top is passed through");
    let (_, _, at) = face_under_in(&view, &c, px(&c, p(3.5, 0.5, 1.0))).unwrap();
    assert!((at.z - 1.0).abs() < 1e-6, "the revealed top is met: {at}");
    let down = Ray {
        origin: p(0.5, -2.0, 10.0),
        dir: DVec3::NEG_Z,
    };
    assert!(pick(&strict, &down, Reach::Around { radius: 0.01 }).is_empty());
    assert!(!pick(&view, &down, Reach::Around { radius: 0.01 }).is_empty());
    let boxed_view = boxed(&view, &c);
    assert!(boxed_view.contains(&EntityId::Edge(edge)));
    assert!(boxed_view.contains(&EntityId::Guide(guide)));
    let boxed_strict = boxed(&strict, &c);
    assert!(!boxed_strict.contains(&EntityId::Edge(edge)));
    assert!(!boxed_strict.contains(&EntityId::Guide(guide)));
    // Inference ignores them whatever picking reveals: the revealed scenery snaps as the
    // strict one does, and finds no edge under the cursor.
    let near_end = px(&c, end) + DVec2::new(2.0, 1.0);
    let snap = infer_in(&view, &c, near_end, &Guides::default(), R);
    assert_eq!(snap, infer_in(&strict, &c, near_end, &Guides::default(), R));
    assert_ne!(snap.kind, SnapKind::Endpoint);
    assert!(edge_under_in(&view, &c, px(&c, mid), R).is_none());
    let (hit, _) = infer_detail(&view, &c, px(&c, guide_at), &Guides::default(), R);
    assert_ne!(hit.kind, SnapKind::Endpoint, "{hit:?}");
}

#[test]
fn a_revealed_hidden_instance_is_picked_and_its_parts_follow_the_strict_ones() {
    let (mut s, i) = scene();
    flag_hidden(&mut s, MeshId::Root, &[EntityId::Instance(i)]);
    let c = camera(DVec3::ZERO);
    let top = p(3.5, 0.5, 1.0);
    let strict = Scenery::of(&s, &[]).unwrap();
    let view = Scenery::of(&s, &[]).unwrap().reveal_hidden(true);
    assert_eq!(strict.len(), 1);
    assert_eq!(view.len(), 2, "the hidden group is a part when revealed");
    assert_eq!(view.part(0).owner, strict.part(0).owner, "the root first");
    assert_eq!(view.part(1).owner, Owner::Instance(i));
    assert_eq!(strict.instance_box(i), None);
    assert_eq!(
        view.instance_box(i),
        Some([p(3.0, 0.0, 0.0), p(4.0, 1.0, 1.0)])
    );
    assert!(boxed(&view, &c).contains(&EntityId::Instance(i)));
    assert!(!boxed(&strict, &c).contains(&EntityId::Instance(i)));
    let down = Ray {
        origin: p(3.5, 0.5, 10.0),
        dir: DVec3::NEG_Z,
    };
    let hits = pick(&view, &down, Reach::Around { radius: 0.01 });
    assert!(hits.iter().any(|h| h.part == 1), "{hits:?}");
    assert!(face_under_in(&view, &c, px(&c, top)).is_some());
    assert!(face_under_in(&strict, &c, px(&c, top)).is_none());
    // The strict view of the revealing scenery has the strict parts only.
    let again = view.seen_strictly();
    assert_eq!(again.len(), 1);
    assert!(!again.reveals_hidden());
    // A hidden tag stays hidden when revealed.
    let tag = add_tag(&mut s, "Gone");
    s.set_tags(MeshId::Root, &[EntityId::Instance(i)], Some(tag))
        .unwrap();
    hide(&mut s, tag);
    let view = Scenery::of(&s, &[]).unwrap().reveal_hidden(true);
    assert_eq!(view.len(), 1);
    assert!(face_under_in(&view, &c, px(&c, top)).is_none());
}
