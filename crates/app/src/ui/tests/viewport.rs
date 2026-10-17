//! What is drawn over the 3D view: the snap marker and its halo, the preview lines, the snap
//! chip and the flash.

use egui::{Color32, FontFamily, Pos2, Rect, Vec2};
use glam::DVec2;
use tools::overlay::{ENDPOINT, HALO_PT, INTERSECTION, LINE_PT, MARKER_PT, ScreenLine, Shape};

use super::*;
use crate::feedback::{Chip, Hud};
use crate::ui::viewport::chip_rect;

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
    let lines: Vec<_> = drawn_shapes(&h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, stroke } if stroke.color == red => {
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
