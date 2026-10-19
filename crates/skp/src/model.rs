//! What the reader hands out.
//!
//! This is the contract in DONE.md section 0, and it exists as one file so
//! that the TypeScript side has exactly one thing to mirror. Two rules shape
//! all of it:
//!
//! 1. **Flat.** Every bulk field is a typed array, never a collection of
//!    per-entity structs. The TypeScript reader allocates roughly 65 million
//!    small objects for a five-million-triangle model and dies at a 16 GB
//!    heap; the C++ one keeps a red-black tree node per vertex and spends
//!    1.05 GB on a 46 MB file. Flat arrays are the whole reason this exists.
//!
//! 2. **Nothing is flattened.** Component instances stay as a definition plus
//!    a transform. A 27.7 MB commercial classroom is 38,019 placed nodes over
//!    629 unique meshes; baking those placements is what turns a model that
//!    renders into one that never paints a frame.

use alloc::string::String;
use alloc::vec::Vec;

/// Millimetres. The file stores inches; everything here is converted once, on
/// the way out, so no consumer has to remember which is which.
pub type Mm = f64;

pub const INCH_TO_MM: f64 = 25.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Inches,
    Feet,
    Millimetres,
    Centimetres,
    Metres,
}

/// A saved scene. Nineteen camera tags exist in the format and the ones that
/// change the framing are all here - in particular `fov_is_height`, without
/// which a field of view cannot be interpreted at all, and the near/far pair,
/// which SketchUp derives from the model bounds and then caps at 3000:1.
#[derive(Debug, Clone, Default)]
pub struct SceneDef {
    pub name: String,
    pub description: String,
    pub eye: Option<[Mm; 3]>,
    pub target: Option<[Mm; 3]>,
    pub up: Option<[f64; 3]>,
    pub fov_deg: f64,
    /// Whether `fov_deg` is already the vertical field of view. When false it
    /// is a horizontal one and must be divided by the image aspect.
    pub fov_is_height: bool,
    pub fixed_fov: bool,
    pub perspective: bool,
    /// Full vertical extent of the frustum in parallel projection.
    pub ortho_height: Mm,
    /// Zero when the camera imposes no aspect of its own.
    pub aspect_ratio: f64,
    /// Aperture width in millimetres for focal-length reporting; zero means
    /// SketchUp's default 36 mm frame.
    pub image_width_mm: f64,
    pub near: Mm,
    pub far: Mm,
    pub allow_clipping_in_parallel: bool,
    /// Flags recording which properties this scene actually restores.
    pub flags: u32,
    pub hidden_layer_ids: Vec<u32>,
    pub style_id: Option<u32>,
    /// The scene's own line settings, when it saves a set of its own.
    ///
    /// A scene stores more than a reference to a style. SketchUp's
    /// `use_rendering_options` flag means the page carries a complete
    /// `RenderingOptions` block that overrides the model's for as long as that
    /// scene is active, and files really do use it: the school cafeteria's
    /// model style says silhouettes off at width 5 while its one saved scene
    /// says on at width 5, and each of the restaurant's six scenes selects
    /// different settings. With only `style_id` to carry, a scene that saves
    /// its own line weights is drawn with the model's and the artist's export
    /// does not match what they saved.
    ///
    /// `None` means the scene restores the model's options, which is the
    /// common case and is not the same as "an empty set".
    ///
    /// The id and name of a `StyleDef` here are the style the scene selected,
    /// if it selected one; the rest is the scene's own.
    pub rendering_options: Option<StyleDef>,
    pub section_plane_ids: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct LayerDef {
    pub id: u32,
    pub name: String,
    pub hidden: bool,
    pub color: [u8; 4],
    /// Folder path, outermost first. SketchUp calls these tag folders and they
    /// nest, so a flat name list cannot represent them.
    pub folder: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MaterialDef {
    pub id: u32,
    pub name: String,
    pub rgba: [u8; 4],
    /// 1.0 is opaque.
    pub opacity: f32,
    /// Where the image bytes are, not the bytes. On the 517 MB pack those
    /// bytes are 380 MB and line art needs none of them.
    pub texture: Option<TextureRef>,
    pub colorized: bool,
}

#[derive(Debug, Clone)]
pub struct TextureRef {
    pub name: String,
    /// Zip entry index for VFF, or a byte range for a legacy file's inline blob.
    pub location: TextureLocation,
    /// Tile size in millimetres - not pixels. Getting this wrong tiles a
    /// 1.9 m sheet 150 times across a 600 mm panel.
    pub tile_mm: [f64; 2],
}

#[derive(Debug, Clone, Copy)]
pub enum TextureLocation {
    ZipEntry(u32),
    Inline { offset: u64, len: u64 },
}

impl TextureRef {
    /// `tile_mm`, with the values a real file can hold and a renderer cannot
    /// use replaced by one it can.
    ///
    /// Two failures occur in the corpus and both are in `materials.rs`'s
    /// notes. A material can declare `hasTexture="1"` and no scale at all,
    /// which arrives here as zero and would divide the UVs by nothing; and the
    /// commercial classroom's `Wood_Grain_Stain` carries
    /// `xScale="3543307" yScale="35"`, a 3.543307 inch tile with its decimal
    /// point lost, which is 90 kilometres and paints a single texel across the
    /// whole model. The raw values stay in `tile_mm` because the audit should
    /// be able to see them; this is what to draw with.
    ///
    /// The clamp is [1 mm, 100 m] and the substitute for a missing value is
    /// 1 metre, which is roughly the size of a wall texture and is what makes
    /// a missing scale look wrong rather than look like nothing.
    pub fn tile_mm_drawable(&self) -> [f64; 2] {
        let one = |v: f64| -> f64 {
            if !v.is_finite() || v <= 0.0 {
                1000.0
            } else {
                v.clamp(1.0, 100_000.0)
            }
        };
        [one(self.tile_mm[0]), one(self.tile_mm[1])]
    }
}

/// The line-art settings that decide what a hidden-line render looks like.
/// Names and ids come from SketchUp's own dictionary, where 78 of these exist.
#[derive(Debug, Clone)]
pub struct StyleDef {
    pub id: u32,
    pub name: String,
    pub render_mode: i32,
    pub draw_silhouettes: bool,
    pub silhouette_width: i32,
    pub extend_lines: bool,
    pub line_extension: i32,
    pub jitter_edges: bool,
    pub draw_depth_cue: bool,
    pub depth_cue_width: i32,
    pub draw_line_ends: bool,
    pub line_end_width: i32,
    pub draw_profiles_only: bool,
    pub draw_back_edges: bool,
    pub edge_display_mode: i32,
    pub edge_color_mode: i32,
    pub display_color_by_layer: bool,
    pub texture_enabled: bool,
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub face_front: [u8; 4],
    pub face_back: [u8; 4],
    pub display_watermarks: bool,
    pub display_sketch_axes: bool,
}

impl Default for StyleDef {
    /// SketchUp's own factory values, read out of
    /// `CRenderingOptions::Initialize`. Note `silhouette_width` is 3, not the
    /// 2 this project has been writing, and the stock Hidden Line style ships
    /// with `draw_silhouettes` *off* - which is why softened surfaces can
    /// export as nothing at all.
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            render_mode: 2,
            draw_silhouettes: true,
            silhouette_width: 3,
            extend_lines: false,
            line_extension: 8,
            jitter_edges: false,
            draw_depth_cue: false,
            depth_cue_width: 4,
            draw_line_ends: false,
            line_end_width: 5,
            draw_profiles_only: false,
            draw_back_edges: false,
            edge_display_mode: 1,
            edge_color_mode: 1,
            display_color_by_layer: false,
            texture_enabled: true,
            background: [255, 255, 255, 255],
            foreground: [0, 0, 0, 255],
            face_front: [255, 255, 255, 255],
            face_back: [128, 128, 160, 255],
            display_watermarks: false,
            display_sketch_axes: true,
        }
    }
}

