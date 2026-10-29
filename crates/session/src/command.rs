use std::fmt;
use std::path::PathBuf;

use kernel::{
    CamFollowerId, ChangeSummary, CouplingId, DefinitionId, EdgeColorMode, EntityId, Fog, Ground,
    JointId, JointKind, MaterialId, PageId, Side, Sketchy, SoftenReport, StyleId, TagId,
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

/// Where an `add_<primitive>` command puts its solid. `group` (default): a new group holding
/// it, `component`: a new component, one instance placed and selected, never merged with the
/// open context's geometry (the outcome's `placed` and `definitions` name them). `geometry`:
/// loose faces and edges in the open context, merging with what they touch. `name` names the
/// group's or component's definition (a component without one is named after the shape:
/// "Box", "Cylinder", ...); with `geometry` a `name` is `invalid_input`; `material` paints the instance, or the faces' front sides for
/// `geometry`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveAs {
    #[default]
    Group,
    Component,
    Geometry,
}

/// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
/// commands never refer to screen positions or cursor state. Points, offsets and entity ids are
/// those of the open context (see `OpenInstance`): the root, or the definition of the instance
/// being edited, in its own coordinates.
/// Which instances a scene stores the positions of: the ids of instances of the root (their
/// current transforms are stored), `true` (add: every instance of the root; update and change:
/// the same set again from the current transforms, every instance of the root when the scene
/// stores none) or `false` (stop storing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScenePositions {
    Flag(bool),
    Ids(Vec<EntityId>),
}

/// Which joints a scene stores the values of: the ids of joints (their current values are
/// stored), `true` (add: every joint that has a value and is not a cam follower; update and
/// change: the same set again from the current values, every such joint when the scene stores
/// none) or `false` (stop storing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SceneJoints {
    Flag(bool),
    Ids(Vec<JointId>),
}

