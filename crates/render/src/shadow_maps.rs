//! The GPU side of shadows (see `shadows`): the cube-map array and the pipeline drawing the
//! casters into it.

use crate::scene::CasterChunk;
use crate::shadows::{
    SHADOW_FORMAT, SHADOW_SIZE, ShadowKey, ShadowSlots, capacity_for, cube_faces, face_sees,
};

/// One cube face as shadow.wgsl reads it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceUniform {
    view_proj: [f32; 16],
    /// The light's position, and its range in w.
    light: [f32; 4],
}

/// The casters as last given to the renderer: a vertex buffer of triangle corners and where
/// each placement's lie.
pub(crate) struct Casters<'a> {
    pub buffer: Option<&'a wgpu::Buffer>,
    pub chunks: &'a [CasterChunk],
}

/// The cube-map array and what drawing into it needs. It starts with one slot and grows (see
/// `capacity_for`), up to `max` slots.
pub(crate) struct ShadowMaps {
    pipeline: wgpu::RenderPipeline,
    face_layout: wgpu::BindGroupLayout,
    /// A uniform per cube face of every slot, `stride` bytes apart.
    faces: wgpu::Buffer,
    face_group: wgpu::BindGroup,
    stride: u64,
    /// The whole array as cube maps, for lookups.
    pub cubes: wgpu::TextureView,
    /// Each layer alone, to draw into.
    layers: Vec<wgpu::TextureView>,
    pub slots: ShadowSlots,
    /// The most slots the device allows (at most `MAX_SHADOWS`).
    pub max: usize,
    /// How many cube faces have been drawn.
    pub drawn: u64,
}

impl ShadowMaps {
    pub fn new(device: &wgpu::Device, max: usize) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/shadow.wgsl"));
        let face_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow face"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<FaceUniform>() as u64
                    ),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&face_layout)],
            immediate_size: 0,
        });
        let corner_attribs = wgpu::vertex_attr_array![0 => Float32x3];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &corner_attribs,
                })],
            },
            // Both sides of a face cast.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = (std::mem::size_of::<FaceUniform>() as u64).div_ceil(align) * align;
        let (faces, face_group, cubes, layers) = Self::allocate(device, &face_layout, stride, 1);
        Self {
            pipeline,
            face_layout,
            faces,
            face_group,
            stride,
            cubes,
            layers,
            slots: ShadowSlots::new(1),
            max,
            drawn: 0,
        }
    }

    /// The face uniforms, their bind group, and the array's views for `capacity` slots.
    fn allocate(
        device: &wgpu::Device,
        face_layout: &wgpu::BindGroupLayout,
        stride: u64,
        capacity: usize,
    ) -> (
        wgpu::Buffer,
        wgpu::BindGroup,
        wgpu::TextureView,
        Vec<wgpu::TextureView>,
    ) {
        let layers = 6 * capacity as u32;
        let faces = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow faces"),
            size: stride * u64::from(layers),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let face_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow face"),
            layout: face_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &faces,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<FaceUniform>() as u64),
                }),
            }],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow maps"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let cubes = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow cubes"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let layer_views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow face"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        (faces, face_group, cubes, layer_views)
    }

    /// Makes room for `needed` slots, up to `max`; true when the array was made anew (every
    /// map is then drawn again, and the scene's bind group must take the new view).
    pub fn grow(&mut self, device: &wgpu::Device, needed: usize) -> bool {
        let capacity = capacity_for(needed, self.max);
        if capacity <= self.slots.capacity() {
            return false;
        }
        let (faces, face_group, cubes, layers) =
            Self::allocate(device, &self.face_layout, self.stride, capacity);
        self.faces = faces;
        self.face_group = face_group;
        self.cubes = cubes;
        self.layers = layers;
        self.slots = ShadowSlots::new(capacity);
        true
    }

    /// Records drawing the maps of `slots` (each given with its light) from `casters`: each
    /// face draws only the placements whose bounds reach the light and lie in its view.
    pub fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        slots: &[(usize, ShadowKey)],
        casters: &Casters,
    ) {
        for &(slot, key) in slots {
            let position = glam::Vec3::from(key.position);
            let [x, y, z] = key.position;
            for (k, view_proj) in cube_faces(position, key.range).iter().enumerate() {
                let face = FaceUniform {
                    view_proj: view_proj.to_cols_array(),
                    light: [x, y, z, key.range],
                };
                let layer = 6 * slot + k;
                let offset = self.stride * layer as u64;
                queue.write_buffer(&self.faces, offset, bytemuck::bytes_of(&face));
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow face"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.layers[layer],
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                self.drawn += 1;
                let Some(buffer) = casters.buffer else {
                    continue;
                };
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.face_group, &[offset as u32]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                // Placements next to each other in the buffer are drawn in one call.
                let mut run: Option<std::ops::Range<u32>> = None;
                for chunk in casters.chunks {
                    let (lo, hi) = (chunk.lo.into(), chunk.hi.into());
                    if !chunk.reaches(position, key.range) || !face_sees(k, position, lo, hi) {
                        continue;
                    }
                    match &mut run {
                        Some(r) if r.end == chunk.vertices.start => r.end = chunk.vertices.end,
                        _ => {
                            if let Some(r) = run.replace(chunk.vertices.clone()) {
                                pass.draw(r, 0..1);
                            }
                        }
                    }
                }
                if let Some(r) = run {
                    pass.draw(r, 0..1);
                }
            }
        }
    }
}
