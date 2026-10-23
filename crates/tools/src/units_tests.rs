//! The tools follow the model's display units: typed lengths, shown lengths and angles, and
//! the length and angle snapping of cursor-driven values.

use glam::DVec3;
use kernel::Model;
use session::{Command, LengthUnit, Units};

use crate::inference::SnapKind;
use crate::rotate::protractor_angle;
use crate::testing::{Rig, only};
use crate::tool::{ToolEvent, ToolKind};

const INCH: f64 = 0.0254;

fn rig(kind: ToolKind, units: Units) -> Rig {
    let mut r = Rig::new(kind, Model::new());
    r.units = units;
    r
}

fn feet_inches() -> Units {
    Units {
        length: LengthUnit::FeetInches,
        ..Units::default()
    }
}

fn snapping(step: f64) -> Units {
    Units {
        length_snap: step,
        ..Units::default()
    }
}

fn measured(r: &Rig) -> String {
    r.tool
        .measurement(&r.units)
        .1
        .unwrap_or_else(|| "nothing".into())
}

fn edge_end(out: &crate::tool::ToolOutput) -> DVec3 {
    match only(out) {
        Command::AddEdge { end, .. } => DVec3::from_array(*end),
        other => panic!("not an edge: {other:?}"),
    }
}

fn polar(deg: f64) -> DVec3 {
    DVec3::new(deg.to_radians().cos(), deg.to_radians().sin(), 0.0)
}

#[test]
fn lengths_are_shown_in_the_model_unit() {
    let cm = Units {
        length: LengthUnit::Centimeters,
        precision: 1,
        ..Units::default()
    };
    let mut r = rig(ToolKind::Line, cm);
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(2.0, 0.0, 0.0));
    assert_eq!(measured(&r), "200 cm");

    let mut r = rig(ToolKind::Line, feet_inches());
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(2.0, 0.0, 0.0));
    assert_eq!(measured(&r), "6' 6 3/4\"");
}

#[test]
fn typed_feet_and_inches_are_read_and_not_rounded() {
    let mut r = rig(
        ToolKind::Line,
        Units {
            length_snap: 1.0,
            ..feet_inches()
        },
    );
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.0, 0.0, 0.0));
    let out = r.value("3'6 1/2\"");
    assert!((edge_end(&out).x - 42.5 * INCH).abs() < 1e-12, "{out:?}");

    // A bare number is in the model's unit: inches here.
    let mut r = rig(ToolKind::Line, feet_inches());
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.0, 0.0, 0.0));
    let out = r.value("10");
    assert!((edge_end(&out).x - 10.0 * INCH).abs() < 1e-12);

    // An explicit suffix still wins.
    let mut r = rig(ToolKind::Line, feet_inches());
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.0, 0.0, 0.0));
    let out = r.value("2m");
    assert!((edge_end(&out).x - 2.0).abs() < 1e-12);

    // Bad text is refused with a message.
    let out = r.value("1/");
    assert!(out.commands.is_empty());
    assert_eq!(out.message.as_deref(), Some("Not a length: 1/"));
}

#[test]
fn the_rectangle_reads_feet_inches_pairs_split_on_the_comma() {
    let mut r = rig(ToolKind::Rectangle, feet_inches());
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.0, 1.0, 0.0));
    let out = r.value("3'6 1/2\", 4'");
    let Command::AddRectangle { opposite, .. } = only(&out) else {
        panic!("{out:?}")
    };
    assert!((opposite[0] - 42.5 * INCH).abs() < 1e-12, "{opposite:?}");
    assert!((opposite[1] - 48.0 * INCH).abs() < 1e-12, "{opposite:?}");
}

#[test]
fn length_snapping_rounds_the_cursors_line_but_not_a_point_snap_or_typed_values() {
    let mut r = rig(ToolKind::Line, snapping(0.25));
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.13, 0.0, 0.0));
    assert_eq!(measured(&r), "1.25 m");
    let out = r.click(DVec3::new(1.13, 0.0, 0.0));
    assert!((edge_end(&out).x - 1.25).abs() < 1e-12);

    // An endpoint keeps its exact position.
    let mut r = rig(ToolKind::Line, snapping(0.25));
    r.click(DVec3::ZERO);
    let hit = r.at(DVec3::new(1.13, 0.0, 0.0), SnapKind::Endpoint, None);
    let out = r.send(ToolEvent::Click(hit));
    assert!((edge_end(&out).x - 1.13).abs() < 1e-12);

    // Typed values are exact.
    let mut r = rig(ToolKind::Line, snapping(0.25));
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.0, 0.0, 0.0));
    let out = r.value("1.13");
    assert!((edge_end(&out).x - 1.13).abs() < 1e-12);

    // Off: nothing is rounded.
    let mut r = rig(ToolKind::Line, Units::default());
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.13, 0.0, 0.0));
    assert_eq!(measured(&r), "1.13 m");
}

