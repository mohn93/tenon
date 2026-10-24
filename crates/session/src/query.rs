use std::collections::BTreeMap;

use kernel::{
    ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId, ImageFormat, MaterialId,
    PageId, StyleId, StyleSettings, TagId, TexturePlacement,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::{Lighting, Projection};
use crate::command::Origin;
use crate::library::LibraryListing;
use crate::units::Units;

/// Read-only questions about the document. Entities and coordinates are those of the open
/// context (see `Command::OpenInstance`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// Vertices, edges, faces, instances and guides. An entity is returned when `ids` is empty
    /// or lists it, `kinds` is empty or lists its kind, and `within` is absent or contains all
    /// its vertices (an instance: its bounding box; a guide point: its position; a guide line
    /// is in no box); unknown ids are simply absent from the result.
    Entities {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        kinds: Vec<EntityKind>,
        #[serde(default)]
        within: Option<Bounds>,
    },
    /// One entity by id ("v3", "e7", "f2", "i4", "g9", "r5", "n6", "s8").
    Entity {
        id: EntityId,
    },
    /// The curve (circle or arc) an edge is part of, given any of its edges.
    Curve {
        id: EntityId,
    },
    /// Whether a group or component is a solid: its own geometry is closed (every edge bounds
    /// two faces, consistently oriented) and it holds no nested groups or components.
    /// Gives its volume in cubic meters of the open context (after the placement's scale),
    /// or why it is not one.
    Solid {
        id: EntityId,
    },
    Camera {},
    /// Where the model stands (see `set_location`).
    Location {},
    /// The sun at the shadow settings' date and time, whether or not shadows are on.
    Sun {},
    Stats {},
    History {},
    /// The selected edges, faces, instances, guides, dimensions and notes.
    Selection {},
    /// Every group and component definition.
    Definitions {},
    /// Every material.
    Materials {},
    /// Every tag, with how many entities are on it.
    Tags {},
    /// Every scene, in order: what each stores and which is selected.
    Scenes {},
    /// The styles (see `add_style`): id, name and settings of each, in the order they were
    /// made. `set_style` chooses one; the `camera` query tells which is chosen.
    Styles {},
    /// One scene: the view and the hidden tags it stores.
    Scene {
        scene: PageId,
    },
    /// The open context: the path of instances being edited, from the root.
    Context {},
    /// The tree of placed groups and components: the root's instances, each with the
    /// instances its definition places, down to `depth` levels below the root's instances
    /// (0: the root's instances only; absent: every level).
    Outliner {
        #[serde(default)]
        depth: Option<u32>,
    },
    /// The file the model was last opened from or saved to, and whether it changed since.
    File {},
    /// Model changes after `since` (a revision number), oldest first.
    Events {
        since: u64,
    },
    /// Every dimension and note of the open context, with its label text.
    Annotations {},
    /// The model's display units (see `set_units`).
    Units {},
    /// Every section plane of the open context, with whether it is the active one.
    SectionPlanes {},
    /// How section planes are shown (see `set_section_display`).
    SectionDisplay {},
    /// The library items in `folder`: the `.skr` files directly in it (no links, no
    /// subfolders; at most 1000, sorted by name ignoring case), each with its name, what it
    /// holds (`component`, `material` or `model`), when it was written and, for a file that
    /// is no usable model, an `error`. `more` is true when the folder holds more.
    Library {
        folder: String,
    },
}

/// An axis-aligned box, bounds included.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub fn contains(&self, p: [f64; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
    }
}

