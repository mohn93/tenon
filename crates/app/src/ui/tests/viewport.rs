//! What is drawn over the 3D view: the snap marker and its halo, the preview lines, the snap
//! chip and the flash, the view's cursor, and the orientation gizmo.

use egui::{Color32, FontFamily, Pos2, Rect, Vec2};
use glam::DVec2;
use session::{Camera, StandardView};
use tools::overlay::{ENDPOINT, HALO_PT, INTERSECTION, LINE_PT, MARKER_PT, ScreenLine, Shape};

use super::*;
use crate::feedback::{Chip, Hud};
use crate::ui::theme::GIZMO_AXIS;
use crate::ui::viewport::{chip_rect, cursor_icon, gizmo_ends};

/// The text shapes painted with `text`.
fn texts_named(h: &H, text: &str) -> Vec<egui::epaint::TextShape> {
    drawn_shapes(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t),
            _ => None,
        })
        .collect()
}

/// The circles painted in `fill`.
fn circles(h: &H, fill: Color32) -> Vec<egui::epaint::CircleShape> {
    drawn_shapes(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Circle(c) if c.fill == fill => Some(c),
            _ => None,
        })
        .collect()
}

/// The default panels with `hud` over the view.
fn with_hud(hud: Hud) -> Fixture {
    Fixture {
        hud,
        ..Fixture::default()
    }
}

fn chip(label: &str, value: Option<&str>, typing: bool) -> Option<Chip> {
    Some(Chip {
        label: Some(label.into()),
        value: value.map(Into::into),
        typing,
    })
}

#[test]
fn the_chip_shows_the_label_and_the_value() {
    let f = with_hud(Hud {
        cursor: Some(DVec2::new(300.0, 200.0)),
        chip: chip("Midpoint in Group", Some("1.5 m"), false),
        flash: Some("Not width,height: 2,x".into()),
        ..Hud::default()
    });
    let h = harness(f);
    let texts = drawn_texts(&h);
    for t in ["Midpoint in Group", "1.5 m", "Not width,height: 2,x"] {
        assert!(texts.iter().any(|s| s == t), "{t} in {texts:?}");
    }
}

#[test]
fn labels_are_inter_and_values_mono_in_the_accent_while_typing() {
    let f = with_hud(Hud {
        cursor: Some(DVec2::new(300.0, 200.0)),
        chip: chip("Endpoint", Some("3.75_"), true),
        ..Hud::default()
    });
    let h = harness(f);
    let format = |text: &str| {
        let shapes = texts_named(&h, text);
        let [t] = shapes.as_slice() else {
            panic!("one {text}: {shapes:?}")
        };
        t.galley.job.sections[0].format.clone()
    };
    let t = theme::tokens(egui::Theme::Dark);
    let label = format("Endpoint");
    assert_eq!(label.font_id.family, FontFamily::Proportional);
    assert_eq!(label.color, t.text_primary);
    let value = format("3.75_");
    assert_eq!(value.font_id.family, FontFamily::Monospace);
    assert_eq!(value.color, t.accent, "typed text is in the accent");
}

#[test]
fn the_flash_is_drawn_by_the_cursor() {
    let cursor = DVec2::new(300.0, 200.0);
    let f = with_hud(Hud {
        cursor: Some(cursor),
        flash: Some("Not width,height: 2,x".into()),
        ..Hud::default()
    });
    let h = harness(f);
    let at = h.state().central.min + Vec2::new(300.0, 200.0);
    let shapes = texts_named(&h, "Not width,height: 2,x");
    assert_eq!(shapes.len(), 1, "{shapes:?}");
    let offset = shapes[0].pos - at;
    assert!(
        offset.x > 0.0 && offset.y > 0.0 && offset.length() < 40.0,
        "the flash sits just past the cursor: {offset:?}"
    );
    // On the danger fill, in the text color for it.
    let t = theme::tokens(egui::Theme::Dark);
    assert_eq!(shapes[0].galley.job.sections[0].format.color, t.on_accent);
    assert!(drawn_shapes(&h).iter().any(|s| matches!(s,
        egui::Shape::Rect(r) if r.fill == t.danger && r.rect.contains(shapes[0].pos))));
}

