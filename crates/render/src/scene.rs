//! GPU vertex data built from a kernel scene.
//!
//! It comes in two layers, so that a selection change does not rebuild the scene:
//! - `scene_base` places every face, edge and guide for an open context, with nothing selected,
//!   and notes where each entity of the open context lies (`Spans`);
//! - `Spans::flags` marks the vertices and guides a selection lights up.
//!
//! `scene_geometry` combines the two. `scene_base` also gathers the scene's lights (see
//! `lights`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytemuck::{Pod, Zeroable};
use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    DimensionId, EdgeId, EntityId, FaceId, Guide, GuideId, InstanceId, MaterialId, MeshId, Model,
    NoteId, Scene, SectionId, SectionPlane, TagId, VertexId, back_texture_position,
};
use session::{AnnotationShape, SectionDisplay, SectionShape};

use crate::annotations::{ANNOTATION_COLOR, AnnotationLook, FADED_ANNOTATION_COLOR};
use crate::grid::LineVertex;
use crate::guides::{FADED_GUIDE_COLOR, GUIDE_COLOR, GuideShape};
use crate::lights::{Glow, Light, connected};
use crate::sections::{
    ACTIVE_SECTION_COLOR, CUT_FILL_COLOR, Cut, FADED_ACTIVE_SECTION_COLOR, SectionLook,
    clip_segment, clip_triangle, cut_surface,
};
use crate::texture::layers;

/// A face corner with its selection flag, as `scene_geometry` returns it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// 1 for selected faces, 0 otherwise.
    pub selected: f32,
    /// The colors of the front and back sides.
    pub front: [f32; 3],
    pub back: [f32; 3],
    /// The opacities of the front and back sides (1: opaque).
    pub alpha: [f32; 2],
    /// The texture coordinates of the front (first two) and back sides (see `BaseFaceVertex`).
    pub uv: [f32; 4],
    /// The texture array layers of the front and back sides; -1 for none.
    pub layer: [f32; 2],
    /// 1 on a side whose material glows, 0 otherwise (see `BaseFaceVertex`).
    pub glow: [f32; 2],
}

/// A face corner as the renderer uploads it; whether it is selected comes from a separate flag
/// buffer (see `Flags`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct BaseFaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// The colors of the front and back sides.
    pub front: [f32; 3],
    pub back: [f32; 3],
    /// The opacities of the front and back sides (1: opaque). A triangle with either below 1
    /// is drawn in the translucent pass (see `SceneBase::glass`).
    pub alpha: [f32; 2],
    /// The texture coordinates of the front (first two) and back sides: the corner's texture
    /// position on that side (see `kernel::Face::texture_position`; the back's is mirrored so
    /// it reads as seen from behind) over its side's texture size, v counted down from the
    /// image's top, shifted by whole repeats per triangle so they stay small. Zero on a side
    /// without a texture.
    pub uv: [f32; 4],
    /// The texture array layers of the front and back sides (see `texture::layers`); -1 for
    /// none.
    pub layer: [f32; 2],
    /// 1 on a side whose material glows, 0 otherwise: such a side is drawn unshaded at its
    /// color (or texture).
    pub glow: [f32; 2],
}

impl BaseFaceVertex {
    /// True when either side lets what is behind it show.
    pub fn is_glass(&self) -> bool {
        self.alpha[0] < 1.0 || self.alpha[1] < 1.0
    }
}

/// The default material's color on a face's front.
pub const DEFAULT_FRONT: [f32; 3] = [0.93, 0.93, 0.90];
/// The default material's color on a face's back.
pub const DEFAULT_BACK: [f32; 3] = [0.60, 0.65, 0.76];

/// How a side looks: its color (sRGB, as the targets store it), opacity, texture (its array
/// layer and its size in meters), which shows instead of the color, and how strongly it glows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    color: [f32; 3],
    alpha: f32,
    texture: Option<(u32, DVec2)>,
    emission: f32,
}

/// The array layer of each textured material (see `texture::layers`).
type Layers = HashMap<MaterialId, u32>;

/// How a side looks: as its material, or opaque in `default`.
fn side_look(
    scene: &Scene,
    layers: &Layers,
    material: Option<MaterialId>,
    default: [f32; 3],
) -> Look {
    let plain = Look {
        color: default,
        alpha: 1.0,
        texture: None,
        emission: 0.0,
    };
    let Some((id, m)) = material.and_then(|id| Some((id, scene.material(id)?))) else {
        return plain;
    };
    Look {
        color: m.color.map(|c| f32::from(c) / 255.0),
        alpha: m.opacity,
        texture: m
            .texture
            .as_ref()
            .zip(layers.get(&id))
            .map(|(t, &layer)| (layer, DVec2::new(t.width, t.height))),
        emission: m.emission,
    }
}

/// How a side looks when faces are colored by tag (see `scene_base_with`): opaque in the
/// tag's color for a face on a tag, else as `material` without its texture.
fn tag_colored(scene: &Scene, tag: Option<TagId>, material: Look) -> Look {
    match tag.and_then(|t| scene.tag(t)) {
        Some(t) => Look {
            color: t.color.map(|c| f32::from(c) / 255.0),
            alpha: 1.0,
            texture: None,
            emission: 0.0,
        },
        None => Look {
            texture: None,
            ..material
        },
    }
}

impl Look {
    /// The texture array layer, or -1 without a texture.
    fn layer(&self) -> f32 {
        self.texture.map_or(-1.0, |(layer, _)| layer as f32)
    }

    /// 1 when the side glows, else 0.
    fn glow(&self) -> f32 {
        if self.emission > 0.0 { 1.0 } else { 0.0 }
    }

    /// A triangle's texture coordinates on this side from its corners' texture positions
    /// (meters): over the texture's size, v counted down from the image's top, shifted by
    /// whole repeats so f32 keeps them exact. Zero without a texture.
    fn uv(&self, positions: [DVec2; 3]) -> [[f32; 2]; 3] {
        let Some((_, size)) = self.texture else {
            return [[0.0; 2]; 3];
        };
        let st = positions.map(|p| DVec2::new(p.x / size.x, -p.y / size.y));
        let shift = st[0].floor();
        st.map(|q| (q - shift).as_vec2().to_array())
    }
}

/// How a face with these sides glows: the stronger side's emission and color (the front's on a
/// tie).
fn face_glow(front: &Look, back: &Look) -> (f32, [f32; 3]) {
    let side = if back.emission > front.emission {
        back
    } else {
        front
    };
    (side.emission, side.color)
}

/// The corners of a triangle with both sides' texture coordinates (see `BaseFaceVertex::uv`),
/// from the corners' positions on the front's texture.
fn both_uv(front: &Look, back: &Look, positions: [DVec2; 3]) -> [[f32; 4]; 3] {
    let (f, b) = (
        front.uv(positions),
        back.uv(positions.map(back_texture_position)),
    );
    [0, 1, 2].map(|k| [f[k][0], f[k][1], b[k][0], b[k][1]])
}

pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
/// Selected edges and guides. scene.wgsl repeats it as `SELECTED_EDGE`.
pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];

/// Edges outside the open instance, while one is open.
pub const FADED_EDGE_COLOR: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
/// The box around the open instance.
pub const CONTEXT_BOX_COLOR: [f32; 4] = [0.45, 0.45, 0.45, 1.0];

/// Triangles for every face, line pairs for every edge, and the guides and annotations (drawn
/// per frame, see `guides::guide_lines` and `annotations::annotation_lines`).
#[derive(Debug, Default, PartialEq)]
pub struct ModelGeometry {
    pub faces: Vec<FaceVertex>,
    pub edges: Vec<LineVertex>,
    pub guides: Vec<GuideShape>,
    pub annotations: Vec<AnnotationLook>,
    pub sections: Vec<SectionLook>,
}

/// Geometry for the whole scene in root coordinates, every instance placed by its transform.
/// `context` is the path of open instances (see `session::Document::context`) and `selection`
/// holds ids of the open context: selected entities there are highlighted, and a selected
/// instance highlights everything it places. While an instance is open, edges and guides
/// outside it are faded and its bounding box is outlined. Each face side gets its material's
/// color, or else that of the nearest painted instance placing it, or else the default.
/// Faces, edges and guides on hidden tags are left out, and instances on hidden tags with all
/// they place (see `Scene::tag_shown`), but for the instances the open context is in.
///
/// This is `scene_base` with `Spans::flags` applied; the renderer keeps the two apart.
pub fn scene_geometry(
    scene: &Scene,
    context: &[InstanceId],
    selection: &BTreeSet<EntityId>,
) -> ModelGeometry {
    let base = scene_base(scene, context);
    let flags = base.spans.flags(selection);
    base.with_flags(&flags)
}

/// The scene placed for one open context with nothing selected (see `scene_geometry`).
#[derive(Debug, Default, PartialEq)]
pub struct SceneBase {
    pub faces: Vec<BaseFaceVertex>,
    /// Edges in EDGE_COLOR inside the open instance and FADED_EDGE_COLOR outside it, then the
    /// open instance's box.
    pub edges: Vec<LineVertex>,
    pub guides: Vec<GuideShape>,
    /// Dimensions and leader notes (screen notes have no lines), in ANNOTATION_COLOR inside
    /// the open instance and FADED_ANNOTATION_COLOR outside it.
    pub annotations: Vec<AnnotationLook>,
    /// The section planes drawn (see `SectionDisplay::planes`), in the guide colors (see `SectionLook`).
    pub sections: Vec<SectionLook>,
    /// The triangles filling the closed cuts of the active section planes, in
    /// `CUT_FILL_COLOR`. They are drawn after the faces, nearer than the faces they lie on.
    pub cut_fill: Vec<LineVertex>,
    /// The cuts' outlines as line pairs in the edge color, drawn wide.
    pub cut_edges: Vec<LineVertex>,
    /// Where the open context's entities lie in the lists above.
    pub spans: Spans,
    /// A light for each placement of a mesh whose own faces glow, in root coordinates. They do
    /// not depend on the open context.
    pub lights: Vec<Light>,
    /// The corners of the triangles that cast shadows, in root coordinates: those of faces
    /// with no glowing side, three a triangle.
    pub casters: Vec<[f32; 3]>,
    /// Where each placement's own casting triangles lie in `casters`, with their bounds, in
    /// the order placed; placements casting nothing have none.
    pub caster_chunks: Vec<CasterChunk>,
}

/// One placement's casting triangles in `SceneBase::casters`.
#[derive(Clone, Debug, PartialEq)]
pub struct CasterChunk {
    pub vertices: Range<u32>,
    /// The corners' bounding box.
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

impl CasterChunk {
    /// Whether the box comes nearer than `range` to `center`, so a light there may be
    /// shadowed by it.
    pub fn reaches(&self, center: glam::Vec3, range: f32) -> bool {
        let nearest = center.clamp(self.lo.into(), self.hi.into());
        nearest.distance_squared(center) < range * range
    }
}

impl SceneBase {
    /// The first vertex of each triangle with a see-through side, in `faces` order. The
    /// renderer draws these after everything opaque.
    pub fn glass(&self) -> Vec<u32> {
        self.faces
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, t)| t[0].is_glass())
            .map(|(k, _)| 3 * k as u32)
            .collect()
    }

    /// The geometry with `flags` (from `self.spans`) shown, as `scene_geometry` returns it.
    pub fn with_flags(&self, flags: &Flags) -> ModelGeometry {
        let faces = self
            .faces
            .iter()
            .zip(&flags.faces)
            .map(|(v, &selected)| FaceVertex {
                pos: v.pos,
                normal: v.normal,
                selected,
                front: v.front,
                back: v.back,
                alpha: v.alpha,
                uv: v.uv,
                layer: v.layer,
                glow: v.glow,
            })
            .collect();
        let edges = self
            .edges
            .iter()
            .zip(&flags.edges)
            .map(|(v, &selected)| LineVertex {
                pos: v.pos,
                color: if selected > 0.5 {
                    SELECTED_EDGE_COLOR
                } else {
                    v.color
                },
            })
            .collect();
        ModelGeometry {
            faces,
            edges,
            guides: shown_guides(&self.guides, &flags.guides, SELECTED_EDGE_COLOR),
            annotations: shown_annotations(
                &self.annotations,
                &flags.annotations,
                SELECTED_EDGE_COLOR,
            ),
            sections: shown_sections(&self.sections, &flags.sections, SELECTED_EDGE_COLOR),
        }
    }
}

