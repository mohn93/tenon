use std::fmt;
use std::path::PathBuf;

use kernel::{
    ChangeSummary, DefinitionId, EntityId, MaterialId, PageId, Side, SoftenReport, TagId,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::{Lighting, Projection, StandardView};
use crate::export::{ExportFormat, ExportReport};
use crate::import::{ImportAs, ImportFormat, ImportReport, UpAxis};
use crate::library::{LibraryKind, LibraryRef};
use crate::schema::ModelPoint;
use crate::text3d::Text3dReport;
use crate::units::{LengthUnit, Units};

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
    /// bound no faces and are only exported to SketchUp files.
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
    /// Add a linear dimension measuring the distance between `a` and `b`, as one undo step.
    /// Each end is a vertex id ("v3"), which the dimension follows, or a point. The dimension
    /// line is drawn from `a` to `b` moved by `offset`, of which only the part perpendicular
    /// to a-b is kept. Ends that coincide are refused.
    AddDimension {
        a: AnchorInput,
        b: AnchorInput,
        offset: [f64; 3],
    },
    /// Add a linear dimension between the two vertices of `edge`, as one undo step.
    AddEdgeDimension { edge: EntityId, offset: [f64; 3] },
    /// Add a radial dimension to the circle or arc `edge` (any of its edges) is part of, as
    /// one undo step: its radius, or with `diameter` its diameter, labelled at `leader` from
    /// the center. It follows the curve while the curve stays a circle or an arc.
    AddRadialDimension {
        edge: EntityId,
        #[serde(default)]
        diameter: bool,
        leader: [f64; 3],
    },
    /// Add a leader note, as one undo step: `text` at `position` with a line to `anchor` (a
    /// vertex id, which it follows, or a point). Without `text` the note shows the anchored
    /// point's coordinates in the display units. Text has 1 to 1024 characters, is not blank
    /// and has at most 64 lines.
    AddNote {
        anchor: AnchorInput,
        #[schemars(with = "ModelPoint")]
        position: [f64; 3],
        #[serde(default)]
        text: Option<String>,
    },
    /// Add a note fixed to the view, as one undo step: `text` at `at`, fractions (0 to 1) of
    /// the view's width and height from its top left corner. Only the root holds screen notes:
    /// refused while an instance is open.
    AddScreenNote { at: [f64; 2], text: String },
    /// Change a note's text, as one undo step. Absent or null `text` gives a leader note its
    /// automatic text (the anchored point's coordinates); a screen note needs text.
    ChangeNote {
        note: EntityId,
        #[serde(default)]
        text: Option<String>,
    },
    /// Add 3D text, as one undo step: the letters of `text` in Inter Regular, as faces with
    /// holes in a new group named "3D Text: " and the text's start, placed at `position` and
    /// selected. The text reads along +X in the open context's XY plane, the first line's
    /// baseline through `position`, each further line `1.2 * height` lower; capital letters are
    /// `height` meters tall (at least 0.001). With `depth` (default 0, flat) above 0 each letter
    /// is pushed up by it into a solid, as `push_pull` does. With `filled` false (default true)
    /// only the letters' outline edges are drawn, flat (`depth` must be 0). Curves become at
    /// most 12 straight edges each, within 1% of the height. Text has 1 to 1024 characters, is
    /// not blank and has at most 64 lines; the whole text must lie within the coordinate limit.
    /// Characters the font has no glyph for, and letters whose outlines cross, are left out and
    /// listed in the outcome's `text_3d.skipped`; spaces only move the next letter on.
    #[serde(rename = "add_3d_text")]
    Add3dText {
        text: String,
        #[schemars(with = "ModelPoint")]
        position: [f64; 3],
        height: f64,
        #[serde(default)]
        depth: f64,
        #[serde(default = "yes")]
        filled: bool,
    },
    /// Add a section plane to the open context, as one undo step: the plane through `point`
    /// with `normal` (any non-zero length; it is normalized), which keeps the side the normal
    /// points to. With `activate` (default true) it becomes the context's active plane, which
    /// cuts the context's geometry and what is inside its instances (a plane in a group cuts
    /// only that group). The outcome's `changes.created` names it ("s5"). A zero or non-finite
    /// normal, or a point beyond the coordinate limit, is `invalid_input`.
    AddSectionPlane {
        #[schemars(with = "ModelPoint")]
        point: [f64; 3],
        normal: [f64; 3],
        #[serde(default)]
        activate: Option<bool>,
    },
    /// Negate a section plane's normal (the other side is kept), as one undo step. The point
    /// and whether it is active stay. A plane not in the open context is `invalid_input`.
    FlipSectionPlane { plane: EntityId },
    /// Make a section plane the open context's active plane, as one undo step (the one that
    /// was active no longer is; already active changes nothing). A plane not in the open
    /// context is `invalid_input`.
    ActivateSectionPlane { plane: EntityId },
    /// Make no section plane of the open context active, as one undo step (none active
    /// changes nothing and makes no undo step). The planes stay.
    DeactivateSection {},
    /// Set how section planes are shown; absent fields stay as they are: `planes` draws the
    /// plane entities, `cuts` applies the active planes' cuts, `fill` fills closed cuts. All
    /// default to true. A view setting like `set_lighting`: saved with the model, but not an
    /// undo step and not a change to the revision.
    SetSectionDisplay {
        #[serde(default)]
        planes: Option<bool>,
        #[serde(default)]
        cuts: Option<bool>,
        #[serde(default)]
        fill: Option<bool>,
    },
    /// Set the model's display units, which dimension and note labels are written in, as one
    /// undo step; absent fields stay as they are. `precision` is the decimals kept (0 to 6),
    /// or for `feet_inches` the finest fraction of an inch, 1/2^precision (6: 1/64). Saved
    /// with the model.
    SetUnits {
        #[serde(default)]
        length: Option<LengthUnit>,
        #[serde(default)]
        precision: Option<u8>,
    },
    /// Erase edges, faces, instances, guides, dimensions, notes and section planes as one undo
    /// step. An erased edge
    /// takes the faces it bounds with it, except that two faces in one plane meeting there merge
    /// into one. Vertices go away with their last edge and cannot be erased directly. A group's
    /// definition goes with its last instance.
    Erase { ids: Vec<EntityId> },
    /// Move vertices, edges, faces, instances, guides, dimensions, notes and section planes by
    /// `offset` meters
    /// as one undo step. Faces that share some of the moved vertices stretch and may tilt, but
    /// must stay flat. An annotation's fixed points and offsets move; its vertex anchors move
    /// with their vertices; a dimension whose two ends the move brings together is removed.
    Move {
        ids: Vec<EntityId>,
        offset: [f64; 3],
    },
    /// Copy edges, faces, instances, guides, dimensions, notes and section planes (copies are
    /// inactive) `count` times (default 1), copy k
    /// shifted by `k * offset`, as one undo step. A copied annotation anchored to a copied vertex
    /// anchors to that vertex's copy; its other vertex anchors become points. Copies merge with
    /// geometry they touch; copied faces keep their orientation. A copied component instance places
    /// the same definition; a copied group gets a definition of its own.
    Copy {
        ids: Vec<EntityId>,
        offset: [f64; 3],
        #[serde(default)]
        count: Option<u32>,
    },
    /// Rotate vertices, edges, faces (their vertices), instances, guides, annotations and section
    /// planes (as `move` moves them) by `angle` degrees
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
    /// Scale vertices, edges, faces (their vertices), instances, guides, annotations and section
    /// planes (as `move` moves them; a fixed circle keeps its area) by `factors` along
    /// model x, y and z about `origin`, as one undo step. Each factor is finite, non-zero and
    /// between 1e-4 and 1e4 in size; a negative factor mirrors (faces stay facing out). Faces
    /// that share some of the scaled vertices stretch and may tilt, but must stay flat.
    Scale {
        ids: Vec<EntityId>,
        #[schemars(with = "ModelPoint")]
        origin: [f64; 3],
        factors: [f64; 3],
    },
    /// Draw a copy of a face's loops, or of a chain of edges, moved `distance` meters sideways
    /// in its plane, as one undo step. `ids` is one face (`distance` > 0 offsets inward: the
    /// outer loop shrinks and the holes grow; < 0 outward: the outer loop only, the holes stay),
    /// or edges forming one unbranched chain in one plane (open or closed), offset to the side
    /// of `toward` (a point in their plane, off the chain; required for edges, not allowed for a
    /// face) by a positive `distance`; such an offset may touch or cross the edges, which are
    /// then split as by a hand drawing. Corners are mitred, very sharp ones bevelled. A loop or
    /// chain whose edges are all on one curve gives a new curve of the same kind (an arc's ends
    /// move along its radii).
    Offset {
        ids: Vec<EntityId>,
        distance: f64,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        toward: Option<[f64; 3]>,
    },
    /// Sweep face `face` (the profile) along `path`, edges forming one connected chain (open or
    /// closed, without branches), as one undo step. The sweep starts at the path end nearest the
    /// face's plane (closed: the nearest path vertex) and carries the face as it is, so a face
    /// beside a circle sweeps around it. Corners are mitred; an open path is capped at both
    /// ends. The face itself is removed. Edges between the faces of two profile sides, or of two
    /// path segments, on one curve that is not a polygon are soft. `path` may not include the
    /// face's own edges unless it is exactly the outer loop of another face.
    FollowMe { face: EntityId, path: Vec<EntityId> },
    /// Turn edges, faces (with their edges), instances, guides, annotations and section planes
    /// into a group:
    /// they move into a new definition, placed by one new instance where they were. Guides,
    /// annotations and section planes alone cannot be grouped; screen notes stay in the root. The instance
    /// becomes the selection.
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
    /// Add a tag, shown. Without `name` it is called "Tag" (or "Tag 2", ... if taken); names
    /// must be distinct, not blank, not "Untagged" (which is having no tag) and at most 256
    /// characters. Without `color` (sRGB, each 0 to 255) it takes the first of twelve palette
    /// colors no tag has. The new id is in the outcome's `changes.tags`.
    AddTag {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<[u8; 3]>,
    },
    /// Rename, recolor, show (`visible`: true) or hide a tag; absent fields stay as they are.
    /// One undo step, a visibility change included.
    ChangeTag {
        tag: TagId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<[u8; 3]>,
        #[serde(default)]
        visible: Option<bool>,
    },
    /// Remove a tag. What is on it, anywhere in the model, moves to `move_to` (absent or null:
    /// Untagged), in the same undo step.
    DeleteTag {
        tag: TagId,
        #[serde(default)]
        move_to: Option<TagId>,
    },
    /// Put edges, faces, instances, guides, dimensions and notes of the open context on `tag`
    /// (absent or null: Untagged), as one undo step. Vertices cannot be tagged.
    SetTag {
        ids: Vec<EntityId>,
        #[serde(default)]
        tag: Option<TagId>,
    },
    /// Save a scene after the last one, as one undo step: the current view (camera,
    /// projection, lighting and color by tag) unless `camera` is false, and which tags are
    /// hidden unless `tags` is false. Without `name` it is called "Scene 1" (or the lowest
    /// number no scene has); names must be distinct, not blank and at most 256 characters.
    /// The new scene is the selected one; its id is in the outcome's `changes.scenes`.
    AddScene {
        #[serde(default)]
        name: Option<String>,
        #[serde(default = "yes")]
        camera: bool,
        #[serde(default = "yes")]
        tags: bool,
    },
    /// Store the current view (`camera`) and hidden tags (`tags`) in a scene again, as one undo
    /// step; false stops storing that part. Absent parts are stored again if the scene stores
    /// them.
    UpdateScene {
        scene: PageId,
        #[serde(default)]
        camera: Option<bool>,
        #[serde(default)]
        tags: Option<bool>,
    },
    /// Rename a scene and choose what it stores, as one undo step (none when nothing changes).
    /// At least one of `name`, `camera` and `tags` is needed. `camera` or `tags` false stops
    /// storing that part; true stores it from the current view or tag visibility only if the
    /// scene does not store it yet. A part that stays stored keeps exactly what was stored.
    ChangeScene {
        scene: PageId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        camera: Option<bool>,
        #[serde(default)]
        tags: Option<bool>,
    },
    /// Move a scene to `index` in the order (0: first), as one undo step. An index past the
    /// last scene is invalid input.
    MoveScene { scene: PageId, index: usize },
    /// Remove a scene, as one undo step.
    DeleteScene { scene: PageId },
    /// Go to a scene: set the camera, projection, lighting and color by tag it stores (a view
    /// change like `set_camera`, not an undo step), and show and hide tags as it stores (one
    /// undo step, only when that changes a tag). It becomes the selected scene.
    GoToScene { scene: PageId },
    /// Hide edges, faces, instances and guides of the open context, as one undo step (an
    /// entity already hidden is ignored; none to hide is a success with no undo step).
    /// A hidden entity is not drawn, picked or snapped to, and leaves the selection.
    /// Hiding a face leaves its edges and hiding an edge leaves its faces; hiding an
    /// instance hides everything inside it. The outcome's `changed` lists what was hidden.
    Hide { ids: Vec<EntityId> },
    /// Show the hidden entities in `ids`, as one undo step. The outcome's `changed` lists
    /// what was shown.
    Unhide { ids: Vec<EntityId> },
    /// Show what the most recent `hide` of this session hid that is in the open context and
    /// still hidden, as one undo step. Not saved: forgotten on `new` and `open`.
    UnhideLast {},
    /// Show every hidden entity of the open context (not those inside its instances), as one
    /// undo step.
    UnhideAll {},
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
    /// Draw the edges where the faces in `ids` (or, when empty, the selection) cross other
    /// faces, as one undo step labelled `intersect_faces`: faces of selected groups and
    /// components count, through their placements. `with` picks what they cross: `model`
    /// (default) every shown face of the model, `selection` only each other, `context` the
    /// faces of the open context's own mesh. New edges go into the open context, and split
    /// the faces of the open context they cross; groups and components are never modified.
    /// No crossings is a success with an empty `intersect` report and no undo step.
    IntersectFaces {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        with: IntersectScope,
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
    /// is true. A SketchUp file (`.skp`, or a file whose header is SketchUp's) opens as a new
    /// model never saved: its geometry and instances in the root, the view zoomed to them,
    /// and the outcome's `import` report; `save` then needs a path.
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
    /// Save a library item to the `.skr` file `path` (named `<name>.skr`): `kind` `component`
    /// saves the component instance `item` (a group is refused: make it a component first)
    /// at the origin with its rotation and scale kept, with the materials and tags it uses;
    /// `material` saves the material `item` alone, texture included; `model` saves the whole
    /// model (no `item`). `name` is what a library lists it by (default: the component's or
    /// material's name, or the file's). An existing file is replaced only with `overwrite`; the
    /// open model's own file is never written. Not an undo step, and the model is unchanged.
    SaveToLibrary {
        path: String,
        kind: LibraryKind,
        #[serde(default)]
        item: Option<LibraryRef>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        overwrite: bool,
    },
    /// Insert the library item at `path` (see `save_to_library`; any `.skr` file is a model
    /// item) into the open context, as one undo step. A component or model: new definitions
    /// are always made, materials and tags are matched by name (missing ones are added),
    /// the minimum corner of its bounding box lands at `at`, or at the open context's origin
    /// without it, and what it made becomes the selection. A material: added to the model;
    /// one with the same name and the same color, opacity, emission and texture is reused,
    /// otherwise the new one is named "<name> 2", "<name> 3"... (the lowest free). The
    /// outcome's `material` is the material's id.
    InsertLibraryItem {
        path: String,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        at: Option<[f64; 3]>,
    },
    /// Write every face of the model (all instances placed, in root coordinates) as triangles
    /// to `path`. `format` defaults to the one the file name's extension names (`.obj`,
    /// `.stl`, `.glb`, `.gltf`, `.skp`, `.svg`). Not an undo step. The outcome's `export` counts the
    /// triangles written and the faces left out because they could not be triangulated. A
    /// SketchUp 2017 file (`skp`) holds the whole scene instead: every definition the root
    /// places written once, faces with holes, soft edges, guides and materials with their
    /// textures; its `export` counts definitions, instances, faces, edges, guides and
    /// materials, with `warnings` for what the file cannot hold (glow, curves, empty groups).
    /// No format holds dimensions or notes (SVG draws them): `warnings` counts those left out.
    /// An SVG (`.svg`, or `format` `svg`) is a line drawing of the current view (the document's
    /// camera and viewport): visible edges only (soft edges, hidden tags and what section cuts
    /// remove are left out), section cuts with their fill and outlines, and the labels and
    /// lines of dimensions and notes; its `export` counts the polylines drawn in `lines`.
    /// An existing file that is not of the format written, or the SketchUp file the model
    /// was opened from, is refused unless `overwrite` is true.
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
        /// SVG only: export at 1:`scale` (a drawing in millimetres, 1 m of the model being
        /// 1000 / `scale` mm), instead of in pixels at the view's size. Needs a parallel
        /// camera: with a perspective one it is refused ("Switch to parallel projection to
        /// export at scale"). Absent: pixels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<f64>,
    },
    /// Read an OBJ, STL or SketchUp file and add its faces to the open context, as one undo
    /// step: as a new group (default) or component named after the file, placed at the
    /// context's origin and selected, or (`"as": "context"`) as loose geometry merging with
    /// what it touches, which becomes the selection. `format` defaults to the one the extension
    /// names (`.obj`, `.stl`, `.skp`), else to SketchUp for a file with SketchUp's header. OBJ
    /// and STL units are meters times `scale` (default 1); `up` is the file's up axis (default
    /// `y` for OBJ, `z` for STL, as `export` writes them). Corners closer than 1e-6 m are
    /// welded, and with `merge_coplanar` (default true) neighbouring faces in one plane with
    /// one material become one face. OBJ materials come from its `mtllib` libraries. A
    /// SketchUp file (2017 or newer) keeps its components and groups (each definition once),
    /// face holes, soft edges, guides and materials; its lengths keep their real size, times `scale`,
    /// and `up` and `merge_coplanar` are ignored. The outcome's `import` says what was made.
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
    /// Paste clip text (what Copy put on the clipboard) into the open context, as one undo
    /// step, and select what was pasted. With `at`, the clip's base point (the minimum corner
    /// of its bounding box) lands at `at`, moved only; without it, the clip lands where it was
    /// copied from in world coordinates. Edges and faces merge with geometry they touch, as
    /// copies do. Materials and tags are matched by name (missing ones are added). Components
    /// copied from this same document (since it was created or opened) keep their
    /// definitions; otherwise, and for every group, new definitions are made. Text that is not
    /// a clip, is damaged or too large (over 64 MiB) is `invalid_input`.
    Paste {
        clip: String,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        at: Option<[f64; 3]>,
    },
    /// Change the selection of edges, faces, instances, guides, dimensions and notes. Not
    /// recorded in undo history.
    Select {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        mode: SelectMode,
        /// Everything in the open context (edges, faces, instances, guides, dimensions and
        /// notes) in place of `ids`, combined by `mode` the same way.
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
    /// root coordinates, whatever instance is open) in the view. Guides do not count, nor does
    /// what hidden tags hide. With nothing to fit, the camera stays.
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
    /// Draw faces in their tags' colors (`on`), or by their materials. A face on no tag keeps
    /// its material's color (textures are not shown while on); one inside a tagged group or
    /// component takes the nearest such instance's tag. A view setting like `set_lighting`.
    SetColorByTag { on: bool },
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
                Command::Select { .. }
                    | Command::Save { .. }
                    | Command::SaveToLibrary { .. }
                    | Command::Export { .. }
            ))
    }

    /// True for commands that only change the view: move the camera, set the lighting or
    /// color by tag.
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
                | Command::SetColorByTag { .. }
                | Command::SetSectionDisplay { .. }
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

