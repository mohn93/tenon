use std::collections::BTreeSet;

use std::borrow::Cow;

use glam::{DAffine3, DVec3, Mat4, Vec3};
use kernel::{EntityId, FogColor, InstanceId, Scene, StyleSettings};
use session::{Camera, Lighting, SectionDisplay, world_drawing_axes};
use wgpu::util::DeviceExt;

use crate::annotations::{AnnotationLook, annotation_lines};
use crate::chunks::{CHUNK_PRIMITIVES, ChunkPlan, Runs};
use crate::grid::{
    LineVertex, NIGHT_BACKGROUND, flat_axis_lines_at, flat_ground_lines_at, vertical_axis_lines_at,
};
use crate::guides::{GuideShape, guide_lines};
use crate::lights::{Light, MAX_LIGHTS, strongest};
use crate::lod::{CELL_PIXELS, Lod, LodVertex};
use crate::scene::{
    BaseFaceVertex, CasterChunk, EdgeColors, EdgeExtra, GlassView, PartsCache, Ranges, Spans,
    scene_base_cached, shown_annotations, shown_guides, shown_sections,
};
use crate::sections::{SectionLook, section_lines};
use crate::shadow_maps::{Casters, ShadowMaps};
use crate::shadows::{MAX_SHADOWS, ShadowKey};
use crate::ssao::{self, Ssao};
use crate::style::ViewStyle;
use crate::sun_shadow::{SUN_MAP_SIZE, SunFit, SunLight, SunMap, depth_texture, fit};
use crate::texture::TextureArray;

pub const SAMPLE_COUNT: u32 = 4;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const LINE_ATTRIBS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
const FACE_ATTRIBS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 3 => Float32x3, 4 => Float32x3, 5 => Float32x2,
    6 => Float32x4, 7 => Float32x2, 8 => Float32x2, 10 => Float32x4
];
/// A corner of a simplified chunk (see `lod::LodVertex`): position, normal, the colors of the two
/// sides and their finishes, from the one vertex buffer such a chunk is drawn with.
const LOD_ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Snorm8x4, 3 => Unorm8x4, 4 => Unorm8x4, 10 => Unorm8x4
];
/// The selection flag of a face or edge vertex, from a second vertex buffer (see
/// `scene::Flags`).
const FLAG_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![2 => Float32];
const PREVIEW_ATTRIBS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
/// The hover flag of a face or edge vertex, from a third vertex buffer (1 hovered, 0 not).
const FACE_HOVER_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![9 => Float32];
const EDGE_HOVER_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![3 => Float32];
/// What an edge vertex carries for the edge effects, from a fourth vertex buffer (see
/// `scene::EdgeExtra`).
const EDGE_EXTRA_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    4 => Float32x3, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
];

/// The limits the renderer asks of a device: the defaults, with room for a cube shadow map per
/// shadowed light (six array layers each) where the adapter has it, and buffers as large as the
/// adapter allows, since the whole flattened model is one vertex buffer (the default's 256 MB
/// holds under a million triangles).
///
/// On the web, a GL (WebGL2) adapter gets `downlevel_webgl2_defaults` as the base instead, since
/// the full defaults exceed what WebGL2 offers.
pub fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    #[cfg(target_arch = "wasm32")]
    let defaults = if adapter.get_info().backend == wgpu::Backend::Gl {
        // The adapter's own texture size, so canvases past 2048 px work.
        wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
    } else {
        // The adapter's own texture size, so the largest canvases work.
        wgpu::Limits::default().using_resolution(adapter.limits())
    };
    #[cfg(not(target_arch = "wasm32"))]
    let defaults = wgpu::Limits::default();
    let wanted = (6 * MAX_SHADOWS) as u32;
    let layers = wanted.min(adapter.limits().max_texture_array_layers);
    wgpu::Limits {
        max_texture_array_layers: layers.max(defaults.max_texture_array_layers),
        max_buffer_size: adapter.limits().max_buffer_size,
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

/// True when the adapter can draw ambient occlusion (the style's `ambient_occlusion`). The
/// passes need depth and normal buffers at half resolution and a depth texture read by the
/// shader; a browser's WebGL2 (GL) adapter is not trusted with them, so there a renderer built
/// for one is told with [`Renderer::disable_ambient_occlusion`].
pub fn supports_ambient_occlusion(adapter: &wgpu::Adapter) -> bool {
    cfg!(not(target_arch = "wasm32")) || adapter.get_info().backend != wgpu::Backend::Gl
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
/// The studio's ambient light, and what its key light adds to a face square-on to it: together
/// 1, so a face square-on to the key is lit as fully as one square-on to the day headlight.
const STUDIO_AMBIENT: f32 = 0.25;
const STUDIO_KEY: f32 = 0.75;
/// The studio's fill and rim lights' strengths.
const STUDIO_FILL: f32 = 0.35;
const STUDIO_RIM: f32 = 0.5;
/// The studio lights' directions toward them in camera space: x right, y up, z toward the
/// viewer (not yet normalized).
const STUDIO_KEY_DIRECTION: [f32; 3] = [-0.5, 0.6, 0.6];
const STUDIO_FILL_DIRECTION: [f32; 3] = [0.6, 0.1, 0.5];
const STUDIO_RIM_DIRECTION: [f32; 3] = [0.0, 0.5, -0.8];

/// The unit vector toward the studio light at `direction` in camera space, in the world.
fn studio_direction(camera: &Camera, direction: [f32; 3]) -> Vec3 {
    let [x, y, z] = direction;
    let (right, up, forward) = (
        camera.right().as_vec3(),
        camera.up().as_vec3(),
        camera.forward().as_vec3(),
    );
    (right * x + up * y - forward * z).normalize_or_zero()
}

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
    /// axis (y), of a section cut edge (z, see `vs_cut_edge_wide`) and of a profile (w, see
    /// `vs_edge_profile_wide`).
    wide: [f32; 4],
    /// x: 1 when profiles are on; y: 1 when edges that are not profiles are shown; z: the
    /// sketchy jitter in pixels; w: the sketchy extension in pixels.
    edge_fx: [f32; 4],
    /// x: the length in pixels of an endpoint mark; y: the step between the copies of a mark,
    /// z: how many steps they span; w: how many steps the copies of a profile span (their step
    /// is `wide.w`).
    edge_fx2: [f32; 4],
    /// xyz: the unit vector toward the sun; w: 0 without sun shading, 1 with it, 2 with its
    /// shadow map too.
    sun: [f32; 4],
    /// x: the direct light a face square-on to the sun adds; y: the light a face in shadow
    /// keeps; z: the world size of a map texel; w: the world depth the map spans.
    sun_levels: [f32; 4],
    /// xyz: the camera's position; w: 1 in a perspective view, 0 in a parallel one.
    cam_pos: [f32; 4],
    /// xyz: the unit direction the camera looks along.
    cam_forward: [f32; 4],
    /// xyz: the offsets of the screen's right and up edges from the view direction (see
    /// `view_ray` in scene.wgsl).
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    /// x: 1 when the canvas is drawn from the model's style; y: 1 when the eye is below the
    /// ground; z: 1 when the ground is drawn seen from below.
    canvas: [f32; 4],
    /// rgb: the sky's color; w: 1 when the style has a sky.
    sky: [f32; 4],
    /// rgb: the ground's color; w: its opacity, 0 without a ground.
    ground: [f32; 4],
    /// x: where the fog starts; y: where it is complete; z: 1 when there is fog.
    fog: [f32; 4],
    fog_color: [f32; 4],
    /// x: 1 in studio lighting, else 0.
    studio: [f32; 4],
    /// xyz: the unit vector toward the studio fill light and the rim light; w: its strength
    /// (0 outside the studio).
    studio_fill: [f32; 4],
    studio_rim: [f32; 4],
    /// x: 1 when the ambient occlusion buffer holds this frame's occlusion.
    ao: [f32; 4],
    /// World to the sun map's clip space.
    sun_view_proj: [f32; 16],
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
        model: Option<&StyleSettings>,
        view: (u32, u32),
    ) -> Self {
        let mut uniforms = Self {
            mode: [0.0; 4],
            background: background(mode, style, model),
            selected_edge: [0.0; 4],
            selected_face: [0.0; 4],
            lines: [0.0; 4],
            canvas_top: [0.0; 4],
            canvas_bottom: [0.0; 4],
            hover_face: [0.0; 4],
            hover_edge: [0.0; 4],
            wide: [0.0; 4],
            edge_fx: [0.0, 1.0, 0.0, 0.0],
            edge_fx2: [0.0; 4],
            sun: [0.0; 4],
            sun_levels: [0.0; 4],
            cam_pos: [0.0; 4],
            cam_forward: [0.0; 4],
            cam_right: [0.0; 4],
            cam_up: [0.0; 4],
            canvas: [0.0; 4],
            sky: [0.0; 4],
            ground: [0.0; 4],
            fog: [0.0; 4],
            fog_color: [0.0; 4],
            studio: [0.0; 4],
            studio_fill: [0.0; 4],
            studio_rim: [0.0; 4],
            ao: [0.0; 4],
            sun_view_proj: Mat4::IDENTITY.to_cols_array(),
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
            // No headlight: the key light shades like the sun (see `sun_frame`).
            Lighting::Studio => ([STUDIO_AMBIENT, 0.0], 0.0),
        };
        uniforms.studio[0] = if mode == Lighting::Studio { 1.0 } else { 0.0 };
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
        if let Some(model) = model {
            uniforms.set_model_style(model, mode);
        }
        uniforms
    }

    /// The model style's sky, ground and fog. The studio has no ground. By night the style's background, sky and ground
    /// are ignored (the night background and no canvas of the style's), but fog still falls,
    /// toward the night background or its own color.
    fn set_model_style(&mut self, model: &StyleSettings, mode: Lighting) {
        if mode != Lighting::Night {
            self.canvas[0] = 1.0;
            if let Some(sky) = model.sky {
                let [r, g, b, _] = rgb(sky);
                self.sky = [r, g, b, 1.0];
            }
        }
        if mode == Lighting::Day
            && let Some(ground) = &model.ground
        {
            let [r, g, b, _] = rgb(ground.color);
            self.ground = [r, g, b, ground.opacity];
            self.canvas[2] = if ground.from_below { 1.0 } else { 0.0 };
        }
        if let Some(fog) = &model.fog {
            let color = match fog.color {
                FogColor::Background => self.background,
                FogColor::Rgb(c) => rgb(c),
            };
            self.fog = [fog.start, fog.end, 1.0, 0.0];
            self.fog_color = color;
        }
    }

    /// The camera's place and view rays, for the fog's distances and the canvas's sky and
    /// ground.
    fn set_camera(&mut self, camera: &Camera) {
        let (eye, forward) = (camera.eye.as_vec3(), camera.forward().as_vec3());
        let half = (camera.fov_y * 0.5).tan();
        let right = camera.right() * half * camera.aspect();
        let up = camera.up() * half;
        let w = if camera.is_parallel() { 0.0 } else { 1.0 };
        self.cam_pos = [eye.x, eye.y, eye.z, w];
        // A parallel view: the target's height over its distance, which sets where the ground
        // plane's row lies; the eye's own height is arbitrary there.
        let target_height = if camera.is_parallel() {
            (camera.target.z / camera.distance()) as f32
        } else {
            0.0
        };
        self.cam_forward = [forward.x, forward.y, forward.z, target_height];
        let (right, up) = (right.as_vec3(), up.as_vec3());
        self.cam_right = [right.x, right.y, right.z, 0.0];
        self.cam_up = [up.x, up.y, up.z, 0.0];
        if self.studio[0] > 0.5 {
            let dir = |d, strength| {
                let v = studio_direction(camera, d);
                [v.x, v.y, v.z, strength]
            };
            self.studio_fill = dir(STUDIO_FILL_DIRECTION, STUDIO_FILL);
            self.studio_rim = dir(STUDIO_RIM_DIRECTION, STUDIO_RIM);
        }
        self.canvas[1] = if !camera.is_parallel() && camera.eye.z < 0.0 {
            1.0
        } else {
            0.0
        };
    }
}

/// An RGB color in bytes as the shaders read it: 0 to 1 with full alpha.
fn rgb([r, g, b]: [u8; 3]) -> [f32; 4] {
    [r, g, b, 255u8].map(|c| f32::from(c) / 255.0)
}

