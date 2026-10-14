struct Uniforms {
    view_proj: mat4x4<f32>,
    // The eye's position (w = 1), or in a parallel view the direction toward the viewer (w = 0).
    eye: vec4<f32>,
    // x: 1 to scale clip z (perspective); y: the depth step of a parallel view.
    depth_offset: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

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
// selected edge (flag 1, from the second vertex buffer) is drawn in SELECTED_EDGE.
@vertex
fn vs_edge(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) selected: f32,
) -> LineOut {
    return line_vertex(pos, select(color, SELECTED_EDGE, selected > 0.5), 1.0005);
}

// Guides: pulled toward the camera like model edges; their color already shows the selection.
@vertex
fn vs_guide(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.0005);
}

// Grid and axes: drawn under faces lying on the ground.
@vertex
fn vs_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 0.998);
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
) -> FaceOut {
    var out: FaceOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    out.normal = normal;
    out.selected = selected;
    out.front_color = front_color;
    out.back_color = back_color;
    out.alpha = alpha;
    return out;
}

// Each side shows its own color (its material's, or the default: light in front, blue-gray
// behind); a headlight at the eye (from the viewer, in a parallel view) shades both. Selected
// faces are tinted blue.
fn face_color(in: FaceOut, front: bool) -> vec3<f32> {
    let n = normalize(in.normal) * select(-1.0, 1.0, front);
    let to_eye = normalize(u.eye.xyz - in.world * u.eye.w);
    let shade = 0.6 + 0.4 * max(dot(n, to_eye), 0.0);
    let base = mix(select(in.back_color, in.front_color, front), SELECTED, 0.5 * in.selected);
    return base * shade;
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
    if select(in.alpha.y, in.alpha.x, front) < 1.0 {
        discard;
    }
    return vec4<f32>(face_color(in, front), 1.0);
}

// See-through sides, blended over everything opaque (back to front, without writing depth).
// A selected one is made more opaque so its tint shows even when it is nearly invisible.
@fragment
fn fs_glass(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let alpha = select(in.alpha.y, in.alpha.x, front);
    if alpha >= 1.0 {
        discard;
    }
    return vec4<f32>(face_color(in, front), mix(alpha, 1.0, 0.4 * in.selected));
}
