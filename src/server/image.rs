//! Receipt photo pre-processing.
//!
//! Not an optimization — a required step. A full-resolution phone photo would
//! blow out both the model's context and VRAM, and Ollama's image decoder is
//! narrow (it will not take webp), so everything is normalized to a modest JPEG
//! before it reaches the model. The original file is kept on disk untouched.

use std::io::Cursor;

use image::{DynamicImage, ImageReader};

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("could not decode image: {0}")]
    Decode(#[from] image::ImageError),
    #[error("could not read image: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Prepared {
    /// Baseline JPEG, ready to base64-encode for Ollama.
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Decodes any supported format, applies EXIF rotation, downscales so the long
/// edge is at most `max_edge`, and re-encodes as JPEG.
pub fn prepare(bytes: &[u8], max_edge: u32) -> Result<Prepared, ImageError> {
    let decoded = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;

    let oriented = apply_exif_orientation(decoded, bytes);
    let scaled = downscale(oriented, max_edge);

    let (width, height) = (scaled.width(), scaled.height());

    // JPEG has no alpha channel, and a lossless webp or PNG screenshot will
    // often be RGBA — encoding that directly fails.
    let rgb = scaled.to_rgb8();

    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut Cursor::new(&mut jpeg), 80)
        .encode_image(&DynamicImage::ImageRgb8(rgb))?;

    Ok(Prepared {
        jpeg,
        width,
        height,
    })
}

fn downscale(img: DynamicImage, max_edge: u32) -> DynamicImage {
    let long_edge = img.width().max(img.height());
    if long_edge <= max_edge {
        return img;
    }
    let scale = max_edge as f32 / long_edge as f32;
    let w = ((img.width() as f32 * scale).round() as u32).max(1);
    let h = ((img.height() as f32 * scale).round() as u32).max(1);
    img.resize(w, h, image::imageops::FilterType::Lanczos3)
}

/// Phone cameras record rotation in EXIF rather than rotating the pixels, and
/// the `image` crate does not apply it on decode. A sideways receipt measurably
/// hurts extraction, so honour the tag.
fn apply_exif_orientation(img: DynamicImage, original: &[u8]) -> DynamicImage {
    match exif_orientation(original).unwrap_or(1) {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        // 1, or anything unexpected: leave it alone.
        _ => img,
    }
}

fn exif_orientation(bytes: &[u8]) -> Option<u32> {
    let mut cursor = Cursor::new(bytes);
    let exif = exif::Reader::new().read_from_container(&mut cursor).ok()?;
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::new(w, h));
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn downscales_the_long_edge_and_preserves_aspect_ratio() {
        let out = prepare(&png(4000, 2000), 1600).unwrap();
        assert_eq!(out.width, 1600);
        assert_eq!(out.height, 800);
    }

    #[test]
    fn leaves_small_images_alone() {
        let out = prepare(&png(800, 600), 1600).unwrap();
        assert_eq!((out.width, out.height), (800, 600));
    }

    /// Portrait receipts are the common case; the long edge is the height.
    #[test]
    fn downscales_by_height_when_portrait() {
        let out = prepare(&png(1000, 3000), 1500).unwrap();
        assert_eq!((out.width, out.height), (500, 1500));
    }

    /// RGBA input must not fail JPEG encoding.
    #[test]
    fn encodes_rgba_input_as_jpeg() {
        let out = prepare(&png(100, 100), 1600).unwrap();
        assert_eq!(&out.jpeg[..2], &[0xFF, 0xD8], "expected a JPEG SOI marker");
    }
}