#[test]
fn with_no_cursor_in_the_view_the_flash_sits_above_the_status_bar() {
    let f = with_hud(Hud {
        chip: chip("Endpoint", None, false),
        flash: Some("Not width,height: 2,x".into()),
        ..Hud::default()
    });
    let h = harness(f);
    let view = h.state().central;
    let shapes = texts_named(&h, "Not width,height: 2,x");
    assert_eq!(shapes.len(), 1, "{shapes:?}");
    let text = Rect::from_min_size(shapes[0].pos, shapes[0].galley.size());
    assert!(
        (text.center().x - view.center().x).abs() < 1.0,
        "centred: {text:?} in {view:?}"
    );
    assert!(
        text.max.y < view.max.y && text.max.y > view.max.y - 20.0,
        "just above the status bar: {text:?} in {view:?}"
    );
    assert!(
        !drawn_texts(&h).iter().any(|s| s == "Endpoint"),
        "no chip without a cursor"
    );
}

#[test]
fn the_chip_hides_over_a_window() {
    let f = Fixture {
        shortcuts: true,
        ..with_hud(Hud {
            cursor: Some(DVec2::new(300.0, 200.0)),
            chip: chip("Endpoint", None, false),
            ..Hud::default()
        })
    };
    let mut h = harness(f);
    assert!(drawn_texts(&h).iter().any(|s| s == "Endpoint"));
    let window = rect_of(&h, "Keyboard Shortcuts");
    h.event(egui::Event::PointerMoved(window.center()));
    h.run_ok();
    assert!(
        !drawn_texts(&h).iter().any(|s| s == "Endpoint"),
        "the window covers the chip's place"
    );
}

#[test]
fn the_chip_shows_with_the_pointer_over_the_view() {
    let mut h = harness(with_hud(Hud {
        cursor: Some(DVec2::new(300.0, 200.0)),
        chip: chip("Endpoint", None, false),
        ..Hud::default()
    }));
    let at = h.state().central.min + Vec2::new(300.0, 200.0);
    h.event(egui::Event::PointerMoved(at));
    h.run_ok();
    assert!(drawn_texts(&h).iter().any(|s| s == "Endpoint"));
}

#[test]
fn over_a_window_the_flash_sits_above_the_status_bar() {
    let f = Fixture {
        shortcuts: true,
        ..with_hud(Hud {
            cursor: Some(DVec2::new(300.0, 200.0)),
            flash: Some("Not width,height: 2,x".into()),
            ..Hud::default()
        })
    };
    let mut h = harness(f);
    let window = rect_of(&h, "Keyboard Shortcuts");
    h.event(egui::Event::PointerMoved(window.center()));
    h.run_ok();
    let view = h.state().central;
    let shapes = texts_named(&h, "Not width,height: 2,x");
    assert_eq!(shapes.len(), 1, "{shapes:?}");
    let text = Rect::from_min_size(shapes[0].pos, shapes[0].galley.size());
    assert!(
        (text.center().x - view.center().x).abs() < 1.0,
        "centred: {text:?} in {view:?}"
    );
    assert!(
        text.max.y < view.max.y && text.max.y > view.max.y - 20.0,
        "not under the window at the cursor: {text:?} in {view:?}"
    );
}

#[test]
fn each_tool_has_its_cursor() {
    use egui::CursorIcon as C;
    for (tool, icon) in [
        (ToolKind::Select, C::Default),
        (ToolKind::Line, C::Crosshair),
        (ToolKind::Rectangle, C::Crosshair),
        (ToolKind::Circle, C::Crosshair),
        (ToolKind::Polygon, C::Crosshair),
        (ToolKind::Arc, C::Crosshair),
        (ToolKind::TapeMeasure, C::Crosshair),
        (ToolKind::Rotate, C::Crosshair),
        (ToolKind::Scale, C::Crosshair),
        (ToolKind::Offset, C::Crosshair),
        (ToolKind::FollowMe, C::Crosshair),
        (ToolKind::Move, C::Move),
        (ToolKind::PushPull, C::ResizeVertical),
        (ToolKind::Paint, C::Default),
        (ToolKind::Erase, C::Default),
    ] {
        assert_eq!(cursor_icon(tool, false, false, false), icon, "{tool:?}");
    }
    assert_eq!(
        cursor_icon(ToolKind::Paint, false, true, false),
        C::PointingHand
    );
    assert_eq!(
        cursor_icon(ToolKind::Erase, false, true, false),
        C::PointingHand
    );
    assert_eq!(cursor_icon(ToolKind::Line, false, false, true), C::Grabbing);
    // The Paste tool places a point, whatever tool it goes back to.
    for tool in [ToolKind::Select, ToolKind::Move, ToolKind::Paint] {
        assert_eq!(
            cursor_icon(tool, true, true, false),
            C::Crosshair,
            "{tool:?}"
        );
    }
    assert_eq!(
        cursor_icon(ToolKind::Select, true, false, true),
        C::Grabbing
    );
}

