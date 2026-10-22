//! Section cuts, their fill and cut edges, and the plane entities in the scene lists and on
//! screen.

use std::collections::BTreeSet;

use kernel::EntityId;
use render::Snapshotter;
use render::scene::{SceneBase, scene_base_shown};
use render::sections::{CUT_FILL_COLOR, SectionLook};
use session::{Command, Document, SectionDisplay};

use crate::golden::{HEIGHT, WIDTH, add_box, doc, look, run};

/// A 2 x 1.5 x 1 m box at the origin and a plane through x = 1 keeping x >= 1 (the box's
/// middle), active in the root.
fn cut_box() -> Document {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: None,
        },
    );
    d
}

fn base(d: &Document, display: SectionDisplay) -> SceneBase {
    scene_base_shown(d.scene(), d.context(), false, display)
}

fn triangle_area(v: &[render::grid::LineVertex]) -> f64 {
    v.chunks(3)
        .map(|t| {
            let p = |k: usize| glam::Vec3::from(t[k].pos).as_dvec3();
            (p(1) - p(0)).cross(p(2) - p(0)).length() / 2.0
        })
        .sum()
}

fn length(v: &[render::grid::LineVertex]) -> f64 {
    v.chunks(2)
        .map(|s| {
            glam::Vec3::from(s[0].pos)
                .as_dvec3()
                .distance(glam::Vec3::from(s[1].pos).as_dvec3())
        })
        .sum()
}

#[test]
fn a_box_cut_through_its_middle_keeps_one_half_filled_and_outlined() {
    let d = cut_box();
    let b = base(&d, SectionDisplay::default());
    assert!(!b.faces.is_empty());
    for v in &b.faces {
        assert!(v.pos[0] >= 1.0 - 1e-5, "face vertex at x = {}", v.pos[0]);
    }
    for e in &b.edges {
        assert!(e.pos[0] >= 1.0 - 1e-5, "edge end at x = {}", e.pos[0]);
    }
    // The cut is the 1.5 x 1 rectangle: filled flat, outlined by its four sides.
    assert!((triangle_area(&b.cut_fill) - 1.5).abs() < 1e-5);
    assert!(b.cut_fill.iter().all(|v| v.color == CUT_FILL_COLOR));
    assert!((length(&b.cut_edges) - 5.0).abs() < 1e-5);
    // Nothing clipped away casts a shadow.
    for c in b.casters.chunks(3) {
        assert!(c[0][0] >= 1.0 - 1e-5);
    }
    // The box is cut in half, so the whole one had more of everything.
    let whole = base(
        &d,
        SectionDisplay {
            cuts: false,
            ..SectionDisplay::default()
        },
    );
    assert!(whole.faces.iter().any(|v| v.pos[0] < 0.5));
}

#[test]
fn display_settings_turn_the_cut_fill_and_the_planes_off() {
    let d = cut_box();
    let no_fill = base(
        &d,
        SectionDisplay {
            fill: false,
            ..SectionDisplay::default()
        },
    );
    assert!(no_fill.cut_fill.is_empty());
    assert!((length(&no_fill.cut_edges) - 5.0).abs() < 1e-5);
    assert_eq!(no_fill.sections.len(), 1);

    let no_cuts = base(
        &d,
        SectionDisplay {
            cuts: false,
            ..SectionDisplay::default()
        },
    );
    assert!(no_cuts.cut_fill.is_empty() && no_cuts.cut_edges.is_empty());
    assert!(no_cuts.faces.iter().any(|v| v.pos[0] < 0.5));
    assert_eq!(no_cuts.sections.len(), 1, "planes are still drawn");

    let no_planes = base(
        &d,
        SectionDisplay {
            planes: false,
            ..SectionDisplay::default()
        },
    );
    assert!(no_planes.sections.is_empty());
    assert!(!no_planes.cut_fill.is_empty(), "the cut is still made");
}

#[test]
fn a_model_without_planes_is_drawn_as_before() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let plain = base(&d, SectionDisplay::default());
    let off = base(
        &d,
        SectionDisplay {
            planes: false,
            cuts: false,
            fill: false,
        },
    );
    assert_eq!(plain, off);
    assert!(plain.cut_fill.is_empty() && plain.cut_edges.is_empty() && plain.sections.is_empty());
}

