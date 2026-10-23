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
// The sun's shadow map, with a sampler comparing a depth with it (see sun_shadow.rs).
@group(0) @binding(6) var sun_map: texture_depth_2d;
@group(0) @binding(7) var sun_sampler: sampler_comparison;

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
    // The step in pixels between the copies of a wide edge (x), of a wide axis (y) and of a section
    // cut edge (z).
    wide: vec4<f32>,
    // xyz: the unit vector toward the sun; w: 0 without sun shading, 1 with it, 2 with its shadow
    // map too.
    sun: vec4<f32>,
    // x: the direct light a face square-on to the sun adds; y: the light a face in shadow keeps;
    // z: the world size of a map texel; w: the world depth the map spans.
    sun_levels: vec4<f32>,
    // xyz: the camera's position; w: 1 in a perspective view, 0 in a parallel one.
    cam_pos: vec4<f32>,
    // xyz: the unit direction the camera looks along; w: in a parallel view, the target's height
    // over its distance from the eye (see `styled_canvas`), else 0.
    cam_forward: vec4<f32>,
    // xyz: the screen's right edge's offset from the view direction, and its up edge's, for the
    // view ray of a pixel (see `view_ray`).
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // x: 1 when the canvas is drawn from the model's style; y: 1 when the eye is below the
    // ground; z: 1 when the ground is drawn seen from below.
    canvas: vec4<f32>,
    // rgb: the sky's color; w: 1 when the style has a sky.
    sky: vec4<f32>,
    // rgb: the ground's color; w: its opacity, 0 without a ground.
    ground: vec4<f32>,
    // x: where the fog starts; y: where it is complete; z: 1 when there is fog.
    fog: vec4<f32>,
    fog_color: vec4<f32>,
    // World to the sun map's clip space.
    sun_view_proj: mat4x4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(3) var<uniform> lighting: Lighting;

// `color` faded toward the background by `fade` (0 to 1; the lighting uniforms set how far).
fn faded_line(color: vec4<f32>, fade: f32) -> vec4<f32> {
    return vec4<f32>(mix(color.rgb, lighting.background.rgb, fade), color.a);
}

// How far `p` is from the eye: the distance in a perspective view, the depth along the view
// direction in a parallel one.
fn eye_distance(p: vec3<f32>) -> f32 {
    let from_eye = p - lighting.cam_pos.xyz;
    return select(dot(from_eye, lighting.cam_forward.xyz), length(from_eye), lighting.cam_pos.w > 0.5);
}

// How far toward the fog color a point `d` away fades: 0 without fog.
fn fog_amount(d: f32) -> f32 {
    if lighting.fog.z < 0.5 {
        return 0.0;
    }
    return smoothstep(lighting.fog.x, lighting.fog.y, d);
}

// `color` faded toward the fog color from `d` away; unchanged without fog.
fn fogged(color: vec3<f32>, d: f32) -> vec3<f32> {
    if lighting.fog.z < 0.5 {
        return color;
    }
    return mix(color, lighting.fog_color.rgb, fog_amount(d));
}

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Where the point is, for the fog's distance per pixel (a distance interpolated between
    // the ends of a long line would be wrong in perspective).
    @location(1) world: vec3<f32>,
    // 1 where the line is fogged.
    @location(2) @interpolate(flat) fogged: f32,
};

// The same line, not fogged (selected and hovered edges, tool overlays).
fn no_fog(line: LineOut) -> LineOut {
    var out = line;
    out.fogged = 0.0;
    return out;
}

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
    out.world = pos;
    out.fogged = 1.0;
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
    let out = line_vertex(
        pos,
        select(
            select(faded_line(color, lighting.lines.x), lighting.selected_edge, selected > 0.5),
            lighting.hover_edge,
            hovered > 0.5,
        ),
        1.0005,
    );
    // Selected and hovered edges stay as they are in the fog.
    if selected > 0.5 || hovered > 0.5 {
        return no_fog(out);
    }
    return out;
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
    return wide_copy(no_fog(out), k, lighting.wide.x);
}

// Copy `k` of a wide line: moved across (even k) or up (odd k) by `step` pixels per pair.
fn wide_copy(line: LineOut, k: u32, step: f32) -> LineOut {
    var out = line;
    let pixels = step * f32(k / 2u + 1u);
    let shift = select(vec2<f32>(0.0, 2.0), vec2<f32>(2.0, 0.0), k % 2u == 0u) * pixels
        / lighting.lines.zw;
    out.pos = vec4<f32>(out.pos.xy + shift * out.pos.w, out.pos.zw);
    return out;
}

