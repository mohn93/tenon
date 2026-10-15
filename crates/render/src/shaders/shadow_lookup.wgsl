// Shadow lookups (see shadows.rs), appended to scene.wgsl on devices with cube-map arrays.

// A cube shadow map per shadowed light, and a sampler comparing a distance with them.
@group(0) @binding(4) var shadow_maps: texture_depth_cube_array;
@group(0) @binding(5) var shadow_sampler: sampler_comparison;

// SHADOW_SIZE in shadows.rs.
const SHADOW_SIZE: f32 = 256.0;
// Against acne (a surface shadowing itself): the lookup point is moved this many shadow-map
// texels off the surface on its lit side, and a caster must be this many meters nearer the
// light than the point to hide it.
const SHADOW_NORMAL_OFFSET: f32 = 1.5;
const SHADOW_BIAS: f32 = 0.02;

// How much of `light`, with its shadow map in `slot`, reaches `p` on a surface facing `n` toward
// it: 1 lit, 0 hidden. Four lookups 1.5 texels apart, each filtered over 2 x 2 texels by the
// comparing sampler, soften the edge.
fn shadow(light: Light, slot: i32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let center = light.position.xyz;
    // A texel's width at the point's distance: a face spans twice the distance at its middle.
    let texel = 2.0 * distance(p, center) / SHADOW_SIZE;
    let v = p + n * (SHADOW_NORMAL_OFFSET * texel) - center;
    let reference = (length(v) - SHADOW_BIAS) / light.position.w;
    // Two directions across v, a texel long there.
    let other = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(v.z) > 0.9 * length(v));
    let a = normalize(cross(v, other)) * texel;
    let b = normalize(cross(v, a)) * texel;
    var lit = 0.0;
    for (var k = 0; k < 4; k++) {
        let s = select(-0.75, 0.75, (k & 1) == 1);
        let t = select(-0.75, 0.75, (k & 2) == 2);
        lit += textureSampleCompareLevel(shadow_maps, shadow_sampler, v + a * s + b * t, slot, reference);
    }
    return 0.25 * lit;
}