fn yes() -> bool {
    true
}

/// A field that may be absent (None), null (`Some(None)`) or a value.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// An annotation's anchor: a vertex id ("v3"), followed as it moves, or a fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    untagged,
    expecting = "an anchor: a vertex id such as \"v3\", or a point [x, y, z]"
)]
pub enum AnchorInput {
    Vertex(EntityId),
    #[schemars(with = "ModelPoint")]
    Point([f64; 3]),
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

/// What `intersect_faces` crosses the faces with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntersectScope {
    /// Every shown face of the model, in groups and components too.
    #[default]
    Model,
    /// Only the selected faces and instances among themselves.
    Selection,
    /// The faces of the open context's own mesh.
    Context,
}

/// What an `intersect_faces` did: ids that exist after it, usable as batch references.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IntersectReport {
    /// Edges the command made (the pieces of an edge it cut included).
    pub added_edges: Vec<EntityId>,
    /// Faces that existed before and were split; they no longer exist.
    pub split_faces: Vec<EntityId>,
    /// The faces the splits made.
    pub created_faces: Vec<EntityId>,
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
    /// What an `intersect_faces` drew and split; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intersect: Option<IntersectReport>,
    /// The ids a `hide`, `unhide`, `unhide_last` or `unhide_all` changed, in id order (empty
    /// when it did nothing); absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<Vec<EntityId>>,
    /// How many faces a `position_texture` changed; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positioned: Option<usize>,
    /// How many definitions a `purge_definitions` removed (nested ones included); absent for
    /// other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged: Option<usize>,
    /// What an `add_3d_text` made and left out; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_3d: Option<Text3dReport>,
    /// The material an `insert_library_item` of a material added or reused; absent for other
    /// commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    /// The display units, after a command that changed them (`set_units`, or an undo or redo
    /// of such a step); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
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
