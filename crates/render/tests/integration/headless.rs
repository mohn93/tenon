use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{DefinitionKind, EntityId, InstanceId, MeshId, Model, Scene, Side};
use render::grid::{BACKGROUND, LineVertex};
use render::{Renderer, ViewStyle, Viewport};
use session::{Camera, Lighting};

const W: u32 = 256; // W * 4 bytes must be a multiple of 256 for texture readback
const H: u32 = 256;

pub(crate) fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: render::required_limits(&adapter),
        ..Default::default()
    }))
    .ok()
}

/// True when the default adapter is a software rasterizer (Mesa lavapipe, WARP).
fn software_adapter() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .is_ok_and(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
}

/// True when the default adapter is Windows' WARP software rasterizer (matched by name, whatever
/// device type the backend reports for it).
fn warp_adapter() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok_and(
        |a| {
            let name = a.get_info().name;
            name.contains("Basic Render Driver") || name.contains("WARP")
        },
    )
}

/// Skips a pixel check that differs on WARP (seen on the Windows CI runner); it passes on Mesa
/// lavapipe and real GPUs, which still run it.
fn skip_on_warp(what: &str) -> bool {
    let warp = warp_adapter();
    if warp {
        eprintln!("WARP adapter, skipping {what}");
    }
    warp
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
    let scene = Scene::from_root(model.clone());
    render_scene_with(camera, &scene, selection, viewport, overlay)
}

/// Renders a whole scene (with its materials) with `selection` highlighted into `viewport`.
fn render_scene_with(
    camera: &Camera,
    scene: &Scene,
    selection: &BTreeSet<EntityId>,
    viewport: Viewport,
    overlay: &[LineVertex],
) -> Option<Vec<u8>> {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let mut renderer = screen.renderer();
    renderer.set_scene(scene, &[], selection);
    Some(screen.draw(&mut renderer, camera, viewport, overlay))
}