/// How `bevel_edges` shapes one edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BevelKind {
    /// A fillet of the size as its radius.
    Round,
    /// A flat bevel of the size as its distance.
    Chamfer,
}

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
    /// the drawing axes on axis planes (the model axes at the root, the open group's or
    /// component's own axes inside it; the world axes unless moved). `normal` picks the plane;
    /// without it the corners must differ in exactly two coordinates along the drawing axes. With
    /// `normal`, the new faces are turned to face along it (without, canonical orientation).
    AddRectangle {
        #[schemars(with = "ModelPoint")]
        corner: [f64; 3],
        #[schemars(with = "ModelPoint")]
        opposite: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
    },
    /// Draw a circle as a regular polygon of `segments` sides (default 24, at most 360) around
    /// `center` with `radius`, in the plane with `normal` (default Z), as one undo step; with an explicit `normal` the
    /// new faces face along it. The
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
    /// plane with `normal` (default Z), as one undo step; with an explicit `normal` the new faces
    /// face along it. Inscribed, `radius` reaches the
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
    /// Draw the polyline through `points` (2 to 10,000; consecutive duplicates are dropped), as
    /// one undo step, with the rules of `add_edge` (faces form when it closes a loop). With
    /// `closed` the last point joins the first (3 distinct points at least). With `curve`
    /// (default true) its edges form one curve, as an arc's do (a straight polyline gets none);
    /// without it they are plain edges.
    AddCurve {
        #[schemars(with = "Vec<ModelPoint>")]
        points: Vec<[f64; 3]>,
        #[serde(default)]
        closed: bool,
        #[serde(default)]
        curve: Option<bool>,
    },
    /// Draw a helix as one polyline on one curve (not a polygon), as one undo step, with the
    /// rules of `add_curve`: it starts at `center` plus `radius` along `start_direction` (any
    /// length, projected into the plane square to `axis`; default the plane's u axis, as for
    /// `add_circle`) and rises `pitch` per turn along `axis` (default Z; any length), turning
    /// counter-clockwise looking down the axis (clockwise with `left_handed`). `turns` is
    /// above 0 and at most 1000 (fractions allowed), `segments` (3 to 360, default 24) the
    /// edges per turn; a part turn gets as many edges as it needs, rounded up, and the last
    /// point lies at the exact end angle. At most 10,000 points. Outcome `created` (edges).
    /// Sweep a small circle along it with `follow_me` for a spring.
    AddHelix {
        #[schemars(with = "ModelPoint")]
        center: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        pitch: f64,
        turns: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default)]
        start_direction: Option<[f64; 3]>,
        #[serde(default)]
        left_handed: bool,
    },
    /// Draw a rectangle at any angle, as one undo step: its first side runs from `corner` to
    /// `edge_end`, its width is the distance from `width_point` to the line of that side, and
    /// it lies in the plane through the side and `width_point`, on the side of `width_point`.
    /// A zero side, or a `width_point` on the side's line, is refused. With `normal` (which
    /// must be perpendicular to the side and to the width direction) the new face faces along
    /// it; without it the orientation is canonical, as for the other shapes.
    AddRotatedRectangle {
        #[schemars(with = "ModelPoint")]
        corner: [f64; 3],
        #[schemars(with = "ModelPoint")]
        edge_end: [f64; 3],
        #[schemars(with = "ModelPoint")]
        width_point: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
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
    /// Label `target` (a face, edge, vertex or instance of the open context), as one undo
    /// step: a leader note with `text` anchored on it. The anchor is a vertex's position (the
    /// note follows it), an edge's midpoint, the point of a face's interior nearest its
    /// centroid, or for an instance its bounding box center moved onto its surface toward the
    /// camera when that is hit. Without `position` the note goes outward from the open
    /// context's box center through the anchor, 15% of the box diagonal away and raised by
    /// 5% of it. Text rules as `add_note`.
    AddLabel {
        target: EntityId,
        text: String,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        position: Option<[f64; 3]>,
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
    /// Place a PNG or JPEG picture as a new group named `name` (default the file's name without
    /// its extension, or "Image"): one rectangle face `width` meters wide (0.001 to 1000) and as
    /// high as the image's proportions make it, painted front and back with a new material of
    /// the same name (made unique) whose texture is the image (turned upright first when its
    /// EXIF orientation says so), fitted to fill the face exactly (the back is not mirrored). Give the image as `path` (a PNG or JPEG file of at most 16
    /// MiB and 8192 pixels on each side) or `data` (the file's bytes in base64); exactly one.
    /// `origin` is the picture's lower left corner, in the open context. `normal` (default Z)
    /// is the way the picture faces; `x_axis` (default horizontal in the plane, or X on a level
    /// one, as textures run) the way its width runs, projected into the plane. The picture lies
    /// `lift` meters (default 0, 0 to 1) along `normal` from the plane through `origin`; the
    /// renderer draws a picture (a group of one textured face) over the face it lies on, so none is needed there. One undo step; the group is
    /// selected and listed in the outcome's `placed` and `definitions`.
    ImportImage {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        data: Option<String>,
        #[schemars(with = "ModelPoint")]
        origin: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
        #[serde(default)]
        x_axis: Option<[f64; 3]>,
        width: f64,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        lift: Option<f64>,
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
    /// and whether it is active stay. A plane of another context is `not_found` (naming the path
    /// to open); an unknown plane is `invalid_input`.
    FlipSectionPlane { plane: EntityId },
    /// Make a section plane the open context's active plane, as one undo step (the one that
    /// was active no longer is; already active changes nothing). A plane not in the open
    /// context is as for `flip_section_plane`.
    ActivateSectionPlane { plane: EntityId },
    /// Make no section plane of the open context active, as one undo step (none active
    /// changes nothing and makes no undo step). The planes stay.
    DeactivateSection {},
    /// Set how section planes are shown; absent fields stay as they are: `planes` draws the
    /// plane entities, `cuts` applies the active planes' cuts, `fill` fills closed cuts. All
    /// default to true; `fill_color` is `dark` (default, #3a3a3a) or `material` (the color of
    /// the material most of the cut faces' area shows on their front, else the nearest painted
    /// instance's material, else the default face color, darkened by 20%). A view setting
    /// like `set_lighting`: saved with the model, but not an undo step and not a change to the
    /// revision.
    SetSectionDisplay {
        #[serde(default)]
        planes: Option<bool>,
        #[serde(default)]
        cuts: Option<bool>,
        #[serde(default)]
        fill: Option<bool>,
        #[serde(default)]
        fill_color: Option<crate::document::SectionFillColor>,
    },
    /// Set whether section planes of the contexts outside each instance in `ids` cut it
    /// (default true for every instance), as one undo step. An instance with `cut` false
    /// stays whole under planes outside it; planes inside its own definition still cut its
    /// contents. Entities that are not instances are `invalid_input`; instances already as
    /// asked are skipped (no change, no undo step when none differs). The outcome's `changed`
    /// lists the instances that changed.
    SetSectionCut { ids: Vec<EntityId>, cut: bool },
    /// Set the model's display units, which lengths and angles are written and typed in and
    /// the tools snap to, as one undo step; absent fields stay as they are. `precision` is
    /// the decimals kept (0 to 6), or for `feet_inches` the finest fraction of an inch,
    /// 1/2^precision (6: 1/64). `length_snap` (meters, 0 = off, up to 1000) rounds lengths
    /// taken from the cursor; `angle_precision` is the decimals of an angle (0 to 3);
    /// `angle_snap` (degrees, 0 = free, up to 90) is the step angles from the cursor round
    /// to. Saved with the model.
    SetUnits {
        #[serde(default)]
        length: Option<LengthUnit>,
        #[serde(default)]
        precision: Option<u8>,
        #[serde(default)]
        length_snap: Option<f64>,
        #[serde(default)]
        angle_precision: Option<u8>,
        #[serde(default)]
        angle_snap: Option<f64>,
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
    /// Spread instances of the open context apart as one undo step: each moves by
    /// `(c_i - center) * factor`, `c_i` its bounding box center in the context's coordinates.
    /// `center` defaults to the center of all their boxes. `factor` is finite, -0.99 to 100
    /// (0 changes nothing; negative pulls them together). Other ids are `invalid_input`.
    ExplodeView {
        ids: Vec<EntityId>,
        factor: f64,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        center: Option<[f64; 3]>,
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
    /// Copy edges, faces, instances, guides, dimensions, notes and section planes along a
    /// path (a chain of edges as for `follow_me`: connected, unbranched, open or closed), as
    /// one undo step, under the rules of `Copy`. Exactly one of `count` (at least 1) or
    /// `spacing` (above 0). Positions are measured by arc length from the path's start: the
    /// end of an open path nearest the centre of the box around `ids` (the path vertex
    /// nearest it for a closed one). With `count` N on an open path, N positions evenly from
    /// start to end inclusive (1: the start); on a closed path N evenly around. With `spacing`
    /// s, 0, s, 2s, ... up to the length (an open path's end included; a closed path's start
    /// is not repeated). Copy k is the originals moved by the path point's offset from the
    /// start and, with `align` (default true), turned about that point by the smallest
    /// rotation taking the path direction at the start to the direction there (at a vertex
    /// the edge leaving it, at the end of an open path the edge arriving). The start makes no
    /// copy; when no position is left nothing changes (an empty outcome, no undo step). At
    /// most 10,000 copies. `placed` lists the instances made.
    CopyAlongPath {
        ids: Vec<EntityId>,
        path: Vec<EntityId>,
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        spacing: Option<f64>,
        #[serde(default)]
        align: Option<bool>,
    },
    /// Rotate vertices, edges, faces (their vertices), instances, guides, annotations and section
    /// planes (as `move` moves them) by `angle` degrees
    /// about the line through `center` along `axis` (default Z), counter-clockwise looking
    /// down the axis, as one undo step (a whole number of turns changes nothing: an empty
    /// outcome, no undo step). Faces that share some of the turned vertices stretch
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
    /// between 1e-4 and 1e4 in size; a negative factor mirrors (faces stay facing out). All
    /// factors exactly 1 change nothing (an empty outcome, no undo step). Faces
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
    /// Turn face `face` of the open context about the line through `center` along `axis`
    /// (default Z; any length; zero or non-finite is `invalid_input`) into a solid, as one undo step.
    /// `angle` in degrees, above 0 and at most 360 (default 360), counter-clockwise looking
    /// down the axis; `segments` (1 to 360, default 24) is the number of steps in a full turn,
    /// scaled by angle/360 and rounded up (at least 1); a step must turn by less than 180
    /// degrees. The face may touch the axis (the swept faces collapse to triangles, or to
    /// nothing along it, never to zero-area faces) but must not cross it, and its plane must
    /// not be square to the axis (use push_pull); a plane that holds the axis or lies beside
    /// it is fine. A full turn removes the face; a partial one keeps it as the start cap and
    /// closes the end with a copy. Edges between consecutive faces swept by one side are
    /// soft, and so are those at a corner between two sides on one curve that is not a
    /// polygon. Outcome `created`.
    Revolve {
        face: EntityId,
        #[schemars(with = "ModelPoint")]
        center: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        #[serde(default)]
        angle: Option<f64>,
        #[serde(default)]
        segments: Option<u32>,
    },
    /// Turn edges, faces (with their edges), instances, guides, annotations and section planes
    /// into a group:
    /// they move into a new definition, placed by one new instance where they were. Guides,
    /// annotations and section planes alone cannot be grouped; screen notes stay in the root. The instance
    /// becomes the selection. `name` (optional, not blank, as for components) names the group's
    /// definition, which the Outliner and the `entities` query show.
    MakeGroup {
        ids: Vec<EntityId>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Like `MakeGroup`, but the definition is a named component: copies of its instances
    /// share it, so editing one edits them all.
    MakeComponent { ids: Vec<EntityId>, name: String },
    /// Replace instances by their geometry, which merges with what it touches. Instances and
    /// guides inside them come out as instances and guides. What comes out becomes the selection.
    Explode { ids: Vec<EntityId> },
    /// Give the component instances in `ids` (when absent: the selection; other entities are
    /// ignored; present and empty: nothing to do) a definition of their own when their
    /// definition is placed elsewhere too (in any context the root reaches), as one undo step.
    /// All the listed instances of one definition share one new copy, named like the old one
    /// with `#N` (the smallest N of 2 or more no definition has). A definition all of whose
    /// instances are listed, groups and sole instances are left as they are. The copy copies
    /// the groups inside it and shares the components inside it. The outcome's `definitions`
    /// lists the new ones, in the id order of the old ones; with nothing to do it is empty
    /// and no undo step is made.
    MakeUnique {
        #[serde(default)]
        ids: Option<Vec<EntityId>>,
    },
    /// Mirror `ids` (when absent: the selection; present and empty: nothing to do, no undo step; nothing selected
    /// when absent is `invalid_input`) as one
    /// undo step; faces keep facing out and
    /// instances may become mirrored. `frame` `axes` (default): about the plane through the
    /// center of the selection's box in the current drawing axes, normal to the `axis` of
    /// those axes (`red` x, `green` y, `blue` z). `own`: `ids` is exactly one instance,
    /// mirrored about its own center along its own `axis`, and stays where it is. The outcome's
    /// `changed` lists the ids flipped.
    Flip {
        #[serde(default)]
        ids: Option<Vec<EntityId>>,
        axis: FlipAxis,
        #[serde(default)]
        frame: FlipFrame,
    },
    /// Move the axes of the open context as one undo step. `x_axis` is normalized and
    /// `y_axis` made perpendicular to it and normalized (so only its side of `x_axis` matters),
    /// `z` is their cross product; zero or parallel vectors are `invalid_input`. At the root
    /// these are the model axes (the camera query reports them). Inside a group or component
    /// they become that definition's origin and axes, in the open context's coordinates: its
    /// geometry and nested instances move by the frame's inverse and every instance of it is
    /// placed by its transform times the frame, so nothing moves in the model.
    SetAxes {
        #[schemars(with = "ModelPoint")]
        origin: [f64; 3],
        x_axis: [f64; 3],
        y_axis: [f64; 3],
    },
    /// Put the model axes back to the world axes, as one undo step (nothing to change makes no
    /// step). Inside a group or component it does nothing: a definition's axes are its own
    /// frame, with nothing to reset to.
    ResetAxes {},
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
    /// Place a new instance of an existing definition (a component, or a group's definition,
    /// which then is shared like a component's) in the open context at `transform`, as one
    /// undo step. The new instance becomes the selection; the outcome's `changes.created` and
    /// `placed` name it. An unknown definition is `not_found`; placing a definition inside
    /// itself, directly or through nesting, is `invalid_input`.
    PlaceInstance {
        definition: DefinitionId,
        #[serde(default)]
        transform: Transform,
    },
    /// Replace the transform of an instance of the open context, as one undo step (the
    /// outcome's `changes.modified` names it). Same transform rules as `place_instance`.
    SetTransform { id: EntityId, transform: Transform },
    /// Make a closed box solid with one corner at `corner` and sides `size` (`[sx, sy, sz]`,
    /// each at least 0.01 mm) along the open context's axes. See `PrimitiveAs` for `as`,
    /// `name` and `material`, shared by every `add_<primitive>` command.
    AddBox {
        #[schemars(with = "ModelPoint")]
        corner: [f64; 3],
        size: [f64; 3],
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// Make a closed cylinder of `segments` sides (default 24, 3 to 360) standing on the circle
    /// of `radius` around `base`, `height` along `axis` (default Z; its direction, any length).
    /// The first base corner lies along the plane's u axis (see `add_circle`); the edges between
    /// the sides are soft. See `PrimitiveAs`.
    AddCylinder {
        #[schemars(with = "ModelPoint")]
        base: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        height: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// Make a closed tube: a cylinder with a bore of `inner_radius` (0 < inner_radius < radius),
    /// an outer and an inner wall and two ring caps. Frame and options as `add_cylinder`.
    AddTube {
        #[schemars(with = "ModelPoint")]
        base: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        inner_radius: f64,
        height: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// Make a closed cone on the circle of `radius` around `base`, `height` along `axis`,
    /// narrowing to `top_radius` (default 0: a point where triangle sides meet; above 0 a
    /// frustum with a top cap, which may be wider than the base). Frame and options as
    /// `add_cylinder`; the sloping edges are soft.
    AddCone {
        #[schemars(with = "ModelPoint")]
        base: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        #[serde(default)]
        top_radius: Option<f64>,
        height: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// Make a closed prism: a regular polygon of `sides` corners (default 6, 3 to 360) inscribed
    /// in `radius` around `base`, pulled `height` along `axis`. The first corner lies along
    /// `direction` projected into the plane (default the plane's u axis). All edges are hard.
    /// Frame and options as `add_cylinder`.
    AddPrism {
        #[schemars(with = "ModelPoint")]
        base: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        #[serde(default)]
        sides: Option<u32>,
        height: f64,
        #[serde(default)]
        direction: Option<[f64; 3]>,
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// Add a closed UV sphere centered at `center`: `segments` around (default 24, 3 to 360)
    /// and half as many rings (at least 2), triangles at the poles, which lie along `axis`
    /// (default +Z). Every edge is soft. Frame and options as `add_cylinder`.
    AddSphere {
        #[schemars(with = "ModelPoint")]
        center: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// Add a closed torus centered at `center` about `axis` (default +Z): a tube of
    /// `tube_radius` swept around a circle of `radius` (`0 < tube_radius < radius`),
    /// `segments` around the axis and `tube_segments` around the tube (each default 24 and
    /// 12, 3 to 360). Every edge is soft. Frame and options as `add_cylinder`.
    AddTorus {
        #[schemars(with = "ModelPoint")]
        center: [f64; 3],
        #[serde(default)]
        axis: Option<[f64; 3]>,
        radius: f64,
        tube_radius: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default)]
        tube_segments: Option<u32>,
        #[serde(default, rename = "as")]
        placement: PrimitiveAs,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        material: Option<MaterialId>,
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
    /// `metalness` (default 0) and `roughness` (default 1), each from 0 to 1, set its finish:
    /// a rough non-metal is matte, a smooth one has highlights and reflections, a metal tints
    /// them with its color.
    /// `density` (kg/m³, 1 to 30000, default none) weighs solids painted with it (see
    /// `mass_properties`).
    /// The new id is in the outcome's `changes.materials`. `existing` says what happens when
    /// `name` is already taken: `fail` (default) is an error, `reuse` returns the existing
    /// material unchanged (no undo step), `update` gives it the color, opacity, texture,
    /// emission, metalness, roughness and density passed, as `change_material` would (one undo
    /// step), and returns it. `reuse` checks only the ranges of the opacity, emission,
    /// metalness, roughness and density given.
    /// `existing` other than `fail` needs a `name`.
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
        #[serde(default)]
        metalness: Option<f32>,
        #[serde(default)]
        roughness: Option<f32>,
        #[serde(default)]
        density: Option<f64>,
        #[serde(default)]
        existing: ExistingMaterial,
    },
    /// Rename, recolor, change the opacity (0 to 1), the texture, the emission (0 to 10), the
    /// metalness and/or the roughness (each 0 to 1) and/or the density (kg/m³, 1 to 30000;
    /// null clears it) of a material; absent fields stay as they are. `texture`: null removes it; an object
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
        #[serde(default)]
        metalness: Option<f32>,
        #[serde(default)]
        roughness: Option<f32>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(with = "Option<f64>")]
        density: Option<Option<f64>>,
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
        /// Also store the positions (transforms) of instances of the root (see
        /// `ScenePositions`); absent or false: not stored.
        #[serde(default)]
        positions: Option<ScenePositions>,
        /// Also store the values of joints (see `SceneJoints`); going to the scene drives them.
        /// Absent or false: not stored.
        #[serde(default)]
        joints: Option<SceneJoints>,
        /// Also store the active section plane of each context and the section display
        /// settings; going to the scene restores them. Absent or false: not stored.
        #[serde(default)]
        sections: Option<bool>,
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
        /// The instance positions (see `ScenePositions`); absent: stored again if stored.
        #[serde(default)]
        positions: Option<ScenePositions>,
        /// The joint values (see `SceneJoints`); absent: stored again if stored.
        #[serde(default)]
        joints: Option<SceneJoints>,
        /// The active section planes and section display; absent: stored again if stored,
        /// false stops storing them.
        #[serde(default)]
        sections: Option<bool>,
    },
    /// Rename a scene and choose what it stores, as one undo step (none when nothing changes).
    /// At least one of `name`, `camera`, `tags`, `positions`, `joints` and `sections` is needed. `camera` or `tags` false stops
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
        /// The instance positions (see `ScenePositions`); absent: kept as stored.
        #[serde(default)]
        positions: Option<ScenePositions>,
        /// The joint values (see `SceneJoints`); absent: kept as stored.
        #[serde(default)]
        joints: Option<SceneJoints>,
        /// The active section planes and section display: false drops them, true stores the
        /// current ones only if the scene stores none; absent: kept as stored.
        #[serde(default)]
        sections: Option<bool>,
    },
    /// Move a scene to `index` in the order (0: first), as one undo step. An index past the
    /// last scene is invalid input.
    MoveScene { scene: PageId, index: usize },
    /// Remove a scene, as one undo step.
    DeleteScene { scene: PageId },
    /// Go to a scene: set the camera, projection, lighting and color by tag it stores (a view
    /// change like `set_camera`, not an undo step), and show and hide tags as it stores (one
    /// undo step, only when that changes a tag). A scene that stores positions also sets those
    /// instances' transforms in that one undo step, labelled "Go to scene <name>"; a scene that
    /// stores joint values then drives those joints (solving couplings and cam followers) in
    /// the same step, and is refused (`invalid_input`, nothing changed) when they cannot all be
    /// met. A scene that stores section planes makes exactly those planes active (a context it
    /// lists no plane for has none active) and sets the section display, in that same step. It
    /// becomes the selected scene.
    GoToScene { scene: PageId },
    /// Add a style after the last one, as one undo step: a copy of the style `from`, or the
    /// default settings (today's look: near-black edges on a light gray background, no
    /// profiles, sketchy edges, endpoints, sky, ground or fog) without it. Without `name` it is
    /// called "Style 1" (or the lowest number no style has); names are trimmed and must be
    /// distinct, not blank and at most 100 characters. The new id is in the outcome's
    /// `changes.styles`. It is not chosen (see `set_style`).
    AddStyle {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        from: Option<StyleId>,
    },
    /// Rename a style and/or change its settings, as one undo step (none when nothing
    /// changes). At least one of `name` and `settings` is needed. In `settings` only the parts
    /// given change: `edges` merges field by field (`color_mode`, `color`, `show_edges`,
    /// `profiles` 2 to 10 px, `sketchy` {`jitter` 0 to 10 px, `extension` 0 to 30 px},
    /// `endpoints` 2 to 30 px; null turns `profiles`, `sketchy` and `endpoints` off);
    /// `background` and `sky` are `[r, g, b]` (0 to 255); `ground` is {`color`, `opacity` 0 to
    /// 1, `from_below`}; `fog` is {`start`, `end` (meters, 0 <= start < end), `color`:
    /// "background" or {"rgb": [r, g, b]}}; null turns `sky`, `ground` and `fog` off. A value
    /// out of range is `invalid_input` and nothing changes.
    ChangeStyle {
        style: StyleId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        settings: Option<StylePatch>,
    },
    /// Remove a style, as one undo step. The view and every scene that chose it go back to no
    /// style (the default look) in the same step; undo restores them.
    DeleteStyle { style: StyleId },
    /// Choose the style the view draws with (absent or null: none, the default look, with the
    /// app theme's background). A view setting like `set_lighting`: saved with the camera and
    /// scenes, but not an undo step and not a change to the revision. Renders (`render_view`,
    /// snapshots and raster print) use it; the line drawing, SVG and vector PDF do not.
    SetStyle {
        #[serde(default)]
        style: Option<StyleId>,
    },
    /// Add a joint between two parts, as one undo step. `a` and `b` are instances of the root
    /// (the root must be the open context); `a` null or absent is the ground, the fixed world.
    /// `kind`: `revolute` (rotation about the axis), `prismatic` (sliding along it),
    /// `cylindrical` (both), `fixed` (no motion) or `ball` (rotation about the point).
    /// `point` and `axis` (a direction, any length) are in the root's coordinates; `reference`
    /// (optional) is a direction that measures the angle, projected perpendicular to the axis
    /// (default: the horizontal direction across the axis, as `add_circle` takes, or +X when
    /// the axis is vertical). The joint holds the parts as they are now: its value starts at
    /// 0. `limits` (optional) is `[min, max]` in degrees for revolute and cylindrical joints
    /// and meters for prismatic ones, min below max and 0 within; a fixed or ball joint has
    /// none. Without `name` it is called "Joint 1" (the lowest number no joint has); names
    /// are trimmed, not blank, at most 100 characters and distinct. The same part twice, a
    /// part that is not an instance of the root, a zero axis or a reference along the axis
    /// is `invalid_input`. The new id is in the outcome's `changes.joints`.
    AddJoint {
        kind: JointKind,
        #[serde(default)]
        a: Option<EntityId>,
        b: EntityId,
        #[schemars(with = "ModelPoint")]
        point: [f64; 3],
        axis: [f64; 3],
        #[serde(default)]
        reference: Option<[f64; 3]>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        limits: Option<[f64; 2]>,
    },
    /// Rename a joint and/or change its limits, as one undo step (none when nothing changes).
    /// At least one of `name` and `limits` is needed; `"limits": null` clears them, an array
    /// sets them as in `add_joint` (the joint's current value must lie within).
    ChangeJoint {
        joint: JointId,
        #[serde(default)]
        name: Option<String>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(with = "Option<[f64; 2]>")]
        limits: Option<Option<[f64; 2]>>,
    },
    /// Remove a joint, as one undo step. Erasing a part removes its joints in the same step.
    DeleteJoint { joint: JointId },
    /// Set a joint's value and move the parts connected to it through joints so that every
    /// joint between them holds, as one undo step. `value` is degrees for revolute and
    /// cylindrical joints and meters for prismatic ones; `slide` (meters, cylindrical only)
    /// sets the slide too, absent keeping it. The other joints' values are free and follow
    /// the new pose. Parts joined to the ground keep it fixed; an assembly with no ground
    /// holds the joint's part `a`. Large turns are taken in steps of at most 10 degrees (a
    /// slide in tenths) so a closed loop keeps its branch. A fixed or ball joint, a value
    /// outside the joint's limits, a pose where the joints cannot all be met (to 1e-9) or one
    /// where another joint would pass its limits is `invalid_input` and changes nothing; the
    /// joint's current value changes nothing (no undo step). The outcome's
    /// `changes.modified` lists the moved instances and `changes.joints` the joints whose
    /// values changed.
    Drive {
        joint: JointId,
        value: f64,
        #[serde(default)]
        slide: Option<f64>,
    },
    /// Couple two joints' values, as gears, chains, belts and racks do, as one undo step:
    /// every drive then holds `driven = ratio * driver + offset`. Values are in the joints'
    /// units (degrees for revolute and cylindrical joints, their angle; meters for prismatic
    /// ones), so `ratio` is driven units per driver unit: 0.5 for a cam turning once in two
    /// crank turns, negative for meshed gears turning opposite ways, meters per degree for a
    /// pinion driving a rack. `ratio` is finite, not 0 and at most 1000 in size; `offset`
    /// (driven units, default: the value that holds now, so adding a coupling moves nothing).
    /// The joints are linked into one assembly however their parts are connected. Two joints
    /// can be coupled once; a fixed or ball joint has no value to couple. Couplings that
    /// contradict each other (a cycle whose ratios do not multiply to 1) are refused when a
    /// drive cannot meet them. The new id ("k<n>") is in the outcome's `changes.couplings`.
    AddCoupling {
        driver: JointId,
        driven: JointId,
        ratio: f64,
        #[serde(default)]
        offset: Option<f64>,
    },
    /// Remove a coupling, as one undo step. Deleting a joint (or erasing its part) removes
    /// its couplings in the same step.
    DeleteCoupling { coupling: CouplingId },
    /// Make a prismatic joint follow a cam, as one undo step: every drive that moves the cam
    /// (or the follower's part) then sets the joint's value so that its frame origin on the
    /// moving part, stood `roller_radius` meters back along the axis (default 0: a flat or
    /// knife follower), touches the cam's surface along the joint's axis line, on the side
    /// the follower is pushed from. `cam` is an instance of the root; `preload_direction`
    /// (a direction, not square to the axis; default: toward the center of the cam's
    /// bounding box along the axis) is the way the follower is pushed onto the cam. The axis
    /// line must meet the cam now; a drive where it no longer does is refused naming the
    /// follower. A follower joint is not driven or coupled itself. The new id ("w<n>") is in
    /// the outcome's `changes.cam_followers`.
    AddCamFollower {
        cam: EntityId,
        follower: JointId,
        #[serde(default)]
        roller_radius: Option<f64>,
        #[serde(default)]
        preload_direction: Option<[f64; 3]>,
    },
    /// Remove a cam follower, as one undo step: the joint slides free again. Deleting the
    /// joint or erasing the cam (or the follower's part) removes it in the same step.
    DeleteCamFollower { cam_follower: CamFollowerId },
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
    /// Turn each face in `ids` over, as one undo step (non-face ids are ignored; none to turn
    /// is a success with no undo step). Faces keep their ids, and each side keeps its material
    /// and texture look. The outcome's `changed` lists the faces turned.
    ReverseFaces { ids: Vec<EntityId> },
    /// Orient the faces connected to face `id` so they all agree with it, as one undo step:
    /// breadth first across edges shared by exactly two faces (an edge of three or more
    /// stops the walk), each neighbour turned over if it runs the shared edge the same way
    /// round. With `ids` not empty only those faces are walked. A surface that cannot be
    /// oriented (a Moebius strip) keeps the first orientation reached and counts the edges
    /// left in disagreement. The outcome's `orient` lists the faces turned and the conflicts.
    OrientFaces {
        id: EntityId,
        #[serde(default)]
        ids: Vec<EntityId>,
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
    /// Solid Tools Union, as one undo step: the groups and components `ids` (2 or more,
    /// instances of the open context, each a solid: see the `solid` query) become one new
    /// group holding everything inside any of them, placed with the first one's transform,
    /// material, tag and hidden flag and its definition's name; the operands are erased.
    /// Faces keep their materials; seams between coplanar faces the operation made are
    /// removed. `placed` holds the new group and `definitions` its new definition. A result that would not be closed is
    /// `invalid_input` ("the result would not be a solid") and changes nothing.
    SolidUnion { ids: Vec<EntityId> },
    /// Solid Tools Subtract, as one undo step: `target` minus each of `tools` (1 or more) in
    /// order, under the rules of `solid_union`; target and tools are erased. The tools' faces
    /// that bound the cut are turned over, with their front material on the cut. When
    /// nothing is left, no group is made: the outcome has `empty: true` and `placed` is
    /// empty.
    SolidSubtract {
        target: EntityId,
        tools: Vec<EntityId>,
    },
    /// Solid Tools Intersect, as one undo step: what lies inside every one of `ids` (2 or
    /// more), under the rules of `solid_union`. When the solids share no volume, no group is
    /// made and the operands are still erased: the outcome has `empty: true`.
    SolidIntersect { ids: Vec<EntityId> },
    /// Solid Tools Trim, as one undo step: as `solid_subtract`, but the `tools` stay where
    /// they are; only `target` is erased and replaced by the new group. When nothing is left
    /// of the target, no group is made and the outcome has `empty: true`.
    SolidTrim {
        target: EntityId,
        tools: Vec<EntityId>,
    },
    /// Solid Tools Split, as one undo step: `a` and `b` (solids) become up to three groups,
    /// in this order: `a` minus `b`, what both share, `b` minus `a`; a piece with nothing in
    /// it is not made. The groups take the transform, material, tag and hidden flag of `a`;
    /// `a` and `b` are erased. `placed` holds the groups.
    SolidSplit { a: EntityId, b: EntityId },
    /// Solid Tools Outer Shell, as one undo step: the union of `ids` (1 or more) with its
    /// internal faces removed: shells inside another shell (voids, and solids floating in
    /// them) are dropped. A single solid with a void inside becomes the solid without it.
    /// Placed and erased as `solid_union`.
    OuterShell { ids: Vec<EntityId> },
    /// Close the groups and components `ids` (of the open context; every one when empty)
    /// where that is safe, as one undo step: each definition they place at any depth that is
    /// not a solid is repaired once (so every instance of a component is fixed) by welding
    /// vertices across tiny gaps, removing stray edges and internal faces, making faces agree,
    /// filling flat holes and turning closed shells outward. A definition changes only when
    /// it is a solid afterwards; otherwise it is left as it was and the report says what is
    /// left and why. Solids are never changed; nothing to change makes no undo step. The
    /// outcome's `make_solid` reports each definition.
    MakeSolid {
        #[serde(default)]
        ids: Vec<EntityId>,
    },
    /// Chamfer straight edges of a solid, as one undo step: each edge in `edges` becomes a
    /// flat face set back `distance` meters (more than 0.000001, measured in the edited mesh's
    /// own coordinates) on both faces beside it. A convex edge loses material, a concave one
    /// gains it. Without `instance`, `edges` are edges of the open context, whose own faces
    /// (nested groups and components aside) must be a solid: open the group first. With
    /// `instance` (a group or component of the open context, a solid), `edges` are edges of
    /// its definition, which is edited (a component's other instances change too). Each edge
    /// must be straight (not part of a curve), not soft, lie between two flat faces at least 1
    /// degree apart, and the setback line must stay inside both faces along the whole edge;
    /// two chosen edges facing each other across a face must not have overlapping setbacks.
    /// Chamfers of convex edges meeting at an outside corner overlap and close it; at any
    /// other end (no other chosen edge, next to a concave edge, or an inside corner) the cut
    /// stops in the plane of the face there. A concave edge whose two faces are both cut away
    /// at an end starts `distance` back from that end. Three chamfers meeting at an outside
    /// corner of three faces close it with a flat triangle. Faces and edges the cut does not
    /// touch keep their ids. `distances`, when given, has one distance per edge (same order as
    /// `edges`) in place of `distance`, which stays the required default.
    ChamferEdges {
        #[serde(default)]
        instance: Option<EntityId>,
        edges: Vec<EntityId>,
        distance: f64,
        #[serde(default)]
        distances: Option<Vec<f64>>,
    },
    /// Round straight edges of a solid, as one undo step: each edge in `edges` becomes an arc
    /// of `radius` meters (more than 0.000001, in the edited mesh's own coordinates) tangent to
    /// both faces beside it, made of `segments` faces per quarter turn (default 8, 1 to 90; in
    /// proportion for other angles, at least 1) whose edges, and the edges where the round
    /// meets the flat faces, are soft. The round touches each face `radius / tan(angle / 2)`
    /// from the edge, `angle` the angle between the faces inside the solid (for a concave
    /// edge, outside it). Everything else is as `chamfer_edges`, with that setback in place of
    /// the distance. `radii`, when given, has one radius per edge (same order as `edges`) in
    /// place of `radius`, which stays the required default; three convex rounds of different
    /// radii at a vertex of three faces get a ringed patch between their end arcs, and three
    /// concave rounds of one radius get a concave sphere piece.
    FilletEdges {
        #[serde(default)]
        instance: Option<EntityId>,
        edges: Vec<EntityId>,
        radius: f64,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default)]
        radii: Option<Vec<f64>>,
    },
    /// Round some edges and chamfer others of a solid in one call, as one undo step. `edges`,
    /// `sizes` (meters, each more than 0.000001) and `kinds` (`round` or `chamfer`) have one
    /// entry per edge, in the same order; a round of `sizes[i]` has `segments` faces per
    /// quarter turn (default 8, 1 to 90). Everything else is as `fillet_edges` and
    /// `chamfer_edges`, sizes snapping together within each kind. A convex corner of three
    /// faces where rounds and chamfers meet gets a ringed patch between the strips' ends, a
    /// chamfer standing in as the round with the same setback; a corner with inside and
    /// outside rounds keeps the crossed cuts.
    BevelEdges {
        #[serde(default)]
        instance: Option<EntityId>,
        edges: Vec<EntityId>,
        sizes: Vec<f64>,
        kinds: Vec<BevelKind>,
        #[serde(default)]
        segments: Option<u32>,
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
    /// Replace the model and view with those saved in a `.tnn` file. Clears undo history;
    /// not an undo step. With changes not saved, fails (`unsaved_changes`) unless `discard`
    /// is true. A SketchUp file (`.skp`, or a file whose header is SketchUp's) opens as a new
    /// model never saved: its geometry and instances in the root, the view zoomed to them,
    /// and the outcome's `import` report; `save` then needs a path.
    Open {
        path: String,
        #[serde(default)]
        discard: bool,
    },
    /// Save the model and view as a `.tnn` file at `path`, or at the path last opened or
    /// saved when `path` is absent. Not an undo step. An existing file there that is not a
    /// model (an empty file is fine) is refused unless `overwrite` is true.
    Save {
        #[serde(default)]
        path: Option<String>,
        /// Replace an existing file even when it is not a `.tnn` model (without it such a
        /// file is refused; an empty or missing file is always fine).
        #[serde(default)]
        overwrite: bool,
    },
    /// Save a library item to the `.tnn` file `path` (named `<name>.tnn`): `kind` `component`
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
    /// Insert the library item at `path` (see `save_to_library`; any `.tnn` file is a model
    /// item) into the open context, as one undo step. A component or model: new definitions
    /// are always made, materials and tags are matched by name (missing ones are added),
    /// the minimum corner of its bounding box lands at `at`, or at the open context's origin
    /// without it, and what it made becomes the selection. A material: added to the model;
    /// one with the same name and the same color, opacity, emission, metalness, roughness and
    /// texture is reused,
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
    /// `.stl`, `.glb`, `.gltf`, `.skp`, `.svg`, `.3mf`, `.csv`). Not an undo step. The outcome's `export` counts the
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
    /// A 3MF package (`.3mf`) holds one object per root group or component (and one for loose
    /// faces) in millimeters with their colors; its `export` lists each object and whether
    /// its mesh is closed.
    /// A CSV (`.csv`, or `format` `csv`) is the cut list of the whole model grouped by
    /// definition (see the `cut_list` query), sizes in the model's display unit named in the
    /// header; it has no triangles, so `export` is empty and an empty model gives a header only.
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
        /// 3MF only: write just the selected parts (root groups and components) and loose
        /// faces. Needs the root as the open context.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        selection: bool,
    },
    /// Read an OBJ, STL, SketchUp, glTF or COLLADA file and add its faces to the open context,
    /// as one undo step: as a new group (default) or component named after the file, placed at
    /// the context's origin and selected, or (`"as": "context"`) as loose geometry merging with
    /// what it touches, which becomes the selection. `format` defaults to the one the extension
    /// names (`.obj`, `.stl`, `.skp`, `.gltf`, `.glb`, `.dae`), else to SketchUp for a file
    /// with SketchUp's header. OBJ and STL units are meters times `scale` (default 1); `up` is the file's up axis (default
    /// `y` for OBJ, `z` for STL, as `export` writes them). Corners closer than 1e-6 m are
    /// welded, and with `merge_coplanar` (default true) neighbouring faces in one plane with
    /// one material become one face. OBJ materials come from its `mtllib` libraries. A
    /// SketchUp file (2017 or newer) keeps its components and groups (each definition once),
    /// face holes, soft edges, guides and materials; its lengths keep their real size, times `scale`,
    /// and `up` and `merge_coplanar` are ignored. A glTF (`.gltf`, `.glb`) or COLLADA
    /// (`.dae`) file keeps its node tree (nodes as groups; meshes several nodes place, and
    /// COLLADA library nodes, as components), materials and base-color textures; its triangles
    /// become faces as OBJ polygons do, with smooth surfaces' edges soft. glTF is meters times
    /// `scale`, `y` up by default; COLLADA's own unit times `scale` and its own up axis unless
    /// `up` is given. Side files (buffers, images) are read only from the file's folder or
    /// below it. The outcome's `import` says what was made.
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
        /// Set by the API under `--api-root`: the OBJ's material libraries, and a glTF's or
        /// COLLADA file's buffers and images, are read only when they lie, after symlinks,
        /// within these folders. Never read from or written to JSON.
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
    /// Turn the view about the eye: `yaw` degrees about the vertical (counter-clockwise seen
    /// from above), then `pitch` degrees raising the view (negative lowers it). The view's
    /// pitch stays within -89 and 89 degrees.
    LookAround {
        #[serde(default)]
        yaw: f64,
        #[serde(default)]
        pitch: f64,
    },
    /// Stand at `point` with the eye `eye_height` meters above it (0.01 to 10,000, default
    /// 1.676, 5'6"), looking level toward `look_at` (its height is ignored) or, without it,
    /// along the current horizontal heading. Switches to perspective.
    PositionCamera {
        #[schemars(with = "ModelPoint")]
        point: [f64; 3],
        #[serde(default)]
        eye_height: Option<f64>,
        #[serde(default)]
        #[schemars(with = "Option<ModelPoint>")]
        look_at: Option<[f64; 3]>,
    },
    /// Light the view by `day` (a headlight and ambient light, glowing faces adding theirs) or
    /// by `night` (both dimmed, a dark background: glowing faces light the scene), or by
    /// `studio` (no headlight: key, fill and rim lights that follow the camera). A view
    /// setting like the camera's: saved with the model, but not an undo step and not a change
    /// to the revision.
    SetLighting { mode: Lighting },
    /// Draw faces in their tags' colors (`on`), or by their materials. A face on no tag keeps
    /// its material's color (textures are not shown while on); one inside a tagged group or
    /// component takes the nearest such instance's tag. A view setting like `set_lighting`.
    SetColorByTag { on: bool },
    /// Draw every shown face as glass (`on`), alpha 0.5 on both sides (a face already more
    /// transparent keeps its alpha), so edges behind faces show. Picking is unchanged. A view
    /// setting like `set_lighting`; print, PDF, SVG and the line drawing ignore it.
    SetXRay { on: bool },
    /// Draw entities hidden by their own flag ghosted (`on`) and let them be picked, box
    /// selected and selected, so they can be unhidden; inference still ignores them and
    /// hidden tags stay hidden. A view setting like `set_lighting`; print, PDF, SVG and the
    /// line drawing ignore it.
    SetHiddenGeometry { on: bool },
    /// Set the sun's shadows; absent fields stay as they are. `on` draws the model lit by the
    /// sun with shadows (ignored while the lighting is `night`). `date` is `YYYY-MM-DD` (years
    /// 1800 to 2200) and `time` is `HH:MM`, the clock at the model's UTC offset (see
    /// `set_location`); `light` and `dark` are percentages from 0 to 100 (the sunlit and
    /// shaded brightness of a face); `on_ground` draws the shadows on the ground plane z = 0.
    /// Defaults: off, 2026-11-08, 13:30, 80, 45, on the ground. A bad field is
    /// `invalid_input` and none is applied. A view setting like `set_lighting`, saved with the
    /// camera and scenes.
    SetShadows {
        #[serde(default)]
        on: Option<bool>,
        #[serde(default)]
        date: Option<String>,
        #[serde(default)]
        time: Option<String>,
        #[serde(default)]
        light: Option<i64>,
        #[serde(default)]
        dark: Option<i64>,
        #[serde(default)]
        on_ground: Option<bool>,
    },
    /// Move the model on the Earth, as one undo step; absent fields stay as they are (nothing
    /// to change makes no step). `latitude` -90 to 90 and `longitude` -180 to 180 degrees;
    /// `north_angle` the degrees from the green axis clockwise (seen from above) to true
    /// north, -360 to 360; `utc_offset` the clock's hours east of UTC, -12 to 14 in steps of
    /// 0.25. Defaults: 40.0167, -105.2833, 0, -7. An out-of-range number is `invalid_input`.
    /// Drawing axes (`set_axes`) do not move north.
    SetLocation {
        #[serde(default)]
        latitude: Option<f64>,
        #[serde(default)]
        longitude: Option<f64>,
        #[serde(default)]
        north_angle: Option<f64>,
        #[serde(default)]
        utc_offset: Option<f64>,
    },
    /// Set how the window moves between scenes, as one undo step (nothing to change makes no
    /// step): `on` (default true) animates going to a scene and `seconds` (0 to 10, default 1)
    /// is how long. Absent fields stay as they are; at least one is needed. A model setting,
    /// saved with the file; going to a scene over the API (`go_to_scene`) is never animated.
    SetSceneTransition {
        #[serde(default)]
        on: Option<bool>,
        #[serde(default)]
        seconds: Option<f64>,
    },
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
                | Command::LookAround { .. }
                | Command::PositionCamera { .. }
                | Command::SetLighting { .. }
                | Command::SetColorByTag { .. }
                | Command::SetXRay { .. }
                | Command::SetHiddenGeometry { .. }
                | Command::SetShadows { .. }
                | Command::SetStyle { .. }
                | Command::SetSectionDisplay { .. }
        )
    }
}

/// The parts of a style's settings `change_style` sets: a part left out stays, a null turns
/// an optional part off.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StylePatch {
    #[serde(default)]
    pub edges: Option<EdgesPatch>,
    #[serde(default)]
    pub background: Option<[u8; 3]>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<[u8; 3]>")]
    pub sky: Option<Option<[u8; 3]>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<Ground>")]
    pub ground: Option<Option<Ground>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<Fog>")]
    pub fog: Option<Option<Fog>>,
    #[serde(default)]
    pub ambient_occlusion: Option<AmbientOcclusionPatch>,
}

/// The parts of a style's ambient occlusion settings `change_style` sets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmbientOcclusionPatch {
    #[serde(default)]
    pub on: Option<bool>,
    /// Meters, 0.001 to 100.
    #[serde(default)]
    pub radius: Option<f32>,
    /// 0 to 2.
    #[serde(default)]
    pub strength: Option<f32>,
}

