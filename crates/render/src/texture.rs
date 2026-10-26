//! Material textures on the GPU: one texture array, a layer per textured material, each image
//! resampled to the layer size with its mipmaps worked out here.

use std::collections::HashMap;
use std::sync::Arc;

use kernel::{MaterialId, Scene, Texture, TextureImage};

/// The largest layer side, in pixels (2048 is also the least `max_texture_dimension_2d` of
/// WebGL2 and downlevel devices; a device allowing less gets smaller layers, see `layer_side`).
pub const MAX_LAYER_SIDE: u32 = 2048;
/// The most bytes the array may take with its mipmaps; more or bigger textures get smaller
/// layers.
pub const MAX_ARRAY_BYTES: u64 = 256 * 1024 * 1024;

/// Every material with a texture, in id order: material k's texture is layer k.
pub fn textured(scene: &Scene) -> Vec<(MaterialId, &Texture)> {
    scene
        .materials()
        .filter_map(|(id, m)| Some((id, m.texture.as_ref()?)))
        .collect()
}

/// The array layer of each textured material (see `textured`).
pub fn layers(scene: &Scene) -> HashMap<MaterialId, u32> {
    textured(scene)
        .into_iter()
        .enumerate()
        .map(|(k, (id, _))| (id, k as u32))
        .collect()
}

/// The side of the array's square layers for `images`: a power of two, enough for the largest
/// image but at most [`MAX_LAYER_SIDE`] and at most `device_max` (the device's largest 2D
/// texture side; an image bigger than the layer is drawn from a scaled copy, the document keeps
/// the original), and small enough that all layers with their mipmaps fit in
/// [`MAX_ARRAY_BYTES`]. 1 without images.
pub fn layer_side(images: &[&TextureImage], device_max: u32) -> u32 {
    let largest = images
        .iter()
        .map(|i| i.width.max(i.height))
        .max()
        .unwrap_or(1);
    // The largest power of two the device allows.
    let device_max = if device_max.is_power_of_two() {
        device_max
    } else {
        (device_max.max(2) / 2).next_power_of_two()
    };
    let mut side = largest
        .next_power_of_two()
        .min(MAX_LAYER_SIDE)
        .min(device_max);
    // A full mip chain adds a third.
    let bytes = |side: u32| images.len() as u64 * u64::from(side).pow(2) * 4 * 4 / 3;
    while side > 1 && bytes(side) > MAX_ARRAY_BYTES {
        side /= 2;
    }
    side
}

/// The source pixels and weights each of `out` pixels along an axis of `len` source pixels
/// takes: the pixels it covers when shrinking, the two nearest when growing. Indices wrap, as
/// the texture repeats.
fn taps(len: u32, out: u32) -> Vec<Vec<(usize, f32)>> {
    let (len, scale) = (len as usize, f64::from(len) / f64::from(out));
    let wrap = |x: f64| x.rem_euclid(len as f64) as usize;
    (0..out)
        .map(|i| {
            if scale > 1.0 {
                let (a, b) = (f64::from(i) * scale, f64::from(i + 1) * scale);
                let mut taps = Vec::new();
                let mut x = a.floor();
                while x < b {
                    let w = (b.min(x + 1.0) - a.max(x)) / scale;
                    if w > 0.0 {
                        taps.push((wrap(x), w as f32));
                    }
                    x += 1.0;
                }
                taps
            } else {
                let c = (f64::from(i) + 0.5) * scale - 0.5;
                let t = c - c.floor();
                vec![
                    (wrap(c.floor()), (1.0 - t) as f32),
                    (wrap(c.floor() + 1.0), t as f32),
                ]
            }
        })
        .collect()
}

