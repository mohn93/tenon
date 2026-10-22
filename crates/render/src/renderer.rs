use std::collections::BTreeSet;

use kernel::{EntityId, InstanceId, Scene};
use session::{Camera, Lighting, SectionDisplay};
use wgpu::util::DeviceExt;

use crate::annotations::{AnnotationLook, annotation_lines};
use crate::grid::{
    LineVertex, NIGHT_BACKGROUND, flat_axis_lines_in, flat_ground_lines_in, vertical_axis_lines_in,
};
use crate::guides::{GuideShape, guide_lines};
use crate::lights::{Light, MAX_LIGHTS, strongest};
use crate::scene::{
    BaseFaceVertex, CasterChunk, Ranges, Spans, scene_base_shown, shown_annotations, shown_guides,
    shown_sections,
};
use crate::sections::{SectionLook, section_lines};
use crate::shadow_maps::{Casters, ShadowMaps};
use crate::shadows::{MAX_SHADOWS, ShadowKey};
use crate::style::ViewStyle;
use crate::texture::TextureArray;

pub const SAMPLE_COUNT: u32 = 4;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const LINE_ATTRIBS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
const FACE_ATTRIBS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 3 => Float32x3, 4 => Float32x3, 5 => Float32x2,
    6 => Float32x4, 7 => Float32x2, 8 => Float32x2
];
/// The selection flag of a face or edge vertex, from a second vertex buffer (see
/// `scene::Flags`).
const FLAG_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![2 => Float32];
const PREVIEW_ATTRIBS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
/// The hover flag of a face or edge vertex, from a third vertex buffer (1 hovered, 0 not).
const FACE_HOVER_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![9 => Float32];
const EDGE_HOVER_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![3 => Float32];

/// The limits the renderer asks of a device: the defaults, with room for a cube shadow map per
/// shadowed light (six array layers each) where the adapter has it.
///
/// On the web, a GL (WebGL2) adapter gets `downlevel_webgl2_defaults` as the base instead, since
/// the full defaults exceed what WebGL2 offers.
pub fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    #[cfg(target_arch = "wasm32")]
    let defaults = if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let defaults = wgpu::Limits::default();
    let wanted = (6 * MAX_SHADOWS) as u32;
    let layers = wanted.min(adapter.limits().max_texture_array_layers);
    wgpu::Limits {
        max_texture_array_layers: layers.max(defaults.max_texture_array_layers),
        ..defaults
    }
}

/// The scene shader, with shadow lookups or without.
fn scene_shader(device: &wgpu::Device, shadows: bool) -> wgpu::ShaderModule {
    let lookup = if shadows {
        include_str!("shaders/shadow_lookup.wgsl")
    } else {
        include_str!("shaders/no_shadows.wgsl")
    };
    let source = format!("{}\n{lookup}", include_str!("shaders/scene.wgsl"));
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// True when the adapter supports cube-map array textures, which light shadows need. On the web
/// (no blocking) the app decides with this and builds the renderer with
/// [`Renderer::without_shadows`] when it is false; natively `Renderer::new` validates the shader
/// instead, which catches the same devices.
pub fn supports_shadows(adapter: &wgpu::Adapter) -> bool {
    adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES)
}

/// The scene shader with shadow lookups and how many lights may cast shadows, or None when the
/// device cannot have them: it allows no cube-map array or rejects the shader (devices without
/// cube-map arrays do). On wasm the shader is not validated here (that would block): callers
/// check [`supports_shadows`] first.
fn shadowed_shader(device: &wgpu::Device) -> Option<(wgpu::ShaderModule, usize)> {
    let max = (device.limits().max_texture_array_layers as usize / 6).min(MAX_SHADOWS);
    if max == 0 {
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = scene_shader(device, true);
        pollster::block_on(scope.pop())
            .is_none()
            .then_some((shader, max))
    }
    #[cfg(target_arch = "wasm32")]
    Some((scene_shader(device, true), max))
}

/// Camera data shared by every pipeline (matches `Uniforms` in scene.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    /// The headlight: the eye's position (w = 1), or in a parallel view the direction
    /// toward the viewer (w = 0).
    eye: [f32; 4],
    /// How lines are pulled toward the camera (see `line_vertex` in scene.wgsl): x scales
    /// clip z by the line's factor (perspective), y adds a fixed depth step (parallel).
    depth_offset: [f32; 4],
}

impl Uniforms {
    fn new(camera: &Camera) -> Self {
        let (eye, depth_offset) = if camera.is_parallel() {
            let toward = (-camera.forward()).as_vec3();
            // The step that moves a line by (factor - 1) eye-to-target distances: clip z
            // runs from 1 to 0 over the depth range.
            let (near, far) = camera.depth_range();
            let unit = (camera.distance() / (far - near)) as f32;
            ([toward.x, toward.y, toward.z, 0.0], [0.0, unit, 0.0, 0.0])
        } else {
            let eye = camera.eye.as_vec3();
            ([eye.x, eye.y, eye.z, 1.0], [1.0, 0.0, 0.0, 0.0])
        };
        Self {
            view_proj: camera.view_proj_f32().to_cols_array(),
            eye,
            depth_offset,
        }
    }
}

/// The ambient light and the headlight's strength by day: faces are lit 0.6 seen edge-on and 1
/// seen square-on.
const DAY: [f32; 2] = [0.6, 0.4];
/// The same at night, dimmed so glowing faces light the scene.
const NIGHT: [f32; 2] = [0.08, 0.12];
/// How far unselected edges, the grid and the axes fade toward the background at night, so they
/// stay quiet outlines rather than bright lines on the dark.
const NIGHT_LINE_FADE: f32 = 0.8;

/// A light as scene.wgsl reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct LightUniform {
    /// The position, and the range in w.
    position: [f32; 4],
    /// sRGB color in rgb; in w the slot of its shadow map, or -1 when it casts none.
    color: [f32; 4],
}

