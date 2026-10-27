# M2b Line Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Draw in the window: a Line tool with endpoint, midpoint, on-edge, axis, on-face and ground inference; faces and edges rendered with reversed-Z depth; undo/redo from the keyboard. Every edit still goes through `session::Command`.

**Architecture:** `kernel` gains face triangulation. `session::Document` gains a revision counter. `render` draws grid/axes, faces (front/back shaded), model edges, and an always-on-top overlay, with reversed-Z infinite-far projection and explicit bind group layouts. New `tools` crate (pure, no GPU): inference, the Line tool, and overlay geometry. `app` gains a window-independent `Editor` that routes input (navigation first, then the tool), executes the tool's commands on the document, and builds the overlay; `app.rs` only maps winit events to it.

**Tech Stack:** Rust 2024, earcutr 0.5.0 (new), wgpu 30.0.1, winit 0.30.13, glam 0.34.1, serde 1.0.229.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md` (milestone M2, second half; sections 2 Tools and inference, 3 Rendering, 4 App shell, 5 acceptance rule).

## Global Constraints

- Dependency direction: `app -> {tools, render} -> session -> kernel`; `render` may also use `kernel` types directly. `tools` has no GPU/window deps; `Editor` has no winit/wgpu deps.
- Tools never mutate the model: they emit `session::Command`s, executed through `Document::execute` (one command = one undo step).
- Meters, Z-up, f64 model math; f32 only for GPU data. Colors authored as sRGB, written to a non-sRGB target.
- Depth is reversed: clear 0.0, faces compare `Greater`, lines `GreaterEqual`; projection `directx::perspective_infinite_reverse`.
- Cursor and viewport are physical pixels; snap radius is 8 logical px times the window scale factor.
- Redraw on demand only.
- Pinned versions: earcutr `=0.5.0` (new); all existing pins unchanged.
- Git: work on `dev`; stage only files you changed; short plain commit messages, no emoji; `cargo fmt --all` before each commit.

## Spec notes (deviations)

- One tool exists, so `LineTool` is a struct with `handle(ToolEvent) -> ToolOutput`; the spec's `Tool` trait arrives with the second tool (M3).
- Model edges are 1-pixel `LineList` lines with a depth nudge (`clip.z * 1.002` in the shader) instead of screen-space wide lines; wide lines are deferred.
- Inference locks (Shift, arrow keys), parallel/perpendicular and intersection snaps are M4 ("full inference set").
- Face orientation is the kernel's canonical plane normal, so some walls show their back (blue-gray) side; reversing/orienting faces is later.

## Out of scope

egui, measurements box (M3), Rectangle/Push-Pull (M3), selection (M4), eye-relative rendering for large coordinates, picking acceleration (BVH).

## File Structure

```
Cargo.toml                          + earcutr
crates/kernel/Cargo.toml            + earcutr
crates/kernel/src/lib.rs            + mod mesh
crates/kernel/src/mesh.rs           Model::triangulate
crates/kernel/tests/mesh.rs
crates/session/src/document.rs      + revision counter
crates/session/src/camera.rs        reversed-Z infinite projection
crates/session/tests/commands.rs    + revision test
crates/render/Cargo.toml            + kernel
crates/render/src/lib.rs            + scene
crates/render/src/scene.rs          FaceVertex, model_geometry
crates/render/src/renderer.rs       4 pipelines, set_model, overlay
crates/render/src/shaders/scene.wgsl  replaces line.wgsl
crates/render/tests/headless.rs     + face/edge/overlay tests
crates/tools/*                      new crate: inference, line tool, overlay
crates/app/Cargo.toml               + tools
crates/app/src/navigation.rs        button() reports whether it consumed the event
crates/app/src/editor.rs            input routing, command execution, overlay
crates/app/src/app.rs               winit -> Editor, keyboard, model upload on revision change
crates/app/src/main.rs              + mod editor
```

---

### Task 1: Face triangulation

**Files:**
- Modify: `Cargo.toml`, `crates/kernel/Cargo.toml`, `crates/kernel/src/lib.rs`
- Create: `crates/kernel/src/mesh.rs`, `crates/kernel/tests/mesh.rs`

**Interfaces:**
- Produces: `Model::triangulate(&self, FaceId) -> Option<Vec<[DVec3; 3]>>`: triangles covering the face minus holes, each counter-clockwise around the face normal; None for a missing face or a triangulation failure.

- [ ] **Step 1: Dependency**

Root `Cargo.toml` `[workspace.dependencies]`: add `earcutr = "=0.5.0"`.
`crates/kernel/Cargo.toml` `[dependencies]`: add `earcutr.workspace = true`.
`crates/kernel/src/lib.rs`: add `mod mesh;` between `mod ids;` and `mod model;`.

- [ ] **Step 2: Write the failing tests**

`crates/kernel/tests/mesh.rs`:
```rust
use glam::DVec3;
use kernel::{FaceId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn polygon(m: &mut Model, pts: &[DVec3]) {
    for i in 0..pts.len() {
        m.add_edge(pts[i], pts[(i + 1) % pts.len()]).unwrap();
    }
}

fn square(m: &mut Model, x0: f64, y0: f64, s: f64) {
    polygon(
        m,
        &[
            p(x0, y0, 0.0),
            p(x0 + s, y0, 0.0),
            p(x0 + s, y0 + s, 0.0),
            p(x0, y0 + s, 0.0),
        ],
    );
}

/// Sum of triangle areas, asserting each triangle winds counter-clockwise around `normal`.
fn covered_area(tris: &[[DVec3; 3]], normal: DVec3) -> f64 {
    tris.iter()
        .map(|t| {
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            assert!(n.dot(normal) > 0.0, "triangle winds the wrong way");
            n.length() * 0.5
        })
        .sum()
}

#[test]
fn square_face_triangulates_to_its_area() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let (id, f) = m.faces().next().unwrap();
    let tris = m.triangulate(id).unwrap();
    assert_eq!(tris.len(), 2);
    assert!((covered_area(&tris, f.normal) - 4.0).abs() < 1e-9);
}

#[test]
fn face_with_hole_triangulates_around_the_hole() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    let (id, f) = m.faces().find(|(_, f)| !f.holes.is_empty()).unwrap();
    let tris = m.triangulate(id).unwrap();
    assert!((covered_area(&tris, f.normal) - 15.0).abs() < 1e-9);
    // No triangle covers the hole's center.
    let c = p(1.5, 1.5, 0.0);
    for t in &tris {
        let inside = (0..3).all(|i| (t[(i + 1) % 3] - t[i]).cross(c - t[i]).dot(f.normal) > 0.0);
        assert!(!inside, "a triangle covers the hole");
    }
}

#[test]
fn tilted_and_concave_faces_triangulate() {
    let mut m = Model::new();
    // L-shaped (concave) face on a tilted plane z = x.
    polygon(
        &mut m,
        &[
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 2.0),
            p(2.0, 1.0, 2.0),
            p(1.0, 1.0, 1.0),
            p(1.0, 2.0, 1.0),
            p(0.0, 2.0, 0.0),
        ],
    );
    let (id, f) = m.faces().next().unwrap();
    let tris = m.triangulate(id).unwrap();
    assert!((covered_area(&tris, f.normal) - m.area(id).unwrap()).abs() < 1e-9);
}

#[test]
fn missing_face_triangulates_to_none() {
    assert!(Model::new().triangulate(FaceId(5)).is_none());
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p kernel --test mesh`
Expected: compile error (`file not found for module mesh` or no method `triangulate`).

- [ ] **Step 4: Implement**

`crates/kernel/src/mesh.rs`:
```rust
//! Triangulation of faces for rendering and picking.

use glam::DVec3;

use crate::geom::Plane;
use crate::ids::FaceId;
use crate::model::Model;

impl Model {
    /// Triangles covering a face (outer loop minus holes), each wound counter-clockwise around
    /// the face normal. None if the face does not exist or cannot be triangulated.
    pub fn triangulate(&self, id: FaceId) -> Option<Vec<[DVec3; 3]>> {
        let face = self.face(id)?;
        let origin = self.position(*face.outer.first()?)?;
        let plane = Plane {
            normal: face.normal,
            d: face.normal.dot(origin),
        };

        let mut points = Vec::new();
        let mut coords = Vec::new();
        let mut hole_starts = Vec::new();
        for (k, lp) in std::iter::once(&face.outer).chain(&face.holes).enumerate() {
            if k > 0 {
                hole_starts.push(points.len());
            }
            for v in lp {
                let p = self.position(*v)?;
                let q = plane.project(p);
                points.push(p);
                coords.extend([q.x, q.y]);
            }
        }

        let indices = earcutr::earcut(&coords, &hole_starts, 2).ok()?;
        let tris = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| {
                let t = [points[c[0]], points[c[1]], points[c[2]]];
                if (t[1] - t[0]).cross(t[2] - t[0]).dot(face.normal) < 0.0 {
                    [t[0], t[2], t[1]]
                } else {
                    t
                }
            })
            .collect();
        Some(tris)
    }
}
```

- [ ] **Step 5: Verify**

Run: `cargo test -p kernel && cargo clippy -p kernel --all-targets -- -D warnings`
Expected: mesh 4 passed; all other kernel tests unchanged; no warnings. Workspace total 102.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock crates/kernel/Cargo.toml crates/kernel/src/lib.rs crates/kernel/src/mesh.rs crates/kernel/tests/mesh.rs
git commit -m "Add face triangulation"
```

---

### Task 2: Document revision counter

**Files:**
- Modify: `crates/session/src/document.rs`, `crates/session/tests/commands.rs`

**Interfaces:**
- Produces: `Document::revision(&self) -> u64`, incremented once per recorded edit, undo, and redo; unchanged by no-op edits and camera commands. Clients rebuild model-derived data when it changes.

- [ ] **Step 1: Write the failing test**

Append to `crates/session/tests/commands.rs`:
```rust
#[test]
fn revision_counts_model_changes_only() {
    let mut d = doc();
    assert_eq!(d.revision(), 0);
    edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    assert_eq!(d.revision(), 1);
    edge(&mut d, [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert_eq!(d.revision(), 1, "a no-op edit does not change the model");
    run(&mut d, json!({"type": "set_camera", "eye": [10.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "fov_y_deg": null})).unwrap();
    assert_eq!(d.revision(), 1, "camera moves do not change the model");
    run(&mut d, json!({"type": "undo"})).unwrap();
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(d.revision(), 3);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p session --test commands revision`
Expected: compile error, no method `revision`.

- [ ] **Step 3: Implement**

In `crates/session/src/document.rs`:
- add a field `revision: u64,` after `redo: Vec<ChangeSet>,` in `Document`, initialized to `0` in `new`;
- add after `pub fn model(...)`:
```rust
    /// Increases every time the model changes (edits, undo, redo). Clients compare it to decide
    /// whether to rebuild anything derived from the model.
    pub fn revision(&self) -> u64 {
        self.revision
    }
```
- add `self.revision += 1;` right after `self.model.apply(&inverse);` (undo), right after `self.model.apply(&changes);` (redo), and inside `record` right after `self.redo.clear();` (only recorded, non-empty edits).

- [ ] **Step 4: Verify**

Run: `cargo test -p session && cargo clippy -p session --all-targets -- -D warnings`
Expected: commands 10 passed; workspace total 103.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/session/src/document.rs crates/session/tests/commands.rs
git commit -m "Add document revision counter"
```

---

### Task 3: Render faces, edges and an overlay with reversed-Z depth

**Files:**
- Modify: `crates/session/src/camera.rs` (projection only), `crates/render/Cargo.toml`, `crates/render/src/lib.rs`, `crates/render/src/renderer.rs` (rewrite), `crates/render/tests/headless.rs` (rewrite), `crates/app/src/app.rs` (one call)
- Create: `crates/render/src/scene.rs`, `crates/render/src/shaders/scene.wgsl`
- Delete: `crates/render/src/shaders/line.wgsl`

**Interfaces:**
- Consumes: `Model::triangulate` (Task 1), `Model::{faces, edges, position}`.
- Produces:
  - `render::scene::{FaceVertex { pos: [f32; 3], normal: [f32; 3] }, EDGE_COLOR, ModelGeometry { faces, edges }, model_geometry(&Model) -> ModelGeometry}`
  - `Renderer::new(device, queue, format)` (unchanged), `Renderer::set_model(&mut self, &Model)`, `Renderer::render(&mut self, target, size, camera, overlay: &[LineVertex])` (new `overlay` argument: line pairs drawn on top of everything)
  - `Camera::proj` is reversed-Z with an infinite far plane.
  - Draw order: grid/axes (pushed slightly behind ground faces), faces (front light gray, back blue-gray, headlight shading), model edges (pulled slightly toward the camera), overlay (no depth test).

- [ ] **Step 1: Write the failing tests**

`crates/render/Cargo.toml` becomes:
```toml
[package]
name = "render"
version.workspace = true
edition.workspace = true

[dependencies]
session = { path = "../session" }
kernel = { path = "../kernel" }
glam.workspace = true
bytemuck.workspace = true
wgpu.workspace = true

[dev-dependencies]
pollster.workspace = true
```

Replace `crates/render/tests/headless.rs` with:
```rust
use glam::{DVec2, DVec3};
use kernel::Model;
use render::Renderer;
use render::grid::{BACKGROUND, LineVertex};
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
    renderer.set_model(model);
    renderer.render(&view, (W, H), camera, overlay);

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
            f(pixel(
                px,
                (s.x as i32 + dx) as u32,
                (s.y as i32 + dy) as u32,
            ))
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
```

Create `crates/render/src/scene.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn empty_model_has_no_geometry() {
        assert_eq!(model_geometry(&Model::new()), ModelGeometry::default());
    }

    #[test]
    fn square_gives_two_triangles_and_four_edges() {
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        for i in 0..4 {
            m.add_edge(pts[i], pts[(i + 1) % 4]).unwrap();
        }
        let g = model_geometry(&m);
        assert_eq!(g.faces.len(), 6);
        assert!(g.faces.iter().all(|v| v.normal == [0.0, 0.0, 1.0]));
        assert_eq!(g.edges.len(), 8);
        assert!(g.edges.iter().all(|v| v.color == EDGE_COLOR));
    }
}
```

`crates/render/src/lib.rs`:
```rust
pub mod grid;
pub mod renderer;
pub mod scene;

pub use renderer::Renderer;
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p render`
Expected: compile errors (no `set_model`, `render` takes 3 arguments, `model_geometry` missing).

- [ ] **Step 3: Implement**

Prepend to `crates/render/src/scene.rs` (above the tests):
```rust
//! GPU vertex data built from a kernel model.

use bytemuck::{Pod, Zeroable};
use kernel::Model;

use crate::grid::LineVertex;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];

/// Triangles for every face and line pairs for every edge.
#[derive(Debug, Default, PartialEq)]
pub struct ModelGeometry {
    pub faces: Vec<FaceVertex>,
    pub edges: Vec<LineVertex>,
}

pub fn model_geometry(model: &Model) -> ModelGeometry {
    let mut g = ModelGeometry::default();
    for (id, face) in model.faces() {
        let normal = face.normal.as_vec3().to_array();
        for tri in model.triangulate(id).unwrap_or_default() {
            for p in tri {
                g.faces.push(FaceVertex {
                    pos: p.as_vec3().to_array(),
                    normal,
                });
            }
        }
    }
    for (_, edge) in model.edges() {
        for v in edge.v {
            if let Some(p) = model.position(v) {
                g.edges.push(LineVertex {
                    pos: p.as_vec3().to_array(),
                    color: EDGE_COLOR,
                });
            }
        }
    }
    g
}
```

Delete the old shader and add the new one:
```bash
git rm crates/render/src/shaders/line.wgsl
```
`crates/render/src/shaders/scene.wgsl`:
```wgsl
struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Depth is reversed (1 = near, 0 = far), so scaling clip z by more than 1 pulls a line slightly
// toward the camera and by less than 1 pushes it away, by a fixed fraction of its distance.
// z is not clamped: vertices behind the camera must keep a positive z so near-plane clipping works.
fn line_vertex(pos: vec3<f32>, color: vec4<f32>, depth_scale: f32) -> LineOut {
    var out: LineOut;
    let clip = u.view_proj * vec4<f32>(pos, 1.0);
    out.pos = vec4<f32>(clip.xy, clip.z * depth_scale, clip.w);
    out.color = color;
    return out;
}

// Model edges: drawn over the faces they bound.
@vertex
fn vs_edge(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.002);
}

// Grid and axes: drawn under faces lying on the ground.
@vertex
fn vs_ground(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 0.998);
}

// Tool previews and inference markers: drawn without depth testing.
@vertex
fn vs_overlay(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    return line_vertex(pos, color, 1.0);
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct FaceOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

const FRONT: vec3<f32> = vec3<f32>(0.93, 0.93, 0.90);
const BACK: vec3<f32> = vec3<f32>(0.60, 0.65, 0.76);

@vertex
fn vs_face(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>) -> FaceOut {
    var out: FaceOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    out.normal = normal;
    return out;
}

// Front sides are light, back sides blue-gray; a headlight at the eye shades both.
@fragment
fn fs_face(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(in.normal) * select(-1.0, 1.0, front);
    let to_eye = normalize(u.eye.xyz - in.world);
    let shade = 0.6 + 0.4 * max(dot(n, to_eye), 0.0);
    let base = select(BACK, FRONT, front);
    return vec4<f32>(base * shade, 1.0);
}
```

Replace `crates/render/src/renderer.rs` with:
```rust
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
```

In `crates/session/src/camera.rs`, replace the `proj` method with:
```rust
    /// Reversed-Z perspective with an infinite far plane: depth is 1 at the near plane and
    /// approaches 0 with distance, so nothing far away is ever clipped.
    pub fn proj(&self) -> DMat4 {
        let near = (self.distance() * 1e-3).max(1e-5);
        directx::perspective_infinite_reverse(self.fov_y, self.aspect(), near)
    }
```

In `crates/app/src/app.rs`, the render call gains an empty overlay (the app is wired fully in Task 6):
```rust
        self.renderer.render(
            &view,
            (self.config.width, self.config.height),
            &self.camera,
            &[],
        );
```

- [ ] **Step 4: Verify**

Run: `cargo test --workspace 2>&1 | grep "test result" && cargo clippy --workspace --all-targets -- -D warnings`
Expected: render 6 unit + 6 headless passed (headless must actually render on this Mac: no "no GPU adapter" message with `-- --nocapture`); workspace total 109; no warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/session/src/camera.rs crates/render/Cargo.toml crates/render/src/lib.rs crates/render/src/scene.rs crates/render/src/renderer.rs crates/render/src/shaders/scene.wgsl crates/render/tests/headless.rs crates/app/src/app.rs Cargo.lock
git commit -m "Render faces, edges and overlay with reversed-Z depth"
```

---

### Task 4: `tools` crate with inference

**Files:**
- Create: `crates/tools/Cargo.toml`, `crates/tools/src/lib.rs`, `crates/tools/src/inference.rs`

**Interfaces:**
- Consumes: `session::{Camera, Ray}` (`ray`, `project`, `anchor_at`, `distance`), `kernel::{Model, geom::{Plane, point_in_polygon}}`.
- Produces:
  - `Axis { X, Y, Z }` with `ALL` and `dir()`
  - `SnapKind { Endpoint, Midpoint, Origin, OnEdge, Axis(Axis), OnFace, Ground, Free }`
  - `Inference { point: DVec3, kind: SnapKind }`
  - `infer(model: &Model, camera: &Camera, cursor: DVec2, anchor: Option<DVec3>, radius_px: f64) -> Inference`. Ranking: endpoint/midpoint/origin (nearest on screen) > on-edge > axis from anchor > on-face (nearest along the ray, holes excluded) > ground (within 50 x camera distance) > free (`camera.anchor_at`).

- [ ] **Step 1: Crate skeleton**

`crates/tools/Cargo.toml`:
```toml
[package]
name = "tools"
version.workspace = true
edition.workspace = true

[dependencies]
kernel = { path = "../kernel" }
session = { path = "../session" }
glam.workspace = true
```

`crates/tools/src/lib.rs`:
```rust
//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod inference;

pub use inference::{Axis, Inference, SnapKind, infer};
```

- [ ] **Step 2: Write the failing tests**

Create `crates/tools/src/inference.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 10.0;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    fn px(c: &Camera, p: DVec3) -> DVec2 {
        c.project(p).unwrap()
    }

    fn square(m: &mut Model, z: f64) {
        let c = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, z), DVec3::new(b.0, b.1, z))
                .unwrap();
        }
    }

    #[test]
    fn snaps_to_a_nearby_endpoint_exactly() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(3.0, 1.0, 0.0)) + DVec2::new(4.0, -3.0),
            None,
            R,
        );
        assert_eq!(
            hit,
            Inference {
                point: DVec3::new(3.0, 1.0, 0.0),
                kind: SnapKind::Endpoint
            }
        );
    }

    #[test]
    fn snaps_to_midpoints_and_the_origin() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let mid = infer(&m, &c, px(&c, DVec3::new(2.0, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert!(mid.point.abs_diff_eq(DVec3::new(2.0, 1.0, 0.0), 1e-12));
        let origin = infer(&m, &c, px(&c, DVec3::ZERO) + DVec2::new(2.0, 2.0), None, R);
        assert_eq!(origin.kind, SnapKind::Origin);
    }

    #[test]
    fn snaps_onto_an_edge_between_its_points() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(5.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(4.2, 1.0, 0.0)) + DVec2::new(0.0, 3.0),
            None,
            R,
        );
        assert_eq!(hit.kind, SnapKind::OnEdge);
        assert!((hit.point.y - 1.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
        assert!((hit.point.x - 4.2).abs() < 0.1);
    }

    #[test]
    fn follows_an_axis_from_the_anchor() {
        let c = cam();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let target = DVec3::new(1.0, 1.0, 2.5); // straight up (blue)
        let hit = infer(
            &Model::new(),
            &c,
            px(&c, target) + DVec2::new(5.0, 0.0),
            Some(anchor),
            R,
        );
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
        assert!(hit.point.x == 1.0 && hit.point.y == 1.0);
        assert!((hit.point.z - 2.5).abs() < 0.2);
    }

    #[test]
    fn points_beat_axes() {
        let (c, mut m) = (cam(), Model::new());
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        m.add_edge(DVec3::new(2.0, 3.0, 0.0), DVec3::new(2.0, 5.0, 0.0))
            .unwrap();
        let hit = infer(&m, &c, px(&c, DVec3::new(2.0, 3.0, 0.0)), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
    }

    #[test]
    fn lands_on_faces_then_ground_then_free() {
        let (c, mut m) = (cam(), Model::new());
        square(&mut m, 1.0);
        let on_face = infer(&m, &c, px(&c, DVec3::new(1.3, 0.7, 1.0)), None, R);
        assert_eq!(on_face.kind, SnapKind::OnFace);
        assert!((on_face.point.z - 1.0).abs() < 1e-9);

        let ground = infer(
            &Model::new(),
            &c,
            px(&c, DVec3::new(-3.0, 4.0, 0.0)),
            None,
            R,
        );
        assert_eq!(ground.kind, SnapKind::Ground);
        assert!(ground.point.z.abs() < 1e-9);

        let mut level = cam();
        level.eye = DVec3::new(0.0, -10.0, 1.0);
        level.target = DVec3::new(0.0, 0.0, 1.0);
        let sky = infer(&Model::new(), &level, DVec2::new(400.0, 0.0), None, R);
        assert_eq!(sky.kind, SnapKind::Free);
    }

    #[test]
    fn inner_face_inside_another_faces_hole_is_hit() {
        let (c, mut m) = (cam(), Model::new());
        let pts = [(-2.0, -2.0), (4.0, -2.0), (4.0, 4.0), (-2.0, 4.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, 1.0), DVec3::new(b.0, b.1, 1.0))
                .unwrap();
        }
        square(&mut m, 1.0); // inner square (0,0)-(2,2): a face and a hole of the outer face
        let inside_inner = infer(&m, &c, px(&c, DVec3::new(1.2, 0.8, 1.0)), None, R);
        assert_eq!(inside_inner.kind, SnapKind::OnFace);
        assert!((inside_inner.point.z - 1.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p tools`
Expected: compile errors (`cannot find function infer`, ...).

- [ ] **Step 4: Implement**

Prepend to `crates/tools/src/inference.rs`:
```rust
//! Snapping the cursor to meaningful 3D points.

use glam::{DVec2, DVec3};
use kernel::Model;
use kernel::geom::{Plane, point_in_polygon};
use session::{Camera, Ray};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn dir(self) -> DVec3 {
        match self {
            Axis::X => DVec3::X,
            Axis::Y => DVec3::Y,
            Axis::Z => DVec3::Z,
        }
    }
}

/// What the cursor snapped to, from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Origin,
    OnEdge,
    /// Along a red/green/blue axis through the point the tool is drawing from.
    Axis(Axis),
    OnFace,
    Ground,
    /// Nothing to snap to: a point in front of the camera under the cursor.
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inference {
    pub point: DVec3,
    pub kind: SnapKind,
}

/// Finds the 3D point under `cursor` (physical pixels). `anchor` is the point a tool is drawing
/// from, enabling axis inference. Snap candidates count only within `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > on-edge > axis > on-face >
/// ground > free.
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let ray = camera.ray(cursor);
    let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
    let within = |p: DVec3| screen_dist(p).filter(|d| *d <= radius_px);

    // Points: vertices, edge midpoints, origin.
    let mut best: Option<(f64, Inference)> = None;
    let offer = |p: DVec3, kind: SnapKind, best: &mut Option<(f64, Inference)>| {
        if let Some(d) = within(p)
            && best.as_ref().is_none_or(|(bd, _)| d < *bd)
        {
            *best = Some((d, Inference { point: p, kind }));
        }
    };
    for (_, v) in model.vertices() {
        offer(v.pos, SnapKind::Endpoint, &mut best);
    }
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            offer((a + b) * 0.5, SnapKind::Midpoint, &mut best);
        }
    }
    offer(DVec3::ZERO, SnapKind::Origin, &mut best);
    if let Some((_, hit)) = best {
        return hit;
    }

    // On an edge.
    let mut on_edge: Option<(f64, DVec3)> = None;
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            if let Some(d) = within(p)
                && on_edge.is_none_or(|(bd, _)| d < bd)
            {
                on_edge = Some((d, p));
            }
        }
    }
    if let Some((_, point)) = on_edge {
        return Inference {
            point,
            kind: SnapKind::OnEdge,
        };
    }

    // Along an axis from the anchor.
    if let Some(anchor) = anchor {
        let mut on_axis: Option<(f64, Inference)> = None;
        for axis in Axis::ALL {
            let p = closest_on_line_to_ray(anchor, axis.dir(), &ray, false);
            if let Some(d) = within(p)
                && p.distance(anchor) > 0.0
                && on_axis.is_none_or(|(bd, _)| d < bd)
            {
                on_axis = Some((
                    d,
                    Inference {
                        point: p,
                        kind: SnapKind::Axis(axis),
                    },
                ));
            }
        }
        if let Some((_, hit)) = on_axis {
            return hit;
        }
    }

    // On a face (nearest along the ray).
    if let Some(point) = nearest_face_hit(model, &ray) {
        return Inference {
            point,
            kind: SnapKind::OnFace,
        };
    }

    match ray.hit_ground() {
        Some(point) if point.distance(ray.origin) <= camera.distance() * 50.0 => Inference {
            point,
            kind: SnapKind::Ground,
        },
        _ => Inference {
            point: camera.anchor_at(cursor),
            kind: SnapKind::Free,
        },
    }
}

/// Point on the line `origin + t * dir` closest to the ray; `t` is clamped to [0, 1] when
/// `segment` is true (dir is then the segment vector).
fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segment: bool) -> DVec3 {
    let w0 = origin - ray.origin;
    let a = dir.dot(dir);
    let b = dir.dot(ray.dir);
    let c = ray.dir.dot(ray.dir);
    let d = dir.dot(w0);
    let e = ray.dir.dot(w0);
    let denom = a * c - b * b;
    let mut t = if denom.abs() < 1e-12 * a * c {
        0.0
    } else {
        (b * e - c * d) / denom
    };
    if segment {
        t = t.clamp(0.0, 1.0);
    }
    origin + dir * t
}

fn nearest_face_hit(model: &Model, ray: &Ray) -> Option<DVec3> {
    let mut best: Option<(f64, DVec3)> = None;
    for (_, f) in model.faces() {
        let Some(origin) = f.outer.first().and_then(|v| model.position(*v)) else {
            continue;
        };
        let plane = Plane {
            normal: f.normal,
            d: f.normal.dot(origin),
        };
        let denom = plane.normal.dot(ray.dir);
        if denom.abs() < 1e-12 {
            continue;
        }
        let t = -plane.distance(ray.origin) / denom;
        if t <= 0.0 || best.is_some_and(|(bt, _)| t >= bt) {
            continue;
        }
        let p = ray.origin + ray.dir * t;
        let project = |l: &Vec<kernel::VertexId>| -> Vec<DVec2> {
            l.iter()
                .filter_map(|v| model.position(*v))
                .map(|q| plane.project(q))
                .collect()
        };
        let q = plane.project(p);
        if point_in_polygon(q, &project(&f.outer))
            && !f.holes.iter().any(|h| point_in_polygon(q, &project(h)))
        {
            best = Some((t, p));
        }
    }
    best.map(|(_, p)| p)
}
```

- [ ] **Step 5: Verify**

Run: `cargo test -p tools && cargo clippy -p tools --all-targets -- -D warnings`
Expected: 7 passed; workspace total 116; no warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/tools Cargo.lock
git commit -m "Add tools crate with inference"
```

---

### Task 5: Line tool and overlay geometry

**Files:**
- Create: `crates/tools/src/line.rs`, `crates/tools/src/overlay.rs`
- Modify: `crates/tools/src/lib.rs`

**Interfaces:**
- Consumes: `Inference`, `SnapKind`, `Axis` (Task 4); `session::Command`; `kernel::geom::LENGTH_TOL`.
- Produces:
  - `ToolEvent { Move(Inference), Click(Inference), Cancel }`; `ToolOutput { commands: Vec<Command>, preview: Vec<[DVec3; 2]> }`
  - `LineTool::new()`, `anchor() -> Option<DVec3>`, `handle(ToolEvent) -> ToolOutput`: first click starts a chain; each further click emits one `Command::AddEdge` from the last point and continues from the clicked point; clicking the chain's first point closes and ends it; clicking the last point again does nothing; `Cancel` drops the chain.
  - `tools::overlay::{Segment { a, b, color }, overlay(preview, hit: Option<&Inference>, camera) -> Vec<Segment>}` plus color constants: the preview is axis-colored when the hit is an axis inference; endpoint/origin, midpoint, on-edge and on-face hits get a square marker about 10 px wide on screen; ground/free/axis hits get no marker.

- [ ] **Step 1: Exports**

`crates/tools/src/lib.rs` becomes:
```rust
//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod inference;
pub mod line;
pub mod overlay;

pub use inference::{Axis, Inference, SnapKind, infer};
pub use line::{LineTool, ToolEvent, ToolOutput};
```

- [ ] **Step 2: Write the failing tests**

Create `crates/tools/src/line.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;

    fn at(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
        }
    }

    fn edge(a: (f64, f64), b: (f64, f64)) -> Command {
        Command::AddEdge {
            start: [a.0, a.1, 0.0],
            end: [b.0, b.1, 0.0],
        }
    }

    #[test]
    fn first_click_starts_and_moving_previews() {
        let mut t = LineTool::new();
        assert!(t.handle(ToolEvent::Click(at(0.0, 0.0))).commands.is_empty());
        let out = t.handle(ToolEvent::Move(at(2.0, 0.0)));
        assert_eq!(out.preview, vec![[DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)]]);
        assert_eq!(t.anchor(), Some(DVec3::ZERO));
    }

    #[test]
    fn clicks_chain_edges() {
        let mut t = LineTool::new();
        t.handle(ToolEvent::Click(at(0.0, 0.0)));
        let a = t.handle(ToolEvent::Click(at(2.0, 0.0)));
        let b = t.handle(ToolEvent::Click(at(2.0, 1.0)));
        assert_eq!(a.commands, vec![edge((0.0, 0.0), (2.0, 0.0))]);
        assert_eq!(b.commands, vec![edge((2.0, 0.0), (2.0, 1.0))]);
        assert_eq!(t.anchor(), Some(DVec3::new(2.0, 1.0, 0.0)));
    }

    #[test]
    fn clicking_the_first_point_closes_the_chain() {
        let mut t = LineTool::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)] {
            t.handle(ToolEvent::Click(at(x, y)));
        }
        let out = t.handle(ToolEvent::Click(at(0.0, 0.0)));
        assert_eq!(out.commands, vec![edge((1.0, 1.0), (0.0, 0.0))]);
        assert_eq!(t.anchor(), None);
        assert!(out.preview.is_empty());
    }

    #[test]
    fn clicking_the_same_point_twice_does_nothing() {
        let mut t = LineTool::new();
        t.handle(ToolEvent::Click(at(1.0, 1.0)));
        assert!(t.handle(ToolEvent::Click(at(1.0, 1.0))).commands.is_empty());
        assert_eq!(t.anchor(), Some(DVec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn cancel_drops_the_chain() {
        let mut t = LineTool::new();
        t.handle(ToolEvent::Click(at(0.0, 0.0)));
        let out = t.handle(ToolEvent::Cancel);
        assert!(out.preview.is_empty() && out.commands.is_empty());
        assert_eq!(t.anchor(), None);
    }
}
```

Create `crates/tools/src/overlay.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    #[test]
    fn preview_takes_the_axis_color() {
        let hit = Inference {
            point: DVec3::new(0.0, 3.0, 0.0),
            kind: SnapKind::Axis(Axis::Y),
        };
        let segs = overlay(&[[DVec3::ZERO, hit.point]], Some(&hit), &cam());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].color, GREEN);
    }

    #[test]
    fn endpoint_marker_is_a_square_of_constant_screen_size() {
        let c = cam();
        for p in [DVec3::new(1.0, 0.0, 0.0), DVec3::new(30.0, 40.0, 0.0)] {
            let hit = Inference {
                point: p,
                kind: SnapKind::Endpoint,
            };
            let segs = overlay(&[], Some(&hit), &c);
            assert_eq!(segs.len(), 4);
            assert!(segs.iter().all(|s| s.color == ENDPOINT));
            let side_px = c
                .project(segs[0].a)
                .unwrap()
                .distance(c.project(segs[0].b).unwrap());
            assert!(
                (side_px - 2.0 * MARKER_PX).abs() < 0.5,
                "marker side {side_px}px"
            );
        }
    }

    #[test]
    fn ground_hits_have_no_marker() {
        let hit = Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Ground,
        };
        assert!(overlay(&[], Some(&hit), &cam()).is_empty());
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p tools`
Expected: compile errors (`cannot find type LineTool`, `cannot find function overlay`, ...).

- [ ] **Step 4: Implement**

Prepend to `crates/tools/src/line.rs`:
```rust
//! The Line tool: click to start, click again to place each edge; edges chain from the last point.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::inference::Inference;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// Escape: abandon the current chain.
    Cancel,
}

/// What a tool wants done after an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    /// Commands to execute, in order (each is one undo step).
    pub commands: Vec<Command>,
    /// Segments to draw as a live preview.
    pub preview: Vec<[DVec3; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct LineTool {
    /// First point of the current chain; clicking it again closes the chain.
    first: Option<DVec3>,
    /// Point the next edge starts from.
    last: Option<DVec3>,
    /// Latest inferred cursor point.
    hover: Option<DVec3>,
}

impl LineTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The point the next edge would start from, if a chain is in progress.
    pub fn anchor(&self) -> Option<DVec3> {
        self.last
    }

    pub fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                let p = hit.point;
                self.hover = Some(p);
                match self.last {
                    None => {
                        self.first = Some(p);
                        self.last = Some(p);
                    }
                    Some(last) if last.distance(p) < LENGTH_TOL => {}
                    Some(last) => {
                        out.commands.push(Command::AddEdge {
                            start: last.to_array(),
                            end: p.to_array(),
                        });
                        if self.first.is_some_and(|f| f.distance(p) < LENGTH_TOL) {
                            *self = Self {
                                hover: self.hover,
                                ..Self::default()
                            };
                        } else {
                            self.last = Some(p);
                        }
                    }
                }
            }
            ToolEvent::Cancel => {
                *self = Self {
                    hover: self.hover,
                    ..Self::default()
                }
            }
        }
        if let (Some(a), Some(b)) = (self.last, self.hover)
            && a.distance(b) >= LENGTH_TOL
        {
            out.preview.push([a, b]);
        }
        out
    }
}
```

Prepend to `crates/tools/src/overlay.rs`:
```rust
//! What tools draw on top of the scene: the preview line and the inference marker.

use glam::DVec3;
use session::Camera;

use crate::inference::{Axis, Inference, SnapKind};

pub type Color = [f32; 4];

pub const PREVIEW: Color = [0.1, 0.1, 0.1, 1.0];
pub const RED: Color = [0.85, 0.15, 0.15, 1.0];
pub const GREEN: Color = [0.15, 0.65, 0.20, 1.0];
pub const BLUE: Color = [0.15, 0.30, 0.85, 1.0];
pub const ENDPOINT: Color = [0.10, 0.60, 0.10, 1.0];
pub const MIDPOINT: Color = [0.10, 0.70, 0.80, 1.0];
pub const ON_EDGE: Color = [0.85, 0.10, 0.10, 1.0];
pub const ON_FACE: Color = [0.20, 0.25, 0.85, 1.0];

/// Marker half-size in pixels.
const MARKER_PX: f64 = 5.0;

/// A colored world-space segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: DVec3,
    pub b: DVec3,
    pub color: Color,
}

/// Preview segments (axis-colored when following an axis) plus a marker at the inference point
/// sized to stay about the same on screen.
pub fn overlay(preview: &[[DVec3; 2]], hit: Option<&Inference>, camera: &Camera) -> Vec<Segment> {
    let line_color = match hit.map(|h| h.kind) {
        Some(SnapKind::Axis(Axis::X)) => RED,
        Some(SnapKind::Axis(Axis::Y)) => GREEN,
        Some(SnapKind::Axis(Axis::Z)) => BLUE,
        _ => PREVIEW,
    };
    let mut out: Vec<Segment> = preview
        .iter()
        .map(|[a, b]| Segment {
            a: *a,
            b: *b,
            color: line_color,
        })
        .collect();

    let Some(hit) = hit else { return out };
    let color = match hit.kind {
        SnapKind::Endpoint | SnapKind::Origin => ENDPOINT,
        SnapKind::Midpoint => MIDPOINT,
        SnapKind::OnEdge => ON_EDGE,
        SnapKind::OnFace => ON_FACE,
        _ => return out,
    };
    let depth = (hit.point - camera.eye).dot(camera.forward()).max(1e-6);
    let size = MARKER_PX * 2.0 * depth * (camera.fov_y * 0.5).tan() / camera.viewport.y;
    let (r, u) = (camera.right() * size, camera.up() * size);
    let p = hit.point;
    let corners = [p - r - u, p + r - u, p + r + u, p - r + u];
    for i in 0..4 {
        out.push(Segment {
            a: corners[i],
            b: corners[(i + 1) % 4],
            color,
        });
    }
    out
}
```

- [ ] **Step 5: Verify**

Run: `cargo test -p tools && cargo clippy -p tools --all-targets -- -D warnings`
Expected: 15 passed; workspace total 124; no warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/tools/src/lib.rs crates/tools/src/line.rs crates/tools/src/overlay.rs
git commit -m "Add Line tool and overlay geometry"
```

---

### Task 6: Editor and app wiring

**Files:**
- Modify: `crates/app/Cargo.toml`, `crates/app/src/navigation.rs`, `crates/app/src/main.rs`, `crates/app/src/app.rs`
- Create: `crates/app/src/editor.rs`

**Interfaces:**
- Consumes: `Document` (+ `revision`), `LineTool`, `infer`, `overlay`, `Renderer::{set_model, render}`, `render::grid::LineVertex`.
- Produces:
  - `Navigation::button(...) -> bool`: true when navigation used the event (middle press/release, alt+left press, release of the drag's own button); the tool only sees unconsumed left presses.
  - `Editor`: `new(viewport, pick_radius_px)`, `resize`, `set_pick_radius`, `set_modifiers`, `focus_lost`, `cursor_moved(pos) -> bool`, `button(Button, pressed) -> bool`, `scroll(steps) -> bool`, `cancel()`, `undo()`, `redo()`, `overlay() -> Vec<LineVertex>`; bool returns mean "redraw needed"; `pub doc: Document`.
  - App: left click draws; Esc cancels; Cmd+Z (or Ctrl+Z) undoes, Cmd+Shift+Z redoes; model buffers re-uploaded when `doc.revision()` changes; snap radius 8 logical px x scale factor (updated on `ScaleFactorChanged`).

- [ ] **Step 1: Dependency and module**

`crates/app/Cargo.toml`: add `tools = { path = "../tools" }` after the `session` line.

`crates/app/src/main.rs` becomes:
```rust
mod app;
mod editor;
mod navigation;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("create event loop");
    // Redraw on demand only; the loop sleeps until input arrives.
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut app::App::default())
        .expect("run app");
}
```

- [ ] **Step 2: Write the failing tests**

Replace `crates/app/src/navigation.rs` with (the `button` method now returns whether it consumed the event, and there is one new test, `button_reports_whether_navigation_used_it`):
```rust
use glam::{DVec2, DVec3};
use session::Camera;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Orbit { pivot: DVec3 },
    Pan { anchor: DVec3 },
}

/// Turns raw pointer input into camera moves. Middle drag orbits, Shift+middle pans,
/// Alt+left does the same for trackpads, scroll zooms toward the cursor.
#[derive(Default)]
pub struct Navigation {
    cursor: DVec2,
    shift: bool,
    alt: bool,
    drag: Option<(Button, Drag)>,
}

impl Navigation {
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.shift = shift;
        self.alt = alt;
    }

    /// Drops any drag and modifier state, e.g. when the window loses focus and release
    /// events would never arrive. The cursor position is kept.
    pub fn reset(&mut self) {
        self.drag = None;
        self.shift = false;
        self.alt = false;
    }

    /// Handles a button press or release. Returns true if navigation used the event, so it
    /// must not also go to the active tool.
    pub fn button(&mut self, cam: &Camera, button: Button, pressed: bool) -> bool {
        if !pressed {
            if self
                .drag
                .is_some_and(|(started_by, _)| started_by == button)
            {
                self.drag = None;
                return true;
            }
            return false;
        }
        let navigates = button == Button::Middle || (button == Button::Left && self.alt);
        if !navigates {
            return false;
        }
        let point = cam.anchor_at(self.cursor);
        let drag = if self.shift {
            Drag::Pan { anchor: point }
        } else {
            Drag::Orbit { pivot: point }
        };
        self.drag = Some((button, drag));
        true
    }

    pub fn cursor_moved(&mut self, cam: &mut Camera, pos: DVec2) -> bool {
        let delta = pos - self.cursor;
        self.cursor = pos;
        match self.drag {
            Some((_, Drag::Orbit { pivot })) => cam.orbit(pivot, delta),
            Some((_, Drag::Pan { anchor })) => cam.pan(anchor, delta),
            None => return false,
        }
        true
    }

    pub fn scroll(&mut self, cam: &mut Camera, steps: f64) -> bool {
        if steps == 0.0 {
            return false;
        }
        let anchor = cam.anchor_at(self.cursor);
        cam.zoom(anchor, steps);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Navigation, Camera) {
        let mut nav = Navigation::default();
        let mut cam = Camera::new(DVec2::new(800.0, 600.0));
        nav.cursor_moved(&mut cam, DVec2::new(400.0, 300.0));
        (nav, cam)
    }

    #[test]
    fn button_reports_whether_navigation_used_it() {
        let (mut nav, cam) = setup();
        assert!(
            !nav.button(&cam, Button::Left, true),
            "plain left belongs to tools"
        );
        assert!(!nav.button(&cam, Button::Left, false));
        assert!(nav.button(&cam, Button::Middle, true));
        assert!(nav.button(&cam, Button::Middle, false));
        nav.set_modifiers(false, true);
        assert!(nav.button(&cam, Button::Left, true), "alt+left orbits");
    }

    #[test]
    fn hover_without_buttons_does_nothing() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(450.0, 320.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn middle_drag_orbits_around_point_under_cursor() {
        let (mut nav, mut cam) = setup();
        let pivot = cam.anchor_at(DVec2::new(400.0, 300.0));
        let d = (cam.eye - pivot).length();
        nav.button(&cam, Button::Middle, true);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert!(((cam.eye - pivot).length() - d).abs() < 1e-9);
        assert!(!cam.eye.abs_diff_eq(Camera::new(cam.viewport).eye, 1e-6));
    }

    #[test]
    fn shift_middle_drag_pans_without_rotating() {
        let (mut nav, mut cam) = setup();
        let forward = cam.forward();
        let target = cam.target;
        nav.set_modifiers(true, false);
        nav.button(&cam, Button::Middle, true);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(430.0, 280.0)));
        assert!(cam.forward().abs_diff_eq(forward, 1e-12));
        assert!(!cam.target.abs_diff_eq(target, 1e-6));
    }

    #[test]
    fn plain_left_drag_is_left_for_tools() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        nav.button(&cam, Button::Left, true);
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 350.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn alt_left_drag_orbits_for_trackpads() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        nav.set_modifiers(false, true);
        nav.button(&cam, Button::Left, true);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn release_ends_drag() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true);
        nav.button(&cam, Button::Middle, false);
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn other_button_release_does_not_end_middle_drag() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true);
        nav.button(&cam, Button::Left, true);
        nav.button(&cam, Button::Left, false);
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn middle_release_does_not_end_alt_left_orbit() {
        let (mut nav, mut cam) = setup();
        nav.set_modifiers(false, true);
        nav.button(&cam, Button::Left, true);
        nav.button(&cam, Button::Middle, false);
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn reset_clears_drag_and_modifiers() {
        let (mut nav, mut cam) = setup();
        nav.set_modifiers(true, false);
        nav.button(&cam, Button::Middle, true);
        nav.reset();
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_eq!(cam, before);
        nav.button(&cam, Button::Middle, true);
        let forward = cam.forward();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        // Orbit rotates the view direction; pan would not.
        assert!(!cam.forward().abs_diff_eq(forward, 1e-9));
    }

    #[test]
    fn scroll_zooms_toward_point_under_cursor() {
        let (mut nav, mut cam) = setup();
        let anchor = cam.anchor_at(DVec2::new(400.0, 300.0));
        let d = (cam.eye - anchor).length();
        assert!(nav.scroll(&mut cam, 1.0));
        assert!((cam.eye - anchor).length() < d);
        assert!(!nav.scroll(&mut cam, 0.0));
    }
}
```

Create `crates/app/src/editor.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use session::{Query, QueryResult};

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 10.0)
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc.camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    fn stats(e: &Editor) -> (usize, usize) {
        match e.doc.query(&Query::Stats).unwrap() {
            QueryResult::Stats { edges, faces, .. } => (edges, faces),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn square(e: &mut Editor) {
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0), (1.0, 1.0)] {
            click_at(e, DVec3::new(x, y, 0.0));
        }
    }

    #[test]
    fn clicking_four_corners_draws_a_face() {
        let mut e = editor();
        square(&mut e);
        assert_eq!(stats(&e), (4, 1));
        let model = e.doc.model();
        let (id, _) = model.faces().next().unwrap();
        assert!((model.area(id).unwrap() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn undo_and_redo_step_through_edges() {
        let mut e = editor();
        square(&mut e);
        e.undo();
        assert_eq!(stats(&e), (3, 0));
        e.redo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn preview_follows_the_cursor_until_cancelled() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let px = e.doc.camera.project(DVec3::new(-2.0, 4.0, 0.0)).unwrap();
        assert!(e.cursor_moved(px));
        assert!(!e.overlay().is_empty(), "preview line expected");
        e.cancel();
        assert!(
            e.overlay().is_empty(),
            "ground hover has no marker and no preview after cancel"
        );
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn alt_left_drag_orbits_without_drawing() {
        let mut e = editor();
        let before = e.doc.camera.clone();
        e.cursor_moved(DVec2::new(400.0, 300.0));
        e.set_modifiers(false, true);
        e.button(Button::Left, true);
        e.cursor_moved(DVec2::new(460.0, 300.0));
        e.button(Button::Left, false);
        assert_ne!(e.doc.camera, before);
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn hovering_an_endpoint_shows_a_marker() {
        let mut e = editor();
        square(&mut e);
        let px = e.doc.camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 0.0));
        assert_eq!(e.overlay().len(), 8, "a 4-sided marker, no preview");
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p app`
Expected: compile errors in editor tests (`cannot find type Editor`); app.rs still compiles against the old navigation call (its `button` result is unused).

- [ ] **Step 4: Implement**

Prepend to `crates/app/src/editor.rs`:
```rust
//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces the overlay to draw.

use glam::{DVec2, DVec3};
use render::grid::LineVertex;
use session::{Command, Document};
use tools::overlay::overlay;
use tools::{Inference, LineTool, ToolEvent, ToolOutput, infer};

use crate::navigation::{Button, Navigation};

pub struct Editor {
    pub doc: Document,
    nav: Navigation,
    tool: LineTool,
    cursor: DVec2,
    hit: Option<Inference>,
    preview: Vec<[DVec3; 2]>,
    /// Snap radius in physical pixels.
    pick_radius_px: f64,
}

impl Editor {
    pub fn new(viewport: DVec2, pick_radius_px: f64) -> Self {
        Self {
            doc: Document::new(viewport),
            nav: Navigation::default(),
            tool: LineTool::new(),
            cursor: DVec2::ZERO,
            hit: None,
            preview: Vec::new(),
            pick_radius_px,
        }
    }

    pub fn resize(&mut self, viewport: DVec2) {
        self.doc.camera.viewport = viewport;
    }

    pub fn set_pick_radius(&mut self, px: f64) {
        self.pick_radius_px = px;
    }

    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.nav.set_modifiers(shift, alt);
    }

    pub fn focus_lost(&mut self) {
        self.nav.reset();
    }

    /// Returns true if anything visible changed.
    pub fn cursor_moved(&mut self, pos: DVec2) -> bool {
        self.cursor = pos;
        let camera_moved = self.nav.cursor_moved(&mut self.doc.camera, pos);
        self.update_hover() || camera_moved
    }

    pub fn button(&mut self, button: Button, pressed: bool) -> bool {
        if self.nav.button(&self.doc.camera, button, pressed) {
            return false;
        }
        if button != Button::Left || !pressed {
            return false;
        }
        self.update_hover();
        let Some(hit) = self.hit else { return false };
        let out = self.tool.handle(ToolEvent::Click(hit));
        self.apply(out);
        self.update_hover();
        true
    }

    pub fn scroll(&mut self, steps: f64) -> bool {
        let zoomed = self.nav.scroll(&mut self.doc.camera, steps);
        if zoomed {
            self.update_hover();
        }
        zoomed
    }

    /// Escape: abandon the line being drawn.
    pub fn cancel(&mut self) -> bool {
        let out = self.tool.handle(ToolEvent::Cancel);
        self.apply(out);
        self.update_hover();
        true
    }

    pub fn undo(&mut self) -> bool {
        self.run(Command::Undo);
        self.update_hover();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.run(Command::Redo);
        self.update_hover();
        true
    }

    /// Line pairs for the renderer's overlay pass.
    pub fn overlay(&self) -> Vec<LineVertex> {
        overlay(&self.preview, self.hit.as_ref(), &self.doc.camera)
            .into_iter()
            .flat_map(|s| {
                [s.a, s.b].map(|p| LineVertex {
                    pos: p.as_vec3().to_array(),
                    color: s.color,
                })
            })
            .collect()
    }

    fn apply(&mut self, out: ToolOutput) {
        for command in out.commands {
            self.run(command);
        }
        self.preview = out.preview;
    }

    fn run(&mut self, command: Command) {
        if let Err(e) = self.doc.execute(command) {
            eprintln!("command failed: {e}");
        }
    }

    /// Recomputes the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        let hit = infer(
            self.doc.model(),
            &self.doc.camera,
            self.cursor,
            self.tool.anchor(),
            self.pick_radius_px,
        );
        let out = self.tool.handle(ToolEvent::Move(hit));
        let changed = self.hit != Some(hit) || self.preview != out.preview;
        self.hit = Some(hit);
        self.preview = out.preview;
        changed
    }
}
```

Replace `crates/app/src/app.rs` with:
```rust
use std::sync::Arc;

use glam::DVec2;
use render::Renderer;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::editor::Editor;
use crate::navigation::Button;

/// Trackpad pixel deltas per zoom step.
const PIXELS_PER_SCROLL_STEP: f64 = 50.0;
/// Snap radius in logical pixels (scaled to physical pixels by the window's scale factor).
const PICK_RADIUS_LOGICAL_PX: f64 = 8.0;

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    editor: Editor,
    /// Document revision the renderer's model buffers were built from.
    drawn_revision: Option<u64>,
    modifiers: ModifiersState,
}

impl State {
    fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        // Colors are authored as sRGB values, so prefer a non-sRGB target that writes them as-is.
        // Keep the default format's channel layout when its non-sRGB twin is available.
        let caps = surface.get_capabilities(&adapter);
        let linear = config.format.remove_srgb_suffix();
        config.format = if caps.formats.contains(&linear) {
            linear
        } else {
            caps.formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .unwrap_or(config.format)
        };
        surface.configure(&device, &config);

        let renderer = Renderer::new(&device, &queue, config.format);
        let editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            PICK_RADIUS_LOGICAL_PX * window.scale_factor(),
        );
        Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            editor,
            drawn_revision: None,
            modifiers: ModifiersState::empty(),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.editor.resize(DVec2::new(width as f64, height as f64));
    }

    fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            _ => return, // Timeout or occluded: skip this frame.
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let revision = self.editor.doc.revision();
        if self.drawn_revision != Some(revision) {
            self.renderer.set_model(self.editor.doc.model());
            self.drawn_revision = Some(revision);
        }
        self.renderer.render(
            &view,
            (self.config.width, self.config.height),
            &self.editor.doc.camera,
            &self.editor.overlay(),
        );
        self.window.pre_present_notify();
        self.queue.present(frame);
    }
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        _ => Button::Other,
    }
}

#[derive(Default)]
pub struct App {
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("tenon")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let state = State::new(window);
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::Resized(size) => {
                s.resize(size.width, size.height);
                true
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.editor
                    .set_pick_radius(PICK_RADIUS_LOGICAL_PX * scale_factor);
                false
            }
            WindowEvent::RedrawRequested => {
                s.render();
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                s.modifiers = m.state();
                s.editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                false
            }
            WindowEvent::Focused(false) => {
                s.editor.focus_lost();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                s.editor.cursor_moved(DVec2::new(position.x, position.y))
            }
            WindowEvent::MouseInput { state, button, .. } => s
                .editor
                .button(map_button(button), state == ElementState::Pressed),
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / PIXELS_PER_SCROLL_STEP,
                };
                s.editor.scroll(steps)
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command_key = s.modifiers.super_key() || s.modifiers.control_key();
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => s.editor.cancel(),
                    Key::Character(c) if command_key && c.eq_ignore_ascii_case("z") => {
                        if s.modifiers.shift_key() {
                            s.editor.redo()
                        } else {
                            s.editor.undo()
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if changed {
            s.window.request_redraw();
        }
    }
}
```

- [ ] **Step 5: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 130 (app 16: navigation 11 + editor 5).

- [ ] **Step 6: Smoke check**

Run the app in the background with `RUST_LOG=warn cargo run -p app` for about 8 seconds; confirm the process is alive with no panic or wgpu validation error, then stop it.

- [ ] **Step 7: Commit**

```bash
git add crates/app/Cargo.toml crates/app/src/main.rs crates/app/src/navigation.rs crates/app/src/editor.rs crates/app/src/app.rs Cargo.lock
git commit -m "Wire Line tool, inference and undo into the app"
```

- [ ] **Step 8: Manual check (user)**

`cargo run -p app`, then: click four ground points and the first again -> a face appears; moving from a point along red/green/blue colors the preview line; hovering a corner shows a green square; Cmd+Z removes the last edge (and the face), Cmd+Shift+Z restores it; Esc drops a line in progress; middle-drag / Alt+drag still orbit without drawing.
