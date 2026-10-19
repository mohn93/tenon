use std::fmt;
use std::path::PathBuf;

use kernel::{ChangeSummary, DefinitionId, EntityId, MaterialId, Side, SoftenReport};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::{Lighting, Projection, StandardView};
use crate::export::{ExportFormat, ExportReport};
use crate::import::{ImportAs, ImportFormat, ImportReport, UpAxis};
use crate::schema::ModelPoint;

/// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
/// commands never refer to screen positions or cursor state. Points, offsets and entity ids are
/// those of the open context (see `OpenInstance`): the root, or the definition of the instance
/// being edited, in its own coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
    AddEdge {
        #[schemars(with = "ModelPoint")]
        start: [f64; 3],
        #[schemars(with = "ModelPoint")]
        end: [f64; 3],
    },
    /// Draw a rectangle from one corner to the opposite one, as one undo step. Its sides follow
    /// the world axes on axis planes. `normal` picks the plane; without it the corners must
    /// differ in exactly two coordinates.
    AddRectangle {
        #[schemars(with = "ModelPoint")]
        corner: [f64; 3],
        #[schemars(with = "ModelPoint")]
        opposite: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
    },
    /// Draw a circle as a regular polygon of `segments` sides (default 24, at most 360) around
    /// `center` with `radius`, in the plane with `normal` (default Z), as one undo step. The
    /// first corner lies along the plane's u axis (see `add_rectangle`).
    AddCircle {
        #[schemars(with = "ModelPoint")]
        center: [f64; 3],
        radius: f64,
        #[serde(default)]
        normal: Option<[f64; 3]>,
        #[serde(default)]
        segments: Option<u32>,
    },
    /// Draw a regular polygon of `sides` sides (default 6, 3 to 360) around `center`, in the
    /// plane with `normal` (default Z), as one undo step. Inscribed, `radius` reaches the
    /// corners and the first corner lies along `direction`; with `circumscribed`, `radius`
    /// reaches the middles of the sides and the middle of the side ending at the first corner
    /// lies along `direction`. `direction` (default the plane's u axis, see `add_rectangle`)
    /// is projected into the plane. Its edges form one curve marked as a polygon, so pulling
    /// it keeps the sides hard.
    AddPolygon {
        #[schemars(with = "ModelPoint")]
        center: [f64; 3],
        radius: f64,
        #[serde(default)]
        sides: Option<u32>,
        #[serde(default)]
        normal: Option<[f64; 3]>,
        #[serde(default)]
        circumscribed: bool,
        #[serde(default)]
        direction: Option<[f64; 3]>,
    },
    /// Draw an arc, as `segments` straight edges (default 12, 2 to 360) tagged as one curve,
    /// in one undo step, given in one of three ways: around `center` from `start`, turning
    /// `angle` degrees counter-clockwise around `normal` (default Z; negative angles turn
    /// clockwise; `start` must lie in the plane through `center` with that normal); from
    /// `start` to `end` passing through `through` (the three points set the plane; no
    /// `normal`); or from `start` to `end` with its middle `bulge` meters from the chord, to
    /// the left of start to end looking down `normal` (default Z; negative: to the right), as
    /// the Arc tool draws it.
    AddArc {
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        center: Option<[f64; 3]>,
        #[schemars(with = "ModelPoint")]
        start: [f64; 3],
        #[serde(default)]
        angle: Option<f64>,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        end: Option<[f64; 3]>,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        through: Option<[f64; 3]>,
        #[serde(default)]
        bulge: Option<f64>,
        #[serde(default)]
        normal: Option<[f64; 3]>,
        #[serde(default)]
        segments: Option<u32>,
    },
    /// Move a face along its normal by `distance` meters (negative: against the normal),
    /// extruding new sides or stretching the faces around it.
    PushPull { face: EntityId, distance: f64 },
    /// Add a guide line: the infinite line through `point` along `direction` (any non-zero
    /// length), as one undo step. Guides are construction geometry: tools snap to them, they
    /// bound no faces and are not exported.
    AddGuideLine {
        #[schemars(with = "ModelPoint")]
        point: [f64; 3],
        direction: [f64; 3],
    },
    /// Add a guide point at `position`, as one undo step.
    AddGuidePoint {
        #[schemars(with = "ModelPoint")]
        position: [f64; 3],
    },
    /// Erase every guide of the open context, as one undo step (nothing to erase is not an
    /// error and makes no undo step).
    DeleteGuides {},
    /// Erase edges, faces, instances and guides as one undo step. An erased edge takes the
    /// faces it bounds with it, except that two faces in one plane meeting there merge into
    /// one. Vertices go away with their last edge and cannot be erased directly. A group's
    /// definition goes with its last instance.
    Erase { ids: Vec<EntityId> },
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
        #[schemars(with = "ModelPoint")]
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
    MakeGroup { ids: Vec<EntityId> },
    /// Like `MakeGroup`, but the definition is a named component: copies of its instances
    /// share it, so editing one edits them all.
    MakeComponent { ids: Vec<EntityId>, name: String },
    /// Replace instances by their geometry, which merges with what it touches. Instances and
    /// guides inside them come out as instances and guides. What comes out becomes the selection.
    Explode { ids: Vec<EntityId> },
    /// Edit the definition an instance of the open context places: its entities become the
    /// ones commands and queries see, in its own coordinates. Clears the selection. Not
    /// recorded in undo history.
    OpenInstance { id: EntityId },
    /// Go back to editing the context the open instance is in. Clears the selection. Not
    /// recorded in undo history.
    CloseInstance {},
    /// Make `path` the open context: instances from the root, each placed in the definition
    /// of the one before (empty: the root), as the `outliner` query lists them. Clears the
    /// selection unless `path` is already open. Not recorded in undo history.
    OpenPath { path: Vec<EntityId> },
    /// Rename a group or component definition. A component's name must not be empty.
    RenameDefinition {
        definition: DefinitionId,
        name: String,
    },
    /// Remove every definition that no instance places (components left after their last
    /// instance was erased or exploded), and what only they placed.
    PurgeDefinitions {},
    /// Add a material: a named sRGB color (each 0 to 255). Without `name` it is called
    /// "Material" (or "Material 2", ... if taken). Names must be distinct, not blank, and at
    /// most 256 characters. `opacity` (default 1, opaque) is from 0 to 1; below 1 faces
    /// painted with it are see-through. `texture` reads a PNG or JPEG image for its faces
    /// (`path` required); `color` may then be left out and is the image's average color.
    /// `emission` (default 0) from 0 to 10 makes its faces glow and light their surroundings.
    /// The new id is in the outcome's `changes.materials`.
    AddMaterial {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<[u8; 3]>,
        #[serde(default)]
        opacity: Option<f32>,
        #[serde(default)]
        texture: Option<TextureSource>,
        #[serde(default)]
        emission: Option<f32>,
    },
    /// Rename, recolor, change the opacity (0 to 1), the texture and/or the emission (0 to 10)
    /// of a material; absent fields stay as they are. `texture`: null removes it; an object
    /// sets it, reading `path` (the color becomes the image's average unless `color` is
    /// given), or without `path` keeps the image and changes its size.
    ChangeMaterial {
        material: MaterialId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<[u8; 3]>,
        #[serde(default)]
        opacity: Option<f32>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(with = "Option<TextureSource>")]
        texture: Option<Option<TextureSource>>,
        #[serde(default)]
        emission: Option<f32>,
    },
    /// Remove a material. Faces and instances painted with it, anywhere in the model, go back
    /// to the default material.
    DeleteMaterial { material: MaterialId },
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
    /// Make edges of the open context soft (`soft`: true) or hard again, as one undo step: the
    /// edges in `ids` and the edges around the faces in `ids`. A soft edge is hidden and the
    /// faces on its sides are shaded as one smooth surface. Only an edge between exactly two
    /// faces can be soft; others are left as they are and counted in the outcome's `soften`.
    SoftenEdges { ids: Vec<EntityId>, soft: bool },
    /// Soften each edge between two faces whose normals differ by at most `max_angle_deg`
    /// (0 to 180), as one undo step: among the edges in `ids` and the edges around the faces
    /// in `ids`, or among every edge of the open context when `ids` is empty or absent. Other
    /// edges stay as they are. The outcome's `soften` counts what changed.
    SoftenByAngle {
        #[serde(default)]
        ids: Vec<EntityId>,
        max_angle_deg: f64,
    },
    /// Move, turn and scale the texture on faces of the open context, as one undo step: on
    /// top of the projection, `offset` [u, v] in meters, `rotation_deg` counter-clockwise
    /// seen from the face's front (or `rotate_by`: that many degrees more than each face's
    /// own), and `scale` [u, v] (multipliers, 0.001 to 1000). Fields left out stay as they
    /// are; `reset: true` first clears the placement. The outcome's `positioned` counts the
    /// faces that changed.
    PositionTexture {
        ids: Vec<EntityId>,
        #[serde(default)]
        offset: Option<[f64; 2]>,
        #[serde(default)]
        rotation_deg: Option<f64>,
        #[serde(default)]
        rotate_by: Option<f64>,
        #[serde(default)]
        scale: Option<[f64; 2]>,
        #[serde(default)]
        reset: bool,
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
    /// saved when `path` is absent. Not an undo step. An existing file there that is not a
    /// model (an empty file is fine) is refused unless `overwrite` is true.
    Save {
        #[serde(default)]
        path: Option<String>,
        /// Replace an existing file even when it is not a `.skr` model (without it such a
        /// file is refused; an empty or missing file is always fine).
        #[serde(default)]
        overwrite: bool,
    },
    /// Write every face of the model (all instances placed, in root coordinates) as triangles
    /// to `path`. `format` defaults to the one the file name's extension names (`.obj`,
    /// `.stl`, `.glb`, `.gltf`). Not an undo step. The outcome's `export` counts the triangles
    /// written and the faces left out because they could not be triangulated. An existing
    /// file that is not of the format written is refused unless `overwrite` is true.
    Export {
        path: String,
        #[serde(default)]
        format: Option<ExportFormat>,
        /// Replace an existing file even when it is not of the format written; for an OBJ,
        /// also its `.mtl` and images.
        #[serde(default)]
        overwrite: bool,
        /// Set by the API under `--api-root`: the file, and the `.mtl` and images an OBJ
        /// writes beside it, must not be symbolic links. Never read from or written to JSON.
        #[serde(skip)]
        refuse_links: bool,
    },
    /// Read an OBJ or STL file and add its faces to the open context, as one undo step: as a
    /// new group (default) or component named after the file, placed at the context's origin
    /// and selected, or (`"as": "context"`) as loose geometry merging with what it touches,
    /// which becomes the selection. `format` defaults to the one the extension names (`.obj`,
    /// `.stl`). File units are meters times `scale` (default 1); `up` is the file's up axis
    /// (default `y` for OBJ, `z` for STL, as `export` writes them). Corners closer than
    /// 1e-6 m are welded, and with `merge_coplanar` (default true) neighbouring faces in one
    /// plane with one material become one face. OBJ materials come from its `mtllib`
    /// libraries. The outcome's `import` says what was made.
    Import {
        path: String,
        #[serde(default)]
        format: Option<ImportFormat>,
        #[serde(default, rename = "as")]
        place: ImportAs,
        #[serde(default)]
        scale: Option<f64>,
        #[serde(default)]
        up: Option<UpAxis>,
        #[serde(default)]
        merge_coplanar: Option<bool>,
        /// Set by the API under `--api-root`: the OBJ's material libraries are read only when
        /// they lie, after symlinks, within these folders. Never read from or written to JSON.
        #[serde(skip)]
        library_roots: Option<Vec<PathBuf>>,
    },
    /// Change the selection of edges, faces, instances and guides. Not recorded in undo history.
    Select {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        mode: SelectMode,
        /// Everything in the open context (edges, faces, instances and guides) in place of
        /// `ids`, combined by `mode` the same way.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        all: bool,
    },
    /// Undo the last `count` steps (default 1), each its own revision. With fewer steps to
    /// undo than `count`, fails (`nothing_to_undo`) and undoes nothing. With `expect`, fails
    /// (`unexpected_step`) unless the next step to undo matches it.
    Undo {
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        expect: Option<ExpectedStep>,
    },
    /// Redo the last `count` undone steps (default 1), like `undo`.
    Redo {
        #[serde(default)]
        count: Option<u32>,
    },
    /// Place the camera: from `eye` looking at `target`, with the vertical field of view
    /// `fov_y_deg` (1 to 170; default: unchanged) and `projection` (default: unchanged). A
    /// view straight down or up takes the screen's up direction from the horizontal part of
    /// `up` (default +Y); any other view has +Z up and ignores `up`. Camera commands are not
    /// recorded in undo history and do not change the model's revision.
    SetCamera {
        #[schemars(with = "ModelPoint")]
        eye: [f64; 3],
        #[schemars(with = "ModelPoint")]
        target: [f64; 3],
        fov_y_deg: Option<f64>,
        #[serde(default)]
        up: Option<[f64; 3]>,
        #[serde(default)]
        projection: Option<Projection>,
    },
    /// Turn the camera about its target, or about the root point `about` (as dragging with the
    /// middle button turns it about the point under the cursor): `yaw` degrees about the
    /// vertical (counter-clockwise seen from above), then `pitch` degrees raising the eye
    /// (negative lowers it). The view stops 0.01 rad short of straight down or up.
    Orbit {
        #[serde(default)]
        yaw: f64,
        #[serde(default)]
        pitch: f64,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        about: Option<[f64; 3]>,
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
    /// the same way. With `about` (a root point), zoom toward it as the scroll wheel zooms
    /// toward the point under the cursor: it stays where it is on screen and, in perspective,
    /// the target moves to its depth. The eye comes to `1 / factor` of its distance from the
    /// point only in perspective with the point ahead of the eye and above the 0.01 m floor; in
    /// a parallel view the view height shrinks by the factor instead, and at the floor or with
    /// the point behind the eye the eye moves on past it, as the wheel does.
    Zoom {
        factor: f64,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        about: Option<[f64; 3]>,
    },
    /// Keep the view direction and fit the whole model (every vertex and placed instance, in
    /// root coordinates, whatever instance is open) in the view. Guides do not count. With
    /// nothing to fit, the camera stays.
    ZoomExtents {},
    /// Look at the target from a standard direction, from the same distance.
    StandardView { view: StandardView },
    /// Switch between perspective and parallel projection. The view height at the target
    /// stays the same.
    SetProjection { projection: Projection },
    /// Light the view by `day` (a headlight and ambient light, glowing faces adding theirs) or
    /// by `night` (both dimmed, a dark background: glowing faces light the scene). A view
    /// setting like the camera's: saved with the model, but not an undo step and not a change
    /// to the revision.
    SetLighting { mode: Lighting },
}

impl Command {
    /// The command's `type` as agents write it ("add_edge", "push_pull"): the label of the undo
    /// step it makes.
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
            .unwrap_or_default()
    }

    /// True for commands that can change the model or the open context (everything except
    /// camera, selection, saving and exporting).
    pub fn changes_model(&self) -> bool {
        !(self.is_camera()
            || matches!(
                self,
                Command::Select { .. } | Command::Save { .. } | Command::Export { .. }
            ))
    }

    /// True for commands that only change the view: move the camera or set the lighting.
    pub fn is_camera(&self) -> bool {
        matches!(
            self,
            Command::SetCamera { .. }
                | Command::Orbit { .. }
                | Command::Pan { .. }
                | Command::Zoom { .. }
                | Command::ZoomExtents {}
                | Command::StandardView { .. }
                | Command::SetProjection { .. }
                | Command::SetLighting { .. }
        )
    }
}

