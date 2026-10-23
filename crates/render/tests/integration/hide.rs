//! Entities hidden by their own flag in what the renderer draws, at any instance depth.

use glam::DVec3;
use kernel::{DefinitionKind, EntityId, Guide, InstanceId, MeshId, Model, Scene};
use render::scene::{
    GHOST_EDGE_COLOR, GHOST_FACE_ALPHA, GlassView, SceneBase, X_RAY_ALPHA, scene_base,
    scene_base_shown,
};
use session::SectionDisplay;

fn square_at(m: &mut Model, x: f64) {
    let p = |dx: f64, y: f64| DVec3::new(x + dx, y, 0.0);
    m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
        .unwrap();
}

fn counts(b: &SceneBase) -> (usize, usize, usize) {
    (b.faces.len(), b.edges.len(), b.guides.len())
}

fn hide(s: &mut Scene, mesh: MeshId, ids: &[EntityId]) {
    s.edit(mesh, |m| Ok(m.hide_entities(ids, true).0)).unwrap();
}

/// A square component at x 0, placed again at x 3, a loose square at x 6 and a guide point.
/// Returns the scene, the component's definition mesh and its two instances.
fn scene() -> (Scene, MeshId, Vec<InstanceId>) {
    let mut m = Model::new();
    square_at(&mut m, 0.0);
    m.add_guide(Guide::point(DVec3::new(9.0, 9.0, 0.0)))
        .unwrap();
    let edges: Vec<_> = m.edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    let mut s = Scene::from_root(m);
    let (first, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Square",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[first], DVec3::new(3.0, 0.0, 0.0), 1)
        .unwrap();
    s.edit(MeshId::Root, |m| {
        let p = |dx: f64, y: f64| DVec3::new(6.0 + dx, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
    })
    .unwrap();
    let ids = s.root().instances().map(|(id, _)| id).collect();
    let def = MeshId::Definition(s.root().instance(first).unwrap().definition);
    (s, def, ids)
}

#[test]
fn nothing_hidden_changes_nothing() {
    let (s, _, _) = scene();
    // Three squares of two triangles, each edge two vertices, 4 edges apiece.
    assert_eq!(counts(&scene_base(&s, &[])), (18, 24, 1));
}

#[test]
fn a_hidden_face_in_a_definition_is_left_out_of_both_placements() {
    let (mut s, def, _) = scene();
    let face = s.mesh(def).unwrap().faces().next().unwrap().0;
    hide(&mut s, def, &[EntityId::Face(face)]);
    let base = scene_base(&s, &[]);
    assert_eq!(counts(&base), (6, 24, 1), "only the loose square's face");
    assert!(base.faces.iter().all(|v| v.pos[0] >= 6.0));
    assert_eq!(base.casters.len(), 6, "no shadow from a hidden face");
}

#[test]
fn a_hidden_edge_is_not_drawn_but_its_face_is() {
    let (mut s, def, _) = scene();
    let edge = s.mesh(def).unwrap().edges().next().unwrap().0;
    hide(&mut s, def, &[EntityId::Edge(edge)]);
    assert_eq!(counts(&scene_base(&s, &[])), (18, 20, 1), "one edge, twice");
    let root_edge = s.root().edges().next().unwrap().0;
    hide(&mut s, MeshId::Root, &[EntityId::Edge(root_edge)]);
    assert_eq!(counts(&scene_base(&s, &[])), (18, 18, 1));
}

#[test]
fn a_hidden_instance_and_a_hidden_guide_are_left_out() {
    let (mut s, _, ids) = scene();
    hide(&mut s, MeshId::Root, &[EntityId::Instance(ids[0])]);
    let base = scene_base(&s, &[]);
    assert_eq!(counts(&base), (12, 16, 1));
    let guide = s.root().guides().next().unwrap().0;
    hide(&mut s, MeshId::Root, &[EntityId::Guide(guide)]);
    assert_eq!(counts(&scene_base(&s, &[])), (12, 16, 0));
    // Opened, the instance being edited stays drawn.
    assert_eq!(scene_base(&s, &[ids[0]]).faces.len(), 18);
}

#[test]
fn a_hidden_instance_two_levels_deep_hides_what_it_places() {
    let (mut s, _, ids) = scene();
    let (outer, _) = s
        .make_group(MeshId::Root, &[], &[], &ids, DefinitionKind::Group, "")
        .unwrap();
    let outer_def = MeshId::Definition(s.root().instance(outer).unwrap().definition);
    assert_eq!(scene_base(&s, &[]).faces.len(), 18);
    let inner = s.mesh(outer_def).unwrap().instances().next().unwrap().0;
    hide(&mut s, outer_def, &[EntityId::Instance(inner)]);
    assert_eq!(counts(&scene_base(&s, &[])), (12, 16, 1));
}

fn shown_with(s: &Scene, context: &[InstanceId], glass: GlassView) -> SceneBase {
    scene_base_shown(s, context, false, glass, SectionDisplay::default())
}

const XRAY: GlassView = GlassView {
    x_ray: true,
    hidden_geometry: false,
};
const GHOSTS: GlassView = GlassView {
    x_ray: false,
    hidden_geometry: true,
};

#[test]
fn with_both_off_the_base_is_unchanged() {
    let (mut s, def, ids) = scene();
    assert_eq!(
        shown_with(&s, &[], GlassView::default()),
        scene_base(&s, &[])
    );
    let face = s.mesh(def).unwrap().faces().next().unwrap().0;
    hide(&mut s, def, &[EntityId::Face(face)]);
    hide(&mut s, MeshId::Root, &[EntityId::Instance(ids[1])]);
    assert_eq!(
        shown_with(&s, &[], GlassView::default()),
        scene_base(&s, &[])
    );
}

#[test]
fn x_ray_draws_every_shown_face_as_glass_on_both_sides() {
    let (s, _, _) = scene();
    let plain = scene_base(&s, &[]);
    assert!(plain.glass().is_empty());
    let base = shown_with(&s, &[], XRAY);
    assert_eq!(counts(&base), counts(&plain), "the same geometry");
    assert_eq!(base.edges, plain.edges, "edges are as they were");
    assert_eq!(base.glass().len(), 6, "every triangle is in the glass pass");
    assert!(base.faces.iter().all(|v| v.alpha == [X_RAY_ALPHA; 2]));
    assert_eq!(base.casters, plain.casters, "shadows are unaffected");
}

#[test]
fn x_ray_keeps_a_more_transparent_side_and_leaves_hidden_faces_hidden() {
    let (mut s, def, _) = scene();
    let (clear, _) = s
        .add_material_with_opacity("Clear", [10, 20, 30], 0.3)
        .unwrap();
    let face = s.mesh(def).unwrap().faces().next().unwrap().0;
    s.paint(def, &[face], &[], kernel::Side::Front, Some(clear))
        .unwrap();
    let base = shown_with(&s, &[], XRAY);
    assert!(
        base.faces
            .iter()
            .any(|v| v.alpha[0] == 0.3 && v.alpha[1] == X_RAY_ALPHA)
    );
    hide(&mut s, def, &[EntityId::Face(face)]);
    assert_eq!(
        shown_with(&s, &[], XRAY).faces.len(),
        6,
        "hidden stays hidden"
    );
}

#[test]
fn hidden_geometry_draws_hidden_faces_edges_guides_and_instances_ghosted() {
    let (mut s, def, ids) = scene();
    let face = s.mesh(def).unwrap().faces().next().unwrap().0;
    let edge = s.mesh(def).unwrap().edges().next().unwrap().0;
    hide(&mut s, def, &[EntityId::Face(face), EntityId::Edge(edge)]);
    let off = scene_base(&s, &[]);
    assert_eq!(counts(&off), (6, 20, 1));
    let on = shown_with(&s, &[], GHOSTS);
    // Everything is drawn again: both placements' faces and edges.
    assert_eq!(counts(&on), (18, 24, 1));
    let ghost_faces: Vec<_> = on.faces.iter().filter(|v| v.pos[0] < 5.0).collect();
    assert_eq!(ghost_faces.len(), 12);
    assert!(ghost_faces.iter().all(|v| v.alpha == [GHOST_FACE_ALPHA; 2]));
    assert!(
        on.faces
            .iter()
            .filter(|v| v.pos[0] >= 6.0)
            .all(|v| v.alpha == [1.0; 2])
    );
    assert_eq!(on.glass().len(), 4, "the ghost faces are glass");
    let grey = on
        .edges
        .iter()
        .filter(|v| v.color == GHOST_EDGE_COLOR)
        .count();
    assert_eq!(grey, 4, "the hidden edge in both placements");
    // Ghosts cast no shadow.
    assert_eq!(on.casters.len(), 6);
    // A hidden tag still hides, hidden or not.
    let (tag, _) = s
        .add_tag(kernel::Tag {
            name: "Gone".into(),
            color: [1, 2, 3],
            visible: true,
        })
        .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(ids[0])], Some(tag))
        .unwrap();
    s.change_tag(
        tag,
        kernel::TagEdit {
            visible: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(shown_with(&s, &[], GHOSTS).faces.len(), 12);
    // A hidden guide is drawn in the ghost color.
    let guide = s.root().guides().next().unwrap().0;
    hide(&mut s, MeshId::Root, &[EntityId::Guide(guide)]);
    assert_eq!(counts(&shown_with(&s, &[], GHOSTS)).2, 1);
    assert_eq!(
        shown_with(&s, &[], GHOSTS).guides[0].color,
        GHOST_EDGE_COLOR
    );
    assert_eq!(counts(&scene_base(&s, &[])).2, 0);
}

#[test]
fn a_hidden_instance_is_ghosted_with_all_it_places_and_can_be_selected() {
    let (mut s, _, ids) = scene();
    hide(&mut s, MeshId::Root, &[EntityId::Instance(ids[0])]);
    let on = shown_with(&s, &[], GHOSTS);
    assert_eq!(on.faces.len(), 18);
    assert_eq!(on.faces.iter().filter(|v| v.alpha[0] < 1.0).count(), 6);
    assert_eq!(on.casters.len(), 12, "the shown squares still cast");
    // Its ranges are known, so selecting it lights its faces and edges.
    let flags = on
        .spans
        .flags(&[EntityId::Instance(ids[0])].into_iter().collect());
    assert!(flags.faces.iter().filter(|f| **f > 0.0).count() == 6);
    assert!(flags.edges.iter().filter(|f| **f > 0.0).count() == 8);
}

#[test]
fn ghosts_in_a_nested_hidden_instance_are_drawn_when_revealed() {
    let (mut s, _, ids) = scene();
    let (outer, _) = s
        .make_group(MeshId::Root, &[], &[], &ids, DefinitionKind::Group, "")
        .unwrap();
    let outer_def = MeshId::Definition(s.root().instance(outer).unwrap().definition);
    let inner = s.mesh(outer_def).unwrap().instances().next().unwrap().0;
    hide(&mut s, outer_def, &[EntityId::Instance(inner)]);
    assert_eq!(scene_base(&s, &[]).faces.len(), 12);
    let on = shown_with(&s, &[], GHOSTS);
    assert_eq!(on.faces.len(), 18);
    assert_eq!(on.faces.iter().filter(|v| v.alpha[0] < 1.0).count(), 6);
}

#[test]
fn glass_and_ghosts_show_in_a_capture_and_off_changes_nothing() {
    use std::collections::BTreeSet;
    let Some(mut snap) = render::Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let (w, h) = (64, 64);
    let mut cam = session::Camera::new(glam::DVec2::new(f64::from(w), f64::from(h)));
    cam.eye = DVec3::new(0.5, -1.5, 3.0);
    cam.target = DVec3::new(0.5, 0.5, 0.0);
    let at = cam.project(DVec3::new(0.4, 0.6, 0.0)).unwrap();
    let mut capture = |scene: &Scene, glass: GlassView| {
        snap.set_glass_view(glass);
        let px = snap
            .capture(
                scene,
                &[],
                &BTreeSet::new(),
                &cam,
                session::Lighting::Day,
                w,
                h,
            )
            .expect("capture");
        let i = ((at.y as u32 * w + at.x as u32) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let mut m = Model::new();
    square_at(&mut m, 0.0);
    let mut s = Scene::from_root(m);
    let empty = capture(&Scene::new(), GlassView::default());
    let plain = capture(&s, GlassView::default());
    assert_ne!(plain, empty);
    let xray = capture(&s, XRAY);
    assert_ne!(xray, plain, "glass lets the background through");
    assert_ne!(xray, empty);
    let face = s.root().faces().next().unwrap().0;
    hide(&mut s, MeshId::Root, &[EntityId::Face(face)]);
    assert_eq!(
        capture(&s, GlassView::default()),
        empty,
        "hidden, not drawn"
    );
    assert_eq!(capture(&s, XRAY), empty, "x-ray does not reveal it");
    let ghost = capture(&s, GHOSTS);
    assert_ne!(ghost, empty, "the ghost is drawn");
    assert_ne!(ghost, plain, "fainter than the face");
    assert_eq!(capture(&s, GlassView::default()), empty, "off again");
}
