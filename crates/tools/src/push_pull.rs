//! The Push/Pull tool: click a face, move to set the distance along its normal, click again (or
//! type the distance). A drag off the face works as the two clicks. Right after a pull, a typed
//! distance redoes it at that distance; a double-click on a face repeats the last pull distance
//! (kept for the session).

use glam::{DVec2, DVec3};
use kernel::geom::LENGTH_TOL;
use kernel::{EntityId, FaceId, MaterialId, Model};
use session::Command;

use crate::inference::{Inference, SnapKind, closest_on_line_to_ray, face_under_in};
use crate::scenery::Owner;
use crate::tool::{DragStart, Phase, Remembered, Solid, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length, toward_cursor};

/// A push/pull in progress.
#[derive(Clone, Debug)]
struct Drag {
    face: FaceId,
    normal: DVec3,
    /// Where the face was clicked; the cursor is tracked along the normal line through it.
    start: DVec3,
    /// Where the drag's first click was pressed and released, on screen.
    press: DVec2,
    release: Option<DVec2>,
    /// The face's loops (outer first), for the preview.
    loops: Vec<Vec<DVec3>>,
    /// The face's triangles, for the preview solid's cap.
    triangles: Vec<[DVec3; 3]>,
    /// The face's front material, for the preview solid.
    material: Option<MaterialId>,
    distance: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PushPullTool {
    drag: Option<Drag>,
    /// The face a click would push or pull, while no push/pull is in progress.
    hovered: Option<FaceId>,
    drag_start: DragStart,
    /// The pull a typed value redoes: the face (its id before the pull, which an undo
    /// restores) and the distance.
    last: Option<(FaceId, f64)>,
    /// The distance a double-click repeats.
    repeat: Option<f64>,
}

impl PushPullTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// A Push/Pull tool whose double-click repeats `distance`.
    pub fn with_distance(distance: Option<f64>) -> Self {
        PushPullTool {
            repeat: distance,
            ..Self::default()
        }
    }

    fn finish(&mut self, distance: f64, out: &mut ToolOutput) {
        if let Some(d) = self.drag.take() {
            self.pull(d.face, distance, out);
        }
    }

    /// Pulls `face` by `distance`, as the pull a typed value redoes and a double-click repeats.
    fn pull(&mut self, face: FaceId, distance: f64, out: &mut ToolOutput) {
        self.last = Some((face, distance));
        self.repeat = Some(distance);
        out.commands.push(Command::PushPull {
            face: EntityId::Face(face),
            distance,
        });
    }

    /// Redoes the last pull at the typed `text`, keeping its direction.
    fn retype(&mut self, text: &str, out: &mut ToolOutput) {
        let Some((face, last)) = self.last else {
            out.message = Some("Click a face first".into());
            return;
        };
        match parse_length(text) {
            None => out.message = Some(format!("Not a length: {text}")),
            Some(v) if v.abs() < LENGTH_TOL => {
                out.message = Some("Distance must not be zero".into());
            }
            Some(v) => {
                out.commands.push(Command::Undo { count: None });
                self.pull(face, if last < 0.0 { -v } else { v }, out);
            }
        }
    }

