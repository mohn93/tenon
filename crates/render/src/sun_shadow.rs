//! Sun shading and the sun's shadow map: one orthographic depth map along the sun's rays,
//! fitted to the casters, drawn again only when the sun or the casters change (see `SunMap`).

use glam::{Mat4, Vec2, Vec3, Vec4};

/// The side of the sun's shadow map in pixels: smaller on the web, where memory and fill rate
/// are tighter.
#[cfg(target_arch = "wasm32")]
pub const SUN_MAP_SIZE: u32 = 1024;
#[cfg(not(target_arch = "wasm32"))]
pub const SUN_MAP_SIZE: u32 = 2048;

const SUN_MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Faces at least this opaque cast the sun's shadow.
pub const SUN_CASTER_OPACITY: f32 = 0.7;

/// How far a shadow may reach over the ground past the casters, in casters' diagonals: a sun
/// near the horizon throws shadows far beyond anything worth drawing.
const GROUND_REACH: f32 = 3.0;

/// The map's border, in texels, so the PCF taps at the edge find what the map holds.
const BORDER_TEXELS: f32 = 4.0;

/// The sun as the renderer lights a view with it: the settings of `session::Document::shadows`
/// with the sun's direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunLight {
    /// The unit vector toward the sun.
    pub direction: Vec3,
    /// How much direct light adds to a face square-on to the sun, and how much ambient light a
    /// face in shadow keeps, 0 to 1.
    pub light: f32,
    pub dark: f32,
    /// Whether the ground shows shadows.
    pub on_ground: bool,
}

impl SunLight {
    /// The sun of `doc`'s shadow settings, or None when shadows are off.
    pub fn of(doc: &session::Document) -> Option<Self> {
        let shadows = doc.shadows;
        shadows.on.then(|| Self {
            direction: doc.sun().direction.as_vec3(),
            light: f32::from(shadows.light) / 100.0,
            dark: f32::from(shadows.dark) / 100.0,
            on_ground: shadows.on_ground,
        })
    }

    /// Whether the sun is above the horizon: below it only the dark level shades.
    pub fn is_up(&self) -> bool {
        self.direction.z > 0.0
    }
}

/// The casters' bounds, and where a map along `direction` reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunFit {
    /// World to the map's clip space: x and y -1 to 1 across the map, z 0 (near) to 1 (far).
    pub view_proj: Mat4,
    /// The world length of one texel, and of the map's depth range.
    pub texel: f32,
    pub depth_range: f32,
    /// The ground rectangle (x, y) shadows can fall on: the casters' footprint and where the
    /// sun throws their corners.
    pub ground: [Vec2; 2],
}

