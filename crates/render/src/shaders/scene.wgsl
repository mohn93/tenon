struct Uniforms {
    view_proj: mat4x4<f32>,
    // The eye's position (w = 1), or in a parallel view the direction toward the viewer (w = 0).
    eye: vec4<f32>,
    // x: 1 to scale clip z (perspective); y: the depth step of a parallel view.
    depth_offset: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// One layer per textured material (see texture.rs); it repeats both ways.
@group(0) @binding(1) var textures: texture_2d_array<f32>;
@group(0) @binding(2) var texture_sampler: sampler;

// A point light from glowing faces (see lights.rs).
struct Light {
    // xyz: where it is; w: how far it reaches.
    position: vec4<f32>,
    // rgb: its color.
    color: vec4<f32>,
};

// MAX_LIGHTS in lights.rs.
const MAX_LIGHTS: u32 = 128u;

struct Lighting {
    // x: how many lights; y: the ambient light; z: the headlight; w: 1 at night, else 0.
    mode: vec4<f32>,
    // The background, which lines fade toward at night.
    background: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(3) var<uniform> lighting: Lighting;

// How far unselected edges, the grid and the axes fade toward the background at night, so they
// stay quiet outlines rather than bright lines on the dark.
const NIGHT_LINE_FADE: f32 = 0.8;

fn night_line(color: vec4<f32>) -> vec4<f32> {
    let fade = NIGHT_LINE_FADE * lighting.mode.w;
    return vec4<f32>(mix(color.rgb, lighting.background.rgb, fade), color.a);
}

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Depth is reversed (1 = near, 0 = far), so scaling clip z by more than 1 pulls a line slightly
// toward the camera and by less than 1 pushes it away, by a fixed fraction of its distance.
// z is not clamped: vertices behind the camera must keep a positive z so near-plane clipping works.
// Parallel depth is linear, so there the line moves by (depth_scale - 1) eye-to-target distances
// wherever it is.
fn line_vertex(pos: vec3<f32>, color: vec4<f32>, depth_scale: f32) -> LineOut {
    var out: LineOut;
    let clip = u.view_proj * vec4<f32>(pos, 1.0);
    let offset = depth_scale - 1.0;
    let z = clip.z * (1.0 + offset * u.depth_offset.x) + offset * u.depth_offset.y;
    out.pos = vec4<f32>(clip.xy, z, clip.w);
    out.color = color;
    return out;
}

// SELECTED_EDGE_COLOR in scene.rs.
const SELECTED_EDGE: vec4<f32> = vec4<f32>(0.10, 0.35, 0.95, 1.0);

// Model edges: pulled toward the camera by 0.05% of clip depth, enough to win against the faces
// they bound at this depth precision without showing through faces in front of them. A
// selected edge (flag 1, from the second vertex buffer) is drawn in SELECTED_EDGE; the others
// fade at night.
@vertex
fn vs_edge(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) selected: f32,
) -> LineOut {
    return line_vertex(pos, select(night_line(color), SELECTED_EDGE, selected > 0.5), 1.0005);
}

// Guides: pulled toward the camera like model edges; their color already shows the selection.
@vertex
fn vs_guide(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.0005);
}

// Grid and axes: drawn under faces lying on the ground; faded at night.
@vertex
fn vs_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, night_line(color), 0.998);
}

// The flat ground grid, drawn without depth testing before everything else; faded at night.
@vertex
fn vs_flat_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, night_line(color), 1.0);
}

// Tool previews and inference markers: drawn without depth testing.
@vertex
fn vs_overlay(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.0);
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct FaceOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) selected: f32,
    @location(3) front_color: vec3<f32>,
    @location(4) back_color: vec3<f32>,
    // The opacities of the front (x) and back (y) sides.
    @location(5) alpha: vec2<f32>,
    // The texture coordinates of the front (xy) and back (zw) sides.
    @location(6) uv: vec4<f32>,
    // The texture layers of the front (x) and back (y) sides; -1 for none.
    @location(7) @interpolate(flat) layer: vec2<f32>,
    // 1 on a glowing front (x) or back (y) side.
    @location(8) @interpolate(flat) glow: vec2<f32>,
};