/// The annotations of a `SceneBase` with the selected ones (`lit`, one per annotation) in
/// `color`.
pub fn shown_annotations(
    annotations: &[AnnotationLook],
    lit: &[bool],
    color: [f32; 4],
) -> Vec<AnnotationLook> {
    annotations
        .iter()
        .zip(lit)
        .map(|(a, &lit)| AnnotationLook {
            color: if lit { color } else { a.color },
            ..*a
        })
        .collect()
}

/// The guides of a `SceneBase` with the selected ones (`lit`, one per guide) in `color`.
pub fn shown_guides(guides: &[GuideShape], lit: &[bool], color: [f32; 4]) -> Vec<GuideShape> {
    guides
        .iter()
        .zip(lit)
        .map(|(g, &lit)| GuideShape {
            color: if lit { color } else { g.color },
            ..*g
        })
        .collect()
}

/// The section planes of a `SceneBase` with the selected ones (`lit`, one per plane) in `color`.
pub fn shown_sections(sections: &[SectionLook], lit: &[bool], color: [f32; 4]) -> Vec<SectionLook> {
    sections
        .iter()
        .zip(lit)
        .map(|(s, &lit)| SectionLook {
            color: if lit { color } else { s.color },
            ..*s
        })
        .collect()
}

/// Where each selectable entity of the open context lies in a `SceneBase`'s lists, so a
/// selection can be shown without rebuilding them. Each list is sorted by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spans {
    face_vertices: usize,
    edge_vertices: usize,
    guide_count: usize,
    annotation_count: usize,
    section_count: usize,
    faces: Vec<(FaceId, Range<usize>)>,
    edges: Vec<(EdgeId, Range<usize>)>,
    /// Each guide's index.
    guides: Vec<(GuideId, usize)>,
    /// Each dimension's and leader note's index among the annotations.
    dimensions: Vec<(DimensionId, usize)>,
    notes: Vec<(NoteId, usize)>,
    /// Each section plane's index among the sections.
    sections: Vec<(SectionId, usize)>,
    /// Everything an instance places, at any depth.
    instances: Vec<(InstanceId, Ranges)>,
}

/// What a selection lights up in a `SceneBase`: a flag per face vertex and per edge vertex (1
/// selected, 0 not), as the renderer uploads them, and one per guide and per annotation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flags {
    pub faces: Vec<f32>,
    pub edges: Vec<f32>,
    pub guides: Vec<bool>,
    pub annotations: Vec<bool>,
    pub sections: Vec<bool>,
}

/// Where one entity of the open context lies in a `SceneBase`'s lists (a group: all it places).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ranges {
    pub faces: Range<usize>,
    pub edges: Range<usize>,
    pub guides: Range<usize>,
    pub annotations: Range<usize>,
    pub sections: Range<usize>,
}

impl Spans {
    /// Where `id`, an id of the open context the spans were built for, lies; None for ids not
    /// found there and for vertices.
    pub fn ranges(&self, id: EntityId) -> Option<Ranges> {
        match id {
            EntityId::Face(id) => find(&self.faces, id).map(|r| Ranges {
                faces: r.clone(),
                ..Ranges::default()
            }),
            EntityId::Edge(id) => find(&self.edges, id).map(|r| Ranges {
                edges: r.clone(),
                ..Ranges::default()
            }),
            EntityId::Guide(id) => find(&self.guides, id).map(|&k| Ranges {
                guides: k..k + 1,
                ..Ranges::default()
            }),
            EntityId::Dimension(id) => find(&self.dimensions, id).map(|&k| Ranges {
                annotations: k..k + 1,
                ..Ranges::default()
            }),
            EntityId::Note(id) => find(&self.notes, id).map(|&k| Ranges {
                annotations: k..k + 1,
                ..Ranges::default()
            }),
            EntityId::Instance(id) => find(&self.instances, id).cloned(),
            EntityId::Section(id) => find(&self.sections, id).map(|&k| Ranges {
                sections: k..k + 1,
                ..Ranges::default()
            }),
            EntityId::Vertex(_) => None,
        }
    }

    /// The flags for `selection`, ids of the open context the spans were built for. Ids not
    /// found there, and vertices, light nothing.
    pub fn flags(&self, selection: &BTreeSet<EntityId>) -> Flags {
        let mut f = Flags {
            faces: vec![0.0; self.face_vertices],
            edges: vec![0.0; self.edge_vertices],
            guides: vec![false; self.guide_count],
            annotations: vec![false; self.annotation_count],
            sections: vec![false; self.section_count],
        };
        for r in selection.iter().filter_map(|&id| self.ranges(id)) {
            f.faces[r.faces].fill(1.0);
            f.edges[r.edges].fill(1.0);
            f.guides[r.guides].fill(true);
            f.annotations[r.annotations].fill(true);
            f.sections[r.sections].fill(true);
        }
        f
    }
}

/// The value stored for `id` in a list sorted by id.
fn find<K: Ord + Copy, V>(list: &[(K, V)], id: K) -> Option<&V> {
    list.binary_search_by_key(&id, |(k, _)| *k)
        .ok()
        .map(|i| &list[i].1)
}

/// The scene in root coordinates with the open instance `context` (see `scene_geometry`), with
/// nothing selected. Each mesh is triangulated once, however often it is placed.
pub fn scene_base(scene: &Scene, context: &[InstanceId]) -> SceneBase {
    scene_base_with(scene, context, false)
}

/// `scene_base`, with faces colored by tag when `color_by_tag`: a face on a tag, or else
/// placed by an instance on one (the nearest), is drawn opaque in that tag's color on both
/// sides; other faces keep their material's color, without textures. Edges keep their colors,
/// and lights and shadows stay as the materials make them.
pub fn scene_base_with(scene: &Scene, context: &[InstanceId], color_by_tag: bool) -> SceneBase {
    scene_base_shown(scene, context, color_by_tag, SectionDisplay::default())
}

/// `scene_base_with`, with the section planes shown as `section` says. A placement is cut by
/// the active section plane of its own mesh and of each mesh it is placed in, when
/// `section.cuts`: faces and edges are clipped to what the planes keep, and where a plane cuts
/// the placement's surface, `cut_edges` hold the outline and, with `section.fill`, `cut_fill`
/// the closed cuts' fill. Clipped faces cast shadows only where they remain. The plane
/// entities are listed in `sections` when `section.planes`. Lights from glowing faces are
/// those of the whole faces, cut or not.
pub fn scene_base_shown(
    scene: &Scene,
    context: &[InstanceId],
    color_by_tag: bool,
    section: SectionDisplay,
) -> SceneBase {
    let layers = layers(scene);
    let parts = placed_parts(scene, &layers);
    let (faces, edges) = drawn_count(scene, &parts, MeshId::Root, &mut HashMap::new());
    let mut walk = Walk {
        scene,
        context,
        layers: &layers,
        parts: &parts,
        glowing: scene.materials().any(|(_, m)| m.glows()),
        color_by_tag,
        display: section,
        planes: Vec::new(),
        cuts: HashMap::new(),
        sides: HashMap::new(),
        glows: HashMap::new(),
        path: Vec::new(),
        base: SceneBase {
            faces: Vec::with_capacity(faces),
            edges: Vec::with_capacity(edges + 2 * BOX_EDGES.len()),
            ..SceneBase::default()
        },
    };
    walk.mesh(MeshId::Root, DAffine3::IDENTITY, None, None);
    let mut base = walk.base;
    if !context.is_empty()
        && let Some((mesh, t)) = scene.resolve(context)
        && let Some([lo, hi]) = scene.shown_bounds(mesh, &[])
    {
        let corner = |k: usize| {
            let c = DVec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            );
            t.transform_point3(c).as_vec3().to_array()
        };
        for (a, b) in BOX_EDGES {
            for k in [a, b] {
                base.edges.push(LineVertex {
                    pos: corner(k),
                    color: CONTEXT_BOX_COLOR,
                });
            }
        }
    }
    let spans = &mut base.spans;
    spans.face_vertices = base.faces.len();
    spans.edge_vertices = base.edges.len();
    spans.guide_count = base.guides.len();
    spans.annotation_count = base.annotations.len();
    spans.section_count = base.sections.len();
    // `Spans::flags` finds ids by binary search; meshes list their entities in id order.
    debug_assert!(spans.faces.is_sorted_by_key(|(id, _)| *id));
    debug_assert!(spans.edges.is_sorted_by_key(|(id, _)| *id));
    debug_assert!(spans.guides.is_sorted_by_key(|(id, _)| *id));
    debug_assert!(spans.dimensions.is_sorted_by_key(|(id, _)| *id));
    debug_assert!(spans.notes.is_sorted_by_key(|(id, _)| *id));
    debug_assert!(spans.sections.is_sorted_by_key(|(id, _)| *id));
    debug_assert!(spans.instances.is_sorted_by_key(|(id, _)| *id));
    base
}

/// Corner pairs of a box's edges; corner k has bit 0 for x, bit 1 for y, bit 2 for z.
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// A clipped corner's normal and texture coordinates, `t` of the way from `a` to `b`.
fn mix_corner(a: (DVec3, [f32; 4]), b: (DVec3, [f32; 4]), t: f64) -> (DVec3, [f32; 4]) {
    let t32 = t as f32;
    (
        a.0.lerp(b.0, t),
        std::array::from_fn(|k| a.1[k] + (b.1[k] - a.1[k]) * t32),
    )
}

/// What a mesh draws wherever it is placed, in its own coordinates.
struct MeshParts {
    /// The triangles of every face, three corners each, wound counter-clockwise around the
    /// face's normal.
    corners: Vec<DVec3>,
    /// The normal at each corner: the face's own, or across soft edges the smooth one.
    normals: Vec<DVec3>,
    /// Each corner's position on its face's front texture, in meters (see
    /// `kernel::Face::texture_position`).
    positions: Vec<DVec2>,
    faces: Vec<FacePart>,
    /// The ends of the hard edges (soft edges are not drawn).
    ends: Vec<DVec3>,
    edges: Vec<(EdgeId, Option<TagId>, Range<usize>)>,
}

struct FacePart {
    id: FaceId,
    /// The face's triangle corners in `MeshParts::corners`, `normals` and `positions`.
    corners: Range<usize>,
    /// The looks of the sides painted on the face itself; unpainted sides take the look of
    /// the nearest painted instance placing them.
    front: Option<Look>,
    back: Option<Look>,
    tag: Option<TagId>,
}

impl MeshParts {
    fn of(scene: &Scene, layers: &Layers, model: &Model) -> Self {
        // A face of n loop corners and h holes has n + 2h - 2 triangles.
        let (mut face_count, mut triangles) = (0, 0);
        for (_, f) in model.faces() {
            let n: usize = f.outer.len() + f.holes.iter().map(Vec::len).sum::<usize>();
            face_count += 1;
            triangles += (n + 2 * f.holes.len()).saturating_sub(2);
        }
        let mut corners = Vec::with_capacity(3 * triangles);
        let mut normals = Vec::with_capacity(3 * triangles);
        let mut positions = Vec::with_capacity(3 * triangles);
        let mut faces = Vec::with_capacity(face_count);
        let smooth = model.smooth_normals();
        for (id, face) in model.faces() {
            let start = corners.len();
            for tri in model.shaded_triangles(id, &smooth).unwrap_or_default() {
                corners.extend(tri.corners);
                normals.extend(tri.normals);
                positions.extend(tri.corners.map(|p| face.texture_position(p)));
            }
            faces.push(FacePart {
                id,
                corners: start..corners.len(),
                front: face
                    .front
                    .map(|m| side_look(scene, layers, Some(m), DEFAULT_FRONT)),
                back: face
                    .back
                    .map(|m| side_look(scene, layers, Some(m), DEFAULT_BACK)),
                tag: face.tag,
            });
        }
        let hard = model.edges().filter(|(_, e)| !e.soft).count();
        let mut ends = Vec::with_capacity(2 * hard);
        let mut edges = Vec::with_capacity(hard);
        // Soft edges are hidden, selected or not: the faces around them read as one surface.
        for (id, edge) in model.edges().filter(|(_, e)| !e.soft) {
            let start = ends.len();
            ends.extend(edge.v.iter().filter_map(|v| model.position(*v)));
            edges.push((id, edge.tag, start..ends.len()));
        }
        MeshParts {
            corners,
            normals,
            positions,
            faces,
            ends,
            edges,
        }
    }
}