/// The parts of a style's edge settings `change_style` sets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgesPatch {
    #[serde(default)]
    pub color_mode: Option<EdgeColorMode>,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    #[serde(default)]
    pub show_edges: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<f32>")]
    pub profiles: Option<Option<f32>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<Sketchy>")]
    pub sketchy: Option<Option<Sketchy>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<f32>")]
    pub endpoints: Option<Option<f32>>,
}

/// A texture for `add_material` or `change_material`: an image file and the size one copy of
/// it covers on the model, in meters (from 0.001 to 1000). A new image without `width` is 1 m
/// wide (or as wide as the texture it replaces), and without `height` keeps the image's
/// proportions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextureSource {
    /// A PNG or JPEG file of at most 16 MiB and 8192 pixels on each side.
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

/// The axis `Command::Flip` mirrors along, by its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlipAxis {
    Red,
    Green,
    Blue,
}

impl FlipAxis {
    /// The axis' index: x 0, y 1, z 2.
    pub fn index(self) -> usize {
        match self {
            FlipAxis::Red => 0,
            FlipAxis::Green => 1,
            FlipAxis::Blue => 2,
        }
    }
}

/// Whose axes `Command::Flip` mirrors along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlipFrame {
    /// The current drawing axes.
    #[default]
    Axes,
    /// The one selected instance's own axes.
    Own,
}