/// Everything a viewer needs before any geometry arrives: enough to draw the
/// scene list, the tag tree and a camera, within a second on any file.
#[derive(Debug, Clone, Default)]
pub struct SceneIndex {
    pub writer_version: String,
    pub generation_is_vff: bool,
    pub unit: Option<LengthUnit>,
    /// min xyz then max xyz, millimetres. None when the file records none.
    pub bbox: Option<[Mm; 6]>,
    pub scenes: Vec<SceneDef>,
    pub layers: Vec<LayerDef>,
    pub materials: Vec<MaterialDef>,
    pub styles: Vec<StyleDef>,
    pub active_style: Option<u32>,
    pub counts: Counts,
    /// The file's own 256x256 preview, if it has one.
    pub thumbnail_png: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    pub definitions: u32,
    pub instances: u32,
    pub triangles: u64,
    pub edges: u64,
    pub textures: u32,
}

/// Triangles for one definition, in that definition's own coordinates.
///
/// Positions are `f32` millimetres. Normals stay in local space on purpose:
/// the renderer derives world normals from the instance transform the way
/// glTF requires, which is what keeps mirrored and non-uniformly scaled
/// placements correct without baking a copy per placement.
#[derive(Debug, Default)]
pub struct MeshBatch {
    pub definition_id: u32,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    /// Two floats per vertex, parallel to `positions`, in **millimetres of
    /// texture space** - not tile repeats, not pixels.
    ///
    /// # The convention, for whoever fills this from the other container
    ///
    /// A consumer gets tile repeats by dividing by the material's
    /// `TextureRef::tile_mm`:
    ///
    /// ```text
    /// repeat_u = uvs[2 * v]     / tile_mm[0]
    /// repeat_v = uvs[2 * v + 1] / tile_mm[1]
    /// ```
    ///
    /// Millimetres and not repeats, for one reason that decides it: a face
    /// with no material of its own is painted by its instance (SketchUp's
    /// "paint the component"), and the reader emitting a mesh batch does not
    /// know which instance will carry it, so it cannot know the tile size to
    /// divide by. In millimetres the same vertices are correct under every
    /// paint; in repeats they would be correct under one. The renderer divides
    /// at draw time, where the material is finally known.
    ///
    /// Secondary reason: every other length in this contract is millimetres,
    /// so there is one unit to remember rather than two.
    ///
    /// # How to produce it
    ///
    /// `geom::UvMap` is the whole recipe - construct it from a face's normal
    /// and its texture transform and call `at()` per vertex. It works in
    /// whatever unit it is handed, so pass positions in millimetres and the
    /// UVs come out in millimetres. `geom.rs`'s module comment records which
    /// records the transform lives in and what is known about them.
    ///
    /// # Empty means untextured
    ///
    /// A batch whose faces carry no texture emits `uvs` **empty** rather than
    /// a run of zeros: the array is either `2 * positions.len() / 3` long or
    /// zero length, and nothing in between. Zeros would have cost 8 bytes a
    /// vertex - 20 MB on a commercial pack - to say the same nothing, and a
    /// consumer that has to check `uvs.len()` anyway (because the field was
    /// always empty before this existed) is not helped by filling it.
    ///
    /// A batch where *some* faces are textured is full length, with the
    /// untextured faces carrying their default projection, which is a real
    /// coordinate rather than a placeholder.
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    pub material_id: u32,
    pub layer_id: u32,
}