/// A W x H texture to render into and read back.
pub(crate) struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Offscreen {
    /// None without a GPU.
    pub(crate) fn new() -> Option<Self> {
        let (device, queue) = gpu()?;
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
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some(Self {
            device,
            queue,
            texture,
            view,
        })
    }

    pub(crate) fn renderer(&self) -> Renderer {
        Renderer::new(&self.device, &self.queue, FORMAT)
    }

    /// Draws a frame with `renderer` and returns tightly packed RGBA8 pixels.
    fn draw(
        &self,
        renderer: &mut Renderer,
        camera: &Camera,
        viewport: Viewport,
        overlay: &[LineVertex],
    ) -> Vec<u8> {
        let (device, queue, texture) = (&self.device, &self.queue, &self.texture);
        renderer.render_in(&self.view, (W, H), viewport, camera, overlay);

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
                texture,
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
        buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped range")
            .to_vec()
    }
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
    // Kept for inspection (CI uploads target/tmp images when a test fails).
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("headless/axes.png");
    crate::golden::write_png(&out, &px, W, H);
    if software_adapter() {
        // Mesa lavapipe draws the far ground grid as wide bands that hide the Z axis above
        // the horizon (seen in CI); real GPUs draw it as here. Not yet explained.
        eprintln!("software GPU adapter, skipping the axes check");
        return;
    }
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
fn painted_sides_show_their_colors_and_selection_still_tints() {
    let mut scene = Scene::from_root(ground_square());
    let (red, _) = scene.add_material("Red", [220, 20, 20]).unwrap();
    let (green, _) = scene.add_material("Green", [20, 200, 20]).unwrap();
    let face = scene.root().faces().next().unwrap().0;
    scene
        .paint(MeshId::Root, &[face], &[], Side::Front, Some(red))
        .unwrap();
    scene
        .paint(MeshId::Root, &[face], &[], Side::Back, Some(green))
        .unwrap();
    let cam = camera();
    let at = DVec3::new(0.5, 0.5, 0.0);
    let full = Viewport::full((W, H));
    let Some(px) = render_scene_with(&cam, &scene, &BTreeSet::new(), full, &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, at);
    assert!(
        p[0] as i32 > p[1] as i32 + 80,
        "front should be red, got {p:?}"
    );
    let mut below = camera();
    below.eye = DVec3::new(-8.0, -12.0, -7.0);
    let px = render_scene_with(&below, &scene, &BTreeSet::new(), full, &[]).unwrap();
    let p = pixel_at(&px, &below, at);
    assert!(
        p[1] as i32 > p[0] as i32 + 80,
        "back should be green, got {p:?}"
    );
    let selection = BTreeSet::from([EntityId::Face(face)]);
    let px = render_scene_with(&cam, &scene, &selection, full, &[]).unwrap();
    let p = pixel_at(&px, &cam, at);
    assert!(
        p[2] as i32 > 60 && (p[2] as i32) > p[1] as i32,
        "selected red face should be tinted blue, got {p:?}"
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
        tag: None,
        hidden: false,
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

/// A parallel view from the front (looking along +Y at the origin, 10 m away).
fn parallel_front() -> Camera {
    let mut cam = camera();
    cam.eye = DVec3::new(0.0, -10.0, 0.0);
    cam.target = DVec3::ZERO;
    cam.projection = session::Projection::Parallel;
    cam
}

/// A 4 m square upright face across the view at `y`.
fn upright_square(m: &mut Model, y: f64) {
    let c = [(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        m.add_edge(DVec3::new(a.0, y, a.1), DVec3::new(b.0, y, b.1))
            .unwrap();
    }
}

#[test]
fn a_parallel_view_hides_an_edge_just_behind_a_face() {
    let cam = parallel_front();
    let mut model = Model::new();
    upright_square(&mut model, -0.3);
    model
        .add_edge(DVec3::new(-1.0, 0.0, 0.5), DVec3::new(1.0, 0.0, 0.5))
        .unwrap();
    let Some(px) = render_scene(&cam, &model, &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, DVec3::new(0.0, 0.0, 0.5));
    assert!(p[0] > 120, "edge behind the face shows through: {p:?}");
    let dark = |p: [u8; 4]| p[0] < 110 && p[1] < 110 && p[2] < 110;
    assert!(
        near(&px, &cam, DVec3::new(2.0, -0.3, 0.5), dark),
        "the face's own edge is drawn"
    );
}

#[test]
fn a_parallel_view_lights_faces_behind_the_eye_head_on() {
    let cam = parallel_front();
    let mut model = Model::new();
    upright_square(&mut model, -12.0);
    let Some(px) = render_scene(&cam, &model, &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, DVec3::new(0.5, -12.0, 0.7));
    assert!(
        p[0].max(p[1]).max(p[2]) > 180,
        "a face square to the view is fully lit: {p:?}"
    );
}

/// A diamond on the ground made a component (its edges run across its bounding box, so the
/// open instance's box does not cover them), placed again 1 m higher and turned a little.
/// Returns the scene and both instances.
fn stacked_diamonds() -> (Scene, InstanceId, InstanceId) {
    let mut m = Model::new();
    let c = [(2.0, 0.0), (0.0, 2.0), (-2.0, 0.0), (0.0, -2.0)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        m.add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (low, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Tile",
        )
        .unwrap();
    s.copy_instances(MeshId::Root, &[low], DVec3::new(0.5, 0.5, 1.0), 1)
        .unwrap();
    let high = s
        .root()
        .instances()
        .map(|(id, _)| id)
        .find(|id| *id != low)
        .unwrap();
    let turn = glam::DAffine3::from_rotation_z(0.2);
    s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[high], turn)))
        .unwrap();
    (s, low, high)
}

#[test]
fn changing_only_the_selection_draws_what_a_rebuild_draws() {
    let (scene, low, high) = stacked_diamonds();
    let def = MeshId::Definition(scene.root().instance(low).unwrap().definition);
    let tile = scene.mesh(def).unwrap();
    let face = EntityId::Face(tile.faces().next().unwrap().0);
    let edges: Vec<EntityId> = tile.edges().map(|(id, _)| EntityId::Edge(id)).collect();
    let set = |ids: &[EntityId]| ids.iter().copied().collect::<BTreeSet<_>>();
    let (lows, highs) = (EntityId::Instance(low), EntityId::Instance(high));
    let cases = [
        (
            vec![],
            vec![
                set(&[lows]),
                set(&[]),
                set(&[highs]),
                set(&[lows, highs]),
                set(&[]),
            ],
        ),
        (
            vec![high],
            vec![
                set(&[face]),
                set(&[edges[0], edges[2]]),
                set(&[]),
                set(&[face, edges[1], edges[3]]),
            ],
        ),
    ];
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut cam = camera();
    cam.eye = DVec3::new(6.0, -8.0, 7.0);
    let full = Viewport::full((W, H));
    let mut kept = screen.renderer();
    for (context, selections) in &cases {
        kept.set_model(&scene, context);
        for selection in selections {
            kept.set_selection(selection);
            let changed = screen.draw(&mut kept, &cam, full, &[]);
            let mut fresh = screen.renderer();
            fresh.set_scene(&scene, context, selection);
            let rebuilt = screen.draw(&mut fresh, &cam, full, &[]);
            assert!(
                changed == rebuilt,
                "context {context:?}, selection {selection:?}: the frames differ"
            );
        }
    }
    // The flags show on their own: selected edges turn blue and a selected face is tinted.
    kept.set_model(&scene, &[high]);
    let plain = screen.draw(&mut kept, &cam, full, &[]);
    kept.set_selection(&set(&edges));
    let edges_lit = screen.draw(&mut kept, &cam, full, &[]);
    kept.set_selection(&set(&[face]));
    let face_lit = screen.draw(&mut kept, &cam, full, &[]);
    // Pixels that turned bluer (blue less red up by at least `by`) than in `plain`.
    let bluer = |px: &[u8], by: i32| {
        let blue = |p: &[u8]| i32::from(p[2]) - i32::from(p[0]);
        px.chunks(4)
            .zip(plain.chunks(4))
            .filter(|(p, q)| blue(p) >= blue(q) + by)
            .count()
    };
    let n = bluer(&edges_lit, 60);
    assert!(n > 100, "selected edges turn blue: {n} pixels");
    let n = bluer(&face_lit, 40);
    assert!(n > 2000, "a selected face is tinted: {n} pixels");
}

/// Looking down from high above the origin (from below for a negative `z`).
fn overhead(z: f64) -> Camera {
    let mut cam = camera();
    cam.eye = DVec3::new(0.3, -0.5, z);
    cam.target = DVec3::ZERO;
    cam
}

/// `ground_square` with a 2 x 2 pane 1 m above it; returns the scene, the ground face and the
/// pane.
fn square_under_pane() -> (Scene, kernel::FaceId, kernel::FaceId) {
    let mut m = ground_square();
    let ground = m.faces().next().unwrap().0;
    m.add_polygon(&[
        DVec3::new(-1.0, -1.0, 1.0),
        DVec3::new(1.0, -1.0, 1.0),
        DVec3::new(1.0, 1.0, 1.0),
        DVec3::new(-1.0, 1.0, 1.0),
    ])
    .unwrap();
    let pane = m
        .faces()
        .map(|(id, _)| id)
        .find(|id| *id != ground)
        .unwrap();
    (Scene::from_root(m), ground, pane)
}

#[test]
fn glass_shows_what_is_behind_it_tinted_and_its_frame() {
    let (mut scene, ground, pane) = square_under_pane();
    let (red, _) = scene.add_material("Red", [220, 20, 20]).unwrap();
    let (glass, _) = scene
        .add_material_with_opacity("Glass", [40, 40, 240], 0.5)
        .unwrap();
    scene
        .paint(MeshId::Root, &[ground], &[], Side::Both, Some(red))
        .unwrap();
    let cam = overhead(12.0);
    let full = Viewport::full((W, H));
    // Through the pane, at a point of the ground it covers (away from the blue axis).
    let through = DVec3::new(0.5, 0.4, 1.0);
    let Some(bare) = render_scene_with(&cam, &scene, &BTreeSet::new(), full, &[]) else {
        return;
    };
    let opaque_pane = pixel_at(&bare, &cam, through);
    assert!(
        opaque_pane[0] > 200,
        "an unpainted pane is light: {opaque_pane:?}"
    );
    scene
        .paint(MeshId::Root, &[pane], &[], Side::Both, Some(glass))
        .unwrap();
    let px = render_scene_with(&cam, &scene, &BTreeSet::new(), full, &[]).unwrap();
    let p = pixel_at(&px, &cam, through);
    assert!(
        p[0] as i32 > 60 && p[2] as i32 > 60 && (p[1] as i32) < 60,
        "red ground through blue glass is purple, got {p:?}"
    );
    // The frame still draws dark over the ground behind it.
    let corner = DVec3::new(0.0, -1.0, 1.0);
    assert!(
        near(&px, &cam, corner, |p| p[0] < 90 && p[1] < 90 && p[2] < 90),
        "the pane's edge should stay dark"
    );
    // Invisible glass shows the ground as it is.
    let id = scene
        .materials()
        .find(|(_, m)| m.name == "Glass")
        .unwrap()
        .0;
    scene.change_material(id, None, None, Some(0.0)).unwrap();
    let px = render_scene_with(&cam, &scene, &BTreeSet::new(), full, &[]).unwrap();
    let seen = pixel_at(&px, &cam, through);
    let ground_only = pixel_at(&px, &cam, DVec3::new(0.5, 0.4, 0.0));
    assert!(
        seen[0] > 150 && seen[1] < 60,
        "got {seen:?} over {ground_only:?}"
    );
}

#[test]
fn each_side_of_a_pane_has_its_own_opacity_and_selected_glass_is_tinted() {
    let (mut scene, ground, pane) = square_under_pane();
    let (red, _) = scene.add_material("Red", [220, 20, 20]).unwrap();
    let (green, _) = scene.add_material("Green", [20, 200, 20]).unwrap();
    let (glass, _) = scene
        .add_material_with_opacity("Glass", [235, 235, 235], 0.3)
        .unwrap();
    scene
        .paint(MeshId::Root, &[ground], &[], Side::Both, Some(red))
        .unwrap();
    // The pane faces up: see-through from above, opaque green from below.
    scene
        .paint(MeshId::Root, &[pane], &[], Side::Front, Some(glass))
        .unwrap();
    scene
        .paint(MeshId::Root, &[pane], &[], Side::Back, Some(green))
        .unwrap();
    assert!(scene.root().face(pane).unwrap().normal.z > 0.5);
    let cam = overhead(12.0);
    let full = Viewport::full((W, H));
    let through = DVec3::new(0.5, 0.4, 1.0);
    let Some(px) = render_scene_with(&cam, &scene, &BTreeSet::new(), full, &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, through);
    assert!(
        p[0] as i32 > p[1] as i32 + 60,
        "from above the red ground shows through: {p:?}"
    );
    // From below, with the ground hidden, the pane's back is opaque green.
    let below = overhead(-12.0);
    let mut only_pane = scene.clone();
    only_pane
        .edit(MeshId::Root, |m| Ok(m.erase(&[], &[ground])))
        .unwrap();
    let px = render_scene_with(&below, &only_pane, &BTreeSet::new(), full, &[]).unwrap();
    let p = pixel_at(&px, &below, through);
    assert!(
        p[1] as i32 > p[0] as i32 + 60 && p[1] as i32 > p[2] as i32 + 60,
        "from below the back is green: {p:?}"
    );
    // Selected, the glass is tinted blue.
    let selection = BTreeSet::from([EntityId::Face(pane)]);
    let plain = render_scene_with(&cam, &only_pane, &BTreeSet::new(), full, &[]).unwrap();
    let lit = render_scene_with(&cam, &only_pane, &selection, full, &[]).unwrap();
    let (p, q) = (
        pixel_at(&plain, &cam, through),
        pixel_at(&lit, &cam, through),
    );
    assert!(
        (q[2] as i32 - q[0] as i32) > (p[2] as i32 - p[0] as i32) + 30,
        "selection should tint the glass: {p:?} -> {q:?}"
    );
}

/// A 10 m floor, an upright wall 4 m wide and 2 m tall along y at `wall_x`, and a lamp: a 0.6 m
/// square glowing in front, 1.6 m up at x = `lamp_x`.
fn lamp_and_wall(wall_x: f64, lamp_x: f64) -> Scene {
    let p = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
    let mut m = Model::new();
    let floor = [(-5.0, -5.0), (5.0, -5.0), (5.0, 5.0), (-5.0, 5.0)];
    m.add_polygon(&floor.map(|(x, y)| p(x, y, 0.0))).unwrap();
    let wall = [(-2.0, 0.0), (2.0, 0.0), (2.0, 2.0), (-2.0, 2.0)];
    m.add_polygon(&wall.map(|(y, z)| p(wall_x, y, z))).unwrap();
    let before: Vec<_> = m.faces().map(|(id, _)| id).collect();
    let shade = [(-0.3, -0.3), (0.3, -0.3), (0.3, 0.3), (-0.3, 0.3)];
    m.add_polygon(&shade.map(|(x, y)| p(lamp_x + x, y, 1.6)))
        .unwrap();
    let lamp = m
        .faces()
        .map(|(id, _)| id)
        .find(|id| !before.contains(id))
        .unwrap();
    let mut s = Scene::from_root(m);
    let (glow, _) = s
        .add_material_from(kernel::Material {
            name: "Lamp".into(),
            color: [255, 255, 255],
            opacity: 1.0,
            texture: None,
            emission: 6.0,
        })
        .unwrap();
    s.paint(MeshId::Root, &[lamp], &[], Side::Front, Some(glow))
        .unwrap();
    s
}

fn brightness(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

#[test]
fn a_wall_shades_the_floor_behind_it_and_shadow_maps_are_kept_between_frames() {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = screen.renderer();
    if !r.casts_shadows() {
        eprintln!("no cube-map arrays, skipping");
        return;
    }
    let cam = overhead(16.0);
    let full = Viewport::full((W, H));
    r.set_lighting(Lighting::Night);
    r.set_scene(&lamp_and_wall(0.0, -1.5), &[], &BTreeSet::new());
    let px = screen.draw(&mut r, &cam, full, &[]);
    // Two floor points as far from the lamp, one in the open and one behind the wall.
    let open = brightness(pixel_at(&px, &cam, DVec3::new(-4.5, 0.0, 0.0)));
    let behind = brightness(pixel_at(&px, &cam, DVec3::new(1.5, 0.0, 0.0)));
    assert!(open > behind + 40, "open {open}, behind the wall {behind}");
    // One light: its six faces drawn once, then kept while the camera moves and while the
    // same scene is set again.
    assert_eq!(r.shadow_faces_drawn(), 6);
    let mut moved = cam.clone();
    moved.eye.x += 1.0;
    screen.draw(&mut r, &moved, full, &[]);
    r.set_scene(&lamp_and_wall(0.0, -1.5), &[], &BTreeSet::new());
    screen.draw(&mut r, &cam, full, &[]);
    assert_eq!(r.shadow_faces_drawn(), 6);
    // The wall made a group: the same faces in the same places keep the map. Opening the group,
    // selecting it and painting the floor with a plain color keep it too.
    let mut grouped = lamp_and_wall(0.0, -1.5);
    let ids: Vec<_> = grouped.root().faces().map(|(id, _)| id).collect();
    let (floor, wall) = (ids[0], ids[1]);
    let (group, _) = grouped
        .make_group(
            MeshId::Root,
            &[],
            &[wall],
            &[],
            DefinitionKind::Group,
            "Wall",
        )
        .unwrap();
    r.set_scene(&grouped, &[], &BTreeSet::new());
    screen.draw(&mut r, &cam, full, &[]);
    let kept = r.shadow_faces_drawn();
    r.set_model(&grouped, &[group]);
    screen.draw(&mut r, &cam, full, &[]);
    assert_eq!(r.shadow_faces_drawn(), kept, "opening a group");
    r.set_selection(&BTreeSet::from([EntityId::Face(wall)]));
    screen.draw(&mut r, &cam, full, &[]);
    assert_eq!(r.shadow_faces_drawn(), kept, "selecting");
    let (red, _) = grouped.add_material("Red", [220, 20, 20]).unwrap();
    grouped
        .paint(MeshId::Root, &[floor], &[], Side::Front, Some(red))
        .unwrap();
    r.set_model(&grouped, &[group]);
    screen.draw(&mut r, &cam, full, &[]);
    assert_eq!(r.shadow_faces_drawn(), kept, "painting a plain color");
    // The wall moving or the lamp moving draws the map again.
    r.set_scene(&lamp_and_wall(0.5, -1.5), &[], &BTreeSet::new());
    screen.draw(&mut r, &cam, full, &[]);
    assert_eq!(r.shadow_faces_drawn(), kept + 6);
    r.set_scene(&lamp_and_wall(0.5, -1.0), &[], &BTreeSet::new());
    screen.draw(&mut r, &cam, full, &[]);
    assert_eq!(r.shadow_faces_drawn(), kept + 12);
}

/// A 10 m floor, an upright wall 4 m wide and 2 m tall along y at x = 0, and a lamp mounted on
/// it: a 0.6 m upright glowing square `gap` meters in front of the wall (at -x), 1 m up.
fn wall_lamp(gap: f64) -> Scene {
    let p = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
    let mut m = Model::new();
    let floor = [(-5.0, -5.0), (5.0, -5.0), (5.0, 5.0), (-5.0, 5.0)];
    m.add_polygon(&floor.map(|(x, y)| p(x, y, 0.0))).unwrap();
    let wall = [(-2.0, 0.0), (2.0, 0.0), (2.0, 2.0), (-2.0, 2.0)];
    m.add_polygon(&wall.map(|(y, z)| p(0.0, y, z))).unwrap();
    let before: Vec<_> = m.faces().map(|(id, _)| id).collect();
    let shade = [(-0.3, 0.7), (0.3, 0.7), (0.3, 1.3), (-0.3, 1.3)];
    m.add_polygon(&shade.map(|(y, z)| p(-gap, y, z))).unwrap();
    let lamp = m
        .faces()
        .map(|(id, _)| id)
        .find(|id| !before.contains(id))
        .unwrap();
    let mut s = Scene::from_root(m);
    let (glow, _) = s
        .add_material_from(kernel::Material {
            name: "Lamp".into(),
            color: [255, 255, 255],
            opacity: 1.0,
            texture: None,
            emission: 6.0,
        })
        .unwrap();
    for side in [Side::Front, Side::Back] {
        s.paint(MeshId::Root, &[lamp], &[], side, Some(glow))
            .unwrap();
    }
    s
}

#[test]
fn a_lamp_a_centimeter_off_a_wall_does_not_light_through_it() {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = screen.renderer();
    if !r.casts_shadows() {
        eprintln!("no cube-map arrays, skipping");
        return;
    }
    let cam = overhead(16.0);
    r.set_lighting(Lighting::Night);
    r.set_scene(&wall_lamp(0.01), &[], &BTreeSet::new());
    let px = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    let front = brightness(pixel_at(&px, &cam, DVec3::new(-1.5, 0.0, 0.0)));
    let behind = brightness(pixel_at(&px, &cam, DVec3::new(1.5, 0.0, 0.0)));
    assert!(
        front > behind + 40,
        "front {front}, behind the wall {behind}"
    );
}

#[test]
fn without_shadows_the_lamp_lights_through_the_wall() {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = Renderer::without_shadows(&screen.device, &screen.queue, FORMAT);
    assert!(!r.casts_shadows());
    let cam = overhead(16.0);
    r.set_lighting(Lighting::Night);
    r.set_scene(&lamp_and_wall(0.0, -1.5), &[], &BTreeSet::new());
    let px = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    let open = brightness(pixel_at(&px, &cam, DVec3::new(-4.5, 0.0, 0.0)));
    let behind = brightness(pixel_at(&px, &cam, DVec3::new(1.5, 0.0, 0.0)));
    assert!(
        open.abs_diff(behind) < 30,
        "open {open}, behind the wall {behind}"
    );
    assert_eq!(r.shadow_faces_drawn(), 0);
}

/// Renders `model` with `selection`, lit by `lighting` and drawn in `style`, from `camera`.
fn render_styled(
    camera: &Camera,
    model: &Model,
    selection: &BTreeSet<EntityId>,
    style: ViewStyle,
    lighting: Lighting,
) -> Option<Vec<u8>> {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let mut renderer = screen.renderer();
    renderer.set_scene(&Scene::from_root(model.clone()), &[], selection);
    renderer.set_style(style);
    renderer.set_lighting(lighting);
    Some(screen.draw(&mut renderer, camera, Viewport::full((W, H)), &[]))
}

/// A style selecting in magenta, a color nothing else in these scenes has.
const MAGENTA: ViewStyle = ViewStyle {
    selected_edge: [1.0, 0.0, 1.0, 1.0],
    selected_face: [1.0, 0.0, 1.0],
    face_tint: 0.18,
    wide_selected_edges: true,
    ..ViewStyle::CLASSIC
};

fn magenta(p: [u8; 4]) -> bool {
    p[0] > 150 && p[1] < 110 && p[2] > 150
}

/// Whether channel `i` of `p` clearly outweighs the other two.
fn dominant(p: [u8; 4], i: usize) -> bool {
    (0..3).all(|k| k == i || i32::from(p[i]) > i32::from(p[k]) + 40)
}

/// Whether red and blue together clearly outweigh green.
fn magenta_dominant(p: [u8; 4]) -> bool {
    i32::from(p[0]) > i32::from(p[1]) + 40 && i32::from(p[2]) > i32::from(p[1]) + 40
}

fn count(px: &[u8], f: impl Fn([u8; 4]) -> bool) -> usize {
    px.chunks(4).filter(|p| f([p[0], p[1], p[2], p[3]])).count()
}

#[test]
fn a_style_tints_selected_faces_lightly_in_its_color() {
    let cam = camera();
    let model = ground_square();
    let face = model.faces().next().unwrap().0;
    let selected = BTreeSet::from([EntityId::Face(face)]);
    let (Some(plain), Some(tinted)) = (
        render_styled(&cam, &model, &BTreeSet::new(), MAGENTA, Lighting::Day),
        render_styled(&cam, &model, &selected, MAGENTA, Lighting::Day),
    ) else {
        return;
    };
    let at = DVec3::new(0.5, 0.5, 0.0);
    let (p, t) = (pixel_at(&plain, &cam, at), pixel_at(&tinted, &cam, at));
    assert!(t[0] > p[0] && t[2] > p[2], "toward magenta: {p:?} -> {t:?}");
    assert!(
        (i32::from(p[1]) - i32::from(t[1])).abs() < 60,
        "18% leaves the face showing: {p:?} -> {t:?}"
    );
}

#[test]
fn wide_selected_edges_cover_more_pixels() {
    let cam = camera();
    let model = ground_square();
    let edges: BTreeSet<EntityId> = model.edges().map(|(id, _)| EntityId::Edge(id)).collect();
    let thin = ViewStyle {
        wide_selected_edges: false,
        ..MAGENTA
    };
    let (Some(a), Some(b)) = (
        render_styled(&cam, &model, &edges, thin, Lighting::Day),
        render_styled(&cam, &model, &edges, MAGENTA, Lighting::Day),
    ) else {
        return;
    };
    let (thin_px, wide_px) = (count(&a, magenta), count(&b, magenta));
    assert!(thin_px > 0, "the selected edges show");
    assert!(
        wide_px as f64 > 1.6 * thin_px as f64,
        "{thin_px} -> {wide_px} pixels"
    );
}

#[test]
fn the_day_canvas_is_a_gradient_and_night_keeps_its_background() {
    // Looking horizontally: the upper half sees only sky.
    let mut cam = camera();
    cam.eye = DVec3::new(0.0, -10.0, 1.0);
    cam.target = DVec3::new(0.0, 0.0, 1.0);
    let style = ViewStyle {
        day_canvas: [[0.1, 0.1, 0.1, 1.0], [0.9, 0.9, 0.9, 1.0]],
        ..ViewStyle::CLASSIC
    };
    let empty = Model::new();
    let none = BTreeSet::new();
    let (Some(day), Some(night)) = (
        render_styled(&cam, &empty, &none, style, Lighting::Day),
        render_styled(&cam, &empty, &none, style, Lighting::Night),
    ) else {
        return;
    };
    let (top, quarter) = (pixel(&day, 0, 0), pixel(&day, 0, H / 4));
    assert!(
        i32::from(quarter[0]) - i32::from(top[0]) > 30,
        "lighter downward: {top:?} {quarter:?}"
    );
    assert_eq!(
        pixel(&night, 0, 0),
        pixel(&night, 0, H / 4),
        "night is one color"
    );
}

#[test]
fn restyling_recolors_a_selected_guide() {
    let c = camera();
    let mut m = Model::new();
    // Through the target, raised half a pixel onto the middle of a pixel row, so
    // multisampling covers it whole rather than sharing it between two rows.
    m.add_guide(kernel::Guide::Line {
        point: c.target + c.up() * (0.5 * c.pixel_size(c.target)),
        direction: c.right(),
        tag: None,
        hidden: false,
    })
    .unwrap();
    let guide = m.guides().next().unwrap().0;
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut renderer = screen.renderer();
    // The axes cross the view too: count against the same frame with the guide unselected.
    let scene = Scene::from_root(m);
    renderer.set_scene(&scene, &[], &BTreeSet::new());
    renderer.set_style(MAGENTA);
    let plain = screen.draw(&mut renderer, &c, Viewport::full((W, H)), &[]);
    let (plain_magenta, plain_green) = (
        count(&plain, magenta_dominant),
        count(&plain, |p| dominant(p, 1)),
    );
    renderer.set_selection(&BTreeSet::from([EntityId::Guide(guide)]));
    let px = screen.draw(&mut renderer, &c, Viewport::full((W, H)), &[]);
    assert!(
        count(&px, magenta_dominant) > plain_magenta + 20,
        "the guide in the style's color"
    );
    let green = ViewStyle {
        selected_edge: [0.0, 1.0, 0.0, 1.0],
        ..MAGENTA
    };
    renderer.set_style(green);
    let px = screen.draw(&mut renderer, &c, Viewport::full((W, H)), &[]);
    assert_eq!(
        count(&px, magenta_dominant),
        plain_magenta,
        "no rebuild needed"
    );
    assert!(
        count(&px, |p| dominant(p, 1)) > plain_green + 20,
        "the guide turns green"
    );
}

const HOVER: render::HoverLook = render::HoverLook {
    face: [1.0, 0.0, 1.0],
    face_mix: 0.25,
    edge: [1.0, 0.0, 1.0, 1.0],
};

/// Renders the ground square in `style` three times through one renderer: plain, with `hover`
/// hovered, then with the hover cleared.
fn hovered_frames(hover: EntityId, style: ViewStyle) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let cam = camera();
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(ground_square()), &[], &BTreeSet::new());
    r.set_style(style);
    let frame = |r: &mut Renderer| screen.draw(r, &cam, Viewport::full((W, H)), &[]);
    let plain = frame(&mut r);
    r.set_hover(Some((hover, HOVER)));
    let hovered = frame(&mut r);
    r.set_hover(None);
    let cleared = frame(&mut r);
    Some((plain, hovered, cleared))
}

/// The positions of an edge's ends.
fn edge_ends(model: &Model, edge: kernel::EdgeId) -> (DVec3, DVec3) {
    let e = model.edge(edge).unwrap();
    (
        model.position(e.v[0]).unwrap(),
        model.position(e.v[1]).unwrap(),
    )
}

#[test]
fn hovering_a_face_tints_it_toward_the_hover_color() {
    let model = ground_square();
    let face = model.faces().next().unwrap().0;
    let Some((plain, hovered, cleared)) = hovered_frames(EntityId::Face(face), ViewStyle::CLASSIC)
    else {
        return;
    };
    let at = DVec3::new(0.5, 0.5, 0.0);
    let (p, h) = (
        pixel_at(&plain, &camera(), at),
        pixel_at(&hovered, &camera(), at),
    );
    assert!(
        h[0] > p[0] && h[2] > p[2] && h[1] < p[1],
        "toward magenta: {p:?} -> {h:?}"
    );
    assert_eq!(
        plain, cleared,
        "clearing the hover draws what was drawn before"
    );
}

#[test]
fn a_hovered_edge_takes_the_hover_color_and_widens_where_selections_do() {
    let model = ground_square();
    let edge = model.edges().next().unwrap().0;
    let len = {
        let c = camera();
        let (a, b) = edge_ends(&model, edge);
        c.project(a).unwrap().distance(c.project(b).unwrap())
    };
    // CLASSIC never widens: the edge is drawn 1 px in the hover color.
    let Some((plain, thin, cleared)) = hovered_frames(EntityId::Edge(edge), ViewStyle::CLASSIC)
    else {
        return;
    };
    assert_eq!(count(&plain, magenta), 0);
    let thin = count(&thin, magenta);
    assert!(thin > 0, "the hovered edge in the hover color");
    assert_eq!(plain, cleared);
    // A style that widens selected edges widens a hovered one, about 2 px.
    let wide_style = ViewStyle {
        wide_selected_edges: true,
        ..ViewStyle::CLASSIC
    };
    let Some((_, wide, _)) = hovered_frames(EntityId::Edge(edge), wide_style) else {
        return;
    };
    let wide = count(&wide, magenta);
    assert!(wide as f64 > 1.6 * thin as f64, "{thin} -> {wide} pixels");
    assert!(
        wide as f64 > 1.6 * len,
        "{wide} pixels for {len} px of edge"
    );
}

#[test]
fn a_new_model_drops_the_hover() {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let cam = camera();
    let square = ground_square();
    let face = square.faces().next().unwrap().0;
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(square), &[], &BTreeSet::new());
    r.set_hover(Some((EntityId::Face(face), HOVER)));
    // A model without that face: nothing hovered, nothing written out of range.
    let empty = Scene::from_root(Model::new());
    r.set_model(&empty, &[]);
    let after = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    r.set_hover(None);
    let mut fresh = screen.renderer();
    fresh.set_scene(&empty, &[], &BTreeSet::new());
    let expected = screen.draw(&mut fresh, &cam, Viewport::full((W, H)), &[]);
    assert_eq!(after, expected);
    assert_eq!(
        screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]),
        expected
    );
}

