//! LCD image and animation decoding.
//!
//! Ported from `MyLianPC.Lcd/LcdMedia.cs`. Every frame is scaled with a
//! centre-crop "cover" fit to the 480×480 panel. Animation delays are clamped to
//! the rate the panel's JPEG path sustains comfortably.

use std::io::Cursor;

use image::{AnimationDecoder, RgbaImage};
use tiny_skia::{IntSize, Pixmap};

use crate::render::SIZE;

pub const MAX_FRAMES: usize = 600;
/// ~15 fps: what the panel's JPEG path sustains comfortably.
pub const MIN_FRAME_MS: u64 = 66;

pub const SUPPORTED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];

pub struct LcdMedia {
    pub frames: Vec<Pixmap>,
    pub durations_ms: Vec<u64>,
}

impl LcdMedia {
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Decodes a still image or every frame of an animated GIF. An unsupported
    /// or corrupt file returns a human-readable error.
    pub fn load(data: &[u8]) -> Result<Self, String> {
        if data.starts_with(b"GIF8") {
            let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(data))
                .map_err(|error| format!("Unsupported or corrupt GIF: {error}"))?;
            return Self::from_frames(
                decoder
                    .into_frames()
                    .collect_frames()
                    .map_err(|error| format!("Could not decode the GIF: {error}"))?
                    .into_iter()
                    .map(|frame| {
                        let (numerator, denominator) = frame.delay().numer_denom_ms();
                        let ms = if denominator == 0 {
                            100
                        } else {
                            (numerator as u64 / denominator as u64).max(MIN_FRAME_MS)
                        };
                        (frame.into_buffer(), ms)
                    })
                    .collect(),
            );
        }
        if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(data))
                .map_err(|error| format!("Unsupported or corrupt WebP: {error}"))?;
            let frames = decoder
                .into_frames()
                .collect_frames()
                .map_err(|error| format!("Could not decode the WebP: {error}"))?;
            return Self::from_frames(
                frames
                    .into_iter()
                    .map(|frame| {
                        let (numerator, denominator) = frame.delay().numer_denom_ms();
                        let ms = if denominator == 0 {
                            100
                        } else {
                            (numerator as u64 / denominator as u64).max(MIN_FRAME_MS)
                        };
                        (frame.into_buffer(), ms)
                    })
                    .collect(),
            );
        }
        let image = image::load_from_memory(data)
            .map_err(|error| format!("Unsupported or corrupt image file: {error}"))?;
        let rgba = image.to_rgba8();
        Ok(Self {
            frames: vec![to_pixmap(&cover(&rgba))],
            durations_ms: vec![100],
        })
    }

    fn from_frames(frames: Vec<(RgbaImage, u64)>) -> Result<Self, String> {
        if frames.is_empty() {
            return Err("The animation has no frames.".to_string());
        }
        let mut pixmaps = Vec::new();
        let mut durations = Vec::new();
        for (frame, duration) in frames.into_iter().take(MAX_FRAMES) {
            durations.push(duration);
            pixmaps.push(to_pixmap(&cover(&frame)));
        }
        Ok(Self {
            frames: pixmaps,
            durations_ms: durations,
        })
    }

    /// Validates that a stored file decodes; used before accepting an upload.
    pub fn validate(data: &[u8]) -> Result<(), String> {
        Self::load(data).map(|_| ())
    }
}

/// Centre-crop "cover" fit to the panel.
fn cover(source: &RgbaImage) -> RgbaImage {
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        return RgbaImage::new(SIZE, SIZE);
    }
    let scale = (SIZE as f32 / width as f32).max(SIZE as f32 / height as f32);
    let new_width = ((width as f32 * scale).ceil() as u32).max(SIZE);
    let new_height = ((height as f32 * scale).ceil() as u32).max(SIZE);
    let resized = image::imageops::resize(
        source,
        new_width,
        new_height,
        image::imageops::FilterType::Triangle,
    );
    let x = (new_width - SIZE) / 2;
    let y = (new_height - SIZE) / 2;
    image::imageops::crop_imm(&resized, x, y, SIZE, SIZE).to_image()
}

fn to_pixmap(image: &RgbaImage) -> Pixmap {
    let (width, height) = image.dimensions();
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        let alpha = a as u32;
        if alpha == 0 {
            data.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            // tiny-skia stores premultiplied RGBA.
            let premultiply = |value: u8| ((value as u32 * alpha) / 255) as u8;
            data.extend_from_slice(&[premultiply(r), premultiply(g), premultiply(b), a]);
        }
    }
    Pixmap::from_vec(data, IntSize::from_wh(width, height).expect("image size"))
        .expect("RGBA pixmap")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn still_images_are_cover_fitted_to_the_panel() {
        let media = LcdMedia::load(&png(960, 480)).expect("decodes");
        assert_eq!(media.frames.len(), 1);
        assert_eq!(media.frames[0].width(), SIZE);
        assert_eq!(media.frames[0].height(), SIZE);
        assert!(!media.is_animated());
    }

    #[test]
    fn corrupt_files_produce_an_error() {
        assert!(LcdMedia::load(b"not an image").is_err());
    }

    #[test]
    fn supported_extensions_match_the_service() {
        for extension in ["png", "jpg", "jpeg", "bmp", "gif", "webp"] {
            assert!(SUPPORTED_EXTENSIONS.contains(&extension));
        }
    }
}
