//! Reading texture images: PNG and JPEG files decoded to sRGB pixels.

use std::io::Cursor;
use std::sync::Arc;

use image::{ImageReader, Limits};
use kernel::{ImageFormat, Texture, TextureImage};

/// The largest texture file read, in bytes.
pub const MAX_TEXTURE_BYTES: u64 = 16 * 1024 * 1024;
/// The most pixels along either side of a texture image.
pub const MAX_TEXTURE_PIXELS: u32 = 8192;

/// The most pixels all of a document's texture images hold together (1 GiB decoded), so a
/// small file of many large, plain images cannot take all memory when opened.
pub const MAX_TOTAL_TEXTURE_PIXELS: u64 = 1 << 28;

/// The bytes of base64 `data`, as encoders and people write it: whitespace (line breaks every
/// 76 characters, indentation) anywhere is ignored, the `=` padding may be left off, and a
/// leading data URL header (`data:image/png;base64,`) is taken off. Fails, saying why, when it
/// is not base64 or holds more than [`MAX_TEXTURE_BYTES`] decoded bytes; the size is counted in
/// decoded bytes, not characters, before anything is copied.
pub fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    use base64::alphabet::STANDARD;
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

    let engine = GeneralPurpose::new(
        &STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let data = data.trim_start();
    let data = match data.get(..5) {
        Some(head) if head.eq_ignore_ascii_case("data:") => data
            .split_once(";base64,")
            .map(|(_, rest)| rest)
            .ok_or("a data URL must be base64 (data:...;base64,...)")?,
        _ => data,
    };
    let trimmed = data.trim_end().trim_end_matches('=');
    let digits = trimmed.chars().filter(|c| !c.is_whitespace()).count() as u64;
    // 4 characters hold 3 bytes; a trailing 2 or 3 characters hold 1 or 2.
    if digits / 4 * 3 + (digits % 4).saturating_sub(1) > MAX_TEXTURE_BYTES {
        return Err(format!(
            "file too large: a texture is at most {MAX_TEXTURE_BYTES} bytes"
        ));
    }
    let compact: String = data.split_whitespace().collect();
    engine
        .decode(compact)
        .map_err(|_| "data is not base64".to_string())
}

/// A PNG or JPEG file's format and size in pixels, read from its header without decoding it.
/// Fails, saying why, for other formats, broken headers and images more than
/// [`MAX_TEXTURE_PIXELS`] wide or high.
pub fn dimensions(encoded: &[u8]) -> Result<(ImageFormat, u32, u32), String> {
    if encoded.len() as u64 > MAX_TEXTURE_BYTES {
        return Err(format!(
            "file too large: a texture is at most {MAX_TEXTURE_BYTES} bytes"
        ));
    }
    let reader = ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(|_| "the image's header is broken".to_string())?;
    let format = match reader.format() {
        Some(image::ImageFormat::Png) => ImageFormat::Png,
        Some(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
        _ => return Err("not a PNG or JPEG image".into()),
    };
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "the image's header is broken".to_string())?;
    if width > MAX_TEXTURE_PIXELS || height > MAX_TEXTURE_PIXELS {
        return Err(format!(
            "the image is {width} x {height} pixels: a texture has at most \
             {MAX_TEXTURE_PIXELS} on each side"
        ));
    }
    Ok((format, width, height))
}

/// Fails, saying why, when an image of `pixels` added to `others` already held goes over
/// [`MAX_TOTAL_TEXTURE_PIXELS`].
pub fn check_total_pixels(others: u64, pixels: u64) -> Result<(), String> {
    if others.saturating_add(pixels) > MAX_TOTAL_TEXTURE_PIXELS {
        return Err(format!(
            "too many texture pixels: all textures together hold at most \
             {MAX_TOTAL_TEXTURE_PIXELS} pixels ({others} already, {pixels} more)"
        ));
    }
    Ok(())
}

