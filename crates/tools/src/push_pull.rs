//! The Push/Pull tool: click a face, move to set the distance along its normal, click again (or
//! type the distance).

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use kernel::{EntityId, FaceId, Model};
use session::Command;

use crate::inference::{Inference, SnapKind, closest_on_line_to_ray, face_under_in};
use crate::scenery::Owner;
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length, toward_cursor};

/// A push/pull in progress.
#[derive(Clone, Debug)]
struct Drag {
    face: FaceId,
    normal: DVec3,
    /// Where the face was clicked; the cursor is tracked along the normal line through it.
    start: DVec3,
    /// The face's loops (outer first), for the preview.
    loops: Vec<Vec<DVec3>>,
    distance: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PushPullTool {
    drag: Option<Drag>,
    /// Outline of the face under the cursor while no push/pull is in progress.
    hover: Vec<Vec<DVec3>>,
}

impl PushPullTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn finish(&mut self, distance: f64, out: &mut ToolOutput) {
        if let Some(d) = self.drag.take() {
            out.commands.push(Command::PushPull {
                face: EntityId::Face(d.face),
                distance,
            });
        }
    }
}

fn loops(model: &Model, id: FaceId) -> Vec<Vec<DVec3>> {
    let f = model.face(id).unwrap();
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .map(|l| l.iter().filter_map(|v| model.position(*v)).collect())
        .collect()
}

fn outline(loops: &[Vec<DVec3>], offset: DVec3, out: &mut Vec<[DVec3; 2]>) {
    for l in loops {
        for i in 0..l.len() {
            out.push([l[i] + offset, l[(i + 1) % l.len()] + offset]);
        }
    }
}

/// Distance from the face's plane along its normal for the cursor: the height of a snapped
/// point, otherwise of the point on the normal line nearest the cursor ray.
fn measure(drag: &Drag, hit: &Inference, ctx: &ToolContext) -> f64 {
    let snapped = matches!(
        hit.kind,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Origin
            | SnapKind::Intersection
            | SnapKind::OnEdge
    );
    let p = if snapped {
        hit.point
    } else {
        let ray = ctx.camera.ray(ctx.cursor);
        closest_on_line_to_ray(drag.start, drag.normal, &ray, false)
    };
    (p - drag.loops[0][0]).dot(drag.normal)
}