/// An axis-aligned box from `min`, `size` on each side, as 12 triangles wound
/// counter-clockwise seen from outside, with flat normals, in `color`.
fn box_vertices(min: DVec3, size: f64, color: [f32; 4]) -> Vec<render::PreviewVertex> {
    cuboid_vertices(min, DVec3::splat(size), color)
}

/// `box_vertices` with a size of its own along each axis.
fn cuboid_vertices(min: DVec3, size: DVec3, color: [f32; 4]) -> Vec<render::PreviewVertex> {
    let corner = |k: usize| {
        let bit = |b: usize, s: f64| if k & b == 0 { 0.0 } else { s };
        min + DVec3::new(bit(1, size.x), bit(2, size.y), bit(4, size.z))
    };
    // Each side's corners, counter-clockwise seen from outside.
    let sides = [
        [0, 2, 3, 1], // -z
        [4, 5, 7, 6], // +z
        [0, 1, 5, 4], // -y
        [2, 6, 7, 3], // +y
        [0, 4, 6, 2], // -x
        [1, 3, 7, 5], // +x
    ];
    let mut out = Vec::new();
    for side in sides {
        let [a, b, c, d] = side.map(corner);
        let n = (b - a).cross(c - a).normalize().as_vec3().to_array();
        for p in [a, b, c, a, c, d] {
            out.push(render::PreviewVertex {
                pos: p.as_vec3().to_array(),
                normal: n,
                color,
            });
        }
    }
    out
}

