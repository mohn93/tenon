use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, Model, Scene};
use render::grid::{BACKGROUND, LineVertex};
use render::{Renderer, Viewport};
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
    render_scene(camera, &Model::new(), &[])
}

fn render_scene(camera: &Camera, model: &Model, overlay: &[LineVertex]) -> Option<Vec<u8>> {
    render_with(
        camera,
        model,
        &BTreeSet::new(),
        Viewport::full((W, H)),
        overlay,
    )
}

/// Renders `model` with `selection` highlighted into `viewport` of a W x H target.
fn render_with(
    camera: &Camera,
    model: &Model,
    selection: &BTreeSet<EntityId>,
    viewport: Viewport,
    overlay: &[LineVertex],
) -> Option<Vec<u8>> {
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
    renderer.set_scene(&Scene::from_root(model.clone()), &[], selection);
    renderer.render_in(&view, (W, H), viewport, camera, overlay);

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

/// A 4 m square face on the ground, centered on the origin.
fn ground_square() -> Model {
    let mut m = Model::new();
    let c = [(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        m.add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    m
}

fn pixel_at(px: &[u8], cam: &Camera, p: DVec3) -> [u8; 4] {
    let s = cam.project(p).unwrap();
    pixel(px, s.x as u32, s.y as u32)
}

/// Whether any pixel within one pixel of the projection of `p` satisfies `f`.
fn near(px: &[u8], cam: &Camera, p: DVec3, f: impl Fn([u8; 4]) -> bool) -> bool {
    let s = cam.project(p).unwrap();
    (-1..=1).any(|dy| {
        (-1..=1).any(|dx| {
            let x = s.x as i32 + dx;
            let y = s.y as i32 + dy;
            if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
                return false;
            }
            f(pixel(px, x as u32, y as u32))
        })
    })
}

fn grayish(p: [u8; 4]) -> bool {
    let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
    (r - g).abs() < 12 && (g - b).abs() < 16
}

#[test]
fn face_front_side_hides_the_grid() {
    let cam = camera();
    let Some(px) = render_scene(&cam, &ground_square(), &[]) else {
        return;
    };
    // A point inside the face away from edges and grid lines.
    let p = pixel_at(&px, &cam, DVec3::new(0.5, 0.5, 0.0));
    assert!(grayish(p), "front color should be light gray, got {p:?}");
    assert!(p[0] < 225 && p[0] > 120, "face shading out of range: {p:?}");
    // The grid line x = 1 runs under the face and must not show through.
    let under = pixel_at(&px, &cam, DVec3::new(1.0, 0.5, 0.0));
    assert!(
        (under[0] as i32 - p[0] as i32).abs() < 8,
        "grid shows through the face: {under:?} vs {p:?}"
    );
}

#[test]
fn face_back_side_is_blue_gray() {
    let mut cam = camera();
    cam.eye = DVec3::new(-8.0, -12.0, -7.0);
    let Some(px) = render_scene(&cam, &ground_square(), &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, DVec3::new(0.5, 0.5, 0.0));
    assert!(
        p[2] as i32 > p[0] as i32 + 15,
        "back side should be blue-gray, got {p:?}"
    );
}

#[test]
fn edges_draw_over_their_face() {
    let cam = camera();
    let Some(px) = render_scene(&cam, &ground_square(), &[]) else {
        return;
    };
    let dark = |p: [u8; 4]| p[0] < 110 && p[1] < 110 && p[2] < 110;
    assert!(
        near(&px, &cam, DVec3::new(2.0, 0.5, 0.0), dark),
        "edge x = 2 not visible"
    );
    assert!(
        near(&px, &cam, DVec3::new(0.5, -2.0, 0.0), dark),
        "edge y = -2 not visible"
    );
}

#[test]
fn overlay_draws_on_top_of_faces() {
    let cam = camera();
    let magenta = [1.0, 0.0, 1.0, 1.0];
    // A segment just below the face plane: depth-tested lines would be hidden.
    let overlay = [
        LineVertex {
            pos: [-1.0, 0.5, -0.05],
            color: magenta,
        },
        LineVertex {
            pos: [1.5, 0.5, -0.05],
            color: magenta,
        },
    ];
    let Some(px) = render_scene(&cam, &ground_square(), &overlay) else {
        return;
    };
    let is_magenta = |p: [u8; 4]| p[0] > 150 && p[2] > 150 && p[1] < 120;
    assert!(
        near(&px, &cam, DVec3::new(0.25, 0.5, -0.05), is_magenta),
        "overlay hidden by face"
    );
}

#[test]
fn interior_edge_draws_over_faces_on_both_sides() {
    let cam = camera();
    // The diagonal splits the square face into two faces that share it.
    let mut model = ground_square();
    model
        .add_edge(DVec3::new(-2.0, -2.0, 0.0), DVec3::new(2.0, 2.0, 0.0))
        .unwrap();
    let Some(px) = render_scene(&cam, &model, &[]) else {
        return;
    };
    let dark = |p: [u8; 4]| p[0] < 110 && p[1] < 110 && p[2] < 110;
    assert!(
        near(&px, &cam, DVec3::new(0.5, 0.5, 0.0), dark),
        "interior edge not visible"
    );
}

#[test]
fn edge_well_behind_a_face_is_hidden() {
    let cam = camera();
    // A free edge half a meter below the face, under its middle.
    let mut model = ground_square();
    model
        .add_edge(DVec3::new(-1.0, 0.5, -0.5), DVec3::new(1.0, 0.5, -0.5))
        .unwrap();
    let Some(px) = render_scene(&cam, &model, &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, DVec3::new(0.0, 0.5, -0.5));
    assert!(p[0] > 120, "edge behind the face shows through: {p:?}");
}

#[test]
fn grid_hidden_under_ground_face_at_grazing_angle() {
    let mut cam = camera();
    cam.eye = DVec3::new(0.0, -15.0, 1.6);
    cam.target = DVec3::ZERO;
    let mut model = Model::new();
    let c = [
        (-150.0, -150.0),
        (150.0, -150.0),
        (150.0, 150.0),
        (-150.0, 150.0),
    ];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        model
            .add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let Some(px) = render_scene(&cam, &model, &[]) else {
        return;
    };
    let x = W / 2;
    let reds: Vec<i32> = (H / 2..H).map(|y| pixel(&px, x, y)[0] as i32).collect();
    // Keep only rows that land on the face (not the sky or the far edge).
    let face: Vec<i32> = reds.iter().copied().filter(|&r| r > 150).collect();
    assert!(
        face.len() > 50,
        "face covers too few pixels: {}",
        face.len()
    );
    // Headlight shading varies smoothly down the column (about 1 level per row), while a grid
    // line would be a sharp step of more than 10 levels, so the largest neighbour step is the test.
    let step = face.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap();
    assert!(
        step < 4,
        "grid stripes show through the face: largest step {step}"
    );
}

#[test]
fn selected_faces_are_tinted_blue() {
    let cam = camera();
    let model = ground_square();
    let face = model.faces().next().unwrap().0;
    let selection = BTreeSet::from([EntityId::Face(face)]);
    let Some(px) = render_with(&cam, &model, &selection, Viewport::full((W, H)), &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, DVec3::new(0.5, 0.5, 0.0));
    assert!(
        p[2] as i32 > p[0] as i32 + 30,
        "selected face should be blue, got {p:?}"
    );
}

#[test]
fn scene_stays_inside_its_viewport() {
    // The left half holds the scene, sized to match; the right half is only background.
    let mut cam = camera();
    cam.viewport = DVec2::new((W / 2) as f64, H as f64);
    let viewport = Viewport {
        x: 0,
        y: 0,
        width: W / 2,
        height: H,
    };
    let Some(px) = render_with(&cam, &ground_square(), &BTreeSet::new(), viewport, &[]) else {
        return;
    };
    let expected = BACKGROUND.map(|c| (c * 255.0).round() as i32);
    for y in (0..H).step_by(7) {
        for x in (W / 2..W).step_by(5) {
            let p = pixel(&px, x, y);
            assert!(
                (0..3).all(|ch| (p[ch] as i32 - expected[ch]).abs() <= 1),
                "pixel {x},{y} = {p:?} outside the viewport"
            );
        }
    }
    let face = cam.project(DVec3::new(0.5, 0.5, 0.0)).unwrap();
    assert!(face.x < (W / 2) as f64);
    assert!(grayish(pixel(&px, face.x as u32, face.y as u32)));
}

#[test]
fn a_guide_line_is_drawn_dashed_across_the_view() {
    let c = camera();
    let mut m = Model::new();
    // A horizontal guide line across the view through the target, in front of the background.
    m.add_guide(kernel::Guide::Line {
        point: c.target,
        direction: c.right(),
    })
    .unwrap();
    let Some(pixels) = render_scene(&c, &m, &[]) else {
        return;
    };
    // The line runs along the boundary between the two middle rows, so multisampling shares
    // it between them: a column is part of a dash when either row is clearly darkened.
    let dark = |x: u32| (H / 2 - 1..=H / 2).any(|y| pixel(&pixels, x, y)[0] < 200);
    let along: Vec<bool> = (0..W).map(dark).collect();
    let lit = along.iter().filter(|b| **b).count();
    // Dashes and gaps of about 8 px: roughly half the row, broken many times.
    assert!(
        lit > W as usize / 4 && lit < W as usize * 3 / 4,
        "{lit} guide pixels"
    );
    let breaks = along.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(breaks > 20, "{breaks} dash ends");
}