/// The kinds of entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Vertex,
    Edge,
    Face,
    Instance,
    Guide,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VertexInfo {
    pub id: EntityId,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeInfo {
    pub id: EntityId,
    pub vertices: [EntityId; 2],
    pub start: [f64; 3],
    pub end: [f64; 3],
    /// The curve the edge is part of ("c12"), absent for a plain edge. Edges with the same
    /// curve form one circle or arc; the `curve` query describes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    /// True for a soft edge: hidden, between faces that form one smooth surface. Absent when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub soft: bool,
    /// True for a hidden edge (see `hide`). Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

/// A curve: a chain of edges drawn as one circle or arc. Its shape is worked out from where
/// its edges are now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CurveInfo {
    /// "c12": the curve id its edges report.
    pub id: String,
    pub kind: CurveKind,
    /// In order along the curve, counter-clockwise around `normal` when there is one.
    pub edges: Vec<EntityId>,
    /// In order; a circle's first vertex is not repeated at the end.
    pub vertices: Vec<EntityId>,
    /// Straight segments; an edge other geometry split in two still counts once.
    pub segments: usize,
    /// Total length of the edges, in meters.
    pub length: f64,
    /// Center, unit normal (pointing up: +z, or on a vertical plane +y, else +x), radius (a
    /// polygon's corner radius) and swept angle in degrees (360 for a circle or polygon);
    /// absent when the corners are off a circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FaceInfo {
    pub id: EntityId,
    pub outer: Vec<EntityId>,
    pub holes: Vec<Vec<EntityId>>,
    pub normal: [f64; 3],
    pub area: f64,
    /// The material of the side the normal points to; absent for the default material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_material: Option<MaterialId>,
    /// The material of the other side; absent for the default material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_material: Option<MaterialId>,
    /// Where the texture lies on top of its projection (see `position_texture`); absent when
    /// it lies as projected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_position: Option<TexturePlacement>,
    /// True for a hidden face (see `hide`). Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

/// A placed group or component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstanceInfo {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub name: String,
    pub kind: DefinitionKind,
    /// Column-major 3x4 matrix mapping the definition's coordinates into the open context's:
    /// x axis, y axis, z axis, translation.
    pub transform: [f64; 12],
    /// The box around the instance's geometry, in the open context's coordinates. Absent for
    /// an empty definition.
    pub bounds: Option<Bounds>,
    /// The material shown on face sides inside it without one of their own; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    /// True for a hidden instance (see `hide`; not for one on a hidden tag). Absent when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// False for an instance section planes outside it do not cut (see `set_section_cut`).
    /// Absent when true.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub section_cut: bool,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// Whether a guide is a line or a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuideKind {
    Line,
    Point,
}

/// A guide line (through `point` along the unit vector `direction`) or a guide point (at
/// `point`, no `direction`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GuideInfo {
    pub id: EntityId,
    pub kind: GuideKind,
    pub point: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 3]>,
    /// True for a hidden guide (see `hide`). Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

/// The model axes: `origin` and the unit vectors `x` (red), `y` (green) and `z` (blue).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AxesInfo {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

/// The sun's shadow settings: `date` as `YYYY-MM-DD`, `time` as `HH:MM` (the clock at the
/// model's UTC offset), `light` and `dark` as percentages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShadowsInfo {
    pub on: bool,
    pub date: String,
    pub time: String,
    pub light: u8,
    pub dark: u8,
    pub on_ground: bool,
}

impl From<kernel::Shadows> for ShadowsInfo {
    fn from(s: kernel::Shadows) -> Self {
        ShadowsInfo {
            on: s.on,
            date: format!("{:04}-{:02}-{:02}", s.year, s.month, s.day),
            time: format!("{:02}:{:02}", s.minutes / 60, s.minutes % 60),
            light: s.light,
            dark: s.dark,
            on_ground: s.on_ground,
        }
    }
}

/// Where the model stands (see `set_location`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LocationInfo {
    pub latitude: f64,
    pub longitude: f64,
    pub north_angle: f64,
    pub utc_offset: f64,
}

impl From<kernel::Location> for LocationInfo {
    fn from(l: kernel::Location) -> Self {
        LocationInfo {
            latitude: l.latitude,
            longitude: l.longitude,
            north_angle: l.north_angle,
            utc_offset: l.utc_offset,
        }
    }
}