/// How the scene is lit (matches `Lighting` in scene.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightingUniforms {
    /// x: how many lights; y: the ambient light; z: the headlight; w: 1 at night, else 0.
    mode: [f32; 4],
    /// The background, which lines fade toward.
    background: [f32; 4],
    /// Selected edges and guides.
    selected_edge: [f32; 4],
    /// rgb: the color selected faces are tinted toward; w: how far.
    selected_face: [f32; 4],
    /// x: how far unselected edges fade toward the background; y: how far the grid and axes
    /// do; zw: the view's size in pixels.
    lines: [f32; 4],
    /// The day canvas's top and bottom.
    canvas_top: [f32; 4],
    canvas_bottom: [f32; 4],
    /// rgb: the color hovered faces are mixed toward; w: how far. Zero without a hover.
    hover_face: [f32; 4],
    /// Hovered edges and guides. Zero without a hover.
    hover_edge: [f32; 4],
    /// The step in pixels between the copies of a wide edge (x, see `vs_edge_wide`), of a wide
    /// axis (y) and of a section cut edge (z, see `vs_cut_edge_wide`).
    wide: [f32; 4],
    lights: [LightUniform; MAX_LIGHTS],
}

impl LightingUniforms {
    /// The lighting for `mode` with `used` (at most MAX_LIGHTS), each with its shadow map's
    /// slot if it casts shadows, drawn in `style` into a view `view` pixels in size.
    fn new(
        used: &[Light],
        shadow_slots: &[Option<usize>],
        mode: Lighting,
        style: &ViewStyle,
        view: (u32, u32),
    ) -> Self {
        let mut uniforms = Self {
            mode: [0.0; 4],
            background: background(mode, style),
            selected_edge: [0.0; 4],
            selected_face: [0.0; 4],
            lines: [0.0; 4],
            canvas_top: [0.0; 4],
            canvas_bottom: [0.0; 4],
            hover_face: [0.0; 4],
            hover_edge: [0.0; 4],
            wide: [0.0; 4],
            lights: [LightUniform::default(); MAX_LIGHTS],
        };
        for ((u, l), slot) in uniforms.lights.iter_mut().zip(used).zip(shadow_slots) {
            let p = l.position.as_vec3();
            u.position = [p.x, p.y, p.z, l.range() as f32];
            let slot = slot.map_or(-1.0, |s| s as f32);
            u.color = [l.color[0], l.color[1], l.color[2], slot];
        }
        let ([ambient, headlight], night) = match mode {
            Lighting::Day => (DAY, 0.0),
            Lighting::Night => (NIGHT, 1.0),
        };
        uniforms.mode = [used.len() as f32, ambient, headlight, night];
        let night = mode == Lighting::Night;
        let [r, g, b] = style.selected_face;
        uniforms.selected_edge = style.selected_edge;
        uniforms.selected_face = [r, g, b, style.face_tint];
        uniforms.lines = [
            if night { NIGHT_LINE_FADE } else { 0.0 },
            if night {
                NIGHT_LINE_FADE
            } else {
                style.day_ground_fade
            },
            view.0.max(1) as f32,
            view.1.max(1) as f32,
        ];
        uniforms.canvas_top = style.day_canvas[0];
        uniforms.canvas_bottom = style.day_canvas[1];
        uniforms
    }
}

/// The color the target is cleared to.
fn background(mode: Lighting, style: &ViewStyle) -> [f32; 4] {
    match mode {
        Lighting::Day => style.day_background(),
        Lighting::Night => NIGHT_BACKGROUND,
    }
}

/// How the hovered entity is shown: its faces mixed `face_mix` toward `face` (sRGB), its
/// edges and guides in `edge`, edges about 2 pt wide where the style widens selected edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverLook {
    pub face: [f32; 3],
    pub face_mix: f32,
    pub edge: [f32; 4],
}

/// A vertex of a tool's preview solid, in root coordinates; `color` is sRGB with its alpha.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PreviewVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

/// Selected and hovered edges, in points, where the style widens them.
const WIDE_EDGE_PT: f32 = 2.0;
/// Section cut edges, in points: twice the width of selected edges.
const CUT_EDGE_PT: f32 = 2.0 * WIDE_EDGE_PT;
/// The axes, in points, where the style widens them.
const WIDE_AXIS_PT: f32 = 1.5;

/// How a line `width` physical pixels wide is drawn from 1 px lines: the line, then
/// (instances, step): copies `step` pixels apart, half of them across and half up. A line
/// at most 1 px wide needs none.
fn spread(width: f32) -> (u32, f32) {
    let extra = width - 1.0;
    if extra <= 0.0 {
        return (0, 0.0);
    }
    let copies = extra.ceil();
    (2 * copies as u32, extra / copies)
}

/// Pulls the preview solid toward the camera (depth is reversed, so up) so that a side lying
/// in the plane of a model face, as when a face is pushed into a box, is tinted evenly rather
/// than fighting the face for depth.
const PREVIEW_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 4,
    slope_scale: 1.0,
    clamp: 0.0,
};

/// A rectangle of the render target in physical pixels, origin top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    /// The whole target.
    pub fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// Multisampled color and depth attachments matching the output size.
struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// A vertex buffer and how many vertices it holds, with, for model faces and edges, a second
/// buffer holding each vertex's selection flag and a third holding its hover flag.
struct Mesh {
    buffer: wgpu::Buffer,
    count: u32,
    flags: Option<wgpu::Buffer>,
    hover: Option<wgpu::Buffer>,
}

/// The triangles of `faces` with a see-through side, drawn after everything opaque.
struct Glass {
    /// The first vertex of each opaque triangle, for drawing them alone.
    solid: wgpu::Buffer,
    solid_count: u32,
    /// The vertices of the see-through triangles, far to near as last sorted.
    order: wgpu::Buffer,
    /// Each see-through triangle's first vertex and center.
    triangles: Vec<(u32, glam::Vec3)>,
    /// The view `order` was sorted for.
    sorted_for: Option<[f32; 4]>,
}

