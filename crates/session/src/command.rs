use std::fmt;

use kernel::{ChangeSummary, DefinitionId, EntityId, MaterialId, Side};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::{Projection, StandardView};
use crate::export::ExportFormat;

/// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
/// commands never refer to screen positions or cursor state. Points, offsets and entity ids are
/// those of the open context (see `OpenInstance`): the root, or the definition of the instance
/// being edited, in its own coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
    AddEdge {
        start: [f64; 3],
        end: [f64; 3],
    },
    /// Draw a rectangle from one corner to the opposite one, as one undo step. Its sides follow
    /// the world axes on axis planes. `normal` picks the plane; without it the corners must
    /// differ in exactly two coordinates.
    AddRectangle {
        corner: [f64; 3],
        opposite: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
    },
    /// Draw a circle as a regular polygon of `segments` sides (default 24, at most 360) around
    /// `center` with `radius`, in the plane with `normal` (default Z), as one undo step. The
    /// first corner lies along the plane's u axis (see `add_rectangle`).
    AddCircle {
        center: [f64; 3],
        radius: f64,
        #[serde(default)]
        normal: Option<[f64; 3]>,
        #[serde(default)]
        segments: Option<u32>,
    },
    /// Draw an arc around `center` from `start`, turning `angle` degrees counter-clockwise
    /// around `normal` (default Z; negative angles turn clockwise), as `segments` straight
    /// edges (default 12, 2 to 360) tagged as one curve, in one undo step. `start` must lie in
    /// the plane through `center` with that normal.
    AddArc {
        center: [f64; 3],
        start: [f64; 3],
        angle: f64,
        #[serde(default)]
        normal: Option<[f64; 3]>,
        #[serde(default)]
        segments: Option<u32>,
    },
    /// Move a face along its normal by `distance` meters (negative: against the normal),
    /// extruding new sides or stretching the faces around it.
    PushPull {
        face: EntityId,
        distance: f64,
    },
    /// Add a guide line: the infinite line through `point` along `direction` (any non-zero
    /// length), as one undo step. Guides are construction geometry: tools snap to them, they
    /// bound no faces and are not exported.
    AddGuideLine {
        point: [f64; 3],
        direction: [f64; 3],
    },
    /// Add a guide point at `position`, as one undo step.
    AddGuidePoint {
        position: [f64; 3],
    },
    /// Erase every guide of the open context, as one undo step (nothing to erase is not an
    /// error and makes no undo step).
    DeleteGuides,
    /// Erase edges, faces, instances and guides as one undo step. An erased edge takes the
    /// faces it bounds with it, except that two faces in one plane meeting there merge into
    /// one. Vertices go away with their last edge and cannot be erased directly. A group's
    /// definition goes with its last instance.
    Erase {
        ids: Vec<EntityId>,
    },
    /// Move vertices, edges, faces, instances and guides by `offset` meters as one undo step.
    /// Faces that share some of the moved vertices stretch and may tilt, but must stay flat.
    Move {
        ids: Vec<EntityId>,
        offset: [f64; 3],
    },
    /// Copy edges, faces, instances and guides `count` times (default 1), copy k shifted by
    /// `k * offset`, as one undo step. Copies merge with geometry they touch; copied faces keep
    /// their orientation. A copied component instance places the same definition; a copied
    /// group gets a definition of its own.
    Copy {
        ids: Vec<EntityId>,
        offset: [f64; 3],
        #[serde(default)]
        count: Option<u32>,
    },
    /// Rotate vertices, edges, faces (their vertices), instances and guides by `angle` degrees
    /// about the line through `center` along `axis` (default Z), counter-clockwise looking
    /// down the axis, as one undo step. Faces that share some of the turned vertices stretch
    /// and may tilt, but must stay flat. With `copies`, the originals stay and that many copies
    /// are made instead, copy k turned by `k * angle`, under the rules of `Copy`.
    Rotate {
        ids: Vec<EntityId>,
        center: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        angle: f64,
        #[serde(default)]
        copies: Option<u32>,
    },
    /// Turn edges, faces (with their edges), instances and guides into a group: they move into
    /// a new definition, placed by one new instance where they were. Guides alone cannot be
    /// grouped. The instance becomes the selection.
    MakeGroup {
        ids: Vec<EntityId>,
    },
    /// Like `MakeGroup`, but the definition is a named component: copies of its instances
    /// share it, so editing one edits them all.
    MakeComponent {
        ids: Vec<EntityId>,
        name: String,
    },
    /// Replace instances by their geometry, which merges with what it touches. Instances and
    /// guides inside them come out as instances and guides. What comes out becomes the selection.
    Explode {
        ids: Vec<EntityId>,
    },
    /// Edit the definition an instance of the open context places: its entities become the
    /// ones commands and queries see, in its own coordinates. Clears the selection. Not
    /// recorded in undo history.
    OpenInstance {
        id: EntityId,
    },
    /// Go back to editing the context the open instance is in. Clears the selection. Not
    /// recorded in undo history.
    CloseInstance,
    /// Make `path` the open context: instances from the root, each placed in the definition
    /// of the one before (empty: the root), as the `outliner` query lists them. Clears the
    /// selection unless `path` is already open. Not recorded in undo history.
    OpenPath {
        path: Vec<EntityId>,
    },
    /// Rename a group or component definition. A component's name must not be empty.
    RenameDefinition {
        definition: DefinitionId,
        name: String,
    },
    /// Remove every definition that no instance places (components left after their last
    /// instance was erased or exploded), and what only they placed.
    PurgeDefinitions,
    /// Add a material: a named sRGB color (each 0 to 255). Without `name` it is called
    /// "Material" (or "Material 2", ... if taken). Names must be distinct, not blank, and at
    /// most 256 characters. The new id is in the outcome's `changes.materials`.
    AddMaterial {
        #[serde(default)]
        name: Option<String>,
        color: [u8; 3],
    },
    /// Rename and/or recolor a material; absent fields stay as they are.
    ChangeMaterial {
        material: MaterialId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<[u8; 3]>,
    },
    /// Remove a material. Faces and instances painted with it, anywhere in the model, go back
    /// to the default material.
    DeleteMaterial {
        material: MaterialId,
    },
    /// Paint faces and instances of the open context with `material` (absent or null: the
    /// default material), as one undo step. On faces `side` (default `front`) picks the side
    /// the normal points to, the other one, or both. An instance's material shows on the face
    /// sides inside it (at any depth) that have no material of their own.
    Paint {
        ids: Vec<EntityId>,
        #[serde(default)]
        material: Option<MaterialId>,
        #[serde(default)]
        side: Side,
    },
    /// Start an empty model with the default view. Clears undo history; not an undo step.
    /// With changes not saved, fails (`unsaved_changes`) unless `discard` is true.
    New {
        #[serde(default)]
        discard: bool,
    },
    /// Replace the model and view with those saved in a `.skr` file. Clears undo history;
    /// not an undo step. With changes not saved, fails (`unsaved_changes`) unless `discard`
    /// is true.
    Open {
        path: String,
        #[serde(default)]
        discard: bool,
    },
    /// Save the model and view as a `.skr` file at `path`, or at the path last opened or
    /// saved when `path` is absent. Not an undo step.
    Save {
        #[serde(default)]
        path: Option<String>,
    },
    /// Write every face of the model (all instances placed, in root coordinates) as triangles
    /// to `path`. `format` defaults to the one the file name's extension names (`.obj`,
    /// `.stl`, `.glb`, `.gltf`). Not an undo step.
    Export {
        path: String,
        #[serde(default)]
        format: Option<ExportFormat>,
    },
    /// Change the selection of edges, faces, instances and guides. Not recorded in undo history.
    Select {
        ids: Vec<EntityId>,
        #[serde(default)]
        mode: SelectMode,
    },
    Undo,
    Redo,
    /// Place the camera: from `eye` looking at `target`, with the vertical field of view
    /// `fov_y_deg` (1 to 170; default: unchanged) and `projection` (default: unchanged). A
    /// view straight down or up takes the screen's up direction from the horizontal part of
    /// `up` (default +Y); any other view has +Z up and ignores `up`. Camera commands are not
    /// recorded in undo history and do not change the model's revision.
    SetCamera {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: Option<f64>,
        #[serde(default)]
        up: Option<[f64; 3]>,
        #[serde(default)]
        projection: Option<Projection>,
    },
    /// Turn the camera about its target: `yaw` degrees about the vertical (counter-clockwise
    /// seen from above), then `pitch` degrees raising the eye (negative lowers it). The view
    /// stops 0.01 rad short of straight down or up.
    Orbit {
        #[serde(default)]
        yaw: f64,
        #[serde(default)]
        pitch: f64,
    },
    /// Move the eye and target together, `right` and `up` meters along the screen's right
    /// and up directions.
    Pan {
        #[serde(default)]
        right: f64,
        #[serde(default)]
        up: f64,
    },
    /// Move the eye toward the target to `1 / factor` of its distance (factor above 1 zooms
    /// in, below 1 out; never nearer than 0.01 m). In a parallel view the view height shrinks
    /// the same way.
    Zoom {
        factor: f64,
    },
    /// Keep the view direction and fit the whole model (every vertex and placed instance, in
    /// root coordinates, whatever instance is open) in the view. Guides do not count. With
    /// nothing to fit, the camera stays.
    ZoomExtents,
    /// Look at the target from a standard direction, from the same distance.
    StandardView {
        view: StandardView,
    },
    /// Switch between perspective and parallel projection. The view height at the target
    /// stays the same.
    SetProjection {
        projection: Projection,
    },
}

