# M9b Textures Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A material can carry a PNG or JPEG texture (set and removed through `add_material` / `change_material`, saved inside `.skr` schema 7), faces show it projected in their plane in the window and in `render_view`, glTF and OBJ exports carry it, the `materials` query reports it, and the Materials list loads, removes and sizes it.

**Architecture:**
- Kernel `texture.rs`: `TextureImage` (format, the file's bytes, decoded RGBA8 pixels), `Texture { image: Arc<TextureImage>, width, height }` on `Material.texture`, limits, and the projection (`texture_axes`, `texture_position`). `Scene::add_material_with_texture` / `change_material_with_texture` keep one `Step` per command, so undo restores the old texture. `Triangle.uvs` carries each corner's texture position (meters, in the definition's plane). `MaterialData.texture` (`TextureData`) names the image's archive entry; the image itself rides in a `#[serde(skip)]` field.
- Session `texture.rs` decodes with the `image` crate (PNG and JPEG only) and works out the average color. `TextureSource {path, width, height}` on the two commands (`change_material`'s is absent / null / object). `file.rs` stores each image as the archive entry `textures/m<id>.<ext>`. `export.rs` adds `TEXCOORD_0`, images (in the buffer), a sampler and `baseColorTexture` to glTF, and `vt`, `map_Kd` and image files to OBJ.
- Render `texture.rs`: one `Rgba8Unorm` texture array, a square power-of-two layer per textured material (images resampled on the CPU, mipmaps worked out on the CPU), rebuilt only when the set of images changes; a repeating, mipmapped sampler. Face vertices gain `uv: [f32; 4]` (front and back) and `layer: [f32; 2]` (-1 for none); the fragment shader shows the texel instead of the color.
- App: `MaterialRow.texture` (`TextureRow`: a 32 x 32 swatch image, size in meters and pixels); Texture... (file dialog), Remove and Size fields under the opacity slider; editor actions run `change_material`.

**Tech Stack:** Rust 2024; wgpu 30, egui 0.36; new dependency `image` 0.25.10 (features `png`, `jpeg` only) in `session`.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md` (Milestones, "Post-v1 milestones", M9b).

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Comments succinct, in the style of the surrounding code.
- Every new capability is a `session::Command` reachable through the API and covered by a command-driven test. Each modeling command is one undo step.
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` passes after every task.
- One new dependency: `image = { version = "=0.25.10", default-features = false, features = ["png", "jpeg"] }` (workspace), used by `session`. It is already in `Cargo.lock` (egui-winit's clipboard pulls it in with `png`); the `jpeg` feature adds `zune-core` and `zune-jpeg`, also already locked. No other dependency changes.
- Limits from the spec: 16 MiB per texture file, 4096 px per side, 256 textures per model.
- Git: work on `dev`; stage only the files you changed (other agents may share the checkout: check `git status` and `git diff` first); short plain commit messages.
- Do not launch the app window.

## Spec notes (rulings made in this plan)

- **Where the texture lives.** The kernel keeps the decoded pixels and the file's bytes in an `Arc<TextureImage>` shared by every clone of the material (undo steps, files, the renderer's change check). A 4096 x 4096 image takes 64 MiB decoded; nothing more is kept.
- **Command shape.** `texture` is `{"path", "width", "height"}`, each optional in the object. `add_material` needs `path`; `change_material` without `path` keeps the image and changes its size (`invalid_input` when the material has none). A new image without `width` is 1 m wide, or as wide as the texture it replaces; without `height` it keeps the image's proportions. On `change_material`, an absent `texture` keeps it and `null` removes it. Unknown fields in the object are refused when the command is parsed.
- **Color.** Reading a new image sets the material's color to the image's average (each pixel's sRGB values counted once, whatever its alpha), unless `color` is given in the same command. `add_material` may leave `color` out when it has a texture (`invalid_input` "give a material a color or a texture" otherwise). Removing a texture keeps the color. The color shows on formats without textures and in the window's default swatch.
- **Errors.** A file that cannot be read, is not PNG or JPEG, is over 16 MiB or over 4096 px on a side is a `file` error naming the path. Sizes outside 0.001 to 1000 m and a 257th texture are `invalid_input`. Nothing changes on any error.
- **Transparency.** A texture's own alpha is ignored (textures are opaque); the material's opacity applies as before.
- **Projection.** `u = normalize(Z x n)`, or `X` when `|Z x n| < 1e-6`; `v = normalize(n x u)` (up on walls); position = `(p . u, p . v)` in meters, in the coordinates of the definition holding the face. The image's bottom left corner sits at whole multiples of its size. Both sides of a face use the front's projection, so from behind it reads mirrored. Back sides show their own material's texture with that projection.
- **Precision.** Texture coordinates are shifted by whole repeats per triangle (render and glTF), so f32 stays exact far from the origin. OBJ `vt` values are written unshifted (text f64).
- **Files.** Each image is stored, as read, in the archive entry `textures/m<id>.png` or `.jpg` (uncompressed: the formats compress already); `document.json` gives the material `"texture": {"file", "width", "height"}`. `schema_version` is 7 when a material has a texture, else 6 when one is see-through (`file::TRANSLUCENT_SCHEMA_VERSION`, new), else 5, so files without textures are written exactly as before. Reading checks each image entry against the 16 MiB and 4096 px limits; a missing or broken image fails the open with a `file` error naming the material.
- **Rendering.** One `Rgba8Unorm` array: textures hold sRGB values, like the vertex colors and the render targets, so a texel replaces a color one for one. Layers are square, a power of two large enough for the largest image but at most 1024, halved until the array with mipmaps fits 256 MiB (256 textures get 256 px layers). Each image is resampled to the layer on the CPU (box filter shrinking, linear growing, wrapping as the texture repeats), and its mipmaps are 2 x 2 averages. Material k in id order among the textured ones is layer k. The array is rebuilt only when the list of (material, image) changes (images compared by `Arc` identity); a new size in meters only rebuilds the vertices. The sampler repeats both ways with linear filtering and linear mipmaps (no anisotropy, to keep golden images stable). The shader samples before any `discard` (the lookup needs the whole pixel quad). With no textures a 1 x 1 layer is bound.
- **Vertex format.** `BaseFaceVertex` and `FaceVertex` gain `uv: [f32; 4]` (front then back) and `layer: [f32; 2]` (-1 for none): 80 bytes a corner (56 before). Vertex attribute locations 6 and 7.
- **glTF.** Only when some material has a texture: a third buffer view of `TEXCOORD_0` for the textured materials' triangles (v counted down from the image's top), then each texture's image file, as read, in the buffer (both `.glb` and `.gltf`; 4-byte aligned), one sampler (linear, mipmapped, repeat), `textures`, `images` with `mimeType`. A textured material has `baseColorTexture` and a white `baseColorFactor` with the opacity as alpha. Untextured exports are byte for byte as before.
- **OBJ.** Textured materials' triangles name `vt` coordinates (`f v/vt/vn`, counted from the image's bottom); the library gets `map_Kd <library stem>_<material>.<ext>` and that image, as read, is written beside it (replacing a file there) before the library. STL is unchanged. Back-side materials are still not exported.
- **Window.** A textured material's swatch is a 32 x 32 copy of its image (made when the image changes, dropped with the material). For the chosen material, under the opacity slider: Texture... (a native dialog for `.png`, `.jpg`, `.jpeg`; the new image keeps the size of the one it replaces), Remove and the image's pixel size when it has one, then Size fields (meters, 3 decimals, 0.001 to 1000; one change per drag or typed value). `UiAction::SetTextureSize` carries millimeters so `UiAction` stays `Eq`.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/kernel/src/texture.rs` (new) | `ImageFormat`, `TextureImage`, `Texture`, limits, `check_texture`, `texture_axes`, `texture_position` | 1 |
| `crates/kernel/src/material.rs`, `scene.rs`, `edit.rs`, `lib.rs` | `Material.texture`, `add_material_with_texture`, `change_material_with_texture`, `texture_count`, validation, `KernelError::MaterialTexture`, re-exports | 1 |
| `crates/kernel/src/data.rs` | `MaterialData.texture`, `TextureData` | 1 |
| `crates/kernel/src/triangles.rs` | `Triangle.uvs` | 1 |
| `crates/kernel/tests/textures.rs` (new) | kernel texture tests | 1 |
| `Cargo.toml`, `crates/session/Cargo.toml`, `Cargo.lock` | the `image` dependency | 2 |
| `crates/session/src/texture.rs` (new) | `decode`, `average_color`, limits | 2 |
| `crates/session/src/command.rs`, `document.rs`, `document/materials.rs`, `document/import.rs`, `query.rs`, `lib.rs` | `TextureSource`, command handling, `TextureInfo` | 2 |
| `crates/session/src/file.rs` | image entries, schema 7 | 2 |
| `crates/session/tests/textures.rs` (new), `crates/session/tests/materials.rs` | command-driven tests | 2 |
| `crates/app/src/editor.rs`, `crates/app/src/info.rs`, `crates/render/tests/golden.rs` | new command fields at existing call sites | 2 |
| `crates/session/src/export.rs`, `document/files.rs`, `crates/session/tests/export.rs` | glTF and OBJ textures | 3 |
| `crates/render/src/texture.rs` (new) | `textured`, `layers`, `layer_side`, `resample`, `mipmaps`, `TextureArray` | 4 |
| `crates/render/src/scene.rs`, `renderer.rs`, `shaders/scene.wgsl`, `lib.rs` | vertex uv and layer, bind group, sampling | 4 |
| `crates/render/tests/golden.rs`, `crates/render/tests/golden/textured.png` (new) | textured golden image | 4 |
| `crates/app/src/ui.rs`, `app.rs`, `gui.rs`, `editor.rs` | Materials list texture controls and swatches | 5 |
| `docs/agent-api.md` | commands, Textures, files, export, window | 2, 3, 5 |

The code below was written and verified in a scratch worktree of `dev` at f7cb434: after each task `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` passed. Workspace test totals after each task: 953, 959, 961, 966, 968 (from 948). Code for new files is given in full; changes to existing files are unified diffs against the state the previous task leaves. Where a diff mixes tests and code in one file, it is in Step 3; expect the Step 2 run to fail to compile.

---

### Task 1: Kernel: textures on materials and texture positions on triangles

**Files:**
- Create: `crates/kernel/src/texture.rs`
- Modify: `crates/kernel/src/material.rs`, `crates/kernel/src/scene.rs`, `crates/kernel/src/edit.rs`, `crates/kernel/src/data.rs`, `crates/kernel/src/triangles.rs`, `crates/kernel/src/lib.rs`
- Test: `crates/kernel/tests/textures.rs`

**Interfaces:**
- Consumes: `Scene::apply_step`, `Scene::check_free_name`, `check_opacity`, `Model::shaded_triangles` (M9a).
- Produces:
  - `kernel::{ImageFormat (Png | Jpeg; extension(), mime()), TextureImage { format, encoded: Vec<u8>, width: u32, height: u32, pixels: Vec<u8> }, Texture { image: Arc<TextureImage>, width: f64, height: f64 }}`
  - `kernel::{MAX_TEXTURES = 256, MIN_TEXTURE_SIZE = 0.001, MAX_TEXTURE_SIZE = 1000.0}`
  - `kernel::texture_axes(normal: DVec3) -> (DVec3, DVec3)`, `kernel::texture_position(normal: DVec3, p: DVec3) -> DVec2`
  - `Material.texture: Option<Texture>` (Material no longer derives Serialize/Deserialize; nothing used them)
  - `Scene::add_material_with_texture(&mut self, name: &str, color: [u8; 3], opacity: f32, texture: Option<Texture>) -> Result<(MaterialId, SceneChange), KernelError>`
  - `Scene::change_material_with_texture(&mut self, id, name: Option<&str>, color: Option<[u8; 3]>, opacity: Option<f32>, texture: Option<Option<Texture>>) -> Result<SceneChange, KernelError>`
  - `Scene::texture_count(&self) -> usize`
  - `KernelError::MaterialTexture(String)`
  - `kernel::TextureData { file: String, width: f64, height: f64, #[serde(skip)] image: Option<Arc<TextureImage>> }`, `MaterialData.texture: Option<TextureData>`
  - `Triangle.uvs: [DVec2; 3]` (texture positions in meters)

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/textures.rs`:

```rust
use std::sync::Arc;

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    DefinitionKind, ImageFormat, KernelError, MAX_TEXTURES, MeshId, Model, Scene, Texture,
    TextureImage, texture_axes, texture_position,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 2 x 1 pixel image: red, then blue.
fn image() -> Arc<TextureImage> {
    Arc::new(TextureImage {
        format: ImageFormat::Png,
        encoded: b"not decoded by the kernel".to_vec(),
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
    })
}

fn texture(width: f64, height: f64) -> Texture {
    Texture {
        image: image(),
        width,
        height,
    }
}

#[test]
fn textures_lie_along_the_horizontal_or_along_x_on_level_faces() {
    // A wall facing -y: u runs along +x, v up.
    let (u, v) = texture_axes(-DVec3::Y);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12) && v.abs_diff_eq(DVec3::Z, 1e-12));
    // A wall facing +x: u along +y.
    let (u, v) = texture_axes(DVec3::X);
    assert!(u.abs_diff_eq(DVec3::Y, 1e-12) && v.abs_diff_eq(DVec3::Z, 1e-12));
    // Level faces: u along x, v along y on top, along -y underneath.
    let (u, v) = texture_axes(DVec3::Z);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12) && v.abs_diff_eq(DVec3::Y, 1e-12));
    let (u, v) = texture_axes(-DVec3::Z);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12) && v.abs_diff_eq(-DVec3::Y, 1e-12));
    // A roof sloping up toward +y: u stays horizontal, v climbs the slope.
    let n = p(0.0, -1.0, 1.0).normalize();
    let (u, v) = texture_axes(n);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12));
    assert!(v.abs_diff_eq(p(0.0, 1.0, 1.0).normalize(), 1e-12));
    assert!(texture_position(-DVec3::Y, p(2.0, 5.0, 3.0)).abs_diff_eq(DVec2::new(2.0, 3.0), 1e-12));
}

#[test]
fn a_material_takes_a_texture_and_undo_restores_the_old_one() {
    let mut s = Scene::from_root(Model::new());
    let (id, _) = s
        .add_material_with_texture("Brick", [200, 0, 0], 1.0, Some(texture(1.0, 0.5)))
        .unwrap();
    assert_eq!(s.texture_count(), 1);
    assert_eq!(s.material(id).unwrap().texture, Some(texture(1.0, 0.5)));

    let bigger = s
        .change_material_with_texture(id, None, None, None, Some(Some(texture(2.0, 1.0))))
        .unwrap();
    assert_eq!(s.material(id).unwrap().texture, Some(texture(2.0, 1.0)));
    s.apply(&bigger.inverse());
    assert_eq!(s.material(id).unwrap().texture, Some(texture(1.0, 0.5)));

    // Changing only the name keeps the texture; removing it keeps the color.
    s.change_material(id, Some("Old brick"), None, None)
        .unwrap();
    assert!(s.material(id).unwrap().texture.is_some());
    let removed = s
        .change_material_with_texture(id, None, None, None, Some(None))
        .unwrap();
    let m = s.material(id).unwrap();
    assert_eq!((m.texture.clone(), m.color), (None, [200, 0, 0]));
    s.apply(&removed.inverse());
    assert_eq!(s.material(id).unwrap().texture, Some(texture(1.0, 0.5)));
    s.validate().unwrap();
}

#[test]
fn texture_sizes_and_counts_are_limited() {
    let mut s = Scene::from_root(Model::new());
    for bad in [0.0, 0.0005, -1.0, 1001.0, f64::NAN, f64::INFINITY] {
        let err = s
            .add_material_with_texture("Bad", [0, 0, 0], 1.0, Some(texture(bad, 1.0)))
            .unwrap_err();
        assert!(
            matches!(err, KernelError::MaterialTexture(_)),
            "{bad}: {err:?}"
        );
    }
    let broken = Texture {
        image: Arc::new(TextureImage {
            pixels: vec![0; 4],
            ..(*image()).clone()
        }),
        width: 1.0,
        height: 1.0,
    };
    assert!(matches!(
        s.add_material_with_texture("Broken", [0, 0, 0], 1.0, Some(broken)),
        Err(KernelError::MaterialTexture(_))
    ));
    assert_eq!(s.materials().count(), 0);

    for k in 0..MAX_TEXTURES {
        s.add_material_with_texture(&format!("T{k}"), [0, 0, 0], 1.0, Some(texture(1.0, 1.0)))
            .unwrap();
    }
    let err = s
        .add_material_with_texture("One more", [0, 0, 0], 1.0, Some(texture(1.0, 1.0)))
        .unwrap_err();
    assert!(err.to_string().contains("at most 256 textures"), "{err}");
    // A material that has one can still change it.
    let (first, _) = s.materials().next().unwrap();
    s.change_material_with_texture(first, None, None, None, Some(Some(texture(3.0, 3.0))))
        .unwrap();
    // One without cannot take one.
    let (plain, _) = s.add_material("Plain", [1, 2, 3]).unwrap();
    assert!(
        s.change_material_with_texture(plain, None, None, None, Some(Some(texture(1.0, 1.0))))
            .is_err()
    );
}

#[test]
fn textures_reach_files_with_their_images() {
    let mut s = Scene::from_root(Model::new());
    let (id, _) = s
        .add_material_with_texture("Brick", [200, 0, 0], 0.5, Some(texture(1.0, 0.5)))
        .unwrap();
    let mut data = s.to_data();
    let t = data.materials[0].texture.as_ref().unwrap();
    assert_eq!(t.file, format!("textures/m{}.png", id.0));
    assert_eq!((t.width, t.height), (1.0, 0.5));
    assert_eq!(t.image.as_deref(), Some(&*image()));
    // The image is not written: a reader puts it back.
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(
        json["materials"][0]["texture"],
        serde_json::json!({"file": t.file, "width": 1.0, "height": 0.5})
    );
    assert_eq!(Scene::from_data(&data).unwrap(), s);
    data.materials[0].texture.as_mut().unwrap().image = None;
    let err = Scene::from_data(&data).unwrap_err();
    assert!(err.contains("texture has no image"), "{err}");
    data.materials[0].texture.as_mut().unwrap().image = Some(image());
    data.materials[0].texture.as_mut().unwrap().width = -2.0;
    assert!(Scene::from_data(&data).is_err());
}

#[test]
fn triangles_carry_texture_positions_in_their_definition() {
    // A 1 m wall square in the xz plane (the kernel turns its front to +y), made a component
    // and placed 10 m along x, turned a quarter about z.
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 0.0, 1.0),
        p(0.0, 0.0, 1.0),
    ])
    .unwrap();
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (instance, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Panel",
        )
        .unwrap();
    let turn = DAffine3::from_translation(p(10.0, 0.0, 0.0))
        * DAffine3::from_rotation_z(std::f64::consts::FRAC_PI_2);
    s.edit(MeshId::Root, |m| {
        Ok(m.transform_instances(&[instance], turn))
    })
    .unwrap();
    let tris = s.triangles();
    assert_eq!(tris.len(), 2);
    for t in &tris {
        let normal = (t.corners[1] - t.corners[0])
            .cross(t.corners[2] - t.corners[0])
            .normalize();
        assert!(normal.abs_diff_eq(t.normal, 1e-9));
        assert!(t.normal.abs_diff_eq(-DVec3::X, 1e-9));
        for (c, uv) in t.corners.iter().zip(t.uvs) {
            // A definition point (x, 0, z) lands at (10, x, z). Its front faces +y there, so
            // the texture's u runs along -x and v up, whatever the placement.
            let def_x = c.y;
            assert!(uv.abs_diff_eq(DVec2::new(-def_x, c.z), 1e-9), "{c} {uv}");
        }
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p kernel --test textures`
Expected: compile errors (no `Texture`, `TextureImage`, `add_material_with_texture`, ...).

- [ ] **Step 3: Implement**

Create `crates/kernel/src/texture.rs`:

```rust
//! Textures: images on materials, projected in each face's plane.
//!
//! A face shows its material's texture laid flat in its own plane, in the coordinates of the
//! definition holding it: the image's width runs along the face's horizontal direction (x on
//! level faces) and its height across, repeating every `width` by `height` meters. Faces in one
//! plane continue each other's pattern, and a component carries its texture with it.

use std::sync::Arc;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// The most materials with a texture a scene holds.
pub const MAX_TEXTURES: usize = 256;
/// A texture's least width and height on the model, in meters.
pub const MIN_TEXTURE_SIZE: f64 = 0.001;
/// A texture's greatest width and height on the model, in meters.
pub const MAX_TEXTURE_SIZE: f64 = 1000.0;

/// Below this length of z x normal a face counts as level, and its texture runs along x.
const LEVEL: f64 = 1e-6;

/// The file format a texture image was read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    /// The usual file name extension.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }

    /// The media type.
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }
}

/// A decoded image with the file it was decoded from, kept for saving and export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureImage {
    pub format: ImageFormat,
    /// The file's bytes as read.
    pub encoded: Vec<u8>,
    /// Size in pixels.
    pub width: u32,
    pub height: u32,
    /// sRGB red, green, blue and alpha of each pixel, rows from the top.
    pub pixels: Vec<u8>,
}

/// A material's texture: an image and the size one copy of it covers on the model. Clones
/// share the image.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub image: Arc<TextureImage>,
    /// Meters along the face's horizontal direction (see [`texture_axes`]).
    pub width: f64,
    /// Meters across it.
    pub height: f64,
}

/// Fails, saying why, when a texture's size is not from [`MIN_TEXTURE_SIZE`] to
/// [`MAX_TEXTURE_SIZE`] meters or its pixels do not match the image's size.
pub(crate) fn check_texture(t: &Texture) -> Result<(), String> {
    for (what, x) in [("width", t.width), ("height", t.height)] {
        if !(x.is_finite() && (MIN_TEXTURE_SIZE..=MAX_TEXTURE_SIZE).contains(&x)) {
            return Err(format!(
                "a texture's {what} must be from {MIN_TEXTURE_SIZE} to {MAX_TEXTURE_SIZE} m, \
                 not {x}"
            ));
        }
    }
    let i = &t.image;
    let size = 4 * i.width as usize * i.height as usize;
    if size == 0 || i.pixels.len() != size {
        return Err("a texture image's pixels do not match its size".into());
    }
    Ok(())
}

