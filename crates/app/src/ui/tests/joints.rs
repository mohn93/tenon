use kernel::{EntityId, InstanceId, JointId, JointKind};
use session::JointInfo;

use super::*;
use crate::ui::JointRow;

const HINGE: JointId = JointId(1);
const SLIDER: JointId = JointId(2);

fn info(id: JointId, name: &str, kind: JointKind, value: f64) -> JointRow {
    JointRow {
        info: JointInfo {
            id,
            name: name.into(),
            kind,
            a: Some(EntityId::Instance(InstanceId(1))),
            b: EntityId::Instance(InstanceId(2)),
            value,
            slide: None,
            limits: None,
            point: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
        },
        selected: false,
        follows: false,
    }
}

/// A hinge at 30 degrees limited to -90 to 90, and an unlimited slider at 0.5 m.
fn joints_tab() -> Fixture {
    let mut hinge = info(HINGE, "Hinge", JointKind::Revolute, 30.0);
    hinge.info.limits = Some([-90.0, 90.0]);
    Fixture {
        dock: DockLayout {
            tab: DockTab::Joints,
            ..Default::default()
        },
        joints: vec![hinge, info(SLIDER, "Rail", JointKind::Prismatic, 0.5)],
        ..Default::default()
    }
}

fn tall(f: Fixture) -> H {
    harness_sized(f, Vec2::new(1280.0, 1600.0))
}

#[test]
fn the_tab_is_in_the_dock() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Joints"),
        [UiAction::SetDock(DockLayout {
            tab: DockTab::Joints,
            ..Default::default()
        })]
    );
    let h = tall(Fixture {
        dock: DockLayout {
            tab: DockTab::Joints,
            ..Default::default()
        },
        ..Default::default()
    });
    h.get_by_label("No joints");
}

#[test]
fn the_panel_lists_joints_with_kind_value_and_limits() {
    let h = tall(joints_tab());
    h.get_by_label("Hinge");
    h.get_by_label("Rail");
    h.get_by_label("Revolute 30.0\u{b0}");
    h.get_by_label("Prismatic 0.5 m");
    h.get_by_label("Limits -90.0\u{b0} to 90.0\u{b0}");
    h.get_by_label("Hinge angle value");
    h.get_by_label("Rail distance value");
    h.get_by_label("Delete Hinge");
}

#[test]
fn clicking_a_row_selects_the_joints_parts() {
    let mut h = tall(joints_tab());
    assert_eq!(click(&mut h, "Rail"), [UiAction::SelectJoint(SLIDER)]);
    let mut f = joints_tab();
    f.joints[0].selected = true;
    let h = tall(f);
    assert_eq!(is_on(&h, "Hinge"), Some(true));
    assert_eq!(is_on(&h, "Rail"), Some(false));
}

#[test]
fn delete_removes_the_joint() {
    let mut h = tall(joints_tab());
    assert_eq!(
        click(&mut h, "Delete Rail"),
        [UiAction::DeleteJoint(SLIDER)]
    );
}

#[test]
fn play_plays_the_joint() {
    let mut h = tall(joints_tab());
    assert_eq!(click(&mut h, "Play Rail"), [UiAction::PlayJoint(SLIDER)]);
    assert_eq!(click(&mut h, "Play Hinge"), [UiAction::PlayJoint(HINGE)]);
}

#[test]
fn a_joint_that_follows_a_cam_has_no_play_button_or_slider() {
    let mut f = joints_tab();
    f.joints[1].follows = true;
    let h = tall(f);
    assert!(h.query_by_label("Play Rail").is_none());
    assert!(h.query_by_label("Rail distance value").is_none());
    h.get_by_label("Follows its cam; drive the cam's joints");
    h.get_by_label("Hinge angle value");
    h.get_by_label("Play Hinge");
    h.get_by_label("Delete Rail");
}

#[test]
fn a_drag_drives_live_and_reports_its_end_once() {
    let mut h = tall(joints_tab());
    let slider = rect_of(&h, "Hinge angle");
    let width = h
        .ctx
        .global_style()
        .spacing
        .slider_width
        .min(slider.width());
    let start = Pos2::new(slider.min.x + 0.2 * width, slider.center().y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(start));
    h.event(button(start, true));
    h.run_ok();
    for k in 1..=10 {
        h.event(egui::Event::PointerMoved(
            start + Vec2::new(3.0 * k as f32, 0.0),
        ));
        h.run_ok();
    }
    let live = h.state_mut().take();
    assert!(live.len() >= 2, "a drive per new value: {live:?}");
    assert!(
        live.iter().all(|a| matches!(
            a,
            UiAction::DriveJoint {
                joint: HINGE,
                slide: None,
                done: false,
                ..
            }
        )),
        "{live:?}"
    );
    h.event(button(start + Vec2::new(30.0, 0.0), false));
    h.run_ok();
    let end = h.state_mut().take();
    assert!(
        matches!(
            end.as_slice(),
            [UiAction::DriveJoint {
                joint: HINGE,
                done: true,
                ..
            }]
        ),
        "{end:?}"
    );
}

