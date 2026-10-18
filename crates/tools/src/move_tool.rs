//! The Move tool: click a base point, then the destination; the selection (or else the edge or
//! face under the first click) moves by the difference. Tapping Ctrl switches to copying.
//! After a copy, typing "5x" makes 5 copies at that spacing and "/5" puts 5 copies evenly up
//! to it, replacing the single copy.

use glam::DVec3;
use kernel::EntityId;
use kernel::geom::LENGTH_TOL;
use session::{Command, MAX_COPIES};

use crate::select::{box_outline, face_outline, guide_outline, pick, whole_curve};
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{Array, format_length, parse_array, parse_length};

/// A move in progress.
#[derive(Clone, Debug)]
struct Drag {
    base: DVec3,
    ids: Vec<EntityId>,
    /// What moves, drawn at its original position.
    outline: Vec<[DVec3; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct MoveTool {
    drag: Option<Drag>,
    hover: Option<DVec3>,
    copy: bool,
    /// Whether something was selected at the last event, for the idle prompt.
    selected: bool,
    /// The last copy made (what and how far), which a typed array replaces.
    last_copy: Option<(Vec<EntityId>, DVec3)>,
}

impl MoveTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn finish(&mut self, offset: DVec3, out: &mut ToolOutput) {
        let Some(drag) = self.drag.take() else { return };
        if self.copy {
            out.commands.push(Command::Copy {
                ids: drag.ids.clone(),
                offset: offset.to_array(),
                count: None,
            });
            self.last_copy = Some((drag.ids, offset));
        } else {
            out.commands.push(Command::Move {
                ids: drag.ids,
                offset: offset.to_array(),
            });
            self.last_copy = None;
        }
    }

    /// Replaces the last copy with an array of copies.
    fn array(&mut self, array: Array, out: &mut ToolOutput) {
        let Some((ids, offset)) = self.last_copy.clone() else {
            out.message = Some("Make a copy first, then type 5x or /5".into());
            return;
        };
        let (spacing, count) = match array {
            Array::Times(n) => (offset, n),
            Array::Divide(n) => (offset / n as f64, n),
        };
        if count > MAX_COPIES {
            out.message = Some(format!("At most {MAX_COPIES} copies"));
            return;
        }
        out.commands.push(Command::Undo {
            count: None,
            expect: None,
        });
        out.commands.push(Command::Copy {
            ids,
            offset: spacing.to_array(),
            count: Some(count),
        });
    }
}

/// Segments outlining edges, faces, instances (their bounding boxes) and guides.
pub(crate) fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
    let m = ctx.model;
    let mut out = Vec::new();
    for id in ids {
        match *id {
            EntityId::Edge(e) => {
                if let Some(e) = m.edge(e)
                    && let (Some(a), Some(b)) = (m.position(e.v[0]), m.position(e.v[1]))
                {
                    out.push([a, b]);
                }
            }
            EntityId::Face(f) => {
                if let Some(f) = m.face(f) {
                    out.extend(face_outline(m, f));
                }
            }
            EntityId::Instance(_) => out.extend(box_outline(ctx, *id)),
            EntityId::Guide(g) => {
                if let Some(g) = m.guide(g) {
                    out.extend(guide_outline(ctx, g));
                }
            }
            EntityId::Vertex(_) => {}
        }
    }
    out
}

