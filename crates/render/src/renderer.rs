use std::collections::BTreeSet;

use kernel::{EntityId, InstanceId, Scene};
use session::Camera;
use wgpu::util::DeviceExt;

use crate::grid::{BACKGROUND, LineVertex, flat_ground_lines, vertical_axis_lines};
use crate::guides::{GuideShape, guide_lines};
use crate::scene::{BaseFaceVertex, Spans, scene_base, shown_guides};

pub const SAMPLE_COUNT: u32 = 4;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const LINE_ATTRIBS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
const FACE_ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 3 => Float32x3, 4 => Float32x3, 5 => Float32x2
];
/// The selection flag of a face or edge vertex, from a second vertex buffer (see
/// `scene::Flags`).
const FLAG_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![2 => Float32];

/// Camera data shared by every pipeline (matches `Uniforms` in scene.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    /// The headlight: the eye's position (w = 1), or in a parallel view the direction
    /// toward the viewer (w = 0).
    eye: [f32; 4],
    /// How lines are pulled toward the camera (see `line_vertex` in scene.wgsl): x scales
    /// clip z by the line's factor (perspective), y adds a fixed depth step (parallel).
    depth_offset: [f32; 4],
}

impl Uniforms {
    fn new(camera: &Camera) -> Self {
        let (eye, depth_offset) = if camera.is_parallel() {
            let toward = (-camera.forward()).as_vec3();
            // The step that moves a line by (factor - 1) eye-to-target distances: clip z
            // runs from 1 to 0 over the depth range.
            let (near, far) = camera.depth_range();
            let unit = (camera.distance() / (far - near)) as f32;
            ([toward.x, toward.y, toward.z, 0.0], [0.0, unit, 0.0, 0.0])
        } else {
            let eye = camera.eye.as_vec3();
            ([eye.x, eye.y, eye.z, 1.0], [1.0, 0.0, 0.0, 0.0])
        };
        Self {
            view_proj: camera.view_proj_f32().to_cols_array(),
            eye,
            depth_offset,
        }
    }
}

/// A rectangle of the render target in physical pixels, origin top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    /// The whole target.
    pub fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// Multisampled color and depth attachments matching the output size.
struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// A vertex buffer and how many vertices it holds, with, for model faces and edges, a second
/// buffer holding each vertex's selection flag.
struct Mesh {
    buffer: wgpu::Buffer,
    count: u32,
    flags: Option<wgpu::Buffer>,
}

/// The triangles of `faces` with a see-through side, drawn after everything opaque.
struct Glass {
    /// The first vertex of each opaque triangle, for drawing them alone.
    solid: wgpu::Buffer,
    solid_count: u32,
    /// The vertices of the see-through triangles, far to near as last sorted.
    order: wgpu::Buffer,
    /// Each see-through triangle's first vertex and center.
    triangles: Vec<(u32, glam::Vec3)>,
    /// The view `order` was sorted for.
    sorted_for: Option<[f32; 4]>,
}