/// The sun lighting a frame: the sun, and the map fitted to the casters when it has shadows.
struct SunFrame {
    light: SunLight,
    fit: Option<SunFit>,
    /// The studio's key light, which shades from any side (the sun below the horizon only
    /// leaves its dark level).
    key: bool,
}

impl LightingUniforms {
    /// Shades by the sun (see `sun_shade` in scene.wgsl). Below the horizon only the dark level
    /// shades.
    fn set_sun(&mut self, sun: &SunFrame) {
        let SunLight {
            direction,
            light,
            dark,
            ..
        } = sun.light;
        let mode = if sun.fit.is_some() { 2.0 } else { 1.0 };
        self.sun = [direction.x, direction.y, direction.z, mode];
        let direct = if sun.key || sun.light.is_up() {
            light
        } else {
            0.0
        };
        let (texel, range, view_proj) = sun.fit.map_or((0.0, 1.0, Mat4::IDENTITY), |f| {
            (f.texel, f.depth_range, f.view_proj)
        });
        self.sun_levels = [direct, dark, texel, range];
        self.sun_view_proj = view_proj.to_cols_array();
    }
}

/// The color the target is cleared to: by day the model style's background if it has one, else
/// the theme's; by night the night background.
fn background(mode: Lighting, style: &ViewStyle, model: Option<&StyleSettings>) -> [f32; 4] {
    match mode {
        Lighting::Day | Lighting::Studio => {
            model.map_or_else(|| style.day_background(), |m| rgb(m.background))
        }
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
/// The marks at the ends of edges, in points, where the style has endpoints.
const ENDPOINT_WIDTH_PT: f32 = 3.0;
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
    /// For model edges, what the edge effects need per vertex; None when no effect is on.
    extras: Option<wgpu::Buffer>,
    /// For model edges with effects: the vertices (in pairs) the profile copies and the
    /// endpoint marks can draw, when fewer than all of them (see `edge_candidates`).
    profile_edges: Option<Indices>,
    endpoint_edges: Option<Indices>,
}

/// An index buffer of vertices to draw and how many it holds.
struct Indices {
    buffer: wgpu::Buffer,
    count: u32,
    /// For chunked edges: where each edge chunk's indices start in the list (one more entry
    /// than there are chunks, the last the list's length). Empty when not chunked.
    chunk_starts: Vec<u32>,
}

/// Triangles (or edges) a model needs before its faces (or edges) are drawn by spatial chunk,
/// leaving out those out of view. Smaller models are drawn whole, as one call.
const CULL_FROM: usize = 20_000;

/// How long after `set_model` the simplified copies are made: while the model is being changed
/// again and again (a transition moving instances), they would be thrown away.
const LOD_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// The simplified copies of a model's solid chunks: they are made from the vertices, which
/// are kept until the model has settled, then built on another thread, so a frame never waits.
enum LodState {
    Off,
    /// The model's vertices, waiting for the model to settle.
    Pending(std::sync::Arc<Vec<BaseFaceVertex>>),
    Building {
        done: std::sync::mpsc::Receiver<Option<Lod>>,
        /// Set to stop a build whose model has been replaced.
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    },
    Ready(Lod),
}

/// What a frame draws of the chunked vertex lists: the vertex ranges in view.
#[derive(Default)]
struct Visible {
    faces: Option<Runs>,
    edges: Option<Runs>,
}

/// Draws `copies` instances of the vertex ranges `runs`.
fn draw_runs(pass: &mut wgpu::RenderPass<'_>, runs: &[std::ops::Range<u32>], copies: u32) {
    for r in runs {
        pass.draw(r.clone(), 0..copies);
    }
}

/// How long the camera must have been still before a held glass order is made exact. The app
/// draws a frame this long after one that drew a held order.
pub const GLASS_SETTLE: std::time::Duration = std::time::Duration::from_millis(80);

/// What every glass triangle was last sorted for: the view, and the distance from the eye to
/// the nearest glass triangle then (infinite for a parallel view).
type SortedFor = ([f32; 4], f32);

/// How far the eye may move, as a fraction of the distance to the nearest glass triangle, before
/// the order is worked out again. Meanwhile the eye has moved by up to this much, so triangles
/// whose distances from it differ by less than twice that can be in the wrong order, which
/// while the camera moves is not seen; once it stops, the order is made exact (see
/// `Glass::sort`).
const RESORT_MOVE: f32 = 0.01;

/// Whether the order sorted for `prev` still holds for `view`.
fn order_holds(prev: &SortedFor, view: [f32; 4]) -> bool {
    let (old, nearest) = prev;
    if *old == view {
        return true;
    }
    // A parallel view has no eye to move: any change of direction sorts again.
    if view[3] == 0.0 || old[3] == 0.0 {
        return false;
    }
    let moved = glam::Vec3::new(view[0] - old[0], view[1] - old[1], view[2] - old[2]).length();
    moved <= RESORT_MOVE * nearest
}

/// The triangles of `faces` with a see-through side, drawn after everything opaque.
struct Glass {
    /// The first vertex of each opaque triangle, for drawing them alone.
    solid: wgpu::Buffer,
    solid_count: u32,
    /// The vertices of the see-through triangles, far to near as last sorted.
    order: wgpu::Buffer,
    /// Each see-through triangle's first vertex and center, by first vertex.
    triangles: Vec<(u32, glam::Vec3)>,
    /// How many vertices of `order` are drawn: those of the triangles in view when last sorted.
    drawn: u32,
    /// What `sorted` was sorted for.
    sorted_for: Option<SortedFor>,
    /// Every triangle (an index into `triangles`) far to near, as last sorted.
    sorted: Vec<u32>,
    /// The vertex ranges in view (None: all) `order` was last filled for.
    drawn_for: Option<Option<Vec<std::ops::Range<u32>>>>,
    /// The view of the last frame, and since when it has not changed, to tell a camera that
    /// has stopped.
    last_view: Option<[f32; 4]>,
    still_since: Option<web_time::Instant>,
    /// Whether `sorted` is a held order, not exact for the last frame's view.
    held: bool,
}

impl Glass {
    /// The see-through triangles of `faces`, or None when there are none.
    /// `solid_list` is false when the opaque triangles are listed elsewhere (in chunks).
    fn new(
        device: &wgpu::Device,
        vertex_count: usize,
        pos: &dyn Fn(usize) -> [f32; 3],
        glass: &[u32],
        solid_list: bool,
    ) -> Option<Glass> {
        if glass.is_empty() {
            return None;
        }
        let mut solid = Vec::new();
        if solid_list {
            solid.reserve(vertex_count - 3 * glass.len());
            let mut next = glass.iter().peekable();
            for first in (0..vertex_count as u32).step_by(3) {
                if next.next_if_eq(&&first).is_none() {
                    solid.extend([first, first + 1, first + 2]);
                }
            }
        }
        let triangles: Vec<(u32, glam::Vec3)> = glass
            .iter()
            .map(|&k| {
                let corner = |i: u32| glam::Vec3::from(pos(i as usize));
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
            drawn: 3 * triangles.len() as u32,
            triangles,
            sorted_for: None,
            sorted: Vec::new(),
            drawn_for: None,
            last_view: None,
            still_since: None,
            held: false,
        })
    }

    fn count(&self) -> u32 {
        self.drawn
    }

    /// True while the order drawn last is a held one, not exact for the view: a frame with
    /// the camera unchanged sorts it exactly.
    fn needs_settle(&self) -> bool {
        self.held
    }

    /// Sorts the triangles far to near from `camera`, unless they already are for its view.
    /// `visible` are the vertex ranges in view when the faces are culled by chunk: only the
    /// triangles in them are sorted and drawn.
    fn sort(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        visible: Option<&[std::ops::Range<u32>]>,
    ) {
        self.sort_at(queue, camera, visible, web_time::Instant::now());
    }

    /// `sort` at the time `now`.
    fn sort_at(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        visible: Option<&[std::ops::Range<u32>]>,
        now: web_time::Instant,
    ) {
        let (origin, toward) = (camera.eye.as_vec3(), camera.forward().as_vec3());
        let view = if camera.is_parallel() {
            [toward.x, toward.y, toward.z, 0.0]
        } else {
            [origin.x, origin.y, origin.z, 1.0]
        };
        // A held order is only for a moving eye: once the view has been still for
        // `GLASS_SETTLE`, it is sorted exactly if it is not already. (A frame or two with the
        // camera unchanged in the middle of a stuttering orbit keeps the hold.)
        if self.last_view != Some(view) || self.still_since.is_none() {
            self.still_since = Some(now);
        }
        let stopped = self
            .still_since
            .is_some_and(|t| now.saturating_duration_since(t) >= GLASS_SETTLE);
        self.last_view = Some(view);
        let exact = self.sorted_for.as_ref().is_some_and(|prev| prev.0 == view);
        if !self
            .sorted_for
            .as_ref()
            .is_some_and(|prev| order_holds(prev, view))
            || (stopped && !exact)
        {
            // Every triangle is sorted, in view or not, so that the set in view changing as the
            // camera turns costs a filter, not a sort.
            let nearest = if view[3] == 0.0 {
                f32::INFINITY
            } else {
                let eye = glam::Vec3::new(view[0], view[1], view[2]);
                let near = self
                    .triangles
                    .iter()
                    .fold(f32::INFINITY, |m, t| m.min(t.1.distance_squared(eye)));
                near.sqrt()
            };
            self.sorted = far_to_near(&self.triangles, view);
            self.sorted_for = Some((view, nearest));
            self.drawn_for = None;
        }
        // (A view that is not finite is never exact and never moves on: nothing to settle.)
        self.held = view.iter().all(|v| v.is_finite())
            && self.sorted_for.as_ref().is_none_or(|prev| prev.0 != view);
        let wanted = visible.map(<[_]>::to_vec);
        if self.drawn_for.as_ref() == Some(&wanted) {
            return;
        }
        let mut indices = Vec::with_capacity(3 * self.sorted.len());
        let mut push = |t: u32| {
            let first = self.triangles[t as usize].0;
            indices.extend([first, first + 1, first + 2]);
        };
        match visible {
            None => self.sorted.iter().for_each(|&t| push(t)),
            Some(runs) => {
                let mut shown = vec![false; self.triangles.len()];
                for r in runs {
                    let from = self.triangles.partition_point(|t| t.0 < r.start);
                    let to = self.triangles.partition_point(|t| t.0 < r.end);
                    shown[from..to].fill(true);
                }
                for &t in &self.sorted {
                    if shown[t as usize] {
                        push(t);
                    }
                }
            }
        }
        if !indices.is_empty() {
            queue.write_buffer(&self.order, 0, bytemuck::cast_slice(&indices));
        }
        self.drawn = indices.len() as u32;
        self.drawn_for = Some(wanted);
    }
}

/// The vertices of `triangles` (first vertex, center) ordered far to near for `view`: the
/// eye's position (w = 1), or the view direction of a parallel view (w = 0).
#[cfg(test)]
fn back_to_front(triangles: &[(u32, glam::Vec3)], view: [f32; 4]) -> Vec<u32> {
    far_to_near(triangles, view)
        .iter()
        .flat_map(|&t| {
            let first = triangles[t as usize].0;
            [first, first + 1, first + 2]
        })
        .collect()
}

/// The indices of `triangles` ordered far to near for `view` (see `back_to_front`).
fn far_to_near(triangles: &[(u32, glam::Vec3)], view: [f32; 4]) -> Vec<u32> {
    let [x, y, z, w] = view;
    let v = glam::Vec3::new(x, y, z);
    let depth = |c: glam::Vec3| {
        if w == 0.0 {
            c.dot(v)
        } else {
            c.distance_squared(v)
        }
    };
    // Each triangle's depth as an integer that orders like the float, with its index, then
    // a least-significant-digit radix sort: linear, where comparing sorts are not.
    let mut keyed: Vec<(u32, u32)> = triangles
        .iter()
        .enumerate()
        .map(|(i, &(_, c))| {
            let bits = depth(c).to_bits();
            let ordered = if bits >> 31 == 1 {
                !bits
            } else {
                bits | 1 << 31
            };
            (ordered, i as u32)
        })
        .collect();
    let mut spare = vec![(0u32, 0u32); keyed.len()];
    for shift in [0, 11, 22] {
        let mut counts = [0usize; 2048];
        for &(k, _) in &keyed {
            counts[(k >> shift) as usize & 2047] += 1;
        }
        // Far to near: the largest digits first.
        let mut at = 0;
        for c in counts.iter_mut().rev() {
            let n = *c;
            *c = at;
            at += n;
        }
        for &(k, i) in &keyed {
            let slot = &mut counts[(k >> shift) as usize & 2047];
            spare[*slot] = (k, i);
            *slot += 1;
        }
        std::mem::swap(&mut keyed, &mut spare);
    }
    keyed.iter().map(|&(_, i)| i).collect()
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
    /// The faces of the simplified chunks (see `lod`): the same shading from a compact vertex.
    face_lod_pipeline: wgpu::RenderPipeline,
    /// The same chunks' normals for the ambient occlusion, made with it.
    ao_lod_pipeline: Option<wgpu::RenderPipeline>,
    /// Whether each face chunk is within reach of a plane that cuts the model, so it is drawn
    /// whole (empty when no plane cuts).
    cut_chunks: Vec<bool>,
    /// The opaque side of a triangle whose other side is see-through.
    solid_side_pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    /// Selected and hovered edges again, across and up, when the style widens them.
    wide_edge_pipeline: wgpu::RenderPipeline,
    /// The same two for edges that carry data for the edge effects (see `scene::EdgeExtra`).
    edge_fx_pipeline: wgpu::RenderPipeline,
    wide_edge_fx_pipeline: wgpu::RenderPipeline,
    /// Profiles again, across and up, as wide as the style says.
    profile_edge_pipeline: wgpu::RenderPipeline,
    /// The marks at the ends of edges, where the style has endpoints.
    endpoint_pipeline: wgpu::RenderPipeline,
    /// The day canvas's gradient, when the style has one.
    canvas_pipeline: wgpu::RenderPipeline,
    /// The ground's sun shadows, multiplied over the background.
    sun_ground_pipeline: wgpu::RenderPipeline,
    sun_ground_clear_pipeline: wgpu::RenderPipeline,
    guide_pipeline: wgpu::RenderPipeline,
    /// The fill of closed cuts, flat, nearer than the faces it lies on.
    cut_fill_pipeline: wgpu::RenderPipeline,
    /// The cuts' outlines, and again across and up for their width (see `vs_cut_edge_wide`).
    cut_edge_pipeline: wgpu::RenderPipeline,
    wide_cut_edge_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    /// A tool's translucent preview solid.
    preview_pipeline: wgpu::RenderPipeline,
    /// What the ambient occlusion's normal pipeline is made from, when it is needed.
    scene_shader: wgpu::ShaderModule,
    scene_layout: wgpu::PipelineLayout,
    /// The ambient occlusion passes, made when a style first asks for it.
    ssao: Option<Ssao>,
    /// Bound as the ambient occlusion while there is none.
    ao_placeholder: wgpu::TextureView,
    /// False where the device cannot draw ambient occlusion (see `disable_ambient_occlusion`).
    ao_available: bool,
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
    /// The sun's settings as of the last `set_sun`; None shades by the headlight.
    sun: Option<SunLight>,
    /// The sun's shadow map, made when the first frame needs it.
    sun_map: Option<SunMap>,
    sun_sampler: wgpu::Sampler,
    /// Bound as the sun map until there is one.
    sun_placeholder: wgpu::TextureView,
    /// The sun casters as of the last `set_model`: how many corners, a hash of them (so a change
    /// is found without keeping a copy), their vertex buffer (None when there are none), their
    /// bounds, and a count of their changes, as for the lamps' casters. A moved caster changes
    /// the bytes and so the hash; only a 64-bit hash collision could miss a change.
    sun_count: u32,
    sun_hash: u64,
    /// The casters of the last `set_model`, kept on the CPU for a sun that is off.
    sun_pending: Vec<[f32; 3]>,
    /// Whether `sun_casters` holds `sun_pending` (or the sun is off and holds nothing).
    sun_synced: bool,
    sun_casters: Option<wgpu::Buffer>,
    sun_bounds: Option<(Vec3, Vec3)>,
    sun_generation: u64,
    /// The ground quad the sun's shadows are drawn on, and the rectangle it was made for.
    sun_ground: Option<(Mesh, [[f32; 2]; 2])>,
    /// Day, night or studio, as of the last `set_lighting`.
    lighting: Lighting,
    /// Faces drawn in their tags' colors, as of the last `set_color_by_tag` (see
    /// `scene::scene_base_with`).
    color_by_tag: bool,
    /// X-ray and Hidden Geometry, as of the last `set_glass_view`.
    glass_view: GlassView,
    /// How section planes are shown, as of the last `set_section_display`.
    section_display: SectionDisplay,
    /// How the frames look, as of the last `set_style`.
    style: ViewStyle,
    /// The model's chosen style, as of the last `set_model_style`; None draws the default look
    /// with the theme's background.
    model_style: Option<StyleSettings>,
    /// Printing: white paper, no canvas gradient, ground grid or axes.
    print: bool,
    /// Cleared to alpha 0 with no canvas, ground grid or axes: see
    /// `set_transparent_background`.
    transparent: bool,
    /// No ground grid, axes or ground shadows: see `set_hide_ground`.
    hide_ground: bool,
    /// The scene's lights, as of the last `set_model`.
    lights: Vec<Light>,
    /// The grid and the axes in the style's colors (see `set_style`) along the drawing axes
    /// `axes` (see `set_axes`); `flat_axes` is the axes lying on the ground alone, for their
    /// wide copies, `vertical_axis` the others (either may be none for axes that are moved).
    flat_ground: Mesh,
    vertical_axis: Option<Mesh>,
    flat_axes: Option<Mesh>,
    /// The drawing axes in world space, as of the last `set_model`.
    axes: DAffine3,
    /// What the scene's positions are relative to, as of the last `set_model` (see
    /// `SceneBase::origin`). Zero for a model near the world origin; the camera, overlay and
    /// preview are moved by it on their way to the GPU.
    origin: DVec3,
    faces: Option<Mesh>,
    /// The fill and the outlines of the section cuts.
    cut_fill: Option<Mesh>,
    cut_edges: Option<Mesh>,
    /// The see-through triangles of `faces`, if any.
    glass: Option<Glass>,
    edges: Option<Mesh>,
    /// The opaque triangles of `faces` and the vertices of `edges` in spatial chunks, for
    /// models big enough to be worth culling (see `set_culling_from`).
    face_chunks: Option<ChunkPlan>,
    edge_chunks: Option<ChunkPlan>,
    /// How many triangles (or edges) a model needs before its drawing is culled by chunk.
    cull_from: usize,
    /// Simplified copies of the opaque chunks of `faces` (see `set_lod`).
    lod: LodState,
    /// Whether chunks far enough away are drawn simplified, and how many pixels across the
    /// cells of a simplified chunk may be.
    lod_on: bool,
    lod_pixels: f32,
    /// When `set_model` last ran with LOD on, and the level each chunk was last drawn at
    /// (0: whole).
    lod_since: Option<std::time::Instant>,
    /// The most bytes a level's buffer may hold (the device's limit and this).
    lod_limit: u64,
    lod_last: std::cell::RefCell<Vec<u8>>,
    /// Which chunks of `face_chunks` hold something selected, and something hovered.
    selected_chunks: Vec<bool>,
    hovered_chunks: Vec<bool>,
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
    /// The meshes' triangles and edges, and the cuts, from the last `set_model`, kept for the
    /// next.
    parts: PartsCache,
}

/// How a pipeline uses the depth buffer.
#[derive(Clone, Copy)]
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
        // The sun's shadow map and its sampler: a 2D depth texture, which every backend has.
        let sun_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 7,
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
        // The ambient occlusion, read with `textureLoad`.
        let ao_entries = [wgpu::BindGroupLayoutEntry {
            binding: 8,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }];
        let entries = if max_shadows.is_some() {
            [
                &scene_entries[..],
                &shadow_entries[..],
                &sun_entries[..],
                &ao_entries[..],
            ]
            .concat()
        } else {
            [&scene_entries[..], &sun_entries[..], &ao_entries[..]].concat()
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
        let lod_faces = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LodVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &LOD_ATTRIBS,
        })];
        let lines_only = [Some(line_layout.clone())];
        let flagged_faces = [
            Some(face_layout),
            Some(flag_layout.clone()),
            Some(hover_layout(&FACE_HOVER_ATTRIBS)),
        ];
        let extra_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<EdgeExtra>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &EDGE_EXTRA_ATTRIBS,
        };
        let plain_lines = [
            Some(line_layout.clone()),
            Some(flag_layout.clone()),
            Some(hover_layout(&EDGE_HOVER_ATTRIBS)),
        ];
        let flagged_lines = [
            Some(line_layout),
            Some(flag_layout),
            Some(hover_layout(&EDGE_HOVER_ATTRIBS)),
            Some(extra_layout),
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
        let face_lod_pipeline = pipeline(
            "simplified faces",
            "vs_face_lod",
            "fs_face",
            &lod_faces,
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
            // Alpha accumulates as coverage (source over), so see-through faces on a
            // transparent background get the alpha they show, not its square.
            Some(wgpu::BlendState {
                color: wgpu::BlendState::ALPHA_BLENDING.color,
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        );
        let edge_pipeline = pipeline(
            "edges",
            "vs_edge",
            "fs_line",
            &plain_lines,
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
            &plain_lines,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        // Profiles again, offset across and up (see `vs_edge_profile_wide`), and the marks at
        // the ends of edges (see `vs_edge_endpoint`).
        let edge_depth = DepthUse {
            write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
        };
        let edge_fx_pipeline = pipeline(
            "edges with effects",
            "vs_edge_fx",
            "fs_line",
            &flagged_lines,
            lines,
            edge_depth,
            None,
        );
        let wide_edge_fx_pipeline = pipeline(
            "wide edges with effects",
            "vs_edge_wide_fx",
            "fs_line",
            &flagged_lines,
            lines,
            edge_depth,
            None,
        );
        let profile_edge_pipeline = pipeline(
            "profile edges",
            "vs_edge_profile_wide",
            "fs_line",
            &flagged_lines,
            lines,
            edge_depth,
            None,
        );
        let endpoint_pipeline = pipeline(
            "edge endpoints",
            "vs_edge_endpoint",
            "fs_line",
            &flagged_lines,
            lines,
            edge_depth,
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
        // The ground's sun shadows: the background times how much darker the shadow makes the
        // ground (see `fs_sun_ground`), drawn without depth.
        let sun_ground_pipeline = pipeline(
            "sun ground",
            "vs_sun_ground",
            "fs_sun_ground",
            &lines_only,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        );
        // The same on a transparent background: black with the alpha the shadow gives.
        let sun_ground_clear_pipeline = pipeline(
            "sun ground clear",
            "vs_sun_ground",
            "fs_sun_ground_clear",
            &lines_only,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            Some(wgpu::BlendState::ALPHA_BLENDING),
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
        let sun_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sun map"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // Bound until the first frame with sun shadows makes the map.
        let sun_placeholder = depth_texture(device, "no sun map", 1, false);
        let textures = TextureArray::new(device, queue, &Scene::default());
        let shadow_maps = max_shadows.map(|max| ShadowMaps::new(device, max));
        let ao_placeholder = ssao::placeholder(device, queue);
        let bind_group = bind_group(
            device,
            &bind_layout,
            [&uniforms, &lighting_uniforms],
            &textures,
            &sampler,
            shadow_maps.as_ref().map(|m| (&m.cubes, &shadow_sampler)),
            (&sun_placeholder, &sun_sampler, &ao_placeholder),
        );
        let (flat_ground, vertical_axis, flat_axes) = ground_meshes(
            device,
            &ViewStyle::CLASSIC,
            ViewStyle::CLASSIC.day_background(),
            DAffine3::IDENTITY,
        );
        Self {
            device: device.clone(),
            queue: queue.clone(),
            format,
            flat_ground_pipeline,
            ground_pipeline,
            wide_flat_axes_pipeline,
            wide_vertical_axis_pipeline,
            face_pipeline,
            face_lod_pipeline,
            ao_lod_pipeline: None,
            cut_chunks: Vec::new(),
            solid_side_pipeline,
            glass_pipeline,
            edge_pipeline,
            wide_edge_pipeline,
            edge_fx_pipeline,
            wide_edge_fx_pipeline,
            profile_edge_pipeline,
            endpoint_pipeline,
            canvas_pipeline,
            sun_ground_pipeline,
            sun_ground_clear_pipeline,
            guide_pipeline,
            cut_fill_pipeline,
            cut_edge_pipeline,
            wide_cut_edge_pipeline,
            overlay_pipeline,
            preview_pipeline,
            scene_shader: shader,
            scene_layout: layout,
            ssao: None,
            ao_placeholder,
            ao_available: true,
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
            sun: None,
            sun_map: None,
            sun_sampler,
            sun_placeholder,
            sun_pending: Vec::new(),
            sun_synced: false,
            sun_casters: None,
            sun_count: 0,
            sun_hash: 0,
            sun_bounds: None,
            sun_generation: 0,
            sun_ground: None,
            lighting: Lighting::Day,
            color_by_tag: false,
            glass_view: GlassView::default(),
            section_display: SectionDisplay::default(),
            style: ViewStyle::CLASSIC,
            model_style: None,
            print: false,
            transparent: false,
            hide_ground: false,
            lights: Vec::new(),
            flat_ground,
            vertical_axis,
            flat_axes,
            axes: DAffine3::IDENTITY,
            origin: DVec3::ZERO,
            faces: None,
            cut_fill: None,
            cut_edges: None,
            glass: None,
            edges: None,
            face_chunks: None,
            edge_chunks: None,
            cull_from: CULL_FROM,
            lod: LodState::Off,
            lod_on: false,
            lod_pixels: CELL_PIXELS,
            lod_since: None,
            lod_limit: u64::MAX,
            lod_last: std::cell::RefCell::new(Vec::new()),
            selected_chunks: Vec::new(),
            hovered_chunks: Vec::new(),
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
            parts: PartsCache::default(),
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

    /// Sets how many triangles (or edges) a model needs for its drawing to be culled by
    /// spatial chunk, from the next `set_model` on: with fewer, it is drawn whole. 0 culls
    /// every model. Chunks hold up to that many triangles (or edges) too, at most a few
    /// thousand.
    pub fn set_culling_from(&mut self, count: usize) {
        self.cull_from = count;
    }

    /// How many edges the profile copies draw in a frame seen by `camera`, and how many they
    /// would draw whole, when the edges are culled by chunk; None when they are drawn whole
    /// or there are no edge effects.
    pub fn visible_profile_edges(&self, camera: &Camera) -> Option<(u32, u32)> {
        let camera = &*self.relative(camera);
        let plan = self.edge_chunks.as_ref()?;
        let edges = self.edges.as_ref()?;
        edges.extras.as_ref()?;
        let runs = plan.visible_wide(camera, self.edge_effect_reach(self.pixels_per_point));
        Some(match &edges.profile_edges {
            Some(list) if !list.chunk_starts.is_empty() => {
                let drawn = candidate_runs(plan, list, &runs.solid)
                    .iter()
                    .map(|r| r.end - r.start)
                    .sum::<u32>();
                (drawn / 2, list.count / 2)
            }
            Some(list) => (list.count / 2, list.count / 2),
            None => (ChunkPlan::covered(&runs.solid) / 2, edges.count / 2),
        })
    }

    /// How many of the model's opaque triangles a frame seen by `camera` draws, and how many
    /// it has in all, when the faces are culled by chunk; None when they are drawn whole.
    pub fn visible_triangles(&self, camera: &Camera) -> Option<(u32, u32)> {
        let camera = &*self.relative(camera);
        let plan = self.face_chunks.as_ref()?;
        let runs = plan.visible(camera);
        Some((ChunkPlan::covered(&runs.solid) / 3, plan.kept as u32 / 3))
    }

    /// Turns on the drawing of far chunks of a big model as simplified copies (off by default:
    /// pictures made off screen are full detail). Takes effect for models set from now on; the
    /// copies are made on another thread once a view needs them, and until they are ready the
    /// chunks are drawn whole. Turning it off draws whole at once, for comparing pictures. Not
    /// on the web.
    pub fn set_lod(&mut self, on: bool) {
        self.lod_on = on && !cfg!(target_arch = "wasm32");
        if !self.lod_on {
            self.drop_lod();
        }
    }

    /// Limits the bytes a level of the simplified copies may take to `bytes`, on top of the
    /// device's own limit: a level over it is left out. For tests.
    pub fn set_lod_buffer_limit(&mut self, bytes: u64) {
        self.lod_limit = bytes;
    }

    /// True while the simplified copies are waiting to be made or being made, so a caller that
    /// draws only on demand knows to call `poll_lod` soon.
    pub fn lod_busy(&self) -> bool {
        matches!(self.lod, LodState::Pending(_) | LodState::Building { .. })
    }

    /// Moves the making of the simplified copies on without drawing: starts it once the model
    /// has settled, takes the copies when done. True when they arrived, so a frame should be
    /// drawn.
    pub fn poll_lod(&mut self) -> bool {
        self.advance_lod()
    }

    /// Sets how many pixels across the cells of a simplified chunk may be on the screen (0.1
    /// to 8); the default is 2, and what is dropped is of about that size.
    pub fn set_lod_pixels(&mut self, pixels: f32) {
        self.lod_pixels = if pixels.is_finite() {
            pixels.clamp(0.1, 8.0)
        } else {
            CELL_PIXELS
        };
    }

    /// Makes the simplified copies of the model now and waits for them, for tests and tools
    /// that draw straight away.
    pub fn wait_for_lod(&mut self) {
        if let LodState::Pending(faces) = std::mem::replace(&mut self.lod, LodState::Off) {
            self.start_lod(faces);
        }
        if let LodState::Building { done, .. } = std::mem::replace(&mut self.lod, LodState::Off) {
            self.lod = done
                .recv()
                .ok()
                .flatten()
                .map_or(LodState::Off, LodState::Ready);
            self.lod_last.get_mut().clear();
        }
    }

    /// The bytes of vertices each level of the simplified copies holds, once they are made.
    pub fn lod_level_bytes(&self) -> Vec<u64> {
        match &self.lod {
            LodState::Ready(lod) => lod.level_bytes(),
            _ => Vec::new(),
        }
    }

    /// The bytes the simplified copies take on the GPU; 0 before they are made.
    pub fn lod_gpu_bytes(&self) -> u64 {
        match &self.lod {
            LodState::Ready(lod) => lod.gpu_bytes(),
            _ => 0,
        }
    }

    /// Waits until the GPU has taken what `set_model` and the like uploaded, and frees what it
    /// replaced; for offscreen use, which draws no frames in between.
    pub fn settle(&self) {
        self.queue.submit([]);
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// How many opaque triangles a frame seen by `camera` draws, counting the simplified ones
    /// as drawn, and how many the model has in all; None when the faces are not chunked.
    pub fn drawn_triangles(&self, camera: &Camera) -> Option<(u32, u32)> {
        let camera = &*self.relative(camera);
        let plan = self.face_chunks.as_ref()?;
        let runs = self.visible(camera, false).faces?;
        let lod: u32 = runs.lod.iter().map(|r| ChunkPlan::covered(r)).sum();
        Some((
            (ChunkPlan::covered(&runs.solid) + lod) / 3,
            plan.kept as u32 / 3,
        ))
    }

    /// True when chunk `k` has something selected or hovered, so it is drawn whole.
    fn chunk_marked(&self, k: usize) -> bool {
        self.selected_chunks.get(k).copied().unwrap_or(false)
            || self.hovered_chunks.get(k).copied().unwrap_or(false)
    }

    /// The chunked lists' ranges in view of `camera`. With `remember`, the levels chosen are
    /// kept to choose the next ones by (see `Lod::pick`).
    fn visible(&self, camera: &Camera, remember: bool) -> Visible {
        let kept = std::cell::RefCell::new(Vec::new());
        let faces = self.face_chunks.as_ref().map(|c| match &self.lod {
            LodState::Ready(lod) if self.lod_on => c.visible_by(camera, |k, per_pixel| {
                let mut last = self.lod_last.borrow_mut();
                if last.len() != c.chunks.len() {
                    *last = vec![0; c.chunks.len()];
                }
                if self.chunk_marked(k) || self.cut_chunks.get(k).copied().unwrap_or(false) {
                    if remember {
                        last[k] = 0;
                    }
                    return None;
                }
                let (next, pick) = lod.pick(k, per_pixel, self.lod_pixels, last[k]);
                if remember {
                    last[k] = next;
                }
                if pick.is_some()
                    && let Some(range) = lod.kept(k)
                {
                    kept.borrow_mut().push(range);
                }
                pick
            }),
            _ => c.visible(camera),
        });
        let faces = faces.map(|mut runs| {
            runs.lod_kept = kept.into_inner();
            runs
        });
        Visible {
            faces,
            edges: self.edge_chunks.as_ref().map(|c| c.visible(camera)),
        }
    }

    /// Drops the simplified copies, and stops a build of them.
    fn drop_lod(&mut self) {
        if let LodState::Building { cancel, .. } = &self.lod {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.lod = LodState::Off;
        self.lod_last.get_mut().clear();
    }

    /// Starts making the simplified copies from `faces` on another thread.
    fn start_lod(&mut self, faces: std::sync::Arc<Vec<BaseFaceVertex>>) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(plan) = self.face_chunks.clone() {
            let device = self.device.clone();
            let limit = self.lod_limit.min(device.limits().max_buffer_size);
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (sender, done) = std::sync::mpsc::channel();
            let flag = cancel.clone();
            std::thread::spawn(move || {
                let lod = Lod::build(&device, &plan, &faces, &flag, limit);
                drop(faces);
                let _ = sender.send(lod);
            });
            self.lod = LodState::Building { done, cancel };
        }
        #[cfg(target_arch = "wasm32")]
        let _ = faces;
    }

    /// Moves the making of the simplified copies on: takes them when they are done, starts
    /// them when the model has settled. True when they arrived.
    fn advance_lod(&mut self) -> bool {
        match std::mem::replace(&mut self.lod, LodState::Off) {
            LodState::Building { done, cancel } => match done.try_recv() {
                Ok(lod) => {
                    self.lod = lod.map_or(LodState::Off, LodState::Ready);
                    self.lod_last.get_mut().clear();
                    return true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.lod = LodState::Building { done, cancel }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("the thread making the simplified copies ended without a result");
                }
            },
            LodState::Pending(faces) => {
                if self.lod_on && self.lod_since.is_some_and(|t| t.elapsed() >= LOD_SETTLE) {
                    self.start_lod(faces);
                } else {
                    self.lod = LodState::Pending(faces);
                }
            }
            other => self.lod = other,
        }
        false
    }

    /// Draws the vertices of the simplified levels in `runs` with `pipeline`, which takes
    /// `LodVertex`es, and the triangles those chunks keep whole with `full`, which takes the
    /// ordinary face vertices and their flags. True when it drew any (the pipeline and vertex
    /// buffers are then not the ones set before).
    fn draw_lod(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        runs: Option<&Runs>,
        pipeline: &wgpu::RenderPipeline,
        full: &wgpu::RenderPipeline,
    ) -> bool {
        let (LodState::Ready(lod), Some(runs)) = (&self.lod, runs) else {
            return false;
        };
        let mut drew = false;
        if let (Some((kept, zeros)), false) = (lod.kept_buffers(), runs.lod_kept.is_empty()) {
            pass.set_pipeline(full);
            pass.set_vertex_buffer(0, kept.slice(..));
            pass.set_vertex_buffer(1, zeros.slice(..));
            pass.set_vertex_buffer(2, zeros.slice(..));
            draw_runs(pass, &runs.lod_kept, 1);
            drew = true;
        }
        for (level, list) in runs.lod.iter().enumerate() {
            if let (Some(buffer), false) = (lod.buffer(level), list.is_empty()) {
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                draw_runs(pass, list, 1);
                drew = true;
            }
        }
        drew
    }

    /// Rebuilds the face and edge buffers from the scene with the open context `context`, with
    /// nothing selected until `set_selection`. Call when the model or the open context changes.
    pub fn set_model(&mut self, scene: &Scene, context: &[InstanceId]) {
        // What the last model holds is freed first, so the new one is not built beside it.
        self.drop_lod();
        self.lod_since = self.lod_on.then(std::time::Instant::now);
        self.faces = None;
        self.glass = None;
        self.edges = None;
        if !self.textures.holds(scene) {
            self.textures = TextureArray::new(&self.device, &self.queue, scene);
            self.rebind();
        }
        let axes = world_drawing_axes(scene, context);
        let edge_colors = self
            .model_style
            .as_ref()
            .map_or_else(EdgeColors::default, |s| EdgeColors::new(&s.edges, axes));
        let base = scene_base_cached(
            scene,
            context,
            self.color_by_tag,
            self.glass_view,
            self.section_display,
            &edge_colors,
            &mut self.parts,
        );
        self.set_axes(axes, base.origin);
        let (face_groups, edge_groups) = base.spans.instance_groups();
        let chunk = self.cull_from.clamp(1, CHUNK_PRIMITIVES);
        // Big models are reordered so that nearby placements lie together (see `ChunkPlan`);
        // the see-through triangles go last, for the glass pass.
        self.face_chunks = (base.faces.len() / 3 >= self.cull_from).then(|| {
            let faces = &base.faces;
            ChunkPlan::new(
                faces.len(),
                &face_groups,
                3,
                chunk,
                &|i| faces[i].pos,
                &|first| !faces[first].is_glass(),
            )
        });
        self.selected_chunks = Vec::new();
        self.hovered_chunks = Vec::new();
        self.edge_chunks = (base.edges.len() / 2 >= self.cull_from).then(|| {
            let edges = &base.edges;
            ChunkPlan::new(
                edges.len(),
                &edge_groups,
                2,
                chunk,
                &|i| edges[i].pos,
                &|_| true,
            )
        });
        self.faces = flagged_mesh(
            &self.device,
            "faces",
            &base.faces,
            self.face_chunks.as_ref(),
        );
        self.glass = match &self.face_chunks {
            Some(plan) => {
                // The see-through triangles are the last ones.
                let tail = plan.permute_tail(&base.faces, plan.kept);
                let glass: Vec<u32> = (plan.kept as u32..plan.total() as u32).step_by(3).collect();
                Glass::new(
                    &self.device,
                    plan.total(),
                    &|i| tail[i - plan.kept].pos,
                    &glass,
                    false,
                )
            }
            None => Glass::new(
                &self.device,
                base.faces.len(),
                &|i| base.faces[i].pos,
                &base.glass(),
                true,
            ),
        };
        self.edges = edge_mesh(
            &self.device,
            &base.edges,
            &base.edge_extras,
            self.edge_chunks.as_ref(),
        );
        self.cut_chunks = match &self.face_chunks {
            Some(c) if !base.cut_planes.is_empty() => c
                .chunks
                .iter()
                .map(|ch| crate::lod::near_planes(ch.lo, ch.hi, &base.cut_planes))
                .collect(),
            _ => Vec::new(),
        };
        self.cut_fill = mesh(&self.device, "cut fill", &base.cut_fill);
        self.cut_edges = mesh(&self.device, "cut edges", &base.cut_edges);
        // The simplified copies are made later, from these vertices.
        if self.lod_on && self.face_chunks.is_some() {
            self.lod = LodState::Pending(std::sync::Arc::new(base.faces));
        }
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
        // The sun's map keeps what casts for it (see `sync_sun_casters`).
        self.sun_pending = base.sun_casters;
        self.sun_synced = false;
        if self.sun.is_some() {
            self.sync_sun_casters();
        }
        // The uploads' staging buffers and the replaced ones are freed only after a submit and
        // a poll; without frames in between, models would pile up.
        self.queue.submit([]);
        let _ = self.device.poll(wgpu::PollType::Poll);
    }

    /// Uploads the sun's casters, and makes the map draw again when they differ from the last
    /// ones, as the shadow casters are. Done only while sun shading is on, so a model without
    /// it pays no hash and holds no buffer; the casters of the last `set_model` wait in
    /// `sun_pending` for a sun.
    fn sync_sun_casters(&mut self) {
        if self.sun_synced {
            return;
        }
        self.sun_synced = true;
        let casters = &self.sun_pending;
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytemuck::cast_slice::<_, u8>(casters).hash(&mut hasher);
            hasher.finish()
        };
        let count = casters.len() as u32;
        if (count, hash) != (self.sun_count, self.sun_hash) {
            self.sun_generation += 1;
            self.sun_casters = (!casters.is_empty()).then(|| {
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("sun casters"),
                        contents: bytemuck::cast_slice(casters),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
            });
            self.sun_bounds = casters.iter().map(|&p| Vec3::from(p)).fold(
                None,
                |bounds: Option<(Vec3, Vec3)>, p| {
                    Some(bounds.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))))
                },
            );
            self.sun_count = count;
            self.sun_hash = hash;
        }
    }

    /// Draws the axes (and the grid's lines through the world origin) as the drawing axes
    /// `axes` say, a frame in world space (see `session::world_drawing_axes`); the ground grid
    /// stays on z = 0. Rebuilt only when they change.
    fn set_axes(&mut self, axes: DAffine3, origin: DVec3) {
        let moved = origin != self.origin;
        if axes == self.axes && !moved {
            return;
        }
        self.axes = axes;
        self.origin = origin;
        (self.flat_ground, self.vertical_axis, self.flat_axes) = ground_meshes(
            &self.device,
            &self.style,
            self.day_toward(&self.style),
            self.drawn_axes(),
        );
        if moved {
            self.write_preview();
        }
    }

    /// The drawing axes in the scene's relative coordinates.
    fn drawn_axes(&self) -> DAffine3 {
        DAffine3::from_translation(-self.origin) * self.axes
    }

    /// A world position (from a caller's f32 overlay or preview) in the scene's relative
    /// coordinates. The subtraction is exact in f64; a position that came in as f32 far out
    /// has already lost its low bits.
    fn relative_point(&self, p: [f32; 3]) -> [f32; 3] {
        (DVec3::from(p.map(f64::from)) - self.origin)
            .as_vec3()
            .to_array()
    }

    /// `camera` as the GPU sees the scene: moved by the scene's origin (see
    /// `SceneBase::origin`) so that eye, view and projection are worked out in f64 from small
    /// numbers. The camera itself, and `camera.eye` far from the origin, stay f64. Borrowed
    /// as it is near the world origin.
    fn relative<'c>(&self, camera: &'c Camera) -> Cow<'c, Camera> {
        if self.origin == DVec3::ZERO {
            return Cow::Borrowed(camera);
        }
        let mut c = camera.clone();
        c.eye -= self.origin;
        c.target -= self.origin;
        Cow::Owned(c)
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
            (
                self.sun_map
                    .as_ref()
                    .map_or(&self.sun_placeholder, |m| &m.view),
                &self.sun_sampler,
                self.ssao
                    .as_ref()
                    .and_then(|s| s.targets())
                    .map_or(&self.ao_placeholder, |t| &t.ao),
            ),
        );
    }

    /// Switches ambient occlusion off for good, as on a browser's WebGL2: a style that asks for
    /// it draws as without.
    pub fn disable_ambient_occlusion(&mut self) {
        self.ao_available = false;
        if self.ssao.take().is_some() {
            self.rebind();
        }
    }

    /// Whether this renderer can draw ambient occlusion.
    pub fn ambient_occlusion_available(&self) -> bool {
        self.ao_available
    }

    /// The model style's ambient occlusion when it is on and available.
    fn ambient_occlusion(&self) -> Option<kernel::AmbientOcclusion> {
        self.model_style
            .map(|s| s.ambient_occlusion)
            .filter(|ao| ao.on && self.ao_available)
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

    /// Draws faces as glass (`x_ray`) and hidden entities ghosted (`hidden_geometry`) from the
    /// next `set_model` on. True when that changes what is drawn, so the model must be set
    /// again.
    pub fn set_glass_view(&mut self, view: GlassView) -> bool {
        let changed = self.glass_view != view;
        self.glass_view = view;
        changed
    }

    /// Shows section planes and cuts as `display` says from the next `set_model` on. True when
    /// that changes what is drawn, so the model must be set again.
    pub fn set_section_display(&mut self, display: SectionDisplay) -> bool {
        let changed = self.section_display != display;
        self.section_display = display;
        changed
    }

    /// Shades the frames drawn from now on by day with `sun`'s light and shadows instead of the
    /// headlight; None shades by the headlight. Night ignores it.
    pub fn set_sun(&mut self, sun: Option<SunLight>) {
        self.sun = sun;
        if sun.is_some() {
            self.sync_sun_casters();
        } else if self.sun_count != 0 {
            // No sun, no casters: they are made again from `sun_pending` if one comes back.
            self.sun_casters = None;
            self.sun_bounds = None;
            self.sun_count = 0;
            self.sun_hash = 0;
            self.sun_synced = false;
        }
    }

    /// How many times the sun's shadow map has been drawn since the renderer was made: once per
    /// change of the sun or the casters or, in the studio, the camera, not per frame.
    pub fn sun_maps_drawn(&self) -> u64 {
        self.sun_map.as_ref().map_or(0, |m| m.drawn)
    }

    /// The light that shades a frame like the sun, with its map drawn (or kept) if it has one:
    /// the sun by day, the studio's key light (from the camera's upper left front) in the studio
    /// (with a map when shadows are on). None by night or without sun shading.
    fn sun_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
    ) -> Option<SunFrame> {
        let (light, key) = match self.lighting {
            Lighting::Day => (self.sun?, false),
            Lighting::Night => return None,
            Lighting::Studio => {
                let direction = studio_direction(camera, STUDIO_KEY_DIRECTION);
                let light = SunLight {
                    direction,
                    light: STUDIO_KEY,
                    dark: STUDIO_AMBIENT,
                    on_ground: self.sun.is_some_and(|s| s.on_ground) && direction.z > 0.0,
                };
                (light, true)
            }
        };
        // The studio's shadows follow the shadow setting: the sun being on.
        let shadows = self.sun.is_some();
        let bounds = self
            .sun_bounds
            .filter(|_| shadows && (key || light.is_up()));
        let fit = bounds.map(|(lo, hi)| fit(light.direction, lo, hi, SUN_MAP_SIZE));
        if let Some(fit) = &fit {
            if self.sun_map.is_none() {
                self.sun_map = Some(SunMap::new(&self.device));
                self.rebind();
            }
            let casters = self.sun_casters.as_ref().map(|b| (b, self.sun_count));
            if let Some(map) = &mut self.sun_map {
                map.draw(
                    &self.queue,
                    encoder,
                    &fit.view_proj,
                    self.sun_generation,
                    casters,
                );
            }
        }
        Some(SunFrame { light, fit, key })
    }

    /// The ground quad for `ground`, kept while it stays the same.
    fn sun_ground_mesh(&mut self, ground: [glam::Vec2; 2]) -> Option<&Mesh> {
        let key = ground.map(|v| v.to_array());
        if self.sun_ground.as_ref().map(|(_, k)| *k) != Some(key) {
            let [lo, hi] = ground;
            let corner = |x: f32, y: f32| LineVertex {
                pos: [x, y, 0.0],
                color: [0.0; 4],
            };
            let quad = [
                corner(lo.x, lo.y),
                corner(hi.x, lo.y),
                corner(hi.x, hi.y),
                corner(lo.x, lo.y),
                corner(hi.x, hi.y),
                corner(lo.x, hi.y),
            ];
            self.sun_ground = mesh(&self.device, "sun ground", &quad).map(|m| (m, key));
        }
        self.sun_ground.as_ref().map(|(m, _)| m)
    }

    /// Lights the frames drawn from now on by day, by night or in the studio.
    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = lighting;
    }

    /// Shows `selection`, ids of the open context given to the last `set_model`, by rewriting
    /// the selection flags only. Call when just the selection changes.
    pub fn set_selection(&mut self, selection: &BTreeSet<EntityId>) {
        let flags = self.spans.flags(selection);
        // The faces' flags in the chunks' order, worked out once.
        let moved = self
            .face_chunks
            .as_ref()
            .map(|plan| plan.permute(&flags.faces));
        self.selected_chunks = match (&self.face_chunks, &moved) {
            (Some(plan), Some(moved)) => plan
                .chunks
                .iter()
                .map(|c| {
                    moved
                        .get(c.vertices.start as usize..c.vertices.end as usize)
                        .is_some_and(|f| f.iter().any(|&f| f != 0.0))
                })
                .collect(),
            _ => Vec::new(),
        };
        for (mesh, flags, plan, moved) in [
            (
                &self.faces,
                &flags.faces,
                &self.face_chunks,
                moved.as_deref(),
            ),
            (&self.edges, &flags.edges, &self.edge_chunks, None),
        ] {
            if let Some(buffer) = mesh.as_ref().and_then(|m| m.flags.as_ref()) {
                match (plan, moved) {
                    (_, Some(moved)) => {
                        self.queue
                            .write_buffer(buffer, 0, bytemuck::cast_slice(moved))
                    }
                    (Some(plan), None) => self.queue.write_buffer(
                        buffer,
                        0,
                        bytemuck::cast_slice(&plan.permute(flags)),
                    ),
                    (None, None) => self
                        .queue
                        .write_buffer(buffer, 0, bytemuck::cast_slice(flags)),
                }
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
        self.hovered_chunks = match &self.face_chunks {
            Some(plan) => {
                let mut marked = vec![false; plan.chunks.len()];
                if !self.hover_ranges.faces.is_empty() {
                    for placed in plan.map_range(self.hover_ranges.faces.clone()) {
                        marked[plan.chunks_in(placed)].fill(true);
                    }
                }
                marked
            }
            None => Vec::new(),
        };
        self.refresh_guides();
    }

    /// Sets the hover flags of `ranges` to `value`.
    fn write_hover(&self, ranges: &Ranges, value: f32) {
        for (mesh, range, plan) in [
            (&self.faces, &ranges.faces, &self.face_chunks),
            (&self.edges, &ranges.edges, &self.edge_chunks),
        ] {
            if let Some(buffer) = mesh.as_ref().and_then(|m| m.hover.as_ref())
                && !range.is_empty()
            {
                // A chunked list is reordered; the range may lie in several places now.
                let placed = match plan {
                    Some(plan) => plan.map_range(range.clone()),
                    None => vec![range.clone()],
                };
                for range in placed {
                    let offset = (range.start * std::mem::size_of::<f32>()) as u64;
                    let flags = vec![value; range.len()];
                    self.queue
                        .write_buffer(buffer, offset, bytemuck::cast_slice(&flags));
                }
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
        self.write_preview();
    }

    /// Uploads `preview_vertices` in the scene's relative coordinates.
    fn write_preview(&mut self) {
        let moved: Vec<PreviewVertex>;
        let triangles = if self.origin == DVec3::ZERO {
            &self.preview_vertices
        } else {
            moved = self
                .preview_vertices
                .iter()
                .map(|v| PreviewVertex {
                    pos: self.relative_point(v.pos),
                    ..*v
                })
                .collect();
            &moved
        };
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
                extras: None,
                profile_edges: None,
                endpoint_edges: None,
            });
        }
        if let Some(preview) = &mut self.preview {
            if !bytes.is_empty() {
                self.queue.write_buffer(&preview.buffer, 0, bytes);
            }
            preview.count = triangles.len() as u32;
        }
    }

    /// True when the glass drawn last is in a held order that is not exact for the camera:
    /// one more frame with the camera unchanged sorts it. A caller that draws only on demand
    /// should draw once shortly after the camera stops.
    pub fn needs_settle_frame(&self) -> bool {
        self.glass.as_ref().is_some_and(Glass::needs_settle)
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

    /// Frees the render targets (they are made again at the next frame): for after an
    /// unusually large capture.
    pub fn release_targets(&mut self) {
        self.targets = None;
    }

    /// Draws the frames from now on on a transparent background (or, off, on the style's): the
    /// target is cleared to alpha 0 and the canvas gradient, the ground grid and the axes stay
    /// out, so what is drawn (faces, edges, guides, the sun's shadow on the ground) is all that
    /// has alpha. Antialiased edges come out with premultiplied color: the color already
    /// multiplied by the coverage in alpha.
    pub fn set_transparent_background(&mut self, on: bool) {
        self.transparent = on;
    }

    /// Draws the frames from now on without the ground: no grid, axes or sun shadows on the
    /// ground, and no canvas gradient unless the style has a sky. The background stays.
    pub fn set_hide_ground(&mut self, on: bool) {
        self.hide_ground = on;
    }

    /// Draws the frames from now on in `style`: the window sets its theme's; render_view and
    /// headless keep `ViewStyle::CLASSIC`. Recolors selected guides without a rebuild; the grid
    /// and axes are rebuilt only when their colors change.
    pub fn set_style(&mut self, style: ViewStyle) {
        if style == self.style {
            return;
        }
        if style.axes != self.style.axes
            || (self.model_style.is_none() && style.day_background() != self.style.day_background())
        {
            (self.flat_ground, self.vertical_axis, self.flat_axes) = ground_meshes(
                &self.device,
                &style,
                self.day_toward(&style),
                self.drawn_axes(),
            );
        }
        self.style = style;
        self.refresh_guides();
    }

    /// Draws the frames from now on in the model's chosen style: its background, sky, ground
    /// and fog (not by night, nor in a print, which ignores them but for edge settings) and edge
    /// colors; None, the default, draws the theme's look. True when that changes what the model
    /// is drawn like, so it must be set again (`set_model`).
    pub fn set_model_style(&mut self, style: Option<&StyleSettings>) -> bool {
        let style = style.copied();
        if style == self.model_style {
            return false;
        }
        // What the scene is built from: the colors, and whether edges carry profile data, soft
        // edges and the flags that hide or move them (the sizes are uniforms).
        let edge_look = |s: &Option<StyleSettings>| {
            s.map(|s| {
                let colors = EdgeColors::new(&s.edges, glam::DAffine3::IDENTITY);
                (
                    s.edges.color_mode,
                    s.edges.color,
                    colors.extras,
                    colors.profiles,
                )
            })
        };
        let recolored = edge_look(&style) != edge_look(&self.model_style);
        let toward = self.day_toward_with(&self.style, style.as_ref());
        let moved = toward != self.day_toward(&self.style);
        self.model_style = style;
        if moved {
            (self.flat_ground, self.vertical_axis, self.flat_axes) =
                ground_meshes(&self.device, &self.style, toward, self.drawn_axes());
        }
        recolored
    }

    /// The bytes held for the edge effects' per-vertex data: 0 when no edge effect is on, which
    /// draws edges from the plain buffers alone.
    pub fn edge_extra_bytes(&self) -> u64 {
        self.edges
            .as_ref()
            .and_then(|m| m.extras.as_ref())
            .map_or(0, |b| b.size())
    }

    /// The bytes the sun's casters hold on the GPU: 0 while sun shading is off.
    pub fn sun_caster_bytes(&self) -> u64 {
        self.sun_casters.as_ref().map_or(0, |b| b.size())
    }

    /// Sets what the chosen style asks of edges in `lighting` (profiles, sketchy edges,
    /// endpoints, hidden edges; sizes in logical pixels, `scale` physical ones to each).
    /// Returns how many instances to draw of the profile copies and of the endpoint marks, 0
    /// for none.
    fn set_edge_effects(&self, lighting: &mut LightingUniforms, scale: f32) -> (u32, u32) {
        let Some(edges) = self.model_style.map(|s| s.edges) else {
            return (0, 0);
        };
        lighting.edge_fx[1] = if edges.show_edges { 1.0 } else { 0.0 };
        let (mut profile_copies, mut endpoint_copies) = (0, 0);
        // The copies of a profile and of a mark are centered on the line: each drawn at its
        // offset, from -width / 2 to width / 2, across and up.
        if let Some(width) = edges.profiles {
            let (shifted, step) = spread(width * scale);
            lighting.edge_fx[0] = 1.0;
            lighting.wide[3] = step;
            lighting.edge_fx2[3] = (shifted / 2) as f32;
            profile_copies = shifted + 2;
        }
        if let Some(sketchy) = edges.sketchy {
            lighting.edge_fx[2] = sketchy.jitter * scale;
            lighting.edge_fx[3] = sketchy.extension * scale;
        }
        if let Some(length) = edges.endpoints {
            let (shifted, step) = spread(ENDPOINT_WIDTH_PT * scale);
            lighting.edge_fx2[0] = length * scale;
            lighting.edge_fx2[1] = step;
            lighting.edge_fx2[2] = (shifted / 2) as f32;
            // Two ends, each with a copy per offset, across and up.
            endpoint_copies = 2 * (shifted + 2);
        }
        (profile_copies, endpoint_copies)
    }

    /// How far, in pixels, the edge effects draw beyond an edge: the widest of the profile
    /// copies, the endpoint marks and the sketchy jitter and extension, added.
    fn edge_effect_reach(&self, scale: f32) -> f32 {
        let Some(edges) = self.model_style.map(|s| s.edges) else {
            return 0.0;
        };
        let sketchy = edges
            .sketchy
            .map_or(0.0, |k| k.jitter.abs() + k.extension.abs());
        (edges.profiles.unwrap_or(0.0)
            + edges.endpoints.map_or(0.0, |l| l + ENDPOINT_WIDTH_PT)
            + sketchy)
            * scale
    }

    /// What the negative axes fade toward by day: the model style's background, else the
    /// theme's.
    fn day_toward(&self, style: &ViewStyle) -> [f32; 4] {
        self.day_toward_with(style, self.model_style.as_ref())
    }

    fn day_toward_with(&self, style: &ViewStyle, model: Option<&StyleSettings>) -> [f32; 4] {
        background(Lighting::Day, style, model)
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
        // Everything below works in the scene's relative coordinates (see `relative`).
        let relative = self.relative(camera);
        let camera = &*relative;
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(self.make_targets(size));
        }
        self.advance_lod();
        let occlusion = self.ambient_occlusion();
        self.prepare_ao(occlusion.is_some(), size);

        let uniforms = Uniforms::new(camera);
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let visible = self.visible(camera, true);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let used = strongest(&self.lights, camera.eye, MAX_LIGHTS);
        let shadow_slots = self.shadow_slots(&used, camera.eye, &mut encoder);
        let sun = self.sun_frame(&mut encoder, camera);
        // A print is white paper: the style's background, sky, ground and fog stay out of it.
        let canvas_style = self.model_style.filter(|_| !self.print);
        let mut lighting = LightingUniforms::new(
            &used,
            &shadow_slots,
            self.lighting,
            &self.style,
            canvas_style.as_ref(),
            (viewport.width, viewport.height),
        );
        lighting.set_camera(camera);
        if occlusion.is_some() {
            lighting.ao[0] = 1.0;
        }
        if let Some(sun) = &sun {
            lighting.set_sun(sun);
        }
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
        let ppp = self.pixels_per_point;
        let (profile_copies, endpoint_copies) = self.set_edge_effects(&mut lighting, ppp);
        self.queue
            .write_buffer(&self.lighting_uniforms, 0, bytemuck::bytes_of(&lighting));
        // The ground shows the sun's shadows unless the camera looks up at it from below.
        let ground_shadows = sun
            .as_ref()
            .filter(|s| s.light.on_ground && camera.eye.z >= 0.0)
            .and_then(|s| s.fit)
            .and_then(|f| self.sun_ground_mesh(f.ground).map(|m| m.count));
        let targets = self.targets.as_ref().expect("targets just created");
        let moved: Vec<LineVertex>;
        let overlay = if self.origin == DVec3::ZERO {
            overlay
        } else {
            moved = overlay
                .iter()
                .map(|v| LineVertex {
                    pos: self.relative_point(v.pos),
                    color: v.color,
                })
                .collect();
            &moved
        };
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
            glass.sort(
                &self.queue,
                camera,
                visible.faces.as_ref().map(|r| &r.rest[..]),
            );
        }

        let [r, g, b, a] = if self.print {
            [1.0; 4]
        } else if self.transparent {
            [0.0; 4]
        } else {
            background(self.lighting, &self.style, canvas_style.as_ref()).map(f64::from)
        };
        // Clamp the viewport to the target; an empty one leaves just the cleared background.
        let (x, y) = (viewport.x.min(size.0), viewport.y.min(size.1));
        let (w, h) = (
            viewport.width.min(size.0 - x),
            viewport.height.min(size.1 - y),
        );
        if let Some(occlusion) = occlusion
            && w > 0
            && h > 0
        {
            self.encode_ao(&mut encoder, camera, [x, y, w, h], occlusion, &visible);
        }
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
            if w > 0 && h > 0 {
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(0, &self.bind_group, &[]);
                let canvas = match &canvas_style {
                    Some(m) if self.hide_ground => m.sky.is_some(),
                    None if self.hide_ground => false,
                    Some(m) => {
                        m.sky.is_some() || (m.ground.is_some() && self.lighting == Lighting::Day)
                    }
                    None => self.style.has_gradient(),
                };
                if !self.print && !self.transparent && self.lighting != Lighting::Night && canvas {
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
                    if let Some(extras) = &mesh.extras {
                        pass.set_vertex_buffer(3, extras.slice(..));
                    }
                };
                let draw = |pass: &mut wgpu::RenderPass, pipeline, mesh: Option<&Mesh>| {
                    if let Some(mesh) = mesh {
                        bind(pass, pipeline, mesh);
                        pass.draw(0..mesh.count, 0..1);
                    }
                };
                // The ground's shadow goes over the grid and the flat axes (none are drawn in a
                // print) so they darken with the ground, and under the vertical axis.
                let sun_ground = |pass: &mut wgpu::RenderPass| {
                    if self.hide_ground {
                        return;
                    }
                    if let (Some(count), Some((ground, _))) = (ground_shadows, &self.sun_ground) {
                        let pipeline = if self.transparent {
                            &self.sun_ground_clear_pipeline
                        } else {
                            &self.sun_ground_pipeline
                        };
                        bind(pass, pipeline, ground);
                        pass.draw(0..count, 0..1);
                    }
                };
                if self.print || self.transparent || self.hide_ground {
                    sun_ground(&mut pass);
                } else if self.origin != DVec3::ZERO {
                    // The grid lies at the world origin, a long way from this model.
                    sun_ground(&mut pass);
                    draw(
                        &mut pass,
                        &self.ground_pipeline,
                        self.vertical_axis.as_ref(),
                    );
                } else {
                    draw(
                        &mut pass,
                        &self.flat_ground_pipeline,
                        Some(&self.flat_ground),
                    );
                    if self.style.wide_axes
                        && let Some(flat) = &self.flat_axes
                    {
                        bind(&mut pass, &self.wide_flat_axes_pipeline, flat);
                        pass.draw(0..flat.count, 0..axis_copies);
                    }
                    sun_ground(&mut pass);
                    draw(
                        &mut pass,
                        &self.ground_pipeline,
                        self.vertical_axis.as_ref(),
                    );
                    if self.style.wide_axes
                        && let Some(vertical) = &self.vertical_axis
                    {
                        bind(&mut pass, &self.wide_vertical_axis_pipeline, vertical);
                        pass.draw(0..vertical.count, 0..axis_copies);
                    }
                }
                match (&self.faces, &self.glass) {
                    (Some(faces), glass) if self.face_chunks.is_some() => {
                        bind(&mut pass, &self.face_pipeline, faces);
                        draw_runs(
                            &mut pass,
                            visible.faces.as_ref().map_or(&[][..], |r| &r.solid),
                            1,
                        );
                        self.draw_lod(
                            &mut pass,
                            visible.faces.as_ref(),
                            &self.face_lod_pipeline,
                            &self.face_pipeline,
                        );
                        if let Some(glass) = glass {
                            bind(&mut pass, &self.solid_side_pipeline, faces);
                            pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..glass.count(), 0, 0..1);
                        }
                    }
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
                // Edges with effects come with their data buffer; the others draw as ever.
                let with_effects = self.edges.as_ref().is_some_and(|m| m.extras.is_some());
                let (edge_pipeline, wide_pipeline) = if with_effects {
                    (&self.edge_fx_pipeline, &self.wide_edge_fx_pipeline)
                } else {
                    (&self.edge_pipeline, &self.wide_edge_pipeline)
                };
                match (&self.edges, &self.edge_chunks) {
                    (Some(edges), Some(_)) => {
                        bind(&mut pass, edge_pipeline, edges);
                        draw_runs(
                            &mut pass,
                            visible.edges.as_ref().map_or(&[][..], |r| &r.solid),
                            1,
                        );
                    }
                    (edges, _) => draw(&mut pass, edge_pipeline, edges.as_ref()),
                }
                let edges_hovered = !self.hover_ranges.edges.is_empty();
                if self.style.wide_selected_edges
                    && (self.edges_selected || edges_hovered)
                    && let Some(edges) = &self.edges
                {
                    bind(&mut pass, wide_pipeline, edges);
                    match &self.edge_chunks {
                        Some(_) => draw_runs(
                            &mut pass,
                            visible.edges.as_ref().map_or(&[][..], |r| &r.solid),
                            wide_copies,
                        ),
                        None => pass.draw(0..edges.count, 0..wide_copies),
                    }
                }
                // The effects reach past the edge's own box and show even from a chunk too small
                // to see, so they use their own, wider view of the chunks.
                let fx_runs = (profile_copies > 0 || endpoint_copies > 0)
                    .then(|| {
                        self.edge_chunks
                            .as_ref()
                            .map(|plan| plan.visible_wide(camera, self.edge_effect_reach(ppp)))
                    })
                    .flatten();
                let cull = self
                    .edge_chunks
                    .as_ref()
                    .zip(fx_runs.as_ref())
                    .map(|(plan, r)| (plan, &r.solid[..]));
                if with_effects
                    && profile_copies > 0
                    && let Some(edges) = &self.edges
                {
                    bind(&mut pass, &self.profile_edge_pipeline, edges);
                    draw_candidates(
                        &mut pass,
                        edges,
                        edges.profile_edges.as_ref(),
                        cull,
                        profile_copies,
                    );
                }
                if with_effects
                    && endpoint_copies > 0
                    && let Some(edges) = &self.edges
                {
                    bind(&mut pass, &self.endpoint_pipeline, edges);
                    draw_candidates(
                        &mut pass,
                        edges,
                        edges.endpoint_edges.as_ref(),
                        cull,
                        endpoint_copies,
                    );
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

    /// Makes (`on`) or drops the ambient occlusion buffers for a target `size` pixels across, and
    /// binds them to the face shader.
    fn prepare_ao(&mut self, on: bool, size: (u32, u32)) {
        let changed = if on {
            let (device, shader, layout) = (&self.device, &self.scene_shader, &self.scene_layout);
            self.ao_lod_pipeline
                .get_or_insert_with(|| ao_normal_pipeline(device, shader, layout, true));
            self.ssao
                .get_or_insert_with(|| {
                    Ssao::new(device, ao_normal_pipeline(device, shader, layout, false))
                })
                .resize(size)
        } else {
            self.ssao.as_mut().is_some_and(Ssao::release)
        };
        if changed {
            self.rebind();
        }
    }

    /// Draws the opaque faces' depth and normals at half resolution into the occlusion buffers
    /// and works the occlusion out from them, for the view `rect` (`x, y, width, height`, in
    /// target pixels) seen by `camera`.
    fn encode_ao(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
        rect: [u32; 4],
        occlusion: kernel::AmbientOcclusion,
        visible: &Visible,
    ) {
        let Some(ssao) = &self.ssao else { return };
        let Some(t) = ssao.targets() else { return };
        let [x, y, w, h] = rect.map(|v| v as f32);
        ssao.write_params(
            &self.queue,
            camera,
            [x, y, w, h],
            occlusion.radius,
            occlusion.strength,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ao normals"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.normals,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.5,
                            g: 0.5,
                            b: 1.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_viewport(x * 0.5, y * 0.5, w * 0.5, h * 0.5, 0.0, 1.0);
            pass.set_bind_group(0, &self.bind_group, &[]);
            if let Some(faces) = &self.faces {
                pass.set_pipeline(ssao.normal_pipeline());
                pass.set_vertex_buffer(0, faces.buffer.slice(..));
                if let Some(flags) = &faces.flags {
                    pass.set_vertex_buffer(1, flags.slice(..));
                }
                if let Some(hover) = &faces.hover {
                    pass.set_vertex_buffer(2, hover.slice(..));
                }
                if self.face_chunks.is_some() {
                    draw_runs(
                        &mut pass,
                        visible.faces.as_ref().map_or(&[][..], |r| &r.solid),
                        1,
                    );
                    if let Some(lod_normals) = &self.ao_lod_pipeline
                        && self.draw_lod(
                            &mut pass,
                            visible.faces.as_ref(),
                            lod_normals,
                            ssao.normal_pipeline(),
                        )
                    {
                        // The glass below is drawn from the full vertices again.
                        pass.set_pipeline(ssao.normal_pipeline());
                        pass.set_vertex_buffer(0, faces.buffer.slice(..));
                        if let Some(flags) = &faces.flags {
                            pass.set_vertex_buffer(1, flags.slice(..));
                        }
                        if let Some(hover) = &faces.hover {
                            pass.set_vertex_buffer(2, hover.slice(..));
                        }
                    }
                    if let Some(glass) = &self.glass {
                        pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..glass.count(), 0, 0..1);
                    }
                } else {
                    match &self.glass {
                        // The see-through sides are left out (the shader discards them from the
                        // sides that have both).
                        Some(glass) => {
                            if glass.solid_count > 0 {
                                pass.set_index_buffer(
                                    glass.solid.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                pass.draw_indexed(0..glass.solid_count, 0, 0..1);
                            }
                            pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..glass.count(), 0, 0..1);
                        }
                        None => pass.draw(0..faces.count, 0..1),
                    }
                }
            }
        }
        ssao.encode(encoder);
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

/// The opaque faces' normals for the ambient occlusion, at one sample per pixel; made with the
/// occlusion passes, when a style first asks for them.
fn ao_normal_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    simplified: bool,
) -> wgpu::RenderPipeline {
    let vector = |stride, attributes| wgpu::VertexBufferLayout {
        array_stride: stride,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes,
    };
    let float = std::mem::size_of::<f32>() as u64;
    let full = [
        Some(vector(
            std::mem::size_of::<BaseFaceVertex>() as u64,
            &FACE_ATTRIBS,
        )),
        Some(vector(float, &FLAG_ATTRIBS)),
        Some(vector(float, &FACE_HOVER_ATTRIBS)),
    ];
    let lod = [Some(vector(
        std::mem::size_of::<LodVertex>() as u64,
        &LOD_ATTRIBS,
    ))];
    let buffers = if simplified { &lod[..] } else { &full[..] };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ao normals"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if simplified { "vs_face_lod" } else { "vs_face" }),
            compilation_options: Default::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_ao_normal"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: ssao::NORMAL_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The bind group every pipeline uses: the camera uniforms, the material textures and their
/// sampler, the lighting uniforms and, when there are any, the shadow maps and their sampler,
/// and the sun's shadow map with its sampler.
fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    [uniforms, lighting]: [&wgpu::Buffer; 2],
    textures: &TextureArray,
    sampler: &wgpu::Sampler,
    shadows: Option<(&wgpu::TextureView, &wgpu::Sampler)>,
    (sun_map, sun_sampler, ao): (&wgpu::TextureView, &wgpu::Sampler, &wgpu::TextureView),
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
    entries.push(wgpu::BindGroupEntry {
        binding: 6,
        resource: wgpu::BindingResource::TextureView(sun_map),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 7,
        resource: wgpu::BindingResource::Sampler(sun_sampler),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 8,
        resource: wgpu::BindingResource::TextureView(ao),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("camera and textures"),
        layout,
        entries: &entries,
    })
}

/// The flat ground (the grid and the axes lying on it), the other axes and the flat axes alone,
/// for the drawing axes `axes` in `style`'s axis colors, their negative halves faded toward the
/// day background `toward`. With the world axes the flat axes are X and Y and the other one is Z.
fn ground_meshes(
    device: &wgpu::Device,
    style: &ViewStyle,
    toward: [f32; 4],
    axes: DAffine3,
) -> (Mesh, Option<Mesh>, Option<Mesh>) {
    let colors = style.axes;
    (
        mesh(
            device,
            "flat ground lines",
            &flat_ground_lines_at(colors, toward, axes),
        )
        .expect("the grid is never empty"),
        mesh(
            device,
            "vertical axis",
            &vertical_axis_lines_at(colors, toward, axes),
        ),
        mesh(
            device,
            "flat axes",
            &flat_axis_lines_at(colors, toward, axes),
        ),
    )
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
        extras: None,
        profile_edges: None,
        endpoint_edges: None,
    })
}