/// A texture for `add_material` or `change_material`: an image file and the size one copy of
/// it covers on the model, in meters (from 0.001 to 1000). A new image without `width` is 1 m
/// wide (or as wide as the texture it replaces), and without `height` keeps the image's
/// proportions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextureSource {
    /// A PNG or JPEG file of at most 16 MiB and 4096 pixels on each side.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
}

/// A field that may be absent (None), null (`Some(None)`) or a value.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
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
    /// What an `import` read and made; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportReport>,
    /// What an `export` wrote; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportReport>,
    /// What a `soften_edges` or `soften_by_angle` changed and skipped; absent for other
    /// commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soften: Option<SoftenReport>,
    /// How many faces a `position_texture` changed; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positioned: Option<usize>,
    /// How many definitions a `purge_definitions` removed (nested ones included); absent for
    /// other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged: Option<usize>,
}

/// Who made an undo step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Made in the window.
    #[default]
    User,
    /// Made through the API.
    Agent,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::User => "user",
            Origin::Agent => "agent",
        }
    }
}

/// The step an `undo` expects to take back: its label, and optionally who made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpectedStep {
    pub label: String,
    #[serde(default)]
    pub origin: Option<Origin>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    NothingToUndo,
    NothingToRedo,
    /// `undo` with `expect`: the next step to undo is not the expected one.
    UnexpectedStep,
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