#[test]
fn length_snapping_rounds_each_side_of_a_rectangle() {
    let mut r = rig(ToolKind::Rectangle, snapping(0.25));
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.13, 0.62, 0.0));
    assert_eq!(measured(&r), "1.25 m, 0.5 m");
    let hit = r.at(DVec3::new(1.13, 0.62, 0.0), SnapKind::Midpoint, None);
    r.send(ToolEvent::Move(hit));
    assert_eq!(measured(&r), "1.13 m, 0.62 m");
}

#[test]
fn length_snapping_rounds_a_radius() {
    for kind in [ToolKind::Circle, ToolKind::Polygon] {
        let mut r = rig(kind, snapping(0.25));
        r.click(DVec3::ZERO);
        r.mv(DVec3::new(1.13, 0.0, 0.0));
        assert_eq!(measured(&r), "1.25 m", "{kind:?}");
        let hit = r.at(DVec3::new(1.13, 0.0, 0.0), SnapKind::Endpoint, None);
        r.send(ToolEvent::Move(hit));
        assert_eq!(measured(&r), "1.13 m", "{kind:?}");
    }
}

#[test]
fn length_snapping_rounds_the_tape_and_the_arc() {
    for kind in [ToolKind::TapeMeasure, ToolKind::Arc] {
        let mut r = rig(kind, snapping(0.25));
        r.click(DVec3::ZERO);
        r.mv(DVec3::new(1.13, 0.0, 0.0));
        assert_eq!(measured(&r), "1.25 m", "{kind:?}");
    }
}

#[test]
fn the_tape_shows_the_model_unit() {
    let mut r = rig(
        ToolKind::TapeMeasure,
        Units {
            length: LengthUnit::Millimeters,
            ..Units::default()
        },
    );
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.5, 0.0, 0.0));
    assert_eq!(measured(&r), "1500 mm");
}

#[test]
fn angle_snapping_sets_the_protractors_steps() {
    let angle_at = |units: Units, deg: f64| {
        let mut r = rig(ToolKind::Protractor, units);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(1.0, 0.0, 0.0));
        r.mv(polar(deg));
        measured(&r)
    };
    let snap = |angle_snap| Units {
        angle_snap,
        ..Units::default()
    };
    assert_eq!(angle_at(snap(5.0), 36.5), "35°");
    assert_eq!(angle_at(snap(15.0), 31.0), "30°");
    // 0 is free.
    assert_eq!(angle_at(snap(0.0), 31.0), "31°");
    assert_eq!(angle_at(snap(0.0), 36.5), "36.5°");
    // The precision decides the decimals shown.
    let fine = Units {
        angle_snap: 0.0,
        angle_precision: 2,
        ..Units::default()
    };
    assert_eq!(angle_at(fine, 31.25), "31.25°");
}

#[test]
fn a_step_finer_than_the_capture_range_snaps_everywhere() {
    let (from, to, axis) = (DVec3::X, polar(31.3), DVec3::Z);
    let a = protractor_angle(from, to, axis, false, 1.0);
    assert!((a - 31.0).abs() < 1e-9, "{a}");
    let a = protractor_angle(from, to, axis, false, 0.0);
    assert!((a - 31.3).abs() < 1e-9, "{a}");
    let a = protractor_angle(from, to, axis, true, 15.0);
    assert!((a - 31.3).abs() < 1e-9, "{a}");
}

#[test]
fn the_protractor_reads_a_slope_in_the_model_unit() {
    let mut r = rig(ToolKind::Protractor, feet_inches());
    r.click(DVec3::ZERO);
    r.click(DVec3::new(1.0, 0.0, 0.0));
    r.mv(polar(10.0));
    // 6:12 is the same slope whether a bare number is inches or meters; 1':1' too.
    let out = r.value("1':1'");
    assert!(
        matches!(only(&out), Command::AddGuideLine { .. }),
        "{out:?}"
    );
}

#[test]
fn length_snapping_rounds_a_move_along_the_inferred_direction() {
    let mut model = Model::new();
    model
        .add_edge(DVec3::new(0.0, 5.0, 0.0), DVec3::new(1.0, 5.0, 0.0))
        .unwrap();
    let edge = model.edges().next().unwrap().0;
    let mut r = Rig::new(ToolKind::Move, model);
    r.units = snapping(0.25);
    r.select(kernel::EntityId::Edge(edge));
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.13, 0.0, 0.0));
    assert_eq!(measured(&r), "1.25 m");
    let hit = r.at(DVec3::new(1.13, 0.0, 0.0), SnapKind::Endpoint, None);
    r.send(ToolEvent::Move(hit));
    assert_eq!(measured(&r), "1.13 m");
}
