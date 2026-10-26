//! `import_image`: a PNG or JPEG as a textured rectangle in its own group.

use std::collections::BTreeSet;
use std::path::Path;

use base64::Engine;
use glam::DVec3;
use kernel::{
    DefinitionKind, EntityId, ImportFace, ImportTree, MAX_TEXTURE_SIZE, MIN_TEXTURE_SIZE, Material,
    MeshId, PlacementEdit, SceneChange, normal_angle, texture_axes,
};

use super::import::read_limited;
use super::{Document, check_coordinates, unit_vector};
use crate::command::{CommandError, ErrorKind};
use crate::texture::{
    MAX_TEXTURE_BYTES, average_color, check_total_pixels, dimensions, texture_from_bytes, upright,
};

/// The name of the group and material when the image has none to give.
const DEFAULT_NAME: &str = "Image";
/// How far a picture lies in front of its plane unless told otherwise, in meters.
pub const DEFAULT_LIFT: f64 = 0.001;
/// The farthest a picture may lie in front of its plane, in meters.
const MAX_LIFT: f64 = 1.0;

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

impl Document {
    /// Places the image as a group of one textured rectangle at `origin` in the open context,
    /// selected. Nothing changes on failure.
    pub(super) fn import_image_changes(
        &mut self,
        (path, data): (Option<&str>, Option<&str>),
        (origin, normal, x_axis): ([f64; 3], Option<[f64; 3]>, Option<[f64; 3]>),
        width: f64,
        name: Option<&str>,
        lift: Option<f64>,
    ) -> Result<SceneChange, CommandError> {
        if name.is_some_and(|n| n.trim().is_empty()) {
            return Err(invalid("name must not be blank"));
        }
        if !(width.is_finite() && (MIN_TEXTURE_SIZE..=MAX_TEXTURE_SIZE).contains(&width)) {
            return Err(invalid(format!(
                "width must be from {MIN_TEXTURE_SIZE} to {MAX_TEXTURE_SIZE} m"
            )));
        }
        let lift = lift.unwrap_or(DEFAULT_LIFT);
        if !(lift.is_finite() && (0.0..=MAX_LIFT).contains(&lift)) {
            return Err(invalid(format!("lift must be from 0 to {MAX_LIFT} m")));
        }
        check_coordinates(&[origin])?;
        let n = match normal {
            None => DVec3::Z,
            Some(n) => unit_vector(DVec3::from(n))
                .ok_or_else(|| invalid("normal must be a finite, non-zero vector"))?,
        };
        let (u0, _) = texture_axes(n);
        let x = match x_axis {
            None => u0,
            Some(a) => {
                let a = DVec3::from(a);
                unit_vector(a)
                    .map(|a| a - n * a.dot(n))
                    .and_then(unit_vector)
                    .filter(|flat| flat.length() > 1e-9)
                    .ok_or_else(|| {
                        invalid("x_axis must be a finite vector that is not along the normal")
                    })?
            }
        };
        let (bytes, stem) = read_image(path, data)?;
        let bytes = upright(bytes).map_err(|e| image_error(path, e))?;
        let (_, pixels_wide, pixels_high) = dimensions(&bytes).map_err(|e| image_error(path, e))?;
        check_total_pixels(
            self.texture_pixels_besides(None),
            u64::from(pixels_wide) * u64::from(pixels_high),
        )
        .map_err(|e| image_error(path, e))?;
        let height = width * f64::from(pixels_high) / f64::from(pixels_wide);
        if !(MIN_TEXTURE_SIZE..=MAX_TEXTURE_SIZE).contains(&height) {
            return Err(invalid(format!(
                "the picture would be {height} m high: a width of {width} m on a \
                 {pixels_wide} x {pixels_high} pixel image must keep its height from \
                 {MIN_TEXTURE_SIZE} to {MAX_TEXTURE_SIZE} m"
            )));
        }
        let texture = texture_from_bytes(bytes, width, height).map_err(|e| image_error(path, e))?;
        let color = average_color(&texture.image);

        // The corners, counter-clockwise around the normal: the front faces along it.
        let y = n.cross(x);
        let at = DVec3::from(origin) + n * lift;
        let corners = [
            at,
            at + x * width,
            at + x * width + y * height,
            at + y * height,
        ];
        let placed: Vec<[f64; 3]> = corners.iter().map(|c| c.to_array()).collect();
        check_coordinates(&placed)?;

        let label = name
            .map(str::to_string)
            .or(stem)
            .unwrap_or_else(|| DEFAULT_NAME.to_string());
        let material = Material {
            name: self.scene.free_material_name(&label),
            color,
            opacity: 1.0,
            texture: Some(texture),
            emission: 0.0,
            metalness: kernel::DEFAULT_METALNESS,
            roughness: kernel::DEFAULT_ROUGHNESS,
            density: None,
        };
        let (material, painted) = self
            .scene
            .add_material_from(material)
            .map_err(|e| invalid(e.to_string()))?;

        let mut tree = ImportTree::default();
        tree.root.points = corners.to_vec();
        tree.root.faces.push(ImportFace {
            loops: vec![vec![0, 1, 2, 3]],
            front: Some(material),
            back: Some(material),
            // Fitted below, once the face's own frame is known.
            texture: None,
            tag: None,
            hidden: false,
        });
        let mesh = self.active();
        let undo = |scene: &mut kernel::Scene, change: &SceneChange| scene.apply(&change.inverse());
        let (instance, _, built) =
            match self
                .scene
                .import_tree(mesh, &tree, DefinitionKind::Group, &label)
            {
                Ok(done) => done,
                Err(e) => {
                    undo(&mut self.scene, &painted);
                    return Err(invalid(e.to_string()));
                }
            };

        // Fit the image to the face: its own corner on the first corner, its width along x.
        let definition = self
            .model()
            .instance(instance)
            .expect("the group was placed")
            .definition;
        let inside = MeshId::Definition(definition);
        let face = self
            .scene
            .mesh(inside)
            .and_then(|m| m.faces().next().map(|(id, f)| (id, f.normal)));
        let fitted = face
            .ok_or_else(|| invalid("the picture's face could not be made"))
            .and_then(|(face, normal)| {
                let (u, v) = texture_axes(normal);
                let edit = PlacementEdit {
                    offset: Some([at.dot(u), at.dot(v)]),
                    rotation_deg: Some(normal_angle(x.dot(v).atan2(x.dot(u)).to_degrees())),
                    ..Default::default()
                };
                self.scene
                    .edit(inside, |m| {
                        m.position_texture(&[face], edit).map(|(c, _)| c)
                    })
                    .map_err(|e| invalid(e.to_string()))
            });
        let fitted = match fitted {
            Ok(change) => change,
            Err(e) => {
                undo(&mut self.scene, &built);
                undo(&mut self.scene, &painted);
                return Err(e);
            }
        };
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        self.made_definitions = Some(vec![definition]);
        Ok(SceneChange::join([painted, built, fitted]))
    }
}

