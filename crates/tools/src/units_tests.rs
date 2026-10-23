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

fn locked_hit(p: DVec3) -> crate::inference::Inference {
    crate::inference::Inference {
        point: p,
        kind: SnapKind::Axis(crate::inference::Axis::X),
        face: None,
        locked: Some(SnapKind::Endpoint),
    }
}

fn move_to(r: &mut Rig, hit: crate::inference::Inference) {
    r.cursor = r.camera.project(hit.point).unwrap();
    r.send(ToolEvent::Move(hit));
}

#[test]
fn a_point_snap_under_a_lock_keeps_its_exact_position() {
    // Through inference: an endpoint at x = 1.13 with the direction locked along X.
    let mut model = Model::new();
    model
        .add_edge(DVec3::new(1.13, 2.0, 0.0), DVec3::new(1.13, 3.0, 0.0))
        .unwrap();
    let mut r = Rig::new(ToolKind::Line, model);
    r.units = snapping(0.25);
    r.click(DVec3::ZERO);
    let guides = crate::inference::Guides {
        anchor: Some(DVec3::ZERO),
        reference: None,
        lock: Some(crate::inference::Lock {
            dir: DVec3::X,
            kind: SnapKind::Axis(crate::inference::Axis::X),
        }),
    };
    let cursor = r.camera.project(DVec3::new(1.13, 2.0, 0.0)).unwrap();
    let hit = crate::inference::infer_detail(
        &crate::scenery::Scenery::single(&r.model),
        &r.camera,
        cursor,
        &guides,
        10.0,
    )
    .0;
    assert_eq!(hit.locked, Some(SnapKind::Endpoint), "{hit:?}");
    r.cursor = cursor;
    let out = r.send(ToolEvent::Click(hit));
    assert!((edge_end(&out).x - 1.13).abs() < 1e-9, "{out:?}");
}

#[test]
fn locked_point_snaps_are_exact_in_every_tool() {
    let p = DVec3::new(1.13, 0.0, 0.0);
    for kind in [
        ToolKind::Line,
        ToolKind::TapeMeasure,
        ToolKind::Arc,
        ToolKind::Circle,
        ToolKind::Polygon,
        ToolKind::RotatedRectangle,
    ] {
        let mut r = rig(kind, snapping(0.25));
        r.click(DVec3::ZERO);
        move_to(&mut r, locked_hit(p));
        assert_eq!(measured(&r), "1.13 m", "{kind:?}");
    }
    let mut r = rig(ToolKind::Rectangle, snapping(0.25));
    r.click(DVec3::ZERO);
    move_to(&mut r, locked_hit(DVec3::new(1.13, 0.62, 0.0)));
    assert_eq!(measured(&r), "1.13 m, 0.62 m");

    let mut model = Model::new();
    model
        .add_edge(DVec3::new(0.0, 5.0, 0.0), DVec3::new(1.0, 5.0, 0.0))
        .unwrap();
    let edge = model.edges().next().unwrap().0;
    let mut r = Rig::new(ToolKind::Move, model);
    r.units = snapping(0.25);
    r.select(kernel::EntityId::Edge(edge));
    r.click(DVec3::ZERO);
    move_to(&mut r, locked_hit(p));
    assert_eq!(measured(&r), "1.13 m");
}

#[test]
fn a_rounded_point_never_leaves_the_ground_it_was_taken_on() {
    // From above the ground the rounded point would sink below it: not rounded.
    let mut r = rig(ToolKind::Line, snapping(0.25));
    r.click(DVec3::new(0.0, 0.0, 3.0));
    let out = r.click(DVec3::new(1.13, 0.0, 0.0));
    assert_eq!(edge_end(&out), DVec3::new(1.13, 0.0, 0.0));
    // From the ground it is rounded.
    let mut r = rig(ToolKind::Line, snapping(0.25));
    r.click(DVec3::new(0.0, 1.0, 0.0));
    let out = r.click(DVec3::new(1.13, 1.0, 0.0));
    assert!((edge_end(&out).x - 1.25).abs() < 1e-12);

    let mut r = rig(ToolKind::TapeMeasure, snapping(0.25));
    r.click(DVec3::new(0.0, 0.0, 3.0));
    r.mv(DVec3::new(1.13, 0.0, 0.0));
    assert_eq!(measured(&r), "3.206 m");
}