#[test]
fn a_value_off_the_slider_step_is_not_driven_without_input() {
    // A solved joint sits a hair off zero or between steps: the slider's rounding to its step
    // is not an edit, so nothing is driven while nobody touches it.
    let mut f = joints_tab();
    f.joints[0].info.value = 3.0e-12;
    f.joints[1].info.value = 0.0123;
    let mut h = tall(f);
    for _ in 0..3 {
        h.run_ok();
    }
    let actions = h.state_mut().take();
    assert!(actions.is_empty(), "{actions:?}");
}

#[test]
fn a_typed_value_drives_the_joint_once() {
    let mut h = tall(joints_tab());
    let actions = type_into(&mut h, "Hinge angle value", 0, "45", egui::Key::Enter);
    assert_eq!(
        actions,
        [UiAction::DriveJoint {
            joint: HINGE,
            value: 45.0,
            slide: None,
            done: true
        }]
    );
}

/// The Joints tab of a rig as the V6 engine's: a long name, a prismatic joint a hair off its
/// slider's step, a cylindrical joint with its slide and a limited hinge.
fn crowded_joints(width: f32) -> Fixture {
    let mut crank = info(JointId(1), "Crank", JointKind::Revolute, -0.0);
    crank.selected = true;
    let mut rod = info(JointId(2), "Rod 1 big end", JointKind::Revolute, -179.95);
    rod.info.limits = Some([-180.0, 180.0]);
    let piston = info(JointId(3), "Piston 1", JointKind::Prismatic, -0.023);
    let mut long = info(
        JointId(4),
        "A joint whose name is far longer than the dock is wide",
        JointKind::Cylindrical,
        -12.5,
    );
    long.info.slide = Some(-1.234);
    let mut fixed = info(
        JointId(5),
        "Fixed bracket with a long name",
        JointKind::Fixed,
        0.0,
    );
    fixed.info.limits = None;
    Fixture {
        dock: DockLayout {
            width,
            tab: DockTab::Joints,
            ..Default::default()
        },
        joints: vec![crank, rod, piston, long, fixed],
        ..Default::default()
    }
}

#[test]
fn every_joint_row_fits_the_narrowest_dock() {
    let width = *crate::ui::dock::DOCK_WIDTH.start();
    for screen in [Vec2::new(1280.0, 1600.0), Vec2::new(1700.0, 1600.0)] {
        let mut h = harness_sized(crowded_joints(width), screen);
        for _ in 0..3 {
            h.run_ok();
        }
        let dock = h.state().central.max.x;
        assert!(
            (dock - (screen.x - width)).abs() <= 1.0,
            "the dock keeps its width: starts at {dock} in a {screen:?} window"
        );
        h.get_by_label("Delete Piston 1");
        h.get_by_label("Piston 1 distance value");
        let root = h.root().accesskit_node().id();
        let outside: Vec<(String, Rect)> = h
            .query_all(egui_kittest::kittest::by().predicate(|_| true))
            .filter(|n| {
                n.accesskit_node().id() != root && n.accesskit_node().data().bounds().is_some()
            })
            .filter(|n| n.accesskit_node().role() != egui::accesskit::Role::Splitter)
            .filter(|n| n.rect().max.x > dock + 1.0)
            .filter(|n| n.rect().min.x < dock - 0.5 || n.rect().max.x > screen.x + 0.5)
            .map(|n| (format!("{:?}", n.accesskit_node().label()), n.rect()))
            .collect();
        assert!(outside.is_empty(), "past the dock at {dock}: {outside:?}");
        let texts: Vec<(String, Rect)> = drawn_text_rects(&h)
            .into_iter()
            .filter(|(_, r)| r.max.x > dock + 1.0)
            .filter(|(_, r)| r.min.x < dock - 0.5 || r.max.x > screen.x + 0.5)
            .collect();
        assert!(texts.is_empty(), "text past the dock at {dock}: {texts:?}");
    }
}

#[test]
fn a_value_too_long_for_the_dock_does_not_widen_it() {
    let width = *crate::ui::dock::DOCK_WIDTH.start();
    let mut f = crowded_joints(width);
    f.joints[2].info.value = 1.0e40;
    let mut h = harness(f);
    for _ in 0..3 {
        h.run_ok();
    }
    let dock = h.state().central.max.x;
    assert!(
        (dock - (SCREEN.x - width)).abs() <= 1.0,
        "the dock keeps its width: starts at {dock}"
    );
}

#[test]
fn a_value_at_zero_shows_no_sign() {
    let mut f = joints_tab();
    f.joints[0].info.value = -0.0;
    f.joints[0].info.limits = None;
    f.joints[1].info.value = -0.0001;
    let mut h = tall(f);
    h.get_by_label("Revolute 0.0\u{b0}");
    h.get_by_label("Prismatic 0 m");
    let value = |h: &H, label: &str| h.get_by_label(label).accesskit_node().value();
    assert_eq!(value(&h, "Hinge angle value").as_deref(), Some("0.0\u{b0}"));
    assert_eq!(value(&h, "Rail distance value").as_deref(), Some("0 m"));
    // A value that rounds to zero.
    h.state_mut().joints[0].info.value = -0.04;
    h.run_ok();
    h.get_by_label("Revolute 0.0\u{b0}");
    assert_eq!(value(&h, "Hinge angle value").as_deref(), Some("0.0\u{b0}"));
}
