//! Import Image: a PNG or JPEG placed with the Image tool.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use session::{CommandError, ErrorKind, texture};
use tools::{ImageSpec, ImageTool};

use super::Editor;

/// The name the picture takes when the file has none to give.
const DEFAULT_NAME: &str = "Image";

/// Whether `name` is an image file this import places (by its extension).
pub fn is_image_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

impl Editor {
    /// Brings out the Image tool with the image `bytes` of the file called `name`: the picture
    /// follows the cursor until two clicks (or a click and a typed width) place it as one
    /// `import_image` step; Escape goes back to the tool used before. An image over the
    /// texture limits (see `import_image`) or a broken file is refused, with why in the status.
    pub fn place_image(&mut self, name: &str, bytes: &[u8]) -> Result<(), CommandError> {
        // Turned upright and decoded once here, so a broken file is refused before the tool
        // comes out and the outline has the proportions the picture will have. The texture
        // budget, as the placement will check it, goes by the header, before the decoding.
        let checked = texture::upright(bytes.to_vec()).and_then(|b| {
            let (_, wide, high) = texture::dimensions(&b)?;
            self.doc
                .texture_budget_allows(u64::from(wide) * u64::from(high))?;
            texture::decode(b.clone())?;
            Ok((b, wide, high))
        });
        let (bytes, wide, high) = match checked {
            Ok(d) => d,
            Err(m) => {
                return Err(self.refuse(CommandError::new(
                    ErrorKind::InvalidInput,
                    format!("{name}: {m}"),
                )));
            }
        };
        let stem = Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_NAME);
        self.start_paste_tool(ImageTool::new(ImageSpec {
            name: stem.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            aspect: f64::from(high) / f64::from(wide),
        }));
        Ok(())
    }

    /// `place_image` for a file at `path` or base64 `data` (exactly one).
    pub fn place_image_from(
        &mut self,
        path: Option<&str>,
        data: Option<&str>,
    ) -> Result<(), CommandError> {
        match (path, data) {
            (Some(path), None) => {
                let mut bytes = Vec::new();
                let read = std::fs::File::open(path).and_then(|f| {
                    f.take(texture::MAX_TEXTURE_BYTES + 1)
                        .read_to_end(&mut bytes)
                });
                if let Err(e) = read {
                    return Err(
                        self.refuse(CommandError::new(ErrorKind::File, format!("{path}: {e}")))
                    );
                }
                self.place_image(path, &bytes)
            }
            (None, Some(data)) => match texture::decode_base64(data) {
                Ok(bytes) => self.place_image(DEFAULT_NAME, &bytes),
                Err(m) => Err(self.refuse(CommandError::new(ErrorKind::InvalidInput, m))),
            },
            _ => Err(self.refuse(CommandError::new(
                ErrorKind::InvalidInput,
                "give the image as path or data, not both or neither",
            ))),
        }
    }
}