/// The directions a face with unit `normal` lays a texture's width (u) and height (v) along:
/// u is horizontal, or x on a level face; v lies in the face across u, so that u, v and the
/// normal turn right-handed (v points up on a wall).
pub fn texture_axes(normal: DVec3) -> (DVec3, DVec3) {
    let across = DVec3::Z.cross(normal);
    let u = if across.length() < LEVEL {
        DVec3::X
    } else {
        across.normalize()
    };
    (u, normal.cross(u).normalize_or_zero())
}

/// Where point `p` of a face with unit `normal` lies on its texture, in meters along the
/// face's u and v directions (see [`texture_axes`]); divided by the texture's width and height
/// it gives the image's repeats, with its bottom left corner at whole numbers.
pub fn texture_position(normal: DVec3, p: DVec3) -> DVec2 {
    let (u, v) = texture_axes(normal);
    DVec2::new(p.dot(u), p.dot(v))
}
```

Apply:

```diff
diff --git a/crates/kernel/src/data.rs b/crates/kernel/src/data.rs
index c4faeab..46f4aa6 100644
--- a/crates/kernel/src/data.rs
+++ b/crates/kernel/src/data.rs
@@ -1,6 +1,7 @@
 //! The model as plain data, for files: every entity, definition and id as stored.
 
 use std::collections::BTreeMap;
+use std::sync::Arc;
 
 use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
@@ -11,6 +12,7 @@ use crate::ids::{
 use crate::material::Material;
 use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
 use crate::scene::{Definition, DefinitionKind, Scene};
+use crate::texture::{Texture, TextureImage};
 
 /// Ids in a file are below this, and its `next_id` is at most this, so every id is exact as a
 /// JSON (double) number and a scene that loads also saves and loads again.
@@ -44,6 +46,23 @@ pub struct MaterialData {
     /// Absent when 1, as in files written before opacity existed.
     #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
     pub opacity: f32,
+    /// Absent without a texture, as in files written before textures existed.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub texture: Option<TextureData>,
+}
+
+/// A material's texture: the file holding its image and the size it covers on the model, in
+/// meters.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
+#[serde(deny_unknown_fields)]
+pub struct TextureData {
+    /// The image file's name in the model file's archive.
+    pub file: String,
+    pub width: f64,
+    pub height: f64,
+    /// The image: `to_data` fills it in, and a reader must before `from_data`. Not written.
+    #[serde(skip)]
+    pub image: Option<Arc<TextureImage>>,
 }
 
 fn opaque() -> f32 {
@@ -154,6 +173,12 @@ impl Scene {
                     name: m.name.clone(),
                     color: m.color,
                     opacity: m.opacity,
+                    texture: m.texture.as_ref().map(|t| TextureData {
+                        file: format!("textures/m{}.{}", id.0, t.image.format.extension()),
+                        width: t.width,
+                        height: t.height,
+                        image: Some(t.image.clone()),
+                    }),
                 })
                 .collect(),
         }