/// Uploads vertices with selection and hover flag buffers beside them, every flag 0 (wgpu
/// zeroes new buffers), or None when there are none to draw. With a `plan`, the vertices are
/// uploaded in the order it gives them (the flags are in that order too).
fn flagged_mesh<T: bytemuck::Pod>(
    device: &wgpu::Device,
    label: &str,
    vertices: &[T],
    plan: Option<&ChunkPlan>,
) -> Option<Mesh> {
    let mut mesh = match plan {
        Some(plan) if !vertices.is_empty() => {
            // Written straight into the buffer, as a model this big is a lot to copy twice.
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of_val(vertices) as u64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: true,
            });
            {
                let mut view = buffer
                    .slice(..)
                    .get_mapped_range_mut()
                    .expect("the buffer was created mapped");
                let stride = std::mem::size_of::<T>();
                for s in plan.segments() {
                    view.slice(s.new * stride..(s.new + s.len) * stride)
                        .copy_from_slice(bytemuck::cast_slice(&vertices[s.old..s.old + s.len]));
                }
            }
            buffer.unmap();
            Mesh {
                buffer,
                count: vertices.len() as u32,
                flags: None,
                hover: None,
                extras: None,
                profile_edges: None,
                endpoint_edges: None,
            }
        }
        _ => mesh(device, label, vertices)?,
    };
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