/// What `Command::AddMaterial` does when the name is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExistingMaterial {
    /// An error.
    #[default]
    Fail,
    /// The existing material is returned as it is.
    Reuse,
    /// The existing material takes the given fields and is returned.
    Update,
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

/// What an `orient_faces` did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrientOutcome {
    /// The faces turned over, in id order.
    pub flipped: Vec<EntityId>,
    /// Shared edges whose faces could not be made to agree.
    pub conflicts: usize,
}

/// What a `make_solid` did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MakeSolidReport {
    /// One entry per definition that was not a solid, in the order the parts reach them.
    pub entries: Vec<MakeSolidEntry>,
    /// How many of the definitions were solids already (left untouched).
    pub solid: usize,
}

/// What `make_solid` did to one definition: when `fixed`, what it changed; otherwise the
/// problems still found after trying (`left`) and why it stopped (`reasons`), the definition
/// left as it was.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MakeSolidEntry {
    pub definition: DefinitionId,
    /// The parts named (or all) that place it.
    pub parts: Vec<EntityId>,
    pub fixed: bool,
    pub welded: usize,
    /// Open edges split where another edge's vertex lies on them.
    #[serde(default)]
    pub tjunctions_mended: usize,
    pub strays_removed: usize,
    pub internal_removed: usize,
    pub holes_filled: usize,
    pub faces_flipped: usize,
    pub left: Vec<crate::query::HealthProblem>,
    pub reasons: Vec<String>,
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
    /// What an `orient_faces` turned over; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orient: Option<OrientOutcome>,
    /// What a `make_solid` fixed and could not; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make_solid: Option<MakeSolidReport>,
    /// What an `intersect_faces` drew and split; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intersect: Option<IntersectReport>,
    /// The ids a `hide`, `unhide`, `unhide_last`, `unhide_all` or `reverse_faces` changed, in
    /// id order (empty when it did nothing); absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<Vec<EntityId>>,
    /// How many faces a `position_texture` changed; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positioned: Option<usize>,
    /// How many definitions a `purge_definitions` removed (nested ones included); absent for
    /// other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged: Option<usize>,
    /// The definitions a `make_unique` made, in order (empty when it did nothing), or the one a
    /// `make_group`, `make_component`, `add_<primitive>` (as group or component) or a Solid Tools command (the new groups' definitions, in `placed` order) made; absent
    /// for other commands. Batch commands can refer to them as `$N:dI`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definitions: Option<Vec<DefinitionId>>,
    /// The instances a command left in the open context, in creation order (instances inside
    /// new definitions are not listed): present whenever the command placed instances (`copy`,
    /// `rotate` with copies, `copy_along_path`, `make_group`, `make_component`, `add_<primitive>`,
    /// `explode`, `paste`, `insert_library_item`, `place_instance`, an `import`, ...), absent otherwise. Batch
    /// `$N:iK` references pick from this list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placed: Option<Vec<EntityId>>,
    /// What an `add_3d_text` made and left out; absent for other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_3d: Option<Text3dReport>,
    /// The material an `insert_library_item` of a material added or reused; absent for other
    /// commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    /// True when a `solid_subtract`, `solid_intersect` or `solid_trim` left nothing (no group was made; the
    /// operands were still erased); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<bool>,
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