const SELECTED: vec3<f32> = vec3<f32>(0.25, 0.45, 0.95);

// `selected` (1 or 0) comes from the second vertex buffer.
@vertex
fn vs_face(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) selected: f32,
    @location(3) front_color: vec3<f32>,
    @location(4) back_color: vec3<f32>,
    @location(5) alpha: vec2<f32>,
    @location(6) uv: vec4<f32>,
    @location(7) layer: vec2<f32>,
    @location(8) glow: vec2<f32>,
) -> FaceOut {
    var out: FaceOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    out.normal = normal;
    out.selected = selected;
    out.front_color = front_color;
    out.back_color = back_color;
    out.alpha = alpha;
    out.uv = uv;
    out.layer = layer;
    out.glow = glow;
    return out;
}

// The light glowing faces cast on a point `p` facing `n`: each light's color by Lambert's
// cosine, falling off with distance (like 1 / (1 + d^2) over a quarter of the range) and
// smoothly to zero at its range. No shadows.
fn lamp_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = min(u32(lighting.mode.x), MAX_LIGHTS);
    for (var i = 0u; i < count; i++) {
        let light = lighting.lights[i];
        let to_light = light.position.xyz - p;
        let d = length(to_light);
        let range = light.position.w;
        if d >= range || d < 1e-4 {
            continue;
        }
        let x = d / range;
        let window = (1.0 - x * x) * (1.0 - x * x);
        let near = 4.0 * x;
        let falloff = window / (1.0 + near * near);
        sum += light.color.rgb * falloff * max(dot(n, to_light / d), 0.0);
    }
    return sum;
}

// Each side shows its own color (its material's, or the default: light in front, blue-gray
// behind), or its material's texture instead; ambient light, a headlight at the eye (from the
// viewer, in a parallel view) and the glowing faces' lights shade both. A glowing side is shown
// unshaded. Selected faces are tinted blue. Call it before any discard: the texture lookup needs
// every pixel of its quad.
fn face_color(in: FaceOut, front: bool) -> vec3<f32> {
    let n = normalize(in.normal) * select(-1.0, 1.0, front);
    let to_eye = normalize(u.eye.xyz - in.world * u.eye.w);
    let layer = select(in.layer.y, in.layer.x, front);
    let uv = select(in.uv.zw, in.uv.xy, front);
    let texel = textureSample(textures, texture_sampler, uv, i32(max(layer, 0.0))).rgb;
    let color = select(select(in.back_color, in.front_color, front), texel, layer >= 0.0);
    let base = mix(color, SELECTED, 0.5 * in.selected);
    if select(in.glow.y, in.glow.x, front) > 0.5 {
        return base;
    }
    let shade = lighting.mode.y + lighting.mode.z * max(dot(n, to_eye), 0.0);
    return base * (vec3<f32>(shade) + lamp_light(in.world, n));
}

// Opaque faces.
@fragment
fn fs_face(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return vec4<f32>(face_color(in, front), 1.0);
}

// The opaque side of a face whose other side is see-through: drawn with the opaque faces, the
// see-through side left to `fs_glass`.
@fragment
fn fs_face_solid_side(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let color = face_color(in, front);
    if select(in.alpha.y, in.alpha.x, front) < 1.0 {
        discard;
    }
    return vec4<f32>(color, 1.0);
}

// See-through sides, blended over everything opaque (back to front, without writing depth).
// A selected one is made more opaque so its tint shows even when it is nearly invisible.
@fragment
fn fs_glass(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let color = face_color(in, front);
    let alpha = select(in.alpha.y, in.alpha.x, front);
    if alpha >= 1.0 {
        discard;
    }
    return vec4<f32>(color, mix(alpha, 1.0, 0.4 * in.selected));
}