#[test]
fn the_view_shows_the_tools_cursor_and_the_panels_keep_theirs() {
    let mut h = harness(Fixture {
        cursor: egui::CursorIcon::Crosshair,
        ..Default::default()
    });
    let view = h.state().central;
    h.event(egui::Event::PointerMoved(view.center()));
    h.run_ok();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Crosshair
    );
    h.event(egui::Event::PointerMoved(rect_of(&h, "Line").center()));
    h.run_ok();
    assert_ne!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Crosshair
    );
}

#[test]
fn a_cursor_a_widget_set_inside_the_view_stays() {
    let mut h = harness(Fixture {
        cursor: egui::CursorIcon::Crosshair,
        ..Default::default()
    });
    let view = h.state().central;
    // The dock's resize edge reaches a few points into the view.
    let edge = Pos2::new(view.max.x - 1.0, view.center().y);
    assert!(view.contains(edge));
    h.event(egui::Event::PointerMoved(edge));
    h.run_ok();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::ResizeHorizontal,
        "the dock's edge keeps its own cursor"
    );
}

#[test]
fn the_chip_stays_inside_the_view() {
    let view = Rect::from_min_size(Pos2::new(44.0, 36.0), Vec2::new(600.0, 400.0));
    let size = Vec2::new(120.0, 40.0);
    let r = chip_rect(Pos2::new(100.0, 100.0), size, view);
    assert_eq!(r.min, Pos2::new(114.0, 118.0));
    let r = chip_rect(Pos2::new(640.0, 430.0), size, view);
    assert!(view.contains_rect(r), "{r:?}");
    assert!(r.max.x <= 640.0 && r.max.y <= 430.0, "flipped left and up");
}

#[test]
fn the_hud_lands_on_its_pixels_at_2x() {
    // pixels_per_point 2: a marker at view pixel (200, 100) is drawn centred at
    // view.min + (100, 50) points, its dot 12 points across.
    let f = with_hud(Hud {
        marker: Some((DVec2::new(200.0, 100.0), Shape::Dot, ENDPOINT)),
        lines: vec![ScreenLine {
            a: DVec2::new(200.0, 100.0),
            b: DVec2::new(400.0, 100.0),
            color: tools::overlay::AXIS_X,
            width: LINE_PT,
        }],
        ..Hud::default()
    });
    let h = harness_scaled(f, 2.0);
    let view = h.state().central;
    let dots = circles(&h, Color32::from_rgb(0x2F, 0xBF, 0x4A));
    assert_eq!(dots.len(), 1);
    assert_eq!(dots[0].center, view.min + Vec2::new(100.0, 50.0));
    assert_eq!(dots[0].radius, MARKER_PT / 2.0);
    let red = Color32::from_rgb(0xE5, 0x48, 0x4D);
    // The gizmo's X axis is red too.
    let gizmo = Rect::from_center_size(
        rect_of(&h, "Switch Projection").center(),
        Vec2::splat(theme::GIZMO),
    );
    let lines: Vec<_> = drawn_shapes(&h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == red && !gizmo.contains(points[0]) =>
            {
                Some((points, stroke.width))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        lines,
        [(
            [
                view.min + Vec2::new(100.0, 50.0),
                view.min + Vec2::new(200.0, 50.0)
            ],
            LINE_PT
        )],
        "2 points wide, whatever the display"
    );
}

#[test]
fn the_halo_follows_the_canvas() {
    for (dark_canvas, halo) in [(false, Color32::WHITE), (true, Color32::BLACK)] {
        let f = with_hud(Hud {
            marker: Some((DVec2::new(200.0, 100.0), Shape::Dot, ENDPOINT)),
            dark_canvas,
            ..Hud::default()
        });
        let h = harness(f);
        let halos = circles(&h, halo);
        assert!(
            halos.iter().any(|c| c.radius == MARKER_PT / 2.0 + HALO_PT),
            "dark canvas {dark_canvas}: {halos:?}"
        );
    }
}

#[test]
fn the_hud_settles_in_one_frame() {
    let f = with_hud(Hud {
        cursor: Some(DVec2::new(300.0, 200.0)),
        marker: Some((DVec2::new(300.0, 200.0), Shape::Cross, INTERSECTION)),
        chip: chip("Intersection", None, false),
        flash: Some("2".into()),
        ..Hud::default()
    });
    let mut h = harness(f);
    assert_eq!(h.try_run().unwrap(), 1);
}

#[test]
fn the_gizmo_sets_standard_views_and_switches_projection() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Front View"),
        [UiAction::StandardView(StandardView::Front)]
    );
    assert_eq!(is_on(&h, "Switch Projection"), Some(false));
    assert_eq!(
        click(&mut h, "Switch Projection"),
        [UiAction::SetProjection(Projection::Parallel)]
    );
    let mut h = harness(Fixture {
        projection: Projection::Parallel,
        ..Default::default()
    });
    assert_eq!(is_on(&h, "Switch Projection"), Some(true));
    assert_eq!(
        click(&mut h, "Switch Projection"),
        [UiAction::SetProjection(Projection::Perspective)]
    );
}