#[test]
fn an_inactive_plane_is_drawn_but_cuts_nothing() {
    let mut d = cut_box();
    run(&mut d, Command::DeactivateSection {});
    let b = base(&d, SectionDisplay::default());
    assert!(b.faces.iter().any(|v| v.pos[0] < 0.5));
    assert!(b.cut_fill.is_empty() && b.cut_edges.is_empty());
    assert_eq!(b.sections.len(), 1);
    assert!(!b.sections[0].active);
}

#[test]
fn the_plane_outline_is_sized_to_the_shown_bounds_and_marks_the_active_plane() {
    let d = cut_box();
    let b = base(&d, SectionDisplay::default());
    let look: &SectionLook = &b.sections[0];
    assert!(look.active);
    // The box's diagonal is sqrt(4 + 2.25 + 1) = 2.69: the side is 1.2 times that.
    let side = look.shape.corners[0].distance(look.shape.corners[1]);
    assert!((side - 1.2 * 7.25f64.sqrt()).abs() < 1e-6, "{side}");
    assert!((look.shape.center.x - 1.0).abs() < 1e-9);
    // Hidden planes (by tag) are not drawn: none here, so only check the selection flags.
    let (id, _) = d.model().sections().next().unwrap();
    let flags = b.spans.flags(&BTreeSet::from([EntityId::Section(id)]));
    assert_eq!(flags.sections, vec![true]);
    let lit = b.with_flags(&flags);
    assert_eq!(lit.sections[0].color, render::scene::SELECTED_EDGE_COLOR);
}

/// A box in a group and a second box beside it at the root, the group opened and a plane put
/// through x = 1 (the middle of the group's box).
fn group_cut() -> Document {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let ids: Vec<EntityId> = d
        .model()
        .faces()
        .map(|(id, _)| EntityId::Face(id))
        .chain(d.model().edges().map(|(id, _)| EntityId::Edge(id)))
        .collect();
    let out = run(&mut d, Command::MakeGroup { ids });
    let group = *out
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .unwrap();
    add_box(&mut d, [4.0, 0.0]);
    run(&mut d, Command::OpenInstance { id: group });
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: None,
        },
    );
    d
}

#[test]
fn a_plane_in_a_group_cuts_only_that_group() {
    let d = group_cut();
    let b = base(&d, SectionDisplay::default());
    let mut group_x = (f32::MAX, f32::MIN);
    let mut other_x = (f32::MAX, f32::MIN);
    for v in &b.faces {
        let x = v.pos[0];
        let range = if x < 3.0 { &mut group_x } else { &mut other_x };
        range.0 = range.0.min(x);
        range.1 = range.1.max(x);
    }
    assert!(
        group_x.0 >= 1.0 - 1e-5 && group_x.1 <= 2.0 + 1e-5,
        "{group_x:?}"
    );
    assert!(
        (other_x.0 - 4.0).abs() < 1e-5 && (other_x.1 - 6.0).abs() < 1e-5,
        "{other_x:?}"
    );
    assert!((triangle_area(&b.cut_fill) - 1.5).abs() < 1e-5);
    assert_eq!(b.sections.len(), 1);
}

#[test]
fn the_cut_follows_a_moved_group() {
    let mut d = group_cut();
    run(&mut d, Command::CloseInstance {});
    let group = d
        .model()
        .instances()
        .map(|(id, _)| EntityId::Instance(id))
        .next()
        .unwrap();
    run(
        &mut d,
        Command::Move {
            ids: vec![group],
            offset: [0.0, 10.0, 0.0],
        },
    );
    let b = base(&d, SectionDisplay::default());
    // The group's faces are 10 m along y now, and still only the half from x = 1.
    let moved: Vec<_> = b.faces.iter().filter(|v| v.pos[1] > 5.0).collect();
    assert!(!moved.is_empty());
    assert!(moved.iter().all(|v| v.pos[0] >= 1.0 - 1e-5));
    assert!(b.cut_fill.iter().all(|v| (v.pos[1] - 10.0) > -1e-5));
}