    /// Repeats the last pull distance on the face of the pull in progress, else on the face
    /// under the cursor.
    fn repeat(&mut self, ctx: &ToolContext, out: &mut ToolOutput) {
        let face = match &self.drag {
            Some(d) => Some(d.face),
            None => open_face_under(ctx).map(|(f, _)| f),
        };
        out.info = true;
        match (self.repeat, face) {
            (None, _) => {
                out.message =
                    Some("Pull a face first; a double-click then repeats that distance".into());
            }
            (Some(_), None) => {
                out.message = Some(
                    "Push/Pull works on faces: double-click a face to repeat the last pull".into(),
                );
            }
            (Some(distance), Some(face)) => {
                out.info = false;
                self.drag = None;
                self.drag_start = DragStart::default();
                self.hovered = None;
                self.pull(face, distance, out);
                self.last = None;
            }
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

/// The open context's face under the cursor, and the point on it there.
fn open_face_under(ctx: &ToolContext) -> Option<(FaceId, DVec3)> {
    face_under_in(ctx.scenery, ctx.camera, ctx.cursor)
        .filter(|(owner, ..)| *owner == Owner::Open)
        .map(|(_, f, p)| (f, p))
}

fn outline(loops: &[Vec<DVec3>], offset: DVec3, out: &mut Vec<[DVec3; 2]>) {
    for l in loops {
        for i in 0..l.len() {
            out.push([l[i] + offset, l[(i + 1) % l.len()] + offset]);
        }
    }
}

/// The solid a pull by `drag.distance` adds: the face's triangles moved by the distance (the
/// cap) and two triangles per loop edge (the sides), wound outward. The base is left out.
fn solid(drag: &Drag) -> Solid {
    let o = drag.normal * drag.distance;
    let cap = drag.triangles.iter().map(|t| t.map(|p| p + o));
    let sides = drag.loops.iter().flat_map(|l| {
        (0..l.len()).flat_map(move |i| {
            let (p, q) = (l[i], l[(i + 1) % l.len()]);
            [[p, q, q + o], [p, q + o, p + o]]
        })
    });
    // Loops and triangles wind counter-clockwise around the normal (holes clockwise), so
    // these face outward when pulled along it; a push turns them inside out.
    let flip = drag.distance < 0.0;
    let triangles = cap
        .chain(sides)
        .map(|[a, b, c]| if flip { [a, c, b] } else { [a, b, c] })
        .collect();
    Solid {
        triangles,
        material: drag.material,
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
        match (&self.drag, self.last) {
            (None, None) => "Click a face to push or pull",
            (None, Some(_)) => "Type a distance to change the pull, or click a face",
            (Some(_), _) => "Move to set the distance, then click or type it",
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

    /// Where the face was clicked.
    fn phase(&self) -> Phase {
        match &self.drag {
            Some(d) => Phase::Active {
                points: vec![d.start],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => match self.drag.as_mut() {
                Some(d) => d.distance = measure(d, &hit, ctx),
                None => self.hovered = open_face_under(ctx).map(|(f, _)| f),
            },
            ToolEvent::Click(hit) => match self.drag.as_ref() {
                None => {
                    if let Some((face, start)) = open_face_under(ctx) {
                        let f = ctx.model.face(face).unwrap();
                        self.drag = Some(Drag {
                            face,
                            normal: f.normal,
                            start,
                            press: ctx.cursor,
                            release: None,
                            loops: loops(ctx.model, face),
                            triangles: ctx.model.triangulate(face).unwrap_or_default(),
                            material: f.front,
                            distance: 0.0,
                        });
                        self.drag_start.arm(ctx.cursor);
                        self.last = None;
                    } else {
                        // Say what Push/Pull works on (informing, not refusing).
                        out.info = true;
                        let grouped = matches!(
                            face_under_in(ctx.scenery, ctx.camera, ctx.cursor),
                            Some((Owner::Instance(_), ..))
                        );
                        out.message = Some(if grouped {
                            "Open the group to push or pull its faces".into()
                        } else {
                            "Push/Pull works on faces: click a face to push or pull it".into()
                        });
                    }
                    self.hovered = None;
                }
                // A click a few pixels from the first's press or release (a double-click's
                // second press) never finishes the pull.
                Some(d)
                    if std::iter::once(d.press)
                        .chain(d.release)
                        .any(|p| p.distance(ctx.cursor) < ctx.drag_threshold) => {}
                Some(d) => {
                    let distance = measure(d, &hit, ctx);
                    if distance.abs() >= LENGTH_TOL {
                        self.finish(distance, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match (self.drag.as_ref(), parse_length(&text)) {
                (None, _) => self.retype(&text, &mut out),
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
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
                if let Some(d) = self.drag.as_mut() {
                    d.release.get_or_insert(ctx.cursor);
                }
            }
            ToolEvent::DoubleClick(_) => self.repeat(ctx, &mut out),
            ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let Some(d) = &self.drag {
            let offset = d.normal * d.distance;
            outline(&d.loops, DVec3::ZERO, &mut out.preview);
            if d.distance.abs() >= LENGTH_TOL {
                outline(&d.loops, offset, &mut out.preview);
                for p in d.loops.iter().flatten() {
                    out.preview.push([*p, *p + offset]);
                }
                out.solid = Some(solid(d));
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.drag = None;
        self.hovered = None;
        self.drag_start = DragStart::default();
        self.last = None;
    }

    fn hovered(&self) -> Option<EntityId> {
        self.hovered.map(EntityId::Face)
    }

    fn remember(&self, r: &mut Remembered) {
        r.pull_distance = self.repeat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::face_under;
    use crate::tool::{Remembered, ToolKind};
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
        /// A rig whose tool repeats `distance` on a double-click.
        fn with_distance(distance: Option<f64>) -> Self {
            Self {
                tool: PushPullTool::with_distance(distance),
                ..Self::new()
            }
        }

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
            let hit = self.hit(p);
            self.send(make(hit))
        }

        /// The inference at `p`, with the face under the cursor (which stays where it is).
        fn hit(&self, p: DVec3) -> Inference {
            Inference {
                point: p,
                kind: SnapKind::OnFace,
                face: face_under(&self.model, &self.camera, self.cursor).map(|(f, _)| f),
            }
        }

        /// Moves the cursor by `offset` pixels.
        fn cursor_offset(&mut self, offset: DVec2) {
            self.cursor += offset;
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
                material: None,
                sample: false,
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
    fn hovering_a_face_reports_it_without_an_outline() {
        let mut r = Rig::new();
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move);
        assert_eq!(r.tool.hovered(), Some(EntityId::Face(r.face)));
        assert!(out.preview.is_empty());
        let off = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Move);
        assert_eq!(r.tool.hovered(), None);
        assert!(off.preview.is_empty());
        // A drag in progress hovers nothing.
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move);
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        assert_eq!(r.tool.hovered(), None);
        r.at(DVec3::new(1.0, 1.0, 0.5), ToolEvent::Move);
        assert_eq!(r.tool.hovered(), None);
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
    fn a_pull_previews_its_cap_and_sides_in_the_faces_material() {
        let mut r = Rig::new();
        let red = Some(MaterialId(5));
        r.model.paint_faces(&[r.face], kernel::Side::Front, red);
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        let out = r.at(DVec3::new(1.0, 1.0, 2.0), ToolEvent::Move);
        let solid = out.solid.expect("a solid while pulling");
        // A square: 2 cap triangles and 2 per side.
        assert_eq!(solid.triangles.len(), 2 + 4 * 2);
        let tops = solid
            .triangles
            .iter()
            .flatten()
            .filter(|p| (p.z - 2.0).abs() < 1e-9)
            .count();
        assert!(tops >= 6, "the cap is moved by the distance");
        assert!(
            solid
                .triangles
                .iter()
                .flatten()
                .all(|p| p.z > -1e-9 && p.z < 2.0 + 1e-9)
        );
        assert_eq!(solid.material, r.model.face(r.face).unwrap().front);
        assert_eq!(solid.material, red);
        // No pull yet: no solid.
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        assert_eq!(r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move).solid, None);
    }

    #[test]
    fn the_pulled_solid_winds_outward_both_ways() {
        for height in [1.5, -1.5] {
            let mut r = Rig::new();
            r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
            let out = r.at(DVec3::new(1.0, 1.0, height), ToolEvent::Move);
            let solid = out.solid.expect("a solid while pulling");
            let centre = DVec3::new(1.0, 1.0, height / 2.0);
            for [a, b, c] in &solid.triangles {
                let normal = (*b - *a).cross(*c - *a);
                let middle = (*a + *b + *c) / 3.0;
                assert!(normal.dot(middle - centre) > 0.0, "{height}: {a} {b} {c}");
            }
        }
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
        // The cursor is over the corner it snapped to.
        r.cursor = r.camera.project(corner.point).unwrap();
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
    fn clicking_off_a_face_says_what_push_pull_works_on() {
        let mut r = Rig::new();
        let out = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Click);
        assert!(out.commands.is_empty() && out.info);
        assert_eq!(r.tool.phase(), Phase::Idle, "no drag");
        assert_eq!(
            out.message.as_deref(),
            Some("Push/Pull works on faces: click a face to push or pull it")
        );
        // A face of a closed group: open the group first.
        let mut s = kernel::Scene::from_root(r.model.clone());
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        s.make_group(
            kernel::MeshId::Root,
            &edges,
            &faces,
            &[],
            kernel::DefinitionKind::Group,
            "",
        )
        .unwrap();
        let scenery = crate::scenery::Scenery::of(&s, &[]).unwrap();
        let ctx = ToolContext {
            model: s.root(),
            scenery: &scenery,
            camera: &r.camera,
            cursor: r.camera.project(DVec3::new(1.0, 1.0, 0.0)).unwrap(),
            pick_radius: 10.0,
            drag_threshold: 4.0,
            shift: false,
            selection: &Default::default(),
            material: None,
            sample: false,
        };
        let mut tool = PushPullTool::new();
        let out = tool.handle(ToolEvent::Click(r.hit(DVec3::new(1.0, 1.0, 0.0))), &ctx);
        assert!(out.commands.is_empty() && out.info);
        assert_eq!(
            out.message.as_deref(),
            Some("Open the group to push or pull its faces")
        );
        assert_eq!(tool.phase(), Phase::Idle);
    }

    #[test]
    fn a_typed_distance_after_a_pull_redoes_it() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(1.0, 1.0, 1.0), ToolEvent::Move);
        let first = r.at(DVec3::new(1.0, 1.0, 1.0), ToolEvent::Click);
        assert!(matches!(first.commands[..], [Command::PushPull { .. }]));
        assert_eq!(
            r.tool.prompt(),
            "Type a distance to change the pull, or click a face"
        );
        let out = r.send(ToolEvent::Value("2.5".into()));
        assert_eq!(
            out.commands,
            vec![Command::Undo { count: None }, push_pull(r.face, 2.5)]
        );
        // Typed again, it redoes the redone pull; bad values change nothing.
        let out = r.send(ToolEvent::Value("3".into()));
        assert_eq!(
            out.commands,
            vec![Command::Undo { count: None }, push_pull(r.face, 3.0)]
        );
        let out = r.send(ToolEvent::Value("abc".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Not a length: abc"));
        let out = r.send(ToolEvent::Value("0".into()));
        assert_eq!(out.message.as_deref(), Some("Distance must not be zero"));
        // A pull down keeps its direction; a negative value reverses it.
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(1.0, 1.0, -1.0), ToolEvent::Move);
        let first = r.at(DVec3::new(1.0, 1.0, -1.0), ToolEvent::Click);
        let [Command::PushPull { distance, .. }] = first.commands[..] else {
            panic!("{:?}", first.commands);
        };
        assert!((distance + 1.0).abs() < 1e-6, "{distance}");
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(out.commands[1], push_pull(r.face, -2.0));
        let out = r.send(ToolEvent::Value("-2".into()));
        assert_eq!(out.commands[1], push_pull(r.face, 2.0));
        // A new pull's first click forgets the last one.
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Cancel);
        let out = r.send(ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Click a face first"));
    }

    #[test]
    fn a_double_click_repeats_the_last_pull_distance() {
        let mut r = Rig::with_distance(Some(1.5));
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Release);
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Release);
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
        assert_eq!(out.commands, vec![push_pull(r.face, 1.5)]);
        assert_eq!(r.tool.phase(), Phase::Idle);
        // Without a distance to repeat, it explains (info) and pulls nothing.
        let mut r = Rig::new();
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
        assert!(out.commands.is_empty() && out.info);
        assert_eq!(
            out.message.as_deref(),
            Some("Pull a face first; a double-click then repeats that distance")
        );
        // Over no face, it says what it works on.
        let mut r = Rig::with_distance(Some(1.5));
        let out = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::DoubleClick);
        assert!(out.commands.is_empty() && out.info);
        assert_eq!(
            out.message.as_deref(),
            Some("Push/Pull works on faces: double-click a face to repeat the last pull")
        );
    }

    #[test]
    fn a_jittery_double_click_repeats_without_a_tiny_pull() {
        let mut r = Rig::with_distance(Some(1.0));
        let p = DVec3::new(1.0, 1.0, 0.0);
        r.at(p, ToolEvent::Click);
        r.at(p, ToolEvent::Release);
        // The second press lands 3 px higher on screen (under the 4 px threshold).
        r.cursor_offset(DVec2::new(0.0, -3.0));
        let second = r.send(ToolEvent::Click(r.hit(p)));
        assert!(
            second.commands.is_empty(),
            "no pull of a few pixels: {:?}",
            second.commands
        );
        r.send(ToolEvent::Release(r.hit(p)));
        let out = r.send(ToolEvent::DoubleClick(r.hit(p)));
        assert_eq!(out.commands, vec![push_pull(r.face, 1.0)]);
    }

    #[test]
    fn a_double_click_whose_first_release_slid_pulls_nothing_tiny() {
        // Press at P, release 3 px away, second press 5 px from P (2 px from the release).
        let mut r = Rig::with_distance(Some(1.0));
        let p = DVec3::new(1.0, 1.0, 0.0);
        r.at(p, ToolEvent::Click);
        r.cursor_offset(DVec2::new(0.0, -3.0));
        r.send(ToolEvent::Release(r.hit(p)));
        r.cursor_offset(DVec2::new(0.0, -2.0));
        let second = r.send(ToolEvent::Click(r.hit(p)));
        assert!(
            second.commands.is_empty(),
            "no pull of a few pixels: {:?}",
            second.commands
        );
        r.send(ToolEvent::Release(r.hit(p)));
        let out = r.send(ToolEvent::DoubleClick(r.hit(p)));
        assert_eq!(out.commands, vec![push_pull(r.face, 1.0)]);
    }

    #[test]
    fn a_retype_after_an_undo_explains_instead_of_undoing() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Value("1".into()));
        // What the editor's undo calls.
        r.tool.on_external_change();
        let out = r.send(ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert_eq!(out.message.as_deref(), Some("Click a face first"));
        assert_eq!(r.tool.prompt(), "Click a face to push or pull");
    }

    #[test]
    fn the_pull_distance_is_remembered_across_tools() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Value("1.25".into()));
        let mut remembered = Remembered::default();
        assert_eq!(remembered.pull_distance, None);
        r.tool.remember(&mut remembered);
        assert_eq!(remembered.pull_distance, Some(1.25));
        // The tool made with it starts with that distance.
        let mut again = Remembered::default();
        ToolKind::PushPull
            .make_with(&remembered)
            .remember(&mut again);
        assert_eq!(again.pull_distance, Some(1.25));
        let mut r = Rig::with_distance(remembered.pull_distance);
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
        assert_eq!(out.commands, vec![push_pull(r.face, 1.25)]);
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
