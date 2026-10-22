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