impl Glass {
    /// The see-through triangles of `faces`, or None when there are none.
    fn new(device: &wgpu::Device, faces: &[BaseFaceVertex], glass: &[u32]) -> Option<Glass> {
        if glass.is_empty() {
            return None;
        }
        let mut solid = Vec::with_capacity(faces.len() - 3 * glass.len());
        let mut next = glass.iter().peekable();
        for first in (0..faces.len() as u32).step_by(3) {
            if next.next_if_eq(&&first).is_none() {
                solid.extend([first, first + 1, first + 2]);
            }
        }
        let triangles: Vec<(u32, glam::Vec3)> = glass
            .iter()
            .map(|&k| {
                let corner = |i: u32| glam::Vec3::from(faces[i as usize].pos);
                (k, (corner(k) + corner(k + 1) + corner(k + 2)) / 3.0)
            })
            .collect();
        let index_buffer = |label: &str, indices: &[u32], usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                // An empty buffer still needs a size; nothing is drawn from it then.
                contents: bytemuck::cast_slice(if indices.is_empty() { &[0u32] } else { indices }),
                usage: wgpu::BufferUsages::INDEX | usage,
            })
        };
        let unsorted: Vec<u32> = glass.iter().flat_map(|&k| [k, k + 1, k + 2]).collect();
        Some(Glass {
            solid: index_buffer("solid faces", &solid, wgpu::BufferUsages::empty()),
            solid_count: solid.len() as u32,
            order: index_buffer("glass faces", &unsorted, wgpu::BufferUsages::COPY_DST),
            triangles,
            sorted_for: None,
        })
    }

    fn count(&self) -> u32 {
        3 * self.triangles.len() as u32
    }

    /// Sorts the triangles far to near from `camera`, unless they already are for its view.
    fn sort(&mut self, queue: &wgpu::Queue, camera: &Camera) {
        let (origin, toward) = (camera.eye.as_vec3(), camera.forward().as_vec3());
        let view = if camera.is_parallel() {
            [toward.x, toward.y, toward.z, 0.0]
        } else {
            [origin.x, origin.y, origin.z, 1.0]
        };
        if self.sorted_for == Some(view) {
            return;
        }
        let indices = back_to_front(&self.triangles, view);
        queue.write_buffer(&self.order, 0, bytemuck::cast_slice(&indices));
        self.sorted_for = Some(view);
    }
}

