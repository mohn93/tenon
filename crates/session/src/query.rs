use std::collections::BTreeMap;

use kernel::{
    CamFollowerId, ChangeSummary, CouplingId, CurveKind, DefinitionId, DefinitionKind, EntityId,
    ImageFormat, JointId, JointKind, MaterialId, PageId, StyleId, StyleSettings, TagId,
    TexturePlacement,
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
    /// Volume, mass, centre of gravity and inertia of instances of the open context (`ids`:
    /// instances, repeats counted once; empty means every instance of it), each and in total,
    /// in the context's coordinates. A part's density is that of the material most of its faces
    /// show on the outside (their front, or their back when the solid faces inward), else its
    /// instance's own material's, else the nearest enclosing instance's (the instances the open
    /// context sits in included), passing over materials without a density. `mass`, `inertia`
    /// and `principal` are absent when a contributing solid has no density; without a mass
    /// `centroid` is the volume's centroid. `inertia` is about the centroid, in kg m2;
    /// `principal` gives its moments (ascending) and axes. Loose geometry that is not closed, or
    /// has a face that cannot be triangulated, is listed in `skipped` and ignored.
    MassProperties {
        #[serde(default)]
        ids: Vec<EntityId>,
    },
    /// The cut list of the open context: its groups and components (`ids`: those listed; every
    /// one when empty) broken down to parts, each a definition's own shown faces; a group holding
    /// groups or components lists the ones inside, not itself. Each row has the part's name,
    /// material, length, width and thickness (the part's own box along its own axes, scaled by
    /// its placement, longest first), quantity and whether it is a solid (with a note why not).
    /// `group_by` `definition` (default) merges parts of one definition, material and size;
    /// `none` has a row per part. Rows are ordered by material, then size. Never changes the
    /// model.
    CutList {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        group_by: CutListGrouping,
    },
    /// Measure the model, exactly one of `distance`, `angle` and `round`. Entities are named by
    /// id in the open context, or by a path of instances from it ending in the entity
    /// (`["i1", "i2", "f3"]`), in which case the placements of the instances apply and results
    /// are in open-context coordinates. `distance`: the minimum distance between two entities
    /// (vertices, edges, faces, instances, guides) and the closest point on each; faces are
    /// planar polygons with holes, instances are measured between their faces (surface to
    /// surface: an instance inside another is the gap between their faces, not 0), and faces
    /// that cannot be triangulated are left out and listed in `skipped`. `angle`: between
    /// two edges (directions) or two faces (normals), 0 to 180 degrees. `round`: the circle
    /// (an edge of a circle or arc, or a flat disc face) or cylinder (a face joined by soft
    /// edges to others around one axis) the entity lies on, fitted by least squares with the
    /// largest deviation of its vertices from the fit.
    Measure {
        #[serde(default)]
        distance: Option<[MeasureRef; 2]>,
        #[serde(default)]
        angle: Option<[MeasureRef; 2]>,
        #[serde(default)]
        round: Option<MeasureRef>,
    },
    /// Which solid parts of the open context overlap: `ids` are its groups and components
    /// (every one when empty), each a part with all it places at any depth. Pairs sharing a
    /// volume, largest first; with `touching`, also pairs whose surfaces meet with no volume
    /// in common. Parts that are not solid are skipped with why. Never changes the model.
    Interference {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        touching: bool,
    },
    /// How near parts come: the minimum distance between the surfaces of `a` and `b`, or of
    /// every pair among `ids` (2 to `MAX_CLEARANCE_IDS`), with the nearest points; 0 when
    /// they touch or overlap. Never changes the model.
    Clearance {
        #[serde(default)]
        a: Option<EntityId>,
        #[serde(default)]
        b: Option<EntityId>,
        #[serde(default)]
        ids: Vec<EntityId>,
    },
    /// Turns joint `joint` through a range on a copy of the model and checks the root's solid
    /// parts at each step: `steps` (2 to `MAX_SWEEP_STEPS`) evenly spaced values from `from`
    /// to `to` (degrees or meters, as `drive`), `slide` held for a cylindrical joint. `ids`
    /// limits the parts (root groups and components; every one when empty). Reports each pair
    /// that comes within `report_within` meters (default 0.005) at some step. Never changes
    /// the model.
    SweepCheck {
        joint: JointId,
        from: f64,
        to: f64,
        steps: u32,
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        slide: Option<f64>,
        #[serde(default)]
        report_within: Option<f64>,
    },
    Camera {},
    /// Where the model stands (see `set_location`).
    Location {},
    /// How going to a scene animates in the window (see `set_scene_transition`).
    SceneTransition {},
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
    /// Every joint, in the order they were made (see `add_joint`).
    Joints {},
    /// One joint.
    Joint {
        id: JointId,
    },
    /// Every coupling between joints, in the order they were made (see `add_coupling`).
    Couplings {},
    /// Every cam follower, in the order they were made (see `add_cam_follower`).
    CamFollowers {},
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

/// Most parts a `clearance` query compares pairwise.
pub const MAX_CLEARANCE_IDS: usize = 200;

/// Two overlapping parts: the volume they share in cubic meters of the open context and a
/// point inside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InterferencePair {
    pub a: EntityId,
    pub b: EntityId,
    pub volume: f64,
    pub point: [f64; 3],
}