/// Below this many faces in the placed meshes, their parts are worked out on this thread alone.
const PARALLEL_FACES: usize = 1000;

/// Most threads that work out mesh parts.
const MAX_THREADS: usize = 8;

/// The parts of every mesh placed from the root, the root included. With many faces, the meshes
/// are shared out among a few threads; the parts are the same either way.
fn placed_parts(scene: &Scene, layers: &Layers) -> HashMap<MeshId, MeshParts> {
    let mut meshes = vec![MeshId::Root];
    let mut seen = HashSet::from([MeshId::Root]);
    let mut k = 0;
    while let Some(&id) = meshes.get(k) {
        if let Some(model) = scene.mesh(id) {
            for (_, instance) in model.instances() {
                let def = MeshId::Definition(instance.definition);
                if seen.insert(def) {
                    meshes.push(def);
                }
            }
        }
        k += 1;
    }
    let models: Vec<(MeshId, &Model)> = meshes
        .into_iter()
        .filter_map(|id| Some((id, scene.mesh(id)?)))
        .collect();
    let faces: usize = models.iter().map(|(_, m)| m.faces().count()).sum();
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_THREADS)
        .min(models.len());
    if faces < PARALLEL_FACES || threads < 2 {
        return models
            .into_iter()
            .map(|(id, m)| (id, MeshParts::of(scene, layers, m)))
            .collect();
    }
    // Each thread takes the next mesh nobody has taken, so big meshes do not hold one up.
    let next = AtomicUsize::new(0);
    let work = || {
        let mut done = Vec::new();
        while let Some(&(id, m)) = models.get(next.fetch_add(1, Ordering::Relaxed)) {
            done.push((id, MeshParts::of(scene, layers, m)));
        }
        done
    };
    std::thread::scope(|s| {
        // A helper that cannot be started leaves its share to the others and to this thread.
        let helpers: Vec<_> = (1..threads)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(s, work).ok())
            .collect();
        let mut parts: HashMap<MeshId, MeshParts> = work().into_iter().collect();
        for h in helpers {
            parts.extend(h.join().expect("a mesh parts thread panicked"));
        }
        parts
    })
}

/// How many face corners and edge ends one placement of mesh `id` draws, nested instances
/// included; `memo` holds the meshes counted so far.
fn drawn_count(
    scene: &Scene,
    parts: &HashMap<MeshId, MeshParts>,
    id: MeshId,
    memo: &mut HashMap<MeshId, (usize, usize)>,
) -> (usize, usize) {
    if let Some(&n) = memo.get(&id) {
        return n;
    }
    let (Some(model), Some(p)) = (scene.mesh(id), parts.get(&id)) else {
        return (0, 0);
    };
    let (mut faces, mut edges) = (p.corners.len(), p.ends.len());
    for (_, instance) in model.instances() {
        let (f, e) = drawn_count(scene, parts, MeshId::Definition(instance.definition), memo);
        // Saturating: a deeply nested model can place more than usize holds.
        faces = faces.saturating_add(f);
        edges = edges.saturating_add(e);
    }
    memo.insert(id, (faces, edges));
    (faces, edges)
}

/// Adds a placement's cut to the base: the fill, and the cut edges in `color`.
fn push_cut(base: &mut SceneBase, cut: &Cut, color: [f32; 4]) {
    base.cut_fill
        .extend(cut.fill.iter().flatten().map(|p| LineVertex {
            pos: p.as_vec3().to_array(),
            color: CUT_FILL_COLOR,
        }));
    base.cut_edges
        .extend(cut.edges.iter().flatten().map(|p| LineVertex {
            pos: p.as_vec3().to_array(),
            color,
        }));
}

struct Walk<'a> {
    scene: &'a Scene,
    context: &'a [InstanceId],
    layers: &'a Layers,
    /// Every placed mesh's parts.
    parts: &'a HashMap<MeshId, MeshParts>,
    /// Whether any material glows; when none does, lights are not looked for.
    glowing: bool,
    /// Faces are drawn in their tags' colors (see `scene_base_with`).
    color_by_tag: bool,
    /// How section planes are shown.
    display: SectionDisplay,
    /// The active section planes of the meshes from the root down to the one being walked, in
    /// root coordinates; empty unless cuts are on.
    planes: Vec<SectionPlane>,
    /// What a mesh's own active plane cuts out of it, in the mesh's coordinates, by the mesh
    /// and the plane: a mesh placed many times, cut by no plane but its own, is cut once.
    cuts: HashMap<(MeshId, [u64; 6]), Cut>,
    /// The side of the square each mesh's section planes are drawn with.
    sides: HashMap<MeshId, f64>,
    /// What each mesh's own faces glow with, by the material it inherits, once worked out: one
    /// glow for an instance's mesh, one per connected group of glowing faces for the root's.
    glows: HashMap<(MeshId, Option<MaterialId>), Vec<Glow>>,
    /// Instances from the root down to the mesh being walked.
    path: Vec<InstanceId>,
    base: SceneBase,
}