impl Tool for PushPullTool {
    fn name(&self) -> &'static str {
        "Push/Pull"
    }

    fn prompt(&self) -> &'static str {
        match self.drag {
            None => "Click a face to push or pull",
            Some(_) => "Move to set the distance, then click or type it",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        (
            "Distance",
            self.drag.as_ref().map(|d| format_length(d.distance)),
        )
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => match self.drag.as_mut() {
                Some(d) => d.distance = measure(d, &hit, ctx),
                None => self.hover = hit.face.map(|f| loops(ctx.model, f)).unwrap_or_default(),
            },
            ToolEvent::Click(hit) => match self.drag.as_ref() {
                None => {
                    let under = face_under_in(ctx.scenery, ctx.camera, ctx.cursor)
                        .filter(|(owner, ..)| *owner == Owner::Open)
                        .map(|(_, f, p)| (f, p));
                    if let Some((face, start)) = under {
                        self.drag = Some(Drag {
                            face,
                            normal: ctx.model.face(face).unwrap().normal,
                            start,
                            loops: loops(ctx.model, face),
                            distance: 0.0,
                        });
                        self.hover.clear();
                    }
                }
                Some(d) => {
                    let distance = measure(d, &hit, ctx);
                    if distance.abs() >= LENGTH_TOL {
                        self.finish(distance, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match (self.drag.as_ref(), parse_length(&text)) {
                (None, _) => out.message = Some("Click a face first".into()),
                (Some(_), None) => out.message = Some(format!("Not a length: {text}")),
                (Some(_), Some(v)) if v.abs() < LENGTH_TOL => {
                    out.message = Some("Distance must not be zero".into());
                }
                (Some(d), Some(v)) => {
                    // A typed distance goes the way the cursor is pulling; negative reverses it.
                    let v = toward_cursor(d.distance, v);
                    self.finish(v, &mut out);
                }
            },
            ToolEvent::Release(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        match &self.drag {
            Some(d) => {
                let offset = d.normal * d.distance;
                outline(&d.loops, DVec3::ZERO, &mut out.preview);
                if d.distance.abs() >= LENGTH_TOL {
                    outline(&d.loops, offset, &mut out.preview);
                    for p in d.loops.iter().flatten() {
                        out.preview.push([*p, *p + offset]);
                    }
                }
            }
            None => outline(&self.hover, DVec3::ZERO, &mut out.preview),
        }
        out
    }

    fn on_external_change(&mut self) {
        self.drag = None;
        self.hover.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::face_under;
    use glam::DVec2;
    use session::Camera;

    /// A 2x2 rectangle on the ground and a camera looking at it.
    struct Rig {
        tool: PushPullTool,
        model: Model,
        face: FaceId,
        camera: Camera,
        cursor: DVec2,
    }

    impl Rig {
        fn new() -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            let face = model.faces().next().unwrap().0;
            Self {
                tool: PushPullTool::new(),
                model,
                face,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        /// Puts the cursor over `p` and sends the event built from the inference there.
        fn at(&mut self, p: DVec3, make: fn(Inference) -> ToolEvent) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            let hit = Inference {
                point: p,
                kind: SnapKind::OnFace,
                face: face_under(&self.model, &self.camera, self.cursor).map(|(f, _)| f),
            };
            self.send(make(hit))
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model),
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                shift: false,
                selection: &Default::default(),
            };
            self.tool.handle(event, &ctx)
        }
    }

    fn push_pull(face: FaceId, distance: f64) -> Command {
        Command::PushPull {
            face: EntityId::Face(face),
            distance,
        }
    }

    #[test]
    fn hovering_a_face_outlines_it() {
        let mut r = Rig::new();
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move);
        assert_eq!(out.preview.len(), 4);
        let off = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Move);
        assert!(off.preview.is_empty());
    }

    #[test]
    fn click_drag_click_pulls_by_the_height_under_the_cursor() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        // The cursor over the point 1.5 m above the click lies on the normal line.
        let up = DVec3::new(1.0, 1.0, 1.5);
        let preview = r.at(up, ToolEvent::Move);
        assert_eq!(preview.preview.len(), 4 + 4 + 4);
        let (label, value) = r.tool.measurement();
        assert_eq!((label, value.as_deref()), ("Distance", Some("1.5 m")));
        let out = r.at(up, ToolEvent::Click);
        let [Command::PushPull { face, distance }] = out.commands.as_slice() else {
            panic!("expected one push_pull, got {:?}", out.commands);
        };
        assert_eq!(*face, EntityId::Face(r.face));
        assert!((distance - 1.5).abs() < 1e-6);
        assert!(r.tool.measurement().1.is_none(), "the drag ended");
    }

    #[test]
    fn snapped_points_set_the_distance_exactly() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        let corner = Inference {
            point: DVec3::new(5.0, 5.0, 0.75),
            kind: SnapKind::Endpoint,
            face: None,
        };
        let out = r.send(ToolEvent::Click(corner));
        assert_eq!(out.commands, vec![push_pull(r.face, 0.75)]);
    }

    #[test]
    fn typed_distance_follows_the_drag_direction() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(1.0, 1.0, -0.5), ToolEvent::Move);
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(out.commands, vec![push_pull(r.face, -2.0)]);
    }

    #[test]
    fn typed_distance_without_dragging_goes_along_the_normal() {
        let mut r = Rig::new();
        for x in [0.3, 0.7, 1.1, 1.3, 1.9] {
            r.at(DVec3::new(x, 1.0, 0.0), ToolEvent::Click);
            r.at(DVec3::new(x, 1.0, 0.0), ToolEvent::Move);
            let out = r.send(ToolEvent::Value("1".into()));
            assert_eq!(
                out.commands,
                vec![push_pull(r.face, 1.0)],
                "clicked at x = {x}"
            );
        }
    }

    #[test]
    fn clicking_off_any_face_or_typing_first_does_nothing() {
        let mut r = Rig::new();
        assert!(
            r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Click)
                .commands
                .is_empty()
        );
        assert!(r.tool.measurement().1.is_none());
        let out = r.send(ToolEvent::Value("1".into()));
        assert_eq!(out.message.as_deref(), Some("Click a face first"));
    }

    #[test]
    fn cancel_and_external_changes_end_the_drag() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Cancel);
        assert!(r.tool.measurement().1.is_none());
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.tool.on_external_change();
        assert!(r.tool.measurement().1.is_none());
    }
}