/// The map for casters inside the box `lo` to `hi` and a sun toward `direction` (above the
/// horizon), `size` pixels wide. It depends on nothing but those, so the view moving never
/// changes it (no shimmer while orbiting). Its edges lie on whole texels of a size that follows
/// the casters' span, so a change of casters may change the texel.
pub fn fit(direction: Vec3, lo: Vec3, hi: Vec3, size: u32) -> SunFit {
    let forward = -direction.normalize();
    let reference = if forward.z.abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Z
    };
    let right = forward.cross(reference).normalize();
    let up = right.cross(forward);

    let corners: Vec<Vec3> = (0..8)
        .map(|k| {
            Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();

    // Where the sun throws each corner on the ground, no farther than the reach.
    let diagonal = (hi - lo).length().max(1.0);
    let reach = GROUND_REACH * diagonal;
    let mut ground_lo = Vec2::new(lo.x, lo.y);
    let mut ground_hi = Vec2::new(hi.x, hi.y);
    for c in &corners {
        let along = c.z.max(0.0) / direction.z.max(1e-4);
        let offset = Vec2::new(-direction.x, -direction.y) * along;
        let offset = if offset.length() > reach {
            offset * (reach / offset.length())
        } else {
            offset
        };
        let g = Vec2::new(c.x, c.y) + offset;
        ground_lo = ground_lo.min(g);
        ground_hi = ground_hi.max(g);
    }
    let margin = 0.01 * (ground_hi - ground_lo).max_element().max(1.0);
    ground_lo -= Vec2::splat(margin);
    ground_hi += Vec2::splat(margin);

    // The map's sides: the casters alone; its depth: also the ground behind them.
    let light_space = |p: Vec3| Vec3::new(p.dot(right), p.dot(up), p.dot(forward));
    let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for c in &corners {
        let p = light_space(*c);
        min = min.min(p);
        max = max.max(p);
    }
    let (mut near, mut far) = (min.z, max.z);
    for (x, y) in [
        (ground_lo.x, ground_lo.y),
        (ground_hi.x, ground_lo.y),
        (ground_hi.x, ground_hi.y),
        (ground_lo.x, ground_hi.y),
    ] {
        let d = light_space(Vec3::new(x, y, 0.0)).z;
        near = near.min(d);
        far = far.max(d);
    }
    let pad = (0.02 * (far - near)).max(0.5);
    let (near, far) = (near - pad, far + pad);

    let span = (max.x - min.x).max(max.y - min.y).max(1e-3);
    let texel = span / (size as f32 - 2.0 * BORDER_TEXELS);
    let side = texel * size as f32;
    let snap = |v: f32| (v / texel).floor() * texel;
    let (x0, y0) = (
        snap((min.x + max.x) / 2.0 - side / 2.0),
        snap((min.y + max.y) / 2.0 - side / 2.0),
    );
    let range = far - near;
    // Rows of the matrix: clip = (right.p, up.p, forward.p) scaled and moved into the map.
    let sx = 2.0 / side;
    let rows = [
        right.extend(0.0) * sx - Vec4::new(0.0, 0.0, 0.0, 1.0 + 2.0 * x0 / side),
        up.extend(0.0) * sx - Vec4::new(0.0, 0.0, 0.0, 1.0 + 2.0 * y0 / side),
        forward.extend(0.0) / range - Vec4::new(0.0, 0.0, 0.0, near / range),
        Vec4::W,
    ];
    SunFit {
        view_proj: Mat4::from_cols(rows[0], rows[1], rows[2], rows[3]).transpose(),
        texel,
        depth_range: range,
        ground: [ground_lo, ground_hi],
    }
}

/// The sun's map pass: a depth-only pipeline drawing the casters into the map texture.
pub(crate) struct SunMap {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// The map, for lookups.
    pub view: wgpu::TextureView,
    /// The matrix and the casters' generation the map was last drawn for.
    drawn_for: Option<([u32; 16], u64)>,
    /// How many times the map has been drawn.
    pub drawn: u64,
}

impl SunMap {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/sun_shadow.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun map"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun map"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attribs = wgpu::vertex_attr_array![0 => Float32x3];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sun map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sun"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attribs,
                })],
            },
            // Both sides of a face cast.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SUN_MAP_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                // Against acne on faces nearly edge-on to the sun.
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sun"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun map matrix"),
            size: std::mem::size_of::<[f32; 16]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun map"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let view = depth_texture(device, "sun map", SUN_MAP_SIZE, true);
        Self {
            pipeline,
            uniform,
            group,
            view,
            drawn_for: None,
            drawn: 0,
        }
    }

    /// Records drawing the map for `view_proj` and the casters (`count` corners in `buffer`,
    /// as of `generation`), unless it holds exactly that already.
    pub fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &Mat4,
        generation: u64,
        casters: Option<(&wgpu::Buffer, u32)>,
    ) {
        let key = (view_proj.to_cols_array().map(f32::to_bits), generation);
        if self.drawn_for == Some(key) {
            return;
        }
        self.drawn_for = Some(key);
        self.drawn += 1;
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&view_proj.to_cols_array()),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sun map"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if let Some((buffer, count)) = casters {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..count, 0..1);
        }
    }
}