impl Walk<'_> {
    /// Adds a mesh placed by `t`; `inherited` is the material of the nearest painted instance
    /// placing it and `inherited_tag` the tag of the nearest tagged one.
    fn mesh(
        &mut self,
        id: MeshId,
        t: DAffine3,
        inherited: Option<MaterialId>,
        inherited_tag: Option<TagId>,
    ) {
        let (Some(model), Some(parts)) = (self.scene.mesh(id), self.parts.get(&id)) else {
            return;
        };
        // Selection ids refer to the open context's placement only, not to other placements
        // of the same component.
        let open = self.path == self.context;
        let inside = self.path.starts_with(self.context);
        // Normals follow the inverse transpose, right for any linear map (placements turn and
        // move, so it is the rotation itself for now).
        let normal_map = t.matrix3.inverse().transpose();
        // A mirroring placement turns triangles inside out; swapping two corners restores the
        // winding, so the front side (and the headlight) stay right.
        let mirrored = t.matrix3.determinant() < 0.0;
        let front = side_look(self.scene, self.layers, inherited, DEFAULT_FRONT);
        let back = side_look(self.scene, self.layers, inherited, DEFAULT_BACK);
        let glow_key = (id, inherited);
        let scene = self.scene;
        let color_by_tag = self.color_by_tag;
        let display = self.display;
        let mut pushed_plane = false;
        // The mesh's own active plane in its own coordinates.
        let mut own_plane = None;
        if display.cuts
            && let Some(plane) = model.active_section().and_then(|s| model.section(s))
        {
            self.planes.push(plane.transformed(t));
            pushed_plane = true;
            own_plane = Some(*plane);
        }
        // The side of the squares this mesh's planes are drawn with, worked out once per mesh.
        let side = (display.planes && model.sections().next().is_some()).then(|| {
            *self
                .sides
                .entry(id)
                .or_insert_with(|| SectionShape::side(scene.shown_bounds(id, &[])))
        });
        let planes: Vec<SectionPlane> = self.planes.clone();
        let cutting = !planes.is_empty();
        // With no plane but its own cutting it, the cut is the same for every placement.
        let cache_key = own_plane.filter(|_| planes.len() == 1).map(|p| {
            let bits = [
                p.point.x, p.point.y, p.point.z, p.normal.x, p.normal.y, p.normal.z,
            ];
            (id, bits.map(f64::to_bits))
        });
        let collect_local = cache_key.is_some_and(|k| !self.cuts.contains_key(&k));
        // The mesh's glow, when it has to be worked out (in its own coordinates). A placement
        // that is cut works out its own: a face cut away entirely does not shine. Each glowing
        // face's glow, with the face.
        let mut glow: Option<Vec<(Glow, FaceId)>> =
            (self.glowing && (!self.glows.contains_key(&glow_key) || cutting)).then(Vec::new);
        // The placement's own surface in root coordinates, before any cut, for the cut outlines.
        let mut surface: Vec<[DVec3; 3]> = Vec::new();
        let base = &mut self.base;
        let first_caster = base.casters.len();
        for face in &parts.faces {
            if !scene.tag_shown(face.tag) {
                continue;
            }
            let start = base.faces.len();
            // Lights and shadows go by the materials; what is drawn may go by the tag.
            let front = face.front.unwrap_or(front);
            let back = face.back.unwrap_or(back);
            let (drawn_front, drawn_back) = if color_by_tag {
                let tag = face.tag.or(inherited_tag);
                (
                    tag_colored(scene, tag, front),
                    tag_colored(scene, tag, back),
                )
            } else {
                (front, back)
            };
            let corner = |(p, n, uv): (DVec3, DVec3, [f32; 4])| BaseFaceVertex {
                pos: p.as_vec3().to_array(),
                normal: (normal_map * n).normalize().as_vec3().to_array(),
                front: drawn_front.color,
                back: drawn_back.color,
                alpha: [drawn_front.alpha, drawn_back.alpha],
                uv,
                layer: [drawn_front.layer(), drawn_back.layer()],
                glow: [drawn_front.glow(), drawn_back.glow()],
            };
            let range = face.corners.clone();
            let (tris, _) = parts.corners[range.clone()].as_chunks::<3>();
            let (normals, _) = parts.normals[range.clone()].as_chunks::<3>();
            let (positions, _) = parts.positions[range].as_chunks::<3>();
            let (emission, glow_color) = face_glow(&front, &back);
            // A lamp's glowing shade does not hide its own light.
            let casts = emission <= 0.0;
            let mut face_glow = Glow::default();
            for ((&[a, b, c], &[na, nb, nc]), &uvs) in tris.iter().zip(normals).zip(positions) {
                if glow.is_some() {
                    face_glow.add_triangle([a, b, c], emission, glow_color);
                }
                let [ua, ub, uc] = both_uv(&drawn_front, &drawn_back, uvs);
                let tri = if mirrored {
                    [(a, na, ua), (c, nc, uc), (b, nb, ub)]
                } else {
                    [(a, na, ua), (b, nb, ub), (c, nc, uc)]
                };
                let tri = tri.map(|(p, n, uv)| (t.transform_point3(p), n, uv));
                let emit = |base: &mut SceneBase, tri: [(DVec3, DVec3, [f32; 4]); 3]| {
                    base.faces.extend(tri.map(corner));
                    if casts {
                        let placed = &base.faces[base.faces.len() - 3..];
                        base.casters.extend(placed.iter().map(|v| v.pos));
                    }
                };
                if cutting {
                    if collect_local {
                        surface.push([a, b, c]);
                    } else if cache_key.is_none() {
                        surface.push(tri.map(|c| c.0));
                    }
                    let attributes = tri.map(|(p, n, uv)| (p, (n, uv)));
                    for piece in clip_triangle(attributes, &planes, &mix_corner) {
                        emit(base, piece.map(|(p, (n, uv))| (p, n, uv)));
                    }
                } else {
                    emit(base, tri);
                }
            }
            if let Some(g) = &mut glow
                && face_glow.glows()
                && (!cutting || base.faces.len() > start)
            {
                g.push((face_glow, face.id));
            }
            if open {
                base.spans.faces.push((face.id, start..base.faces.len()));
            }
        }
        if base.casters.len() > first_caster {
            let corners = &base.casters[first_caster..];
            let (lo, hi) = corners.iter().fold(
                (glam::Vec3::INFINITY, glam::Vec3::NEG_INFINITY),
                |(lo, hi), &p| (lo.min(p.into()), hi.max(p.into())),
            );
            base.caster_chunks.push(CasterChunk {
                vertices: first_caster as u32..base.casters.len() as u32,
                lo: lo.to_array(),
                hi: hi.to_array(),
            });
        }
        let mut cut_glows: Option<Vec<Glow>> = None;
        if let Some(faces) = glow {
            let glows = if id == MeshId::Root {
                let faces: Vec<(Glow, Vec<VertexId>)> = faces
                    .into_iter()
                    .map(|(g, f)| {
                        let vertices = model.face(f).map_or_else(Vec::new, |f| {
                            f.outer
                                .iter()
                                .chain(f.holes.iter().flatten())
                                .copied()
                                .collect()
                        });
                        (g, vertices)
                    })
                    .collect();
                connected(&faces)
            } else {
                let mut all = Glow::default();
                for (g, _) in &faces {
                    all.add(g);
                }
                vec![all]
            };
            if cutting {
                cut_glows = Some(glows);
            } else {
                self.glows.insert(glow_key, glows);
            }
        }
        if let Some(glows) = cut_glows.as_ref().or_else(|| self.glows.get(&glow_key)) {
            base.lights.extend(glows.iter().filter_map(|g| g.placed(t)));
        }
        let color = if inside { EDGE_COLOR } else { FADED_EDGE_COLOR };
        for (eid, tag, ends) in &parts.edges {
            if !scene.tag_shown(*tag) {
                continue;
            }
            let start = base.edges.len();
            let ends = &parts.ends[ends.clone()];
            if cutting {
                // The ends of an edge come in a pair.
                if let [a, b] = ends
                    && let Some(kept) =
                        clip_segment(t.transform_point3(*a), t.transform_point3(*b), &planes)
                {
                    base.edges.extend(kept.map(|p| LineVertex {
                        pos: p.as_vec3().to_array(),
                        color,
                    }));
                }
            } else {
                base.edges.extend(ends.iter().map(|p| LineVertex {
                    pos: t.transform_point3(*p).as_vec3().to_array(),
                    color,
                }));
            }
            if open {
                base.spans.edges.push((*eid, start..base.edges.len()));
            }
        }
        if let (Some(key), Some(plane)) = (cache_key, own_plane) {
            if collect_local && !surface.is_empty() {
                self.cuts
                    .insert(key, cut_surface(&surface, &[plane], display.fill));
            }
            if let Some(cut) = self.cuts.get(&key) {
                // A mirroring placement turns the fill's triangles inside out.
                let place = |tri: &[DVec3; 3]| {
                    let [a, b, c] = tri.map(|p| t.transform_point3(p));
                    if mirrored { [a, c, b] } else { [a, b, c] }
                };
                let fill: Vec<[DVec3; 3]> = cut.fill.iter().map(place).collect();
                let edges: Vec<[DVec3; 2]> = cut
                    .edges
                    .iter()
                    .map(|e| e.map(|p| t.transform_point3(p)))
                    .collect();
                push_cut(base, &Cut { fill, edges }, color);
            }
        } else if cutting && !surface.is_empty() {
            let cut = cut_surface(&surface, &planes, display.fill);
            push_cut(base, &cut, color);
        }
        let color = if inside {
            GUIDE_COLOR
        } else {
            FADED_GUIDE_COLOR
        };
        for (gid, guide) in model.guides() {
            if !scene.tag_shown(guide.tag()) {
                continue;
            }
            let (point, direction) = match guide.transformed(t) {
                Guide::Line {
                    point, direction, ..
                } => (point, Some(direction)),
                Guide::Point { position, .. } => (position, None),
            };
            if open {
                base.spans.guides.push((gid, base.guides.len()));
            }
            base.guides.push(GuideShape {
                point,
                direction,
                color,
            });
        }
        let color = if inside {
            ANNOTATION_COLOR
        } else {
            FADED_ANNOTATION_COLOR
        };
        for (did, d) in model.dimensions() {
            if !scene.tag_shown(d.tag()) {
                continue;
            }
            let Some(shape) = AnnotationShape::of(model, EntityId::Dimension(did)) else {
                continue;
            };
            if open {
                base.spans.dimensions.push((did, base.annotations.len()));
            }
            base.annotations.push(AnnotationLook {
                shape: shape.transformed(t),
                color,
            });
        }
        for (nid, n) in model.notes() {
            // Screen notes are the window's alone.
            if !scene.tag_shown(n.tag()) || matches!(n, kernel::Note::Screen { .. }) {
                continue;
            }
            let Some(shape) = AnnotationShape::of(model, EntityId::Note(nid)) else {
                continue;
            };
            if open {
                base.spans.notes.push((nid, base.annotations.len()));
            }
            base.annotations.push(AnnotationLook {
                shape: shape.transformed(t),
                color,
            });
        }
        if let Some(side) = side {
            for (sid, plane) in model.sections() {
                if !scene.tag_shown(plane.tag()) {
                    continue;
                }
                // Planes past the limit still cut but are neither drawn nor picked.
                if base.sections.len() >= session::MAX_SECTION_OUTLINES {
                    break;
                }
                if open {
                    base.spans.sections.push((sid, base.sections.len()));
                }
                let active = model.active_section() == Some(sid);
                base.sections.push(SectionLook {
                    shape: SectionShape::new(plane, side).transformed(t),
                    color: match (active, inside) {
                        (true, true) => ACTIVE_SECTION_COLOR,
                        (true, false) => FADED_ACTIVE_SECTION_COLOR,
                        (false, true) => GUIDE_COLOR,
                        (false, false) => FADED_GUIDE_COLOR,
                    },
                    active,
                });
            }
        }
        for (iid, instance) in model.instances() {
            // A hidden instance hides all it places, but for the one being edited (or holding
            // it), which stays drawn.
            if !self.scene.tag_shown(instance.tag) && !self.on_context_path(iid) {
                continue;
            }
            let start = (
                self.base.faces.len(),
                self.base.edges.len(),
                self.base.guides.len(),
                self.base.annotations.len(),
                self.base.sections.len(),
            );
            self.path.push(iid);
            self.mesh(
                MeshId::Definition(instance.definition),
                t * instance.transform,
                instance.material.or(inherited),
                instance.tag.or(inherited_tag),
            );
            self.path.pop();
            if open {
                let placed = Ranges {
                    faces: start.0..self.base.faces.len(),
                    edges: start.1..self.base.edges.len(),
                    guides: start.2..self.base.guides.len(),
                    annotations: start.3..self.base.annotations.len(),
                    sections: start.4..self.base.sections.len(),
                };
                self.base.spans.instances.push((iid, placed));
            }
        }
        if pushed_plane {
            self.planes.pop();
        }
    }

    /// Whether instance `iid`, placed in the mesh being walked, is on the open context's path.
    fn on_context_path(&self, iid: InstanceId) -> bool {
        let depth = self.path.len();
        self.context.len() > depth
            && self.context.starts_with(&self.path)
            && self.context[depth] == iid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{DefinitionKind, Model};

    fn square() -> Model {
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        m.add_polygon(&pts).unwrap();
        m
    }

    fn geometry(m: Model, selection: &BTreeSet<EntityId>) -> ModelGeometry {
        scene_geometry(&Scene::from_root(m), &[], selection)
    }

    /// The square made a component, copied 3 m along x. Returns the scene and both instances.
    fn two_squares() -> (Scene, InstanceId, InstanceId) {
        let mut s = Scene::from_root(square());
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (first, _) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Component,
                "Tile",
            )
            .unwrap();
        s.copy_instances(MeshId::Root, &[first], DVec3::new(3.0, 0.0, 0.0), 1)
            .unwrap();
        let second = s
            .root()
            .instances()
            .map(|(id, _)| id)
            .find(|id| *id != first)
            .unwrap();
        (s, first, second)
    }

    #[test]
    fn empty_model_has_no_geometry() {
        assert_eq!(
            geometry(Model::new(), &BTreeSet::new()),
            ModelGeometry::default()
        );
    }

    #[test]
    fn square_gives_two_triangles_and_four_edges() {
        let g = geometry(square(), &BTreeSet::new());
        assert_eq!(g.faces.len(), 6);
        assert!(
            g.faces
                .iter()
                .all(|v| v.normal == [0.0, 0.0, 1.0] && v.selected == 0.0)
        );
        assert_eq!(g.edges.len(), 8);
        assert!(g.edges.iter().all(|v| v.color == EDGE_COLOR));
    }

    #[test]
    fn soft_edges_are_not_drawn() {
        // A triangle pulled up: three side faces. Its sides made a curve first, so the
        // swept edges between them are soft.
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(0.5, 0.8, 0.0)];
        let c = m.add_curve(&pts, true, false).unwrap();
        let face = c
            .summary()
            .created
            .iter()
            .find_map(|id| match id {
                EntityId::Face(f) => Some(*f),
                _ => None,
            })
            .unwrap();
        m.push_pull(face, 1.0).unwrap();
        assert_eq!(m.edges().filter(|(_, e)| e.soft).count(), 3);
        let all: BTreeSet<EntityId> = m.edges().map(|(e, _)| EntityId::Edge(e)).collect();
        let g = geometry(m, &all);
        assert_eq!(g.edges.len(), 2 * 6, "the two rims, selected or not");
    }

    #[test]
    fn faces_are_shaded_smoothly_across_soft_edges_in_any_placement() {
        // A 12-sided cylinder, its sides soft, placed in a group turned a quarter about x and
        // mirrored in z.
        let mut m = Model::new();
        let ring: Vec<DVec3> = (0..12)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 12.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        m.add_curve(&ring, true, false).unwrap();
        let (base, _) = m.faces().next().unwrap();
        let up = m.face(base).unwrap().normal.z.signum();
        m.push_pull(base, up).unwrap();
        let mut s = Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (group, _) = s
            .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        let place = DAffine3::from_rotation_x(std::f64::consts::FRAC_PI_2)
            * DAffine3::from_scale(DVec3::new(1.0, 1.0, -1.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[group], place)))
            .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        let mut sides = 0;
        for tri in g.faces.as_chunks::<3>().0 {
            let p = tri.map(|v| glam::Vec3::from_array(v.pos));
            let face_normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            if face_normal.y.abs() > 0.99 {
                // A cap: flat, along the turned axis.
                assert!(tri.iter().all(|v| v.normal == tri[0].normal));
                continue;
            }
            for (v, p) in tri.iter().zip(p) {
                // Straight out from the turned axis (y), and to the same side as the face.
                let out = glam::Vec3::new(p.x, 0.0, p.z).normalize();
                let n = glam::Vec3::from_array(v.normal);
                assert!(n.distance(out) < 1e-5, "{p} {n}");
                assert!(n.dot(face_normal) > 0.0);
                sides += 1;
            }
        }
        assert_eq!(sides, 12 * 6);
    }

    #[test]
    fn faces_take_their_material_colors_or_the_nearest_painted_instance() {
        let (mut s, first, second) = two_squares();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert!(
            g.faces
                .iter()
                .all(|v| v.front == DEFAULT_FRONT && v.back == DEFAULT_BACK)
        );
        // The first square's instance is blue; the component's face is red on its back only.
        s.paint(MeshId::Root, &[], &[first], kernel::Side::Front, Some(blue))
            .unwrap();
        let d = s.root().instance(first).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        s.paint(
            MeshId::Definition(d),
            &[face],
            &[],
            kernel::Side::Back,
            Some(red),
        )
        .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        let (one, two): (Vec<&FaceVertex>, Vec<&FaceVertex>) =
            g.faces.iter().partition(|v| v.pos[0] <= 1.0);
        assert!(
            one.iter()
                .all(|v| v.front == [0.0, 0.0, 1.0] && v.back == [1.0, 0.0, 0.0])
        );
        assert!(
            two.iter()
                .all(|v| v.front == DEFAULT_FRONT && v.back == [1.0, 0.0, 0.0]),
            "the second placement shares the face but not the instance's blue"
        );
        let _ = second;
    }

    #[test]
    fn see_through_sides_carry_their_opacity_and_are_listed_as_glass() {
        let (mut s, first, _) = two_squares();
        assert!(scene_base(&s, &[]).glass().is_empty());
        let (glass, _) = s
            .add_material_with_opacity("Glass", [0, 0, 255], 0.25)
            .unwrap();
        // The first placement is glass by its instance; the shared face is opaque on its back.
        s.paint(
            MeshId::Root,
            &[],
            &[first],
            kernel::Side::Front,
            Some(glass),
        )
        .unwrap();
        let d = s.root().instance(first).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        s.paint(
            MeshId::Definition(d),
            &[face],
            &[],
            kernel::Side::Back,
            Some(red),
        )
        .unwrap();
        let base = scene_base(&s, &[]);
        let (one, two): (Vec<&BaseFaceVertex>, Vec<_>) =
            base.faces.iter().partition(|v| v.pos[0] <= 1.0);
        assert!(one.iter().all(|v| v.alpha == [0.25, 1.0]));
        assert!(two.iter().all(|v| v.alpha == [1.0, 1.0]));
        let glass_triangles = base.glass();
        assert_eq!(glass_triangles.len(), 2);
        for k in glass_triangles {
            assert!(base.faces[k as usize].pos[0] <= 1.0);
        }
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert_eq!(
            g.faces.iter().filter(|v| v.alpha[0] < 1.0).count(),
            6,
            "scene_geometry carries the opacity too"
        );
    }

    /// A material glowing at `emission` in white.
    fn lamp(s: &mut Scene, emission: f32) -> MaterialId {
        s.add_material_from(kernel::Material {
            name: "Lamp".into(),
            color: [255, 255, 255],
            opacity: 1.0,
            texture: None,
            emission,
        })
        .unwrap()
        .0
    }

    /// The component's square painted with `material` in front.
    fn paint_tile(s: &mut Scene, instance: InstanceId, material: MaterialId) {
        let d = s.root().instance(instance).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        s.paint(
            MeshId::Definition(d),
            &[face],
            &[],
            kernel::Side::Front,
            Some(material),
        )
        .unwrap();
    }

    #[test]
    fn without_glowing_materials_there_are_no_lights() {
        let (mut s, first, _) = two_squares();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        paint_tile(&mut s, first, red);
        let base = scene_base(&s, &[]);
        assert!(base.lights.is_empty());
        assert!(base.faces.iter().all(|v| v.glow == [0.0, 0.0]));
    }

    #[test]
    fn each_placement_of_a_glowing_mesh_is_one_light() {
        let (mut s, first, _) = two_squares();
        let lamp = lamp(&mut s, 2.0);
        paint_tile(&mut s, first, lamp);
        let base = scene_base(&s, &[]);
        // Both placements share the glowing face: a light at each square's center, 1 m² at 2.
        let centers: Vec<DVec3> = base.lights.iter().map(|l| l.position).collect();
        assert_eq!(centers.len(), 2);
        assert!(centers[0].distance(DVec3::new(0.5, 0.5, 0.0)) < 1e-9);
        assert!(centers[1].distance(DVec3::new(3.5, 0.5, 0.0)) < 1e-9);
        assert!(
            base.lights
                .iter()
                .all(|l| (l.strength - 2.0).abs() < 1e-9 && l.color == [1.0; 3])
        );
        // The glowing side is marked; the back is not.
        assert!(base.faces.iter().all(|v| v.glow == [1.0, 0.0]));
        assert!(
            scene_geometry(&s, &[], &BTreeSet::new())
                .faces
                .iter()
                .all(|v| v.glow == [1.0, 0.0])
        );
        // The open context does not change the lights.
        assert_eq!(scene_base(&s, &[first]).lights, base.lights);
    }

    #[test]
    fn loose_glowing_faces_give_a_light_per_connected_group_and_instances_one_each() {
        let square_at = |m: &mut Model, x: f64, y: f64| {
            let p = |dx: f64, dy: f64| DVec3::new(x + dx, y + dy, 0.0);
            m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
                .unwrap();
        };
        // Loose: a and b share an edge, c touches b at a corner, d stands apart; e is plain and
        // touches d, which does not join it to anything.
        let mut m = Model::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (2.0, 1.0), (6.0, 0.0)] {
            square_at(&mut m, x, y);
        }
        let glowing: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
        square_at(&mut m, 7.0, 0.0);
        let mut s = Scene::from_root(m);
        let lamp = lamp(&mut s, 1.0);
        s.paint(MeshId::Root, &glowing, &[], kernel::Side::Front, Some(lamp))
            .unwrap();
        let base = scene_base(&s, &[]);
        let centers: Vec<DVec3> = base.lights.iter().map(|l| l.position).collect();
        assert_eq!(centers.len(), 2, "{centers:?}");
        // a, b and c weigh alike: centers (0.5, 0.5), (1.5, 0.5) and (2.5, 1.5).
        let joined = DVec3::new(4.5 / 3.0, 2.5 / 3.0, 0.0);
        let apart = DVec3::new(6.5, 0.5, 0.0);
        assert!(
            centers.iter().any(|c| c.distance(joined) < 1e-9),
            "{centers:?}"
        );
        assert!(
            centers.iter().any(|c| c.distance(apart) < 1e-9),
            "{centers:?}"
        );
        assert!((base.lights.iter().map(|l| l.strength).sum::<f64>() - 4.0).abs() < 1e-9);

        // The same four squares made a group: one light at their center.
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        s.make_group(
            MeshId::Root,
            &edges,
            &glowing,
            &[],
            DefinitionKind::Group,
            "Lamps",
        )
        .unwrap();
        let base = scene_base(&s, &[]);
        assert_eq!(base.lights.len(), 1);
        let center = DVec3::new((4.5 + 6.5) / 4.0, (2.5 + 0.5) / 4.0, 0.0);
        assert!(base.lights[0].position.distance(center) < 1e-9);
    }

    #[test]
    fn a_painted_instance_glows_with_the_faces_it_colors() {
        let (mut s, first, _) = two_squares();
        let lamp = lamp(&mut s, 1.0);
        s.paint(MeshId::Root, &[], &[first], kernel::Side::Front, Some(lamp))
            .unwrap();
        let base = scene_base(&s, &[]);
        assert_eq!(base.lights.len(), 1, "only the painted placement");
        assert!(base.lights[0].position.distance(DVec3::new(0.5, 0.5, 0.0)) < 1e-9);
        // Its face glows on both sides, inheriting the material.
        let glowing = base.faces.iter().filter(|v| v.glow == [1.0, 1.0]).count();
        assert_eq!(glowing, 6);
    }

    #[test]
    fn a_lamp_nested_in_groups_gives_its_own_light_wherever_it_is_placed() {
        // The first tile, glowing, put in a group; the group copied 10 m along y and turned a
        // quarter about z there. The second tile stays in the root.
        let (mut s, first, second) = two_squares();
        let lamp = lamp(&mut s, 3.0);
        paint_tile(&mut s, first, lamp);
        let (house, _) = s
            .make_group(
                MeshId::Root,
                &[],
                &[],
                &[first],
                DefinitionKind::Group,
                "House",
            )
            .unwrap();
        s.copy_instances(MeshId::Root, &[house], DVec3::new(0.0, 10.0, 0.0), 1)
            .unwrap();
        let copy = s
            .root()
            .instances()
            .map(|(id, _)| id)
            .find(|id| *id != house && *id != second)
            .unwrap();
        let turn = DAffine3::from_translation(DVec3::new(0.0, 10.0, 0.0))
            * DAffine3::from_rotation_z(std::f64::consts::FRAC_PI_2)
            * DAffine3::from_translation(DVec3::new(0.0, -10.0, 0.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[copy], turn)))
            .unwrap();
        let mut centers: Vec<[i64; 3]> = scene_base(&s, &[])
            .lights
            .iter()
            .map(|l| (l.position * 1000.0).round().as_i64vec3().to_array())
            .collect();
        centers.sort();
        // The groups give no light of their own; the tile does, in the root, in the house and
        // in the turned copy, where (0.5, 0.5) lands at (-0.5, 10.5).
        assert_eq!(centers, [[-500, 10500, 0], [500, 500, 0], [3500, 500, 0]]);
    }

    #[test]
    fn faces_without_a_glowing_side_cast_shadows_in_a_chunk_per_placement() {
        let (mut s, first, _) = two_squares();
        // Nothing glows: both tiles cast, each placement its own chunk with its bounds.
        let base = scene_base(&s, &[]);
        assert_eq!(base.casters.len(), 12);
        let positions: Vec<[f32; 3]> = base.faces.iter().map(|v| v.pos).collect();
        assert_eq!(base.casters, positions);
        assert_eq!(base.caster_chunks.len(), 2);
        assert_eq!(base.caster_chunks[0].vertices, 0..6);
        assert_eq!(base.caster_chunks[1].vertices, 6..12);
        assert_eq!(base.caster_chunks[1].lo, [3.0, 0.0, 0.0]);
        assert_eq!(base.caster_chunks[1].hi, [4.0, 1.0, 0.0]);
        // The first tile's definition glows in front: it casts nothing, in either placement
        // (both place the same definition). Painting just the second instance glowing
        // instead leaves the first casting.
        let glowing = lamp(&mut s, 2.0);
        paint_tile(&mut s, first, glowing);
        let base = scene_base(&s, &[]);
        assert!(base.casters.is_empty() && base.caster_chunks.is_empty());
        let (mut s, _, second) = two_squares();
        let glowing = lamp(&mut s, 1.0);
        s.paint(
            MeshId::Root,
            &[],
            &[second],
            kernel::Side::Front,
            Some(glowing),
        )
        .unwrap();
        let base = scene_base(&s, &[]);
        assert_eq!(base.caster_chunks.len(), 1);
        assert_eq!(base.caster_chunks[0].hi, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn a_chunk_reaches_a_light_whose_range_comes_near_its_box() {
        let chunk = CasterChunk {
            vertices: 0..3,
            lo: [0.0, 0.0, 0.0],
            hi: [1.0, 1.0, 1.0],
        };
        let at = |x: f32, y: f32, z: f32| glam::Vec3::new(x, y, z);
        assert!(chunk.reaches(at(0.5, 0.5, 0.5), 0.1));
        assert!(chunk.reaches(at(3.0, 0.5, 0.5), 2.5));
        assert!(!chunk.reaches(at(3.0, 0.5, 0.5), 2.0));
        // Corner distance sqrt(3) from (2, 2, 2).
        assert!(chunk.reaches(at(2.0, 2.0, 2.0), 1.8));
        assert!(!chunk.reaches(at(2.0, 2.0, 2.0), 1.7));
    }

    #[test]
    fn selected_entities_are_marked() {
        let m = square();
        let face = m.faces().next().unwrap().0;
        let edge = m.edges().next().unwrap().0;
        let selection = BTreeSet::from([EntityId::Face(face), EntityId::Edge(edge)]);
        let g = geometry(m, &selection);
        assert!(g.faces.iter().all(|v| v.selected == 1.0));
        let blue = g
            .edges
            .iter()
            .filter(|v| v.color == SELECTED_EDGE_COLOR)
            .count();
        assert_eq!(blue, 2, "one edge, two vertices");
    }

    #[test]
    fn every_instance_is_drawn_where_it_is_placed() {
        let (s, _, _) = two_squares();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert_eq!(g.faces.len(), 12);
        assert_eq!(g.edges.len(), 16);
        let max_x = g.faces.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max);
        assert_eq!(max_x, 4.0);
    }

    #[test]
    fn a_turned_instance_is_drawn_turned_with_turned_normals() {
        let (mut s, _, second) = two_squares();
        // A third of a quarter turn about X through the second square's corner (3, 0, 0).
        let turn = DAffine3::from_translation(DVec3::new(3.0, 0.0, 0.0))
            * DAffine3::from_rotation_x(30f64.to_radians())
            * DAffine3::from_translation(DVec3::new(-3.0, 0.0, 0.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[second], turn)))
            .unwrap();
        let g = scene_geometry(&s, &[second], &BTreeSet::new());
        let (sin, cos) = 30f32.to_radians().sin_cos();
        let turned: Vec<_> = g.faces.iter().filter(|v| v.pos[0] >= 3.0).collect();
        assert_eq!(turned.len(), 6);
        for v in turned {
            let n = v.normal;
            assert!((n[0]).abs() < 1e-6 && (n[1] + sin).abs() < 1e-6 && (n[2] - cos).abs() < 1e-6);
        }
        assert!(
            g.faces.iter().any(|v| (v.pos[2] - sin).abs() < 1e-6),
            "the far side is lifted"
        );
        let lifted_box = g
            .edges
            .iter()
            .filter(|v| v.color == CONTEXT_BOX_COLOR)
            .any(|v| (v.pos[2] - sin).abs() < 1e-6);
        assert!(lifted_box, "the open instance's box turns with it");
    }

    #[test]
    fn a_mirrored_instance_is_wound_around_its_transformed_normal() {
        let (mut s, first, second) = two_squares();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        // The component's face is red in front only; the second placement is mirrored in z.
        let d = s.root().instance(first).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        s.paint(
            MeshId::Definition(d),
            &[face],
            &[],
            kernel::Side::Front,
            Some(red),
        )
        .unwrap();
        let mirror = DAffine3::from_scale(DVec3::new(1.0, 1.0, -1.0));
        s.edit(MeshId::Root, |m| {
            Ok(m.transform_instances(&[second], mirror))
        })
        .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        let mut checked = 0;
        for tri in g.faces.chunks(3).filter(|t| t[0].pos[0] >= 3.0) {
            let [a, b, c] = [0, 1, 2].map(|i| glam::Vec3::from_array(tri[i].pos));
            let n = glam::Vec3::from_array(tri[0].normal);
            assert_eq!(n, glam::Vec3::NEG_Z);
            assert!(
                (b - a).cross(c - a).dot(n) > 0.0,
                "counter-clockwise around the normal"
            );
            assert!(tri.iter().all(|v| v.front == [1.0, 0.0, 0.0]));
            checked += 1;
        }
        assert_eq!(checked, 2);
        // The unmirrored placement still faces +Z.
        assert!(
            g.faces
                .iter()
                .filter(|v| v.pos[0] <= 1.0)
                .all(|v| v.normal == [0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn guides_are_placed_and_lit_when_selected() {
        let (mut s, first, _) = two_squares();
        let d = s.root().instance(first).unwrap().definition;
        let inner = s
            .edit(MeshId::Definition(d), |m| {
                m.add_guide(Guide::Point {
                    position: DVec3::new(0.5, 0.5, 0.0),
                    tag: None,
                })
            })
            .unwrap()
            .summary()
            .created[0];
        let root = s
            .edit(MeshId::Root, |m| {
                m.add_guide(Guide::Line {
                    point: DVec3::ZERO,
                    direction: DVec3::Y,
                    tag: None,
                })
            })
            .unwrap()
            .summary()
            .created[0];
        let g = scene_geometry(&s, &[], &BTreeSet::from([root]));
        // The component is placed twice, so its guide point is drawn twice.
        assert_eq!(g.guides.len(), 3);
        let line = g.guides.iter().find(|s| s.direction.is_some()).unwrap();
        assert_eq!(line.color, SELECTED_EDGE_COLOR);
        let mut points: Vec<f64> = g
            .guides
            .iter()
            .filter(|s| s.direction.is_none())
            .map(|s| s.point.x)
            .collect();
        points.sort_by(f64::total_cmp);
        assert_eq!(points, vec![0.5, 3.5]);
        // Inside the first square, the root's guide line is faded.
        let open = scene_geometry(&s, &[first], &BTreeSet::from([inner]));
        let faded = open.guides.iter().filter(|s| s.color == FADED_GUIDE_COLOR);
        assert_eq!(
            faded.count(),
            2,
            "the root's line and the other square's point"
        );
        assert!(open.guides.iter().any(|s| s.color == SELECTED_EDGE_COLOR));
    }

    #[test]
    fn annotations_are_placed_lit_when_selected_faded_outside_and_hidden_by_tags() {
        use kernel::{Anchor, Dimension, Note};
        let (mut s, first, _) = two_squares();
        let d = MeshId::Definition(s.root().instance(first).unwrap().definition);
        let inner = s
            .edit(d, |m| {
                m.add_dimension(Dimension::Linear {
                    a: Anchor::Point(DVec3::ZERO),
                    b: Anchor::Point(DVec3::X),
                    offset: DVec3::new(0.0, -0.5, 0.0),
                    tag: None,
                })
            })
            .unwrap()
            .summary()
            .created[0];
        let leader = Note::Leader {
            anchor: Anchor::Point(DVec3::ZERO),
            position: DVec3::new(0.0, 0.0, 2.0),
            text: Some("Here".into()),
            tag: None,
        };
        let note = s.add_note(MeshId::Root, leader).unwrap().summary().created[0];
        let screen = Note::Screen {
            at: [0.1, 0.1],
            text: "Title".into(),
            tag: None,
        };
        s.add_note(MeshId::Root, screen).unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::from([note]));
        // The component's dimension twice (placed twice), the leader once, the screen note
        // not at all.
        assert_eq!(g.annotations.len(), 3);
        let lines: Vec<f64> = g
            .annotations
            .iter()
            .filter_map(|a| match a.shape {
                AnnotationShape::Linear { line, .. } => Some(line[0].x),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec![0.0, 3.0], "placed by each instance");
        let lit: Vec<_> = g
            .annotations
            .iter()
            .filter(|a| a.color == SELECTED_EDGE_COLOR)
            .collect();
        assert_eq!(lit.len(), 1);
        assert!(matches!(lit[0].shape, AnnotationShape::Leader { .. }));
        // Inside the first square: its dimension can be selected; the rest is faded.
        let open = scene_geometry(&s, &[first], &BTreeSet::from([inner]));
        let colors: Vec<[f32; 4]> = open.annotations.iter().map(|a| a.color).collect();
        assert_eq!(
            colors
                .iter()
                .filter(|c| **c == FADED_ANNOTATION_COLOR)
                .count(),
            2
        );
        assert!(colors.contains(&SELECTED_EDGE_COLOR));
        // Selecting the instance lights what it places, its dimension too.
        let base = scene_base(&s, &[]);
        let r = base.spans.ranges(EntityId::Instance(first)).unwrap();
        assert_eq!(r.annotations.len(), 1);
        // On a hidden tag, the leader is not drawn.
        let (hidden, _) = s
            .add_tag(kernel::Tag {
                name: "Hidden".into(),
                color: [0, 0, 0],
                visible: false,
            })
            .unwrap();
        s.set_tags(MeshId::Root, &[note], Some(hidden)).unwrap();
        assert_eq!(
            scene_geometry(&s, &[], &BTreeSet::new()).annotations.len(),
            2
        );
    }

    #[test]
    fn a_selected_instance_lights_up_what_it_places() {
        let (s, first, _) = two_squares();
        let selection = BTreeSet::from([EntityId::Instance(first)]);
        let g = scene_geometry(&s, &[], &selection);
        let lit: Vec<_> = g.faces.iter().filter(|v| v.selected == 1.0).collect();
        assert_eq!(lit.len(), 6);
        assert!(lit.iter().all(|v| v.pos[0] <= 1.0), "only the first square");
        let blue = g
            .edges
            .iter()
            .filter(|v| v.color == SELECTED_EDGE_COLOR)
            .count();
        assert_eq!(blue, 8);
    }

    #[test]
    fn an_open_instance_fades_the_rest_and_is_boxed() {
        let (s, first, _) = two_squares();
        let d = s.root().instance(first).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        let selection = BTreeSet::from([EntityId::Face(face)]);
        let g = scene_geometry(&s, &[first], &selection);
        let lit: Vec<_> = g.faces.iter().filter(|v| v.selected == 1.0).collect();
        assert_eq!(lit.len(), 6, "the face is lit in the open placement only");
        assert!(lit.iter().all(|v| v.pos[0] <= 1.0));
        let count = |c: [f32; 4]| g.edges.iter().filter(|v| v.color == c).count();
        assert_eq!(count(EDGE_COLOR), 8, "the open square's edges");
        assert_eq!(count(FADED_EDGE_COLOR), 8, "the other square's edges");
        assert_eq!(count(CONTEXT_BOX_COLOR), 24, "twelve box edges");
    }

    #[test]
    fn the_open_context_box_holds_only_what_is_shown() {
        let (mut s, first, _) = two_squares();
        let d = MeshId::Definition(s.root().instance(first).unwrap().definition);
        // An edge reaching up to z = 5 inside the component, on a hidden tag.
        let tall = s
            .edit(d, |m| m.add_edge(DVec3::ZERO, DVec3::new(0.0, 0.0, 5.0)))
            .unwrap()
            .summary()
            .created
            .into_iter()
            .filter(|id| matches!(id, EntityId::Edge(_)))
            .collect::<Vec<_>>();
        let (hidden, _) = s
            .add_tag(kernel::Tag {
                name: "Hidden".into(),
                color: [0, 0, 0],
                visible: false,
            })
            .unwrap();
        let top = |s: &Scene| {
            let g = scene_geometry(s, &[first], &BTreeSet::new());
            g.edges
                .iter()
                .filter(|v| v.color == CONTEXT_BOX_COLOR)
                .map(|v| v.pos[2])
                .fold(f32::MIN, f32::max)
        };
        assert_eq!(top(&s), 5.0, "shown, the edge is boxed");
        s.set_tags(d, &tall, Some(hidden)).unwrap();
        assert_eq!(top(&s), 0.0, "hidden, it is not");
    }

    fn texture(width: f64, height: f64) -> kernel::Texture {
        kernel::Texture {
            image: std::sync::Arc::new(kernel::TextureImage {
                format: kernel::ImageFormat::Png,
                encoded: Vec::new(),
                width: 1,
                height: 1,
                pixels: vec![255; 4],
            }),
            width,
            height,
        }
    }

    #[test]
    fn textured_sides_name_their_layer_and_carry_texture_coordinates() {
        // The unit square moved far out, so its texture coordinates would be large unshifted.
        let mut m = Model::new();
        let pts = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            .map(|(x, y)| DVec3::new(x + 1000.0, y + 2000.0, 0.0));
        m.add_polygon(&pts).unwrap();
        let mut s = Scene::from_root(m);
        let face = s.root().faces().next().unwrap().0;
        let (plain, _) = s.add_material("Plain", [10, 20, 30]).unwrap();
        let (tiles, _) = s
            .add_material_with_texture("Tiles", [1, 2, 3], 1.0, Some(texture(2.0, 0.5)))
            .unwrap();
        let (wood, _) = s
            .add_material_with_texture("Wood", [4, 5, 6], 1.0, Some(texture(1.0, 1.0)))
            .unwrap();
        assert_eq!(
            crate::texture::layers(&s),
            HashMap::from([(tiles, 0), (wood, 1)])
        );
        s.paint(MeshId::Root, &[face], &[], kernel::Side::Front, Some(tiles))
            .unwrap();
        s.paint(MeshId::Root, &[face], &[], kernel::Side::Back, Some(plain))
            .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert_eq!(g.faces.len(), 6);
        for v in &g.faces {
            assert_eq!(v.layer, [0.0, -1.0]);
            assert_eq!(v.front, [1.0, 2.0, 3.0].map(|c| c / 255.0));
            assert_eq!(&v.uv[2..], [0.0, 0.0]);
            // u = x / 2 and v = -y / 0.5, up to whole repeats, and small.
            let (du, dv) = (
                f64::from(v.uv[0]) - f64::from(v.pos[0]) / 2.0,
                f64::from(v.uv[1]) + f64::from(v.pos[1]) / 0.5,
            );
            assert!((du - du.round()).abs() < 1e-4 && (dv - dv.round()).abs() < 1e-4);
            assert!(v.uv[0].abs() <= 3.0 && v.uv[1].abs() <= 3.0, "{:?}", v.uv);
        }
        s.paint(MeshId::Root, &[face], &[], kernel::Side::Back, Some(wood))
            .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert!(g.faces.iter().all(|v| v.layer == [0.0, 1.0]));
        // Removing the first texture moves the second to layer 0.
        s.change_material_with_texture(tiles, None, None, None, Some(None))
            .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert!(g.faces.iter().all(|v| v.layer == [-1.0, 0.0]));
        assert_eq!(g, single_pass::scene_geometry(&s, &[], &BTreeSet::new()));
    }

    #[test]
    fn placed_textures_turn_and_back_sides_read_unmirrored() {
        // A 1 m wall in the xz plane with the same 1 m texture on both sides, turned a quarter
        // counter-clockwise.
        let mut m = Model::new();
        let pts =
            [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, z)| DVec3::new(x, 0.0, z));
        m.add_polygon(&pts).unwrap();
        let face = m.faces().next().unwrap().0;
        let turn = kernel::PlacementEdit {
            rotation_deg: Some(90.0),
            ..kernel::PlacementEdit::default()
        };
        m.position_texture(&[face], turn).unwrap();
        let mut s = Scene::from_root(m);
        let (tiles, _) = s
            .add_material_with_texture("Tiles", [1, 2, 3], 1.0, Some(texture(1.0, 1.0)))
            .unwrap();
        s.paint(MeshId::Root, &[face], &[], kernel::Side::Both, Some(tiles))
            .unwrap();
        let n = s.root().face(face).unwrap().normal;
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert_eq!(g.faces.len(), 6);
        // For a viewer in front of a side, upright: how a step right and a step up move across
        // the image (its width and its height, up the image). `right` is the viewer's right.
        let image_steps = |tri: &[FaceVertex], k: usize, right: DVec3| {
            let at = |v: &FaceVertex| {
                let p = DVec3::from(v.pos.map(f64::from));
                (
                    DVec2::new(p.dot(right), p.z),
                    // v counts down from the image's top.
                    DVec2::new(f64::from(v.uv[k]), -f64::from(v.uv[k + 1])),
                )
            };
            let ((p0, t0), (p1, t1), (p2, t2)) = (at(&tri[0]), at(&tri[1]), at(&tri[2]));
            let screen = glam::DMat2::from_cols(p1 - p0, p2 - p0);
            let image = glam::DMat2::from_cols(t1 - t0, t2 - t0);
            image * screen.inverse()
        };
        for tri in g.faces.chunks(3) {
            // From the front: the image's width runs up and its height to the left.
            let front = image_steps(tri, 0, DVec3::Z.cross(n));
            assert!(
                front.x_axis.abs_diff_eq(DVec2::new(0.0, -1.0), 1e-6),
                "{front}"
            );
            assert!(
                front.y_axis.abs_diff_eq(DVec2::new(1.0, 0.0), 1e-6),
                "{front}"
            );
            // From behind: the same turn seen from the other side (clockwise), not mirrored.
            let back = image_steps(tri, 2, n.cross(DVec3::Z));
            assert!(
                back.x_axis.abs_diff_eq(DVec2::new(0.0, 1.0), 1e-6),
                "{back}"
            );
            assert!(
                back.y_axis.abs_diff_eq(DVec2::new(-1.0, 0.0), 1e-6),
                "{back}"
            );
            assert!(back.determinant() > 0.0 && front.determinant() > 0.0);
        }
        assert_eq!(g, single_pass::scene_geometry(&s, &[], &BTreeSet::new()));
    }

    /// The single pass `scene_geometry` made before the base and the selection flags were
    /// split, kept to check that the split changes nothing.
    mod single_pass {
        use super::super::*;

        pub fn scene_geometry(
            scene: &Scene,
            context: &[InstanceId],
            selection: &BTreeSet<EntityId>,
        ) -> ModelGeometry {
            let mut g = ModelGeometry::default();
            let layers = crate::texture::layers(scene);
            let mut walk = Walk {
                scene,
                context,
                layers: &layers,
                selection,
                path: Vec::new(),
                g: &mut g,
            };
            walk.mesh(MeshId::Root, DAffine3::IDENTITY, false, None);
            if !context.is_empty()
                && let Some((mesh, t)) = scene.resolve(context)
                && let Some([lo, hi]) = scene.shown_bounds(mesh, &[])
            {
                let corner = |k: usize| {
                    let c = DVec3::new(
                        if k & 1 == 0 { lo.x } else { hi.x },
                        if k & 2 == 0 { lo.y } else { hi.y },
                        if k & 4 == 0 { lo.z } else { hi.z },
                    );
                    t.transform_point3(c).as_vec3().to_array()
                };
                for (a, b) in BOX_EDGES {
                    for k in [a, b] {
                        g.edges.push(LineVertex {
                            pos: corner(k),
                            color: CONTEXT_BOX_COLOR,
                        });
                    }
                }
            }
            g
        }

        struct Walk<'a> {
            scene: &'a Scene,
            context: &'a [InstanceId],
            layers: &'a Layers,
            selection: &'a BTreeSet<EntityId>,
            path: Vec<InstanceId>,
            g: &'a mut ModelGeometry,
        }

        impl Walk<'_> {
            fn mesh(&mut self, id: MeshId, t: DAffine3, lit: bool, inherited: Option<MaterialId>) {
                let Some(model) = self.scene.mesh(id) else {
                    return;
                };
                let open = self.path == self.context;
                let inside = self.path.starts_with(self.context);
                let selected = |e: EntityId| lit || (open && self.selection.contains(&e));
                let normal_map = t.matrix3.inverse().transpose();
                let mirrored = t.matrix3.determinant() < 0.0;
                let smooth = model.smooth_normals();
                for (fid, face) in model.faces() {
                    let flag = if selected(EntityId::Face(fid)) {
                        1.0
                    } else {
                        0.0
                    };
                    let front = side_look(
                        self.scene,
                        self.layers,
                        face.front.or(inherited),
                        DEFAULT_FRONT,
                    );
                    let back = side_look(
                        self.scene,
                        self.layers,
                        face.back.or(inherited),
                        DEFAULT_BACK,
                    );
                    for tri in model.shaded_triangles(fid, &smooth).unwrap_or_default() {
                        let positions = tri.corners.map(|p| face.texture_position(p));
                        let uv = both_uv(&front, &back, positions);
                        let mut corners =
                            [0, 1, 2].map(|k| (tri.corners[k], tri.normals[k], uv[k]));
                        if mirrored {
                            corners.swap(1, 2);
                        }
                        for (p, n, uv) in corners {
                            self.g.faces.push(FaceVertex {
                                pos: t.transform_point3(p).as_vec3().to_array(),
                                normal: (normal_map * n).normalize().as_vec3().to_array(),
                                selected: flag,
                                front: front.color,
                                back: back.color,
                                alpha: [front.alpha, back.alpha],
                                uv,
                                layer: [front.layer(), back.layer()],
                                glow: [front.glow(), back.glow()],
                            });
                        }
                    }
                }
                for (eid, edge) in model.edges().filter(|(_, e)| !e.soft) {
                    let color = if selected(EntityId::Edge(eid)) {
                        SELECTED_EDGE_COLOR
                    } else if inside {
                        EDGE_COLOR
                    } else {
                        FADED_EDGE_COLOR
                    };
                    for v in edge.v {
                        if let Some(p) = model.position(v) {
                            self.g.edges.push(LineVertex {
                                pos: t.transform_point3(p).as_vec3().to_array(),
                                color,
                            });
                        }
                    }
                }
                for (gid, guide) in model.guides() {
                    let color = if selected(EntityId::Guide(gid)) {
                        SELECTED_EDGE_COLOR
                    } else if inside {
                        GUIDE_COLOR
                    } else {
                        FADED_GUIDE_COLOR
                    };
                    let (point, direction) = match guide.transformed(t) {
                        Guide::Line {
                            point, direction, ..
                        } => (point, Some(direction)),
                        Guide::Point { position, .. } => (position, None),
                    };
                    self.g.guides.push(GuideShape {
                        point,
                        direction,
                        color,
                    });
                }
                for (iid, instance) in model.instances() {
                    let lit = selected(EntityId::Instance(iid));
                    self.path.push(iid);
                    self.mesh(
                        MeshId::Definition(instance.definition),
                        t * instance.transform,
                        lit,
                        instance.material.or(inherited),
                    );
                    self.path.pop();
                }
            }
        }
    }

    fn created<T>(c: &kernel::SceneChange, pick: impl Fn(EntityId) -> Option<T>) -> Vec<T> {
        c.summary()
            .created
            .iter()
            .filter_map(|e| pick(*e))
            .collect()
    }

    /// The instance in `mesh` that is not one of `old`.
    fn new_instance(s: &Scene, mesh: MeshId, old: &[InstanceId]) -> InstanceId {
        s.mesh(mesh)
            .unwrap()
            .instances()
            .map(|(id, _)| id)
            .find(|id| !old.contains(id))
            .unwrap()
    }

    /// A box component placed four times (turned, mirrored and scaled, painted), two of them
    /// inside a "tree" component that also holds a face with a hole, a curved (soft-edged) solid,
    /// a guide and a group; the tree is placed three times (turned, and mirrored and painted).
    /// Returns the scene and the contexts to look from.
    fn forest() -> (Scene, Vec<Vec<InstanceId>>) {
        let mut root = Model::new();
        let base = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        root.add_polygon(&base).unwrap();
        let bottom = root.faces().next().unwrap().0;
        root.push_pull(bottom, -1.0).unwrap();
        let mut s = Scene::from_root(root);
        let (red, _) = s.add_material("Red", [200, 30, 30]).unwrap();
        let (green, _) = s.add_material("Green", [30, 160, 40]).unwrap();
        let (blue, _) = s.add_material("Blue", [40, 60, 220]).unwrap();
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (leaf, _) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Component,
                "Leaf",
            )
            .unwrap();
        let leaf_def = MeshId::Definition(s.root().instance(leaf).unwrap().definition);
        let top = s
            .mesh(leaf_def)
            .unwrap()
            .faces()
            .find(|(_, f)| f.normal.z > 0.5)
            .unwrap()
            .0;
        s.paint(leaf_def, &[top], &[], kernel::Side::Front, Some(red))
            .unwrap();
        s.edit(leaf_def, |m| {
            m.add_guide(Guide::Point {
                position: DVec3::new(0.5, 0.5, 0.5),
                tag: None,
            })
        })
        .unwrap();
        // Two more leaves, one turned and one mirrored and stretched.
        s.copy_instances(MeshId::Root, &[leaf], DVec3::new(3.0, 0.0, 0.0), 1)
            .unwrap();
        let turned = new_instance(&s, MeshId::Root, &[leaf]);
        let turn = DAffine3::from_translation(DVec3::new(3.0, 0.0, 0.0))
            * DAffine3::from_rotation_z(0.6)
            * DAffine3::from_rotation_x(0.3)
            * DAffine3::from_translation(DVec3::new(-3.0, 0.0, 0.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[turned], turn)))
            .unwrap();
        s.copy_instances(MeshId::Root, &[leaf], DVec3::new(0.0, 3.0, 0.0), 1)
            .unwrap();
        let mirrored = new_instance(&s, MeshId::Root, &[leaf, turned]);
        let mirror = DAffine3::from_translation(DVec3::new(0.0, 3.0, 0.0))
            * DAffine3::from_scale(DVec3::new(-1.0, 1.5, 1.0))
            * DAffine3::from_translation(DVec3::new(0.0, -3.0, 0.0));
        s.edit(MeshId::Root, |m| {
            Ok(m.transform_instances(&[mirrored], mirror))
        })
        .unwrap();
        // A ground face with a hole, a curved solid (soft sides) and a guide line.
        let at = |x: f64, y: f64| DVec3::new(x, y, 0.0);
        s.edit(MeshId::Root, |m| {
            m.add_polygon(&[at(5.0, 0.0), at(9.0, 0.0), at(9.0, 4.0), at(5.0, 4.0)])
        })
        .unwrap();
        s.edit(MeshId::Root, |m| {
            m.add_polygon(&[at(6.0, 1.0), at(8.0, 1.0), at(8.0, 3.0), at(6.0, 3.0)])
        })
        .unwrap();
        let curve = s
            .edit(MeshId::Root, |m| {
                m.add_curve(&[at(5.0, 6.0), at(6.0, 6.0), at(5.5, 6.8)], true, false)
            })
            .unwrap();
        let cap = created(&curve, |e| match e {
            EntityId::Face(f) => Some(f),
            _ => None,
        })[0];
        s.edit(MeshId::Root, |m| m.push_pull(cap, 1.0)).unwrap();
        s.edit(MeshId::Root, |m| {
            m.add_guide(Guide::Line {
                point: at(0.0, -1.0),
                direction: DVec3::X,
                tag: None,
            })
        })
        .unwrap();
        assert!(s.root().faces().any(|(_, f)| f.holes.len() == 1));
        assert!(s.root().edges().any(|(_, e)| e.soft));
        // The ground pieces become a group inside the tree.
        let ground: Vec<_> = s
            .root()
            .faces()
            .filter(|(_, f)| f.normal.z.abs() > 0.5)
            .filter(|(id, _)| {
                s.root().face(*id).unwrap().outer.iter().all(|v| {
                    s.root()
                        .position(*v)
                        .is_some_and(|p| p.x > 4.5 && p.y < 4.5)
                })
            })
            .map(|(id, _)| id)
            .collect();
        let ground_edges: Vec<_> = s
            .root()
            .edges()
            .filter(|(_, e)| {
                e.v.iter().all(|v| {
                    s.root()
                        .position(*v)
                        .is_some_and(|p| p.x > 4.5 && p.y < 4.5)
                })
            })
            .map(|(id, _)| id)
            .collect();
        let (group, _) = s
            .make_group(
                MeshId::Root,
                &ground_edges,
                &ground,
                &[],
                DefinitionKind::Group,
                "Ground",
            )
            .unwrap();
        let rest_edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let rest_faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (tree, _) = s
            .make_group(
                MeshId::Root,
                &rest_edges,
                &rest_faces,
                &[turned, mirrored, group],
                DefinitionKind::Component,
                "Tree",
            )
            .unwrap();
        let tree_def = MeshId::Definition(s.root().instance(tree).unwrap().definition);
        // One leaf stays in the root, painted; the tree is placed thrice.
        s.copy_instances(MeshId::Root, &[leaf], DVec3::new(-3.0, 0.0, 0.0), 1)
            .unwrap();
        let lone = new_instance(&s, MeshId::Root, &[leaf, tree]);
        s.paint(MeshId::Root, &[], &[lone], kernel::Side::Front, Some(green))
            .unwrap();
        s.copy_instances(MeshId::Root, &[tree], DVec3::new(0.0, 12.0, 0.0), 1)
            .unwrap();
        let tree2 = new_instance(&s, MeshId::Root, &[leaf, tree, lone]);
        let spin = DAffine3::from_translation(DVec3::new(0.0, 12.0, 0.0))
            * DAffine3::from_rotation_z(1.1)
            * DAffine3::from_translation(DVec3::new(0.0, -12.0, 0.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[tree2], spin)))
            .unwrap();
        s.copy_instances(MeshId::Root, &[tree], DVec3::new(20.0, 0.0, 0.0), 1)
            .unwrap();
        let tree3 = new_instance(&s, MeshId::Root, &[leaf, tree, lone, tree2]);
        let flip = DAffine3::from_translation(DVec3::new(20.0, 0.0, 0.0))
            * DAffine3::from_scale(DVec3::new(-1.0, 1.0, 1.0))
            * DAffine3::from_translation(DVec3::new(-20.0, 0.0, 0.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[tree3], flip)))
            .unwrap();
        s.paint(MeshId::Root, &[], &[tree3], kernel::Side::Front, Some(blue))
            .unwrap();
        s.validate().unwrap();
        let inner: Vec<_> = s
            .mesh(tree_def)
            .unwrap()
            .instances()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(inner.len(), 3, "two leaves and the group");
        let contexts = vec![
            vec![],
            vec![tree],
            vec![tree2],
            vec![tree3],
            vec![lone],
            vec![tree, inner[0]],
            vec![tree3, inner[1]],
            vec![tree2, inner[2]],
            vec![InstanceId(u64::MAX)],
        ];
        (s, contexts)
    }

    /// Selections to try in `context`: none, everything, parts of each kind, each instance
    /// alone, and ids that are not in the open mesh.
    fn selections(s: &Scene, context: &[InstanceId]) -> Vec<BTreeSet<EntityId>> {
        let mesh = s.resolve(context).map_or(MeshId::Root, |(m, _)| m);
        let m = s.mesh(mesh).unwrap();
        let all: BTreeSet<EntityId> = m
            .vertices()
            .map(|(id, _)| EntityId::Vertex(id))
            .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .chain(m.instances().map(|(id, _)| EntityId::Instance(id)))
            .chain(m.guides().map(|(id, _)| EntityId::Guide(id)))
            .collect();
        let mut some: BTreeSet<EntityId> = m
            .faces()
            .step_by(2)
            .map(|(id, _)| EntityId::Face(id))
            .collect();
        some.extend(m.edges().step_by(3).map(|(id, _)| EntityId::Edge(id)));
        some.extend(m.guides().map(|(id, _)| EntityId::Guide(id)));
        let mut out = vec![BTreeSet::new(), all, some];
        out.extend(
            m.instances()
                .map(|(id, _)| BTreeSet::from([EntityId::Instance(id)])),
        );
        // The root's ids, stale unless the root is open, plus ids nothing has.
        let mut elsewhere: BTreeSet<EntityId> = s
            .root()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .chain(s.root().instances().map(|(id, _)| EntityId::Instance(id)))
            .collect();
        elsewhere.extend([
            EntityId::Face(FaceId(u64::MAX)),
            EntityId::Edge(EdgeId(u64::MAX)),
            EntityId::Instance(InstanceId(u64::MAX)),
            EntityId::Guide(GuideId(u64::MAX)),
        ]);
        out.push(elsewhere);
        out
    }

    #[test]
    fn base_and_selection_flags_match_the_single_pass() {
        let (s, contexts) = forest();
        let mut lit_any = 0;
        for context in &contexts {
            let base = scene_base(&s, context);
            for selection in selections(&s, context) {
                let expected = single_pass::scene_geometry(&s, context, &selection);
                let flags = base.spans.flags(&selection);
                assert_eq!(
                    base.with_flags(&flags),
                    expected,
                    "context {context:?}, selection {selection:?}"
                );
                assert_eq!(scene_geometry(&s, context, &selection), expected);
                if flags.faces.contains(&1.0) {
                    lit_any += 1;
                }
            }
        }
        assert!(lit_any > 20, "selections lit something: {lit_any}");
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        let (cos_tilt, n) = (0.3f64.cos() as f32, g.faces.len());
        assert!(n > 400, "every placement drawn: {n} corners");
        assert!(
            g.faces
                .iter()
                .any(|v| (v.normal[2] - cos_tilt).abs() < 1e-6),
            "the tilted leaf's top tilts"
        );
        for color in [[200, 30, 30], [30, 160, 40], [40, 60, 220]] {
            let c = color.map(|c| f32::from(c as u8) / 255.0);
            assert!(g.faces.iter().any(|v| v.front == c), "{color:?} is shown");
        }
    }

    #[test]
    fn meshes_shared_out_among_threads_give_the_same_geometry() {
        // 24 groups of a pulled 48-gon, 50 faces each: enough faces for helper threads. A copy
        // of a group gets a mesh of its own.
        let ring: Vec<DVec3> = (0..48)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 48.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        let mut s = Scene::new();
        let made = s
            .edit(MeshId::Root, |m| m.add_curve(&ring, true, false))
            .unwrap();
        let cap = created(&made, |e| match e {
            EntityId::Face(f) => Some(f),
            _ => None,
        })[0];
        s.edit(MeshId::Root, |m| m.push_pull(cap, 1.5)).unwrap();
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (post, _) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Group,
                "Post",
            )
            .unwrap();
        s.copy_instances(MeshId::Root, &[post], DVec3::new(3.0, 0.5, 0.0), 23)
            .unwrap();
        assert_eq!(s.definitions().count(), 24);
        let faces: usize = s.meshes().map(|(_, m)| m.faces().count()).sum();
        assert!(faces >= PARALLEL_FACES, "{faces} faces");
        let posts: Vec<InstanceId> = s.root().instances().map(|(id, _)| id).collect();
        let picked = BTreeSet::from([EntityId::Instance(posts[3]), EntityId::Instance(posts[17])]);
        for context in [vec![], vec![posts[5]]] {
            for selection in [BTreeSet::new(), picked.clone()] {
                assert_eq!(
                    scene_geometry(&s, &context, &selection),
                    single_pass::scene_geometry(&s, &context, &selection),
                    "context {context:?}"
                );
            }
        }
    }

    #[test]
    fn ranges_match_the_flags_of_a_one_entity_selection() {
        let (s, contexts) = forest();
        // The root holds the guide and the instances; the tree, faces and edges.
        let mut kinds = BTreeSet::new();
        for context in &contexts[..2] {
            let spans = scene_base(&s, context).spans;
            let ids = [
                spans.faces.first().map(|f| EntityId::Face(f.0)),
                spans.edges.first().map(|e| EntityId::Edge(e.0)),
                spans.guides.first().map(|g| EntityId::Guide(g.0)),
                spans.instances.first().map(|i| EntityId::Instance(i.0)),
            ];
            for (kind, id) in ids.into_iter().enumerate() {
                let Some(id) = id else { continue };
                kinds.insert(kind);
                let r = spans.ranges(id).expect("an entity of the open context");
                let mut f = spans.flags(&BTreeSet::new());
                f.faces[r.faces].fill(1.0);
                f.edges[r.edges].fill(1.0);
                f.guides[r.guides].fill(true);
                assert_eq!(f, spans.flags(&BTreeSet::from([id])), "{id:?}");
            }
            let unknown = EntityId::Instance(InstanceId(u64::MAX));
            assert_eq!(spans.ranges(unknown), None);
        }
        assert_eq!(kinds.len(), 4, "a face, an edge, a guide and an instance");
        let (mesh, _) = s.resolve(&contexts[1]).unwrap();
        let vertex = s.mesh(mesh).unwrap().vertices().next().unwrap().0;
        let spans = scene_base(&s, &contexts[1]).spans;
        assert_eq!(spans.ranges(EntityId::Vertex(vertex)), None);
    }

    #[test]
    fn flags_ignore_vertices_and_ids_outside_the_open_mesh() {
        let (s, contexts) = forest();
        let tree = &contexts[1];
        let base = scene_base(&s, tree);
        let root_ids: BTreeSet<EntityId> = s
            .root()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .chain(s.root().instances().map(|(id, _)| EntityId::Instance(id)))
            .collect();
        assert!(!root_ids.is_empty());
        let none = base.spans.flags(&BTreeSet::new());
        assert_eq!(base.spans.flags(&root_ids), none);
        let (mesh, _) = s.resolve(tree).unwrap();
        let vertices: BTreeSet<EntityId> = s
            .mesh(mesh)
            .unwrap()
            .vertices()
            .map(|(id, _)| EntityId::Vertex(id))
            .collect();
        assert_eq!(base.spans.flags(&vertices), none);
        assert_eq!(none.faces.len(), base.faces.len());
        assert_eq!(none.edges.len(), base.edges.len());
        assert_eq!(none.guides.len(), base.guides.len());
        assert!(none.faces.iter().chain(&none.edges).all(|f| *f == 0.0));
    }
}