/// The vertices of `triangles` (first vertex, center) ordered far to near for `view`: the
/// eye's position (w = 1), or the view direction of a parallel view (w = 0).
fn back_to_front(triangles: &[(u32, glam::Vec3)], view: [f32; 4]) -> Vec<u32> {
    let [x, y, z, w] = view;
    let v = glam::Vec3::new(x, y, z);
    let depth = |c: glam::Vec3| {
        if w == 0.0 {
            c.dot(v)
        } else {
            c.distance_squared(v)
        }
    };
    let mut keyed: Vec<(f32, u32)> = triangles.iter().map(|&(k, c)| (depth(c), k)).collect();
    keyed.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    keyed.iter().flat_map(|&(_, k)| [k, k + 1, k + 2]).collect()
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    flat_ground_pipeline: wgpu::RenderPipeline,
    ground_pipeline: wgpu::RenderPipeline,
    face_pipeline: wgpu::RenderPipeline,
    /// The opaque side of a triangle whose other side is see-through.
    solid_side_pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    guide_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    flat_ground: Mesh,
    vertical_axis: Mesh,
    faces: Option<Mesh>,
    /// The see-through triangles of `faces`, if any.
    glass: Option<Glass>,
    edges: Option<Mesh>,
    /// Where the open context's entities lie in `faces`, `edges` and `base_guides`.
    spans: Spans,
    /// Guides in root coordinates with nothing selected.
    base_guides: Vec<GuideShape>,
    /// The guides with the selection shown; their lines depend on the camera, so they are
    /// built per frame.
    guides: Vec<GuideShape>,
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
            array_stride: std::mem::size_of::<BaseFaceVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &FACE_ATTRIBS,
        };
        let flag_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<f32>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &FLAG_ATTRIBS,
        };
        let lines_only = [Some(line_layout.clone())];
        let flagged_faces = [Some(face_layout), Some(flag_layout.clone())];
        let flagged_lines = [Some(line_layout), Some(flag_layout)];
        let tested = DepthUse {
            write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
        };
        let pipeline = |label: &str,
                        vertex_entry: &str,
                        fragment_entry: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        depth: DepthUse,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex_entry),
                    compilation_options: Default::default(),
                    buffers,
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
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines = wgpu::PrimitiveTopology::LineList;
        // Flat ground: no depth test at all, so the faces drawn after it always cover it.
        let flat_ground_pipeline = pipeline(
            "flat ground",
            "vs_overlay",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
        );
        let ground_pipeline = pipeline(
            "ground",
            "vs_ground",
            "fs_line",
            &lines_only,
            lines,
            tested,
            None,
        );
        let face_pipeline = pipeline(
            "faces",
            "vs_face",
            "fs_face",
            &flagged_faces,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::Greater,
            },
            None,
        );
        // The opaque sides of see-through triangles join the opaque faces; their see-through
        // sides are blended over everything opaque, tested against depth but not writing it.
        let solid_side_pipeline = pipeline(
            "solid sides",
            "vs_face",
            "fs_face_solid_side",
            &flagged_faces,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::Greater,
            },
            None,
        );
        let glass_pipeline = pipeline(
            "glass",
            "vs_face",
            "fs_glass",
            &flagged_faces,
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Greater,
            },
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        let edge_pipeline = pipeline(
            "edges",
            "vs_edge",
            "fs_line",
            &flagged_lines,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        let guide_pipeline = pipeline(
            "guides",
            "vs_guide",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        let overlay_pipeline = pipeline(
            "overlay",
            "vs_overlay",
            "fs_line",
            &lines_only,
            lines,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
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

        let flat_ground =
            mesh(device, "flat ground lines", &flat_ground_lines()).expect("grid is never empty");
        let vertical_axis =
            mesh(device, "vertical axis", &vertical_axis_lines()).expect("z axis is never empty");
        Self {
            device: device.clone(),
            queue: queue.clone(),
            format,
            flat_ground_pipeline,
            ground_pipeline,
            face_pipeline,
            solid_side_pipeline,
            glass_pipeline,
            edge_pipeline,
            guide_pipeline,
            overlay_pipeline,
            uniforms,
            bind_group,
            flat_ground,
            vertical_axis,
            faces: None,
            glass: None,
            edges: None,
            spans: Spans::default(),
            base_guides: Vec::new(),
            guides: Vec::new(),
            targets: None,
        }
    }

    /// Rebuilds the face and edge buffers from the scene, with the open context `context` and
    /// `selection` shown (see `scene::scene_geometry`): `set_model` then `set_selection`.
    pub fn set_scene(
        &mut self,
        scene: &Scene,
        context: &[InstanceId],
        selection: &BTreeSet<EntityId>,
    ) {
        self.set_model(scene, context);
        self.set_selection(selection);
    }

    /// Rebuilds the face and edge buffers from the scene with the open context `context`, with
    /// nothing selected until `set_selection`. Call when the model or the open context changes.
    pub fn set_model(&mut self, scene: &Scene, context: &[InstanceId]) {
        let base = scene_base(scene, context);
        self.faces = flagged_mesh(&self.device, "faces", &base.faces);
        self.glass = Glass::new(&self.device, &base.faces, &base.glass());
        self.edges = flagged_mesh(&self.device, "edges", &base.edges);
        self.spans = base.spans;
        self.guides = base.guides.clone();
        self.base_guides = base.guides;
    }

    /// Shows `selection`, ids of the open context given to the last `set_model`, by rewriting
    /// the selection flags only. Call when just the selection changes.
    pub fn set_selection(&mut self, selection: &BTreeSet<EntityId>) {
        let flags = self.spans.flags(selection);
        for (mesh, flags) in [(&self.faces, &flags.faces), (&self.edges, &flags.edges)] {
            if let Some(buffer) = mesh.as_ref().and_then(|m| m.flags.as_ref()) {
                self.queue
                    .write_buffer(buffer, 0, bytemuck::cast_slice(flags));
            }
        }
        self.guides = shown_guides(&self.base_guides, &flags.guides);
    }

    /// Draws grid, axes, model and `overlay` (line pairs drawn on top of everything) over the
    /// whole target.
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        self.render_in(target, size, Viewport::full(size), camera, overlay);
    }

    /// Like `render`, but the scene fills only `viewport`; the rest of the target is cleared to
    /// the background. `camera.viewport` should match the viewport's size.
    pub fn render_in(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        viewport: Viewport,
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(self.make_targets(size));
        }
        let targets = self.targets.as_ref().expect("targets just created");

        let uniforms = Uniforms::new(camera);
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let overlay = mesh(&self.device, "overlay", overlay);
        let guides = mesh(&self.device, "guides", &guide_lines(&self.guides, camera));
        if let Some(glass) = &mut self.glass {
            glass.sort(&self.queue, camera);
        }

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
            // Clamp the viewport to the target; an empty one leaves just the cleared background.
            let (x, y) = (viewport.x.min(size.0), viewport.y.min(size.1));
            let (w, h) = (
                viewport.width.min(size.0 - x),
                viewport.height.min(size.1 - y),
            );
            if w > 0 && h > 0 {
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(0, &self.bind_group, &[]);
                let bind = |pass: &mut wgpu::RenderPass, pipeline, mesh: &Mesh| {
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    if let Some(flags) = &mesh.flags {
                        pass.set_vertex_buffer(1, flags.slice(..));
                    }
                };
                let draw = |pass: &mut wgpu::RenderPass, pipeline, mesh: Option<&Mesh>| {
                    if let Some(mesh) = mesh {
                        bind(pass, pipeline, mesh);
                        pass.draw(0..mesh.count, 0..1);
                    }
                };
                draw(
                    &mut pass,
                    &self.flat_ground_pipeline,
                    Some(&self.flat_ground),
                );
                draw(&mut pass, &self.ground_pipeline, Some(&self.vertical_axis));
                match (&self.faces, &self.glass) {
                    (Some(faces), Some(glass)) => {
                        if glass.solid_count > 0 {
                            bind(&mut pass, &self.face_pipeline, faces);
                            pass.set_index_buffer(glass.solid.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..glass.solid_count, 0, 0..1);
                        }
                        bind(&mut pass, &self.solid_side_pipeline, faces);
                        pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..glass.count(), 0, 0..1);
                    }
                    (faces, _) => draw(&mut pass, &self.face_pipeline, faces.as_ref()),
                }
                draw(&mut pass, &self.edge_pipeline, self.edges.as_ref());
                draw(&mut pass, &self.guide_pipeline, guides.as_ref());
                // See-through faces last but for the overlay: everything opaque shows through.
                if let (Some(faces), Some(glass)) = (&self.faces, &self.glass) {
                    bind(&mut pass, &self.glass_pipeline, faces);
                    pass.set_index_buffer(glass.order.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..glass.count(), 0, 0..1);
                }
                draw(&mut pass, &self.overlay_pipeline, overlay.as_ref());
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
        flags: None,
    })
}