/// The image's bytes from the file at `path` or the base64 `data`, and the file's name without
/// its extension.
fn read_image(
    path: Option<&str>,
    data: Option<&str>,
) -> Result<(Vec<u8>, Option<String>), CommandError> {
    match (path, data) {
        (Some(path), None) => {
            super::files::check_nul(Path::new(path))?;
            let bytes = read_limited(Path::new(path), MAX_TEXTURE_BYTES)
                .map_err(|m| CommandError::new(ErrorKind::File, format!("{path}: {m}")))?;
            let stem = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string);
            Ok((bytes, stem))
        }
        (None, Some(data)) => {
            // Refuse before decoding: base64 is 4 characters to 3 bytes.
            if data.len() as u64 > MAX_TEXTURE_BYTES / 3 * 4 + 4 {
                return Err(invalid(format!(
                    "file too large: an image is at most {MAX_TEXTURE_BYTES} bytes"
                )));
            }
            let compact: String = data.split_whitespace().collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(compact)
                .map_err(|_| invalid("data is not base64"))?;
            Ok((bytes, None))
        }
        _ => Err(invalid(
            "give the image as path or data, not both or neither",
        )),
    }
}

/// An image that cannot be used: a `file` error naming the path, else invalid input.
fn image_error(path: Option<&str>, message: String) -> CommandError {
    match path {
        Some(path) => CommandError::new(ErrorKind::File, format!("{path}: {message}")),
        None => invalid(message),
    }
}
