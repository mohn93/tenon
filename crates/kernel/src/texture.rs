//! Textures: images on materials, projected in each face's plane.
//!
//! A face shows its material's texture laid flat in its own plane, in the coordinates of the
//! definition holding it: the image's width runs along the face's horizontal direction (x on
//! level faces) and its height across, repeating every `width` by `height` meters. Faces in one
//! plane continue each other's pattern, and a component carries its texture with it. A face's
//! placement moves, turns and scales its texture on top of that, and the back side shows the
//! texture as seen from behind.

use std::collections::BTreeSet;
use std::sync::Arc;

use glam::{DAffine3, DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::FaceId;
use crate::model::{ChangeSet, Face, Model};

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
    let u = if is_level(normal) {
        DVec3::X
    } else {
        DVec3::Z.cross(normal).normalize()
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

/// The least and greatest scale a texture placement takes along u or v.
pub const MIN_TEXTURE_SCALE: f64 = 0.001;
pub const MAX_TEXTURE_SCALE: f64 = 1000.0;
/// The farthest a texture placement moves the texture along u or v, in meters.
pub const MAX_TEXTURE_OFFSET: f64 = 1.0e6;

/// Where a face's texture lies on top of the projection (see [`texture_position`]): moved by
/// `offset` meters along u and v, turned `rotation_deg` counter-clockwise seen from the face's
/// front about the moved origin, and `scale` times as large along its own width and height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TexturePlacement {
    pub offset: [f64; 2],
    /// From 0 up to (not including) 360.
    pub rotation_deg: f64,
    pub scale: [f64; 2],
}

impl Default for TexturePlacement {
    fn default() -> Self {
        TexturePlacement {
            offset: [0.0, 0.0],
            rotation_deg: 0.0,
            scale: [1.0, 1.0],
        }
    }
}

impl TexturePlacement {
    /// True for the placement that leaves the projection as it is.
    pub fn is_default(&self) -> bool {
        *self == TexturePlacement::default()
    }

    /// A projected texture position (meters along u and v) as a position on the placed texture:
    /// meters along the image's width and height, as if it had its own size.
    pub fn place(&self, p: DVec2) -> DVec2 {
        let (sin, cos) = self.rotation_deg.to_radians().sin_cos();
        let q = p - DVec2::from(self.offset);
        // Turning the texture one way turns the points on it the other.
        let turned = DVec2::new(cos * q.x + sin * q.y, cos * q.y - sin * q.x);
        turned / DVec2::from(self.scale)
    }

    /// The placement that shows the same texture on each side once the face is turned over
    /// (its normal negated): the projection's u flips on a sloping face, v on a level one
    /// (see [`texture_axes`]), and the back mirrors u (see [`back_texture_position`]).
    pub(crate) fn turned_over(&self, level: bool) -> TexturePlacement {
        let [x, y] = self.offset;
        if level {
            TexturePlacement {
                offset: [x, -y],
                rotation_deg: normal_angle(-self.rotation_deg - 180.0),
                scale: self.scale,
            }
        } else {
            TexturePlacement {
                offset: [-x, y],
                rotation_deg: normal_angle(-self.rotation_deg),
                scale: self.scale,
            }
        }
    }
}

impl TexturePlacement {
    /// The placement that keeps a face's texture where it was on the model when the face, of
    /// unit `normal`, is moved by `t`: the projection is taken anew in the moved face's plane,
    /// so the placement takes up the move, turn and (even) scale of `t` within that plane.
    /// A mirror or an uneven scale cannot be shown by a placement; the nearest turn and scale
    /// are taken.
    pub(crate) fn carried(
        placement: Option<TexturePlacement>,
        normal: DVec3,
        t: &DAffine3,
    ) -> Option<TexturePlacement> {
        Self::carry(placement, normal, t).0
    }

    /// `carried`, and whether the placement it should have lies beyond what a placement holds
    /// (so `carried` had to clamp it and the texture moves).
    pub(crate) fn carry(
        placement: Option<TexturePlacement>,
        normal: DVec3,
        t: &DAffine3,
    ) -> (Option<TexturePlacement>, bool) {
        let p = placement.unwrap_or_default();
        let moved = t.transform_vector3(normal).normalize_or_zero();
        if moved == DVec3::ZERO {
            return (placement, false);
        }
        let (u, v) = texture_axes(normal);
        let (u2, v2) = texture_axes(moved);
        let (lu, lv) = (t.transform_vector3(u), t.transform_vector3(v));
        // The plane's map from the old projection to the new: its turn and its scale.
        let (a, b, c, d) = (lu.dot(u2), lv.dot(u2), lu.dot(v2), lv.dot(v2));
        let k = (a * d - b * c).abs().sqrt();
        if !(k.is_finite() && k > 0.0) {
            return (placement, false);
        }
        let turn = c.atan2(a);
        let shift = texture_position(moved, t.translation);
        let offset = shift + k * DVec2::from_angle(turn).rotate(DVec2::from(p.offset));
        let clamp_offset = |x: f64| x.clamp(-MAX_TEXTURE_OFFSET, MAX_TEXTURE_OFFSET);
        let clamp_scale = |s: f64| (s * k).clamp(MIN_TEXTURE_SCALE, MAX_TEXTURE_SCALE);
        let beyond = offset.x.abs() > MAX_TEXTURE_OFFSET
            || offset.y.abs() > MAX_TEXTURE_OFFSET
            || p.scale
                .iter()
                .any(|s| !(MIN_TEXTURE_SCALE..=MAX_TEXTURE_SCALE).contains(&(s * k)));
        let carried = Some(TexturePlacement {
            offset: [clamp_offset(offset.x) + 0.0, clamp_offset(offset.y) + 0.0],
            rotation_deg: normal_angle(p.rotation_deg + turn.to_degrees()),
            scale: p.scale.map(clamp_scale),
        })
        .filter(|q| !q.is_default());
        (carried, beyond)
    }
}

/// An angle in degrees brought into 0 up to (not including) 360.
pub fn normal_angle(deg: f64) -> f64 {
    let a = deg.rem_euclid(360.0) + 0.0;
    if a >= 360.0 { 0.0 } else { a }
}

/// Fails, saying why, when a placement's numbers are not finite, its offset is beyond
/// [`MAX_TEXTURE_OFFSET`], its rotation is not from 0 up to 360 or a scale is not from
/// [`MIN_TEXTURE_SCALE`] to [`MAX_TEXTURE_SCALE`].
pub(crate) fn check_placement(t: &TexturePlacement) -> Result<(), String> {
    if !t
        .offset
        .iter()
        .all(|x| x.is_finite() && x.abs() <= MAX_TEXTURE_OFFSET)
    {
        return Err(format!(
            "a texture offset must be from -{MAX_TEXTURE_OFFSET} to {MAX_TEXTURE_OFFSET} m, not \
             {:?}",
            t.offset
        ));
    }
    if !(t.rotation_deg.is_finite() && (0.0..360.0).contains(&t.rotation_deg)) {
        return Err(format!(
            "a texture rotation must be from 0 up to 360 degrees, not {}",
            t.rotation_deg
        ));
    }
    if !t
        .scale
        .iter()
        .all(|s| s.is_finite() && (MIN_TEXTURE_SCALE..=MAX_TEXTURE_SCALE).contains(s))
    {
        return Err(format!(
            "a texture scale must be from {MIN_TEXTURE_SCALE} to {MAX_TEXTURE_SCALE}, not {:?}",
            t.scale
        ));
    }
    Ok(())
}

/// True when a face with unit `normal` lays its texture along x (see [`texture_axes`]).
pub(crate) fn is_level(normal: DVec3) -> bool {
    DVec3::Z.cross(normal).length() < LEVEL
}

/// Where a point lies on the back side's texture, from where it lies on the front's: u
/// mirrored, so the back reads as the image seen from behind, not mirrored.
pub fn back_texture_position(front: DVec2) -> DVec2 {
    DVec2::new(-front.x, front.y)
}

impl Face {
    /// Where point `p` of the face lies on its front's texture, in meters along the texture's
    /// width and height: the projection (see [`texture_position`]) with the face's placement
    /// on top. The back side's is [`back_texture_position`] of it.
    pub fn texture_position(&self, p: DVec3) -> DVec2 {
        let projected = texture_position(self.normal, p);
        match &self.texture {
            Some(t) => t.place(projected),
            None => projected,
        }
    }
}

/// What [`Model::position_texture`] sets on each face: the fields given, on top of the face's
/// placement or, with `reset`, of none; absent fields stay as they are.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlacementEdit {
    pub offset: Option<[f64; 2]>,
    /// Any finite angle; stored from 0 up to 360.
    pub rotation_deg: Option<f64>,
    /// Turns each face's texture further by this many degrees, after `rotation_deg`.
    pub rotate_by: Option<f64>,
    pub scale: Option<[f64; 2]>,
    pub reset: bool,
}