/// RGBA8 `pixels` of a `width` x `height` image resampled to `side` x `side` (box filtered when
/// shrinking, linear when growing; both ways separately).
pub fn resample(pixels: &[u8], width: u32, height: u32, side: u32) -> Vec<u8> {
    let (w, s) = (width as usize, side as usize);
    let across = taps(width, side);
    let down = taps(height, side);
    // Rows first: height rows of `side` pixels.
    let mut rows = vec![0f32; height as usize * s * 4];
    for y in 0..height as usize {
        for (x, taps) in across.iter().enumerate() {
            for c in 0..4 {
                rows[(y * s + x) * 4 + c] = taps
                    .iter()
                    .map(|&(k, wt)| f32::from(pixels[(y * w + k) * 4 + c]) * wt)
                    .sum();
            }
        }
    }
    let mut out = vec![0u8; s * s * 4];
    for (y, taps) in down.iter().enumerate() {
        for x in 0..s {
            for c in 0..4 {
                let v: f32 = taps
                    .iter()
                    .map(|&(k, wt)| rows[(k * s + x) * 4 + c] * wt)
                    .sum();
                out[(y * s + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The mipmaps of a square RGBA8 image of `side` pixels (a power of two), the image first, each
/// next level the average of 2 x 2 pixels of the one before, down to 1 x 1.
pub fn mipmaps(image: Vec<u8>, side: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![image];
    let mut side = side as usize;
    while side > 1 {
        let half = side / 2;
        let last = levels.last().expect("the image");
        let mut next = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: usize, dy: usize| {
                        u32::from(last[((2 * y + dy) * side + 2 * x + dx) * 4 + c])
                    };
                    next[(y * half + x) * 4 + c] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
        levels.push(next);
        side = half;
    }
    levels
}

/// The scene's textures uploaded as one array, and which images it holds.
pub(crate) struct TextureArray {
    /// Each layer's material and image, to tell when the scene's textures change.
    held: Vec<(MaterialId, Arc<TextureImage>)>,
    pub view: wgpu::TextureView,
}

impl TextureArray {
    /// True when the array holds exactly `scene`'s textures, layer for layer.
    pub fn holds(&self, scene: &Scene) -> bool {
        let now = textured(scene);
        now.len() == self.held.len()
            && now
                .iter()
                .zip(&self.held)
                .all(|((id, t), (held, image))| id == held && Arc::ptr_eq(&t.image, image))
    }

    /// Uploads `scene`'s textures (a single black 1 x 1 layer when there are none, as the
    /// shader always binds an array).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) -> Self {
        let textured = textured(scene);
        let images: Vec<&TextureImage> = textured.iter().map(|(_, t)| &*t.image).collect();
        let side = layer_side(&images, device.limits().max_texture_dimension_2d);
        let levels = side.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("material textures"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                // At least two layers: WebGL2 cannot view a one-layer array as a 2D array.
                depth_or_array_layers: images.len().max(2) as u32,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Colors stay sRGB values, as the vertex colors and render targets hold them.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, image) in images.iter().enumerate() {
            let pixels = resample(&image.pixels, image.width, image.height, side);
            for (level, data) in mipmaps(pixels, side).iter().enumerate() {
                let size = (side >> level).max(1);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * size),
                        rows_per_image: Some(size),
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        TextureArray {
            held: textured
                .into_iter()
                .map(|(id, t)| (id, t.image.clone()))
                .collect(),
            view,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> TextureImage {
        TextureImage {
            format: kernel::ImageFormat::Png,
            encoded: Vec::new(),
            width,
            height,
            pixels: vec![0; (4 * width * height) as usize],
        }
    }

    #[test]
    fn layers_fit_the_largest_image_and_the_memory_budget() {
        const DEVICE: u32 = 16384;
        assert_eq!(layer_side(&[], DEVICE), 1);
        assert_eq!(layer_side(&[&image(3, 2)], DEVICE), 4);
        assert_eq!(layer_side(&[&image(300, 20), &image(64, 64)], DEVICE), 512);
        assert_eq!(layer_side(&[&image(4096, 4096)], DEVICE), MAX_LAYER_SIDE);
        // 256 layers of 1024 would take 1.3 GiB: they shrink to 256.
        let big = image(1024, 1024);
        let many: Vec<&TextureImage> = (0..256).map(|_| &big).collect();
        assert_eq!(layer_side(&many, DEVICE), 256);
    }

    #[test]
    fn a_small_device_gets_layers_it_can_hold() {
        // A 8192 x 8192 picture on a device with 1024 texels a side (or an odd limit): the
        // layer is the largest power of two within the limit.
        let big = image(8192, 8192);
        assert_eq!(layer_side(&[&big], 1024), 1024);
        assert_eq!(layer_side(&[&big], 2048), 2048);
        assert_eq!(layer_side(&[&big], 3000), 2048);
        assert_eq!(layer_side(&[&image(100, 100)], 1024), 128);
        assert_eq!(layer_side(&[&big], 0), 1);
    }

    #[test]
    fn resampling_averages_when_shrinking_and_blends_when_growing() {
        // Two columns, black and white, shrink to one gray pixel.
        let pixels = [0, 0, 0, 255, 255, 255, 255, 255];
        assert_eq!(resample(&pixels, 2, 1, 1), [128, 128, 128, 255]);
        // Grown to 4 x 4: each row runs black, blends, white, blends (wrapping round).
        let out = resample(&pixels, 2, 1, 4);
        let reds: Vec<u8> = out.as_chunks::<4>().0[..4].iter().map(|p| p[0]).collect();
        assert_eq!(reds, [64, 64, 191, 191]);
        assert!(out.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // Same size: unchanged.
        let square: Vec<u8> = (0..16 * 4).map(|k| k as u8).collect();
        assert_eq!(resample(&square, 4, 4, 4), square);
    }

    #[test]
    fn mipmaps_halve_down_to_one_pixel() {
        let mut image = vec![0u8; 4 * 4 * 4];
        image[0] = 200; // one red pixel in the corner
        let levels = mipmaps(image, 4);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), [64, 16, 4]);
        assert_eq!(levels[1][0], 50);
        assert_eq!(levels[2][0], 13);
    }
}