/// The sun in the sky and the model (see the `sun` query).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SunInfo {
    pub direction: [f64; 3],
    pub azimuth: f64,
    pub elevation: f64,
    pub up: bool,
}

impl From<crate::sun::Sun> for SunInfo {
    fn from(s: crate::sun::Sun) -> Self {
        SunInfo {
            direction: s.direction.to_array(),
            azimuth: s.azimuth,
            elevation: s.elevation,
            up: s.is_up(),
        }
    }
}

/// A section plane: through `point` with the unit vector `normal`, keeping the side the normal
/// points to; `active` when it is the one cutting its context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SectionPlaneInfo {
    pub id: EntityId,
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub active: bool,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

/// A dimension end or a leader note's anchor, where it is now: on vertex `vertex`, which it
/// follows, or (no `vertex`) a fixed point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnchorInfo {
    pub point: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex: Option<EntityId>,
}

/// Whether a dimension measures a distance or a circle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    Linear,
    Radial,
}

/// A dimension. Linear: the distance between `a` and `b`, drawn along `line` (the two ends
/// moved by `offset`, which is perpendicular to them). Radial: the radius (or with `diameter`
/// the diameter) of the circle at `center` with `normal` and `radius`, measuring curve
/// `curve` while it is attached to one, labelled at `center + leader`. `value` is the length
/// measured in meters, `label` how it is written in the display units ("R 1.5 m", "Ø 3 m").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DimensionInfo {
    pub id: EntityId,
    pub kind: DimensionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<AnchorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<AnchorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<[[f64; 3]; 2]>,
    /// "c12" while the dimension follows a curve; absent for a fixed circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// True for a radial dimension measuring the diameter. Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diameter: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader: Option<[f64; 3]>,
    pub value: f64,
    pub label: String,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

/// Whether a note points at the model or is fixed to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Leader,
    Screen,
}

/// A note. Leader: text at `position` with a line to `anchor`. Screen: text fixed to the view
/// at `at`, fractions (0 to 1) of its width and height from the top left corner. `text` is
/// what was written (absent for a leader note showing its anchor's coordinates); `label` is
/// what is shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NoteInfo {
    pub id: EntityId,
    pub kind: NoteKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<AnchorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub label: String,
    /// The tag it is on; null when Untagged.
    pub tag: Option<TagId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DefinitionInfo {
    pub id: DefinitionId,
    pub name: String,
    pub kind: DefinitionKind,
    /// How many instances place it, in all contexts.
    pub instances: usize,
    pub edges: usize,
    pub faces: usize,
}

/// A placed group or component in the `outliner` tree. A component's contents appear under
/// each of its instances, with the same ids; a node's path is the ids from the root down to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OutlinerNode {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    /// The definition's name; empty for an unnamed group.
    pub name: String,
    /// The instance's material; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    /// True when the node is on the open context's path (the open instance or one it is in).
    /// Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    /// True when the node is an instance of the open context and is selected. Absent when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    /// True when the node is hidden: its own `hide` flag or hidden tag, or that of an
    /// instance above it. Hidden nodes are still listed. Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// True when the node's own `hide` flag is set (it is hidden itself, not only through a
    /// tag or an instance above). Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own_hidden: bool,
    /// The instances its definition places, in id order.
    pub children: Vec<OutlinerNode>,
    /// True when the query's `depth` left out this node's children (it has some). Absent
    /// when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// One placed group or component in an `OutlinerTable`: what `OutlinerNode` says about it
/// apart from the marks and children, which depend on where it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlinerRow {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    /// The definition's name; empty for an unnamed group.
    pub name: String,
    pub material: Option<MaterialId>,
    /// True when the instance is hidden or on a hidden tag (instances above it may hide it
    /// too).
    pub hidden: bool,
    /// True when the instance's own hide flag is set.
    pub own_hidden: bool,
}

/// The outliner tree stored flat (`Document::outliner_rows`): the root's instances and, for
/// each definition, the instances it places. Its size is linear in the model however deep the
/// nesting; a node's children are `children(node.definition)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlinerTable {
    pub roots: Vec<OutlinerRow>,
    pub by_definition: BTreeMap<DefinitionId, Vec<OutlinerRow>>,
}

