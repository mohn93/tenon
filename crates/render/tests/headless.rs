use glam::{DVec2, DVec3};
use render::{Renderer, grid::BACKGROUND};
use session::Camera;

const W: u32 = 256; // W * 4 bytes must be a multiple of 256 for texture readback
const H: u32 = 256;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// Renders one frame and returns tightly packed RGBA8 pixels, or None without a GPU.
fn render(camera: &Camera) -> Option<Vec<u8>> {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = Renderer::new(&device, &queue, format);
    renderer.render(&view, (W, H), camera);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (W * H * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    let pixels = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped range")
        .to_vec();
    Some(pixels)
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

fn camera() -> Camera {
    Camera::new(DVec2::new(W as f64, H as f64))
}

#[test]
fn sky_is_cleared_to_background() {
    let mut cam = camera();
    // Looking horizontally: the top row sees only sky (except the centre column, where the Z axis climbs).
    cam.eye = DVec3::new(0.0, -10.0, 1.0);
    cam.target = DVec3::new(0.0, 0.0, 1.0);
    let Some(px) = render(&cam) else { return };
    let expected = BACKGROUND.map(|c| (c * 255.0).round() as i32);
    for x in [0, W / 4, 3 * W / 4, W - 1] {
        let p = pixel(&px, x, 0);
        for ch in 0..3 {
            assert!(
                (p[ch] as i32 - expected[ch]).abs() <= 1,
                "pixel {x},0 = {p:?}"
            );
        }
    }
}

#[test]
fn all_three_axes_are_visible() {
    let Some(px) = render(&camera()) else { return };
    let any = |f: &dyn Fn([u8; 4]) -> bool| px.chunks(4).any(|c| f([c[0], c[1], c[2], c[3]]));
    let dominant = |p: [u8; 4], i: usize| {
        let (a, b) = match i {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        p[i] as i32 > p[a] as i32 + 40 && p[i] as i32 > p[b] as i32 + 40
    };
    assert!(any(&|p| dominant(p, 0)), "no red axis pixels");
    assert!(any(&|p| dominant(p, 1)), "no green axis pixels");
    assert!(any(&|p| dominant(p, 2)), "no blue axis pixels");
}