/// `flagged_mesh` of model edges, with a fourth buffer holding `extras` when there are any (with
/// none, no edge effect is on and nothing more is held).
fn edge_mesh(
    device: &wgpu::Device,
    vertices: &[LineVertex],
    extras: &[EdgeExtra],
    plan: Option<&ChunkPlan>,
) -> Option<Mesh> {
    let mut mesh = flagged_mesh(device, "edges", vertices, plan)?;
    let moved;
    let extras = match plan {
        Some(plan) => {
            moved = plan.permute(extras);
            &moved[..]
        }
        None => extras,
    };
    if !extras.is_empty() {
        let (profiles, endpoints) = edge_candidates(extras);
        let indices = |list: Vec<u32>| {
            // Only worth an index buffer when it leaves edges out.
            (list.len() < extras.len()).then(|| Indices {
                count: list.len() as u32,
                // The list ascends in the new vertex order, as the chunks do.
                chunk_starts: plan.map_or_else(Vec::new, |plan| {
                    plan.chunks
                        .iter()
                        .map(|c| list.partition_point(|&v| v < c.vertices.start) as u32)
                        .chain([list.len() as u32])
                        .collect()
                }),
                buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("edge candidates"),
                    // A buffer is never empty; `count` says how much of it is read.
                    contents: bytemuck::cast_slice(if list.is_empty() { &[0u32] } else { &list }),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            })
        };
        mesh.profile_edges = indices(profiles);
        mesh.endpoint_edges = indices(endpoints);
        mesh.extras = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("edge extras"),
                contents: bytemuck::cast_slice(extras),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
    }
    Some(mesh)
}