#[test]
fn a_preview_solid_blends_over_what_is_behind_it_and_hides_behind_faces() {
    // A red box on the ground square at 70%: over the square, the pixel is reddish but shows
    // the square through it; clearing it gives back the plain frame.
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(ground_square()), &[], &BTreeSet::new());
    let cam = camera();
    let plain = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    r.set_preview(&box_vertices(
        DVec3::new(0.2, 0.2, 0.0),
        0.6,
        [1.0, 0.0, 0.0, 0.7],
    ));
    let shown = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    let p = pixel_at(&shown, &cam, DVec3::new(0.5, 0.5, 0.6));
    assert!(dominant(p, 0), "red: {p:?}");
    assert!(p[1] > 30 && p[2] > 30, "the square shows through: {p:?}");
    r.set_preview(&[]);
    assert_eq!(
        screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]),
        plain
    );
}

#[test]
fn a_preview_solid_under_a_face_is_hidden() {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(ground_square()), &[], &BTreeSet::new());
    let cam = camera();
    let plain = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    r.set_preview(&box_vertices(
        DVec3::new(-0.5, -0.5, -1.0),
        0.6,
        [1.0, 0.0, 0.0, 0.7],
    ));
    let under = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    assert_eq!(under, plain, "the square hides the solid under it");
    // A larger solid reuses the buffer after growing it, and still shows above the square.
    r.set_preview(&box_vertices(
        DVec3::new(-1.0, -1.0, 0.0),
        1.5,
        [1.0, 0.0, 0.0, 0.7],
    ));
    let above = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    assert!(dominant(
        pixel_at(&above, &cam, DVec3::new(-0.25, -0.25, 1.5)),
        0
    ));
}