impl Glass {
    /// The see-through triangles of `faces`, or None when there are none.
    fn new(device: &wgpu::Device, faces: &[BaseFaceVertex], glass: &[u32]) -> Option<Glass> {
        if glass.is_empty() {
            return None;
        }
        let mut solid = Vec::with_capacity(faces.len() - 3 * glass.len());
        let mut next = glass.iter().peekable();
        for first in (0..faces.len() as u32).step_by(3) {
            if next.next_if_eq(&&first).is_none() {
                solid.extend([first, first + 1, first + 2]);
            }
        }
        let triangles: Vec<(u32, glam::Vec3)> = glass
            .iter()
            .map(|&k| {
                let corner = |i: u32| glam::Vec3::from(faces[i as usize].pos);
                (k, (corner(k) + corner(k + 1) + corner(k + 2)) / 3.0)
            })
            .collect();
        let index_buffer = |label: &str, indices: &[u32], usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                // An empty buffer still needs a size; nothing is drawn from it then.
                contents: bytemuck::cast_slice(if indices.is_empty() { &[0u32] } else { indices }),
                usage: wgpu::BufferUsages::INDEX | usage,
            })
        };
        let unsorted: Vec<u32> = glass.iter().flat_map(|&k| [k, k + 1, k + 2]).collect();
        Some(Glass {
            solid: index_buffer("solid faces", &solid, wgpu::BufferUsages::empty()),
            solid_count: solid.len() as u32,
            order: index_buffer("glass faces", &unsorted, wgpu::BufferUsages::COPY_DST),
            triangles,
            sorted_for: None,
        })
    }

    fn count(&self) -> u32 {
        3 * self.triangles.len() as u32
    }

    /// Sorts the triangles far to near from `camera`, unless they already are for its view.
    fn sort(&mut self, queue: &wgpu::Queue, camera: &Camera) {
        let (origin, toward) = (camera.eye.as_vec3(), camera.forward().as_vec3());
        let view = if camera.is_parallel() {
            [toward.x, toward.y, toward.z, 0.0]
        } else {
            [origin.x, origin.y, origin.z, 1.0]
        };
        if self.sorted_for == Some(view) {
            return;
        }
        let indices = back_to_front(&self.triangles, view);
        queue.write_buffer(&self.order, 0, bytemuck::cast_slice(&indices));
        self.sorted_for = Some(view);
    }
}

/// The vertices of `triangles` (first vertex, center) ordered far to near for `view`: the
/// eye's position (w = 1), or the view direction of a parallel view (w = 0).
fn back_to_front(triangles: &[(u32, glam::Vec3)], view: [f32; 4]) -> Vec<u32> {
    let [x, y, z, w] = view;
    let v = glam::Vec3::new(x, y, z);
    let depth = |c: glam::Vec3| {
        if w == 0.0 {
            c.dot(v)
        } else {
            c.distance_squared(v)
        }
    };
    let mut keyed: Vec<(f32, u32)> = triangles.iter().map(|&(k, c)| (depth(c), k)).collect();
    keyed.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    keyed.iter().flat_map(|&(_, k)| [k, k + 1, k + 2]).collect()
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    flat_ground_pipeline: wgpu::RenderPipeline,
    ground_pipeline: wgpu::RenderPipeline,
    /// The X and Y axes, then the Z axis, again across and up, when the style widens them.
    wide_flat_axes_pipeline: wgpu::RenderPipeline,
    wide_vertical_axis_pipeline: wgpu::RenderPipeline,
    face_pipeline: wgpu::RenderPipeline,
    /// The opaque side of a triangle whose other side is see-through.
    solid_side_pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    /// Selected and hovered edges again, across and up, when the style widens them.
    wide_edge_pipeline: wgpu::RenderPipeline,
    /// The day canvas's gradient, when the style has one.
    canvas_pipeline: wgpu::RenderPipeline,
    guide_pipeline: wgpu::RenderPipeline,
    /// The fill of closed cuts, flat, nearer than the faces it lies on.
    cut_fill_pipeline: wgpu::RenderPipeline,
    /// The cuts' outlines, and again across and up for their width (see `vs_cut_edge_wide`).
    cut_edge_pipeline: wgpu::RenderPipeline,
    wide_cut_edge_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    /// A tool's translucent preview solid.
    preview_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    /// The lighting uniforms, rewritten every frame (see `LightingUniforms`).
    lighting_uniforms: wgpu::Buffer,
    bind_layout: wgpu::BindGroupLayout,
    /// Repeats both ways, filtering linearly between pixels and mipmaps.
    sampler: wgpu::Sampler,
    /// The scene's textures, as of the last `set_model`.
    textures: TextureArray,
    /// Compares distances with the shadow maps, filtering between texels.
    shadow_sampler: wgpu::Sampler,
    /// The camera uniforms, the textures, the sampler, the lighting uniforms and, when the
    /// device has them, the shadow maps and their sampler.
    bind_group: wgpu::BindGroup,
    /// The shadow maps; None when the device cannot have them.
    shadow_maps: Option<ShadowMaps>,
    /// The casters' corners as of the last `set_model`, and their vertex buffer (None when
    /// there are none).
    caster_corners: Vec<[f32; 3]>,
    casters: Option<wgpu::Buffer>,
    caster_chunks: Vec<CasterChunk>,
    /// Counts the changes of the casters, so shadow maps drawn for older ones are drawn again.
    caster_generation: u64,
    /// Day or night, as of the last `set_lighting`.
    lighting: Lighting,
    /// Faces drawn in their tags' colors, as of the last `set_color_by_tag` (see
    /// `scene::scene_base_with`).
    color_by_tag: bool,
    /// How section planes are shown, as of the last `set_section_display`.
    section_display: SectionDisplay,
    /// How the frames look, as of the last `set_style`.
    style: ViewStyle,
    /// Printing: white paper, no canvas gradient, ground grid or axes.
    print: bool,
    /// The scene's lights, as of the last `set_model`.
    lights: Vec<Light>,
    /// The grid and the axes in the style's colors (see `set_style`); `flat_axes` is the X and
    /// Y axes alone, for their wide copies.
    flat_ground: Mesh,
    vertical_axis: Mesh,
    flat_axes: Mesh,
    faces: Option<Mesh>,
    /// The fill and the outlines of the section cuts.
    cut_fill: Option<Mesh>,
    cut_edges: Option<Mesh>,
    /// The see-through triangles of `faces`, if any.
    glass: Option<Glass>,
    edges: Option<Mesh>,
    /// Whether any edge is selected, as of the last `set_selection`.
    edges_selected: bool,
    /// The hovered entity and its look, as of the last `set_hover`, and where it lies.
    hover: Option<(EntityId, HoverLook)>,
    hover_ranges: Ranges,
    /// Physical pixels per point, as of the last `set_pixels_per_point`.
    pixels_per_point: f32,
    /// The preview solid's triangles as of the last `set_preview`, and their buffer, which
    /// is kept and grown as needed (its count is 0 while there are none).
    preview_vertices: Vec<PreviewVertex>,
    preview: Option<Mesh>,
    /// Where the open context's entities lie in `faces`, `edges` and `base_guides`.
    spans: Spans,
    /// Guides in root coordinates with nothing selected.
    base_guides: Vec<GuideShape>,
    /// The guides with the selection shown; their lines depend on the camera, so they are
    /// built per frame.
    guides: Vec<GuideShape>,
    /// Which of `base_guides` are selected, as of the last `set_selection`.
    guide_flags: Vec<bool>,
    /// Dimensions and leader notes in root coordinates with nothing selected, with the
    /// selection and hover shown, and which are selected: as the guides.
    base_annotations: Vec<AnnotationLook>,
    annotations: Vec<AnnotationLook>,
    annotation_flags: Vec<bool>,
    /// The section planes as the guides: in root coordinates with nothing selected, with the
    /// selection and hover shown, and which are selected.
    base_sections: Vec<SectionLook>,
    sections: Vec<SectionLook>,
    section_flags: Vec<bool>,
    targets: Option<Targets>,
}