/// The index ranges of `list` holding the edges of the chunks in the vertex ranges `runs`
/// (ascending), adjacent ones merged. Each range starts and ends on a pair.
fn candidate_runs(
    plan: &ChunkPlan,
    list: &Indices,
    runs: &[std::ops::Range<u32>],
) -> Vec<std::ops::Range<u32>> {
    let mut out: Vec<std::ops::Range<u32>> = Vec::new();
    for r in runs {
        let chunks = plan.chunks_in(r.start as usize..r.end as usize);
        let (from, to) = (
            list.chunk_starts[chunks.start],
            list.chunk_starts[chunks.end],
        );
        if from == to {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == from => last.end = to,
            _ => out.push(from..to),
        }
    }
    out
}

/// Draws `copies` instances of the edges in `candidates`, or of all of `edges` when it is None.
/// With `cull` (the edge chunks and the vertex ranges in view), only the edges of those
/// chunks. Edge vertices come in pairs, so an index keeps its parity (the shader tells an
/// edge's ends apart by `vertex_index`).
fn draw_candidates(
    pass: &mut wgpu::RenderPass<'_>,
    edges: &Mesh,
    candidates: Option<&Indices>,
    cull: Option<(&ChunkPlan, &[std::ops::Range<u32>])>,
    copies: u32,
) {
    match (candidates, cull) {
        (Some(list), _) if list.count == 0 => {}
        (Some(list), Some((plan, runs))) if !list.chunk_starts.is_empty() => {
            pass.set_index_buffer(list.buffer.slice(..), wgpu::IndexFormat::Uint32);
            for r in candidate_runs(plan, list, runs) {
                pass.draw_indexed(r, 0, 0..copies);
            }
        }
        (Some(list), _) => {
            pass.set_index_buffer(list.buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..list.count, 0, 0..copies);
        }
        (None, Some((_, runs))) => draw_runs(pass, runs, copies),
        (None, None) => pass.draw(0..edges.count, 0..copies),
    }
}