impl Tool for MoveTool {
    fn name(&self) -> &'static str {
        "Move"
    }

    fn prompt(&self) -> &'static str {
        match (&self.drag, self.copy, &self.last_copy) {
            (Some(_), false, _) => "Click the destination, or type a distance",
            (Some(_), true, _) => "Copy: click the destination, or type a distance",
            (None, _, Some(_)) => "Type 5x or /5 for more copies, or click a new base point",
            (None, false, None) if self.selected => {
                "Click a base point to move the selection (Ctrl copies)"
            }
            (None, true, None) if self.selected => "Copy: click a base point to copy the selection",
            (None, false, None) => {
                "Click an edge, face or group to pick it up, or select first (Ctrl copies)"
            }
            (None, true, None) => {
                "Copy: click an edge, face or group to pick it up, or select first"
            }
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        let distance = self
            .drag
            .as_ref()
            .zip(self.hover)
            .map(|(d, h)| format_length(d.base.distance(h)));
        ("Distance", distance)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.drag.as_ref().map(|d| d.base)
    }

    /// The base point.
    fn phase(&self) -> Phase {
        match &self.drag {
            Some(d) => Phase::Active {
                points: vec![d.base],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.selected = !ctx.selection.is_empty();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => match &self.drag {
                None => {
                    let ids: Vec<EntityId> = if ctx.selection.is_empty() {
                        pick(ctx).map_or(Vec::new(), |id| whole_curve(ctx.model, id))
                    } else {
                        ctx.selection.iter().copied().collect()
                    };
                    if ids.is_empty() {
                        out.message =
                            Some("Select geometry, or click an edge, face or group".into());
                    } else {
                        self.drag = Some(Drag {
                            base: hit.point,
                            outline: outline(ctx, &ids),
                            ids,
                        });
                        self.hover = Some(hit.point);
                    }
                }
                Some(d) => {
                    let offset = hit.point - d.base;
                    if offset.length() >= LENGTH_TOL {
                        self.finish(offset, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match &self.drag {
                Some(d) => {
                    let dir = self
                        .hover
                        .map(|h| h - d.base)
                        .filter(|v| v.length() >= LENGTH_TOL);
                    match (dir, parse_length(&text)) {
                        (None, _) => {
                            out.message = Some("Move the cursor to set a direction first".into());
                        }
                        (_, None) => out.message = Some(format!("Not a distance: {text}")),
                        (Some(_), Some(len)) if len.abs() < LENGTH_TOL => {
                            out.message = Some("Distance must not be zero".into());
                        }
                        (Some(dir), Some(len)) => self.finish(dir.normalize() * len, &mut out),
                    }
                }
                None => match parse_array(&text) {
                    Some(array) => self.array(array, &mut out),
                    None => out.message = Some(format!("Not an array like 5x or /5: {text}")),
                },
            },
            ToolEvent::ToggleCopy => self.copy = !self.copy,
            ToolEvent::Cancel => self.drag = None,
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) => {}
        }
        if let (Some(d), Some(hover)) = (&self.drag, self.hover) {
            let offset = hover - d.base;
            out.preview = d.outline.clone();
            if offset.length() >= LENGTH_TOL {
                out.preview
                    .extend(d.outline.iter().map(|[a, b]| [*a + offset, *b + offset]));
                out.preview.push([d.base, hover]);
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.drag = None;
        self.last_copy = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;
    use std::collections::BTreeSet;

    /// A 2 m square (face f8) and a camera looking at it.
    struct Rig {
        tool: MoveTool,
        model: Model,
        camera: Camera,
        selection: BTreeSet<EntityId>,
        cursor: DVec2,
    }

    impl Rig {
        fn new(selected: &[&str]) -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            Self {
                tool: MoveTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                selection: selected.iter().map(|s| s.parse().unwrap()).collect(),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model),
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &self.selection,
                material: None,
                sample: false,
            };
            self.tool.handle(event, &ctx)
        }

        /// Puts the cursor over `p` and clicks there.
        fn click(&mut self, p: DVec3) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            self.send(ToolEvent::Click(at(p)))
        }
    }

    fn at(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn ids(s: &[&str]) -> Vec<EntityId> {
        s.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn the_idle_prompt_says_what_a_click_moves() {
        let prompts = |selected: &[&str]| {
            let mut r = Rig::new(selected);
            r.send(ToolEvent::Move(at(DVec3::ZERO)));
            let moving = r.tool.prompt();
            r.send(ToolEvent::ToggleCopy);
            r.send(ToolEvent::Move(at(DVec3::ZERO)));
            (moving, r.tool.prompt())
        };
        assert_eq!(
            prompts(&["f8"]),
            (
                "Click a base point to move the selection (Ctrl copies)",
                "Copy: click a base point to copy the selection"
            )
        );
        assert_eq!(
            prompts(&[]),
            (
                "Click an edge, face or group to pick it up, or select first (Ctrl copies)",
                "Copy: click an edge, face or group to pick it up, or select first"
            )
        );
    }

    #[test]
    fn two_clicks_move_the_selection() {
        let mut r = Rig::new(&["f8", "e2"]);
        r.click(DVec3::ZERO);
        let moved = r.send(ToolEvent::Move(at(DVec3::new(1.0, 2.0, 0.0))));
        assert_eq!(
            moved.preview.len(),
            5 + 5 + 1,
            "outline, moved outline, guide"
        );
        let out = r.click(DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(
            out.commands,
            vec![Command::Move {
                ids: ids(&["e2", "f8"]),
                offset: [1.0, 2.0, 0.0]
            }]
        );
        assert!(r.tool.anchor().is_none());
    }

    #[test]
    fn without_a_selection_the_first_click_picks_what_to_move() {
        let mut r = Rig::new(&[]);
        assert!(r.click(DVec3::new(1.0, 1.0, 0.0)).message.is_none());
        let out = r.click(DVec3::new(1.0, 1.0, 1.0));
        assert_eq!(
            out.commands,
            vec![Command::Move {
                ids: ids(&["f8"]),
                offset: [0.0, 0.0, 1.0]
            }]
        );
        let empty = r.click(DVec3::new(-3.0, 4.0, 0.0));
        assert!(empty.message.is_some() && empty.commands.is_empty());
    }

    #[test]
    fn typed_distance_moves_along_the_cursor_direction() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::ZERO);
        r.send(ToolEvent::Move(at(DVec3::new(0.5, 0.0, 0.0))));
        let out = r.send(ToolEvent::Value("3".into()));
        assert_eq!(
            out.commands,
            vec![Command::Move {
                ids: ids(&["f8"]),
                offset: [3.0, 0.0, 0.0]
            }]
        );
    }

    #[test]
    fn ctrl_copies_and_arrays_replace_the_copy() {
        let mut r = Rig::new(&["f8"]);
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        let out = r.click(DVec3::new(3.0, 0.0, 0.0));
        assert_eq!(
            out.commands,
            vec![Command::Copy {
                ids: ids(&["f8"]),
                offset: [3.0, 0.0, 0.0],
                count: None
            }]
        );
        let times = r.send(ToolEvent::Value("4x".into()));
        assert_eq!(
            times.commands,
            vec![
                Command::Undo {
                    count: None,
                    expect: None
                },
                Command::Copy {
                    ids: ids(&["f8"]),
                    offset: [3.0, 0.0, 0.0],
                    count: Some(4)
                }
            ]
        );
        let divide = r.send(ToolEvent::Value("/3".into()));
        assert_eq!(
            divide.commands,
            vec![
                Command::Undo {
                    count: None,
                    expect: None
                },
                Command::Copy {
                    ids: ids(&["f8"]),
                    offset: [1.0, 0.0, 0.0],
                    count: Some(3)
                }
            ]
        );
    }

    #[test]
    fn oversized_arrays_are_refused_and_keep_the_copy() {
        let mut r = Rig::new(&["f8"]);
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(3.0, 0.0, 0.0));
        for text in ["1001x", "/1001"] {
            let out = r.send(ToolEvent::Value(text.into()));
            assert!(out.commands.is_empty() && out.message.is_some(), "{text}");
            assert_eq!(out.message.as_deref(), Some("At most 1000 copies"));
        }
        let ok = r.send(ToolEvent::Value("100x".into()));
        assert_eq!(ok.commands.len(), 2);
    }

    #[test]
    fn arrays_need_a_fresh_copy() {
        let mut r = Rig::new(&["f8"]);
        assert!(r.send(ToolEvent::Value("3x".into())).message.is_some());
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(3.0, 0.0, 0.0));
        r.tool.on_external_change();
        let out = r.send(ToolEvent::Value("3x".into()));
        assert!(out.commands.is_empty() && out.message.is_some());
    }

    /// An 8-sided circle of radius 1 on the ground, as the model and as a document's.
    fn circle_rig(r: &mut Rig) -> session::Document {
        let ring: Vec<DVec3> = (0..8)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 8.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        let mut model = Model::new();
        model.add_curve(&ring, true).unwrap();
        r.model = model;
        let mut d = session::Document::new(DVec2::new(800.0, 600.0));
        d.execute(Command::AddCircle {
            center: [0.0; 3],
            radius: 1.0,
            normal: None,
            segments: Some(8),
        })
        .unwrap();
        d
    }

    /// The middle of one rim edge.
    fn rim() -> DVec3 {
        let at = |deg: f64| {
            let a = deg.to_radians();
            DVec3::new(a.cos(), a.sin(), 0.0)
        };
        (at(225.0) + at(270.0)) / 2.0
    }

    fn circle_of(d: &session::Document, ids: &[EntityId]) -> kernel::CurveCircle {
        let EntityId::Edge(e) = ids[0] else {
            panic!("{ids:?}")
        };
        let c = d.model().curve_of(e).unwrap();
        assert_eq!(c.kind(), kernel::CurveKind::Circle);
        c.circle.unwrap()
    }

    #[test]
    fn without_a_selection_moving_a_rim_edge_moves_the_whole_circle() {
        let mut r = Rig::new(&[]);
        let mut d = circle_rig(&mut r);
        r.click(rim());
        let out = r.click(rim() + DVec3::new(0.0, 0.0, 3.0));
        let [Command::Move { ids, offset }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(ids.len(), 8, "every edge of the circle");
        assert_eq!(*offset, [0.0, 0.0, 3.0]);
        d.execute(out.commands[0].clone()).unwrap();
        let circle = circle_of(&d, ids);
        assert!(circle.center.distance(DVec3::new(0.0, 0.0, 3.0)) < 1e-9);
        assert!((circle.radius - 1.0).abs() < 1e-9);
    }
}