impl OutlinerTable {
    /// The instances `definition` places, in id order (none for an unknown definition).
    pub fn children(&self, definition: DefinitionId) -> &[OutlinerRow] {
        self.by_definition
            .get(&definition)
            .map_or(&[], Vec::as_slice)
    }
}

/// A material and how much of the model uses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaterialInfo {
    pub id: MaterialId,
    pub name: String,
    /// sRGB red, green and blue, 0 to 255.
    pub color: [u8; 3],
    /// From 0 (invisible) to 1 (opaque). Always present.
    pub opacity: f32,
    /// How strongly its faces glow, from 0 (not at all) to 10. Always present.
    pub emission: f32,
    /// How metallic its surface is, from 0 (not at all) to 1. Always present.
    pub metalness: f32,
    /// How rough its surface is, from 0 (a mirror) to 1 (matte). Always present.
    pub roughness: f32,
    /// The texture faces painted with it show; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureInfo>,
    /// Faces painted with it on either side, in the whole model (each definition counted once).
    pub faces: usize,
    /// Instances painted with it, in the whole model.
    pub instances: usize,
}

/// A tag and how much of the model is on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TagInfo {
    pub id: TagId,
    pub name: String,
    /// sRGB red, green and blue, 0 to 255.
    pub color: [u8; 3],
    /// False when what is on it is hidden.
    pub visible: bool,
    /// Edges, faces, instances, guides, dimensions and notes on it, in the whole model (each
    /// definition counted once).
    pub count: usize,
}

/// A scene in the `scenes` list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SceneInfo {
    pub id: PageId,
    pub name: String,
    /// Whether it stores a view (camera, projection, lighting, color by tag).
    pub camera: bool,
    /// Whether it stores which tags are hidden.
    pub tags: bool,
    /// Whether it is the selected scene (the one last added or gone to).
    pub selected: bool,
}

/// The view a scene stores, in the `camera` query's terms.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SceneView {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub fov_y_deg: f64,
    pub projection: Projection,
    pub lighting: Lighting,
    pub color_by_tag: bool,
    pub x_ray: bool,
    pub hidden_geometry: bool,
    pub shadows: ShadowsInfo,
    /// The style the scene chooses; absent when it chooses none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleId>,
}

/// A style in the `styles` list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StyleInfo {
    pub id: StyleId,
    pub name: String,
    pub settings: StyleSettings,
}