#[test]
fn the_gizmo_sits_in_the_views_top_right() {
    let h = harness(Fixture::default());
    let (view, centre) = (h.state().central, rect_of(&h, "Switch Projection").center());
    let expected = Pos2::new(view.max.x - 12.0 - 36.0, view.min.y + 12.0 + 36.0);
    assert!(
        centre.distance(expected) < 1.0,
        "{centre:?} vs {expected:?}"
    );
}

#[test]
fn the_axis_ends_set_the_views_they_look_from() {
    let ends = gizmo_ends(&Camera::new(DVec2::new(800.0, 600.0)));
    let end = |v: StandardView| *ends.iter().find(|e| e.0 == v).unwrap();
    for (pos, neg) in [
        (StandardView::Right, StandardView::Left),
        (StandardView::Back, StandardView::Front),
        (StandardView::Top, StandardView::Bottom),
    ] {
        let (p, n) = (end(pos), end(neg));
        assert!(
            (p.1 + n.1).length() < 1e-3,
            "{pos:?} and {neg:?} are opposite"
        );
        assert!((p.2 + n.2).abs() < 1e-3);
        assert!(p.1.length() <= GIZMO_AXIS + 1e-3);
    }
    assert!(
        end(StandardView::Right).1.x > 0.0,
        "+X on the right in the default view"
    );
    assert!(end(StandardView::Top).1.y < 0.0, "+Z up the screen");
}

#[test]
fn an_axis_pointing_at_the_viewer_leaves_the_centre_clickable() {
    let mut camera = Camera::new(DVec2::new(1280.0, 800.0));
    camera.look_from(StandardView::Top);
    let mut h = harness(Fixture {
        camera,
        ..Default::default()
    });
    assert!(h.query_by_label("Top View").is_none());
    assert!(h.query_by_label("Bottom View").is_none());
    assert_eq!(
        click(&mut h, "Switch Projection"),
        [UiAction::SetProjection(Projection::Parallel)]
    );
}

#[test]
fn the_gizmo_takes_the_pointer_from_the_tools() {
    let h = harness(Fixture::default());
    let at = rect_of(&h, "Front View").center();
    let layer = h.ctx.layer_id_at(at).expect("the gizmo's area");
    assert_ne!(
        layer.order,
        egui::Order::Background,
        "Gui::wants_pointer_at is true there"
    );
    assert!(h.query_by_label("Switch Projection").is_some());
    let flying = harness(Fixture {
        flying: true,
        ..Default::default()
    });
    assert!(flying.query_by_label("Switch Projection").is_none());
}
