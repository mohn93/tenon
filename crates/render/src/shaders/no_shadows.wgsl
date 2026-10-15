// Appended to scene.wgsl on devices without cube-map arrays: no shadows, every light reaches
// everything in its range.
fn shadow(light: Light, slot: i32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    return 1.0;
}