/// Uploads vertices with a flag buffer beside them, every flag 0 (wgpu zeroes new buffers),
/// or None when there are none to draw.
fn flagged_mesh<T: bytemuck::Pod>(
    device: &wgpu::Device,
    label: &str,
    vertices: &[T],
) -> Option<Mesh> {
    let mut mesh = mesh(device, label, vertices)?;
    mesh.flags = Some(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (vertices.len() * std::mem::size_of::<f32>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }));
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::back_to_front;

    #[test]
    fn glass_triangles_are_ordered_far_to_near() {
        let triangles = [
            (0, Vec3::new(0.0, 1.0, 0.0)),
            (3, Vec3::new(0.0, 5.0, 0.0)),
            (6, Vec3::new(0.0, 3.0, 0.0)),
        ];
        // An eye at the origin: the triangle 5 m away first.
        assert_eq!(
            back_to_front(&triangles, [0.0, 0.0, 0.0, 1.0]),
            [3, 4, 5, 6, 7, 8, 0, 1, 2]
        );
        // An eye beyond them looks the other way round.
        assert_eq!(
            back_to_front(&triangles, [0.0, 10.0, 0.0, 1.0]),
            [0, 1, 2, 6, 7, 8, 3, 4, 5]
        );
        // A parallel view looking along -y.
        assert_eq!(
            back_to_front(&triangles, [0.0, -1.0, 0.0, 0.0]),
            [0, 1, 2, 6, 7, 8, 3, 4, 5]
        );
    }
}
