//! The Freehand tool: press, drag, release. The path lies in the plane of the face under the
//! press, or in the horizontal plane (the drawing axes' red-green plane) through it, follows
//! the pointer, and is simplified (Douglas-Peucker, 2 px at the press point's depth) into one
//! curve. Releasing within the snap radius of the start closes it; Shift held at the press
//! draws a polyline of plain edges instead of a curve.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::rectangle::{on_plane, plane_normal};
use crate::simplify::douglas_peucker;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::format_length;

/// How far the simplified path may stray from the pointer's, in pixels.
const TOLERANCE_PX: f64 = 2.0;
/// The most pointer samples one stroke keeps (a command takes at most as many points); later
/// ones are dropped.
const MAX_SAMPLES: usize = 10_000;
/// The most segments of the stroke the preview draws (longer strokes are thinned).
const PREVIEW_SEGMENTS: usize = 1_000;

/// A stroke in progress.
#[derive(Clone, Debug)]
struct Stroke {
    /// The press point and the normal of the plane the path lies in.
    origin: DVec3,
    normal: DVec3,
    /// The simplification tolerance, in model units.
    tolerance: f64,
    /// Whether the edges form a curve (no Shift at the press).
    curve: bool,
    samples: Vec<DVec3>,
}

impl Stroke {
    fn length(&self) -> f64 {
        self.samples.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    fn simplified(&self, path: &[DVec3]) -> Vec<DVec3> {
        douglas_peucker(path, self.tolerance)
    }

    /// The command for the stroke released with the cursor at `end` on screen, if it drew
    /// anything: closed when that is within `radius` pixels of the start.
    fn command(&self, ctx: &ToolContext) -> Option<Command> {
        let first = self.samples[0];
        let near_start = ctx
            .camera
            .project(first)
            .is_some_and(|px| px.distance(ctx.cursor) <= ctx.pick_radius);
        let command = |points: Vec<DVec3>, closed| Command::AddCurve {
            points: points.iter().map(|p| p.to_array()).collect(),
            closed,
            curve: Some(self.curve),
        };
        if near_start && self.samples.len() >= 3 {
            let mut ring = self.samples.clone();
            *ring.last_mut().unwrap() = first;
            let mut points = self.simplified(&ring);
            points.pop();
            if points.len() >= 3 {
                return Some(command(points, true));
            }
        }
        let points = self.simplified(&self.samples);
        let last = *points.last()?;
        (points.len() >= 2 && points[0].distance(last) >= LENGTH_TOL)
            .then(|| command(points, false))
    }

    /// The stroke as preview segments, thinned to `PREVIEW_SEGMENTS`.
    fn preview(&self) -> Vec<[DVec3; 2]> {
        let step = self.samples.len().div_ceil(PREVIEW_SEGMENTS + 1).max(1);
        let mut points: Vec<DVec3> = self.samples.iter().step_by(step).copied().collect();
        let last = *self.samples.last().unwrap();
        if points.last() != Some(&last) {
            points.push(last);
        }
        points.windows(2).map(|w| [w[0], w[1]]).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FreehandTool {
    stroke: Option<Stroke>,
}

impl FreehandTool {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Tool for FreehandTool {
    fn name(&self) -> &'static str {
        "Freehand"
    }

    fn prompt(&self) -> &'static str {
        match self.stroke {
            None => "Press and drag to draw a curve (Shift at the press: a polyline)",
            Some(_) => "Release to finish; near the start it closes",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        (
            "Length",
            self.stroke.as_ref().map(|s| format_length(s.length())),
        )
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn phase(&self) -> Phase {
        match &self.stroke {
            Some(s) => Phase::Active {
                points: vec![s.origin],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Click(hit) => {
                self.stroke = Some(Stroke {
                    origin: hit.point,
                    normal: plane_normal(&hit, ctx),
                    tolerance: TOLERANCE_PX * ctx.camera.pixel_size(hit.point),
                    curve: !ctx.shift,
                    samples: vec![hit.point],
                });
            }
            ToolEvent::Move(hit) => {
                if let Some(s) = &mut self.stroke
                    && s.samples.len() < MAX_SAMPLES
                    && let Some(p) = on_plane(s.origin, s.normal, &hit, ctx)
                    && s.samples.last().is_some_and(|q| q.distance(p) > 1e-9)
                {
                    s.samples.push(p);
                }
            }
            ToolEvent::Release(hit) => {
                if let Some(mut s) = self.stroke.take() {
                    if let Some(p) = on_plane(s.origin, s.normal, &hit, ctx)
                        && s.samples.len() < MAX_SAMPLES
                        && s.samples.last().is_some_and(|q| q.distance(p) > 1e-9)
                    {
                        s.samples.push(p);
                    }
                    out.commands.extend(s.command(ctx));
                }
            }
            ToolEvent::Value(_) => out.message = Some("Freehand takes no typed value".into()),
            ToolEvent::Cancel => self.on_external_change(),
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
        }
        if let Some(s) = &self.stroke {
            out.preview = s.preview();
        }
        out
    }

    fn on_external_change(&mut self) {
        self.stroke = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
    use crate::testing::{Rig, only};
    use crate::tool::ToolKind;
    use glam::{DAffine3, DMat3};
    use kernel::Model;
    use std::time::Instant;

    fn p(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    fn rig() -> Rig {
        Rig::new(ToolKind::Freehand, Model::new())
    }

    /// Presses at the first point, moves through the rest and releases at the last.
    fn stroke(r: &mut Rig, path: &[DVec3]) -> ToolOutput {
        r.click(path[0]);
        for q in &path[1..] {
            r.mv(*q);
        }
        r.release(*path.last().unwrap())
    }

    /// Points from `a` to `b`, `n` steps (both included).
    fn line(a: DVec3, b: DVec3, n: usize) -> Vec<DVec3> {
        (0..=n).map(|i| a.lerp(b, i as f64 / n as f64)).collect()
    }

    fn curve(out: &ToolOutput) -> (Vec<DVec3>, bool, Option<bool>) {
        match only(out) {
            Command::AddCurve {
                points,
                closed,
                curve,
            } => (
                points.iter().map(|q| DVec3::from_array(*q)).collect(),
                *closed,
                *curve,
            ),
            other => panic!("not a curve: {other:?}"),
        }
    }

    #[test]
    fn a_drag_makes_one_simplified_curve() {
        let mut r = rig();
        let mut path = line(p(0.0, 0.0), p(2.0, 0.0), 40);
        path.extend(line(p(2.0, 0.0), p(2.0, 2.0), 40));
        let out = stroke(&mut r, &path);
        let (points, closed, curve) = curve(&out);
        assert_eq!(points, vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0)]);
        assert!(!closed);
        assert_eq!(curve, Some(true));
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn the_preview_follows_the_pointer() {
        let mut r = rig();
        r.click(p(0.0, 0.0));
        r.mv(p(1.0, 0.0));
        let out = r.mv(p(1.0, 1.0));
        assert_eq!(out.preview.len(), 2);
        assert!(matches!(r.tool.phase(), Phase::Active { .. }));
    }

    #[test]
    fn shift_at_the_press_makes_a_polyline() {
        let mut r = rig();
        r.shift = true;
        r.click(p(0.0, 0.0));
        r.shift = false;
        r.mv(p(1.0, 0.0));
        let out = r.release(p(1.0, 1.0));
        assert_eq!(curve(&out).2, Some(false));
        // Shift only counts at the press.
        r.click(p(0.0, 0.0));
        r.shift = true;
        r.mv(p(1.0, 0.0));
        let out = r.release(p(1.0, 1.0));
        assert_eq!(curve(&out).2, Some(true));
    }

    #[test]
    fn releasing_near_the_start_closes_the_loop() {
        let mut r = rig();
        let mut path = line(p(0.0, 0.0), p(2.0, 0.0), 20);
        path.extend(line(p(2.0, 0.0), p(2.0, 2.0), 20));
        path.extend(line(p(2.0, 2.0), p(0.0, 2.0), 20));
        path.extend(line(p(0.0, 2.0), p(0.0, 0.01), 20));
        let out = stroke(&mut r, &path);
        let (points, closed, _) = curve(&out);
        assert!(closed);
        assert_eq!(
            points,
            vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)]
        );
    }

    #[test]
    fn releasing_far_from_the_start_stays_open() {
        let mut r = rig();
        let mut path = line(p(0.0, 0.0), p(2.0, 0.0), 20);
        path.extend(line(p(2.0, 0.0), p(2.0, 2.0), 20));
        path.extend(line(p(2.0, 2.0), p(0.0, 2.0), 20));
        path.extend(line(p(0.0, 2.0), p(0.0, 0.5), 20));
        let (points, closed, _) = curve(&stroke(&mut r, &path));
        assert!(!closed);
        assert_eq!(points.len(), 5);
    }

    #[test]
    fn a_click_without_a_drag_draws_nothing() {
        let mut r = rig();
        r.click(p(1.0, 1.0));
        let out = r.release(p(1.0, 1.0));
        assert!(out.commands.is_empty());
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn escape_cancels_the_stroke() {
        let mut r = rig();
        r.click(p(0.0, 0.0));
        r.mv(p(1.0, 0.0));
        let out = r.send(ToolEvent::Cancel);
        assert!(out.commands.is_empty() && out.preview.is_empty());
        assert_eq!(r.tool.phase(), Phase::Idle);
        assert!(r.release(p(2.0, 0.0)).commands.is_empty());
    }

    #[test]
    fn the_path_lies_in_the_plane_of_the_face_under_the_press() {
        // A wall in the plane x = 0.
        let mut m = Model::new();
        m.add_polygon(&[
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.0, 4.0, 0.0),
            DVec3::new(0.0, 4.0, 3.0),
            DVec3::new(0.0, 0.0, 3.0),
        ])
        .unwrap();
        let wall = m.faces().next().unwrap().0;
        let mut r = Rig::new(ToolKind::Freehand, m);
        let on_wall = |y: f64, z: f64| DVec3::new(0.0, y, z);
        let hit = r.at(on_wall(1.0, 1.0), SnapKind::OnFace, Some(wall));
        r.send(ToolEvent::Click(hit));
        // The inference is off the wall (in front of it); the cursor is over the wall.
        let target = on_wall(2.0, 2.0);
        r.cursor = r.camera.project(target).unwrap();
        r.send(ToolEvent::Move(crate::inference::Inference {
            point: target + DVec3::X,
            kind: SnapKind::Free,
            face: None,
        }));
        let target = on_wall(3.0, 1.0);
        let hit = r.at(target, SnapKind::OnFace, Some(wall));
        let out = r.send(ToolEvent::Release(hit));
        let (points, _, _) = curve(&out);
        assert_eq!(points.len(), 3, "{points:?}");
        assert!(points.iter().all(|q| q.x.abs() < 1e-9), "{points:?}");
        assert!(points[1].abs_diff_eq(on_wall(2.0, 2.0), 1e-6), "{points:?}");
    }

    #[test]
    fn on_the_ground_the_plane_follows_the_drawing_axes() {
        let mut r = rig();
        // Axes with the blue axis along -Y: a vertical plane through the press.
        r.axes = DAffine3::from_mat3_translation(
            DMat3::from_cols(DVec3::X, DVec3::Z, -DVec3::Y),
            DVec3::ZERO,
        );
        r.click(p(0.0, 0.0));
        // The cursor is over (1, 0, 1); the inference is on the ground behind it.
        let target = DVec3::new(1.0, 0.0, 1.0);
        r.cursor = r.camera.project(target).unwrap();
        r.send(ToolEvent::Move(crate::inference::Inference {
            point: p(1.0, 1.0),
            kind: SnapKind::Ground,
            face: None,
        }));
        let hit = r.at(DVec3::new(2.0, 0.0, 0.0), SnapKind::Ground, None);
        let out = r.send(ToolEvent::Release(hit));
        let (points, _, _) = curve(&out);
        assert_eq!(points.len(), 3, "{points:?}");
        assert!(points.iter().all(|q| q.y.abs() < 1e-9), "{points:?}");
        assert!(points[1].abs_diff_eq(target, 1e-6), "{points:?}");
    }

    #[test]
    fn a_ten_thousand_sample_drag_is_fast_and_within_the_limit() {
        let mut r = rig();
        let start = Instant::now();
        r.click(p(0.0, 0.0));
        for i in 1..10_000 {
            let t = i as f64 * 0.002;
            let radius = 0.5 + 0.01 * t;
            r.mv(p(radius * t.cos() - 0.5, radius * t.sin()));
        }
        let out = r.release(p(3.0, 3.0));
        let elapsed = start.elapsed().as_secs_f64();
        let (points, _, _) = curve(&out);
        assert!(elapsed < 10.0, "{elapsed} s");
        assert!(points.len() >= 20 && points.len() <= 10_000);
    }

    #[test]
    fn samples_beyond_the_limit_are_dropped() {
        let mut r = rig();
        r.click(p(0.0, 0.0));
        for i in 1..15_000 {
            r.mv(p(i as f64 * 0.001, if i % 2 == 0 { 0.0 } else { 0.5 }));
        }
        let out = r.release(p(20.0, 0.0));
        let (points, _, _) = curve(&out);
        assert!(
            points.len() >= 2 && points.len() <= 10_000,
            "{}",
            points.len()
        );
    }

    #[test]
    fn typed_values_are_not_taken() {
        let mut r = rig();
        let out = r.value("3");
        assert!(out.commands.is_empty() && out.message.is_some());
    }
}
