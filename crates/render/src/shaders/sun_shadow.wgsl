// Drawing the sun's shadow map (see sun_shadow.rs): the casters' depth along the sun's rays,
// seen through one orthographic projection.

struct Sun {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> sun: Sun;

@vertex
fn vs_sun(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return sun.view_proj * vec4<f32>(pos, 1.0);
}

@fragment
fn fs_sun() {}
