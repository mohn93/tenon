//! What every interactive tool implements, and the tools the app can switch between.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, MaterialId, Model};
use session::{
    AnchorInput, Camera, Command, DEFAULT_ARC_SEGMENTS, DEFAULT_CIRCLE_SEGMENTS,
    DEFAULT_POLYGON_SIDES,
};

use crate::arc::ArcTool;
use crate::axes::AxesTool;
use crate::circle::CircleTool;
use crate::dimension::DimensionTool;
use crate::erase::EraseTool;
use crate::follow_me::FollowMeTool;
use crate::freehand::FreehandTool;
use crate::inference::{Axis, Inference};
use crate::line::LineTool;
use crate::move_tool::MoveTool;
use crate::offset::OffsetTool;
use crate::paint::PaintTool;
use crate::polygon::PolygonTool;
use crate::protractor::ProtractorTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;
use crate::rotate::RotateTool;
use crate::rotated_rectangle::RotatedRectangleTool;
use crate::scale::ScaleTool;
use crate::scenery::{Label, Scenery};
use crate::section::SectionPlaneTool;
use crate::select::SelectTool;
use crate::tape::TapeMeasureTool;
use crate::text::TextTool;

/// What a tool may look at while handling an event. Points, the camera and entity ids are in
/// the open context's coordinates (see `session::Document::context`).
pub struct ToolContext<'a> {
    /// The open context's mesh: what tools edit.
    pub model: &'a Model,
    /// Everything visible, for inference and picking; `scenery.open()` is `model`.
    pub scenery: &'a Scenery<'a>,
    pub camera: &'a Camera,
    /// Cursor position in physical pixels.
    pub cursor: DVec2,
    /// How close (physical pixels) the cursor must be to an edge to pick it.
    pub pick_radius: f64,
    /// How far (physical pixels) the cursor must travel from a press for it to be a drag.
    pub drag_threshold: f64,
    /// Whether this press of the primary button comes within the double-click time of the
    /// previous click's release (false for every other event). The window times it; in
    /// `tool_input` only a `double_click` or `triple_click` event's later presses are quick.
    pub quick_press: bool,
    /// Whether Shift is held.
    pub shift: bool,
    /// The selected edges and faces.
    pub selection: &'a BTreeSet<EntityId>,
    /// The material the Paint tool paints with (None: the default material).
    pub material: Option<MaterialId>,
    /// Whether the sampling modifier is held (Cmd on macOS, Ctrl elsewhere): the Paint tool
    /// then picks up the material clicked instead of painting.
    pub sample: bool,
    /// The annotations' labels as the window last drew them, for picking; empty where no
    /// window draws them (headless, the API).
    pub labels: &'a [Label],
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// The primary button was released at the current inference.
    Release(Inference),
    /// The primary button was double-clicked at the current inference (sent after the second
    /// release).
    DoubleClick(Inference),
    /// The primary button was clicked a third time in quick succession (sent after the third
    /// release; the second sent `DoubleClick`).
    TripleClick(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
    /// The copy modifier was tapped (Ctrl): tools that can copy switch between moving and
    /// copying; the Tape Measure switches guide creation on and off.
    ToggleCopy,
}

/// What a tool wants done after an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    /// Commands to execute, in order (each is one undo step).
    pub commands: Vec<Command>,
    /// Segments to draw as a live preview.
    pub preview: Vec<[DVec3; 2]>,
    /// A solid to draw as a live preview.
    pub solid: Option<Solid>,
    /// Feedback for the user, e.g. why typed text was ignored.
    pub message: Option<String>,
    /// `message` informs (a side count, a measured distance) rather than refuses.
    pub info: bool,
    /// A material the tool picked up to paint with next (Some(None): the default material).
    pub sampled: Option<Option<MaterialId>>,
    /// A note whose text the window should ask for (its note sheet).
    pub note: Option<NoteRequest>,
}