#[test]
fn headless_renders_show_the_fill_and_the_cut_edges() {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut d = cut_box();
    // From the cut-away side, looking along +x at the cut face.
    look(&mut d, [-6.0, 0.75, 0.5], [1.0, 0.75, 0.5]);
    let pixels = |snap: &mut Snapshotter, d: &Document| {
        snap.set_section_display(d.section_display);
        snap.capture(
            d.scene(),
            d.context(),
            d.selection(),
            &d.camera,
            d.lighting,
            WIDTH,
            HEIGHT,
        )
        .expect("capture")
    };
    let at = |p: &[u8], x: u32, y: u32| {
        let k = ((y * WIDTH + x) * 4) as usize;
        [p[k], p[k + 1], p[k + 2]]
    };
    let dark = |c: [u8; 3]| c.iter().all(|v| (50..=70).contains(v));
    let with_fill = pixels(&mut snap, &d);
    assert!(
        dark(at(&with_fill, WIDTH / 2, HEIGHT / 2)),
        "the cut face is dark grey: {:?}",
        at(&with_fill, WIDTH / 2, HEIGHT / 2)
    );
    run(
        &mut d,
        Command::SetSectionDisplay {
            planes: None,
            cuts: None,
            fill: Some(false),
        },
    );
    let without = pixels(&mut snap, &d);
    assert!(
        !dark(at(&without, WIDTH / 2, HEIGHT / 2)),
        "without the fill the inside of the box shows"
    );
    run(
        &mut d,
        Command::SetSectionDisplay {
            planes: None,
            cuts: Some(false),
            fill: Some(true),
        },
    );
    let whole = pixels(&mut snap, &d);
    assert_ne!(whole, with_fill);
    assert_ne!(whole, without);
}

#[test]
fn a_plane_on_a_hidden_tag_is_not_drawn_but_still_cuts() {
    let mut d = cut_box();
    let tag = run(
        &mut d,
        Command::AddTag {
            name: None,
            color: None,
        },
    )
    .changes
    .tags[0];
    let (id, _) = d.model().sections().next().unwrap();
    run(
        &mut d,
        Command::SetTag {
            ids: vec![EntityId::Section(id)],
            tag: Some(tag),
        },
    );
    run(
        &mut d,
        Command::ChangeTag {
            tag,
            name: None,
            color: None,
            visible: Some(false),
        },
    );
    let b = base(&d, SectionDisplay::default());
    assert!(b.sections.is_empty());
    assert!(b.faces.iter().all(|v| v.pos[0] >= 1.0 - 1e-5));
    assert!(!b.cut_fill.is_empty());
}

/// A box in a group with a plane through it (in the group's coordinates), the group closed:
/// the box is 2 x 1.5 x 1 at the origin, the plane keeps what `normal` points to.
fn group_with_plane(point: [f64; 3], normal: [f64; 3]) -> (Document, EntityId) {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let ids: Vec<EntityId> = d
        .model()
        .faces()
        .map(|(id, _)| EntityId::Face(id))
        .chain(d.model().edges().map(|(id, _)| EntityId::Edge(id)))
        .collect();
    let out = run(&mut d, Command::MakeGroup { ids });
    let group = *out
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .unwrap();
    run(&mut d, Command::OpenInstance { id: group });
    run(
        &mut d,
        Command::AddSectionPlane {
            point,
            normal,
            activate: None,
        },
    );
    run(&mut d, Command::CloseInstance {});
    (d, group)
}