/// Edges for one definition - the material of line art, which no existing
/// reader emits into anything renderable.
///
/// `face_normals` carries both adjacent face normals per edge, already negated
/// for a reversed edge use, because SketchUp's profile test is
/// `dot(n0, v) * dot(n1, v) < 0` with an edge-on tolerance, evaluated per edge
/// in the definition's local space. `face_counts` under 2 means a boundary
/// edge, which is always a profile.
///
/// # The sign convention, stated as something that can be checked
///
/// Each normal is taken from the face through its edge *use* and negated when
/// that use is reversed - and then **the second one is negated again on
/// storage**. The invariant that follows is testable without knowing anything
/// about the file: for two COPLANAR faces sharing an edge, the two stored
/// normals must come out **equal**, not opposite.
///
/// Doing only half of the negation passes every count and every bounds check
/// and still ruins the drawing, because `d0 * d1 < 0` then fires on every
/// coplanar pair - which is every triangulation diagonal in the model. The
/// symptom is a curved surface drawn as dense hatching instead of as an
/// outline, and it only becomes visible once the softened range is drawn at
/// all. Measured over a sampled camera:
///
/// ```text
///                        edges whose two normals are     softened edges
///                        nearly antiparallel             the profile test keeps
///   VFF classroom                        0.1%                       5.5%
///   legacy cafeteria                    77.9%                      94.7%
/// ```
///
/// A real model has adjacent faces meeting at a dihedral angle, so antiparallel
/// pairs are rare. 77.9% is the missing negation, not geometry.
#[derive(Debug, Default)]
pub struct EdgeBatch {
    pub definition_id: u32,
    /// Edges are ordered in three ranges, and this is the end of the first.
    ///
    /// ```text
    /// [0, drawable)             neither hidden nor soft nor smooth
    /// [drawable, silhouette)    soft or smooth, not hidden
    /// [silhouette, len())       hidden
    /// ```
    ///
    /// The first range is what SketchUp draws unconditionally.
    ///
    /// A renderer in hidden-line mode draws only this prefix. That is not a
    /// small saving: across the corpus 79 to 82 percent of edges are soft,
    /// smooth or hidden and never appear in line art, and issuing them as
    /// instances only to discard them in the vertex shader costs the same as
    /// drawing them. On a commercial pack it is the difference between 27
    /// million edge instances a frame and 3.4 million.
    ///
    /// Showing hidden geometry just means drawing the whole array instead.
    pub drawable: u32,
    /// End of the softened range: the edges that are drawn **only** when the
    /// style has silhouettes on **and** they pass the profile test.
    ///
    /// This is not a refinement, it is a whole class of line art that was
    /// missing. Measured on a fixture SketchUp built to order, whose answer is
    /// known by construction rather than by trusting the application:
    ///
    /// ```text
    ///   shape                          sil off   sil on w1   w2    w5
    ///   plain cube                         619         619   638   643
    ///   cylinder, sides softened            74         293   306   310
    ///   cube, every edge soft and smooth    22         256   272   277
    ///   cube, every edge hidden              0           0     0     0
    /// ```
    ///
    /// So a softened edge is not drawn normally, and the ones lying on the
    /// silhouette ARE drawn the moment silhouettes are switched on - at the
    /// silhouette width. A softened cylinder with silhouettes off has no sides
    /// at all; it is a floating ellipse. A hidden edge is never drawn, at any
    /// setting, not even as a silhouette.
    ///
    /// Dropping these was not marginal: SketchUp counts 69% of the school
    /// cafeteria's 197,099 edges as soft, smooth or hidden, and without this
    /// range the viewer cannot draw the outline of any curved or softened
    /// surface from any angle - every chair back, bottle, parasol, bowl,
    /// cushion and lampshade in these packs.
    ///
    /// Zero from a producer that does not partition this way means "no
    /// softened range", which degrades to the old behaviour rather than to
    /// nonsense; `edges::sort_drawable_first` sets it.
    pub silhouette: u32,
    /// Two points per edge: 6 floats.
    pub positions: Vec<f32>,
    /// Two normals per edge: 6 floats.
    pub face_normals: Vec<f32>,
    /// bit0 hidden, bit3 soft, bit4 smooth - confirmed against
    /// `CEdge::IsSoft` and `IsSmooth`, which read bits 3 and 4.
    pub flags: Vec<u8>,
    pub face_counts: Vec<u8>,
    pub layer_ids: Vec<u32>,
}

