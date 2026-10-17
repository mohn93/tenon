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
    // rgb: its color; w: the slot of its shadow map, or -1 when it casts no shadows.
    color: vec4<f32>,
};

// MAX_LIGHTS in lights.rs.
const MAX_LIGHTS: u32 = 128u;

struct Lighting {
    // x: how many lights; y: the ambient light; z: the headlight; w: 1 at night, else 0.
    mode: vec4<f32>,
    // The background, which lines fade toward.
    background: vec4<f32>,
    // Selected edges and guides.
    selected_edge: vec4<f32>,
    // rgb: the color selected faces are tinted toward; w: how far.
    selected_face: vec4<f32>,
    // x: how far unselected edges fade toward the background; y: how far the grid and axes do;
    // zw: the view's size in pixels.
    lines: vec4<f32>,
    // The day canvas's top and bottom.
    canvas_top: vec4<f32>,
    canvas_bottom: vec4<f32>,
    // rgb: the color hovered faces are mixed toward; w: how far. Zero without a hover.
    hover_face: vec4<f32>,
    // Hovered edges and guides.
    hover_edge: vec4<f32>,
    // x: the step in pixels between the copies of a wide edge.
    wide: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(3) var<uniform> lighting: Lighting;

// `color` faded toward the background by `fade` (0 to 1; the lighting uniforms set how far).
fn faded_line(color: vec4<f32>, fade: f32) -> vec4<f32> {
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

// Model edges: pulled toward the camera by 0.05% of clip depth, enough to win against the faces
// they bound at this depth precision without showing through faces in front of them. A
// selected edge (flag 1, from the second vertex buffer) is drawn in the style's selected-edge
// color; the others fade toward the background as far as the lighting says. A hovered edge
// (flag 1, from the third vertex buffer) is drawn in the hover color.
@vertex
fn vs_edge(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) selected: f32,
    @location(3) hovered: f32,
) -> LineOut {
    return line_vertex(
        pos,
        select(
            select(faded_line(color, lighting.lines.x), lighting.selected_edge, selected > 0.5),
            lighting.hover_edge,
            hovered > 0.5,
        ),
        1.0005,
    );
}

// Selected and hovered edges drawn again, offset across (even instances) and up (odd ones) by
// `lighting.wide.x` pixels per pair, so they read as wide as the instances span; the other
// edges are moved outside the view and clipped away.
@vertex
fn vs_edge_wide(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) selected: f32,
    @location(3) hovered: f32,
    @builtin(instance_index) k: u32,
) -> LineOut {
    var out = line_vertex(
        pos,
        select(lighting.selected_edge, lighting.hover_edge, hovered > 0.5),
        1.0005,
    );
    if selected < 0.5 && hovered < 0.5 {
        out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return out;
    }
    let step = lighting.wide.x * f32(k / 2u + 1u);
    let pixel = select(vec2<f32>(0.0, 2.0), vec2<f32>(2.0, 0.0), k % 2u == 0u) * step
        / lighting.lines.zw;
    out.pos = vec4<f32>(out.pos.xy + pixel * out.pos.w, out.pos.zw);
    return out;
}

// Guides: pulled toward the camera like model edges; their color already shows the selection.
@vertex
fn vs_guide(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.0005);
}

// Grid and axes: drawn under faces lying on the ground; faded as far as the lighting says.
@vertex
fn vs_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, faded_line(color, lighting.lines.y), 0.998);
}

// The flat ground grid, drawn without depth testing before everything else; faded like the axes.
@vertex
fn vs_flat_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, faded_line(color, lighting.lines.y), 1.0);
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

struct CanvasOut {
    @builtin(position) pos: vec4<f32>,
    // 0 at the view's top, 1 at its bottom.
    @location(0) down: f32,
};

// One triangle covering the view, at the far plane.
@vertex
fn vs_canvas(@builtin(vertex_index) i: u32) -> CanvasOut {
    let xy = vec2<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0);
    var out: CanvasOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.down = (1.0 - xy.y) * 0.5;
    return out;
}

@fragment
fn fs_canvas(in: CanvasOut) -> @location(0) vec4<f32> {
    return mix(lighting.canvas_top, lighting.canvas_bottom, in.down);
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
    @location(9) @interpolate(flat) hovered: f32,
};

// `selected` (1 or 0) comes from the second vertex buffer, `hovered` from the third.
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
    @location(9) hovered: f32,
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
    out.hovered = hovered;
    return out;
}

// The light glowing faces cast on a point `p` facing `n`: each light's color by Lambert's
// cosine, falling off with distance (like 1 / (1 + d^2) over a quarter of the range) and
// smoothly to zero at its range, and stopped by casters where the light has a shadow map
// (`shadow`, from shadow_lookup.wgsl or no_shadows.wgsl).
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
        let facing = dot(n, to_light / d);
        if facing <= 0.0 {
            continue;
        }
        var lit = 1.0;
        if light.color.w >= 0.0 {
            lit = shadow(light, i32(light.color.w), p, n);
        }
        sum += light.color.rgb * falloff * facing * lit;
    }
    return sum;
}

// Each side shows its own color (its material's, or the default: light in front, blue-gray
// behind), or its material's texture instead; ambient light, a headlight at the eye (from the
// viewer, in a parallel view) and the glowing faces' lights shade both. A glowing side is shown
// unshaded. Selected faces are tinted toward the style's color; unselected ones skip the mix so
// their arithmetic stays exactly as before the tint came from a uniform. A hovered face is
// mixed toward the hover color the same way. Call it before any discard: the texture lookup
// needs every pixel of its quad.
fn face_color(in: FaceOut, front: bool) -> vec3<f32> {
    let n = normalize(in.normal) * select(-1.0, 1.0, front);
    let to_eye = normalize(u.eye.xyz - in.world * u.eye.w);
    let layer = select(in.layer.y, in.layer.x, front);
    let uv = select(in.uv.zw, in.uv.xy, front);
    let texel = textureSample(textures, texture_sampler, uv, i32(max(layer, 0.0))).rgb;
    let color = select(select(in.back_color, in.front_color, front), texel, layer >= 0.0);
    let tinted = mix(color, lighting.selected_face.rgb, lighting.selected_face.w * in.selected);
    let base = select(color, tinted, in.selected > 0.0);
    let shown = select(
        base,
        mix(base, lighting.hover_face.rgb, lighting.hover_face.w),
        in.hovered > 0.5,
    );
    if select(in.glow.y, in.glow.x, front) > 0.5 {
        return shown;
    }
    let shade = lighting.mode.y + lighting.mode.z * max(dot(n, to_eye), 0.0);
    return shown * (vec3<f32>(shade) + lamp_light(in.world, n));
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

struct PreviewOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

// A tool's preview solid, in root coordinates.
@vertex
fn vs_preview(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
) -> PreviewOut {
    var out: PreviewOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    out.normal = normal;
    out.color = color;
    return out;
}

// The preview solid, shaded by the ambient light and the headlight, at its own opacity.
@fragment
fn fs_preview(in: PreviewOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let to_eye = normalize(u.eye.xyz - in.world * u.eye.w);
    let shade = lighting.mode.y + lighting.mode.z * abs(dot(n, to_eye));
    return vec4<f32>(in.color.rgb * shade, in.color.a);
}