/// Checks that the group's drawn faces are all on the kept side of the plane (given as its
/// point and normal in the group's own coordinates, `to_local` mapping root to those) and that
/// the fill lies in the plane with `area`.
fn assert_cut_by(
    d: &Document,
    to_local: impl Fn(DVec3) -> DVec3,
    point: DVec3,
    normal: DVec3,
    area: f64,
) {
    let b = base(d, SectionDisplay::default());
    let local = |p: [f32; 3]| to_local(glam::Vec3::from(p).as_dvec3());
    assert!(!b.faces.is_empty());
    for v in &b.faces {
        assert!(
            (local(v.pos) - point).dot(normal) >= -1e-4,
            "a face vertex on the cut-away side: {:?}",
            v.pos
        );
    }
    assert!(!b.cut_fill.is_empty(), "the cut is filled");
    for v in &b.cut_fill {
        assert!((local(v.pos) - point).dot(normal).abs() < 1e-4);
    }
    assert!(
        (triangle_area(&b.cut_fill) - area).abs() < 1e-4,
        "{}",
        triangle_area(&b.cut_fill)
    );
}

use glam::DVec3;

#[test]
fn a_plane_in_a_rotated_group_cuts_on_its_own_side() {
    let (mut d, group) = group_with_plane([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    run(
        &mut d,
        Command::Rotate {
            ids: vec![group],
            center: [0.0, 0.0, 0.0],
            axis: Some([0.0, 0.0, 1.0]),
            angle: 90.0,
            copies: None,
        },
    );
    // A quarter turn about z: local (x, y) is world (-y, x), so world (x, y) is local (y, -x).
    assert_cut_by(
        &d,
        |p| DVec3::new(p.y, -p.x, p.z),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::X,
        1.5,
    );
}

#[test]
fn a_tilted_plane_in_a_stretched_group_keeps_its_side() {
    let n = DVec3::new(1.0, 0.0, 1.0).normalize();
    let point = DVec3::new(1.0, 0.75, 0.5);
    let (mut d, group) = group_with_plane(point.to_array(), n.to_array());
    run(
        &mut d,
        Command::Scale {
            ids: vec![group],
            origin: [0.0, 0.0, 0.0],
            factors: [3.0, 1.0, 1.0],
        },
    );
    let b = base(&d, SectionDisplay::default());
    // In the group's coordinates every face vertex is on the kept side and the fill is on the
    // plane, whatever the stretch did to the plane's tilt in the world.
    let local = |p: [f32; 3]| {
        let p = glam::Vec3::from(p).as_dvec3();
        DVec3::new(p.x / 3.0, p.y, p.z)
    };
    assert!(!b.faces.is_empty());
    for v in &b.faces {
        assert!((local(v.pos) - point).dot(n) >= -1e-4, "{:?}", v.pos);
    }
    assert!(!b.cut_fill.is_empty());
    for v in &b.cut_fill {
        assert!((local(v.pos) - point).dot(n).abs() < 1e-4);
    }
    // The tilted cut of a 2 x 1.5 x 1 box through (1, 0.75, 0.5): a 1.5 wide rectangle
    // sqrt(2) long, stretched 3 along x only.
    let expected = {
        let (a, b) = (DVec3::new(0.5, 0.0, 1.0), DVec3::new(1.5, 0.0, 0.0));
        let stretched = |p: DVec3| DVec3::new(3.0 * p.x, p.y, p.z);
        stretched(a).distance(stretched(b)) * 1.5
    };
    assert!(
        (triangle_area(&b.cut_fill) - expected).abs() < 1e-3,
        "{}",
        triangle_area(&b.cut_fill)
    );
}

#[test]
fn a_plane_in_a_mirrored_group_keeps_the_mirrored_side() {
    let (mut d, group) = group_with_plane([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    run(
        &mut d,
        Command::Scale {
            ids: vec![group],
            origin: [0.0, 0.0, 0.0],
            factors: [-1.0, 1.0, 1.0],
        },
    );
    // Local x is world -x: the kept half is the one at world x <= -1.
    assert_cut_by(
        &d,
        |p| DVec3::new(-p.x, p.y, p.z),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::X,
        1.5,
    );
    let b = base(&d, SectionDisplay::default());
    assert!(b.faces.iter().all(|v| v.pos[0] <= -1.0 + 1e-4));
}

#[test]
fn a_box_with_a_cavity_is_filled_with_a_hole() {
    let mut d = doc();
    let outer = run(
        &mut d,
        Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [4.0, 4.0, 0.0],
            normal: None,
        },
    );
    crate::golden::raise(&mut d, crate::golden::created_face(&outer), 4.0);
    let inner = run(
        &mut d,
        Command::AddRectangle {
            corner: [1.0, 1.0, 1.0],
            opposite: [3.0, 3.0, 1.0],
            normal: None,
        },
    );
    crate::golden::raise(&mut d, crate::golden::created_face(&inner), 2.0);
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [0.0, 0.0, 2.0],
            normal: [0.0, 0.0, 1.0],
            activate: None,
        },
    );
    let b = base(&d, SectionDisplay::default());
    // The 4 x 4 square less the 2 x 2 hole.
    assert!(
        (triangle_area(&b.cut_fill) - 12.0).abs() < 1e-4,
        "{}",
        triangle_area(&b.cut_fill)
    );
    // Outlined twice over: 16 around, 8 around the hole.
    assert!(
        (length(&b.cut_edges) - 24.0).abs() < 1e-4,
        "{}",
        length(&b.cut_edges)
    );
}

#[test]
fn a_single_face_cut_shows_only_a_cut_edge() {
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        },
    );
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: None,
        },
    );
    let b = base(&d, SectionDisplay::default());
    assert!(b.cut_fill.is_empty());
    assert!(
        (length(&b.cut_edges) - 2.0).abs() < 1e-4,
        "{}",
        length(&b.cut_edges)
    );
    assert!(b.faces.iter().all(|v| v.pos[0] >= 1.0 - 1e-5));
}

