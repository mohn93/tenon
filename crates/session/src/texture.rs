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