/// How a pipeline uses the depth buffer.
struct DepthUse {
    write: bool,
    compare: wgpu::CompareFunction,
}

impl Renderer {
    /// A renderer whose lights cast shadows where the device allows (see `casts_shadows`).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::make(device, queue, format, true)
    }

    /// A renderer whose lights cast no shadows, as on a device without cube-map arrays.
    pub fn without_shadows(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self::make(device, queue, format, false)
    }

    fn make(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        shadows: bool,
    ) -> Self {
        let shadowed = shadows.then(|| shadowed_shader(device)).flatten();
        let (shader, max_shadows) = match shadowed {
            Some((shader, max)) => (shader, Some(max)),
            None => (scene_shader(device, false), None),
        };
        let shadow_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::CubeArray,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ];
        let scene_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let entries = if max_shadows.is_some() {
            [&scene_entries[..], &shadow_entries[..]].concat()
        } else {
            scene_entries.to_vec()
        };
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera and textures"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &LINE_ATTRIBS,
        };
        let face_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BaseFaceVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &FACE_ATTRIBS,
        };
        let flag_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<f32>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &FLAG_ATTRIBS,
        };
        let hover_layout = |attributes| wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<f32>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes,
        };
        let lines_only = [Some(line_layout.clone())];
        let flagged_faces = [
            Some(face_layout),
            Some(flag_layout.clone()),
            Some(hover_layout(&FACE_HOVER_ATTRIBS)),
        ];
        let flagged_lines = [
            Some(line_layout),
            Some(flag_layout),
            Some(hover_layout(&EDGE_HOVER_ATTRIBS)),
        ];
        let tested = DepthUse {
            write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
        };
        let pipeline = |label: &str,
                        vertex_entry: &str,
                        fragment_entry: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        depth: DepthUse,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex_entry),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth.write),
                    depth_compare: Some(depth.compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLE_COUNT,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines = wgpu::PrimitiveTopology::LineList;
        // Flat ground: no depth test at all, so the faces drawn after it always cover it.
        let flat_ground_pipeline = pipeline(
            "flat ground",
            "vs_flat_ground",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
        );
        let ground_pipeline = pipeline(
            "ground",
            "vs_ground",
            "fs_line",
            &lines_only,
            lines,
            tested,
            None,
        );
        // The axes' wide copies (see `vs_flat_ground_wide`), tested as their 1 px lines are.
        let wide_flat_axes_pipeline = pipeline(
            "wide flat axes",
            "vs_flat_ground_wide",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
        );
        let wide_vertical_axis_pipeline = pipeline(
            "wide vertical axis",
            "vs_ground_wide",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        let face_pipeline = pipeline(
            "faces",
            "vs_face",
            "fs_face",
            &flagged_faces,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::Greater,
            },
            None,
        );
        // The opaque sides of see-through triangles join the opaque faces; their see-through
        // sides are blended over everything opaque, tested against depth but not writing it.
        let solid_side_pipeline = pipeline(
            "solid sides",
            "vs_face",
            "fs_face_solid_side",
            &flagged_faces,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::Greater,
            },
            None,
        );
        let glass_pipeline = pipeline(
            "glass",
            "vs_face",
            "fs_glass",
            &flagged_faces,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Greater,
            },
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        let edge_pipeline = pipeline(
            "edges",
            "vs_edge",
            "fs_line",
            &flagged_lines,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        // Selected and hovered edges again, offset across and up (see `vs_edge_wide`).
        let wide_edge_pipeline = pipeline(
            "wide edges",
            "vs_edge_wide",
            "fs_line",
            &flagged_lines,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        // The day canvas's gradient, under everything.
        let canvas_pipeline = pipeline(
            "canvas",
            "vs_canvas",
            "fs_canvas",
            &[],
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
        );
        let guide_pipeline = pipeline(
            "guides",
            "vs_guide",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        // The fill of closed cuts, both sides: it lies where the faces end, so it is pulled
        // toward the camera (see `vs_cut_fill`) to win against faces lying in the plane.
        let cut_fill_pipeline = pipeline(
            "cut fill",
            "vs_cut_fill",
            "fs_line",
            &lines_only,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        let cut_edge_pipeline = pipeline(
            "cut edges",
            "vs_cut_edge",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        let wide_cut_edge_pipeline = pipeline(
            "wide cut edges",
            "vs_cut_edge_wide",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        let overlay_pipeline = pipeline(
            "overlay",
            "vs_overlay",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
        );
        // The preview solid: blended over the scene and hidden by what is in front of it,
        // without writing depth; back faces are culled so a single layer shows.
        let preview_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("preview solid"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_preview"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PreviewVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &PREVIEW_ATTRIBS,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: PREVIEW_DEPTH_BIAS,
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_preview"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lighting_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting uniforms"),
            size: std::mem::size_of::<LightingUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow maps"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let textures = TextureArray::new(device, queue, &Scene::default());
        let shadow_maps = max_shadows.map(|max| ShadowMaps::new(device, max));
        let bind_group = bind_group(
            device,
            &bind_layout,
            [&uniforms, &lighting_uniforms],
            &textures,
            &sampler,
            shadow_maps.as_ref().map(|m| (&m.cubes, &shadow_sampler)),
        );

        let [flat_ground, vertical_axis, flat_axes] = ground_meshes(device, &ViewStyle::CLASSIC);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            format,
            flat_ground_pipeline,
            ground_pipeline,
            wide_flat_axes_pipeline,
            wide_vertical_axis_pipeline,
            face_pipeline,
            solid_side_pipeline,
            glass_pipeline,
            edge_pipeline,
            wide_edge_pipeline,
            canvas_pipeline,
            guide_pipeline,
            cut_fill_pipeline,
            cut_edge_pipeline,
            wide_cut_edge_pipeline,
            overlay_pipeline,
            preview_pipeline,
            uniforms,
            lighting_uniforms,
            bind_layout,
            sampler,
            textures,
            shadow_sampler,
            bind_group,
            shadow_maps,
            caster_corners: Vec::new(),
            casters: None,
            caster_chunks: Vec::new(),
            caster_generation: 0,
            lighting: Lighting::Day,
            color_by_tag: false,
            section_display: SectionDisplay::default(),
            style: ViewStyle::CLASSIC,
            print: false,
            lights: Vec::new(),
            flat_ground,
            vertical_axis,
            flat_axes,
            faces: None,
            cut_fill: None,
            cut_edges: None,
            glass: None,
            edges: None,
            edges_selected: false,
            hover: None,
            hover_ranges: Ranges::default(),
            pixels_per_point: 1.0,
            preview_vertices: Vec::new(),
            preview: None,
            spans: Spans::default(),
            base_guides: Vec::new(),
            guides: Vec::new(),
            guide_flags: Vec::new(),
            base_annotations: Vec::new(),
            annotations: Vec::new(),
            annotation_flags: Vec::new(),
            base_sections: Vec::new(),
            sections: Vec::new(),
            section_flags: Vec::new(),
            targets: None,
        }
    }

    /// Rebuilds the face and edge buffers from the scene, with the open context `context` and
    /// `selection` shown (see `scene::scene_geometry`): `set_model` then `set_selection`.
    pub fn set_scene(
        &mut self,
        scene: &Scene,
        context: &[InstanceId],
        selection: &BTreeSet<EntityId>,
    ) {
        self.set_model(scene, context);
        self.set_selection(selection);
    }

    /// Rebuilds the face and edge buffers from the scene with the open context `context`, with
    /// nothing selected until `set_selection`. Call when the model or the open context changes.
    pub fn set_model(&mut self, scene: &Scene, context: &[InstanceId]) {
        if !self.textures.holds(scene) {
            self.textures = TextureArray::new(&self.device, &self.queue, scene);
            self.rebind();
        }
        let base = scene_base_shown(scene, context, self.color_by_tag, self.section_display);
        self.faces = flagged_mesh(&self.device, "faces", &base.faces);
        self.glass = Glass::new(&self.device, &base.faces, &base.glass());
        self.edges = flagged_mesh(&self.device, "edges", &base.edges);
        self.cut_fill = mesh(&self.device, "cut fill", &base.cut_fill);
        self.cut_edges = mesh(&self.device, "cut edges", &base.cut_edges);
        self.spans = base.spans;
        self.guides = base.guides.clone();
        self.guide_flags = vec![false; base.guides.len()];
        self.edges_selected = false;
        // The new flag buffers start at 0: nothing is hovered.
        self.hover = None;
        self.hover_ranges = Ranges::default();
        self.base_guides = base.guides;
        self.annotations = base.annotations.clone();
        self.annotation_flags = vec![false; base.annotations.len()];
        self.base_annotations = base.annotations;
        self.sections = base.sections.clone();
        self.section_flags = vec![false; base.sections.len()];
        self.base_sections = base.sections;
        self.lights = base.lights;
        // Shadow maps are drawn again only when the casters move, not when, say, a group is
        // opened or a color changes.
        if base.casters != self.caster_corners {
            self.caster_generation += 1;
            self.casters = (!base.casters.is_empty()).then(|| {
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("shadow casters"),
                        contents: bytemuck::cast_slice(&base.casters),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
            });
            self.caster_corners = base.casters;
        }
        self.caster_chunks = base.caster_chunks;
    }

    /// Makes the bind group anew, after the textures or the shadow maps were.
    fn rebind(&mut self) {
        self.bind_group = bind_group(
            &self.device,
            &self.bind_layout,
            [&self.uniforms, &self.lighting_uniforms],
            &self.textures,
            &self.sampler,
            self.shadow_maps
                .as_ref()
                .map(|m| (&m.cubes, &self.shadow_sampler)),
        );
    }

    /// Whether lights cast shadows on this device.
    pub fn casts_shadows(&self) -> bool {
        self.shadow_maps.is_some()
    }

    /// How many shadow-map cube faces have been drawn since the renderer was made: none while
    /// the shadowed lights and the casters stay as they were.
    pub fn shadow_faces_drawn(&self) -> u64 {
        self.shadow_maps.as_ref().map_or(0, |m| m.drawn)
    }

    /// Picks the lights casting shadows among `used` (the strongest at `eye`, as many as there
    /// are slots), records drawing the maps that are missing or stale, and returns each used
    /// light's slot.
    fn shadow_slots(
        &mut self,
        used: &[Light],
        eye: glam::DVec3,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Vec<Option<usize>> {
        let Some(maps) = &mut self.shadow_maps else {
            return vec![None; used.len()];
        };
        let keys: Vec<ShadowKey> = strongest(used, eye, maps.max)
            .iter()
            .map(ShadowKey::of)
            .collect();
        let grown = maps.grow(&self.device, keys.len());
        let assignment = maps.slots.assign(&keys, self.caster_generation);
        let redraw: Vec<(usize, ShadowKey)> = assignment
            .redraw
            .iter()
            .map(|&slot| {
                let k = assignment.slots.iter().position(|&s| s == slot);
                (slot, keys[k.expect("a redrawn slot is assigned")])
            })
            .collect();
        let casters = Casters {
            buffer: self.casters.as_ref(),
            chunks: &self.caster_chunks,
        };
        maps.draw(&self.queue, encoder, &redraw, &casters);
        if grown {
            self.rebind();
        }
        used.iter()
            .map(|l| {
                let key = ShadowKey::of(l);
                let k = keys.iter().position(|k| *k == key)?;
                Some(assignment.slots[k])
            })
            .collect()
    }

    /// Colors faces by tag (or by material) from the next `set_model` on. True when that
    /// changes how faces are colored, so the model must be set again.
    pub fn set_color_by_tag(&mut self, on: bool) -> bool {
        let changed = self.color_by_tag != on;
        self.color_by_tag = on;
        changed
    }

    /// Shows section planes and cuts as `display` says from the next `set_model` on. True when
    /// that changes what is drawn, so the model must be set again.
    pub fn set_section_display(&mut self, display: SectionDisplay) -> bool {
        let changed = self.section_display != display;
        self.section_display = display;
        changed
    }

    /// Lights the frames drawn from now on by day or by night.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = lighting;
    }

    /// Shows `selection`, ids of the open context given to the last `set_model`, by rewriting
    /// the selection flags only. Call when just the selection changes.
    pub fn set_selection(&mut self, selection: &BTreeSet<EntityId>) {
        let flags = self.spans.flags(selection);
        for (mesh, flags) in [(&self.faces, &flags.faces), (&self.edges, &flags.edges)] {
            if let Some(buffer) = mesh.as_ref().and_then(|m| m.flags.as_ref()) {
                self.queue
                    .write_buffer(buffer, 0, bytemuck::cast_slice(flags));
            }
        }
        self.edges_selected = flags.edges.iter().any(|&f| f > 0.5);
        self.guide_flags = flags.guides;
        self.annotation_flags = flags.annotations;
        self.section_flags = flags.sections;
        self.refresh_guides();
    }

    /// Shows `hover` (an id of the open context given to the last `set_model`) from the next
    /// frame; None shows nothing hovered. Only the old and new entity's flags are rewritten.
    pub fn set_hover(&mut self, hover: Option<(EntityId, HoverLook)>) {
        if hover == self.hover {
            return;
        }
        let ranges = hover
            .and_then(|(id, _)| self.spans.ranges(id))
            .unwrap_or_default();
        self.write_hover(&self.hover_ranges, 0.0);
        self.write_hover(&ranges, 1.0);
        self.hover = hover;
        self.hover_ranges = ranges;
        self.refresh_guides();
    }

    /// Sets the hover flags of `ranges` to `value`.
    fn write_hover(&self, ranges: &Ranges, value: f32) {
        for (mesh, range) in [(&self.faces, &ranges.faces), (&self.edges, &ranges.edges)] {
            if let Some(buffer) = mesh.as_ref().and_then(|m| m.hover.as_ref())
                && !range.is_empty()
            {
                let offset = (range.start * std::mem::size_of::<f32>()) as u64;
                let flags = vec![value; range.len()];
                self.queue
                    .write_buffer(buffer, offset, bytemuck::cast_slice(&flags));
            }
        }
    }

    /// The guides and annotations with the selection and the hover shown.
    fn refresh_guides(&mut self) {
        self.guides = shown_guides(
            &self.base_guides,
            &self.guide_flags,
            self.style.selected_edge,
        );
        self.annotations = shown_annotations(
            &self.base_annotations,
            &self.annotation_flags,
            self.style.selected_edge,
        );
        self.sections = shown_sections(
            &self.base_sections,
            &self.section_flags,
            self.style.selected_edge,
        );
        if let Some((_, look)) = self.hover {
            for s in &mut self.sections[self.hover_ranges.sections.clone()] {
                s.color = look.edge;
            }
            for g in &mut self.guides[self.hover_ranges.guides.clone()] {
                g.color = look.edge;
            }
            for a in &mut self.annotations[self.hover_ranges.annotations.clone()] {
                a.color = look.edge;
            }
        }
    }

    /// The colors the section planes are drawn in, in the order of `scene::SceneBase::sections`:
    /// orange when active, the guide colors otherwise, then the selection's, then the hover's.
    pub fn section_colors(&self) -> Vec<[f32; 4]> {
        self.sections.iter().map(|s| s.color).collect()
    }

    /// Draws `triangles` (three vertices each) from the next frame, blended over the scene and
    /// hidden by what is in front of them; empty draws none. Re-uploads only when it changed,
    /// into the same buffer while they fit.
    pub fn set_preview(&mut self, triangles: &[PreviewVertex]) {
        if triangles == self.preview_vertices.as_slice() {
            return;
        }
        self.preview_vertices.clear();
        self.preview_vertices.extend_from_slice(triangles);
        let bytes: &[u8] = bytemuck::cast_slice(triangles);
        let fits = self
            .preview
            .as_ref()
            .is_some_and(|m| m.buffer.size() >= bytes.len() as u64);
        if !fits {
            let size = (bytes.len() as u64).next_power_of_two();
            self.preview = Some(Mesh {
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("preview solid"),
                    size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                count: 0,
                flags: None,
                hover: None,
            });
        }
        if let Some(preview) = &mut self.preview {
            if !bytes.is_empty() {
                self.queue.write_buffer(&preview.buffer, 0, bytes);
            }
            preview.count = triangles.len() as u32;
        }
    }

    /// Draws the frames from now on for a display of `scale` physical pixels per point, which
    /// widths given in points are multiplied by. render_view and headless keep 1.
    pub fn set_pixels_per_point(&mut self, scale: f32) {
        self.pixels_per_point = scale;
    }

    /// Draws the frames from now on as printed pages (or, off, as views): cleared to white, and
    /// without the canvas gradient, the ground grid and the axes. Shading and lighting are the
    /// view's.
    pub fn set_print_mode(&mut self, on: bool) {
        self.print = on;
    }

    /// Draws the frames from now on in `style`: the window sets its theme's; render_view and
    /// headless keep `ViewStyle::CLASSIC`. Recolors selected guides without a rebuild; the grid
    /// and axes are rebuilt only when their colors change.
    pub fn set_style(&mut self, style: ViewStyle) {
        if style == self.style {
            return;
        }
        if style.axes != self.style.axes || style.day_background() != self.style.day_background() {
            [self.flat_ground, self.vertical_axis, self.flat_axes] =
                ground_meshes(&self.device, &style);
        }
        self.style = style;
        self.refresh_guides();
    }

    /// Draws grid, axes, model and `overlay` (line pairs drawn on top of everything) over the
    /// whole target.
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        self.render_in(target, size, Viewport::full(size), camera, overlay);
    }

    /// Like `render`, but the scene fills only `viewport`; the rest of the target is cleared to
    /// the background (dark at night). `camera.viewport` should match the viewport's size.
    pub fn render_in(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        viewport: Viewport,
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(self.make_targets(size));
        }

        let uniforms = Uniforms::new(camera);
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let used = strongest(&self.lights, camera.eye, MAX_LIGHTS);
        let shadow_slots = self.shadow_slots(&used, camera.eye, &mut encoder);
        let mut lighting = LightingUniforms::new(
            &used,
            &shadow_slots,
            self.lighting,
            &self.style,
            (viewport.width, viewport.height),
        );
        if let Some((_, look)) = self.hover {
            let [r, g, b] = look.face;
            lighting.hover_face = [r, g, b, look.face_mix];
            lighting.hover_edge = look.edge;
        }
        let (wide_copies, wide_step) = spread(WIDE_EDGE_PT * self.pixels_per_point);
        let (axis_copies, axis_step) = spread(WIDE_AXIS_PT * self.pixels_per_point);
        lighting.wide[0] = wide_step;
        lighting.wide[1] = axis_step;
        // Cut edges are twice as wide as the selected edges would be.
        let (cut_copies, cut_step) = spread(CUT_EDGE_PT * self.pixels_per_point);
        lighting.wide[2] = cut_step;
        self.queue
            .write_buffer(&self.lighting_uniforms, 0, bytemuck::bytes_of(&lighting));
        let targets = self.targets.as_ref().expect("targets just created");
        let overlay = mesh(&self.device, "overlay", overlay);
        // Guides and annotations: both drawn anew for the camera, in the same pipeline.
        let mut guide_vertices = guide_lines(&self.guides, camera);
        section_lines(
            &self.sections,
            camera,
            f64::from(self.pixels_per_point),
            &mut guide_vertices,
        );
        annotation_lines(
            &self.annotations,
            camera,
            f64::from(self.pixels_per_point),
            &mut guide_vertices,
        );
        let guides = mesh(&self.device, "guides", &guide_vertices);
        if let Some(glass) = &mut self.glass {
            glass.sort(&self.queue, camera);
        }

        let [r, g, b, a] = if self.print {
            [1.0; 4]
        } else {
            background(self.lighting, &self.style).map(f64::from)
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(target),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            // Clamp the viewport to the target; an empty one leaves just the cleared background.
            let (x, y) = (viewport.x.min(size.0), viewport.y.min(size.1));
            let (w, h) = (
                viewport.width.min(size.0 - x),
                viewport.height.min(size.1 - y),
            );
            if w > 0 && h > 0 {
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(0, &self.bind_group, &[]);
                if !self.print && self.lighting == Lighting::Day && self.style.has_gradient() {
                    pass.set_pipeline(&self.canvas_pipeline);
                    pass.draw(0..3, 0..1);
                }
                let bind = |pass: &mut wgpu::RenderPass, pipeline, mesh: &Mesh| {
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    if let Some(flags) = &mesh.flags {
                        pass.set_vertex_buffer(1, flags.slice(..));
                    }
                    if let Some(hover) = &mesh.hover {
                        pass.set_vertex_buffer(2, hover.slice(..));
                    }
                };
                let draw = |pass: &mut wgpu::RenderPass, pipeline, mesh: Option<&Mesh>| {
                    if let Some(mesh) = mesh {
                        bind(pass, pipeline, mesh);
                        pass.draw(0..mesh.count, 0..1);
                    }
                };
                if !self.print {
                    draw(
                        &mut pass,
                        &self.flat_ground_pipeline,
                        Some(&self.flat_ground),
                    );
                    if self.style.wide_axes {
                        bind(&mut pass, &self.wide_flat_axes_pipeline, &self.flat_axes);
                        pass.draw(0..self.flat_axes.count, 0..axis_copies);
                    }
                    draw(&mut pass, &self.ground_pipeline, Some(&self.vertical_axis));
                    if self.style.wide_axes {
                        bind(
                            &mut pass,
                            &self.wide_vertical_axis_pipeline,
                            &self.vertical_axis,
                        );
                        pass.draw(0..self.vertical_axis.count, 0..axis_copies);
                    }
                }
                match (&self.faces, &self.glass) {
                    (Some(faces), Some(glass)) => {
                        if glass.solid_count > 0 {
                            bind(&mut pass, &self.face_pipeline, faces);
                            pass.set_index_buffer(glass.solid.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..glass.solid_count, 0, 0..1);
                        }
                        bind(&mut pass, &self.solid_side_pipeline, faces);
                        pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..glass.count(), 0, 0..1);
                    }
                    (faces, _) => draw(&mut pass, &self.face_pipeline, faces.as_ref()),
                }
                draw(&mut pass, &self.edge_pipeline, self.edges.as_ref());
                let edges_hovered = !self.hover_ranges.edges.is_empty();
                if self.style.wide_selected_edges
                    && (self.edges_selected || edges_hovered)
                    && let Some(edges) = &self.edges
                {
                    bind(&mut pass, &self.wide_edge_pipeline, edges);
                    pass.draw(0..edges.count, 0..wide_copies);
                }
                draw(&mut pass, &self.cut_fill_pipeline, self.cut_fill.as_ref());
                if let Some(cuts) = &self.cut_edges {
                    draw(&mut pass, &self.cut_edge_pipeline, Some(cuts));
                    bind(&mut pass, &self.wide_cut_edge_pipeline, cuts);
                    pass.draw(0..cuts.count, 0..cut_copies);
                }
                draw(&mut pass, &self.guide_pipeline, guides.as_ref());
                // See-through faces last but for the overlay: everything opaque shows through.
                if let (Some(faces), Some(glass)) = (&self.faces, &self.glass) {
                    bind(&mut pass, &self.glass_pipeline, faces);
                    pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..glass.count(), 0, 0..1);
                }
                if let Some(preview) = self.preview.as_ref().filter(|m| m.count > 0) {
                    draw(&mut pass, &self.preview_pipeline, Some(preview));
                }
                draw(&mut pass, &self.overlay_pipeline, overlay.as_ref());
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    fn make_targets(&self, size: (u32, u32)) -> Targets {
        let texture = |label, format| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: SAMPLE_COUNT,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Targets {
            size,
            msaa: texture("msaa color", self.format),
            depth: texture("depth", DEPTH_FORMAT),
        }
    }
}

/// The bind group every pipeline uses: the camera uniforms, the material textures and their
/// sampler, the lighting uniforms and, when there are any, the shadow maps and their sampler.
fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    [uniforms, lighting]: [&wgpu::Buffer; 2],
    textures: &TextureArray,
    sampler: &wgpu::Sampler,
    shadows: Option<(&wgpu::TextureView, &wgpu::Sampler)>,
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(&textures.view),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(sampler),
        },
        wgpu::BindGroupEntry {
            binding: 3,
            resource: lighting.as_entire_binding(),
        },
    ];
    if let Some((maps, compare)) = shadows {
        entries.push(wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::TextureView(maps),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: wgpu::BindingResource::Sampler(compare),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("camera and textures"),
        layout,
        entries: &entries,
    })
}