#[test]
fn styled_axes_take_their_colors_and_wide_axes_cover_more_pixels() {
    if skip_on_warp("the axes pixel counts") {
        return;
    }
    let cam = camera();
    let none = BTreeSet::new();
    let magenta_axes = ViewStyle {
        axes: [[1.0, 0.0, 1.0, 1.0]; 3],
        ..ViewStyle::CLASSIC
    };
    let wide = ViewStyle {
        wide_axes: true,
        ..magenta_axes
    };
    let empty = Model::new();
    let (Some(classic), Some(thin), Some(thick)) = (
        render_styled(&cam, &empty, &none, ViewStyle::CLASSIC, Lighting::Day),
        render_styled(&cam, &empty, &none, magenta_axes, Lighting::Day),
        render_styled(&cam, &empty, &none, wide, Lighting::Day),
    ) else {
        return;
    };
    assert_eq!(count(&classic, magenta), 0);
    let (t, w) = (count(&thin, magenta), count(&thick, magenta));
    assert!(t > 0, "the axes in the style's colors");
    assert!(w as f64 > 1.25 * t as f64, "{t} -> {w} pixels");
}

#[test]
fn the_axes_are_drawn_where_the_drawing_axes_are() {
    if skip_on_warp("the moved axes pixel check") {
        return;
    }
    let cam = camera();
    let style = ViewStyle {
        axes: [[1.0, 0.0, 1.0, 1.0]; 3],
        ..ViewStyle::CLASSIC
    };
    let draw = |axes: Option<kernel::Axes>, context: &[InstanceId]| {
        let screen = Offscreen::new()?;
        let mut scene = Scene::from_root(Model::new());
        if let Some(axes) = axes {
            scene.set_axes(axes);
        }
        let mut renderer = screen.renderer();
        renderer.set_scene(&scene, context, &BTreeSet::new());
        renderer.set_style(style);
        Some(screen.draw(&mut renderer, &cam, Viewport::full((W, H)), &[]))
    };
    // Red along world y from (2, -2, 0), green along world -x.
    let moved = kernel::Axes::new(DVec3::new(2.0, -2.0, 0.0), DVec3::Y, -DVec3::X).unwrap();
    let (Some(world), Some(turned)) = (draw(None, &[]), draw(Some(moved), &[])) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // On the new red axis, and on the old X axis.
    let on_red = DVec3::new(2.0, 1.0, 0.0);
    let on_world_x = DVec3::new(3.0, 0.0, 0.0);
    assert!(!near(&world, &cam, on_red, magenta));
    assert!(near(&world, &cam, on_world_x, magenta));
    assert!(near(&turned, &cam, on_red, magenta), "the red axis moved");
    assert!(
        !near(&turned, &cam, on_world_x, magenta),
        "nothing is left on the world's X axis"
    );
    // The axes' origin is on the new axes too, the world origin is not.
    assert!(near(&turned, &cam, DVec3::new(2.0, -2.0, 0.0), magenta));
}

