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