#[test]
fn an_active_plane_is_orange_then_selected_then_hovered() {
    use render::HoverLook;
    use render::scene::SELECTED_EDGE_COLOR;
    use render::sections::{ACTIVE_SECTION_COLOR, FADED_ACTIVE_SECTION_COLOR};

    let Some(screen) = crate::headless::Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut d = cut_box();
    run(
        &mut d,
        Command::AddSectionPlane {
            point: [0.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: Some(false),
        },
    );
    let ids: Vec<EntityId> = d
        .model()
        .sections()
        .map(|(id, _)| EntityId::Section(id))
        .collect();
    let (active, inactive) = (ids[0], ids[1]);
    assert_eq!(
        d.model().active_section().map(EntityId::Section),
        Some(active)
    );
    let mut r = screen.renderer();
    r.set_scene(d.scene(), &[], &BTreeSet::new());
    assert_eq!(
        r.section_colors(),
        [ACTIVE_SECTION_COLOR, render::guides::GUIDE_COLOR]
    );
    // Selecting recolors both the active and the inactive plane.
    r.set_selection(&BTreeSet::from([active, inactive]));
    assert_eq!(r.section_colors(), [SELECTED_EDGE_COLOR; 2]);
    let hover = HoverLook {
        face: [1.0; 3],
        face_mix: 0.5,
        edge: [0.9, 0.1, 0.9, 1.0],
    };
    r.set_hover(Some((active, hover)));
    assert_eq!(r.section_colors(), [hover.edge, SELECTED_EDGE_COLOR]);
    r.set_hover(None);
    r.set_selection(&BTreeSet::new());
    assert_eq!(
        r.section_colors(),
        [ACTIVE_SECTION_COLOR, render::guides::GUIDE_COLOR]
    );
    // Hovering without a selection: the hover wins over the orange.
    r.set_hover(Some((active, hover)));
    assert_eq!(r.section_colors()[0], hover.edge);

    // A plane of the root seen from inside an open group is drawn faded.
    let (mut g, group) = group_with_plane([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    run(&mut g, Command::OpenInstance { id: group });
    run(&mut g, Command::DeactivateSection {});
    run(&mut g, Command::CloseInstance {});
    run(
        &mut g,
        Command::AddSectionPlane {
            point: [5.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            activate: None,
        },
    );
    run(&mut g, Command::OpenInstance { id: group });
    let b = base(&g, SectionDisplay::default());
    assert_eq!(b.sections.len(), 2);
    assert!(
        b.sections
            .iter()
            .any(|s| s.color == FADED_ACTIVE_SECTION_COLOR && s.active)
    );
}