/// Decodes a PNG or JPEG file's bytes, keeping them. Fails, saying why, as [`dimensions`] does
/// and for broken files. Any alpha is kept in the pixels but not shown (textures are opaque).
pub fn decode(encoded: Vec<u8>) -> Result<TextureImage, String> {
    let (format, _, _) = dimensions(&encoded)?;
    let mut reader = ImageReader::new(Cursor::new(&encoded[..]))
        .with_guessed_format()
        .map_err(|_| "the image is broken".to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_PIXELS);
    limits.max_image_height = Some(MAX_TEXTURE_PIXELS);
    reader.limits(limits);
    let pixels = reader
        .decode()
        .map_err(|_| "the image is broken".to_string())?
        .into_rgba8();
    Ok(TextureImage {
        format,
        width: pixels.width(),
        height: pixels.height(),
        pixels: pixels.into_raw(),
        encoded,
    })
}

/// A PNG or JPEG file's bytes turned upright as its EXIF orientation says (a phone keeps a
/// photo on its side and says so), re-encoded in the same format; the bytes as they are when
/// they need no turning. Fails, saying why, as [`decode`] does, and when the turned image
/// would be more than [`MAX_TEXTURE_BYTES`].
pub fn upright(encoded: Vec<u8>) -> Result<Vec<u8>, String> {
    use image::ImageDecoder;
    use image::metadata::Orientation;

    let (format, _, _) = dimensions(&encoded)?;
    let turned = {
        let mut reader = ImageReader::new(Cursor::new(&encoded[..]))
            .with_guessed_format()
            .map_err(|_| "the image is broken".to_string())?;
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_TEXTURE_PIXELS);
        limits.max_image_height = Some(MAX_TEXTURE_PIXELS);
        reader.limits(limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|_| "the image is broken".to_string())?;
        // An unreadable orientation is taken as none, as viewers do.
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        if orientation == Orientation::NoTransforms {
            None
        } else {
            let mut image = image::DynamicImage::from_decoder(decoder)
                .map_err(|_| "the image is broken".to_string())?;
            image.apply_orientation(orientation);
            Some(image)
        }
    };
    let Some(image) = turned else {
        return Ok(encoded);
    };
    let mut turned = Vec::new();
    let written = match format {
        ImageFormat::Jpeg => image::DynamicImage::ImageRgb8(image.to_rgb8()).write_with_encoder(
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut turned, 92),
        ),
        ImageFormat::Png => {
            image.write_with_encoder(image::codecs::png::PngEncoder::new(&mut turned))
        }
    };
    written.map_err(|_| "the image could not be turned upright".to_string())?;
    if turned.len() as u64 > MAX_TEXTURE_BYTES {
        return Err(format!(
            "file too large: turned upright, the image is more than {MAX_TEXTURE_BYTES} bytes"
        ));
    }
    Ok(turned)
}