/// Where an instance sits: either `matrix` (12 numbers, column-major 3x4: x axis, y axis, z
/// axis, translation, the layout the `entities` query reports for an instance), or any of
/// `origin` (default 0), `x_axis` (default [1,0,0]), `y_axis` (default [0,1,0]) and `z_axis`
/// (default [0,0,1]); not both. All numbers must be finite, the origin and each axis
/// component within +/-1e6, and the axes' determinant at least 1e-12 in size. Mirrors
/// (negative determinant), scale and skew are allowed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<ModelPoint>")]
    pub origin: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_axis: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_axis: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_axis: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<[f64; 12]>,
}

impl Transform {
    /// The transform as an affine map, or `invalid_input` for a mix of forms, a number that
    /// is not finite or too large, or axes that are (nearly) singular. Never panics.
    pub fn to_affine(&self) -> Result<glam::DAffine3, CommandError> {
        use glam::{DAffine3, DMat3, DVec3};
        let bad = |m: &str| CommandError::new(ErrorKind::InvalidInput, m.to_string());
        let (x, y, z, o) = match self.matrix {
            Some(m) => {
                if self.origin.is_some()
                    || self.x_axis.is_some()
                    || self.y_axis.is_some()
                    || self.z_axis.is_some()
                {
                    return Err(bad("give either matrix or origin and axes, not both"));
                }
                (
                    [m[0], m[1], m[2]],
                    [m[3], m[4], m[5]],
                    [m[6], m[7], m[8]],
                    [m[9], m[10], m[11]],
                )
            }
            None => (
                self.x_axis.unwrap_or([1.0, 0.0, 0.0]),
                self.y_axis.unwrap_or([0.0, 1.0, 0.0]),
                self.z_axis.unwrap_or([0.0, 0.0, 1.0]),
                self.origin.unwrap_or([0.0; 3]),
            ),
        };
        let limit = crate::document::MAX_COORDINATE;
        if ![x, y, z, o]
            .iter()
            .flatten()
            .all(|c| c.is_finite() && c.abs() <= limit)
        {
            return Err(bad(&format!(
                "transform numbers must be finite and within +/-{limit}"
            )));
        }
        let linear = DMat3::from_cols(DVec3::from(x), DVec3::from(y), DVec3::from(z));
        let det = linear.determinant();
        if !det.is_finite() || det.abs() < kernel::MIN_DETERMINANT {
            return Err(bad("the transform's axes are singular (zero volume)"));
        }
        Ok(DAffine3 {
            matrix3: linear,
            translation: DVec3::from(o),
        })
    }
}