/// A material's texture: the size one copy covers on the model, in meters, and the image's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextureInfo {
    pub width: f64,
    pub height: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub format: ImageFormat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    Entities {
        vertices: Vec<VertexInfo>,
        edges: Vec<EdgeInfo>,
        faces: Vec<FaceInfo>,
        instances: Vec<InstanceInfo>,
        guides: Vec<GuideInfo>,
        revision: u64,
    },
    Vertex(VertexInfo),
    Edge(EdgeInfo),
    Face(FaceInfo),
    Instance(InstanceInfo),
    Guide(GuideInfo),
    Dimension(DimensionInfo),
    Note(NoteInfo),
    SectionPlane(SectionPlaneInfo),
    Curve(CurveInfo),
    /// The `solid` query: `volume` is present exactly when `solid` is true; `reason` says why
    /// not otherwise (e.g. "edge e12 bounds 1 face").
    Solid {
        solid: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        volume: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// The view: `up` is the screen's up direction (+Z, except in a view straight down or
    /// up); `view_height` is the height the view spans at the target (in a parallel view, the
    /// whole view's height); `lighting` is `day` or `night`; `color_by_tag` is whether faces
    /// are drawn in their tags' colors; `x_ray` whether every shown face is drawn as glass;
    /// `hidden_geometry` whether hidden entities are drawn ghosted and can be picked.
    Camera {
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        fov_y_deg: f64,
        projection: Projection,
        view_height: f64,
        lighting: Lighting,
        color_by_tag: bool,
        x_ray: bool,
        hidden_geometry: bool,
        /// The sun's shadow settings (see `set_shadows`).
        shadows: ShadowsInfo,
        /// The style the view draws with (see `set_style`); absent when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<StyleId>,
        /// The model axes (see `set_axes`).
        axes: AxesInfo,
    },
    Location(LocationInfo),
    /// The sun: `direction` the unit vector from the model towards it in the model's x, y and
    /// z, `azimuth` the degrees clockwise from true north and `elevation` the degrees above
    /// the horizon, `up` whether it is above the horizon.
    Sun(SunInfo),
    /// Counts for the open context, plus the number of definitions.
    Stats {
        vertices: usize,
        edges: usize,
        faces: usize,
        instances: usize,
        definitions: usize,
        revision: u64,
    },
    /// Undo history: how many steps can be undone and redone, and their labels (the `type` of
    /// the command that made each, or `batch` for an atomic batch of differing commands), the
    /// next to undo or redo first. `undo_origins` and `redo_origins` say who made each step, in
    /// the same order.
    History {
        undo_depth: usize,
        redo_depth: usize,
        undo: Vec<String>,
        redo: Vec<String>,
        undo_origins: Vec<Origin>,
        redo_origins: Vec<Origin>,
        revision: u64,
    },
    Selection {
        ids: Vec<EntityId>,
        revision: u64,
    },
    Definitions {
        definitions: Vec<DefinitionInfo>,
        revision: u64,
    },
    Materials {
        materials: Vec<MaterialInfo>,
        revision: u64,
    },
    Tags {
        tags: Vec<TagInfo>,
        revision: u64,
    },
    /// The styles, in the order they were made, each with its settings.
    Styles {
        styles: Vec<StyleInfo>,
        revision: u64,
    },
    Scenes {
        scenes: Vec<SceneInfo>,
        revision: u64,
    },
    /// `view` is null when the scene does not store a view, `hidden_tags` when it does not
    /// store tag visibility (else the tags it hides; every other tag is shown).
    Scene {
        id: PageId,
        name: String,
        view: Option<SceneView>,
        hidden_tags: Option<Vec<TagId>>,
        revision: u64,
    },
    /// `path` lists the open instances from the root; empty when the root is open.
    /// `definition` is the definition being edited.
    Context {
        path: Vec<EntityId>,
        definition: Option<DefinitionId>,
    },
    /// The root's instances, in id order, each with what it places.
    Outliner {
        instances: Vec<OutlinerNode>,
        revision: u64,
    },
    /// `path` is null for a model never saved. `modified` is true when the model changed
    /// since it was opened, saved or started.
    File {
        path: Option<String>,
        modified: bool,
        revision: u64,
    },
    /// `truncated` is true when changes after `since` were dropped from the log; the client
    /// should re-read the model (e.g. with `entities`) instead of applying `events`.
    Events {
        revision: u64,
        events: Vec<ChangeEvent>,
        truncated: bool,
    },
    /// The open context's dimensions and notes, in id order.
    Annotations {
        dimensions: Vec<DimensionInfo>,
        notes: Vec<NoteInfo>,
        revision: u64,
    },
    Units(Units),
    /// The open context's section planes, in id order.
    SectionPlanes {
        planes: Vec<SectionPlaneInfo>,
        revision: u64,
    },
    /// Whether the plane entities are drawn, the active planes' cuts applied and closed cuts
    /// filled.
    SectionDisplay {
        planes: bool,
        cuts: bool,
        fill: bool,
        fill_color: crate::document::SectionFillColor,
    },
    /// The items of a library folder.
    Library(LibraryListing),
}

/// One model change: the revision it produced and what it changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeEvent {
    pub revision: u64,
    pub changes: ChangeSummary,
    /// The display units, when the change set them (`set_units`, or an undo or redo of such a
    /// step); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
}