// The fill of closed section cuts: pulled toward the camera a little less than the cut edges
// (so those win) but more than the faces, which it must beat where they lie in the cut plane.
@vertex
fn vs_cut_fill(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.0003);
}

// Section cut edges, faded like model edges; `vs_cut_edge_wide` draws them again across (even
// instances) and up (odd ones) by `lighting.wide.z` pixels per pair.
@vertex
fn vs_cut_edge(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, faded_line(color, lighting.lines.x), 1.0006);
}

@vertex
fn vs_cut_edge_wide(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @builtin(instance_index) k: u32,
) -> LineOut {
    return wide_copy(
        line_vertex(pos, faded_line(color, lighting.lines.x), 1.0006),
        k,
        lighting.wide.z,
    );
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

// Wide copies of the flat X and Y axes (as `vs_flat_ground`) and of the Z axis (as
// `vs_ground`), `lighting.wide.y` pixels apart (see `wide_copy`).
@vertex
fn vs_flat_ground_wide(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @builtin(instance_index) k: u32,
) -> LineOut {
    return wide_copy(
        line_vertex(pos, faded_line(color, lighting.lines.y), 1.0),
        k,
        lighting.wide.y,
    );
}

@vertex
fn vs_ground_wide(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @builtin(instance_index) k: u32,
) -> LineOut {
    return wide_copy(
        line_vertex(pos, faded_line(color, lighting.lines.y), 0.998),
        k,
        lighting.wide.y,
    );
}

// Tool previews and inference markers: drawn without depth testing.
@vertex
fn vs_overlay(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return no_fog(line_vertex(pos, color, 1.0));
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    if in.fogged < 0.5 {
        return in.color;
    }
    return vec4<f32>(fogged(in.color.rgb, eye_distance(in.world)), in.color.a);
}

struct CanvasOut {
    @builtin(position) pos: vec4<f32>,
    // 0 at the view's top, 1 at its bottom.
    @location(0) down: f32,
    // The pixel's place on the screen, -1 to 1 (up and right are positive).
    @location(1) ndc: vec2<f32>,
};

// One triangle covering the view, at the far plane.
@vertex
fn vs_canvas(@builtin(vertex_index) i: u32) -> CanvasOut {
    let xy = vec2<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0);
    var out: CanvasOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.down = (1.0 - xy.y) * 0.5;
    out.ndc = xy;
    return out;
}

// The direction a pixel looks in, not normalized: the view direction plus the pixel's offset
// across the screen.
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    return lighting.cam_forward.xyz + ndc.x * lighting.cam_right.xyz + ndc.y * lighting.cam_up.xyz;
}

// The canvas of a model style: the background, with the sky above the horizon (the background at
// the horizon, the sky color toward the zenith) and the ground below it, mixed over by its
// opacity. From below the ground, the ground is the half of the view above the horizon, and only
// when it is drawn from below. A parallel view has no vanishing point, so its pixels are
// classified by the height of the point they show at the target's distance (over that
// distance) added to the view direction's: a level view splits at the row of the ground plane,
// where the grid lies, and a top view is all ground.
fn styled_canvas(ndc: vec2<f32>) -> vec3<f32> {
    let background = lighting.background.rgb;
    let ray = view_ray(ndc);
    let up = (ray.z + lighting.cam_forward.w) / length(ray);
    var base = background;
    if lighting.sky.w > 0.5 && up > 0.0 {
        base = mix(background, lighting.sky.rgb, sqrt(min(up, 1.0)));
    }
    let below = lighting.canvas.y > 0.5;
    let above_ground_seen = (up < 0.0);
    let below_ground_seen = (up > 0.0 && lighting.canvas.z > 0.5);
    let seen = select(above_ground_seen, below_ground_seen, below);
    if lighting.ground.w > 0.0 && seen {
        return mix(base, lighting.ground.rgb, lighting.ground.w);
    }
    return base;
}

