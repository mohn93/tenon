//! Offscreen rendering to RGBA pixels, for headless mode and the API's `render_view`.

use std::collections::BTreeSet;

use kernel::{EntityId, InstanceId, Scene};
use session::{Camera, Lighting, SectionDisplay};

use crate::Renderer;

/// Renders models to memory with its own offscreen renderer.
pub struct Snapshotter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    software: bool,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Snapshotter {
    /// Creates its own GPU device. None when no adapter is available, and always None on the web
    /// (creating a device there is asynchronous; captures are not supported on the web).
    #[cfg(target_arch = "wasm32")]
    pub fn new_headless() -> Option<Self> {
        None
    }

    /// Creates its own GPU device. None when no adapter is available.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_headless() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::required_limits(&adapter),
            ..Default::default()
        }))
        .ok()?;
        let mut snap = Self::with_device(&device, &queue);
        snap.software = adapter.get_info().device_type == wgpu::DeviceType::Cpu;
        Some(snap)
    }

    /// True when `new_headless` found a software adapter (e.g. Mesa lavapipe), whose rasterizer
    /// differs from real GPUs. False for a shared device, whose adapter is not known here.
    pub fn is_software_adapter(&self) -> bool {
        self.software
    }

    /// Shares an existing device, e.g. the window's.
    pub fn with_device(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            renderer: Renderer::new(device, queue, FORMAT),
            software: false,
        }
    }

    /// Colors faces by tag (or by material, the default) in the captures from now on.
    pub fn set_color_by_tag(&mut self, on: bool) {
        self.renderer.set_color_by_tag(on);
    }

    /// Captures from now on as printed pages (or, off, as views): white paper without the
    /// ground grid and axes.
    pub fn set_print_mode(&mut self, on: bool) {
        self.renderer.set_print_mode(on);
    }

    /// Shows section planes and cuts as `display` says in the captures from now on.
    pub fn set_section_display(&mut self, display: SectionDisplay) {
        self.renderer.set_section_display(display);
    }

    /// Renders `scene` as seen by `camera` (its viewport is replaced by the image size) and lit
    /// by `lighting` into tightly packed RGBA8 pixels, top row first, with the open context and
    /// `selection` shown. None if the GPU readback fails.
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        scene: &Scene,
        context: &[InstanceId],
        selection: &BTreeSet<EntityId>,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        self.renderer.set_scene(scene, context, selection);
        self.capture_prepared(camera, lighting, width, height)
    }

    /// Sets the scene to render, with the open context and `selection` shown, for later
    /// [`Snapshotter::capture_prepared`] calls: many views of one scene upload it once.
    pub fn prepare(
        &mut self,
        scene: &Scene,
        context: &[InstanceId],
        selection: &BTreeSet<EntityId>,
    ) {
        self.renderer.set_scene(scene, context, selection);
    }

    /// [`Snapshotter::capture`] of the scene [`Snapshotter::prepare`] set (or the last capture's).
    pub fn capture_prepared(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        let (width, height) = (width.max(1), height.max(1));
        let mut camera = camera.clone();
        camera.viewport = glam::DVec2::new(width as f64, height as f64);

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("snapshot"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.set_lighting(lighting);
        self.renderer.render(&view, (width, height), &camera, &[]);

        // Texture-to-buffer copies need rows padded to a multiple of 256 bytes.
        let row = width * 4;
        let padded_row =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snapshot readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);
        // Reading the pixels back blocks until the GPU is done, which a browser cannot do.
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (&buffer, padded_row, row, height);
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.read_back(&buffer, padded_row, row, height)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_back(
        &self,
        buffer: &wgpu::Buffer,
        padded_row: u32,
        row: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let mapped = buffer.slice(..).get_mapped_range().ok()?;
        let mut pixels = Vec::with_capacity((row * height) as usize);
        for r in mapped.chunks(padded_row as usize) {
            pixels.extend_from_slice(&r[..row as usize]);
        }
        Some(pixels)
    }
}
