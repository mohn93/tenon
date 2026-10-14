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