/// Two parts whose surfaces meet (at `point`) with no volume in common.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TouchingPair {
    pub a: EntityId,
    pub b: EntityId,
    pub point: [f64; 3],
}

/// A pair whose overlap could not be measured, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FailedPair {
    pub a: EntityId,
    pub b: EntityId,
    pub reason: String,
}

/// A part left out because it is not a solid part, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedPart {
    pub id: EntityId,
    pub reason: String,
}

/// The minimum distance between two parts' surfaces and the nearest point on each; 0 when
/// they touch or overlap (`overlap` says they share a volume).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClearancePair {
    pub a: EntityId,
    pub b: EntityId,
    pub distance: f64,
    pub point_a: [f64; 3],
    pub point_b: [f64; 3],
    pub overlap: bool,
}

/// Most steps a `sweep_check` takes.
pub const MAX_SWEEP_STEPS: u32 = 10_000;

/// One pair of parts over a sweep, joint values in degrees or meters: the first and last
/// values where they overlap (absent when they never do), the largest volume they share (0
/// when none) and where, the smallest clearance (0 when they touch or overlap) and the first
/// value where it occurs, and the first value where their overlap could not be measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SweepPair {
    pub a: EntityId,
    pub b: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<f64>,
    pub volume: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_at: Option<f64>,
    pub clearance: f64,
    pub clearance_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<SweepStop>,
}

/// A joint value (degrees or meters) and why the sweep stopped there, or why a pair's overlap
/// could not be measured there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SweepStop {
    pub value: f64,
    pub reason: String,
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

/// How going to a scene animates (see `set_scene_transition`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SceneTransitionInfo {
    pub on: bool,
    pub seconds: f64,
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

/// An entity named by id, or by a path of instances ending in the entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MeasureRef {
    Id(EntityId),
    Path(Vec<EntityId>),
}

impl MeasureRef {
    pub fn path(&self) -> Vec<EntityId> {
        match self {
            MeasureRef::Id(id) => vec![*id],
            MeasureRef::Path(p) => p.clone(),
        }
    }
}

/// The minimum distance between two entities (see `Query::Measure`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DistanceInfo {
    /// Meters; 0 when they touch or cross.
    pub distance: f64,
    /// The closest point on the first and on the second entity.
    pub points: [[f64; 3]; 2],
    /// Faces of measured instances that could not be triangulated and were left out, as paths
    /// from the instance ("i4/i7/f12"); absent when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

/// The angle between two edges or two faces (see `Query::Measure`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AngleInfo {
    /// 0 to 180: between the edges' directions (each from its first to its second vertex) or
    /// the faces' normals.
    pub degrees: f64,
    /// 0 to 90: between the lines or planes, whichever way they point.
    pub acute: f64,
}

/// What a round feature is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoundKind {
    Circle,
    Cylinder,
}

/// A circle or cylinder fitted to a round feature (see `Query::Measure`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RoundInfo {
    pub kind: RoundKind,
    /// The circle's center; the middle of the cylinder's axis segment.
    pub center: [f64; 3],
    /// The unit axis, pointing up (positive z, else y, else x).
    pub axis: [f64; 3],
    pub radius: f64,
    /// A cylinder's extent along its axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// The largest distance of a fitted vertex from the circle or cylinder, so the error of a
    /// faceted surface shows.
    pub max_deviation: f64,
    /// Vertices fitted.
    pub samples: usize,
}

/// How the `cut_list` query merges parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CutListGrouping {
    /// Parts of one definition, material and size are one row.
    #[default]
    Definition,
    /// A row per part.
    None,
}

/// One row of the cut list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CutRowInfo {
    pub name: String,
    /// The material's name; absent when the part has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Meters, the longest side of the part's own box.
    pub length: f64,
    pub width: f64,
    pub thickness: f64,
    pub quantity: usize,
    /// Whether the part is a closed solid.
    pub solid: bool,
    /// Why it is not a solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The groups and components of the open context the parts belong to.
    pub ids: Vec<EntityId>,
}

/// Mass properties of one instance or of a sum of instances (see `Query::MassProperties`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MassInfo {
    /// Cubic meters.
    pub volume: f64,
    /// Kilograms; absent when a contributing solid has no density.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass: Option<f64>,
    pub centroid: [f64; 3],
    /// The 3x3 inertia tensor about the centroid (rows), kg m2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inertia: Option<[[f64; 3]; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<PrincipalInfo>,
}

/// The principal moments of inertia, ascending, and their unit axes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PrincipalInfo {
    pub moments: [f64; 3],
    pub axes: [[f64; 3]; 3],
}