impl EdgeBatch {
    pub fn len(&self) -> usize {
        self.flags.len()
    }
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}

/// Where a definition is placed. One row per placement, never geometry.
#[derive(Debug, Default)]
pub struct InstanceTable {
    pub definition_id: u32,
    /// Row-major 4x4 per instance, millimetres, model space.
    pub matrices: Vec<f32>,
    pub layer_ids: Vec<u32>,
    /// `u32::MAX` where the instance imposes no paint of its own.
    pub material_ids: Vec<u32>,
    /// Parallel to the rows above; empty when names were not requested.
    pub names: Vec<String>,
}

pub const NO_MATERIAL: u32 = u32::MAX;

impl InstanceTable {
    pub fn len(&self) -> usize {
        self.matrices.len() / 16
    }
    pub fn is_empty(&self) -> bool {
        self.matrices.is_empty()
    }
}

/// Flags on an edge, matching the file's own bit layout.
pub mod edge_flag {
    pub const HIDDEN: u8 = 0x01;
    pub const SOFT: u8 = 0x08;
    pub const SMOOTH: u8 = 0x10;

    /// What SketchUp draws: an edge that is neither softened, smoothed nor
    /// hidden. Across this project's fixtures 27.3% of edges fail this, and on
    /// a curved-surface model up to 66%.
    pub fn is_drawable(flags: u8) -> bool {
        flags & (HIDDEN | SOFT | SMOOTH) == 0
    }
}