/// A texture `width` by `height` meters of a PNG or JPEG file's bytes, decoded as [`decode`]
/// does. Fails, saying why, as [`decode`] does; the size is checked when the texture is given
/// to a material.
pub fn texture_from_bytes(bytes: Vec<u8>, width: f64, height: f64) -> Result<Texture, String> {
    Ok(Texture {
        image: Arc::new(decode(bytes)?),
        width,
        height,
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

    #[test]
    fn base64_takes_line_breaks_and_missing_padding() {
        // "hello" is aGVsbG8= padded.
        for text in ["aGVsbG8=", "aGVsbG8", "aGVs\nbG8=", "  aGVs\r\n  bG8\n"] {
            assert_eq!(decode_base64(text).unwrap(), b"hello", "{text:?}");
        }
        assert!(decode_base64("aGV$bG8=").is_err());
        // Padding in the middle is not base64.
        assert!(decode_base64("aGU=bG8=").is_err());
    }

    #[test]
    fn base64_takes_a_data_url() {
        for text in [
            "data:image/png;base64,aGVsbG8=",
            "  DATA:image/jpeg;base64,\naGVs\nbG8",
        ] {
            assert_eq!(decode_base64(text).unwrap(), b"hello", "{text:?}");
        }
        let err = decode_base64("data:text/plain,hello").unwrap_err();
        assert!(err.contains("data URL"), "{err}");
    }

    #[test]
    fn base64_size_counts_decoded_bytes_not_characters() {
        let at_limit = "QUJD".repeat(MAX_TEXTURE_BYTES as usize / 3);
        // Line breaks every 76 characters make the text longer, not the file.
        let broken: String = at_limit
            .as_bytes()
            .chunks(76)
            .map(|c| format!("{}\n", std::str::from_utf8(c).unwrap()))
            .collect();
        assert!(broken.len() as u64 > MAX_TEXTURE_BYTES / 3 * 4);
        assert_eq!(
            decode_base64(&broken).unwrap().len() as u64,
            MAX_TEXTURE_BYTES / 3 * 3
        );
        let over = "QUJD".repeat(MAX_TEXTURE_BYTES as usize / 3 + 1);
        assert!(decode_base64(&over).unwrap_err().contains("too large"));
    }

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
        let err = decode(png(8193, 1)).unwrap_err();
        assert!(err.contains("at most 8192"), "{err}");
        let err = decode(vec![0; MAX_TEXTURE_BYTES as usize + 1]).unwrap_err();
        assert!(err.contains("file too large"), "{err}");
    }

    /// A JPEG `width` x `height` pixels, left half dark and right half light, whose EXIF
    /// orientation is `orientation`.
    fn oriented_jpeg(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        let mut jpeg = Vec::new();
        image::RgbImage::from_fn(width, height, |x, _| {
            image::Rgb(if x < width / 2 { [0; 3] } else { [255; 3] })
        })
        .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .unwrap();
        // An APP1 Exif segment after SOI: a little-endian TIFF header with one IFD entry.
        let mut tiff = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        tiff.extend(orientation.to_le_bytes());
        tiff.extend([0, 0, 0, 0, 0, 0]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut out = jpeg[..2].to_vec();
        out.extend([0xFF, 0xE1]);
        out.extend(((app1.len() + 2) as u16).to_be_bytes());
        out.extend(app1);
        out.extend(&jpeg[2..]);
        out
    }

    #[test]
    fn upright_turns_a_photo_kept_on_its_side_and_leaves_others_as_they_are() {
        let plain = png(3, 2);
        assert_eq!(upright(plain.clone()).unwrap(), plain);
        let level = oriented_jpeg(16, 8, 1);
        assert_eq!(upright(level.clone()).unwrap(), level);
        // 6: turn 90 degrees clockwise to show; the dark left half ends on top.
        let turned = decode(upright(oriented_jpeg(16, 8, 6)).unwrap()).unwrap();
        assert_eq!(
            (turned.format, turned.width, turned.height),
            (ImageFormat::Jpeg, 8, 16)
        );
        let at = |x: u32, y: u32| turned.pixels[((y * turned.width + x) * 4) as usize];
        assert!(
            at(4, 2) < 60 && at(4, 13) > 200,
            "{} {}",
            at(4, 2),
            at(4, 13)
        );
    }

    #[test]
    fn the_total_pixel_budget_counts_what_is_held_and_what_comes() {
        assert_eq!(dimensions(&png(5, 3)).unwrap(), (ImageFormat::Png, 5, 3));
        check_total_pixels(MAX_TOTAL_TEXTURE_PIXELS - 15, 15).unwrap();
        let err = check_total_pixels(MAX_TOTAL_TEXTURE_PIXELS - 15, 16).unwrap_err();
        assert!(err.contains("too many texture pixels"), "{err}");
        assert!(check_total_pixels(u64::MAX, 1).is_err());
    }
}
