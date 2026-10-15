// Drawing cube shadow maps (see shadows.rs): each face of a light's map keeps, in every
// direction, how far the nearest caster is over the light's range.

struct Face {
    view_proj: mat4x4<f32>,
    // xyz: the light; w: its range.
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> face: Face;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_shadow(@location(0) pos: vec3<f32>) -> Out {
    var out: Out;
    out.pos = face.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    return out;
}

// The distance from the light rather than the depth along the face's axis, so a lookup in any
// direction compares like with like.
@fragment
fn fs_shadow(in: Out) -> @builtin(frag_depth) f32 {
    return min(distance(in.world, face.light.xyz) / face.light.w, 1.0);
}