#[test]
fn a_pushed_in_preview_tints_the_box_sides_it_lies_on_evenly() {
    if skip_on_warp("the preview tint check") {
        return;
    }
    // A 0.6 m box with its top pushed 0.3 m down: the solid's sides lie in the box's side
    // planes, facing the same way. Every pixel of a side they cover is tinted, none speckled.
    let mut m = Model::new();
    let (lo, size) = (DVec3::new(0.2, 0.2, 0.0), 0.6);
    m.add_polygon(&[
        lo,
        lo + DVec3::new(size, 0.0, 0.0),
        lo + DVec3::new(size, size, 0.0),
        lo + DVec3::new(0.0, size, 0.0),
    ])
    .unwrap();
    let base = m.faces().next().unwrap().0;
    let up = if m.face(base).unwrap().normal.z > 0.0 {
        1.0
    } else {
        -1.0
    };
    m.push_pull(base, up * size).unwrap();
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(m), &[], &BTreeSet::new());
    let cam = camera();
    let plain = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    r.set_preview(&cuboid_vertices(
        lo + DVec3::new(0.0, 0.0, 0.3),
        DVec3::new(size, size, 0.3),
        [1.0, 0.0, 0.0, 0.7],
    ));
    let shown = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    // The -y side (y = 0.2) and the -x side (x = 0.2) face the default camera.
    let mut sampled = 0;
    for i in 1..10 {
        for j in 1..6 {
            let (u, z) = (0.2 + 0.06 * i as f64, 0.3 + 0.05 * j as f64);
            for p in [DVec3::new(u, 0.2, z), DVec3::new(0.2, u, z)] {
                let (a, b) = (pixel_at(&plain, &cam, p), pixel_at(&shown, &cam, p));
                assert!(
                    i32::from(b[0]) > i32::from(b[1]) + 40
                        && i32::from(b[1]) + 30 < i32::from(a[1]),
                    "at {p}: {a:?} -> {b:?}"
                );
                sampled += 1;
            }
        }
    }
    assert_eq!(sampled, 90);
}