@fragment
fn fs_canvas(in: CanvasOut) -> @location(0) vec4<f32> {
    if lighting.canvas.x > 0.5 {
        return vec4<f32>(styled_canvas(in.ndc), 1.0);
    }
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

// How much of the sun reaches `p` on a surface facing `n` toward the sun by `facing` (a cosine):
// 1 lit, 0 in shadow. The point is moved off the surface along its normal, more where the sun
// grazes it, and compared with the map in 3 x 3 taps a texel apart, each filtered over 2 x 2
// texels by the comparing sampler.
const SUN_NORMAL_OFFSET: f32 = 1.0;
const SUN_SLOPE_OFFSET: f32 = 1.5;
const SUN_BIAS: f32 = 1.5;

fn sun_lit(p: vec3<f32>, n: vec3<f32>, facing: f32) -> f32 {
    let texel = lighting.sun_levels.z;
    let sine = sqrt(max(1.0 - facing * facing, 0.0));
    let moved = p + n * (texel * (SUN_NORMAL_OFFSET + SUN_SLOPE_OFFSET * sine));
    let clip = lighting.sun_view_proj * vec4<f32>(moved, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || clip.z > 1.0 {
        return 1.0;
    }
    let reference = clip.z - SUN_BIAS * texel / lighting.sun_levels.w;
    let step = 1.0 / f32(textureDimensions(sun_map).x);
    var lit = 0.0;
    for (var i = -1; i <= 1; i++) {
        for (var j = -1; j <= 1; j++) {
            let at = uv + vec2<f32>(f32(i), f32(j)) * step;
            lit += textureSampleCompareLevel(sun_map, sun_sampler, at, reference);
        }
    }
    return lit / 9.0;
}

// The light the sun gives `p` on a surface facing `n`: what shadow keeps, plus the direct light
// by Lambert's cosine, where the map lets it through.
fn sun_shade(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let facing = max(dot(n, lighting.sun.xyz), 0.0);
    var lit = 1.0;
    if lighting.sun.w > 1.5 && facing > 0.0 {
        lit = sun_lit(p, n, facing);
    }
    return lighting.sun_levels.y + lighting.sun_levels.x * facing * lit;
}

// Each side shows its own color (its material's, or the default: light in front, blue-gray
// behind), or its material's texture instead; ambient light, a headlight at the eye (from the
// viewer, in a parallel view) and the glowing faces' lights shade both. A glowing side is shown
// unshaded. Selected faces are tinted toward the style's color; unselected ones skip the mix so
// their arithmetic stays exactly as before the tint came from a uniform. A hovered face is
// mixed toward the hover color from its own color, so the hover wins over the selection.
// Call it before any discard: the texture lookup needs every pixel of its quad.
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
        mix(color, lighting.hover_face.rgb, lighting.hover_face.w),
        in.hovered > 0.5,
    );
    if select(in.glow.y, in.glow.x, front) > 0.5 {
        return shown;
    }
    var shade = lighting.mode.y + lighting.mode.z * max(dot(n, to_eye), 0.0);
    if lighting.sun.w > 0.5 {
        shade = sun_shade(in.world, n);
    }
    return shown * (vec3<f32>(shade) + lamp_light(in.world, n));
}

// A face's color in the fog; selected and hovered faces stay as they are.
fn face_fog(in: FaceOut, color: vec3<f32>) -> vec3<f32> {
    if in.selected > 0.0 || in.hovered > 0.5 {
        return color;
    }
    return fogged(color, eye_distance(in.world));
}

// Opaque faces.
@fragment
fn fs_face(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return vec4<f32>(face_fog(in, face_color(in, front)), 1.0);
}

// The opaque side of a face whose other side is see-through: drawn with the opaque faces, the
// see-through side left to `fs_glass`.
@fragment
fn fs_face_solid_side(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let color = face_color(in, front);
    if select(in.alpha.y, in.alpha.x, front) < 1.0 {
        discard;
    }
    return vec4<f32>(face_fog(in, color), 1.0);
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
    return vec4<f32>(face_fog(in, color), mix(alpha, 1.0, 0.4 * in.selected));
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

// The ground's shadows (see sun_shadow.rs): a quad on z = 0 drawn over the background, which it
// multiplies by how much darker the ground is in shadow than in the sun.
struct GroundOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_sun_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> GroundOut {
    var out: GroundOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    return out;
}

@fragment
fn fs_sun_ground(in: GroundOut) -> @location(0) vec4<f32> {
    let facing = lighting.sun.z;
    let lit = sun_lit(in.world, vec3<f32>(0.0, 0.0, 1.0), facing);
    let sunny = lighting.sun_levels.y + lighting.sun_levels.x * facing;
    let ratio = lighting.sun_levels.y / max(sunny, 1e-4);
    // A shadow fades out in the fog with what it falls on.
    let shade = mix(ratio, 1.0, lit);
    return vec4<f32>(vec3<f32>(mix(shade, 1.0, fog_amount(eye_distance(in.world)))), 1.0);
}