/// The flat ground (grid and X and Y axes), the Z axis and the X and Y axes alone, in
/// `style`'s axis colors, their negative halves faded toward its day background.
fn ground_meshes(device: &wgpu::Device, style: &ViewStyle) -> [Mesh; 3] {
    let (colors, toward) = (style.axes, style.day_background());
    [
        ("flat ground lines", flat_ground_lines_in(colors, toward)),
        ("vertical axis", vertical_axis_lines_in(colors, toward)),
        ("flat axes", flat_axis_lines_in(colors, toward)),
    ]
    .map(|(label, lines)| mesh(device, label, &lines).expect("the axes are never empty"))
}

/// Uploads vertices, or None when there are none to draw.
fn mesh<T: bytemuck::Pod>(device: &wgpu::Device, label: &str, vertices: &[T]) -> Option<Mesh> {
    if vertices.is_empty() {
        return None;
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some(Mesh {
        buffer,
        count: vertices.len() as u32,
        flags: None,
        hover: None,
    })
}

/// Uploads vertices with selection and hover flag buffers beside them, every flag 0 (wgpu
/// zeroes new buffers), or None when there are none to draw.
fn flagged_mesh<T: bytemuck::Pod>(
    device: &wgpu::Device,
    label: &str,
    vertices: &[T],
) -> Option<Mesh> {
    let mut mesh = mesh(device, label, vertices)?;
    let flags = || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (vertices.len() * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    mesh.flags = Some(flags());
    mesh.hover = Some(flags());
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{back_to_front, spread};

    #[test]
    fn wide_lines_spread_into_one_pixel_copies() {
        // 2 pt edges and 1.5 pt axes at 1x and at 2x.
        assert_eq!(spread(2.0), (2, 1.0));
        assert_eq!(spread(1.5), (2, 0.5));
        assert_eq!(spread(4.0), (6, 1.0));
        assert_eq!(spread(3.0), (4, 1.0));
        // A line no wider than a pixel needs no copies.
        assert_eq!(spread(1.0), (0, 0.0));
        assert_eq!(spread(0.75), (0, 0.0));
    }

    #[test]
    fn glass_triangles_are_ordered_far_to_near() {
        let triangles = [
            (0, Vec3::new(0.0, 1.0, 0.0)),
            (3, Vec3::new(0.0, 5.0, 0.0)),
            (6, Vec3::new(0.0, 3.0, 0.0)),
        ];
        // An eye at the origin: the triangle 5 m away first.
        assert_eq!(
            back_to_front(&triangles, [0.0, 0.0, 0.0, 1.0]),
            [3, 4, 5, 6, 7, 8, 0, 1, 2]
        );
        // An eye beyond them looks the other way round.
        assert_eq!(
            back_to_front(&triangles, [0.0, 10.0, 0.0, 1.0]),
            [0, 1, 2, 6, 7, 8, 3, 4, 5]
        );
        // A parallel view looking along -y.
        assert_eq!(
            back_to_front(&triangles, [0.0, -1.0, 0.0, 0.0]),
            [0, 1, 2, 6, 7, 8, 3, 4, 5]
        );
    }
}