/// A note whose text is asked for: one the Text tool is placing (its text then goes back to
/// the tool as a `ToolEvent::Value`), or one of the open context to change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteRequest {
    /// A leader note pointing at `anchor` (at `point`), its text at `position`; no text gives
    /// it its automatic text.
    Leader {
        anchor: AnchorInput,
        point: DVec3,
        position: DVec3,
    },
    /// A note fixed to the view at `at` (fractions of its width and height from the top left);
    /// it needs text.
    Screen { at: [f64; 2] },
    /// The note of the open context to change the text of (a double-click with Select).
    Edit(EntityId),
}

/// A translucent solid a tool previews (Push/Pull's pulled shape), in the open context's
/// coordinates, shown in `material`'s color (None: the default material). Its triangles wind
/// counter-clockwise seen from outside the solid.
#[derive(Clone, Debug, PartialEq)]
pub struct Solid {
    pub triangles: Vec<[DVec3; 3]>,
    pub material: Option<MaterialId>,
}

/// Where a tool is in its operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// No operation in progress: the next click starts one.
    Idle,
    /// An operation in progress, with the points it was given so far, in the open context's
    /// coordinates (none for the Select tool's click or box, which happen on screen).
    Active { points: Vec<DVec3> },
}

/// An interactive way to build commands. Tools never edit the model themselves.
pub trait Tool {
    fn name(&self) -> &'static str;
    /// What the user should do next.
    fn prompt(&self) -> &'static str;
    /// The measurements box label and the live value, e.g. ("Length", Some("2.5 m")).
    fn measurement(&self) -> (&'static str, Option<String>);
    /// The point axis inference starts from, if any.
    fn anchor(&self) -> Option<DVec3>;
    /// Whether an operation is in progress, and the points it was given so far.
    fn phase(&self) -> Phase;
    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput;
    /// The model changed outside the tool (agent edit, undo, redo): drop any operation in
    /// progress, since it may refer to geometry that no longer exists.
    fn on_external_change(&mut self);
    /// The editor refused the output just returned, which would undo a step that is not the
    /// tool's own (a retype after another step): undo what that output changed in the tool,
    /// then drop its operation as for an external change.
    fn on_refused(&mut self) {
        self.on_external_change();
    }
    /// What a click would act on now (Select, Push/Pull, Paint, Erase): an id of the open
    /// context; None for the other tools, while an operation is in progress, and until the
    /// cursor moves again after a click, Escape or an external change.
    fn hovered(&self) -> Option<EntityId> {
        None
    }
    /// Writes the settings this tool keeps for the session (such as a circle's side count)
    /// into `r`, so the tool made next time starts with them.
    fn remember(&self, _r: &mut Remembered) {}
    /// Before its first click, a tool whose operation lies in a plane (Rotate) can lock that
    /// plane's normal to an axis (`Some`) or unlock it (`None`). True if the tool took it.
    fn lock_plane(&mut self, _axis: Option<Axis>) -> bool {
        false
    }
    /// The axis `lock_plane` locked, if any.
    fn plane_lock(&self) -> Option<Axis> {
        None
    }
    /// Whether the tool waits for the text of a note it asked for (`ToolOutput::note`).
    fn awaits_text(&self) -> bool {
        false
    }
}

/// Drag to draw: where the press that started an operation went down, until its release.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DragStart(Option<DVec2>);

impl DragStart {
    /// Remembers the press at `cursor` (call when a click started the operation).
    pub(crate) fn arm(&mut self, cursor: DVec2) {
        self.0 = Some(cursor);
    }

