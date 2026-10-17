use super::*;
use crate::ui::dock::{AREA_MIN, top_height};

/// Presses at `from`, then holds the primary button down while the pointer moves to `to` in
/// four steps, running after each.
fn press_and_move(h: &mut H, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(
            from + (to - from) * k as f32 / 4.0,
        ));
        h.run_ok();
    }
}

/// Releases the primary button at `at`, runs and returns the actions.
fn release(h: &mut H, at: Pos2) -> Vec<UiAction> {
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_ok();
    h.state_mut().take()
}

/// Drags from `from` by `by` with the primary button, then runs; returns the actions.
fn drag(h: &mut H, from: Pos2, by: Vec2) -> Vec<UiAction> {
    press_and_move(h, from, from + by);
    release(h, from + by)
}

#[test]
fn the_dock_reports_its_width_once_on_release() {
    let mut h = harness(Fixture::default());
    // The dock starts where the 3D view ends.
    let edge = Pos2::new(h.state().central.max.x, 400.0);
    assert!(
        (edge.x - (SCREEN.x - 280.0)).abs() <= 1.0,
        "default width 280: {edge:?}"
    );
    // Halfway through the drag nothing is reported.
    let end = edge - Vec2::new(60.0, 0.0);
    press_and_move(&mut h, edge, end);
    assert_eq!(h.state_mut().take(), []);
    assert!(
        (h.state().central.max.x - end.x).abs() <= 1.0,
        "the dock follows the pointer"
    );
    assert_eq!(
        release(&mut h, end),
        [UiAction::SetDock(DockLayout {
            width: 340.0,
            ..Default::default()
        })]
    );
    // Far past the range: the widest width.
    let mut h = harness(Fixture::default());
    let edge = Pos2::new(h.state().central.max.x, 400.0);
    assert_eq!(
        drag(&mut h, edge, Vec2::new(-600.0, 0.0)),
        [UiAction::SetDock(DockLayout {
            width: 420.0,
            ..Default::default()
        })]
    );
    // A click in the 3D view reports nothing.
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_at(&mut h, Pos2::new(500.0, 400.0), egui::Modifiers::NONE),
        []
    );
}

#[test]
fn the_dock_opens_at_its_saved_width_and_tab() {
    let h = harness(Fixture {
        dock: DockLayout {
            width: 400.0,
            tab: DockTab::Materials,
            ..Default::default()
        },
        ..Default::default()
    });
    assert!((h.state().central.max.x - (SCREEN.x - 400.0)).abs() <= 1.0);
    assert_eq!(is_on(&h, "Materials"), Some(true));
    assert_eq!(is_on(&h, "Entity Info"), Some(false));
}

#[test]
fn clicking_a_tab_reports_it() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Materials"),
        [UiAction::SetDock(DockLayout {
            tab: DockTab::Materials,
            ..Default::default()
        })]
    );
    assert_eq!(click(&mut h, "Entity Info"), [], "already open");
}

#[test]
fn dragging_the_divider_reports_the_split_on_release() {
    let mut h = harness(Fixture::default());
    let divider = rect_of(&h, "Resize Outliner").center();
    let end = divider + Vec2::new(0.0, 50.0);
    press_and_move(&mut h, divider, end);
    assert_eq!(h.state_mut().take(), [], "nothing reported mid-drag");
    let live = rect_of(&h, "Resize Outliner").center().y;
    assert!(
        (live - end.y).abs() <= 1.0,
        "the divider follows the pointer: {live} for {}",
        end.y
    );
    let actions = release(&mut h, end);
    let [UiAction::SetDock(d)] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    assert!(d.split > 0.55 && d.split <= 0.8, "{d:?}");
    assert_eq!(
        (d.width, d.tab),
        (280.0, DockTab::EntityInfo),
        "only the split"
    );
    assert_eq!((d.split * 100.0).round() / 100.0, d.split, "in hundredths");
}

#[test]
fn both_areas_keep_their_least_height() {
    assert_eq!(top_height(0.55, 1000.0), 550.0);
    assert_eq!(top_height(0.2, 500.0), AREA_MIN);
    assert_eq!(top_height(0.8, 500.0), 500.0 - AREA_MIN);
    assert_eq!(
        top_height(0.5, 200.0),
        100.0,
        "too short for both: shared by split"
    );
}