/// A square depth texture's view; `target` makes it a render attachment too.
pub(crate) fn depth_texture(
    device: &wgpu::Device,
    label: &str,
    size: u32,
    target: bool,
) -> wgpu::TextureView {
    let usage = if target {
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
    } else {
        wgpu::TextureUsages::TEXTURE_BINDING
    };
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SUN_MAP_FORMAT,
            usage,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    const HI: Vec3 = Vec3::new(2.0, 1.5, 1.0);

    fn corners() -> Vec<Vec3> {
        (0..8)
            .map(|k| {
                Vec3::new(
                    if k & 1 == 0 { LO.x } else { HI.x },
                    if k & 2 == 0 { LO.y } else { HI.y },
                    if k & 4 == 0 { LO.z } else { HI.z },
                )
            })
            .collect()
    }

    fn sun(azimuth_deg: f32, elevation_deg: f32) -> Vec3 {
        let (a, e) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
        Vec3::new(e.cos() * a.sin(), e.cos() * a.cos(), e.sin())
    }

    #[test]
    fn the_map_holds_every_caster_corner_and_the_ground_behind_them() {
        for (az, el) in [(0.0, 45.0), (200.0, 30.0), (90.0, 5.0), (10.0, 89.0)] {
            let d = sun(az, el);
            let f = fit(d, LO, HI, 2048);
            for c in corners() {
                let p = f.view_proj * c.extend(1.0);
                assert!(p.x.abs() < 1.0 && p.y.abs() < 1.0, "{az} {el}: {p:?}");
                assert!((0.0..1.0).contains(&p.z), "{az} {el}: depth {}", p.z);
            }
            // Ground points under the casters are inside the depth range, so a lookup
            // compares them with the casters' depth rather than past the far plane.
            for g in [f.ground[0], f.ground[1]] {
                let p = f.view_proj * Vec4::new(g.x, g.y, 0.0, 1.0);
                assert!(
                    (0.0..=1.0).contains(&p.z),
                    "{az} {el}: ground depth {}",
                    p.z
                );
            }
        }
    }

    #[test]
    fn the_ground_covers_the_shadow_of_the_box() {
        // Sun from the west at 45 degrees: the box's shadow falls east by its height.
        let f = fit(sun(270.0, 45.0), LO, HI, 2048);
        assert!(
            f.ground[0].x <= 0.0 && f.ground[1].x >= 3.0 - 1e-3,
            "{:?}",
            f.ground
        );
        assert!(f.ground[0].y <= 0.0 && f.ground[1].y >= 1.5);
    }

    #[test]
    fn a_low_sun_throws_shadows_no_farther_than_the_reach() {
        let f = fit(sun(270.0, 0.5), LO, HI, 2048);
        let diagonal = (HI - LO).length();
        assert!(
            f.ground[1].x < 2.0 + (GROUND_REACH + 0.1) * diagonal,
            "{:?}",
            f.ground
        );
    }

    #[test]
    fn the_map_does_not_depend_on_anything_but_the_sun_and_the_casters() {
        let a = fit(sun(100.0, 40.0), LO, HI, 2048);
        let b = fit(sun(100.0, 40.0), LO, HI, 2048);
        assert_eq!(a, b);
    }

    #[test]
    fn the_map_edges_lie_on_whole_texels() {
        let d = sun(120.0, 35.0);
        let f = fit(d, LO, HI, 2048);
        // A point whose light-space x is a whole number of texels lands on a texel boundary.
        let right = (-d).cross(Vec3::Z).normalize();
        for k in [3.0, 100.0, 517.0] {
            let p = f.view_proj * (right * (k * f.texel)).extend(1.0);
            let pixel = (p.x + 1.0) / 2.0 * 2048.0;
            assert!((pixel - pixel.round()).abs() < 0.05, "{k}: {pixel}");
        }
    }

    #[test]
    fn a_sun_straight_up_still_makes_a_map() {
        let f = fit(Vec3::Z, LO, HI, 2048);
        for c in corners() {
            let p = f.view_proj * c.extend(1.0);
            assert!(p.x.abs() < 1.0 && p.y.abs() < 1.0);
        }
    }
}
