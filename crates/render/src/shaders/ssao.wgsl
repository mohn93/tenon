// Screen-space ambient occlusion (see ssao.rs): from the depth and the world normals of the
// opaque faces at half resolution, how much of the hemisphere above each pixel's surface is
// open within a radius, then a blur that does not cross creases.

struct Params {
    // Clip space to world, and world to clip space.
    inv_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    // xyz: the eye (any point on the view plane in a parallel view).
    cam_pos: vec4<f32>,
    // xyz: the unit direction the camera looks along.
    cam_forward: vec4<f32>,
    // The view's rectangle in these (half resolution) pixels: x, y, width, height.
    rect: vec4<f32>,
    // x: the radius in meters; y: the strength.
    settings: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var depth_map: texture_depth_2d;
@group(0) @binding(2) var normal_map: texture_2d<f32>;
// The occlusion as the first pass made it (the blur's input).
@group(0) @binding(3) var raw_map: texture_2d<f32>;

const SAMPLES: u32 = 16u;
const PI2: f32 = 6.28318530718;
const GOLDEN_ANGLE: f32 = 2.39996323;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

// Where the surface seen at pixel `px` with depth `depth` lies.
fn world_at(px: vec2<i32>, depth: f32) -> vec3<f32> {
    let uv = (vec2<f32>(px) + 0.5 - p.rect.xy) / p.rect.zw;
    let h = p.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    return h.xyz / h.w;
}

// How far `x` is along the view direction.
fn view_depth(x: vec3<f32>) -> f32 {
    return dot(x - p.cam_pos.xyz, p.cam_forward.xyz);
}

fn normal_at(px: vec2<i32>) -> vec3<f32> {
    return normalize(textureLoad(normal_map, px, 0).xyz * 2.0 - 1.0);
}

// A different turn of the sample pattern for each pixel of a 4 x 4 block (which the blur
// averages over).
fn turn(px: vec2<i32>) -> f32 {
    let k = u32(px.x & 3) + 4u * u32(px.y & 3);
    return fract(f32(k) * 0.61803398875 + 0.17) * PI2;
}

@fragment
fn fs_ssao(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(frag.xy);
    let size = vec2<i32>(textureDimensions(depth_map));
    let depth = textureLoad(depth_map, px, 0);
    if depth <= 0.0 {
        return vec4<f32>(1.0);
    }
    let radius = p.settings.x;
    let at = world_at(px, depth);
    let n = normal_at(px);
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t0 = normalize(cross(helper, n));
    let b0 = cross(n, t0);
    let a0 = turn(px);
    let t = t0 * cos(a0) + b0 * sin(a0);
    let b = cross(n, t);
    let own = view_depth(at);
    let bias = 0.01 * radius;
    var open = 0.0;
    for (var i = 0u; i < SAMPLES; i++) {
        let k = (f32(i) + 0.5) / f32(SAMPLES);
        // Directions spread evenly in azimuth and by the cosine of the angle to the normal; the
        // lengths vary, most of them short.
        let up = sqrt(k);
        let side = sqrt(1.0 - k);
        let phi = f32(i) * GOLDEN_ANGLE;
        let spread = fract(f32(i) * 0.7548776662 + 0.25);
        let reach = radius * mix(0.1, 1.0, spread);
        let dir = t * (cos(phi) * side) + b * (sin(phi) * side) + n * up;
        let s = at + n * bias + dir * reach;
        let clip = p.view_proj * vec4<f32>(s, 1.0);
        var blocked = 0.0;
        if clip.w > 0.0 {
            let ndc = clip.xy / clip.w;
            let pixel = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * p.rect.zw + p.rect.xy;
            let sp = vec2<i32>(floor(pixel));
            if all(sp >= vec2<i32>(0)) && all(sp < size) && all(pixel >= p.rect.xy)
                && all(pixel < p.rect.xy + p.rect.zw) {
                let sd = textureLoad(depth_map, sp, 0);
                if sd > 0.0 {
                    // The sample is behind the surface seen at its pixel when it lies below that
                    // surface's plane (the pixel's depth alone would call a sample just above a
                    // sloping open face hidden by it, as a pixel is wider than the sample).
                    let q = world_at(sp, sd);
                    if dot(s - q, normal_at(sp)) < -bias {
                        // Surfaces far in front of this one (beyond the radius) do not shade it.
                        blocked = smoothstep(0.0, 1.0, radius / max(abs(own - view_depth(q)), 1e-6));
                    }
                }
            }
        }
        open += 1.0 - blocked;
    }
    return vec4<f32>(open / f32(SAMPLES));
}

// The occlusion averaged over 5 x 5 pixels, each weighted by how near it lies to this pixel's
// surface plane and how alike their normals are, so the dark of a crease does not spread over
// the faces beside it.
@fragment
fn fs_blur(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(frag.xy);
    let size = vec2<i32>(textureDimensions(depth_map));
    let depth = textureLoad(depth_map, px, 0);
    if depth <= 0.0 {
        // Nothing here: a depth no surface matches.
        return vec4<f32>(1.0, 60000.0, 0.0, 1.0);
    }
    let radius = p.settings.x;
    let at = world_at(px, depth);
    let n = normal_at(px);
    var sum = 0.0;
    var total = 0.0;
    for (var i = -2; i <= 2; i++) {
        for (var j = -2; j <= 2; j++) {
            let q = px + vec2<i32>(i, j);
            if any(q < vec2<i32>(0)) || any(q >= size) {
                continue;
            }
            let d = textureLoad(depth_map, q, 0);
            if d <= 0.0 {
                continue;
            }
            let off = dot(world_at(q, d) - at, n) / (0.2 * radius);
            let alike = max(dot(normal_at(q), n), 0.0);
            let w = exp(-off * off) * alike * alike * alike * alike;
            sum += w * textureLoad(raw_map, q, 0).r;
            total += w;
        }
    }
    let open = select(textureLoad(raw_map, px, 0).r, sum / total, total > 1e-4);
    return vec4<f32>(1.0 - clamp(p.settings.y * (1.0 - open), 0.0, 1.0), view_depth(at), 0.0, 1.0);
}
