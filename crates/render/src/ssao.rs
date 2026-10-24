//! Ambient occlusion (the style's `ambient_occlusion`): before the main pass the opaque faces are
//! drawn at half resolution into a depth buffer and a buffer of world normals (the renderer's
//! `ao_normal_pipeline`), a hemisphere-sampling pass turns those into how open each pixel is,
//! and a blur that stops at creases and edges smooths it. The face shader then multiplies the
//! ambient and diffuse light by that (not emission, highlights, reflections or edges).

use glam::Mat4;
use session::Camera;
use wgpu::util::DeviceExt;

use crate::renderer::DEPTH_FORMAT;

/// The format of the world normals (each component as 0 to 1 for -1 to 1).
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The format of the occlusion: 1 is open, 0 shut.
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Matches `Params` in ssao.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    inv_view_proj: [f32; 16],
    view_proj: [f32; 16],
    cam_pos: [f32; 4],
    cam_forward: [f32; 4],
    rect: [f32; 4],
    settings: [f32; 4],
}

/// The size of the half resolution buffers for a target `size` pixels across.
fn half(size: (u32, u32)) -> (u32, u32) {
    (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1))
}

/// The buffers of one target size.
pub struct AoTargets {
    size: (u32, u32),
    pub depth: wgpu::TextureView,
    pub normals: wgpu::TextureView,
    /// The finished occlusion, which the face shader reads.
    pub ao: wgpu::TextureView,
    raw: wgpu::TextureView,
    ssao_bind: wgpu::BindGroup,
    blur_bind: wgpu::BindGroup,
}

/// The occlusion passes and their buffers, made when a style first asks for ambient occlusion.
pub struct Ssao {
    device: wgpu::Device,
    ssao_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    ssao_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    targets: Option<AoTargets>,
}

fn layout(device: &wgpu::Device, label: &str, raw: bool) -> wgpu::BindGroupLayout {
    let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let float = wgpu::TextureSampleType::Float { filterable: false };
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        texture(1, wgpu::TextureSampleType::Depth),
        texture(2, float),
    ];
    if raw {
        entries.push(texture(3, float));
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

impl Ssao {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ssao.wgsl").into()),
        });
        let ssao_layout = layout(device, "ssao", false);
        let blur_layout = layout(device, "ssao blur", true);
        let pipeline = |label: &str, bind: &wgpu::BindGroupLayout, fragment: &str| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bind)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: AO_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let ssao_pipeline = pipeline("ssao", &ssao_layout, "fs_ssao");
        let blur_pipeline = pipeline("ssao blur", &blur_layout, "fs_blur");
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ssao params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            device: device.clone(),
            ssao_layout,
            blur_layout,
            ssao_pipeline,
            blur_pipeline,
            params,
            targets: None,
        }
    }

    pub fn targets(&self) -> Option<&AoTargets> {
        self.targets.as_ref()
    }

    /// Drops the buffers; true when there were any.
    pub fn release(&mut self) -> bool {
        self.targets.take().is_some()
    }

    /// Makes the buffers for a target `size` pixels across unless they are that size already;
    /// true when they are new.
    pub fn resize(&mut self, size: (u32, u32)) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let (width, height) = half(size);
        let texture = |label, format| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let depth = texture("ao depth", DEPTH_FORMAT);
        let normals = texture("ao normals", NORMAL_FORMAT);
        let raw = texture("ao raw", AO_FORMAT);
        let ao = texture("ao", AO_FORMAT);
        fn entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            }
        }
        let params = wgpu::BindGroupEntry {
            binding: 0,
            resource: self.params.as_entire_binding(),
        };
        let ssao_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssao"),
            layout: &self.ssao_layout,
            entries: &[params.clone(), entry(1, &depth), entry(2, &normals)],
        });
        let blur_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssao blur"),
            layout: &self.blur_layout,
            entries: &[params, entry(1, &depth), entry(2, &normals), entry(3, &raw)],
        });
        self.targets = Some(AoTargets {
            size,
            depth,
            normals,
            ao,
            raw,
            ssao_bind,
            blur_bind,
        });
        true
    }

    /// Writes the passes' parameters: the camera, the view's rectangle in target pixels
    /// (`x, y, width, height`), the radius and the strength.
    pub fn write_params(
        &self,
        queue: &wgpu::Queue,
        camera: &Camera,
        rect: [f32; 4],
        radius: f32,
        strength: f32,
    ) {
        let view_proj = (camera.proj() * camera.view()).as_mat4();
        let inverse = (camera.proj() * camera.view()).inverse().as_mat4();
        let (eye, forward) = (camera.eye.as_vec3(), camera.forward().as_vec3());
        let params = Params {
            inv_view_proj: Mat4::to_cols_array(&inverse),
            view_proj: view_proj.to_cols_array(),
            cam_pos: [eye.x, eye.y, eye.z, 0.0],
            cam_forward: [forward.x, forward.y, forward.z, 0.0],
            rect: rect.map(|v| v * 0.5),
            settings: [radius, strength, 0.0, 0.0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    /// The occlusion and blur passes, after the depth and normals are drawn.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(t) = &self.targets else { return };
        for (label, pipeline, bind, target) in [
            ("ssao", &self.ssao_pipeline, &t.ssao_bind, &t.raw),
            ("ssao blur", &self.blur_pipeline, &t.blur_bind, &t.ao),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// A 1 x 1 white occlusion texture: what the face shader is bound to while no style asks for
/// ambient occlusion (and never reads).
pub fn placeholder(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("no ao"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: AO_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::default(),
            &[255],
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}