#[test]
fn the_hover_wins_over_the_selection_on_a_face() {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let cam = camera();
    let square = ground_square();
    let face = EntityId::Face(square.faces().next().unwrap().0);
    let scene = Scene::from_root(square);
    let hovered = |selection: &BTreeSet<EntityId>| {
        let mut r = screen.renderer();
        r.set_scene(&scene, &[], selection);
        r.set_style(MAGENTA);
        r.set_hover(Some((face, HOVER)));
        let px = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
        pixel_at(&px, &cam, DVec3::new(0.5, 0.5, 0.0))
    };
    assert_eq!(
        hovered(&BTreeSet::from([face])),
        hovered(&BTreeSet::new()),
        "a selected face hovered looks as an unselected one hovered"
    );
}

#[test]
fn a_dimension_is_drawn_and_lit_when_selected_or_hovered() {
    let c = camera();
    let mut m = Model::new();
    // A dimension across the view through the target, offset toward the camera's up.
    let (a, b) = (c.target - c.right() * 2.0, c.target + c.right() * 2.0);
    m.add_dimension(kernel::Dimension::Linear {
        a: kernel::Anchor::Point(a),
        b: kernel::Anchor::Point(b),
        offset: c.up() * 0.5,
        tag: None,
    })
    .unwrap();
    let dimension = EntityId::Dimension(m.dimensions().next().unwrap().0);
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(m.clone()), &[], &BTreeSet::new());
    r.set_style(MAGENTA);
    let frame = |r: &mut Renderer| screen.draw(r, &c, Viewport::full((W, H)), &[]);
    let plain = frame(&mut r);
    // Drawn: the dimension line is dark where the empty view is not.
    let empty = render(&c).unwrap();
    let dark = |px: &[u8]| count(px, |p| p[0] < 100 && p[1] < 100 && p[2] < 100);
    assert!(dark(&plain) > dark(&empty) + 40, "the dimension's lines");
    let base = count(&plain, magenta_dominant);
    r.set_selection(&BTreeSet::from([dimension]));
    assert!(
        count(&frame(&mut r), magenta_dominant) > base + 40,
        "selected"
    );
    r.set_selection(&BTreeSet::new());
    assert_eq!(count(&frame(&mut r), magenta_dominant), base);
    r.set_hover(Some((dimension, HOVER)));
    assert!(
        count(&frame(&mut r), magenta_dominant) > base + 40,
        "hovered"
    );
    r.set_hover(None);
    assert_eq!(count(&frame(&mut r), magenta_dominant), base);
}