#[test]
fn a_rotated_rectangles_side_and_width_round() {
    let mut r = rig(ToolKind::RotatedRectangle, snapping(0.25));
    r.click(DVec3::ZERO);
    r.mv(DVec3::new(1.13, 0.0, 0.0));
    assert_eq!(measured(&r), "1.25 m");
    r.click(DVec3::new(1.13, 0.0, 0.0));
    r.mv(DVec3::new(0.3, 0.62, 0.0));
    assert_eq!(measured(&r), "0.5 m");
    let out = r.click(DVec3::new(0.3, 0.62, 0.0));
    let Command::AddRotatedRectangle {
        edge_end,
        width_point,
        ..
    } = only(&out)
    else {
        panic!("{out:?}")
    };
    assert!((edge_end[0] - 1.25).abs() < 1e-12, "{edge_end:?}");
    assert!((width_point[1] - 0.5).abs() < 1e-12, "{width_point:?}");
}

#[test]
fn the_arcs_bulge_rounds() {
    let mut r = rig(ToolKind::Arc, snapping(0.25));
    r.click(DVec3::ZERO);
    r.click(DVec3::new(2.0, 0.0, 0.0));
    r.mv(DVec3::new(1.0, 0.62, 0.0));
    assert_eq!(measured(&r), "0.5 m");
    let hit = r.at(DVec3::new(1.0, 0.62, 0.0), SnapKind::Endpoint, None);
    r.send(ToolEvent::Move(hit));
    assert_eq!(measured(&r), "0.62 m");
}

#[test]
fn the_tape_rounds_from_an_edge() {
    let mut model = Model::new();
    model
        .add_edge(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0))
        .unwrap();
    let mut r = Rig::new(ToolKind::TapeMeasure, model);
    r.units = snapping(0.25);
    let on_edge = r.at(DVec3::new(2.0, 0.0, 0.0), SnapKind::OnEdge, None);
    r.send(ToolEvent::Click(on_edge));
    r.mv(DVec3::new(2.0, 0.62, 0.0));
    assert_eq!(measured(&r), "0.5 m", "the distance from the edge");
}

#[test]
fn rotates_angle_follows_the_step_and_zero_is_free() {
    let mut model = Model::new();
    model
        .add_edge(DVec3::new(0.0, 5.0, 0.0), DVec3::new(1.0, 5.0, 0.0))
        .unwrap();
    let edge = model.edges().next().unwrap().0;
    let angle_at = |snap: f64, deg: f64| {
        let mut r = Rig::new(ToolKind::Rotate, model.clone());
        r.units.angle_snap = snap;
        r.select(kernel::EntityId::Edge(edge));
        r.click(DVec3::ZERO);
        r.click(DVec3::new(1.0, 0.0, 0.0));
        r.mv(polar(deg));
        measured(&r)
    };
    assert_eq!(angle_at(15.0, 31.0), "30°");
    assert_eq!(angle_at(5.0, 36.5), "35°");
    assert_eq!(angle_at(1.0, 31.3), "31°");
    assert_eq!(angle_at(0.0, 31.3), "31.3°");
}

#[test]
fn a_step_that_does_not_divide_180_stays_in_range() {
    let a = protractor_angle(DVec3::X, polar(179.9), DVec3::Z, false, 3.7);
    assert!((a - (-178.7)).abs() < 1e-9, "{a}");
    let a = protractor_angle(DVec3::X, polar(-179.9), DVec3::Z, false, 3.7);
    assert!((a - 178.7).abs() < 1e-9, "{a}");
}

#[test]
fn rectangles_round_along_moved_drawing_axes() {
    let mut r = rig(ToolKind::Rectangle, snapping(0.25));
    r.axes = glam::DAffine3::from_rotation_z(30f64.to_radians());
    let (u, v) = (
        glam::DMat3::from_rotation_z(30f64.to_radians()) * DVec3::X,
        glam::DMat3::from_rotation_z(30f64.to_radians()) * DVec3::Y,
    );
    r.click(DVec3::ZERO);
    r.mv(u * 1.13 + v * 0.62);
    assert_eq!(measured(&r), "1.25 m, 0.5 m");
}