/// The vertices of the edges (both of each, in order) that the profile copies and the endpoint
/// marks can ever draw, by what the edges alone say: the open instance's box is drawn as it is,
/// and an edge whose two faces continue each other exactly (their side directions are exact
/// opposites, which makes the shader's profile product negative or zero for every eye) is never
/// a silhouette, so a soft one is never drawn and an ordinary one is no profile. Faces that are
/// only nearly coplanar still have a thin wedge of eyes that see a silhouette, so they stay.
/// Which of the rest are profiles depends on the eye, so the shader still decides that.
fn edge_candidates(extras: &[EdgeExtra]) -> (Vec<u32>, Vec<u32>) {
    let mut profiles = Vec::new();
    let mut endpoints = Vec::new();
    let (pairs, _) = extras.as_chunks::<2>();
    for (k, pair) in pairs.iter().enumerate() {
        let e = &pair[0];
        if e.meta[0] < 0.5 {
            continue;
        }
        let first = 2 * k as u32;
        let a = Vec3::from_slice(&e.side_a);
        let b = Vec3::from_slice(&e.side_b);
        let continues = e.side_a[3] > 0.5 && e.side_b[3] > 0.5 && (a + b) == Vec3::ZERO;
        if !continues {
            profiles.extend([first, first + 1]);
        }
        if e.meta[0] < 1.5 || !continues {
            endpoints.extend([first, first + 1]);
        }
    }
    (profiles, endpoints)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{back_to_front, order_holds, spread};

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
    fn edge_candidates_leave_out_what_can_never_be_drawn() {
        use super::edge_candidates;
        use crate::scene::EdgeExtra;
        let edge = |x: f32, a: [f32; 4], b: [f32; 4]| {
            [EdgeExtra {
                side_a: a,
                side_b: b,
                meta: [x, 0.0, 0.0, 0.0],
                ..EdgeExtra::default()
            }; 2]
        };
        let up = [0.0, 0.0, 1.0, 1.0];
        let down = [0.0, 0.0, -1.0, 1.0];
        let right = [1.0, 0.0, 0.0, 1.0];
        let none = [0.0; 4];
        let extras: Vec<EdgeExtra> = [
            edge(0.0, up, right),                    // 0: a box edge
            edge(1.0, up, right),                    // 1: an ordinary corner
            edge(1.0, up, down),                     // 2: an ordinary edge between coplanar faces
            edge(2.0, up, down),                     // 3: a soft one, between coplanar faces
            edge(2.0, up, [0.0, 0.2, -0.98, 1.0]),   // 4: a soft one on a curve
            edge(1.0, up, none),                     // 5: an edge with one face
            edge(2.0, up, [0.0, 0.0004, -1.0, 1.0]), // 6: a soft one, nearly but not exactly flat
        ]
        .concat();
        let (profiles, endpoints) = edge_candidates(&extras);
        assert_eq!(profiles, [2, 3, 8, 9, 10, 11, 12, 13]);
        assert_eq!(endpoints, [2, 3, 4, 5, 8, 9, 10, 11, 12, 13]);
    }

    #[test]
    fn glass_order_is_kept_for_small_eye_moves_only() {
        // Triangles in a row along x, 10 m apart, 100 m from an eye at the origin.
        let triangles: Vec<(u32, Vec3)> = (0..5)
            .map(|i| (3 * i, Vec3::new(10.0 * i as f32, 100.0, 0.0)))
            .collect();
        let eye = |x: f32| [x, 0.0, 0.0, 1.0];
        let nearest = 100.0;
        let prev = (eye(0.0), nearest);
        let sorted = back_to_front(&triangles, eye(0.0));
        let is_far_to_near = |order: &[u32], view: [f32; 4]| {
            let d: Vec<f32> = order
                .iter()
                .step_by(3)
                .map(|&k| {
                    triangles[(k / 3) as usize]
                        .1
                        .distance_squared(Vec3::from_slice(&view))
                })
                .collect();
            d.windows(2).all(|w| w[0] >= w[1])
        };
        // A small move keeps the order, and it is still back to front.
        assert!(order_holds(&prev, eye(0.5)));
        assert!(is_far_to_near(&sorted, eye(0.5)));
        // A large move sorts again, and the old order is no longer right.
        assert!(!order_holds(&prev, eye(40.0)));
        assert!(!is_far_to_near(&sorted, eye(40.0)));
        assert!(is_far_to_near(
            &back_to_front(&triangles, eye(40.0)),
            eye(40.0)
        ));
        // A parallel view keeps its order only when unchanged.
        let par = ([0.0, -1.0, 0.0, 0.0], f32::INFINITY);
        assert!(order_holds(&par, [0.0, -1.0, 0.0, 0.0]));
        assert!(!order_holds(&par, [0.0, -0.99, 0.1, 0.0]));
    }

    #[test]
    fn a_held_glass_order_is_made_exact_once_the_camera_stops() {
        use super::Glass;
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        else {
            return;
        };
        // Two triangles 10 cm apart sideways, 10 m from the eye.
        let centers = [[0.0, 10.0, 0.0], [0.1, 10.0, 0.0]];
        let mut glass = Glass::new(&device, 6, &|i| centers[i / 3], &[0, 3], false).unwrap();
        let camera = |x: f64| {
            let mut c = session::Camera::new(glam::DVec2::new(64.0, 64.0));
            c.eye = glam::DVec3::new(x, 0.0, 0.0);
            c.target = glam::DVec3::new(x, 10.0, 0.0);
            c
        };
        assert!(!glass.needs_settle());
        let t0 = web_time::Instant::now();
        let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
        glass.sort_at(&queue, &camera(0.0), None, at(0));
        // The second triangle is farther from an eye at x = 0.
        assert_eq!(glass.sorted, [1, 0]);
        assert!(!glass.needs_settle(), "the first order is exact");
        // Moved 9.9 cm: within the hold distance, so the old order stays while moving.
        glass.sort_at(&queue, &camera(0.099), None, at(16));
        assert_eq!(glass.sorted, [1, 0]);
        assert!(glass.needs_settle());
        // A frame with the camera unchanged soon after keeps the hold (a stuttering orbit).
        glass.sort_at(&queue, &camera(0.099), None, at(32));
        assert_eq!(glass.sorted, [1, 0]);
        assert!(glass.needs_settle());
        // Still for the settle time: sorted exactly, the first is now farther.
        glass.sort_at(&queue, &camera(0.099), None, at(16) + super::GLASS_SETTLE);
        assert_eq!(glass.sorted, [0, 1]);
        assert!(!glass.needs_settle());
        // An exact order asks for nothing more.
        glass.sort_at(&queue, &camera(0.099), None, at(500));
        assert!(!glass.needs_settle());
        // A camera that is not finite is never waited for.
        glass.sort_at(&queue, &camera(f64::NAN), None, at(600));
        assert!(!glass.needs_settle());
        glass.sort_at(&queue, &camera(f64::NAN), None, at(900));
        assert!(!glass.needs_settle());
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