    /// Whether this release is the operation's next click: a press was armed and the cursor
    /// is now `ctx.drag_threshold` or more from it, inside the view (a release over a panel
    /// leaves the operation at its first point). Always disarms.
    pub(crate) fn released(&mut self, ctx: &ToolContext) -> bool {
        let c = ctx.cursor;
        let in_view = c.cmpge(DVec2::ZERO).all() && c.cmple(ctx.camera.viewport).all();
        self.0
            .take()
            .is_some_and(|press| in_view && press.distance(c) >= ctx.drag_threshold)
    }
}

/// Tool settings kept for the session, across switching tools.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remembered {
    /// Sides of the circles the Circle tool draws.
    pub circle_sides: u32,
    /// Sides of the polygons the Polygon tool draws.
    pub polygon_sides: u32,
    /// Whether the Polygon tool draws circumscribed polygons (radius to the middles of the
    /// sides) rather than inscribed ones.
    pub polygon_circumscribed: bool,
    /// Segments of the arcs the Arc tool draws.
    pub arc_segments: u32,
    /// The last Push/Pull distance, which a double-click on a face repeats.
    pub pull_distance: Option<f64>,
    /// The last Offset distance, which a double-click on a face repeats.
    pub offset_distance: Option<f64>,
}

impl Default for Remembered {
    fn default() -> Self {
        Remembered {
            circle_sides: DEFAULT_CIRCLE_SEGMENTS,
            polygon_sides: DEFAULT_POLYGON_SIDES,
            polygon_circumscribed: false,
            arc_segments: DEFAULT_ARC_SEGMENTS,
            pull_distance: None,
            offset_distance: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Select,
    Line,
    Rectangle,
    RotatedRectangle,
    Circle,
    Polygon,
    Arc,
    Freehand,
    PushPull,
    Move,
    Rotate,
    Scale,
    Offset,
    FollowMe,
    TapeMeasure,
    Protractor,
    Dimension,
    Text,
    Erase,
    Paint,
    SectionPlane,
    Axes,
}

impl ToolKind {
    /// Every tool, in toolbar order.
    pub const ALL: [ToolKind; 22] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::RotatedRectangle,
        ToolKind::Circle,
        ToolKind::Polygon,
        ToolKind::Arc,
        ToolKind::Freehand,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::Scale,
        ToolKind::Offset,
        ToolKind::FollowMe,
        ToolKind::TapeMeasure,
        ToolKind::Protractor,
        ToolKind::Dimension,
        ToolKind::Text,
        ToolKind::Erase,
        ToolKind::Paint,
        ToolKind::SectionPlane,
        ToolKind::Axes,
    ];

    /// The tool with default settings.
    pub fn make(self) -> Box<dyn Tool> {
        self.make_with(&Remembered::default())
    }

    /// The tool, starting with the settings in `r`.
    pub fn make_with(self, r: &Remembered) -> Box<dyn Tool> {
        match self {
            ToolKind::Select => Box::new(SelectTool::new()),
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::RotatedRectangle => Box::new(RotatedRectangleTool::new()),
            ToolKind::Circle => Box::new(CircleTool::with_sides(r.circle_sides)),
            ToolKind::Polygon => Box::new(PolygonTool::with_settings(
                r.polygon_sides,
                r.polygon_circumscribed,
            )),
            ToolKind::Arc => Box::new(ArcTool::with_segments(r.arc_segments)),
            ToolKind::PushPull => Box::new(PushPullTool::with_distance(r.pull_distance)),
            ToolKind::Freehand => Box::new(FreehandTool::new()),
            ToolKind::Move => Box::new(MoveTool::new()),
            ToolKind::Rotate => Box::new(RotateTool::new()),
            ToolKind::Scale => Box::new(ScaleTool::new()),
            ToolKind::Offset => Box::new(OffsetTool::with_distance(r.offset_distance)),
            ToolKind::FollowMe => Box::new(FollowMeTool::new()),
            ToolKind::TapeMeasure => Box::new(TapeMeasureTool::new()),
            ToolKind::Protractor => Box::new(ProtractorTool::new()),
            ToolKind::Dimension => Box::new(DimensionTool::new()),
            ToolKind::Text => Box::new(TextTool::new()),
            ToolKind::Erase => Box::new(EraseTool::new()),
            ToolKind::Paint => Box::new(PaintTool::new()),
            ToolKind::SectionPlane => Box::new(SectionPlaneTool::new()),
            ToolKind::Axes => Box::new(AxesTool::new()),
        }
    }

    /// The tool's name, as its `Tool::name` reports it.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Line => "Line",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::RotatedRectangle => "Rotated Rectangle",
            ToolKind::Freehand => "Freehand",
            ToolKind::Protractor => "Protractor",
            ToolKind::Circle => "Circle",
            ToolKind::Polygon => "Polygon",
            ToolKind::Arc => "Arc",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Move => "Move",
            ToolKind::Rotate => "Rotate",
            ToolKind::Scale => "Scale",
            ToolKind::Offset => "Offset",
            ToolKind::FollowMe => "Follow Me",
            ToolKind::TapeMeasure => "Tape Measure",
            ToolKind::Dimension => "Dimension",
            ToolKind::Text => "Text",
            ToolKind::Erase => "Erase",
            ToolKind::Paint => "Paint",
            ToolKind::SectionPlane => "Section Plane",
            ToolKind::Axes => "Axes",
        }
    }

    /// The key that selects the tool, if it has one (the Dimension, Text, Section Plane,
    /// Freehand, Rotated Rectangle and Protractor tools have none).
    pub fn shortcut(self) -> Option<char> {
        Some(match self {
            ToolKind::Select => ' ',
            ToolKind::Line => 'l',
            ToolKind::Rectangle => 'r',
            ToolKind::Circle => 'c',
            ToolKind::Polygon => 'n',
            ToolKind::Arc => 'a',
            ToolKind::PushPull => 'p',
            ToolKind::Move => 'm',
            ToolKind::Rotate => 'q',
            ToolKind::Scale => 's',
            ToolKind::Offset => 'o',
            ToolKind::FollowMe => 'j',
            ToolKind::TapeMeasure => 't',
            ToolKind::Dimension
            | ToolKind::Text
            | ToolKind::SectionPlane
            | ToolKind::RotatedRectangle
            | ToolKind::Freehand
            | ToolKind::Protractor => return None,
            ToolKind::Erase => 'e',
            ToolKind::Paint => 'b',
            ToolKind::Axes => 'y',
        })
    }

    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        let c = c.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.shortcut() == Some(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
    use kernel::FaceId;

    #[test]
    fn labels_match_tool_names_and_shortcuts_round_trip() {
        for kind in ToolKind::ALL {
            assert_eq!(kind.make().name(), kind.label());
            if let Some(c) = kind.shortcut() {
                assert_eq!(ToolKind::from_shortcut(c), Some(kind));
            }
        }
        assert_eq!(ToolKind::Dimension.shortcut(), None);
        assert_eq!(ToolKind::Text.shortcut(), None);
        assert_eq!(ToolKind::from_shortcut('E'), Some(ToolKind::Erase));
        assert_eq!(ToolKind::from_shortcut('z'), None);
    }

    /// `phase` with its points rounded to micrometers (Push/Pull's comes from a ray).
    fn rounded(phase: Phase) -> Phase {
        match phase {
            Phase::Idle => Phase::Idle,
            Phase::Active { points } => Phase::Active {
                points: points.iter().map(|p| (*p * 1e6).round() / 1e6).collect(),
            },
        }
    }

    /// Clicks a fresh `kind` tool at each of `clicks` (points on a selected 2 m square face at
    /// the origin, seen from the default camera) and returns its phase after each, then after
    /// the button's release, rounded.
    fn phases(kind: ToolKind, clicks: &[DVec3]) -> Vec<Phase> {
        let mut model = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        model
            .add_polygon(&[p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)])
            .unwrap();
        let face: FaceId = model.faces().map(|(id, _)| id).next().unwrap();
        let selection = BTreeSet::from([EntityId::Face(face)]);
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let scenery = Scenery::single(&model);
        let mut tool = kind.make();
        let mut out = vec![rounded(tool.phase())];
        for &point in clicks {
            let ctx = ToolContext {
                model: &model,
                scenery: &scenery,
                camera: &camera,
                cursor: camera.project(point).unwrap(),
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &selection,
                material: None,
                sample: false,
                labels: &[],
            };
            let hit = Inference {
                point,
                kind: SnapKind::OnFace,
                face: Some(face),
            };
            tool.handle(ToolEvent::Click(hit), &ctx);
            out.push(rounded(tool.phase()));
            tool.handle(ToolEvent::Release(hit), &ctx);
            out.push(rounded(tool.phase()));
        }
        out
    }

    /// Builds a pointer event from the inference under the cursor.
    type Event = fn(Inference) -> ToolEvent;

    /// A fresh `kind` tool over the selected 2 m square of `phases`, sent `steps` in order: each
    /// puts the cursor over its point and sends the event built from the inference there (on
    /// the square for points on it, free otherwise). Returns every command and the phase after.
    fn drive(kind: ToolKind, steps: &[(Event, DVec3)]) -> (Vec<Command>, Phase) {
        let mut model = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        model
            .add_polygon(&[p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)])
            .unwrap();
        let face: FaceId = model.faces().map(|(id, _)| id).next().unwrap();
        let selection = BTreeSet::from([EntityId::Face(face)]);
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let scenery = Scenery::single(&model);
        let mut tool = kind.make();
        let mut commands = Vec::new();
        for &(make, point) in steps {
            let ctx = ToolContext {
                model: &model,
                scenery: &scenery,
                camera: &camera,
                cursor: camera.project(point).unwrap(),
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &selection,
                material: None,
                sample: false,
                labels: &[],
            };
            let on_square = point.z.abs() < 1e-9
                && (0.0..=2.0).contains(&point.x)
                && (0.0..=2.0).contains(&point.y);
            let hit = Inference {
                point,
                kind: if on_square {
                    SnapKind::OnFace
                } else {
                    SnapKind::Free
                },
                face: on_square.then_some(face),
            };
            commands.extend(tool.handle(make(hit), &ctx).commands);
        }
        (commands, rounded(tool.phase()))
    }

    /// A fresh `kind` tool over the selected 2 m square of `phases`: pressed over `from`, the
    /// cursor moved to `to`, released there. Returns every command and the phase after.
    fn dragged(kind: ToolKind, from: DVec3, to: DVec3) -> (Vec<Command>, Phase) {
        drive(
            kind,
            &[
                (ToolEvent::Click, from),
                (ToolEvent::Move, to),
                (ToolEvent::Release, to),
            ],
        )
    }

    #[test]
    fn a_release_past_the_drag_threshold_is_the_second_click() {
        let (a, b) = (DVec3::new(0.5, 0.5, 0.0), DVec3::new(1.5, 1.2, 0.0));
        let (cmds, phase) = dragged(ToolKind::Rectangle, a, b);
        assert!(
            matches!(cmds[..], [Command::AddRectangle { .. }]),
            "{cmds:?}"
        );
        assert_eq!(phase, Phase::Idle);
        let (cmds, _) = dragged(ToolKind::Circle, a, b);
        assert!(matches!(cmds[..], [Command::AddCircle { .. }]), "{cmds:?}");
        let (cmds, _) = dragged(ToolKind::Polygon, a, b);
        assert!(matches!(cmds[..], [Command::AddPolygon { .. }]), "{cmds:?}");
        // Line: one edge, and the chain goes on from its end.
        let (cmds, phase) = dragged(ToolKind::Line, a, b);
        assert!(matches!(cmds[..], [Command::AddEdge { .. }]), "{cmds:?}");
        assert!(matches!(phase, Phase::Active { .. }));
        // Arc: the drag gives the end; the bulge is still to come.
        let (cmds, phase) = dragged(ToolKind::Arc, a, b);
        assert!(cmds.is_empty());
        assert!(matches!(phase, Phase::Active { ref points } if points.len() == 2));
        // Push/Pull: dragged up off the face, it pulls.
        let (cmds, _) = dragged(
            ToolKind::PushPull,
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(1.0, 1.0, 0.8),
        );
        assert!(
            matches!(cmds[..], [Command::PushPull { distance, .. }] if distance > 0.1),
            "{cmds:?}"
        );
    }

    #[test]
    fn a_release_near_the_press_waits_for_the_next_click() {
        // 1 mm on the ground is well under 4 px from the default camera.
        let a = DVec3::new(0.5, 0.5, 0.0);
        for kind in [
            ToolKind::Line,
            ToolKind::Rectangle,
            ToolKind::Circle,
            ToolKind::Polygon,
            ToolKind::Arc,
        ] {
            let (cmds, phase) = dragged(kind, a, a + DVec3::new(0.001, 0.0, 0.0));
            assert!(cmds.is_empty(), "{kind:?}");
            assert!(matches!(phase, Phase::Active { .. }), "{kind:?}");
        }
    }

    #[test]
    fn only_the_press_that_starts_an_operation_drags() {
        let a = DVec3::new(0.5, 0.5, 0.0);
        let b = DVec3::new(1.5, 0.5, 0.0);
        let c = DVec3::new(1.5, 1.5, 0.0);
        let d = DVec3::new(0.2, 1.8, 0.0);
        // Line: click a, click b (edge a-b), press at c (edge b-c: a press in a chain is a
        // click), drag and release at d far away. The release adds nothing.
        let (cmds, phase) = drive(
            ToolKind::Line,
            &[
                (ToolEvent::Click, a),
                (ToolEvent::Release, a),
                (ToolEvent::Click, b),
                (ToolEvent::Release, b),
                (ToolEvent::Click, c),
                (ToolEvent::Move, d),
                (ToolEvent::Release, d),
            ],
        );
        assert!(
            matches!(cmds[..], [Command::AddEdge { .. }, Command::AddEdge { .. }]),
            "{cmds:?}"
        );
        assert_eq!(phase, active(&[a, c]), "the chain goes on from c");
        // Arc: the bulge press is a click too; dragging it does not finish the arc again.
        let (cmds, phase) = drive(
            ToolKind::Arc,
            &[
                (ToolEvent::Click, a),
                (ToolEvent::Release, a),
                (ToolEvent::Click, b),
                (ToolEvent::Move, c),
                (ToolEvent::Release, c),
            ],
        );
        assert!(cmds.is_empty(), "{cmds:?}");
        assert_eq!(phase, active(&[a, b]));
    }

    #[test]
    fn a_drag_released_outside_the_view_leaves_the_first_point() {
        // The press in the view, the release over a panel left of it: the operation stays at
        // its first point, as after a click.
        let (model, face) = square_model();
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let scenery = Scenery::single(&model);
        let selection = BTreeSet::new();
        let a = DVec3::new(0.5, 0.5, 0.0);
        for kind in [
            ToolKind::Line,
            ToolKind::Rectangle,
            ToolKind::Circle,
            ToolKind::Polygon,
        ] {
            let mut tool = kind.make();
            let ctx = |cursor| ToolContext {
                model: &model,
                scenery: &scenery,
                camera: &camera,
                cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &selection,
                material: None,
                sample: false,
                labels: &[],
            };
            let press = Inference {
                point: a,
                kind: SnapKind::OnFace,
                face: Some(face),
            };
            tool.handle(ToolEvent::Click(press), &ctx(camera.project(a).unwrap()));
            let off = Inference {
                point: DVec3::new(-3.0, 1.0, 0.0),
                kind: SnapKind::Ground,
                face: None,
            };
            let out = tool.handle(ToolEvent::Release(off), &ctx(DVec2::new(-40.0, 300.0)));
            assert!(out.commands.is_empty(), "{kind:?}: {:?}", out.commands);
            assert_eq!(rounded(tool.phase()), active(&[a]), "{kind:?}");
        }
    }

    /// The 2 m square of `phases`, unselected, and its face.
    fn square_model() -> (Model, FaceId) {
        let mut model = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        model
            .add_polygon(&[p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)])
            .unwrap();
        let face = model.faces().map(|(id, _)| id).next().unwrap();
        (model, face)
    }

    /// Moves the cursor of `tool` over `point` on `face`, seen from the default camera.
    fn moved(tool: &mut dyn Tool, model: &Model, point: DVec3, face: FaceId) {
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let scenery = Scenery::single(model);
        let ctx = ToolContext {
            model,
            scenery: &scenery,
            camera: &camera,
            cursor: camera.project(point).unwrap(),
            pick_radius: 10.0,
            drag_threshold: 4.0,
            quick_press: false,
            shift: false,
            selection: &BTreeSet::new(),
            material: None,
            sample: false,
            labels: &[],
        };
        let hit = Inference {
            point,
            kind: SnapKind::OnFace,
            face: Some(face),
        };
        tool.handle(ToolEvent::Move(hit), &ctx);
    }

    #[test]
    fn external_changes_clear_the_hover() {
        // A 2 m square seen from the default camera; the cursor over its middle.
        for kind in [
            ToolKind::Select,
            ToolKind::PushPull,
            ToolKind::Paint,
            ToolKind::Erase,
        ] {
            let (model, face) = square_model();
            let mut tool = kind.make();
            moved(&mut *tool, &model, DVec3::new(1.0, 1.0, 0.0), face);
            assert_eq!(tool.hovered(), Some(EntityId::Face(face)), "{kind:?}");
            tool.on_external_change();
            assert_eq!(tool.hovered(), None, "{kind:?}");
        }
        for kind in [ToolKind::Line, ToolKind::Move, ToolKind::TapeMeasure] {
            assert_eq!(kind.make().hovered(), None);
        }
    }

    fn active(points: &[DVec3]) -> Phase {
        Phase::Active {
            points: points.to_vec(),
        }
    }

    #[test]
    fn phases_list_the_points_clicked_so_far() {
        let a = DVec3::new(0.5, 0.5, 0.0);
        let b = DVec3::new(1.5, 0.5, 0.0);
        let c = DVec3::new(1.5, 1.5, 0.0);
        let idle = Phase::Idle;
        assert_eq!(
            phases(ToolKind::Line, &[a, b]),
            vec![
                idle.clone(),
                active(&[a]),
                active(&[a]),
                active(&[a, b]),
                active(&[a, b])
            ]
        );
        for kind in [ToolKind::Rectangle, ToolKind::Circle, ToolKind::Polygon] {
            assert_eq!(
                phases(kind, &[a]),
                vec![idle.clone(), active(&[a]), active(&[a])],
                "{kind:?}"
            );
        }
        assert_eq!(
            phases(ToolKind::Arc, &[a, b]),
            vec![
                idle.clone(),
                active(&[a]),
                active(&[a]),
                active(&[a, b]),
                active(&[a, b])
            ]
        );
        for kind in [ToolKind::PushPull, ToolKind::Move, ToolKind::TapeMeasure] {
            assert_eq!(
                phases(kind, &[a]),
                vec![idle.clone(), active(&[a]), active(&[a])],
                "{kind:?}"
            );
        }
        assert_eq!(
            phases(ToolKind::Rotate, &[a, b]),
            vec![
                idle.clone(),
                active(&[a]),
                active(&[a]),
                active(&[a, b]),
                active(&[a, b])
            ]
        );
        // A finished operation is idle again.
        let finished = phases(ToolKind::Rectangle, &[a, c]);
        assert_eq!(finished.last(), Some(&idle));
        // The Select tool is active while its button is held; Erase and Paint act at once.
        assert_eq!(
            phases(ToolKind::Select, &[a]),
            vec![idle.clone(), active(&[]), idle.clone()]
        );
        for kind in [ToolKind::Erase, ToolKind::Paint] {
            assert_eq!(phases(kind, &[a]), vec![idle.clone(); 3], "{kind:?}");
        }
    }
}