/// An instance's mass properties.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstanceMassInfo {
    pub id: EntityId,
    #[serde(flatten)]
    pub mass: MassInfo,
}

/// A part left out of a mass sum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedInfo {
    /// The instance path, "i3/i7"; "i3" alone for its own loose geometry.
    pub path: String,
    pub reason: String,
}

impl From<&kernel::mass::MassProps> for MassInfo {
    fn from(p: &kernel::mass::MassProps) -> Self {
        MassInfo {
            volume: p.volume,
            mass: p.mass,
            centroid: p.centroid.to_array(),
            // glam matrices are column major and this one is symmetric; rows are written.
            inertia: p.inertia.map(|i| i.transpose().to_cols_array_2d()),
            principal: p.principal.map(|pr| PrincipalInfo {
                moments: pr.moments,
                axes: pr.axes.map(|a| a.to_array()),
            }),
        }
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
    /// Its density in kg/m³; absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
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
    /// The instances of the root whose positions it stores (empty: none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<EntityId>,
    /// The joints whose values it stores (empty: none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<JointId>,
    /// Whether it is the selected scene (the one last added or gone to).
    pub selected: bool,
}

/// A joint value a scene stores: degrees for an angle, meters for a distance (and `slide`, for
/// a cylindrical joint).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SceneJointValue {
    pub joint: JointId,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide: Option<f64>,
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

/// A joint in the `joints` list. `value` and `limits` are degrees for revolute and cylindrical
/// joints and meters otherwise (0 for fixed and ball joints); `slide` (meters) only for a
/// cylindrical one. `a` is null for the ground. `point` and `axis` are the joint's frame on
/// part A (the ground's, in the root's coordinates), in the root's coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JointInfo {
    pub id: JointId,
    pub name: String,
    pub kind: JointKind,
    pub a: Option<EntityId>,
    pub b: EntityId,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<[f64; 2]>,
    pub point: [f64; 3],
    pub axis: [f64; 3],
}

/// A coupling in the `couplings` list: `driven = ratio * driver + offset`, in the joints'
/// units (degrees or meters; `ratio` is driven units per driver unit, `offset` driven units).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CouplingInfo {
    pub id: CouplingId,
    pub driver: JointId,
    pub driven: JointId,
    pub ratio: f64,
    pub offset: f64,
}

/// A cam follower in the `cam_followers` list: the joint `follower` follows the surface of
/// instance `cam`, pushed onto it along `preload_direction` (a unit vector along the joint's
/// axis, in the root's coordinates), with a roller of `roller_radius` meters (0: flat or
/// knife).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CamFollowerInfo {
    pub id: CamFollowerId,
    pub cam: EntityId,
    pub follower: JointId,
    pub roller_radius: f64,
    pub preload_direction: [f64; 3],
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
    /// The `mass_properties` query: each instance that counts and the sum (absent when none
    /// does).
    MassProperties {
        instances: Vec<InstanceMassInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<MassInfo>,
        skipped: Vec<SkippedInfo>,
    },
    /// The `cut_list` query.
    CutList {
        rows: Vec<CutRowInfo>,
    },
    /// The `measure` query: the one of `distance`, `angle` and `round` asked for.
    Measure {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distance: Option<DistanceInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle: Option<AngleInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        round: Option<RoundInfo>,
    },
    /// The `interference` query: overlapping pairs, largest volume first; `touching` only
    /// when asked; `failed` only when a pair's overlap could not be measured.
    Interference {
        pairs: Vec<InterferencePair>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        touching: Option<Vec<TouchingPair>>,
        skipped: Vec<SkippedPart>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        failed: Vec<FailedPair>,
        revision: u64,
    },
    /// The `clearance` query: one entry per pair of solid parts, in the order asked.
    Clearance {
        pairs: Vec<ClearancePair>,
        skipped: Vec<SkippedPart>,
        revision: u64,
    },
    /// The `sweep_check` query: pairs in order of ids; `steps` checked; `stopped` only when a
    /// value the solver could not meet ended the sweep.
    SweepCheck {
        pairs: Vec<SweepPair>,
        skipped: Vec<SkippedPart>,
        steps: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stopped: Option<SweepStop>,
        revision: u64,
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
    SceneTransition(SceneTransitionInfo),
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
    /// The joints, in the order they were made.
    Joints {
        joints: Vec<JointInfo>,
        revision: u64,
    },
    Joint(JointInfo),
    /// The couplings, in the order they were made.
    Couplings {
        couplings: Vec<CouplingInfo>,
        revision: u64,
    },
    /// The cam followers, in the order they were made.
    CamFollowers {
        cam_followers: Vec<CamFollowerInfo>,
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
        /// The instances of the root whose positions it stores, null when it stores none.
        positions: Option<Vec<EntityId>>,
        /// The joint values it stores, null when it stores none.
        joints: Option<Vec<SceneJointValue>>,
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
