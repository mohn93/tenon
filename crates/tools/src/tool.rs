//! What every interactive tool implements, and the tools the app can switch between.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, MaterialId, Model};
use session::{Camera, Command, DEFAULT_ARC_SEGMENTS, DEFAULT_CIRCLE_SEGMENTS};

use crate::arc::ArcTool;
use crate::circle::CircleTool;
use crate::erase::EraseTool;
use crate::inference::Inference;
use crate::line::LineTool;
use crate::move_tool::MoveTool;
use crate::paint::PaintTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;
use crate::rotate::RotateTool;
use crate::scenery::Scenery;
use crate::select::SelectTool;
use crate::tape::TapeMeasureTool;

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
    /// Whether Shift is held.
    pub shift: bool,
    /// The selected edges and faces.
    pub selection: &'a BTreeSet<EntityId>,
    /// The material the Paint tool paints with (None: the default material).
    pub material: Option<MaterialId>,
    /// Whether the sampling modifier is held (Cmd on macOS, Ctrl elsewhere): the Paint tool
    /// then picks up the material clicked instead of painting.
    pub sample: bool,
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
    /// Feedback for the user, e.g. why typed text was ignored.
    pub message: Option<String>,
    /// A material the tool picked up to paint with next (Some(None): the default material).
    pub sampled: Option<Option<MaterialId>>,
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
    /// What a click would act on now (Select, Push/Pull, Paint, Erase): an id of the open
    /// context; None for the other tools, while an operation is in progress, and until the
    /// cursor moves again after a click, Escape or an external change.
    fn hovered(&self) -> Option<EntityId> {
        None
    }
    /// Writes the settings this tool keeps for the session (such as a circle's side count)
    /// into `r`, so the tool made next time starts with them.
    fn remember(&self, _r: &mut Remembered) {}
}

/// Tool settings kept for the session, across switching tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remembered {
    /// Sides of the circles the Circle tool draws.
    pub circle_sides: u32,
    /// Segments of the arcs the Arc tool draws.
    pub arc_segments: u32,
}

impl Default for Remembered {
    fn default() -> Self {
        Remembered {
            circle_sides: DEFAULT_CIRCLE_SEGMENTS,
            arc_segments: DEFAULT_ARC_SEGMENTS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    PushPull,
    Move,
    Rotate,
    TapeMeasure,
    Erase,
    Paint,
}

impl ToolKind {
    /// Every tool, in toolbar order.
    pub const ALL: [ToolKind; 11] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::TapeMeasure,
        ToolKind::Erase,
        ToolKind::Paint,
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
            ToolKind::Circle => Box::new(CircleTool::with_sides(r.circle_sides)),
            ToolKind::Arc => Box::new(ArcTool::with_segments(r.arc_segments)),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
            ToolKind::Move => Box::new(MoveTool::new()),
            ToolKind::Rotate => Box::new(RotateTool::new()),
            ToolKind::TapeMeasure => Box::new(TapeMeasureTool::new()),
            ToolKind::Erase => Box::new(EraseTool::new()),
            ToolKind::Paint => Box::new(PaintTool::new()),
        }
    }

    /// The tool's name, as its `Tool::name` reports it.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Line => "Line",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::Circle => "Circle",
            ToolKind::Arc => "Arc",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Move => "Move",
            ToolKind::Rotate => "Rotate",
            ToolKind::TapeMeasure => "Tape Measure",
            ToolKind::Erase => "Erase",
            ToolKind::Paint => "Paint",
        }
    }

    /// The key that selects the tool.
    pub fn shortcut(self) -> char {
        match self {
            ToolKind::Select => ' ',
            ToolKind::Line => 'l',
            ToolKind::Rectangle => 'r',
            ToolKind::Circle => 'c',
            ToolKind::Arc => 'a',
            ToolKind::PushPull => 'p',
            ToolKind::Move => 'm',
            ToolKind::Rotate => 'q',
            ToolKind::TapeMeasure => 't',
            ToolKind::Erase => 'e',
            ToolKind::Paint => 'b',
        }
    }

    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        let c = c.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.shortcut() == c)
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
            assert_eq!(ToolKind::from_shortcut(kind.shortcut()), Some(kind));
        }
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
                shift: false,
                selection: &selection,
                material: None,
                sample: false,
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
            shift: false,
            selection: &BTreeSet::new(),
            material: None,
            sample: false,
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
        for kind in [ToolKind::Rectangle, ToolKind::Circle] {
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