impl PlacementEdit {
    /// `placement` with this edit applied.
    pub fn applied(&self, placement: Option<TexturePlacement>) -> TexturePlacement {
        let mut t = if self.reset {
            TexturePlacement::default()
        } else {
            placement.unwrap_or_default()
        };
        if let Some(offset) = self.offset {
            t.offset = offset;
        }
        if let Some(r) = self.rotation_deg {
            t.rotation_deg = normal_angle(r);
        }
        if let Some(r) = self.rotate_by {
            t.rotation_deg = normal_angle(t.rotation_deg + r);
        }
        if let Some(scale) = self.scale {
            t.scale = scale;
        }
        t
    }
}

impl Model {
    /// Sets the texture placement of `faces` as one ChangeSet, returning how many changed.
    /// Unknown ids are ignored. Fails, changing nothing, when a given field is out of range
    /// (see [`TexturePlacement`]).
    pub fn position_texture(
        &mut self,
        faces: &[FaceId],
        edit: PlacementEdit,
    ) -> Result<(ChangeSet, usize), KernelError> {
        check_placement(
            &PlacementEdit {
                reset: true,
                ..edit
            }
            .applied(None),
        )
        .map_err(KernelError::TexturePlacement)?;
        let mut changed = 0;
        for &id in faces.iter().collect::<BTreeSet<_>>() {
            let Some(face) = self.face(id) else { continue };
            let to = Some(edit.applied(face.texture)).filter(|t| !t.is_default());
            if to != face.texture {
                self.set_texture(id, to);
                changed += 1;
            }
        }
        Ok((self.take_changes(), changed))
    }
}
