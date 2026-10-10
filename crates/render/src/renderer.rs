use kernel::Model;
use session::Camera;
use wgpu::util::DeviceExt;

use crate::grid::{BACKGROUND, LineVertex, scene_lines};
use crate::scene::{FaceVertex, model_geometry};

pub const SAMPLE_COUNT: u32 = 4;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const LINE_ATTRIBS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
const FACE_ATTRIBS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

/// Camera data shared by every pipeline (matches `Uniforms` in scene.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    eye: [f32; 4],
}

/// Multisampled color and depth attachments matching the output size.
struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// A vertex buffer and how many vertices it holds.
struct Mesh {
    buffer: wgpu::Buffer,
    count: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    ground_pipeline: wgpu::RenderPipeline,
    face_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    ground: Mesh,
    faces: Option<Mesh>,
    edges: Option<Mesh>,
    targets: Option<Targets>,
}

/// How a pipeline uses the depth buffer.
struct DepthUse {
    write: bool,
    compare: wgpu::CompareFunction,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/scene.wgsl"));
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &LINE_ATTRIBS,
        };
        let face_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FaceVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &FACE_ATTRIBS,
        };
        let tested = DepthUse {
            write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
        };
        let pipeline = |label: &str,
                        vertex_entry: &str,
                        fragment_entry: &str,
                        buffer: &wgpu::VertexBufferLayout,
                        topology: wgpu::PrimitiveTopology,
                        depth: DepthUse| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex_entry),
                    compilation_options: Default::default(),
                    buffers: &[Some(buffer.clone())],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth.write),
                    depth_compare: Some(depth.compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLE_COUNT,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines = wgpu::PrimitiveTopology::LineList;
        let ground_pipeline = pipeline(
            "ground",
            "vs_ground",
            "fs_line",
            &line_layout,
            lines,
            tested,
        );
        let face_pipeline = pipeline(
            "faces",
            "vs_face",
            "fs_face",
            &face_layout,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::Greater,
            },
        );
        let edge_pipeline = pipeline(
            "edges",
            "vs_edge",
            "fs_line",
            &line_layout,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
        );
        let overlay_pipeline = pipeline(
            "overlay",
            "vs_overlay",
            "fs_line",
            &line_layout,
            lines,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
        );

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let ground = mesh(device, "ground lines", &scene_lines()).expect("grid is never empty");
        Self {
            device: device.clone(),
            queue: queue.clone(),
            format,
            ground_pipeline,
            face_pipeline,
            edge_pipeline,
            overlay_pipeline,
            uniforms,
            bind_group,
            ground,
            faces: None,
            edges: None,
            targets: None,
        }
    }

    /// Rebuilds the face and edge buffers from the model. Call when the model changes.
    pub fn set_model(&mut self, model: &Model) {
        let g = model_geometry(model);
        self.faces = mesh(&self.device, "faces", &g.faces);
        self.edges = mesh(&self.device, "edges", &g.edges);
    }

    /// Draws grid, axes, model and `overlay` (line pairs drawn on top of everything).
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(self.make_targets(size));
        }
        let targets = self.targets.as_ref().expect("targets just created");

        let eye = camera.eye.as_vec3();
        let uniforms = Uniforms {
            view_proj: camera.view_proj_f32().to_cols_array(),
            eye: [eye.x, eye.y, eye.z, 1.0],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let overlay = mesh(&self.device, "overlay", overlay);

        let [r, g, b, a] = BACKGROUND.map(f64::from);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(target),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            let layers = [
                (&self.ground_pipeline, Some(&self.ground)),
                (&self.face_pipeline, self.faces.as_ref()),
                (&self.edge_pipeline, self.edges.as_ref()),
                (&self.overlay_pipeline, overlay.as_ref()),
            ];
            for (pipeline, mesh) in layers {
                if let Some(mesh) = mesh {
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    fn make_targets(&self, size: (u32, u32)) -> Targets {
        let texture = |label, format| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: SAMPLE_COUNT,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Targets {
            size,
            msaa: texture("msaa color", self.format),
            depth: texture("depth", DEPTH_FORMAT),
        }
    }
}

/// Uploads vertices, or None when there are none to draw.
fn mesh<T: bytemuck::Pod>(device: &wgpu::Device, label: &str, vertices: &[T]) -> Option<Mesh> {
    if vertices.is_empty() {
        return None;
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some(Mesh {
        buffer,
        count: vertices.len() as u32,
    })
}