@@ -188,10 +213,22 @@ impl Scene {
         let mut materials = BTreeMap::new();
         for m in &data.materials {
             check_id("material m", m.id)?;
+            let texture = match &m.texture {
+                None => None,
+                Some(t) => Some(Texture {
+                    image: t
+                        .image
+                        .clone()
+                        .ok_or_else(|| format!("material m{}'s texture has no image", m.id))?,
+                    width: t.width,
+                    height: t.height,
+                }),
+            };
             let material = Material {
                 name: m.name.clone(),
                 color: m.color,
                 opacity: m.opacity,
+                texture,
             };
             if materials.insert(MaterialId(m.id), material).is_some() {
                 return Err(format!("material m{} appears twice", m.id));
diff --git a/crates/kernel/src/edit.rs b/crates/kernel/src/edit.rs
index 2c80fb3..61d0ffb 100644
--- a/crates/kernel/src/edit.rs
+++ b/crates/kernel/src/edit.rs
@@ -33,6 +33,9 @@ pub enum KernelError {
     MaterialName(String),
     /// A material opacity is not a finite number from 0 to 1; the text says which.
     MaterialOpacity(String),
+    /// A texture's size is out of range, its image is broken, or a model would hold too many;
+    /// the text says which.
+    MaterialTexture(String),
     /// An import has no polygon that makes a face.
     NothingToImport,
     /// An import would make more faces than `MAX_IMPORT_FACES`.
@@ -65,7 +68,9 @@ impl fmt::Display for KernelError {
             KernelError::InstanceNotFound => write!(f, "no such instance"),
             KernelError::DefinitionNotFound => write!(f, "no such definition"),
             KernelError::MaterialNotFound => write!(f, "no such material"),
-            KernelError::MaterialName(why) | KernelError::MaterialOpacity(why) => {
+            KernelError::MaterialName(why)
+            | KernelError::MaterialOpacity(why)
+            | KernelError::MaterialTexture(why) => {
                 write!(f, "{why}")
             }
             KernelError::NothingToImport => {
diff --git a/crates/kernel/src/lib.rs b/crates/kernel/src/lib.rs
index 90a43ff..57a4826 100644
--- a/crates/kernel/src/lib.rs
+++ b/crates/kernel/src/lib.rs
@@ -20,6 +20,7 @@ mod regions;
 mod scene;
 mod smooth;
 mod soften;
+mod texture;
 mod transform;
 mod triangles;
 mod weld;
@@ -27,7 +28,7 @@ mod weld;
 pub use curves::{Curve, CurveCircle, CurveKind};
 pub use data::{
     DefinitionData, EdgeData, FaceData, GuideData, InstanceData, MAX_ID, MaterialData, MeshData,
-    SceneData, VertexData,
+    SceneData, TextureData, VertexData,
 };
 pub use edit::KernelError;
 pub use ids::{
@@ -40,4 +41,8 @@ pub use model::{ChangeSet, ChangeSummary, Edge, Face, Guide, Instance, Model, Op
 pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
 pub use smooth::{ShadedTriangle, SmoothNormals};
 pub use soften::SoftenReport;
+pub use texture::{
+    ImageFormat, MAX_TEXTURE_SIZE, MAX_TEXTURES, MIN_TEXTURE_SIZE, Texture, TextureImage,
+    texture_axes, texture_position,
+};
 pub use triangles::Triangle;
diff --git a/crates/kernel/src/material.rs b/crates/kernel/src/material.rs
index d95d259..fe55c49 100644
--- a/crates/kernel/src/material.rs
+++ b/crates/kernel/src/material.rs
@@ -1,5 +1,6 @@
-//! Materials: named colors shared by every mesh of a scene, painted on either side of a face or
-//! on an instance (whose faces without a material of their own then show it).
+//! Materials: named colors, possibly with a texture, shared by every mesh of a scene, painted on
+//! either side of a face or on an instance (whose faces without a material of their own then
+//! show it).
 
 use std::collections::BTreeSet;
 
@@ -9,18 +10,21 @@ use crate::edit::KernelError;
 use crate::ids::{FaceId, InstanceId, MaterialId};
 use crate::model::{ChangeSet, Model};
 use crate::scene::{MeshId, Scene, SceneChange, Step};
+use crate::texture::{MAX_TEXTURES, Texture, check_texture};
 
 /// The longest material name, in characters.
 pub const MAX_MATERIAL_NAME: usize = 256;
 
-/// A named color, possibly see-through.
-#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
+/// A named color, possibly see-through, possibly with a texture.
+#[derive(Clone, Debug, PartialEq)]
 pub struct Material {
     pub name: String,
-    /// sRGB red, green and blue.
+    /// sRGB red, green and blue. With a texture, faces show the texture and this color stands
+    /// for it (edges, swatches, formats without textures).
     pub color: [u8; 3],
     /// How much of what is behind it the material hides: 1 is opaque, 0 invisible.
     pub opacity: f32,
+    pub texture: Option<Texture>,
 }
 
 impl Material {
@@ -146,9 +150,25 @@ impl Scene {
         name: &str,
         color: [u8; 3],
         opacity: f32,
+    ) -> Result<(MaterialId, SceneChange), KernelError> {
+        self.add_material_with_texture(name, color, opacity, None)
+    }
+
+    /// Adds a material of `opacity` (finite, 0 to 1), with `texture` if given (see
+    /// [`MAX_TEXTURES`] and [`crate::MIN_TEXTURE_SIZE`]). The name must be valid (see
+    /// [`MAX_MATERIAL_NAME`]) and not taken.
+    pub fn add_material_with_texture(
+        &mut self,
+        name: &str,
+        color: [u8; 3],
+        opacity: f32,
+        texture: Option<Texture>,
     ) -> Result<(MaterialId, SceneChange), KernelError> {
         self.check_free_name(name, None)?;
         check_opacity(opacity).map_err(KernelError::MaterialOpacity)?;
+        if let Some(t) = &texture {
+            self.check_new_texture(t, None)?;
+        }
         let id = self.alloc_material();
         let step = Step::AddMaterial(
             id,
@@ -156,6 +176,7 @@ impl Scene {
                 name: name.to_string(),
                 color,
                 opacity,
+                texture,
             },
         );
         self.apply_step(&step);
@@ -170,6 +191,19 @@ impl Scene {
         name: Option<&str>,
         color: Option<[u8; 3]>,
         opacity: Option<f32>,
+    ) -> Result<SceneChange, KernelError> {
+        self.change_material_with_texture(id, name, color, opacity, None)
+    }
+
+    /// `change_material`, and `texture` sets (`Some(Some(_))`) or removes (`Some(None)`) the
+    /// material's texture; None keeps it. One step, so undo restores every part.
+    pub fn change_material_with_texture(
+        &mut self,
+        id: MaterialId,
+        name: Option<&str>,
+        color: Option<[u8; 3]>,
+        opacity: Option<f32>,
+        texture: Option<Option<Texture>>,
     ) -> Result<SceneChange, KernelError> {
         let from = self
             .material(id)
@@ -181,10 +215,14 @@ impl Scene {
         if let Some(opacity) = opacity {
             check_opacity(opacity).map_err(KernelError::MaterialOpacity)?;
         }
+        if let Some(Some(t)) = &texture {
+            self.check_new_texture(t, Some(id))?;
+        }
         let to = Material {
             name: name.map_or_else(|| from.name.clone(), str::to_string),
             color: color.unwrap_or(from.color),
             opacity: opacity.unwrap_or(from.opacity),
+            texture: texture.unwrap_or_else(|| from.texture.clone()),
         };
         if to == from {
             return Ok(SceneChange::default());
@@ -245,6 +283,33 @@ impl Scene {
             .expect("a free name")
     }
 
+    /// How many materials have a texture.
+    pub fn texture_count(&self) -> usize {
+        self.materials()
+            .filter(|(_, m)| m.texture.is_some())
+            .count()
+    }
+
+    /// Fails when `texture` is not valid, or when giving it to material `to` (None: a new one)
+    /// would make more than [`MAX_TEXTURES`] textures.
+    fn check_new_texture(
+        &self,
+        texture: &Texture,
+        to: Option<MaterialId>,
+    ) -> Result<(), KernelError> {
+        check_texture(texture).map_err(KernelError::MaterialTexture)?;
+        let others = self
+            .materials()
+            .filter(|(id, m)| Some(*id) != to && m.texture.is_some())
+            .count();
+        if others >= MAX_TEXTURES {
+            return Err(KernelError::MaterialTexture(format!(
+                "a model holds at most {MAX_TEXTURES} textures"
+            )));
+        }
+        Ok(())
+    }
+
     fn check_free_name(&self, name: &str, except: Option<MaterialId>) -> Result<(), KernelError> {
         check_name(name).map_err(KernelError::MaterialName)?;
         let taken = self
diff --git a/crates/kernel/src/scene.rs b/crates/kernel/src/scene.rs
index ea8b73c..314cb2d 100644
--- a/crates/kernel/src/scene.rs
+++ b/crates/kernel/src/scene.rs
@@ -45,7 +45,7 @@ pub enum Step {
     RenameDefinition(DefinitionId, String, String),
     AddMaterial(MaterialId, Material),
     RemoveMaterial(MaterialId, Material),
-    /// A material's name, color or opacity changed from the first to the second.
+    /// A material's name, color, opacity or texture changed from the first to the second.
     ChangeMaterial(MaterialId, Material, Material),
 }
 
@@ -439,13 +439,16 @@ impl Scene {
     /// Checks every mesh, that instances place existing definitions, that no definition places
     /// itself (directly or deeper), that no number is the id of two things (entities in any
     /// mesh, curves, definitions and materials all take ids from one counter), that every
-    /// material painted exists, that material names are valid and distinct, and that material
-    /// opacities are from 0 to 1.
+    /// material painted exists, that material names are valid and distinct, that material
+    /// opacities are from 0 to 1, and that textures are valid and at most `MAX_TEXTURES`.
     pub fn validate(&self) -> Result<(), String> {
         let mut names = BTreeSet::new();
         for (id, m) in &self.materials {
             crate::material::check_name(&m.name).map_err(|e| format!("material {id}: {e}"))?;
             crate::material::check_opacity(m.opacity).map_err(|e| format!("material {id}: {e}"))?;
+            if let Some(t) = &m.texture {
+                crate::texture::check_texture(t).map_err(|e| format!("material {id}: {e}"))?;
+            }
             if !names.insert(m.name.as_str()) {
                 return Err(format!(
                     "material {id}: two materials are named {:?}",
@@ -453,6 +456,12 @@ impl Scene {
                 ));
             }
         }
+        if self.texture_count() > crate::texture::MAX_TEXTURES {
+            return Err(format!(
+                "more than {} materials have a texture",
+                crate::texture::MAX_TEXTURES
+            ));
+        }
         let missing = |m: &Option<MaterialId>| m.is_some_and(|m| !self.materials.contains_key(&m));
         // Each id with the prefix of what holds it ("v", "e", "f", "i", "g", "c", "d", "m").
         let mut owner: BTreeMap<u64, char> = BTreeMap::new();
diff --git a/crates/kernel/src/triangles.rs b/crates/kernel/src/triangles.rs
index b6c026d..9b5d18d 100644
--- a/crates/kernel/src/triangles.rs
+++ b/crates/kernel/src/triangles.rs
@@ -1,9 +1,10 @@
 //! Every face of a scene as triangles in root coordinates, for export.
 
-use glam::{DAffine3, DVec3};
+use glam::{DAffine3, DVec2, DVec3};
 
 use crate::ids::MaterialId;
 use crate::scene::{MeshId, Scene};
+use crate::texture::texture_position;
 
 /// One triangle of a face, wound counter-clockwise around `normal` (the face's front).
 #[derive(Clone, Copy, Debug, PartialEq)]
@@ -13,6 +14,9 @@ pub struct Triangle {
     /// The normal at each corner: the face's own, or across soft edges the smooth one (see
     /// `Model::corner_normal`).
     pub normals: [DVec3; 3],
+    /// Where each corner lies on the face's texture, in meters (see `texture_position`): in the
+    /// plane of the face as its definition holds it, so a placed copy carries its pattern.
+    pub uvs: [DVec2; 3],
     /// The material shown on the front and on the back: the face's own, or else that of the
     /// nearest painted instance placing it; None for the default material.
     pub front: Option<MaterialId>,
@@ -68,10 +72,12 @@ impl Scene {
             for tri in tris {
                 let [a, b, c] = tri.corners.map(|p| place.transform_point3(p));
                 let [na, nb, nc] = tri.normals.map(|n| (normal_map * n).normalize());
+                let [ua, ub, uc] = tri.corners.map(|p| texture_position(face.normal, p));
                 out.push(Triangle {
                     corners: if mirrored { [a, c, b] } else { [a, b, c] },
                     normal,
                     normals: if mirrored { [na, nc, nb] } else { [na, nb, nc] },
+                    uvs: if mirrored { [ua, uc, ub] } else { [ua, ub, uc] },
                     front,
                     back,
                 });
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p kernel --test textures` (5 pass) and `cargo test -p kernel` (all pass).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/kernel/src/texture.rs crates/kernel/src/material.rs crates/kernel/src/scene.rs crates/kernel/src/edit.rs crates/kernel/src/data.rs crates/kernel/src/triangles.rs crates/kernel/src/lib.rs crates/kernel/tests/textures.rs
git commit -m "Kernel: textures on materials, texture positions on triangles"
```

---

### Task 2: Texture commands, materials query and .skr schema 7

**Files:**
- Create: `crates/session/src/texture.rs`
- Modify: `Cargo.toml`, `crates/session/Cargo.toml`, `Cargo.lock` (cargo updates it), `crates/session/src/command.rs`, `crates/session/src/document.rs`, `crates/session/src/document/materials.rs`, `crates/session/src/document/import.rs`, `crates/session/src/query.rs`, `crates/session/src/file.rs`, `crates/session/src/lib.rs`, `crates/app/src/editor.rs`, `crates/app/src/info.rs`, `crates/render/tests/golden.rs`, `docs/agent-api.md`
- Test: `crates/session/tests/textures.rs`, `crates/session/tests/materials.rs`, unit tests in `crates/session/src/texture.rs`

**Interfaces:**
- Consumes: Task 1's `Texture`, `TextureImage`, `ImageFormat`, `add_material_with_texture`, `change_material_with_texture`, `TextureData`, `KernelError::MaterialTexture`.
- Produces:
  - `session::texture::{decode(encoded: Vec<u8>) -> Result<TextureImage, String>, average_color(&TextureImage) -> [u8; 3], MAX_TEXTURE_BYTES = 16 MiB, MAX_TEXTURE_PIXELS = 4096}`
  - `session::TextureSource { path: Option<String>, width: Option<f64>, height: Option<f64> }`
  - `Command::AddMaterial { name, color: Option<[u8; 3]>, opacity, texture: Option<TextureSource> }`
  - `Command::ChangeMaterial { material, name, color, opacity, texture: Option<Option<TextureSource>> }` (absent / null / object)
  - `session::TextureInfo { width, height, pixel_width, pixel_height, format: ImageFormat }`, `MaterialInfo.texture: Option<TextureInfo>`; `session::ImageFormat` re-exported
  - `session::file::{SCHEMA_VERSION = 7, TRANSLUCENT_SCHEMA_VERSION = 6, OPAQUE_SCHEMA_VERSION = 5}`

- [ ] **Step 1: Write the failing tests**

Create `crates/session/tests/textures.rs`:

```rust
//! Textures on materials, driven through JSON commands and queries.

use std::io::Write;
use std::path::{Path, PathBuf};

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn materials(d: &Document) -> Vec<Value> {
    let q: Query = serde_json::from_value(json!({"type": "materials"})).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()["materials"]
        .as_array()
        .unwrap()
        .clone()
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-textures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A 4 x 2 pixel PNG, its left half red and its right half blue.
fn red_blue_png(name: &str) -> PathBuf {
    let path = scratch(name);
    image::RgbaImage::from_fn(4, 2, |x, _| {
        if x < 2 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    })
    .save(&path)
    .unwrap();
    path
}

fn add_textured(d: &mut Document, path: &Path, width: f64) -> String {
    let out = run(
        d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": path.to_str().unwrap(), "width": width}}),
    )
    .unwrap();
    out["changes"]["materials"][0].as_str().unwrap().to_string()
}

#[test]
fn a_material_reads_a_texture_and_reports_it() {
    let png = red_blue_png("tiles.png");
    let mut d = doc();
    let id = add_textured(&mut d, &png, 0.5);
    let listed = materials(&d);
    // No color given: the image's average. No height: the image's proportions.
    assert_eq!(listed[0]["color"], json!([128, 0, 128]));
    assert_eq!(
        listed[0]["texture"],
        json!({"width": 0.5, "height": 0.25, "pixel_width": 4, "pixel_height": 2,
               "format": "png"})
    );

    // Resizing keeps the image; a size left out stays.
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "texture": {"height": 1.0}}),
    )
    .unwrap();
    assert_eq!(
        materials(&d)[0]["texture"],
        json!({"width": 0.5, "height": 1.0, "pixel_width": 4, "pixel_height": 2,
               "format": "png"})
    );
    // Renaming leaves the texture; null removes it and keeps the color.
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "name": "Plain"}),
    )
    .unwrap();
    assert!(materials(&d)[0]["texture"].is_object());
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "texture": null}),
    )
    .unwrap();
    let m = &materials(&d)[0];
    assert!(m.get("texture").is_none(), "{m}");
    assert_eq!(m["color"], json!([128, 0, 128]));

    // Undo puts the texture back as it was, a step at a time.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["texture"]["height"], 1.0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["texture"]["height"], 0.25);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(materials(&d)[0]["texture"]["height"], 1.0);
}

#[test]
fn a_new_image_takes_the_average_color_unless_one_is_given() {
    let png = red_blue_png("average.png");
    let mut d = doc();
    let out = run(&mut d, json!({"type": "add_material", "color": [1, 2, 3]})).unwrap();
    let id = out["changes"]["materials"][0].clone();
    run(
        &mut d,
        json!({"type": "change_material", "material": id, "color": [9, 9, 9],
               "texture": {"path": png.to_str().unwrap()}}),
    )
    .unwrap();
    let m = &materials(&d)[0];
    assert_eq!(m["color"], json!([9, 9, 9]));
    // 1 m wide by default, as tall as the image's proportions.
    assert_eq!(
        (
            m["texture"]["width"].clone(),
            m["texture"]["height"].clone()
        ),
        (json!(1.0), json!(0.5))
    );
    run(
        &mut d,
        json!({"type": "change_material", "material": id,
               "texture": {"path": png.to_str().unwrap(), "height": 3.0}}),
    )
    .unwrap();
    let m = &materials(&d)[0];
    assert_eq!(m["color"], json!([128, 0, 128]));
    // A new image keeps the width of the one it replaces.
    assert_eq!(
        (
            m["texture"]["width"].clone(),
            m["texture"]["height"].clone()
        ),
        (json!(1.0), json!(3.0))
    );
}

#[test]
fn bad_textures_are_refused() {
    let png = red_blue_png("bad.png");
    let mut d = doc();
    let mut fails = |v: Value, kind: ErrorKind, text: &str| {
        let (k, message) = run(&mut d, v.clone()).unwrap_err();
        assert_eq!(k, kind, "{v}: {message}");
        assert!(message.contains(text), "{v}: {message}");
    };
    let missing = scratch("missing.png");
    fails(
        json!({"type": "add_material", "texture": {"path": missing.to_str().unwrap()}}),
        ErrorKind::File,
        "missing.png",
    );
    let text = scratch("not-an-image.png");
    std::fs::write(&text, "hello").unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": text.to_str().unwrap()}}),
        ErrorKind::File,
        "not a PNG or JPEG image",
    );
    let wide = scratch("wide.png");
    image::RgbaImage::new(4097, 1).save(&wide).unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": wide.to_str().unwrap()}}),
        ErrorKind::File,
        "at most 4096",
    );
    let big = scratch("big.png");
    let mut f = std::fs::File::create(&big).unwrap();
    f.write_all(&vec![0; 16 * 1024 * 1024 + 1]).unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": big.to_str().unwrap()}}),
        ErrorKind::File,
        "file too large",
    );
    let path = png.to_str().unwrap();
    fails(
        json!({"type": "add_material", "texture": {"path": path, "width": 0}}),
        ErrorKind::InvalidInput,
        "width must be from 0.001 to 1000 m",
    );
    fails(
        json!({"type": "add_material", "texture": {"width": 1}}),
        ErrorKind::InvalidInput,
        "needs a path",
    );
    fails(
        json!({"type": "add_material", "name": "Nothing"}),
        ErrorKind::InvalidInput,
        "a color or a texture",
    );
    // Unknown texture fields are not commands at all.
    let unknown = json!({"type": "add_material", "texture": {"path": path, "size": 1}});
    assert!(serde_json::from_value::<Command>(unknown).is_err());
    let out = run(&mut d, json!({"type": "add_material", "color": [1, 2, 3]})).unwrap();
    let id = out["changes"]["materials"][0].clone();
    let (kind, message) = run(
        &mut d,
        json!({"type": "change_material", "material": id, "texture": {"width": 2}}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("no texture"), "{message}");
    assert_eq!(d.revision(), 1, "nothing else changed the model");
}

#[test]
fn textures_are_saved_inside_the_file() {
    let png = red_blue_png("saved.png");
    let mut d = doc();
    let id = add_textured(&mut d, &png, 2.0);
    let path = scratch("textured.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
    assert_eq!(document["schema_version"], 7);
    let file = format!("textures/{id}.png");
    assert_eq!(
        document["model"]["materials"][0]["texture"],
        json!({"file": file, "width": 2.0, "height": 1.0})
    );
    let mut stored = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name(&file).unwrap(), &mut stored).unwrap();
    assert_eq!(
        stored,
        std::fs::read(&png).unwrap(),
        "the file as it was read"
    );

    // The image file is gone: the model has its own copy.
    std::fs::remove_file(&png).unwrap();
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));
    assert_eq!(e.scene(), d.scene());
}

#[test]
fn files_with_missing_or_broken_textures_do_not_open() {
    let png = red_blue_png("broken.png");
    let mut d = doc();
    add_textured(&mut d, &png, 1.0);
    let path = scratch("good.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    let file = document["model"]["materials"][0]["texture"]["file"]
        .as_str()
        .unwrap()
        .to_string();
    let rewrite = |name: &str, image: Option<&[u8]>| {
        let out = scratch(name);
        let mut w = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        w.start_file("document.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(&serde_json::to_vec(&document).unwrap())
            .unwrap();
        if let Some(image) = image {
            w.start_file(file.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(image).unwrap();
        }
        w.finish().unwrap();
        out
    };
    for (out, expected) in [
        (rewrite("no-image.skr", None), "is missing"),
        (
            rewrite("garbled.skr", Some(b"garbled")),
            "not a PNG or JPEG",
        ),
    ] {
        let mut e = doc();
        let (kind, message) = run(
            &mut e,
            json!({"type": "open", "path": out.to_str().unwrap()}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::File);
        assert!(message.contains(expected), "{message}");
    }
}
```

Apply to the existing tests:

```diff
diff --git a/crates/render/tests/golden.rs b/crates/render/tests/golden.rs
index a559785..3805d56 100644
--- a/crates/render/tests/golden.rs
+++ b/crates/render/tests/golden.rs
@@ -244,8 +244,9 @@ fn add_material(doc: &mut Document, name: &str, color: [u8; 3]) -> MaterialId {
         doc,
         Command::AddMaterial {
             name: Some(name.into()),
-            color,
+            color: Some(color),
             opacity: None,
+            texture: None,
         },
     );
     outcome.changes.materials[0]
@@ -392,8 +393,9 @@ fn golden_glass_pane_in_front_of_a_box() {
         &mut d,
         Command::AddMaterial {
             name: Some("Glass".into()),
-            color: [170, 210, 235],
+            color: Some([170, 210, 235]),
             opacity: Some(0.35),
+            texture: None,
         },
     )
     .changes
diff --git a/crates/session/tests/materials.rs b/crates/session/tests/materials.rs
index 8470786..c1c743f 100644
--- a/crates/session/tests/materials.rs
+++ b/crates/session/tests/materials.rs
@@ -543,7 +543,10 @@ fn opacity_is_saved_as_version_6_and_opaque_models_stay_version_5() {
     .unwrap();
     let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
     let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
-    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
+    assert_eq!(
+        document["schema_version"],
+        session::file::TRANSLUCENT_SCHEMA_VERSION
+    );
     assert_eq!(document["schema_version"], 6);
     assert_eq!(document["model"]["materials"][0]["opacity"], 0.4);
     let mut e = doc();
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p session --test textures`
Expected: compile errors (no `texture` field on the commands; `image` not a dependency).

- [ ] **Step 3: Implement**

Add the dependency first (`cargo build` then records `image` under `session` in `Cargo.lock` and adds `zune-core`, `zune-jpeg` to `image`'s dependency list; no new packages). The Step 1 hunks in `crates/session/tests/materials.rs` (a see-through model is now written as version 6, no longer the newest) and `crates/render/tests/golden.rs` (the new command fields) go with this change.

Create `crates/session/src/texture.rs`:

```rust
//! Reading texture images: PNG and JPEG files decoded to sRGB pixels.

use std::io::Cursor;

use image::{ImageReader, Limits};
use kernel::{ImageFormat, TextureImage};

/// The largest texture file read, in bytes.
pub const MAX_TEXTURE_BYTES: u64 = 16 * 1024 * 1024;
/// The most pixels along either side of a texture image.
pub const MAX_TEXTURE_PIXELS: u32 = 4096;

/// Decodes a PNG or JPEG file's bytes, keeping them. Fails, saying why, for other formats,
/// broken files and images more than [`MAX_TEXTURE_PIXELS`] wide or high. Any alpha is kept in
/// the pixels but not shown (textures are opaque).
pub fn decode(encoded: Vec<u8>) -> Result<TextureImage, String> {
    if encoded.len() as u64 > MAX_TEXTURE_BYTES {
        return Err(format!(
            "file too large: a texture is at most {MAX_TEXTURE_BYTES} bytes"
        ));
    }
    let mut reader = ImageReader::new(Cursor::new(&encoded[..]))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = match reader.format() {
        Some(image::ImageFormat::Png) => ImageFormat::Png,
        Some(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
        _ => return Err("not a PNG or JPEG image".into()),
    };
    let (width, height) = ImageReader::new(Cursor::new(&encoded[..]))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if width > MAX_TEXTURE_PIXELS || height > MAX_TEXTURE_PIXELS {
        return Err(format!(
            "the image is {width} x {height} pixels: a texture has at most \
             {MAX_TEXTURE_PIXELS} on each side"
        ));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_PIXELS);
    limits.max_image_height = Some(MAX_TEXTURE_PIXELS);
    reader.limits(limits);
    let pixels = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    Ok(TextureImage {
        format,
        width: pixels.width(),
        height: pixels.height(),
        pixels: pixels.into_raw(),
        encoded,
    })
}

/// The image's average color (sRGB, each pixel counted once whatever its alpha), which the
/// material shows where textures are not drawn.
pub fn average_color(image: &TextureImage) -> [u8; 3] {
    let mut sum = [0u64; 3];
    for p in image.pixels.as_chunks::<4>().0 {
        for (s, c) in sum.iter_mut().zip(p) {
            *s += u64::from(*c);
        }
    }
    let n = (image.pixels.len() / 4).max(1) as u64;
    sum.map(|s| ((s + n / 2) / n) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn pngs_and_jpegs_decode_and_others_do_not() {
        let image = decode(png(3, 2)).unwrap();
        assert_eq!(
            (image.format, image.width, image.height),
            (ImageFormat::Png, 3, 2)
        );
        assert_eq!(image.pixels.len(), 24);
        assert_eq!(average_color(&image), [10, 20, 30]);

        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50]))
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let image = decode(jpeg).unwrap();
        assert_eq!(
            (image.format, image.width, image.height),
            (ImageFormat::Jpeg, 8, 8)
        );
        let [r, g, b] = average_color(&image);
        assert!(r.abs_diff(200) < 6 && g.abs_diff(100) < 6 && b.abs_diff(50) < 6);

        assert!(
            decode(b"GIF89a....".to_vec())
                .unwrap_err()
                .contains("PNG or JPEG")
        );
        let mut broken = png(3, 2);
        broken.truncate(40);
        assert!(decode(broken).is_err());
        let err = decode(png(4097, 1)).unwrap_err();
        assert!(err.contains("at most 4096"), "{err}");
        let err = decode(vec![0; MAX_TEXTURE_BYTES as usize + 1]).unwrap_err();
        assert!(err.contains("file too large"), "{err}");
    }
}
```

Apply:

```diff
diff --git a/Cargo.lock b/Cargo.lock
index 38b4ff0..10dbf80 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1192,6 +1192,8 @@ dependencies = [
  "num-traits",
  "png",
  "tiff",
+ "zune-core",
+ "zune-jpeg",
 ]
 
 [[package]]
@@ -2546,6 +2548,7 @@ version = "0.1.0"
 dependencies = [
  "base64",
  "glam",
+ "image",
  "kernel",
  "schemars",
  "serde",
diff --git a/Cargo.toml b/Cargo.toml
index eea08b8..e23b5e5 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -25,4 +25,5 @@ egui-winit = "=0.36.2"
 rfd = "=0.17.2"
 objc2 = "=0.6.5"
 objc2-foundation = { version = "=0.3.2", default-features = false, features = ["std", "NSProcessInfo", "NSString", "NSObject"] }
+image = { version = "=0.25.10", default-features = false, features = ["png", "jpeg"] }
 zip = { version = "=9.0.2", default-features = false, features = ["deflate-flate2-zlib-rs"] }
diff --git a/crates/app/src/editor.rs b/crates/app/src/editor.rs
index ff8faed..52d71c6 100644
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -370,8 +370,9 @@ impl Editor {
         self.message = None;
         let Some(outcome) = self.run(Command::AddMaterial {
             name: None,
-            color,
+            color: Some(color),
             opacity: None,
+            texture: None,
         }) else {
             return true;
         };
@@ -387,6 +388,7 @@ impl Editor {
             name: Some(name),
             color: None,
             opacity: None,
+            texture: None,
         });
         true
     }
@@ -399,6 +401,7 @@ impl Editor {
             name: None,
             color: Some(color),
             opacity: None,
+            texture: None,
         });
         true
     }
@@ -411,6 +414,7 @@ impl Editor {
             name: None,
             color: None,
             opacity: Some(f32::from(percent.min(100)) / 100.0),
+            texture: None,
         });
         true
     }
diff --git a/crates/app/src/info.rs b/crates/app/src/info.rs
index c88ef97..27ad284 100644
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -251,8 +251,9 @@ mod tests {
         let added = d
             .execute(Command::AddMaterial {
                 name: Some("Brick".into()),
-                color: [180, 60, 40],
+                color: Some([180, 60, 40]),
                 opacity: None,
+                texture: None,
             })
             .unwrap();
         let brick = added.changes.materials[0];
diff --git a/crates/session/Cargo.toml b/crates/session/Cargo.toml
index a9d172c..ef40886 100644
--- a/crates/session/Cargo.toml
+++ b/crates/session/Cargo.toml
@@ -11,3 +11,4 @@ schemars.workspace = true
 serde_json.workspace = true
 zip.workspace = true
 base64.workspace = true
+image.workspace = true
diff --git a/crates/session/src/command.rs b/crates/session/src/command.rs
index e202166..a98651c 100644
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -154,16 +154,23 @@ pub enum Command {
     /// Add a material: a named sRGB color (each 0 to 255). Without `name` it is called
     /// "Material" (or "Material 2", ... if taken). Names must be distinct, not blank, and at
     /// most 256 characters. `opacity` (default 1, opaque) is from 0 to 1; below 1 faces
-    /// painted with it are see-through. The new id is in the outcome's `changes.materials`.
+    /// painted with it are see-through. `texture` reads a PNG or JPEG image for its faces
+    /// (`path` required); `color` may then be left out and is the image's average color. The
+    /// new id is in the outcome's `changes.materials`.
     AddMaterial {
         #[serde(default)]
         name: Option<String>,
-        color: [u8; 3],
+        #[serde(default)]
+        color: Option<[u8; 3]>,
         #[serde(default)]
         opacity: Option<f32>,
+        #[serde(default)]
+        texture: Option<TextureSource>,
     },
-    /// Rename, recolor and/or change the opacity (0 to 1) of a material; absent fields stay
-    /// as they are.
+    /// Rename, recolor, change the opacity (0 to 1) and/or the texture of a material; absent
+    /// fields stay as they are. `texture`: null removes it; an object sets it, reading `path`
+    /// (the color becomes the image's average unless `color` is given), or without `path`
+    /// keeps the image and changes its size.
     ChangeMaterial {
         material: MaterialId,
         #[serde(default)]
@@ -172,6 +179,13 @@ pub enum Command {
         color: Option<[u8; 3]>,
         #[serde(default)]
         opacity: Option<f32>,
+        #[serde(
+            default,
+            deserialize_with = "present",
+            skip_serializing_if = "Option::is_none"
+        )]
+        #[schemars(with = "Option<TextureSource>")]
+        texture: Option<Option<TextureSource>>,
     },
     /// Remove a material. Faces and instances painted with it, anywhere in the model, go back
     /// to the default material.
@@ -343,6 +357,31 @@ impl Command {
     }
 }
 
+/// A texture for `add_material` or `change_material`: an image file and the size one copy of
+/// it covers on the model, in meters (from 0.001 to 1000). A new image without `width` is 1 m
+/// wide (or as wide as the texture it replaces), and without `height` keeps the image's
+/// proportions.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct TextureSource {
+    /// A PNG or JPEG file of at most 16 MiB and 4096 pixels on each side.
+    #[serde(default)]
+    pub path: Option<String>,
+    #[serde(default)]
+    pub width: Option<f64>,
+    #[serde(default)]
+    pub height: Option<f64>,
+}
+
+/// A field that may be absent (None), null (`Some(None)`) or a value.
+fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
+where
+    D: serde::Deserializer<'de>,
+    T: Deserialize<'de>,
+{
+    Option::<T>::deserialize(deserializer).map(Some)
+}
+
 /// How `Command::Select` combines `ids` with the current selection.
 #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
 #[serde(rename_all = "snake_case")]
diff --git a/crates/session/src/document.rs b/crates/session/src/document.rs
index ac9d9d9..d06bcee 100644
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -334,13 +334,21 @@ impl Document {
                 name,
                 color,
                 opacity,
-            } => self.add_material_changes(name.as_deref(), *color, *opacity),
+                texture,
+            } => self.add_material_changes(name.as_deref(), *color, *opacity, texture.as_ref()),
             Command::ChangeMaterial {
                 material,
                 name,
                 color,
                 opacity,
-            } => self.change_material_changes(*material, name.as_deref(), *color, *opacity),
+                texture,
+            } => self.change_material_changes(
+                *material,
+                name.as_deref(),
+                *color,
+                *opacity,
+                texture.as_ref().map(Option::as_ref),
+            ),
             Command::DeleteMaterial { material } => self.delete_material_changes(*material),
             Command::Paint {
                 ids,
diff --git a/crates/session/src/document/import.rs b/crates/session/src/document/import.rs
index 1136ffa..d8118de 100644
--- a/crates/session/src/document/import.rs
+++ b/crates/session/src/document/import.rs
@@ -228,7 +228,7 @@ impl Document {
 }
 
 /// The whole file, failing when it holds more than `limit` bytes.
-fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
+pub(super) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
     // A pipe or device could block forever (or eat the host's stdin): regular files only.
     if !std::fs::metadata(path)
         .map_err(|e| e.to_string())?
diff --git a/crates/session/src/document/materials.rs b/crates/session/src/document/materials.rs
index 0f2f822..f61bc03 100644
--- a/crates/session/src/document/materials.rs
+++ b/crates/session/src/document/materials.rs
@@ -1,40 +1,77 @@
-//! Materials and painting.
+//! Materials, their textures, and painting.
 
-use kernel::{EntityId, KernelError, MaterialId, SceneChange, Side};
+use std::path::Path;
+use std::sync::Arc;
+
+use kernel::{EntityId, KernelError, MaterialId, SceneChange, Side, Texture};
 
 use super::Document;
-use crate::command::{CommandError, ErrorKind};
-use crate::query::MaterialInfo;
+use super::import::read_limited;
+use crate::command::{CommandError, ErrorKind, TextureSource};
+use crate::query::{MaterialInfo, TextureInfo};
+use crate::texture::{MAX_TEXTURE_BYTES, average_color, decode};
 
 /// The name `add_material` gives when none is passed (made unique with a number).
 const DEFAULT_MATERIAL_NAME: &str = "Material";
 
 impl Document {
+    /// Adds a material; without `color` it takes the texture's average color.
     pub(super) fn add_material_changes(
         &mut self,
         name: Option<&str>,
-        color: [u8; 3],
+        color: Option<[u8; 3]>,
         opacity: Option<f32>,
+        texture: Option<&TextureSource>,
     ) -> Result<SceneChange, CommandError> {
+        if texture.is_some_and(|t| t.path.is_none()) {
+            return Err(CommandError::new(
+                ErrorKind::InvalidInput,
+                "a new material's texture needs a path",
+            ));
+        }
+        let texture = texture.map(|t| texture_of(t, None)).transpose()?;
+        let Some(color) = color.or_else(|| texture.as_ref().map(|t| average_color(&t.image)))
+        else {
+            return Err(CommandError::new(
+                ErrorKind::InvalidInput,
+                "give a material a color or a texture",
+            ));
+        };
         let name = match name {
             Some(n) => n.to_string(),
             None => self.scene.free_material_name(DEFAULT_MATERIAL_NAME),
         };
         self.scene
-            .add_material_with_opacity(&name, color, opacity.unwrap_or(1.0))
+            .add_material_with_texture(&name, color, opacity.unwrap_or(1.0), texture)
             .map(|(_, change)| change)
             .map_err(|e| material_error(e, None))
     }
 
+    /// Changes a material. `texture`: None keeps it, `Some(None)` removes it. A newly read
+    /// image sets the color to its average unless `color` is given.
     pub(super) fn change_material_changes(
         &mut self,
         id: MaterialId,
         name: Option<&str>,
         color: Option<[u8; 3]>,
         opacity: Option<f32>,
+        texture: Option<Option<&TextureSource>>,
     ) -> Result<SceneChange, CommandError> {
+        let Some(material) = self.scene.material(id) else {
+            return Err(material_error(KernelError::MaterialNotFound, Some(id)));
+        };
+        let read = matches!(texture, Some(Some(s)) if s.path.is_some());
+        let texture = match texture {
+            None => None,
+            Some(None) => Some(None),
+            Some(Some(source)) => Some(Some(texture_of(source, material.texture.as_ref())?)),
+        };
+        let color = match (&texture, color) {
+            (Some(Some(t)), None) if read => Some(average_color(&t.image)),
+            _ => color,
+        };
         self.scene
-            .change_material(id, name, color, opacity)
+            .change_material_with_texture(id, name, color, opacity, texture)
             .map_err(|e| material_error(e, Some(id)))
     }
 
@@ -91,6 +128,13 @@ impl Document {
                     name: m.name.clone(),
                     color: m.color,
                     opacity: m.opacity,
+                    texture: m.texture.as_ref().map(|t| TextureInfo {
+                        width: t.width,
+                        height: t.height,
+                        pixel_width: t.image.width,
+                        pixel_height: t.image.height,
+                        format: t.image.format,
+                    }),
                     faces: meshes()
                         .flat_map(|mesh| mesh.faces())
                         .filter(|(_, f)| f.paint().contains(&Some(id)))
@@ -105,6 +149,40 @@ impl Document {
     }
 }
 
+/// The texture `source` asks for: the image at its path, or else `current`'s. A new image
+/// without a width is as wide as `current` (or 1 m), and without a height keeps its
+/// proportions; a kept image keeps any size not given. A file that cannot be read or decoded is
+/// a `file` error naming it.
+fn texture_of(source: &TextureSource, current: Option<&Texture>) -> Result<Texture, CommandError> {
+    let image = match &source.path {
+        Some(path) => {
+            let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
+            let bytes = read_limited(Path::new(path), MAX_TEXTURE_BYTES).map_err(file_error)?;
+            Arc::new(decode(bytes).map_err(file_error)?)
+        }
+        None => match current {
+            Some(t) => t.image.clone(),
+            None => {
+                return Err(CommandError::new(
+                    ErrorKind::InvalidInput,
+                    "the material has no texture: give a texture path",
+                ));
+            }
+        },
+    };
+    let width = source.width.or(current.map(|t| t.width)).unwrap_or(1.0);
+    let height = match (source.height, current) {
+        (Some(h), _) => h,
+        (None, Some(t)) if source.path.is_none() => t.height,
+        _ => width * f64::from(image.height) / f64::from(image.width),
+    };
+    Ok(Texture {
+        image,
+        width,
+        height,
+    })
+}
+
 /// A kernel refusal as a command error; a missing material is `not_found`, naming `id`.
 fn material_error(e: KernelError, id: Option<MaterialId>) -> CommandError {
     match (e, id) {
diff --git a/crates/session/src/file.rs b/crates/session/src/file.rs
index 0739459..2f450ac 100644
--- a/crates/session/src/file.rs
+++ b/crates/session/src/file.rs
@@ -1,8 +1,10 @@
 //! The native file format (`.skr`): a zip archive holding `document.json`, which records the
-//! format name, a schema version, the camera and the model with every id.
+//! format name, a schema version, the camera and the model with every id, and the texture
+//! images as the files they were read from (`textures/m<id>.png` or `.jpg`).
 
 use std::io::{Read, Write};
 use std::path::Path;
+use std::sync::Arc;
 
 use kernel::{MAX_ID, MeshData, Scene, SceneData};
 use serde::{Deserialize, Serialize};
@@ -10,16 +12,20 @@ use serde_json::Value;
 
 use crate::camera::{Camera, Projection};
 use crate::document::MAX_COORDINATE;
+use crate::texture::{MAX_TEXTURE_BYTES, decode};
 
 /// The `format` every file records.
 pub const FORMAT: &str = "sketchup-alt";
-/// The newest schema version this build reads, and the one it writes when a material is
-/// see-through. Version 2 added guides, version 3 curves and soft edges, version 4 the
-/// camera's up direction and projection, version 5 materials, and version 6 material
-/// opacity; older files still open.
-pub const SCHEMA_VERSION: u32 = 6;
-/// The version written when every material is opaque: such a model needs nothing version 6
-/// added, so its file is the same as before opacity existed and older builds open it.
+/// The newest schema version this build reads, and the one it writes when a material has a
+/// texture. Version 2 added guides, version 3 curves and soft edges, version 4 the camera's up
+/// direction and projection, version 5 materials, version 6 material opacity and version 7
+/// textures; older files still open.
+pub const SCHEMA_VERSION: u32 = 7;
+/// The version written when a material is see-through but none has a texture.
+pub const TRANSLUCENT_SCHEMA_VERSION: u32 = 6;
+/// The version written when every material is opaque and none has a texture: such a model
+/// needs nothing versions 6 and 7 added, so its file is the same as before and older builds
+/// open it.
 pub const OPAQUE_SCHEMA_VERSION: u32 = 5;
 /// The archive entry holding the document.
 const ENTRY: &str = "document.json";
@@ -58,11 +64,22 @@ pub(crate) fn write(path: &Path, scene: &Scene, camera: &Camera) -> Result<(), S
             "the model has used ids past {MAX_ID}, more than a file can hold"
         ));
     }
+    let textured = model.materials.iter().any(|m| m.texture.is_some());
     let translucent = model.materials.iter().any(|m| m.opacity < 1.0);
+    let images: Vec<(String, Arc<kernel::TextureImage>)> = model
+        .materials
+        .iter()
+        .filter_map(|m| {
+            let t = m.texture.as_ref()?;
+            Some((t.file.clone(), t.image.clone()?))
+        })
+        .collect();
     let stored = Stored {
         format: FORMAT.into(),
-        schema_version: if translucent {
+        schema_version: if textured {
             SCHEMA_VERSION
+        } else if translucent {
+            TRANSLUCENT_SCHEMA_VERSION
         } else {
             OPAQUE_SCHEMA_VERSION
         },
@@ -81,6 +98,14 @@ pub(crate) fn write(path: &Path, scene: &Scene, camera: &Camera) -> Result<(), S
         .compression_method(zip::CompressionMethod::Deflated);
     zip.start_file(ENTRY, options).map_err(|e| e.to_string())?;
     zip.write_all(&json).map_err(|e| e.to_string())?;
+    // PNG and JPEG files are compressed already.
+    let stored =
+        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
+    for (name, image) in &images {
+        zip.start_file(name.as_str(), stored)
+            .map_err(|e| e.to_string())?;
+        zip.write_all(&image.encoded).map_err(|e| e.to_string())?;
+    }
     let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
     replace_file(path, &bytes)
 }
@@ -126,8 +151,22 @@ pub(crate) fn read(path: &Path) -> Result<(Scene, StoredCamera), String> {
             "the file uses schema version {version}, newer than this app's {SCHEMA_VERSION}"
         ));
     }
-    let stored: Stored = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
+    let mut stored: Stored = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
     check_coordinates(&stored.model)?;
+    for m in &mut stored.model.materials {
+        let Some(t) = &mut m.texture else { continue };
+        if t.file == ENTRY {
+            return Err(format!("material m{}'s texture is not an image", m.id));
+        }
+        let entry = zip
+            .by_name(&t.file)
+            .map_err(|_| format!("material m{}'s texture {} is missing", m.id, t.file))?;
+        let bytes = read_limited(entry, MAX_TEXTURE_BYTES)
+            .map_err(|e| format!("material m{}'s texture {}: {e}", m.id, t.file))?;
+        let image =
+            decode(bytes).map_err(|e| format!("material m{}'s texture {}: {e}", m.id, t.file))?;
+        t.image = Some(Arc::new(image));
+    }
     let scene = Scene::from_data(&stored.model)?;
     let c = &stored.camera;
     if !c
diff --git a/crates/session/src/lib.rs b/crates/session/src/lib.rs
index 00b8fbc..2d57cc6 100644
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -11,22 +11,25 @@ mod import;
 mod query;
 mod rectangle;
 mod rotate;
+pub mod texture;
 
 pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points};
 pub use camera::{Camera, Projection, Projector, Ray, StandardView};
 pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
-pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
+pub use command::{
+    BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode, TextureSource,
+};
 pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
 pub use export::ExportReport;
 pub use import::{
     ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
     MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS, UpAxis,
 };
-pub use kernel::{CurveKind, MaterialId, Side};
+pub use kernel::{CurveKind, ImageFormat, MaterialId, Side};
 pub use query::{
     Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
     GuideKind, InstanceInfo, MaterialInfo, OutlinerNode, OutlinerRow, OutlinerTable, Query,
-    QueryResult, VertexInfo,
+    QueryResult, TextureInfo, VertexInfo,
 };
 pub use rectangle::{rectangle_axes, rectangle_corners};
 pub use rotate::rotation;
diff --git a/crates/session/src/query.rs b/crates/session/src/query.rs
index 536b692..a17c368 100644
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -1,6 +1,8 @@
 use std::collections::BTreeMap;
 
-use kernel::{ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId, MaterialId};
+use kernel::{
+    ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId, ImageFormat, MaterialId,
+};
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 
@@ -261,12 +263,25 @@ pub struct MaterialInfo {
     pub color: [u8; 3],
     /// From 0 (invisible) to 1 (opaque). Always present.
     pub opacity: f32,
+    /// The texture faces painted with it show; absent for none.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub texture: Option<TextureInfo>,
     /// Faces painted with it on either side, in the whole model (each definition counted once).
     pub faces: usize,
     /// Instances painted with it, in the whole model.
     pub instances: usize,
 }
 
+/// A material's texture: the size one copy covers on the model, in meters, and the image's.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+pub struct TextureInfo {
+    pub width: f64,
+    pub height: f64,
+    pub pixel_width: u32,
+    pub pixel_height: u32,
+    pub format: ImageFormat,
+}
+
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 #[serde(tag = "type", rename_all = "snake_case")]
 pub enum QueryResult {
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 32a8ba5..62e3c51 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -52,7 +52,7 @@ Queries:
 | `{"type": "camera"}` | the view: `eye`, `target`, `up`, `fov_y_deg`, `projection`, `view_height`; see Camera below |
 | `{"type": "selection"}` | the selected edge, face, instance and guide ids, plus `revision` |
 | `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
-| `{"type": "materials"}` | every material: `id`, `name`, `color`, `opacity`, and how many `faces` and `instances` in the whole model are painted with it; see Materials below |
+| `{"type": "materials"}` | every material: `id`, `name`, `color`, `opacity`, its `texture` if it has one, and how many `faces` and `instances` in the whole model are painted with it; see Materials below |
 | `{"type": "context"}` | the open context: `path` (open instances from the root, empty at the root) and the `definition` being edited |
 | `{"type": "outliner"}` | the tree of placed groups and components in the whole model, plus `revision`; optional `depth` limits it; see Groups and components below |
 | `{"type": "file"}` | `path` of the file last opened or saved (`null` if never), `modified` since then, and `revision` |
@@ -190,7 +190,7 @@ the `guides` list in `entities`, is `{"id": "g9", "kind": "line", "point": [...]
 
 ### Materials
 
-A material is a named color with an opacity. Faces have two sides, each painted with a material or with none
+A material is a named color with an opacity, and possibly a texture (an image). Faces have two sides, each painted with a material or with none
 (the default material: light on the front, blue-gray on the back, as in the window); instances
 can be painted too. Materials belong to the whole model, not to the open context, and are saved
 with it.
@@ -199,11 +199,13 @@ with it.
 |---|---|
 | `{"type": "add_material", "name": "Brick", "color": [180, 60, 40]}` | Adds a material. `color` is sRGB red, green and blue, whole numbers 0 to 255. `opacity` (optional, default 1) is a number from 0 (invisible) to 1 (opaque); anything else is `invalid_input`. Without `name` it is called `Material` (`Material 2`, ... when taken). A name must not be blank, must have at most 256 characters and must differ from every other material's (`invalid_input`). The new id is the outcome's `changes.materials`, e.g. `["m31"]`. |
 | `{"type": "change_material", "material": "m31", "name": "Old brick", "color": [150, 50, 30], "opacity": 0.4}` | Renames, recolors and/or changes the opacity of a material; an absent `name`, `color` or `opacity` stays. `opacity` must be from 0 to 1 (`invalid_input`). |
+| `{"type": "add_material", "name": "Tiles", "texture": {"path": "/abs/tiles.png", "width": 0.6}}` | Adds a material with a texture; see Textures below. `color` may then be left out: it is the image's average color. |
+| `{"type": "change_material", "material": "m31", "texture": {"path": "/abs/oak.jpg", "width": 1.2, "height": 0.2}}` | Gives a material a texture or replaces its image; the color becomes the image's average unless `color` is given too. Without `path` it keeps the image and changes its size (`invalid_input` when the material has none). `"texture": null` removes the texture and keeps the color. |
 | `{"type": "delete_material", "material": "m31"}` | Removes a material. Every face side and instance painted with it, anywhere in the model, goes back to the default material (and comes back with undo). |
 | `{"type": "paint", "ids": ["f8", "i4"], "material": "m31", "side": "front"}` | Paints faces and instances of the open context. `side` is `front` (default: the side the face's `normal` points to), `back` or `both`; it does not matter for instances. An absent or null `material` paints with the default material. Edge and guide ids are `invalid_input` (naming them), as are vertex ids and an empty list; unknown ids and an unknown material are `not_found`. Painting what is already painted so changes nothing. |
 
-Each is one undo step. A command that adds, removes, renames, recolors or changes the opacity of
-a material lists it in
+Each is one undo step. A command that adds, removes, renames, recolors or changes the opacity or
+texture of a material lists it in
 its `changes.materials` (also in `changed` notifications and `events`); painting reports the
 faces and instances as `modified`. Unknown material ids are `not_found`.
 
@@ -215,6 +217,31 @@ own material's opacity, so a face can be glass from one side and opaque from the
 of see-through faces (window frames, say) still draw, and a selected see-through face is tinted
 like any other.
 
+#### Textures
+
+A texture is a PNG or JPEG file of at most 16 MiB and 4096 pixels on each side, read when the
+command runs; the model keeps its own copy (the file is saved inside `.skr`), so the file may
+change or go away later. A model holds at most 256 textures. `texture` is `{"path", "width",
+"height"}`: `width` and `height` are the meters one copy of the image covers on the model, each
+from 0.001 to 1000 (`invalid_input` otherwise). A new image without `width` is 1 m wide (or as
+wide as the texture it replaces), and without `height` keeps the image's proportions. A file
+that cannot be read, is not a PNG or JPEG, or is too large gives the error kind `file`, naming
+the path; nothing changes.
+
+A face shows its side's material's texture in place of its color, shaded like any face (the
+material's opacity still applies; the image's own transparency is ignored). The image lies flat
+in the face's plane, in the coordinates of the group or component holding the face: its width
+runs along the face's horizontal direction (`u` = z x normal, normalized; along x on a level
+face) and its height across it (`v` = normal x u, up on a wall), repeating every `width` by
+`height` meters with a copy's bottom left corner at the definition's origin. Faces in one plane
+continue each other's pattern, and a component carries its pattern with it when moved, turned or
+scaled. Both sides of a face use the same placement, so from behind the image reads mirrored.
+The material's `color` stays the color edges, the Materials list and formats without textures
+show.
+
+The `materials` query adds `"texture": {"width": 0.6, "height": 0.3, "pixel_width": 1024,
+"pixel_height": 512, "format": "png"}` (`format` `png` or `jpeg`) to a material with a texture.
+
 The `entity` result of a face (and its entry in `entities`) has `front_material` and
 `back_material`, an instance's has `material`; each is absent for the default material.
 
@@ -398,13 +425,16 @@ lists these controls) turns it on; Esc or F turns it off.
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
-(`"sketchup-alt"`), `schema_version` (6 when a material is see-through, else 5; a material's
-`opacity` is written only when below 1, so a model whose materials are all opaque is saved
-exactly as before opacity existed and older builds still open it; version 5 files open with
+(`"sketchup-alt"`), `schema_version` (7 when a material has a texture, else 6 when a material
+is see-through, else 5; a material's `opacity` is written only when below 1, so a model whose
+materials are all opaque is saved exactly as before opacity existed and older builds still open
+it; version 6 and older files open without textures; version 5 files open with
 every material opaque, version 4 files, written before materials, open unpainted, version 3 files, written before camera projections, open in perspective, and
 versions 2 and 1, written before curves and guides, still open), the camera (eye, target, field
 of view, up and projection) and the model with every id, edges with their curve and soft flag,
-the materials and what is painted with them.
+the materials and what is painted with them. Each texture's image is stored in the archive as
+the file it was read from, `textures/m31.png` (or `.jpg`), and the material's `texture` in
+`document.json` names it with its `width` and `height`.
 
 | Command | Effect |
 |---|---|
@@ -421,7 +451,8 @@ reason (a missing file, not a sketchup-alt file, a missing or newer schema versi
 over 256 MiB, an id of 2^53 or more or one number used as the id of two things, a transform that cannot be inverted, a coordinate beyond the
 +/-1e6 m limit, a curve whose edges branch, fall apart or are straight, a soft edge bounding
 fewer than two faces, a face or instance painted with a material the file lacks, a material
-name that is blank, too long or used twice, or a material opacity outside 0 to 1); the model is left as it was. `save` also fails
+name that is blank, too long or used twice, a material opacity outside 0 to 1, or a texture
+whose image is missing, broken or too large, or whose size is out of range); the model is left as it was. `save` also fails
 when the model has given out ids past 2^53, which no file can hold. `open` and `new` never drop changes the user has
 not saved (`modified` in the `file` query) unless told to: they fail with `unsaved_changes`, and
 a client that means it, or has asked the user, passes `"discard": true`. The window asks the user
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p session --test textures` (5 pass), `cargo test -p session --lib texture` (1 pass) and `cargo test -p session` (all pass).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add Cargo.toml Cargo.lock crates/session/Cargo.toml crates/session/src/texture.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/materials.rs crates/session/src/document/import.rs crates/session/src/query.rs crates/session/src/file.rs crates/session/src/lib.rs crates/session/tests/textures.rs crates/session/tests/materials.rs crates/app/src/editor.rs crates/app/src/info.rs crates/render/tests/golden.rs docs/agent-api.md
git commit -m "Textures on add_material and change_material; .skr schema 7"
```

---

### Task 3: glTF and OBJ export with textures

**Files:**
- Modify: `crates/session/src/export.rs`, `crates/session/src/document/files.rs`, `docs/agent-api.md`
- Test: `crates/session/tests/export.rs`

**Interfaces:**
- Consumes: `Triangle.uvs` (Task 1), `Material.texture`, `ImageFormat::{extension, mime}`, the `texture` field of `add_material` (Task 2).
- Produces: `export::Encoded.images: Vec<(String, Vec<u8>)>` (file names beside the `.mtl`); `obj()` now returns `Encoded`.

- [ ] **Step 1: Write the failing tests**

Apply to the existing tests:

```diff
diff --git a/crates/session/tests/export.rs b/crates/session/tests/export.rs
index 78415ba..8eb22e5 100644
--- a/crates/session/tests/export.rs
+++ b/crates/session/tests/export.rs
@@ -700,3 +700,146 @@ fn gltf_normals_are_smooth_across_soft_edges() {
     }
     assert_eq!(sides, 12 * 6);
 }
+
+/// `two_boxes` with the first crate painted with a material whose texture is a 4 x 2 pixel PNG
+/// covering 2 x 1 m. Returns the PNG's bytes.
+fn textured_boxes(d: &mut Document, png: &str) -> Vec<u8> {
+    two_boxes(d);
+    let path = scratch(png);
+    image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([60 * x as u8, 0, 200, 255]))
+        .save(&path)
+        .unwrap();
+    let out = run(
+        d,
+        json!({"type": "add_material", "name": "Tiles",
+               "texture": {"path": path.to_str().unwrap(), "width": 2}}),
+    )
+    .unwrap();
+    let material = out["changes"]["materials"][0].clone();
+    let instances = ask(d, json!({"type": "entities", "kinds": ["instance"]}));
+    let first = instances["instances"][0]["id"].clone();
+    run(
+        d,
+        json!({"type": "paint", "ids": [first], "material": material}),
+    )
+    .unwrap();
+    std::fs::read(&path).unwrap()
+}
+
+/// The `.glb` chunks: the document and the buffer.
+fn glb_parts(bytes: &[u8]) -> (Value, Vec<u8>) {
+    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
+    let json_len = word(12);
+    let doc = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
+    let bin = 20 + json_len;
+    (doc, bytes[bin + 8..bin + 8 + word(bin)].to_vec())
+}
+
+fn floats(bytes: &[u8]) -> Vec<f32> {
+    bytes
+        .as_chunks::<4>()
+        .0
+        .iter()
+        .map(|b| f32::from_le_bytes(*b))
+        .collect()
+}
+
+#[test]
+fn gltf_embeds_textures_with_texture_coordinates() {
+    let mut d = doc();
+    let png = textured_boxes(&mut d, "gltf-tiles.png");
+    let (doc, buffer) = glb_parts(&export(&mut d, "textured.glb"));
+    let tiles = &doc["materials"][0];
+    assert_eq!(tiles["name"], "Tiles");
+    assert_eq!(
+        tiles["pbrMetallicRoughness"]["baseColorTexture"],
+        json!({"index": 0})
+    );
+    assert_eq!(
+        tiles["pbrMetallicRoughness"]["baseColorFactor"],
+        json!([1.0, 1.0, 1.0, 1.0])
+    );
+    assert_eq!(doc["textures"], json!([{"sampler": 0, "source": 0}]));
+    assert_eq!(doc["samplers"][0]["wrapS"], 10497);
+    assert_eq!(doc["samplers"][0]["wrapT"], 10497);
+    assert_eq!(doc["images"][0]["mimeType"], "image/png");
+    // The image is the file as read.
+    let view = &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().unwrap() as usize];
+    let at = view["byteOffset"].as_u64().unwrap() as usize;
+    let len = view["byteLength"].as_u64().unwrap() as usize;
+    assert_eq!(&buffer[at..at + len], &png[..]);
+    // The default material has no texture coordinates.
+    let primitives = doc["meshes"][0]["primitives"].as_array().unwrap();
+    assert!(primitives[1]["attributes"]["TEXCOORD_0"].is_null());
+    assert!(doc["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"].is_null());
+
+    // On the crate's wall facing -y (z = 0 in Y-up) the texture runs along x and up, one copy
+    // per 2 x 1 m: s = x / 2 and t = -height, up to whole repeats.
+    let accessor = |k: &Value| {
+        let a = &doc["accessors"][k.as_u64().unwrap() as usize];
+        let view = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
+        let at =
+            (view["byteOffset"].as_u64().unwrap() + a["byteOffset"].as_u64().unwrap()) as usize;
+        let width = if a["type"] == "VEC2" { 8 } else { 12 };
+        floats(&buffer[at..at + width * a["count"].as_u64().unwrap() as usize])
+    };
+    let positions = accessor(&primitives[0]["attributes"]["POSITION"]);
+    let st = accessor(&primitives[0]["attributes"]["TEXCOORD_0"]);
+    assert_eq!(st.len(), 36 * 2);
+    let mut wall = 0;
+    for (p, s) in positions
+        .as_chunks::<9>()
+        .0
+        .iter()
+        .zip(st.as_chunks::<6>().0)
+    {
+        if [p[2], p[5], p[8]].iter().any(|z| *z != 0.0) {
+            continue;
+        }
+        for k in 0..3 {
+            let (x, height) = (p[3 * k], p[3 * k + 1]);
+            let (ds, dt) = (s[2 * k] - x / 2.0, s[2 * k + 1] + height);
+            assert!((ds - ds.round()).abs() < 1e-6, "{p:?} {s:?}");
+            assert!((dt - dt.round()).abs() < 1e-6, "{p:?} {s:?}");
+            assert!(
+                s[2 * k].abs() <= 2.0 && s[2 * k + 1].abs() <= 2.0,
+                "kept small"
+            );
+        }
+        wall += 1;
+    }
+    assert_eq!(wall, 2);
+
+    // A .gltf carries the image in its embedded buffer the same way.
+    let doc: Value = serde_json::from_slice(&export(&mut d, "textured.gltf")).unwrap();
+    assert_eq!(doc["images"][0]["mimeType"], "image/png");
+    assert!(doc["images"][0]["bufferView"].is_u64());
+}
+
+#[test]
+fn obj_writes_texture_coordinates_and_images_beside_the_library() {
+    let mut d = doc();
+    let png = textured_boxes(&mut d, "obj-tiles.png");
+    let obj = String::from_utf8(export(&mut d, "tiled-boxes.obj")).unwrap();
+    assert!(obj.lines().any(|l| l.starts_with("vt ")), "{obj}");
+    let faces: Vec<&str> = obj.lines().filter(|l| l.starts_with("f ")).collect();
+    assert_eq!(faces.len(), 24);
+    // The textured crate's faces name texture coordinates; the default ones do not.
+    assert_eq!(faces.iter().filter(|f| !f.contains("//")).count(), 12);
+    let library = std::fs::read_to_string(scratch_kept("tiled-boxes.mtl")).unwrap();
+    assert!(
+        library.contains("newmtl Tiles\nKd ") && library.contains("map_Kd tiled-boxes_Tiles.png\n"),
+        "{library}"
+    );
+    assert_eq!(
+        std::fs::read(scratch_kept("tiled-boxes_Tiles.png")).unwrap(),
+        png
+    );
+}
+
+/// A path in the scratch directory, left as it is.
+fn scratch_kept(name: &str) -> PathBuf {
+    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
+        .join(format!("session-export-{}", std::process::id()))
+        .join(name)
+}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p session --test export texture`
Expected: both new tests fail (no `images`, `textures` or `TEXCOORD_0` in the glTF; no `vt` or `map_Kd` in the OBJ).

- [ ] **Step 3: Implement**

Apply:

```diff
diff --git a/crates/session/src/document/files.rs b/crates/session/src/document/files.rs
index 504c490..c042335 100644
--- a/crates/session/src/document/files.rs
+++ b/crates/session/src/document/files.rs
@@ -127,6 +127,12 @@ impl Document {
                     ),
                 ));
             }
+            // Texture images go beside the library, before it, as it names them.
+            for (name, bytes) in &encoded.images {
+                let image = mtl_path.with_file_name(name);
+                file::replace_file(&image, bytes)
+                    .map_err(|e| file_error(&image.display().to_string(), e))?;
+            }
             file::replace_file(&mtl_path, mtl)
                 .map_err(|e| file_error(&mtl_path.display().to_string(), e))?;
         }
diff --git a/crates/session/src/export.rs b/crates/session/src/export.rs
index 7961925..4aa8ea7 100644
--- a/crates/session/src/export.rs
+++ b/crates/session/src/export.rs
@@ -1,15 +1,16 @@
 //! Exporting the model's faces as triangles: OBJ (with a `.mtl` beside it when faces are
-//! painted), STL, and glTF (binary `.glb` or `.gltf` with its buffer embedded). Units are
-//! meters. OBJ and glTF are Y-up (x, z, -y); STL keeps the model's Z-up coordinates. Colors are
-//! those of the faces' front sides; STL has none.
+//! painted, and texture images beside that), STL, and glTF (binary `.glb` or `.gltf` with its
+//! buffer embedded, textures included). Units are meters. OBJ and glTF are Y-up (x, z, -y); STL
+//! keeps the model's Z-up coordinates. Colors and textures are those of the faces' front sides;
+//! STL has none.
 
 use std::collections::{BTreeSet, HashMap};
 use std::fmt::Write as _;
 use std::path::Path;
 
 use base64::Engine;
-use glam::DVec3;
-use kernel::{MaterialId, Scene, Triangle};
+use glam::{DVec2, DVec3};
+use kernel::{MaterialId, Scene, Texture, Triangle};
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 use serde_json::json;
@@ -55,10 +56,12 @@ pub struct ExportReport {
 }
 
 /// What an export writes: the file's bytes and, for an OBJ whose faces are painted, the bytes
-/// of the material library (`.mtl`) it names.
+/// of the material library (`.mtl`) it names and the texture images the library names, each
+/// with its file name (written beside the library).
 pub(crate) struct Encoded {
     pub bytes: Vec<u8>,
     pub mtl: Option<Vec<u8>>,
+    pub images: Vec<(String, Vec<u8>)>,
 }
 
 /// The bytes of `triangles` in `format`. Materials are looked up in `scene`; an OBJ refers to
@@ -72,26 +75,16 @@ pub(crate) fn encode(
     mtl_name: &str,
 ) -> Result<Encoded, String> {
     let groups = Groups::of(scene, triangles);
+    let only = |bytes| Encoded {
+        bytes,
+        mtl: None,
+        images: Vec::new(),
+    };
     Ok(match format {
-        ExportFormat::Obj => {
-            let (obj, mtl) = obj(&groups, mtl_name);
-            Encoded {
-                bytes: obj.into_bytes(),
-                mtl: mtl.map(String::into_bytes),
-            }
-        }
-        ExportFormat::Stl => Encoded {
-            bytes: stl(triangles)?,
-            mtl: None,
-        },
-        ExportFormat::Glb => Encoded {
-            bytes: glb(&groups)?,
-            mtl: None,
-        },
-        ExportFormat::Gltf => Encoded {
-            bytes: gltf(&groups).into_bytes(),
-            mtl: None,
-        },
+        ExportFormat::Obj => obj(&groups, mtl_name),
+        ExportFormat::Stl => only(stl(triangles)?),
+        ExportFormat::Glb => only(glb(&groups)?),
+        ExportFormat::Gltf => only(gltf(&groups).into_bytes()),
     })
 }
 
@@ -134,30 +127,34 @@ struct Group<'a> {
     color: [f64; 3],
     /// 1 is opaque.
     opacity: f32,
+    texture: Option<&'a Texture>,
     triangles: Vec<&'a Triangle>,
 }
 
 impl<'a> Groups<'a> {
-    fn of(scene: &Scene, triangles: &'a [Triangle]) -> Groups<'a> {
+    fn of(scene: &'a Scene, triangles: &'a [Triangle]) -> Groups<'a> {
         let mut groups: Vec<Group<'a>> = Vec::new();
         for t in triangles {
             let material = t.front.filter(|m| scene.material(*m).is_some());
             let i = match groups.iter().position(|g| g.material == material) {
                 Some(i) => i,
                 None => {
-                    let (name, color, opacity) = match material.and_then(|m| scene.material(m)) {
-                        Some(m) => (
-                            m.name.clone(),
-                            m.color.map(|c| f64::from(c) / 255.0),
-                            m.opacity,
-                        ),
-                        None => ("default".to_string(), DEFAULT_COLOR, 1.0),
-                    };
+                    let (name, color, opacity, texture) =
+                        match material.and_then(|m| scene.material(m)) {
+                            Some(m) => (
+                                m.name.clone(),
+                                m.color.map(|c| f64::from(c) / 255.0),
+                                m.opacity,
+                                m.texture.as_ref(),
+                            ),
+                            None => ("default".to_string(), DEFAULT_COLOR, 1.0, None),
+                        };
                     groups.push(Group {
                         material,
                         name,
                         color,
                         opacity,
+                        texture,
                         triangles: Vec::new(),
                     });
                     groups.len() - 1
@@ -178,6 +175,17 @@ impl<'a> Groups<'a> {
     }
 }
 
+/// A triangle's texture coordinates on `texture`: its corners' texture positions over the
+/// texture's size, with v counted down from the image's top (glTF's way), shifted by whole
+/// repeats so the numbers stay small.
+fn texcoords(t: &Triangle, texture: &Texture) -> [DVec2; 3] {
+    let st = t
+        .uvs
+        .map(|p| DVec2::new(p.x / texture.width, -p.y / texture.height));
+    let shift = st[0].floor();
+    st.map(|q| q - shift)
+}
+
 /// An sRGB component (0 to 1) as linear light, as glTF colors and most `.mtl` readers take it.
 pub(crate) fn linear(c: f64) -> f64 {
     if c <= 0.04045 {
@@ -228,9 +236,10 @@ fn y_up(p: DVec3) -> DVec3 {
     DVec3::new(p.x + 0.0, p.z + 0.0, -p.y + 0.0)
 }
 
-/// OBJ with shared positions and normals; faces are triangles. When some faces are painted,
-/// each material's faces follow a `usemtl` line and the material library is returned too.
-fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
+/// OBJ with shared positions, normals and texture coordinates; faces are triangles. When some
+/// faces are painted, each material's faces follow a `usemtl` line and the material library is
+/// returned too, with the texture images it names (`<library stem>_<material>.png` or `.jpg`).
+fn obj(groups: &Groups, mtl_name: &str) -> Encoded {
     let mut out = String::from("# sketchup-alt export: meters, Y up\n");
     let painted = groups.painted();
     if painted {
@@ -240,6 +249,7 @@ fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
     let names = obj_names(groups);
     let mut positions: HashMap<[u64; 3], usize> = HashMap::new();
     let mut normals: HashMap<[u64; 3], usize> = HashMap::new();
+    let mut texcoords: HashMap<[u64; 2], usize> = HashMap::new();
     let mut faces = String::new();
     let key = |p: DVec3| p.to_array().map(|c| (c + 0.0).to_bits());
     for (g, name) in groups.groups.iter().zip(&names) {
@@ -247,6 +257,19 @@ fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
             let _ = writeln!(faces, "usemtl {name}");
         }
         for t in &g.triangles {
+            // OBJ counts v up from the image's bottom, so the meters divide straight.
+            let vt = g.texture.map(|tex| {
+                t.uvs.map(|p| {
+                    let st = DVec2::new(p.x / tex.width, p.y / tex.height);
+                    let next = texcoords.len() + 1;
+                    *texcoords
+                        .entry(st.to_array().map(|c| (c + 0.0).to_bits()))
+                        .or_insert_with(|| {
+                            let _ = writeln!(out, "vt {} {}", st.x + 0.0, st.y + 0.0);
+                            next
+                        })
+                })
+            });
             let n = y_up(t.normal);
             let next = normals.len() + 1;
             let ni = *normals.entry(key(n)).or_insert_with(|| {
@@ -262,15 +285,28 @@ fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
                     next
                 });
             }
-            let _ = writeln!(
-                faces,
-                "f {}//{ni} {}//{ni} {}//{ni}",
-                ids[0], ids[1], ids[2]
-            );
+            match vt {
+                Some([ta, tb, tc]) => {
+                    let _ = writeln!(
+                        faces,
+                        "f {}/{ta}/{ni} {}/{tb}/{ni} {}/{tc}/{ni}",
+                        ids[0], ids[1], ids[2]
+                    );
+                }
+                None => {
+                    let _ = writeln!(
+                        faces,
+                        "f {}//{ni} {}//{ni} {}//{ni}",
+                        ids[0], ids[1], ids[2]
+                    );
+                }
+            }
         }
     }
     out.push_str(&faces);
+    let mut images = Vec::new();
     let mtl = painted.then(|| {
+        let stem = mtl_name.strip_suffix(".mtl").unwrap_or(mtl_name);
         let mut mtl = String::from("# sketchup-alt materials: diffuse colors, linear\n");
         for (g, name) in groups.groups.iter().zip(&names) {
             let [r, gr, b] = g.color.map(linear);
@@ -280,10 +316,19 @@ fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
                 let d = f64::from(g.opacity);
                 let _ = writeln!(mtl, "d {d:.6}\nTr {:.6}", 1.0 - d);
             }
+            if let Some(t) = g.texture {
+                let file = format!("{stem}_{name}.{}", t.image.format.extension());
+                let _ = writeln!(mtl, "map_Kd {file}");
+                images.push((file, t.image.encoded.clone()));
+            }
         }
         mtl
     });
-    (out, mtl)
+    Encoded {
+        bytes: out.into_bytes(),
+        mtl: mtl.map(String::into_bytes),
+        images,
+    }
 }
 
 /// Binary STL: an 80-byte header, the triangle count, then per triangle its normal, its three
@@ -307,9 +352,10 @@ fn stl(triangles: &[Triangle]) -> Result<Vec<u8>, String> {
     Ok(out)
 }
 
-/// The vertex buffer (positions then corner normals, f32 little-endian, Y-up, each material's
-/// triangles together) and the glTF document describing it: one primitive per material, its
-/// buffer without a `uri` (a `.glb`'s own chunk).
+/// The buffer (positions then corner normals, f32 little-endian, Y-up, each material's
+/// triangles together; then, when some material has a texture, the texture coordinates of
+/// those materials' triangles and each texture's image file) and the glTF document describing
+/// it: one primitive per material, its buffer without a `uri` (a `.glb`'s own chunk).
 fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
     let count: usize = groups.groups.iter().map(|g| g.triangles.len() * 3).sum();
     let mut buffer = Vec::with_capacity(count * 24);
@@ -338,6 +384,32 @@ fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
         }
     }
     let half = count * 12;
+    let mut views = vec![
+        json!({"buffer": 0, "byteOffset": 0, "byteLength": half, "target": 34962}),
+        json!({"buffer": 0, "byteOffset": half, "byteLength": half, "target": 34962}),
+    ];
+    // Each textured material's first texture coordinate in the third view.
+    let mut texcoord_starts = Vec::new();
+    let mut texcoord_count = 0;
+    for g in &groups.groups {
+        texcoord_starts.push(texcoord_count);
+        if let Some(texture) = g.texture {
+            for t in &g.triangles {
+                for st in texcoords(t, texture) {
+                    for x in st.to_array() {
+                        buffer.extend_from_slice(&(x as f32).to_le_bytes());
+                    }
+                }
+            }
+            texcoord_count += g.triangles.len() * 3;
+        }
+    }
+    if texcoord_count > 0 {
+        views.push(json!({"buffer": 0, "byteOffset": 2 * half,
+            "byteLength": texcoord_count * 8, "target": 34962}));
+    }
+    let mut images = Vec::new();
+    let mut textures = Vec::new();
     let mut accessors = Vec::new();
     let mut primitives = Vec::new();
     let mut materials = Vec::new();
@@ -353,15 +425,33 @@ fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
             json!({"bufferView": 1, "byteOffset": start * 12, "componentType": 5126,
             "count": n, "type": "VEC3"}),
         );
-        primitives.push(
-            json!({"attributes": {"POSITION": 2 * k, "NORMAL": 2 * k + 1},
-            "mode": 4, "material": k}),
-        );
+        let mut attributes =
+            json!({"POSITION": accessors.len() - 2, "NORMAL": accessors.len() - 1});
         let mut pbr = json!({"metallicFactor": 0});
-        if g.material.is_some() {
+        if let Some(texture) = g.texture {
+            attributes["TEXCOORD_0"] = json!(accessors.len());
+            accessors.push(
+                json!({"bufferView": 2, "byteOffset": texcoord_starts[k] * 8,
+                "componentType": 5126, "count": n, "type": "VEC2"}),
+            );
+            // The image file as it was read, aligned to 4 bytes in the buffer.
+            while buffer.len() % 4 != 0 {
+                buffer.push(0);
+            }
+            let image = &texture.image;
+            views.push(json!({"buffer": 0, "byteOffset": buffer.len(),
+                "byteLength": image.encoded.len()}));
+            buffer.extend_from_slice(&image.encoded);
+            images.push(json!({"bufferView": views.len() - 1, "mimeType": image.format.mime()}));
+            textures.push(json!({"sampler": 0, "source": images.len() - 1}));
+            // The texture shows instead of the color; the opacity stays.
+            pbr["baseColorFactor"] = json!([1.0, 1.0, 1.0, decimal(g.opacity)]);
+            pbr["baseColorTexture"] = json!({"index": textures.len() - 1});
+        } else if g.material.is_some() {
             let [r, gr, b] = g.color.map(linear);
             pbr["baseColorFactor"] = json!([r, gr, b, decimal(g.opacity)]);
         }
+        primitives.push(json!({"attributes": attributes, "mode": 4, "material": k}));
         let mut material = json!({"name": g.name, "doubleSided": true,
             "pbrMetallicRoughness": pbr});
         if g.opacity < 1.0 {
@@ -370,7 +460,7 @@ fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
         materials.push(material);
         start += n;
     }
-    let doc = json!({
+    let mut doc = json!({
         "asset": {"version": "2.0", "generator": "sketchup-alt"},
         "scene": 0,
         "scenes": [{"nodes": [0]}],
@@ -378,12 +468,16 @@ fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
         "materials": materials,
         "meshes": [{"primitives": primitives}],
         "buffers": [{"byteLength": buffer.len()}],
-        "bufferViews": [
-            {"buffer": 0, "byteOffset": 0, "byteLength": half, "target": 34962},
-            {"buffer": 0, "byteOffset": half, "byteLength": half, "target": 34962}
-        ],
+        "bufferViews": views,
         "accessors": accessors
     });
+    if !textures.is_empty() {
+        // Linear filtering with mipmaps, repeating both ways.
+        doc["samplers"] =
+            json!([{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]);
+        doc["textures"] = json!(textures);
+        doc["images"] = json!(images);
+    }
     (buffer, doc)
 }
 
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 62e3c51..7e52969 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -471,13 +471,15 @@ never overwrites a model; use `save`.
 
 | Format | Content |
 |---|---|
-| `obj` | Text OBJ: shared `v` positions and `vn` normals, one `f` triangle per line; Y up (x, z, -y). When faces are painted, it names a material library (`mtllib house.mtl`) written beside it under the same name with `.mtl` (replacing a file there), and each material's triangles follow a `usemtl` line; the library gives each material its diffuse color (`Kd`, linear) and, for a see-through one, its opacity as `d` and its transparency `Tr` (1 - `d`). Material names have spaces and other separators replaced by `_` (and a number added when two become alike); faces without a material use `default`. Without painted faces no `.mtl` is written. |
+| `obj` | Text OBJ: shared `v` positions and `vn` normals, one `f` triangle per line; Y up (x, z, -y). When faces are painted, it names a material library (`mtllib house.mtl`) written beside it under the same name with `.mtl` (replacing a file there), and each material's triangles follow a `usemtl` line; the library gives each material its diffuse color (`Kd`, linear) and, for a see-through one, its opacity as `d` and its transparency `Tr` (1 - `d`). Material names have spaces and other separators replaced by `_` (and a number added when two become alike); faces without a material use `default`. Without painted faces no `.mtl` is written. A material with a texture adds `map_Kd house_Tiles.png` (the library's name, `_`, the material's name, and the image's own extension), and that image file, as it was read, is written beside the library (replacing a file there); its triangles name `vt` texture coordinates (`f v/vt/vn`), counted from the image's bottom left. |
 | `stl` | Binary STL: one record per triangle with its normal; Z up, as in the model; no colors or opacity |
-| `glb` | Binary glTF 2.0: one mesh of flat-shaded triangles (`POSITION`, `NORMAL`), one primitive per material; materials are double-sided, so lone faces show from behind; a painted one has its name and its color as `baseColorFactor` (linear, with the opacity as alpha), and a see-through one `alphaMode` `BLEND`; the default one is named `default`; Y up |
+| `glb` | Binary glTF 2.0: one mesh of flat-shaded triangles (`POSITION`, `NORMAL`), one primitive per material; materials are double-sided, so lone faces show from behind; a painted one has its name and its color as `baseColorFactor` (linear, with the opacity as alpha), and a see-through one `alphaMode` `BLEND`; the default one is named `default`; a material with a texture has a `TEXCOORD_0` attribute and a `baseColorTexture` (its image file, as it was read, in the buffer; a sampler with linear mipmapped filtering that repeats both ways) with a white `baseColorFactor` (and the opacity as alpha); Y up |
 | `gltf` | The same glTF document as JSON, its buffer embedded as a base64 data URI |
 
-Colors come from each face's front side (its own material, else that of the nearest painted
-instance placing it); back-side materials are not exported. An OBJ export whose `.mtl` would
+Colors and textures come from each face's front side (its own material, else that of the nearest
+painted instance placing it); back-side materials are not exported. Texture coordinates follow
+the projection described under Textures. The images are written before the `.mtl`. An OBJ
+export whose `.mtl` would
 be the export path itself or the open model's file is refused with `invalid_input`. The
 `.mtl` is written before the `.obj`, so a failure writing the `.obj` can leave a new `.mtl`
 beside the old (or no) `.obj`.
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p session --test export` (21 pass; the untextured tests still see the old buffers).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/session/src/export.rs crates/session/src/document/files.rs crates/session/tests/export.rs docs/agent-api.md
git commit -m "Export: textures in glTF and OBJ"
```

---

### Task 4: Texture array rendering and the textured golden image

**Files:**
- Create: `crates/render/src/texture.rs`, `crates/render/tests/golden/textured.png` (generated)
- Modify: `crates/render/src/scene.rs`, `crates/render/src/renderer.rs`, `crates/render/src/shaders/scene.wgsl`, `crates/render/src/lib.rs`
- Test: `crates/render/tests/golden.rs`, unit tests in `crates/render/src/texture.rs` and `crates/render/src/scene.rs`

**Interfaces:**
- Consumes: `Material.texture`, `texture_position`, `Scene::add_material_with_texture`, `change_material_with_texture` (Task 1); `session::TextureSource` (Task 2).
- Produces:
  - `render::texture::{textured(&Scene) -> Vec<(MaterialId, &Texture)>, layers(&Scene) -> HashMap<MaterialId, u32>, layer_side(&[&TextureImage]) -> u32, resample(pixels: &[u8], width: u32, height: u32, side: u32) -> Vec<u8>, mipmaps(Vec<u8>, side: u32) -> Vec<Vec<u8>>, MAX_LAYER_SIDE = 1024, MAX_ARRAY_BYTES = 256 MiB}` (`resample` is used by the app for swatches in Task 5)
  - `BaseFaceVertex` / `FaceVertex` gain `uv: [f32; 4]`, `layer: [f32; 2]`
  - Bind group 0: binding 1 `texture_2d_array<f32>`, binding 2 filtering sampler

- [ ] **Step 1: Write the failing tests**

Apply to the existing tests:

```diff
diff --git a/crates/render/tests/golden.rs b/crates/render/tests/golden.rs
index 3805d56..f76cbc3 100644
--- a/crates/render/tests/golden.rs
+++ b/crates/render/tests/golden.rs
@@ -689,6 +689,73 @@ fn golden_open_group_fades_the_rest() {
     check("open_group", &d);
 }
 
+/// A 16 x 16 pixel PNG in four quadrants, red top left, yellow top right, green bottom left and
+/// blue bottom right, with a white line along its top and left edges; returns its path.
+fn quadrants_png() -> PathBuf {
+    let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
+        .join(format!("golden-textures-{}", std::process::id()))
+        .join("quadrants.png");
+    let mut rgba = Vec::new();
+    for y in 0..16 {
+        for x in 0..16 {
+            let color = match (x < 8, y < 8) {
+                _ if x == 0 || y == 0 => [255, 255, 255],
+                (true, true) => [220, 40, 40],
+                (false, true) => [235, 200, 40],
+                (true, false) => [40, 170, 70],
+                (false, false) => [40, 80, 220],
+            };
+            rgba.extend(color);
+            rgba.push(255);
+        }
+    }
+    write_png(&path, &rgba, 16, 16);
+    path
+}
+
+#[test]
+fn golden_textured_box_and_ground() {
+    let mut d = doc();
+    add_box(&mut d, [0.0, 0.0]);
+    // A ground square beside the box, to show the texture on a level face.
+    run(
+        &mut d,
+        Command::AddRectangle {
+            corner: [2.5, 0.0, 0.0],
+            opposite: [4.5, 2.0, 0.0],
+            normal: None,
+        },
+    );
+    let tiles = run(
+        &mut d,
+        Command::AddMaterial {
+            name: Some("Tiles".into()),
+            color: None,
+            opacity: None,
+            texture: Some(session::TextureSource {
+                path: Some(quadrants_png().display().to_string()),
+                width: Some(1.0),
+                height: Some(1.0),
+            }),
+        },
+    )
+    .changes
+    .materials[0];
+    let faces = all_faces(&d);
+    run(
+        &mut d,
+        Command::Paint {
+            ids: faces,
+            material: Some(tiles),
+            side: Side::Both,
+        },
+    );
+    // Each wall shows whole 1 m copies upright (white lines along their tops and left sides);
+    // the top and the ground square show them with north up.
+    look(&mut d, [5.5, -5.0, 4.5], [2.0, 0.9, 0.3]);
+    check("textured", &d);
+}
+
 /// Every stored golden image belongs to a test here, so stale ones do not pile up.
 #[test]
 fn every_golden_image_has_a_test() {
@@ -706,6 +773,7 @@ fn every_golden_image_has_a_test() {
         "selection",
         "open_group",
         "glass",
+        "textured",
     ];
     let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
     let mut stored: Vec<String> = std::fs::read_dir(&dir)
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p render --test golden`
Expected: `golden_textured_box_and_ground` panics with "no readable golden image at .../textured.png", and `every_golden_image_has_a_test` fails (no stored `textured.png`). Textured faces would still show their color until Step 3.

- [ ] **Step 3: Implement**

After applying, generate the new image and look at it before committing: `UPDATE_GOLDEN=1 cargo test -p render --test golden textured`. It should show the box and the ground square covered in 1 m copies of the four-color image: on each wall red top left, yellow top right, green bottom left, blue bottom right, with white lines along the top and left of each copy; on the box top and the ground square the same with north up. The other golden images must not change (`git status crates/render/tests/golden/`).

Create `crates/render/src/texture.rs`:

```rust
//! Material textures on the GPU: one texture array, a layer per textured material, each image
//! resampled to the layer size with its mipmaps worked out here.

use std::collections::HashMap;
use std::sync::Arc;

use kernel::{MaterialId, Scene, Texture, TextureImage};

/// The largest layer side, in pixels.
pub const MAX_LAYER_SIDE: u32 = 1024;
/// The most bytes the array may take with its mipmaps; more or bigger textures get smaller
/// layers.
pub const MAX_ARRAY_BYTES: u64 = 256 * 1024 * 1024;

/// Every material with a texture, in id order: material k's texture is layer k.
pub fn textured(scene: &Scene) -> Vec<(MaterialId, &Texture)> {
    scene
        .materials()
        .filter_map(|(id, m)| Some((id, m.texture.as_ref()?)))
        .collect()
}

/// The array layer of each textured material (see `textured`).
pub fn layers(scene: &Scene) -> HashMap<MaterialId, u32> {
    textured(scene)
        .into_iter()
        .enumerate()
        .map(|(k, (id, _))| (id, k as u32))
        .collect()
}

/// The side of the array's square layers for `images`: a power of two, enough for the largest
/// image but at most [`MAX_LAYER_SIDE`], and small enough that all layers with their mipmaps
/// fit in [`MAX_ARRAY_BYTES`]. 1 without images.
pub fn layer_side(images: &[&TextureImage]) -> u32 {
    let largest = images
        .iter()
        .map(|i| i.width.max(i.height))
        .max()
        .unwrap_or(1);
    let mut side = largest.next_power_of_two().min(MAX_LAYER_SIDE);
    // A full mip chain adds a third.
    let bytes = |side: u32| images.len() as u64 * u64::from(side).pow(2) * 4 * 4 / 3;
    while side > 1 && bytes(side) > MAX_ARRAY_BYTES {
        side /= 2;
    }
    side
}

/// The source pixels and weights each of `out` pixels along an axis of `len` source pixels
/// takes: the pixels it covers when shrinking, the two nearest when growing. Indices wrap, as
/// the texture repeats.
fn taps(len: u32, out: u32) -> Vec<Vec<(usize, f32)>> {
    let (len, scale) = (len as usize, f64::from(len) / f64::from(out));
    let wrap = |x: f64| x.rem_euclid(len as f64) as usize;
    (0..out)
        .map(|i| {
            if scale > 1.0 {
                let (a, b) = (f64::from(i) * scale, f64::from(i + 1) * scale);
                let mut taps = Vec::new();
                let mut x = a.floor();
                while x < b {
                    let w = (b.min(x + 1.0) - a.max(x)) / scale;
                    if w > 0.0 {
                        taps.push((wrap(x), w as f32));
                    }
                    x += 1.0;
                }
                taps
            } else {
                let c = (f64::from(i) + 0.5) * scale - 0.5;
                let t = c - c.floor();
                vec![
                    (wrap(c.floor()), (1.0 - t) as f32),
                    (wrap(c.floor() + 1.0), t as f32),
                ]
            }
        })
        .collect()
}

/// RGBA8 `pixels` of a `width` x `height` image resampled to `side` x `side` (box filtered when
/// shrinking, linear when growing; both ways separately).
pub fn resample(pixels: &[u8], width: u32, height: u32, side: u32) -> Vec<u8> {
    let (w, s) = (width as usize, side as usize);
    let across = taps(width, side);
    let down = taps(height, side);
    // Rows first: height rows of `side` pixels.
    let mut rows = vec![0f32; height as usize * s * 4];
    for y in 0..height as usize {
        for (x, taps) in across.iter().enumerate() {
            for c in 0..4 {
                rows[(y * s + x) * 4 + c] = taps
                    .iter()
                    .map(|&(k, wt)| f32::from(pixels[(y * w + k) * 4 + c]) * wt)
                    .sum();
            }
        }
    }
    let mut out = vec![0u8; s * s * 4];
    for (y, taps) in down.iter().enumerate() {
        for x in 0..s {
            for c in 0..4 {
                let v: f32 = taps
                    .iter()
                    .map(|&(k, wt)| rows[(k * s + x) * 4 + c] * wt)
                    .sum();
                out[(y * s + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The mipmaps of a square RGBA8 image of `side` pixels (a power of two), the image first, each
/// next level the average of 2 x 2 pixels of the one before, down to 1 x 1.
pub fn mipmaps(image: Vec<u8>, side: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![image];
    let mut side = side as usize;
    while side > 1 {
        let half = side / 2;
        let last = levels.last().expect("the image");
        let mut next = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: usize, dy: usize| {
                        u32::from(last[((2 * y + dy) * side + 2 * x + dx) * 4 + c])
                    };
                    next[(y * half + x) * 4 + c] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
        levels.push(next);
        side = half;
    }
    levels
}

/// The scene's textures uploaded as one array, and which images it holds.
pub(crate) struct TextureArray {
    /// Each layer's material and image, to tell when the scene's textures change.
    held: Vec<(MaterialId, Arc<TextureImage>)>,
    pub view: wgpu::TextureView,
}

impl TextureArray {
    /// True when the array holds exactly `scene`'s textures, layer for layer.
    pub fn holds(&self, scene: &Scene) -> bool {
        let now = textured(scene);
        now.len() == self.held.len()
            && now
                .iter()
                .zip(&self.held)
                .all(|((id, t), (held, image))| id == held && Arc::ptr_eq(&t.image, image))
    }

    /// Uploads `scene`'s textures (a single black 1 x 1 layer when there are none, as the
    /// shader always binds an array).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) -> Self {
        let textured = textured(scene);
        let images: Vec<&TextureImage> = textured.iter().map(|(_, t)| &*t.image).collect();
        let side = layer_side(&images);
        let levels = side.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("material textures"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: images.len().max(1) as u32,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Colors stay sRGB values, as the vertex colors and render targets hold them.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, image) in images.iter().enumerate() {
            let pixels = resample(&image.pixels, image.width, image.height, side);
            for (level, data) in mipmaps(pixels, side).iter().enumerate() {
                let size = (side >> level).max(1);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * size),
                        rows_per_image: Some(size),
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        TextureArray {
            held: textured
                .into_iter()
                .map(|(id, t)| (id, t.image.clone()))
                .collect(),
            view,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> TextureImage {
        TextureImage {
            format: kernel::ImageFormat::Png,
            encoded: Vec::new(),
            width,
            height,
            pixels: vec![0; (4 * width * height) as usize],
        }
    }

    #[test]
    fn layers_fit_the_largest_image_and_the_memory_budget() {
        assert_eq!(layer_side(&[]), 1);
        assert_eq!(layer_side(&[&image(3, 2)]), 4);
        assert_eq!(layer_side(&[&image(300, 20), &image(64, 64)]), 512);
        assert_eq!(layer_side(&[&image(4096, 4096)]), MAX_LAYER_SIDE);
        // 256 layers of 1024 would take 1.3 GiB: they shrink to 256.
        let big = image(1024, 1024);
        let many: Vec<&TextureImage> = (0..256).map(|_| &big).collect();
        assert_eq!(layer_side(&many), 256);
    }

    #[test]
    fn resampling_averages_when_shrinking_and_blends_when_growing() {
        // Two columns, black and white, shrink to one gray pixel.
        let pixels = [0, 0, 0, 255, 255, 255, 255, 255];
        assert_eq!(resample(&pixels, 2, 1, 1), [128, 128, 128, 255]);
        // Grown to 4 x 4: each row runs black, blends, white, blends (wrapping round).
        let out = resample(&pixels, 2, 1, 4);
        let reds: Vec<u8> = out.as_chunks::<4>().0[..4].iter().map(|p| p[0]).collect();
        assert_eq!(reds, [64, 64, 191, 191]);
        assert!(out.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // Same size: unchanged.
        let square: Vec<u8> = (0..16 * 4).map(|k| k as u8).collect();
        assert_eq!(resample(&square, 4, 4, 4), square);
    }

    #[test]
    fn mipmaps_halve_down_to_one_pixel() {
        let mut image = vec![0u8; 4 * 4 * 4];
        image[0] = 200; // one red pixel in the corner
        let levels = mipmaps(image, 4);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), [64, 16, 4]);
        assert_eq!(levels[1][0], 50);
        assert_eq!(levels[2][0], 13);
    }
}
```

Apply:

```diff
diff --git a/crates/render/src/lib.rs b/crates/render/src/lib.rs
index b86ea14..d3ccb7a 100644
--- a/crates/render/src/lib.rs
+++ b/crates/render/src/lib.rs
@@ -3,6 +3,7 @@ pub mod guides;
 pub mod renderer;
 pub mod scene;
 pub mod snapshot;
+pub mod texture;
 
 pub use renderer::{Renderer, Viewport};
 pub use snapshot::Snapshotter;
diff --git a/crates/render/src/renderer.rs b/crates/render/src/renderer.rs
index bc242a2..d9249e6 100644
--- a/crates/render/src/renderer.rs
+++ b/crates/render/src/renderer.rs
@@ -7,14 +7,16 @@ use wgpu::util::DeviceExt;
 use crate::grid::{BACKGROUND, LineVertex, flat_ground_lines, vertical_axis_lines};
 use crate::guides::{GuideShape, guide_lines};
 use crate::scene::{BaseFaceVertex, Spans, scene_base, shown_guides};
+use crate::texture::TextureArray;
 
 pub const SAMPLE_COUNT: u32 = 4;
 pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
 
 const LINE_ATTRIBS: [wgpu::VertexAttribute; 2] =
     wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
-const FACE_ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
-    0 => Float32x3, 1 => Float32x3, 3 => Float32x3, 4 => Float32x3, 5 => Float32x2
+const FACE_ATTRIBS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
+    0 => Float32x3, 1 => Float32x3, 3 => Float32x3, 4 => Float32x3, 5 => Float32x2,
+    6 => Float32x4, 7 => Float32x2
 ];
 /// The selection flag of a face or edge vertex, from a second vertex buffer (see
 /// `scene::Flags`).
@@ -193,6 +195,12 @@ pub struct Renderer {
     guide_pipeline: wgpu::RenderPipeline,
     overlay_pipeline: wgpu::RenderPipeline,
     uniforms: wgpu::Buffer,
+    bind_layout: wgpu::BindGroupLayout,
+    /// Repeats both ways, filtering linearly between pixels and mipmaps.
+    sampler: wgpu::Sampler,
+    /// The scene's textures, as of the last `set_model`.
+    textures: TextureArray,
+    /// The camera uniforms, the textures and the sampler.
     bind_group: wgpu::BindGroup,
     flat_ground: Mesh,
     vertical_axis: Mesh,
@@ -220,17 +228,35 @@ impl Renderer {
     pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
         let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/scene.wgsl"));
         let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
-            label: Some("camera"),
-            entries: &[wgpu::BindGroupLayoutEntry {
-                binding: 0,
-                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
-                ty: wgpu::BindingType::Buffer {
-                    ty: wgpu::BufferBindingType::Uniform,
-                    has_dynamic_offset: false,
-                    min_binding_size: None,
+            label: Some("camera and textures"),
+            entries: &[
+                wgpu::BindGroupLayoutEntry {
+                    binding: 0,
+                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
+                    ty: wgpu::BindingType::Buffer {
+                        ty: wgpu::BufferBindingType::Uniform,
+                        has_dynamic_offset: false,
+                        min_binding_size: None,
+                    },
+                    count: None,
+                },
+                wgpu::BindGroupLayoutEntry {
+                    binding: 1,
+                    visibility: wgpu::ShaderStages::FRAGMENT,
+                    ty: wgpu::BindingType::Texture {
+                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
+                        view_dimension: wgpu::TextureViewDimension::D2Array,
+                        multisampled: false,
+                    },
+                    count: None,
+                },
+                wgpu::BindGroupLayoutEntry {
+                    binding: 2,
+                    visibility: wgpu::ShaderStages::FRAGMENT,
+                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
+                    count: None,
                 },
-                count: None,
-            }],
+            ],
         });
         let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
             label: Some("scene"),
@@ -409,14 +435,17 @@ impl Renderer {
             usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
             mapped_at_creation: false,
         });
-        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
-            label: Some("camera"),
-            layout: &bind_layout,
-            entries: &[wgpu::BindGroupEntry {
-                binding: 0,
-                resource: uniforms.as_entire_binding(),
-            }],
+        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
+            label: Some("material textures"),
+            address_mode_u: wgpu::AddressMode::Repeat,
+            address_mode_v: wgpu::AddressMode::Repeat,
+            mag_filter: wgpu::FilterMode::Linear,
+            min_filter: wgpu::FilterMode::Linear,
+            mipmap_filter: wgpu::MipmapFilterMode::Linear,
+            ..Default::default()
         });
+        let textures = TextureArray::new(device, queue, &Scene::default());
+        let bind_group = bind_group(device, &bind_layout, &uniforms, &textures, &sampler);
 
         let flat_ground =
             mesh(device, "flat ground lines", &flat_ground_lines()).expect("grid is never empty");
@@ -435,6 +464,9 @@ impl Renderer {
             guide_pipeline,
             overlay_pipeline,
             uniforms,
+            bind_layout,
+            sampler,
+            textures,
             bind_group,
             flat_ground,
             vertical_axis,
@@ -463,6 +495,16 @@ impl Renderer {
     /// Rebuilds the face and edge buffers from the scene with the open context `context`, with
     /// nothing selected until `set_selection`. Call when the model or the open context changes.
     pub fn set_model(&mut self, scene: &Scene, context: &[InstanceId]) {
+        if !self.textures.holds(scene) {
+            self.textures = TextureArray::new(&self.device, &self.queue, scene);
+            self.bind_group = bind_group(
+                &self.device,
+                &self.bind_layout,
+                &self.uniforms,
+                &self.textures,
+                &self.sampler,
+            );
+        }
         let base = scene_base(scene, context);
         self.faces = flagged_mesh(&self.device, "faces", &base.faces);
         self.glass = Glass::new(&self.device, &base.faces, &base.glass());
@@ -632,6 +674,35 @@ impl Renderer {
     }
 }
 
+/// The bind group every pipeline uses: the camera uniforms, the material textures and their
+/// sampler.
+fn bind_group(
+    device: &wgpu::Device,
+    layout: &wgpu::BindGroupLayout,
+    uniforms: &wgpu::Buffer,
+    textures: &TextureArray,
+    sampler: &wgpu::Sampler,
+) -> wgpu::BindGroup {
+    device.create_bind_group(&wgpu::BindGroupDescriptor {
+        label: Some("camera and textures"),
+        layout,
+        entries: &[
+            wgpu::BindGroupEntry {
+                binding: 0,
+                resource: uniforms.as_entire_binding(),
+            },
+            wgpu::BindGroupEntry {
+                binding: 1,
+                resource: wgpu::BindingResource::TextureView(&textures.view),
+            },
+            wgpu::BindGroupEntry {
+                binding: 2,
+                resource: wgpu::BindingResource::Sampler(sampler),
+            },
+        ],
+    })
+}
+
 /// Uploads vertices, or None when there are none to draw.
 fn mesh<T: bytemuck::Pod>(device: &wgpu::Device, label: &str, vertices: &[T]) -> Option<Mesh> {
     if vertices.is_empty() {
diff --git a/crates/render/src/scene.rs b/crates/render/src/scene.rs
index 0413b50..9b44911 100644
--- a/crates/render/src/scene.rs
+++ b/crates/render/src/scene.rs
@@ -12,13 +12,15 @@ use std::ops::Range;
 use std::sync::atomic::{AtomicUsize, Ordering};
 
 use bytemuck::{Pod, Zeroable};
-use glam::{DAffine3, DVec3};
+use glam::{DAffine3, DVec2, DVec3};
 use kernel::{
     EdgeId, EntityId, FaceId, Guide, GuideId, InstanceId, MaterialId, MeshId, Model, Scene,
+    texture_position,
 };
 
 use crate::grid::LineVertex;
 use crate::guides::{FADED_GUIDE_COLOR, GUIDE_COLOR, GuideShape};
+use crate::texture::layers;
 
 /// A face corner with its selection flag, as `scene_geometry` returns it.
 #[repr(C)]
@@ -33,6 +35,10 @@ pub struct FaceVertex {
     pub back: [f32; 3],
     /// The opacities of the front and back sides (1: opaque).
     pub alpha: [f32; 2],
+    /// The texture coordinates of the front (first two) and back sides (see `BaseFaceVertex`).
+    pub uv: [f32; 4],
+    /// The texture array layers of the front and back sides; -1 for none.
+    pub layer: [f32; 2],
 }
 
 /// A face corner as the renderer uploads it; whether it is selected comes from a separate flag
@@ -48,6 +54,13 @@ pub struct BaseFaceVertex {
     /// The opacities of the front and back sides (1: opaque). A triangle with either below 1
     /// is drawn in the translucent pass (see `SceneBase::glass`).
     pub alpha: [f32; 2],
+    /// The texture coordinates of the front (first two) and back sides: the corner's texture
+    /// position over its side's texture size, v counted down from the image's top, shifted by
+    /// whole repeats per triangle so they stay small. Zero on a side without a texture.
+    pub uv: [f32; 4],
+    /// The texture array layers of the front and back sides (see `texture::layers`); -1 for
+    /// none.
+    pub layer: [f32; 2],
 }
 
 impl BaseFaceVertex {
@@ -62,16 +75,67 @@ pub const DEFAULT_FRONT: [f32; 3] = [0.93, 0.93, 0.90];
 /// The default material's color on a face's back.
 pub const DEFAULT_BACK: [f32; 3] = [0.60, 0.65, 0.76];
 
-/// How a side looks: its color (sRGB, as the targets store it) and opacity.
-type Look = ([f32; 3], f32);
+/// How a side looks: its color (sRGB, as the targets store it), opacity, and texture (its
+/// array layer and its size in meters), which shows instead of the color.
+#[derive(Clone, Copy, Debug, PartialEq)]
+struct Look {
+    color: [f32; 3],
+    alpha: f32,
+    texture: Option<(u32, DVec2)>,
+}
+
+/// The array layer of each textured material (see `texture::layers`).
+type Layers = HashMap<MaterialId, u32>;
 
 /// How a side looks: as its material, or opaque in `default`.
-fn side_look(scene: &Scene, material: Option<MaterialId>, default: [f32; 3]) -> Look {
-    material
-        .and_then(|m| scene.material(m))
-        .map_or((default, 1.0), |m| {
-            (m.color.map(|c| f32::from(c) / 255.0), m.opacity)
-        })
+fn side_look(
+    scene: &Scene,
+    layers: &Layers,
+    material: Option<MaterialId>,
+    default: [f32; 3],
+) -> Look {
+    let plain = Look {
+        color: default,
+        alpha: 1.0,
+        texture: None,
+    };
+    let Some((id, m)) = material.and_then(|id| Some((id, scene.material(id)?))) else {
+        return plain;
+    };
+    Look {
+        color: m.color.map(|c| f32::from(c) / 255.0),
+        alpha: m.opacity,
+        texture: m
+            .texture
+            .as_ref()
+            .zip(layers.get(&id))
+            .map(|(t, &layer)| (layer, DVec2::new(t.width, t.height))),
+    }
+}
+
+impl Look {
+    /// The texture array layer, or -1 without a texture.
+    fn layer(&self) -> f32 {
+        self.texture.map_or(-1.0, |(layer, _)| layer as f32)
+    }
+
+    /// A triangle's texture coordinates on this side from its corners' texture positions
+    /// (meters): over the texture's size, v counted down from the image's top, shifted by
+    /// whole repeats so f32 keeps them exact. Zero without a texture.
+    fn uv(&self, positions: [DVec2; 3]) -> [[f32; 2]; 3] {
+        let Some((_, size)) = self.texture else {
+            return [[0.0; 2]; 3];
+        };
+        let st = positions.map(|p| DVec2::new(p.x / size.x, -p.y / size.y));
+        let shift = st[0].floor();
+        st.map(|q| (q - shift).as_vec2().to_array())
+    }
+}
+
+/// The corners of a triangle with both sides' texture coordinates (see `BaseFaceVertex::uv`).
+fn both_uv(front: &Look, back: &Look, positions: [DVec2; 3]) -> [[f32; 4]; 3] {
+    let (f, b) = (front.uv(positions), back.uv(positions));
+    [0, 1, 2].map(|k| [f[k][0], f[k][1], b[k][0], b[k][1]])
 }
 
 pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
@@ -149,6 +213,8 @@ impl SceneBase {
                 front: v.front,
                 back: v.back,
                 alpha: v.alpha,
+                uv: v.uv,
+                layer: v.layer,
             })
             .collect();
         let edges = self
@@ -267,11 +333,13 @@ fn find<K: Ord + Copy, V>(list: &[(K, V)], id: K) -> Option<&V> {
 /// The scene in root coordinates with the open instance `context` (see `scene_geometry`), with
 /// nothing selected. Each mesh is triangulated once, however often it is placed.
 pub fn scene_base(scene: &Scene, context: &[InstanceId]) -> SceneBase {
-    let parts = placed_parts(scene);
+    let layers = layers(scene);
+    let parts = placed_parts(scene, &layers);
     let (faces, edges) = drawn_count(scene, &parts, MeshId::Root, &mut HashMap::new());
     let mut walk = Walk {
         scene,
         context,
+        layers: &layers,
         parts: &parts,
         path: Vec::new(),
         base: SceneBase {
@@ -338,6 +406,8 @@ struct MeshParts {
     corners: Vec<DVec3>,
     /// The normal at each corner: the face's own, or across soft edges the smooth one.
     normals: Vec<DVec3>,
+    /// Each corner's position on its face's texture, in meters (see `texture_position`).
+    positions: Vec<DVec2>,
     faces: Vec<FacePart>,
     /// The ends of the hard edges (soft edges are not drawn).
     ends: Vec<DVec3>,
@@ -346,7 +416,7 @@ struct MeshParts {
 
 struct FacePart {
     id: FaceId,
-    /// The face's triangle corners in `MeshParts::corners` and `MeshParts::normals`.
+    /// The face's triangle corners in `MeshParts::corners`, `normals` and `positions`.
     corners: Range<usize>,
     /// The looks of the sides painted on the face itself; unpainted sides take the look of
     /// the nearest painted instance placing them.
@@ -355,7 +425,7 @@ struct FacePart {
 }
 
 impl MeshParts {
-    fn of(scene: &Scene, model: &Model) -> Self {
+    fn of(scene: &Scene, layers: &Layers, model: &Model) -> Self {
         // A face of n loop corners and h holes has n + 2h - 2 triangles.
         let (mut face_count, mut triangles) = (0, 0);
         for (_, f) in model.faces() {
@@ -365,6 +435,7 @@ impl MeshParts {
         }
         let mut corners = Vec::with_capacity(3 * triangles);
         let mut normals = Vec::with_capacity(3 * triangles);
+        let mut positions = Vec::with_capacity(3 * triangles);
         let mut faces = Vec::with_capacity(face_count);
         let smooth = model.smooth_normals();
         for (id, face) in model.faces() {
@@ -372,12 +443,17 @@ impl MeshParts {
             for tri in model.shaded_triangles(id, &smooth).unwrap_or_default() {
                 corners.extend(tri.corners);
                 normals.extend(tri.normals);
+                positions.extend(tri.corners.map(|p| texture_position(face.normal, p)));
             }
             faces.push(FacePart {
                 id,
                 corners: start..corners.len(),
-                front: face.front.map(|m| side_look(scene, Some(m), DEFAULT_FRONT)),
-                back: face.back.map(|m| side_look(scene, Some(m), DEFAULT_BACK)),
+                front: face
+                    .front
+                    .map(|m| side_look(scene, layers, Some(m), DEFAULT_FRONT)),
+                back: face
+                    .back
+                    .map(|m| side_look(scene, layers, Some(m), DEFAULT_BACK)),
             });
         }
         let hard = model.edges().filter(|(_, e)| !e.soft).count();
@@ -392,6 +468,7 @@ impl MeshParts {
         MeshParts {
             corners,
             normals,
+            positions,
             faces,
             ends,
             edges,
@@ -407,7 +484,7 @@ const MAX_THREADS: usize = 8;
 
 /// The parts of every mesh placed from the root, the root included. With many faces, the meshes
 /// are shared out among a few threads; the parts are the same either way.
-fn placed_parts(scene: &Scene) -> HashMap<MeshId, MeshParts> {
+fn placed_parts(scene: &Scene, layers: &Layers) -> HashMap<MeshId, MeshParts> {
     let mut meshes = vec![MeshId::Root];
     let mut seen = HashSet::from([MeshId::Root]);
     let mut k = 0;
@@ -434,7 +511,7 @@ fn placed_parts(scene: &Scene) -> HashMap<MeshId, MeshParts> {
     if faces < PARALLEL_FACES || threads < 2 {
         return models
             .into_iter()
-            .map(|(id, m)| (id, MeshParts::of(scene, m)))
+            .map(|(id, m)| (id, MeshParts::of(scene, layers, m)))
             .collect();
     }
     // Each thread takes the next mesh nobody has taken, so big meshes do not hold one up.
@@ -442,7 +519,7 @@ fn placed_parts(scene: &Scene) -> HashMap<MeshId, MeshParts> {
     let work = || {
         let mut done = Vec::new();
         while let Some(&(id, m)) = models.get(next.fetch_add(1, Ordering::Relaxed)) {
-            done.push((id, MeshParts::of(scene, m)));
+            done.push((id, MeshParts::of(scene, layers, m)));
         }
         done
     };
@@ -486,6 +563,7 @@ fn drawn_count(
 struct Walk<'a> {
     scene: &'a Scene,
     context: &'a [InstanceId],
+    layers: &'a Layers,
     /// Every placed mesh's parts.
     parts: &'a HashMap<MeshId, MeshParts>,
     /// Instances from the root down to the mesh being walked.
@@ -510,27 +588,32 @@ impl Walk<'_> {
         // A mirroring placement turns triangles inside out; swapping two corners restores the
         // winding, so the front side (and the headlight) stay right.
         let mirrored = t.matrix3.determinant() < 0.0;
-        let front = side_look(self.scene, inherited, DEFAULT_FRONT);
-        let back = side_look(self.scene, inherited, DEFAULT_BACK);
+        let front = side_look(self.scene, self.layers, inherited, DEFAULT_FRONT);
+        let back = side_look(self.scene, self.layers, inherited, DEFAULT_BACK);
         let base = &mut self.base;
         for face in &parts.faces {
             let start = base.faces.len();
-            let (front, front_alpha) = face.front.unwrap_or(front);
-            let (back, back_alpha) = face.back.unwrap_or(back);
-            let corner = |(p, n): (DVec3, DVec3)| BaseFaceVertex {
+            let front = face.front.unwrap_or(front);
+            let back = face.back.unwrap_or(back);
+            let corner = |(p, n, uv): (DVec3, DVec3, [f32; 4])| BaseFaceVertex {
                 pos: t.transform_point3(p).as_vec3().to_array(),
                 normal: (normal_map * n).normalize().as_vec3().to_array(),
-                front,
-                back,
-                alpha: [front_alpha, back_alpha],
+                front: front.color,
+                back: back.color,
+                alpha: [front.alpha, back.alpha],
+                uv,
+                layer: [front.layer(), back.layer()],
             };
-            let (tris, _) = parts.corners[face.corners.clone()].as_chunks::<3>();
-            let (normals, _) = parts.normals[face.corners.clone()].as_chunks::<3>();
-            for (&[a, b, c], &[na, nb, nc]) in tris.iter().zip(normals) {
+            let range = face.corners.clone();
+            let (tris, _) = parts.corners[range.clone()].as_chunks::<3>();
+            let (normals, _) = parts.normals[range.clone()].as_chunks::<3>();
+            let (positions, _) = parts.positions[range].as_chunks::<3>();
+            for ((&[a, b, c], &[na, nb, nc]), &uvs) in tris.iter().zip(normals).zip(positions) {
+                let [ua, ub, uc] = both_uv(&front, &back, uvs);
                 let tri = if mirrored {
-                    [(a, na), (c, nc), (b, nb)]
+                    [(a, na, ua), (c, nc, uc), (b, nb, ub)]
                 } else {
-                    [(a, na), (b, nb), (c, nc)]
+                    [(a, na, ua), (b, nb, ub), (c, nc, uc)]
                 };
                 base.faces.extend(tri.map(corner));
             }
@@ -988,6 +1071,70 @@ mod tests {
         assert_eq!(count(CONTEXT_BOX_COLOR), 24, "twelve box edges");
     }
 
+    fn texture(width: f64, height: f64) -> kernel::Texture {
+        kernel::Texture {
+            image: std::sync::Arc::new(kernel::TextureImage {
+                format: kernel::ImageFormat::Png,
+                encoded: Vec::new(),
+                width: 1,
+                height: 1,
+                pixels: vec![255; 4],
+            }),
+            width,
+            height,
+        }
+    }
+
+    #[test]
+    fn textured_sides_name_their_layer_and_carry_texture_coordinates() {
+        // The unit square moved far out, so its texture coordinates would be large unshifted.
+        let mut m = Model::new();
+        let pts = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
+            .map(|(x, y)| DVec3::new(x + 1000.0, y + 2000.0, 0.0));
+        m.add_polygon(&pts).unwrap();
+        let mut s = Scene::from_root(m);
+        let face = s.root().faces().next().unwrap().0;
+        let (plain, _) = s.add_material("Plain", [10, 20, 30]).unwrap();
+        let (tiles, _) = s
+            .add_material_with_texture("Tiles", [1, 2, 3], 1.0, Some(texture(2.0, 0.5)))
+            .unwrap();
+        let (wood, _) = s
+            .add_material_with_texture("Wood", [4, 5, 6], 1.0, Some(texture(1.0, 1.0)))
+            .unwrap();
+        assert_eq!(
+            crate::texture::layers(&s),
+            HashMap::from([(tiles, 0), (wood, 1)])
+        );
+        s.paint(MeshId::Root, &[face], &[], kernel::Side::Front, Some(tiles))
+            .unwrap();
+        s.paint(MeshId::Root, &[face], &[], kernel::Side::Back, Some(plain))
+            .unwrap();
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        assert_eq!(g.faces.len(), 6);
+        for v in &g.faces {
+            assert_eq!(v.layer, [0.0, -1.0]);
+            assert_eq!(v.front, [1.0, 2.0, 3.0].map(|c| c / 255.0));
+            assert_eq!(&v.uv[2..], [0.0, 0.0]);
+            // u = x / 2 and v = -y / 0.5, up to whole repeats, and small.
+            let (du, dv) = (
+                f64::from(v.uv[0]) - f64::from(v.pos[0]) / 2.0,
+                f64::from(v.uv[1]) + f64::from(v.pos[1]) / 0.5,
+            );
+            assert!((du - du.round()).abs() < 1e-4 && (dv - dv.round()).abs() < 1e-4);
+            assert!(v.uv[0].abs() <= 3.0 && v.uv[1].abs() <= 3.0, "{:?}", v.uv);
+        }
+        s.paint(MeshId::Root, &[face], &[], kernel::Side::Back, Some(wood))
+            .unwrap();
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        assert!(g.faces.iter().all(|v| v.layer == [0.0, 1.0]));
+        // Removing the first texture moves the second to layer 0.
+        s.change_material_with_texture(tiles, None, None, None, Some(None))
+            .unwrap();
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        assert!(g.faces.iter().all(|v| v.layer == [-1.0, 0.0]));
+        assert_eq!(g, single_pass::scene_geometry(&s, &[], &BTreeSet::new()));
+    }
+
     /// The single pass `scene_geometry` made before the base and the selection flags were
     /// split, kept to check that the split changes nothing.
     mod single_pass {
@@ -999,9 +1146,11 @@ mod tests {
             selection: &BTreeSet<EntityId>,
         ) -> ModelGeometry {
             let mut g = ModelGeometry::default();
+            let layers = crate::texture::layers(scene);
             let mut walk = Walk {
                 scene,
                 context,
+                layers: &layers,
                 selection,
                 path: Vec::new(),
                 g: &mut g,
@@ -1034,6 +1183,7 @@ mod tests {
         struct Walk<'a> {
             scene: &'a Scene,
             context: &'a [InstanceId],
+            layers: &'a Layers,
             selection: &'a BTreeSet<EntityId>,
             path: Vec<InstanceId>,
             g: &'a mut ModelGeometry,
@@ -1056,23 +1206,36 @@ mod tests {
                     } else {
                         0.0
                     };
-                    let (front, front_alpha) =
-                        side_look(self.scene, face.front.or(inherited), DEFAULT_FRONT);
-                    let (back, back_alpha) =
-                        side_look(self.scene, face.back.or(inherited), DEFAULT_BACK);
+                    let front = side_look(
+                        self.scene,
+                        self.layers,
+                        face.front.or(inherited),
+                        DEFAULT_FRONT,
+                    );
+                    let back = side_look(
+                        self.scene,
+                        self.layers,
+                        face.back.or(inherited),
+                        DEFAULT_BACK,
+                    );
                     for tri in model.shaded_triangles(fid, &smooth).unwrap_or_default() {
-                        let mut corners = [0, 1, 2].map(|k| (tri.corners[k], tri.normals[k]));
+                        let positions = tri.corners.map(|p| texture_position(face.normal, p));
+                        let uv = both_uv(&front, &back, positions);
+                        let mut corners =
+                            [0, 1, 2].map(|k| (tri.corners[k], tri.normals[k], uv[k]));
                         if mirrored {
                             corners.swap(1, 2);
                         }
-                        for (p, n) in corners {
+                        for (p, n, uv) in corners {
                             self.g.faces.push(FaceVertex {
                                 pos: t.transform_point3(p).as_vec3().to_array(),
                                 normal: (normal_map * n).normalize().as_vec3().to_array(),
                                 selected: flag,
-                                front,
-                                back,
-                                alpha: [front_alpha, back_alpha],
+                                front: front.color,
+                                back: back.color,
+                                alpha: [front.alpha, back.alpha],
+                                uv,
+                                layer: [front.layer(), back.layer()],
                             });
                         }
                     }
diff --git a/crates/render/src/shaders/scene.wgsl b/crates/render/src/shaders/scene.wgsl
index b942c99..b360272 100644
--- a/crates/render/src/shaders/scene.wgsl
+++ b/crates/render/src/shaders/scene.wgsl
@@ -7,6 +7,9 @@ struct Uniforms {
 };
 
 @group(0) @binding(0) var<uniform> u: Uniforms;
+// One layer per textured material (see texture.rs); it repeats both ways.
+@group(0) @binding(1) var textures: texture_2d_array<f32>;
+@group(0) @binding(2) var texture_sampler: sampler;
 
 struct LineOut {
     @builtin(position) pos: vec4<f32>,
@@ -75,6 +78,10 @@ struct FaceOut {
     @location(4) back_color: vec3<f32>,
     // The opacities of the front (x) and back (y) sides.
     @location(5) alpha: vec2<f32>,
+    // The texture coordinates of the front (xy) and back (zw) sides.
+    @location(6) uv: vec4<f32>,
+    // The texture layers of the front (x) and back (y) sides; -1 for none.
+    @location(7) @interpolate(flat) layer: vec2<f32>,
 };
 
 const SELECTED: vec3<f32> = vec3<f32>(0.25, 0.45, 0.95);
@@ -88,6 +95,8 @@ fn vs_face(
     @location(3) front_color: vec3<f32>,
     @location(4) back_color: vec3<f32>,
     @location(5) alpha: vec2<f32>,
+    @location(6) uv: vec4<f32>,
+    @location(7) layer: vec2<f32>,
 ) -> FaceOut {
     var out: FaceOut;
     out.pos = u.view_proj * vec4<f32>(pos, 1.0);
@@ -97,17 +106,24 @@ fn vs_face(
     out.front_color = front_color;
     out.back_color = back_color;
     out.alpha = alpha;
+    out.uv = uv;
+    out.layer = layer;
     return out;
 }
 
 // Each side shows its own color (its material's, or the default: light in front, blue-gray
-// behind); a headlight at the eye (from the viewer, in a parallel view) shades both. Selected
-// faces are tinted blue.
+// behind), or its material's texture instead; a headlight at the eye (from the viewer, in a
+// parallel view) shades both. Selected faces are tinted blue. Call it before any discard: the
+// texture lookup needs every pixel of its quad.
 fn face_color(in: FaceOut, front: bool) -> vec3<f32> {
     let n = normalize(in.normal) * select(-1.0, 1.0, front);
     let to_eye = normalize(u.eye.xyz - in.world * u.eye.w);
     let shade = 0.6 + 0.4 * max(dot(n, to_eye), 0.0);
-    let base = mix(select(in.back_color, in.front_color, front), SELECTED, 0.5 * in.selected);
+    let layer = select(in.layer.y, in.layer.x, front);
+    let uv = select(in.uv.zw, in.uv.xy, front);
+    let texel = textureSample(textures, texture_sampler, uv, i32(max(layer, 0.0))).rgb;
+    let color = select(select(in.back_color, in.front_color, front), texel, layer >= 0.0);
+    let base = mix(color, SELECTED, 0.5 * in.selected);
     return base * shade;
 }
 
@@ -121,19 +137,21 @@ fn fs_face(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4
 // see-through side left to `fs_glass`.
 @fragment
 fn fs_face_solid_side(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
+    let color = face_color(in, front);
     if select(in.alpha.y, in.alpha.x, front) < 1.0 {
         discard;
     }
-    return vec4<f32>(face_color(in, front), 1.0);
+    return vec4<f32>(color, 1.0);
 }
 
 // See-through sides, blended over everything opaque (back to front, without writing depth).
 // A selected one is made more opaque so its tint shows even when it is nearly invisible.
 @fragment
 fn fs_glass(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
+    let color = face_color(in, front);
     let alpha = select(in.alpha.y, in.alpha.x, front);
     if alpha >= 1.0 {
         discard;
     }
-    return vec4<f32>(face_color(in, front), mix(alpha, 1.0, 0.4 * in.selected));
+    return vec4<f32>(color, mix(alpha, 1.0, 0.4 * in.selected));
 }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p render` (unit tests including `texture::tests` and `textured_sides_name_their_layer_and_carry_texture_coordinates`, `headless`, `golden` with 11 images: all pass).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/render/src/texture.rs crates/render/src/scene.rs crates/render/src/renderer.rs crates/render/src/shaders/scene.wgsl crates/render/src/lib.rs crates/render/tests/golden.rs crates/render/tests/golden/textured.png
git commit -m "Render: texture array with mipmaps; textured golden image"
```

---

### Task 5: Materials list: texture swatch, Texture..., Remove and size fields

**Files:**
- Modify: `crates/app/src/ui.rs`, `crates/app/src/app.rs`, `crates/app/src/gui.rs`, `crates/app/src/editor.rs`, `docs/agent-api.md`
- Test: unit tests in `crates/app/src/editor.rs` and `crates/app/src/ui.rs`

**Interfaces:**
- Consumes: `Command::ChangeMaterial { texture }`, `TextureSource` (Task 2); `render::texture::resample` (Task 4); `kernel::{MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE, TextureImage}`.
- Produces:
  - `ui::TextureRow { swatch: egui::TextureId, width: f64, height: f64, pixels: [u32; 2] }`, `MaterialRow.texture: Option<TextureRow>`
  - `UiAction::{LoadTexture(MaterialId), RemoveTexture(MaterialId), SetTextureSize(MaterialId, [u32; 2] /* mm */)}`
  - `Editor::{load_texture(&mut self, MaterialId, &Path) -> bool, remove_texture(&mut self, MaterialId) -> bool, set_texture_size(&mut self, MaterialId, [u32; 2]) -> bool}`
  - `Gui::ctx(&self) -> &egui::Context`

- [ ] **Step 1: Write the failing tests**

The tests sit next to the code they test, in `editor.rs` (`textures_load_resize_and_come_off_through_commands`: writes a 71-byte 2 x 1 PNG to the temp directory, loads, resizes, undoes, removes, and refuses a text file) and `ui.rs` (`the_materials_list_loads_removes_and_sizes_textures`: clicks down the panel and types into the size fields). They are part of the Step 3 diff; add the two test functions first.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p app --bin app textures`
Expected: compile errors (no `Editor::load_texture`, `TextureRow`, `UiAction::LoadTexture`).

- [ ] **Step 3: Implement**

Apply:

```diff
diff --git a/crates/app/src/app.rs b/crates/app/src/app.rs
index 580f0b4..8e46940 100644
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -1,11 +1,11 @@
-use std::collections::BTreeSet;
+use std::collections::{BTreeSet, HashMap};
 use std::path::{Path, PathBuf};
 use std::sync::Arc;
 use std::time::{Duration, Instant};
 
 use api::{Hub, listen_unix};
 use glam::DVec2;
-use kernel::{EntityId, InstanceId};
+use kernel::{EntityId, InstanceId, MaterialId, TextureImage};
 use render::{Renderer, Snapshotter, Viewport};
 use session::{Command, OutlinerTable};
 use tools::Axis;
@@ -25,7 +25,10 @@ use crate::gui::Gui;
 use crate::host::WindowHost;
 use crate::info::entity_info;
 use crate::navigation::{Button, Scroll};
-use crate::ui::{MaterialRow, UiAction, View};
+use crate::ui::{MaterialRow, TextureRow, UiAction, View};
+
+/// The side of a texture's swatch image in the Materials list, in pixels.
+const SWATCH_PIXELS: u32 = 32;
 
 /// Sent to the event loop when an agent API request arrives.
 #[derive(Debug)]
@@ -299,6 +302,8 @@ struct State {
     drawn: Option<Drawn>,
     /// The Outliner's rows and the revision they were built at.
     outliner: Option<(u64, OutlinerTable)>,
+    /// Each textured material's swatch in the Materials list, with the image it shows.
+    swatches: HashMap<MaterialId, (Arc<TextureImage>, egui::TextureHandle)>,
     modifiers: ModifiersState,
     /// Watches for a Ctrl tap, which switches copying (see `CtrlTap`).
     ctrl_tap: CtrlTap,
@@ -381,6 +386,7 @@ impl State {
             gui,
             drawn: None,
             outliner: None,
+            swatches: HashMap::new(),
             modifiers: ModifiersState::empty(),
             ctrl_tap: CtrlTap::default(),
             view,
@@ -514,6 +520,65 @@ impl State {
         self.editor.save_file(path.as_deref())
     }
 
+    /// Gives a material the PNG or JPEG image chosen in a dialog as its texture.
+    fn load_texture(&mut self, material: MaterialId) {
+        let chosen = rfd::FileDialog::new()
+            .add_filter("PNG or JPEG image", &["png", "jpg", "jpeg"])
+            .pick_file();
+        if let Some(path) = chosen {
+            self.editor.load_texture(material, &path);
+        }
+    }
+
+    /// The Materials list rows, with a swatch image for each texture (made when a material's
+    /// image changes; those of materials gone are dropped).
+    fn material_rows(&mut self) -> Vec<MaterialRow> {
+        let materials: Vec<(MaterialId, kernel::Material)> = self
+            .editor
+            .doc()
+            .scene()
+            .materials()
+            .map(|(id, m)| (id, m.clone()))
+            .collect();
+        self.swatches
+            .retain(|id, _| materials.iter().any(|(m, _)| m == id));
+        materials
+            .into_iter()
+            .map(|(id, m)| MaterialRow {
+                id,
+                texture: m.texture.as_ref().map(|t| TextureRow {
+                    swatch: self.swatch(id, &t.image),
+                    width: t.width,
+                    height: t.height,
+                    pixels: [t.image.width, t.image.height],
+                }),
+                name: m.name,
+                color: m.color,
+                opacity: m.opacity,
+            })
+            .collect()
+    }
+
+    /// The swatch image of material `id`'s texture `image`.
+    fn swatch(&mut self, id: MaterialId, image: &Arc<TextureImage>) -> egui::TextureId {
+        if let Some((held, handle)) = self.swatches.get(&id)
+            && Arc::ptr_eq(held, image)
+        {
+            return handle.id();
+        }
+        let pixels =
+            render::texture::resample(&image.pixels, image.width, image.height, SWATCH_PIXELS);
+        let side = SWATCH_PIXELS as usize;
+        let handle = self.gui.ctx().load_texture(
+            format!("texture-swatch-{}", id.0),
+            egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels),
+            egui::TextureOptions::LINEAR,
+        );
+        let shown = handle.id();
+        self.swatches.insert(id, (image.clone(), handle));
+        shown
+    }
+
     /// Imports an OBJ or STL file chosen in a dialog as a group in the open context.
     fn import(&mut self) {
         let chosen = rfd::FileDialog::new()
@@ -609,6 +674,13 @@ impl State {
             UiAction::SetMaterialOpacity(material, percent) => {
                 self.editor.set_material_opacity(material, percent);
             }
+            UiAction::LoadTexture(material) => self.load_texture(material),
+            UiAction::RemoveTexture(material) => {
+                self.editor.remove_texture(material);
+            }
+            UiAction::SetTextureSize(material, millimeters) => {
+                self.editor.set_texture_size(material, millimeters);
+            }
             UiAction::OutlinerSelect { path, toggle } => {
                 self.editor.outliner_select(&path, toggle);
             }
@@ -682,18 +754,7 @@ impl State {
         }
         let status = self.editor.status_parts();
         let info = entity_info(self.editor.doc());
-        let materials: Vec<MaterialRow> = self
-            .editor
-            .doc()
-            .scene()
-            .materials()
-            .map(|(id, m)| MaterialRow {
-                id,
-                name: m.name.clone(),
-                color: m.color,
-                opacity: m.opacity,
-            })
-            .collect();
+        let materials = self.material_rows();
         // Rebuilt only when the model changes; the open and selected marks are worked out
         // while drawing, from the context and selection.
         let revision = self.editor.doc().revision();
diff --git a/crates/app/src/editor.rs b/crates/app/src/editor.rs
index 52d71c6..b96f081 100644
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -11,7 +11,7 @@ use kernel::{EntityId, InstanceId, MaterialId};
 use render::grid::LineVertex;
 use session::{
     BatchError, Camera, Command, CommandError, Document, ErrorKind, ImportAs, Outcome, Projection,
-    SelectMode, StandardView,
+    SelectMode, StandardView, TextureSource,
 };
 use tools::overlay::overlay;
 use tools::{
@@ -419,6 +419,55 @@ impl Editor {
         true
     }
 
+    /// Gives a material the image at `path` as its texture (keeping the size of the texture it
+    /// replaces, if any). A file that cannot be used says why in the status.
+    pub fn load_texture(&mut self, material: MaterialId, path: &Path) -> bool {
+        self.message = None;
+        self.run(Command::ChangeMaterial {
+            material,
+            name: None,
+            color: None,
+            opacity: None,
+            texture: Some(Some(TextureSource {
+                path: Some(path.display().to_string()),
+                width: None,
+                height: None,
+            })),
+        });
+        true
+    }
+
+    /// Takes a material's texture off; its color stays.
+    pub fn remove_texture(&mut self, material: MaterialId) -> bool {
+        self.message = None;
+        self.run(Command::ChangeMaterial {
+            material,
+            name: None,
+            color: None,
+            opacity: None,
+            texture: Some(None),
+        });
+        true
+    }
+
+    /// Sets the width and height a material's texture covers, in millimeters.
+    pub fn set_texture_size(&mut self, material: MaterialId, millimeters: [u32; 2]) -> bool {
+        self.message = None;
+        let [width, height] = millimeters.map(|mm| f64::from(mm) / 1000.0);
+        self.run(Command::ChangeMaterial {
+            material,
+            name: None,
+            color: None,
+            opacity: None,
+            texture: Some(Some(TextureSource {
+                path: None,
+                width: Some(width),
+                height: Some(height),
+            })),
+        });
+        true
+    }
+
     /// Deletes a material; what was painted with it goes back to the default material.
     pub fn delete_material(&mut self, material: MaterialId) -> bool {
         self.message = None;
@@ -3566,6 +3615,68 @@ mod tests {
         assert_eq!(e.current_material(), None);
     }
 
+    /// A 2 x 1 pixel PNG: a red pixel, then a blue one.
+    const RED_BLUE_PNG: [u8; 71] = [
+        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
+        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 14, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0,
+        66, 255, 1, 15, 249, 3, 253, 133, 17, 153, 118, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
+        130,
+    ];
+
+    #[test]
+    fn textures_load_resize_and_come_off_through_commands() {
+        let mut e = editor();
+        e.add_material();
+        let id = e.current_material().unwrap();
+        let png = std::env::temp_dir().join(format!(
+            "sketchup-alt-editor-{}-red-blue.png",
+            std::process::id()
+        ));
+        std::fs::write(&png, RED_BLUE_PNG).unwrap();
+        e.load_texture(id, &png);
+        let m = e.doc().scene().material(id).unwrap();
+        let t = m.texture.as_ref().expect("a texture");
+        assert_eq!((t.width, t.height), (1.0, 0.5));
+        assert_eq!((t.image.width, t.image.height), (2, 1));
+        assert_eq!(m.color, [128, 0, 128], "the image's average");
+
+        e.set_texture_size(id, [2000, 250]);
+        let t = e
+            .doc()
+            .scene()
+            .material(id)
+            .unwrap()
+            .texture
+            .clone()
+            .unwrap();
+        assert_eq!((t.width, t.height), (2.0, 0.25));
+        e.undo();
+        let t = e
+            .doc()
+            .scene()
+            .material(id)
+            .unwrap()
+            .texture
+            .clone()
+            .unwrap();
+        assert_eq!((t.width, t.height), (1.0, 0.5));
+
+        e.remove_texture(id);
+        assert!(e.doc().scene().material(id).unwrap().texture.is_none());
+        e.undo();
+        assert!(e.doc().scene().material(id).unwrap().texture.is_some());
+
+        // A file that is not an image leaves the material and says why.
+        let text = png.with_extension("txt");
+        std::fs::write(&text, "hello").unwrap();
+        let revision = e.doc().revision();
+        e.load_texture(id, &text);
+        assert_eq!(e.doc().revision(), revision);
+        assert!(e.status().contains("not a PNG or JPEG"), "{}", e.status());
+        let _ = std::fs::remove_file(png);
+        let _ = std::fs::remove_file(text);
+    }
+
     #[test]
     fn clicking_a_selected_face_paints_the_whole_selection() {
         let mut e = editor();
diff --git a/crates/app/src/gui.rs b/crates/app/src/gui.rs
index db2a8da..664b785 100644
--- a/crates/app/src/gui.rs
+++ b/crates/app/src/gui.rs
@@ -46,6 +46,11 @@ impl Gui {
         }
     }
 
+    /// The egui context, for loading images the panels show.
+    pub fn ctx(&self) -> &egui::Context {
+        &self.ctx
+    }
+
     /// Feeds a window event to egui. Returns true if egui needs a repaint.
     pub fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
         self.state.on_window_event(window, event).repaint
diff --git a/crates/app/src/ui.rs b/crates/app/src/ui.rs
index 14aa4d4..7acd1d5 100644
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -42,6 +42,12 @@ pub enum UiAction {
     RecolorMaterial(MaterialId, [u8; 3]),
     /// Set a material's opacity, in percent (0 to 100).
     SetMaterialOpacity(MaterialId, u8),
+    /// Choose an image file for a material's texture.
+    LoadTexture(MaterialId),
+    /// Take a material's texture off.
+    RemoveTexture(MaterialId),
+    /// Set the width and height a material's texture covers, in millimeters.
+    SetTextureSize(MaterialId, [u32; 2]),
     /// Select the outliner node at `path` (instances from the root down to it); `toggle`
     /// (Shift, Cmd or Ctrl held) adds or removes it instead of replacing the selection.
     OutlinerSelect {
@@ -72,6 +78,19 @@ pub struct MaterialRow {
     pub color: [u8; 3],
     /// From 0 (invisible) to 1 (opaque).
     pub opacity: f32,
+    pub texture: Option<TextureRow>,
+}
+
+/// A material's texture as the Materials list shows it.
+#[derive(Clone, Debug, PartialEq)]
+pub struct TextureRow {
+    /// A small copy of the image for its swatch.
+    pub swatch: egui::TextureId,
+    /// The size one copy covers on the model, in meters.
+    pub width: f64,
+    pub height: f64,
+    /// The image's size in pixels.
+    pub pixels: [u32; 2],
 }
 
 /// The default material's swatch: its front color.
@@ -126,9 +145,9 @@ const OUTLINER_MIN_HEIGHT: f32 = 120.0;
 /// The Materials list's greatest height, in points.
 const MATERIALS_MAX_HEIGHT: f32 = 220.0;
 
-/// Room kept below the Materials list for its buttons, name field and colors, and for the
-/// Outliner's heading and minimum height, in points.
-const BELOW_MATERIALS_LIST: f32 = 166.0 + OUTLINER_MIN_HEIGHT;
+/// Room kept below the Materials list for its buttons, name field, colors, opacity and texture
+/// controls, and for the Outliner's heading and minimum height, in points.
+const BELOW_MATERIALS_LIST: f32 = 222.0 + OUTLINER_MIN_HEIGHT;
 
 /// The least height the right panel lays out its contents in; a shorter panel scrolls them.
 const PANEL_MIN_HEIGHT: f32 = 480.0;
@@ -375,6 +394,14 @@ fn swatch(ui: &mut Ui, color: Color32) -> egui::Response {
     response
 }
 
+/// A small square showing `image` that can be clicked.
+fn image_swatch(ui: &mut Ui, image: egui::TextureId) -> egui::Response {
+    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
+    let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
+    ui.painter().image(image, rect, whole, Color32::WHITE);
+    response
+}
+
 fn rgb(c: [u8; 3]) -> Color32 {
     Color32::from_rgb(c[0], c[1], c[2])
 }
@@ -391,14 +418,19 @@ fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
     egui::ScrollArea::vertical()
         .max_height(max_height)
         .show(ui, |ui| {
-            let rows = std::iter::once((None, "Default", DEFAULT_SWATCH)).chain(
-                view.materials
-                    .iter()
-                    .map(|m| (Some(m.id), m.name.as_str(), rgb(m.color))),
+            let rows = std::iter::once((None, "Default", DEFAULT_SWATCH, None)).chain(
+                view.materials.iter().map(|m| {
+                    let image = m.texture.as_ref().map(|t| t.swatch);
+                    (Some(m.id), m.name.as_str(), rgb(m.color), image)
+                }),
             );
-            for (id, name, color) in rows {
+            for (id, name, color, image) in rows {
                 ui.horizontal(|ui| {
-                    let picked = swatch(ui, color).clicked();
+                    let picked = match image {
+                        Some(image) => image_swatch(ui, image),
+                        None => swatch(ui, color),
+                    }
+                    .clicked();
                     let label = ui.selectable_label(view.material == id, name);
                     if picked || label.clicked() {
                         actions.push(UiAction::ChooseMaterial(id));
@@ -471,6 +503,61 @@ fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
         }
         ui.data_mut(|d| d.remove::<u8>(key));
     }
+    texture_controls(ui, m, actions);
+}
+
+/// A material's texture controls: Texture... to load an image and Remove, then the size one copy
+/// covers (set when a drag ends or a typed value is entered).
+fn texture_controls(ui: &mut Ui, m: &MaterialRow, actions: &mut Vec<UiAction>) {
+    ui.horizontal(|ui| {
+        if ui
+            .button("Texture...")
+            .on_hover_text("Show a PNG or JPEG image on faces painted with this material")
+            .clicked()
+        {
+            actions.push(UiAction::LoadTexture(m.id));
+        }
+        if let Some(t) = &m.texture {
+            if ui.button("Remove").clicked() {
+                actions.push(UiAction::RemoveTexture(m.id));
+            }
+            ui.label(RichText::new(format!("{} x {} px", t.pixels[0], t.pixels[1])).weak());
+        }
+    });
+    let Some(t) = &m.texture else { return };
+    // Like the opacity: the size being dragged lives in egui's memory until the drag ends.
+    let key = egui::Id::new(("texture-size", m.id.0));
+    let shown = [t.width, t.height];
+    let mut size = ui.data(|d| d.get_temp::<[f64; 2]>(key)).unwrap_or(shown);
+    let (dragged, done) = ui
+        .horizontal(|ui| {
+            ui.label("Size");
+            let field = |ui: &mut Ui, value: &mut f64| {
+                ui.add(
+                    egui::DragValue::new(value)
+                        .range(kernel::MIN_TEXTURE_SIZE..=kernel::MAX_TEXTURE_SIZE)
+                        .speed(0.01)
+                        .max_decimals(3)
+                        .suffix(" m"),
+                )
+            };
+            let w = field(ui, &mut size[0]);
+            ui.label("x");
+            let h = field(ui, &mut size[1]);
+            let dragged = w.dragged() || h.dragged();
+            let done = w.drag_stopped() || h.drag_stopped() || w.changed() || h.changed();
+            (dragged, done)
+        })
+        .inner;
+    if dragged {
+        ui.data_mut(|d| d.insert_temp(key, size));
+    } else {
+        let mm = |m: f64| (m * 1000.0).round() as u32;
+        if done && size.map(mm) != shown.map(mm) {
+            actions.push(UiAction::SetTextureSize(m.id, size.map(mm)));
+        }
+        ui.data_mut(|d| d.remove::<[f64; 2]>(key));
+    }
 }
 
 /// An opacity (0 to 1) in whole percent.
@@ -1149,12 +1236,14 @@ mod tests {
                 name: "Brick".into(),
                 color: [180, 60, 40],
                 opacity: 1.0,
+                texture: None,
             },
             MaterialRow {
                 id: MaterialId(32),
                 name: "Glass".into(),
                 color: [90, 170, 170],
                 opacity: 0.4,
+                texture: None,
             },
         ];
         let mut view = View {
@@ -1222,6 +1311,57 @@ mod tests {
         );
     }
 
+    #[test]
+    fn the_materials_list_loads_removes_and_sizes_textures() {
+        let status = status();
+        let info = vec!["Nothing selected".to_string()];
+        let rows = brick();
+        let view = name_field_view(&status, &info, &rows);
+        // Without a texture: Texture... only.
+        let clicked: Vec<UiAction> = [160.0, 120.0, 80.0]
+            .into_iter()
+            .flat_map(|x| panel_actions(&view, 1280.0 - x, None))
+            .collect();
+        assert!(
+            clicked.contains(&UiAction::LoadTexture(MaterialId(31))),
+            "{clicked:?}"
+        );
+        assert!(
+            !clicked
+                .iter()
+                .any(|a| matches!(a, UiAction::RemoveTexture(_) | UiAction::SetTextureSize(..))),
+            "{clicked:?}"
+        );
+        let mut textured = brick();
+        textured[0].texture = Some(TextureRow {
+            swatch: egui::TextureId::default(),
+            width: 1.0,
+            height: 0.5,
+            pixels: [2, 1],
+        });
+        let view = name_field_view(&status, &info, &textured);
+        let clicked: Vec<UiAction> = [160.0, 120.0, 80.0]
+            .into_iter()
+            .flat_map(|x| panel_actions(&view, 1280.0 - x, None))
+            .collect();
+        for action in [
+            UiAction::ChooseMaterial(Some(MaterialId(31))),
+            UiAction::LoadTexture(MaterialId(31)),
+            UiAction::RemoveTexture(MaterialId(31)),
+        ] {
+            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
+        }
+        // Typing 2 into the width field, then the height field, sets the size in millimeters.
+        let typed: Vec<UiAction> = [160.0, 120.0, 80.0]
+            .into_iter()
+            .flat_map(|x| panel_actions(&view, 1280.0 - x, Some("2")))
+            .collect();
+        for size in [[2000, 500], [1000, 2000]] {
+            let action = UiAction::SetTextureSize(MaterialId(31), size);
+            assert!(typed.contains(&action), "{action:?} in {typed:?}");
+        }
+    }
+
     /// Two materials, the first chosen, for the name-field tests.
     fn name_field_view<'a>(
         status: &'a Status,
@@ -1255,6 +1395,7 @@ mod tests {
             name: "Brick".into(),
             color: [180, 60, 40],
             opacity: 1.0,
+            texture: None,
         }]
     }
 
@@ -1687,6 +1828,7 @@ mod tests {
                 name: format!("Material {k}"),
                 color: [10, 20, 30],
                 opacity: 1.0,
+                texture: None,
             })
             .collect();
         let table = OutlinerTable {
@@ -1733,6 +1875,7 @@ mod tests {
                 name: format!("Material {k}"),
                 color: [10, 20, 30],
                 opacity: 1.0,
+                texture: None,
             })
             .collect();
         let mut view = outliner_view(&status, &info, &table, &[], &selection);
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 7e52969..692d198 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -265,7 +265,11 @@ current material (a click on something selected paints the whole selection's fac
 groups, a click on a group paints the group), and Cmd-click on macOS (Ctrl-click on Windows
 and Linux) picks up the clicked side's material instead. The Materials list under Entity Info chooses the
 current material and adds, renames, recolors and deletes materials and sets their opacity (a
-0-100% slider; one change per drag), all through these commands.
+0-100% slider; one change per drag), all through these commands. A material with a texture
+shows a small copy of its image as its swatch; for the chosen material, Texture... loads a PNG
+or JPEG image from a file dialog (keeping the size of the texture it replaces), Remove takes the
+texture off, and the Size fields set its width and height in meters (one change per drag or
+typed value).
 
 ### Groups and components
 
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app --bin app textures_load` and `cargo test -p app --bin app ui::` (all pass; the existing Materials list tests still find their controls, as `BELOW_MATERIALS_LIST` grew by the two new rows).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/app/src/ui.rs crates/app/src/app.rs crates/app/src/gui.rs crates/app/src/editor.rs docs/agent-api.md
git commit -m "Materials list: texture swatch, Texture..., Remove and size"
```

---

## After the last task

- Workspace tests: 968 (948 before).
- Mark M9b done in `docs/superpowers/roadmap.md` with the first and last commit, and record the milestone (done items and anything deferred) in the project memory, as the roadmap rules ask.
- Not verified while planning: exported glTF and OBJ files were not opened in another program or a glTF validator (the tests check the document's structure and the bytes); the app window was not launched (the Materials list is tested through egui input, the rendering through headless golden images on this machine's GPU).
- Deferred candidates to record: per-face texture positioning (spec non-goal); texture alpha (cut-outs) ignored; back sides read mirrored; OBJ `vt` unshifted far from the origin; decoded images held in memory (64 MiB for a 4096 px image); the texture array is rebuilt on the main thread when an image changes (resampling a 4096 px image to 1024 takes a moment in debug builds); anisotropic filtering off; JPEG EXIF orientation ignored.