impl Command {
    /// True for commands that can change the model or the open context (everything except
    /// camera, selection, saving and exporting).
    pub fn changes_model(&self) -> bool {
        !(self.is_camera()
            || matches!(
                self,
                Command::Select { .. } | Command::Save { .. } | Command::Export { .. }
            ))
    }

    /// True for commands that only move the camera.
    pub fn is_camera(&self) -> bool {
        matches!(
            self,
            Command::SetCamera { .. }
                | Command::Orbit { .. }
                | Command::Pan { .. }
                | Command::Zoom { .. }
                | Command::ZoomExtents
                | Command::StandardView { .. }
                | Command::SetProjection { .. }
        )
    }
}

/// How `Command::Select` combines `ids` with the current selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SelectMode {
    /// The selection becomes exactly `ids`.
    #[default]
    Replace,
    Add,
    Remove,
    /// Selected ids are removed, the others added.
    Toggle,
}

/// What a successful command changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outcome {
    pub changes: ChangeSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    NothingToUndo,
    NothingToRedo,
    NotFound,
    /// A file could not be read or written; `message` says why.
    File,
    /// `new` or `open` would drop changes not saved; pass `"discard": true` to drop them.
    UnsavedChanges,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
    /// Entities the error refers to, when there are any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<EntityId>,
}

impl CommandError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            ids: Vec::new(),
        }
    }

    pub fn with_ids(mut self, ids: Vec<EntityId>) -> Self {
        self.ids = ids;
        self
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CommandError {}

/// A batch stopped at command `index`. `completed` holds the outcomes of the commands before it;
/// for an atomic batch they were rolled back and nothing changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BatchError {
    pub index: usize,
    pub error: CommandError,
    pub completed: Vec<Outcome>,
    pub rolled_back: bool,
}
