//! CPU rendering primitives for the 480×480 LCD.
//!
//! The Windows renderer used SkiaSharp. This module offers the subset of that
//! API the themes need on top of `tiny-skia` (paths, fills, strokes, arcs,
//! rounded rectangles and text), so the theme code stays a close port.

use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use mylianpc_core::color::RgbColor;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};

pub const SIZE: u32 = 480;
pub const CENTER: f32 = SIZE as f32 / 2.0;
/// Radius of the area text may use; the panel is round and the outer edge sits
/// behind the bezel.
const SAFE_RADIUS: f32 = 226.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Normal,
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// System-font lookup with a small cache. Loading system fonts is expensive, so
/// one cache lives for the lifetime of the service.
pub struct FontCache {
    db: fontdb::Database,
    fonts: HashMap<(String, bool), Option<Arc<FontVec>>>,
}

impl Default for FontCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FontCache {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self {
            db,
            fonts: HashMap::new(),
        }
    }

    fn font(&mut self, family: &str, weight: Weight) -> Option<Arc<FontVec>> {
        let bold = weight == Weight::Bold;
        let key = (family.to_string(), bold);
        if let Some(font) = self.fonts.get(&key) {
            return font.clone();
        }
        let loaded = self.load(family, bold);
        self.fonts.insert(key, loaded.clone());
        loaded
    }

    fn load(&self, family: &str, bold: bool) -> Option<Arc<FontVec>> {
        let weight = if bold {
            fontdb::Weight::BOLD
        } else {
            fontdb::Weight::NORMAL
        };
        let preferred: Vec<fontdb::Family> = if family.trim().is_empty() {
            Vec::new()
        } else {
            vec![
                fontdb::Family::Name(family),
                fontdb::Family::Name("DejaVu Sans"),
                fontdb::Family::SansSerif,
            ]
        };
        let families: Vec<fontdb::Family> = if preferred.is_empty() {
            vec![
                fontdb::Family::Name("DejaVu Sans"),
                fontdb::Family::SansSerif,
            ]
        } else {
            preferred
        };
        for family in families {
            let query = fontdb::Query {
                families: &[family],
                weight,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };
            if let Some(id) = self.db.query(&query) {
                if let Some(data) = self
                    .db
                    .with_face_data(id, |data, _index| data.to_vec())
                {
                    if let Ok(font) = FontVec::try_from_vec(data) {
                        return Some(Arc::new(font));
                    }
                }
            }
        }
        None
    }
}

/// The drawing surface for one frame, plus the fonts and profile settings.
pub struct Canvas<'a> {
    pub pixmap: Pixmap,
    pub fonts: &'a mut FontCache,
    pub font_family: String,
    pub font_scale: f32,
}

impl<'a> Canvas<'a> {
    pub fn new(
        fonts: &'a mut FontCache,
        font_family: &str,
        font_scale: f64,
        background: RgbColor,
    ) -> Self {
        let mut pixmap = Pixmap::new(SIZE, SIZE).expect("480x480 pixmap");
        pixmap.fill(Color::from_rgba8(background.r, background.g, background.b, 255));
        Self {
            pixmap,
            fonts,
            font_family: font_family.to_string(),
            font_scale: font_scale.clamp(0.5, 2.0) as f32,
        }
    }

    pub fn clear(&mut self, color: RgbColor) {
        self.pixmap.fill(self.color(color, 255));
    }

    fn color(&self, color: RgbColor, alpha: u8) -> Color {
        Color::from_rgba8(color.r, color.g, color.b, alpha)
    }

    fn paint(&self, color: RgbColor, alpha: u8) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color(self.color(color, alpha));
        paint.anti_alias = true;
        paint
    }

    fn stroke(width: f32) -> Stroke {
        Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        }
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: RgbColor, alpha: u8) {
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(x, y, width, height).expect("rect"));
        if let Some(path) = path.finish() {
            self.pixmap.fill_path(
                &path,
                &self.paint(color, alpha),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_round_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: RgbColor,
        alpha: u8,
    ) {
        if let Some(path) = round_rect_path(x, y, width, height, radius) {
            self.pixmap.fill_path(
                &path,
                &self.paint(color, alpha),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    pub fn fill_polygon(&mut self, points: &[(f32, f32)], color: RgbColor, alpha: u8) {
        if points.len() < 3 {
            return;
        }
        let mut path = PathBuilder::new();
        path.move_to(points[0].0, points[0].1);
        for point in &points[1..] {
            path.line_to(point.0, point.1);
        }
        path.close();
        if let Some(path) = path.finish() {
            self.pixmap.fill_path(
                &path,
                &self.paint(color, alpha),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    pub fn stroke_polygon(
        &mut self,
        points: &[(f32, f32)],
        width: f32,
        color: RgbColor,
        alpha: u8,
    ) {
        if points.len() < 2 {
            return;
        }
        let mut path = PathBuilder::new();
        path.move_to(points[0].0, points[0].1);
        for point in &points[1..] {
            path.line_to(point.0, point.1);
        }
        path.close();
        if let Some(path) = path.finish() {
            self.pixmap.stroke_path(
                &path,
                &self.paint(color, alpha),
                &Self::stroke(width),
                Transform::identity(),
                None,
            );
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: RgbColor, alpha: u8) {
        let mut path = PathBuilder::new();
        path.push_circle(cx, cy, radius);
        if let Some(path) = path.finish() {
            self.pixmap.fill_path(
                &path,
                &self.paint(color, alpha),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    pub fn stroke_circle(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        width: f32,
        color: RgbColor,
        alpha: u8,
    ) {
        let mut path = PathBuilder::new();
        path.push_circle(cx, cy, radius);
        if let Some(path) = path.finish() {
            self.pixmap.stroke_path(
                &path,
                &self.paint(color, alpha),
                &Self::stroke(width),
                Transform::identity(),
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: RgbColor, width: f32, alpha: u8) {
        let mut path = PathBuilder::new();
        path.move_to(x0, y0);
        path.line_to(x1, y1);
        if let Some(path) = path.finish() {
            self.pixmap.stroke_path(
                &path,
                &self.paint(color, alpha),
                &Self::stroke(width),
                Transform::identity(),
                None,
            );
        }
    }

    /// An arc from 135°, sweeping clockwise for `fraction` of 270°.
    pub fn arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        width: f32,
        fraction: Option<f64>,
        color: RgbColor,
    ) {
        self.arc_from(cx, cy, radius, width, 135.0, 270.0, None, color, 35);
        if let Some(fraction) = fraction {
            let fraction = fraction.clamp(0.0, 1.0);
            if fraction > 0.0 {
                self.arc_from(
                    cx,
                    cy,
                    radius,
                    width,
                    135.0,
                    270.0,
                    Some(fraction),
                    color,
                    255,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn arc_from(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        width: f32,
        start_degrees: f64,
        sweep_degrees: f64,
        fraction: Option<f64>,
        color: RgbColor,
        alpha: u8,
    ) {
        let sweep = sweep_degrees * fraction.unwrap_or(1.0);
        let steps = ((sweep.abs() / 4.0).ceil() as usize).max(8);
        let mut path = PathBuilder::new();
        for step in 0..=steps {
            let angle = (start_degrees + sweep * step as f64 / steps as f64).to_radians();
            let x = cx + radius * angle.sin() as f32;
            let y = cy - radius * angle.cos() as f32;
            if step == 0 {
                path.move_to(x, y);
            } else {
                path.line_to(x, y);
            }
        }
        if sweep.abs() >= 359.9 {
            path.close();
        }
        if let Some(path) = path.finish() {
            self.pixmap.stroke_path(
                &path,
                &self.paint(color, alpha),
                &Self::stroke(width),
                Transform::identity(),
                None,
            );
        }
    }

    /// A full 360° gauge when `full`, otherwise the 270° arc used by the themes.
    pub fn gauge(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        fraction: Option<f64>,
        color: RgbColor,
        full: bool,
    ) {
        if !full {
            self.arc(cx, cy, radius, (radius / 7.0).max(6.0), fraction, color);
            return;
        }
        let width = (radius / 7.0).max(6.0);
        self.stroke_circle(cx, cy, radius, width, color, 35);
        if let Some(fraction) = fraction {
            let fraction = fraction.clamp(0.0, 1.0);
            if fraction > 0.0 {
                self.arc_from(cx, cy, radius, width, -90.0, 360.0, Some(fraction), color, 255);
            }
        }
    }

    pub fn bar(&mut self, x: f32, y: f32, width: f32, height: f32, fraction: Option<f64>, color: RgbColor) {
        let radius = height / 2.0;
        self.fill_round_rect(x, y - radius, width, height, radius, color, 35);
        if let Some(fraction) = fraction {
            let fraction = fraction.clamp(0.0, 1.0);
            if fraction > 0.0 {
                self.fill_round_rect(
                    x,
                    y - radius,
                    (width * fraction as f32).max(height),
                    height,
                    radius,
                    color,
                    255,
                );
            }
        }
    }

    /// A value with a small caption underneath (one column of a stat row).
    pub fn stat(
        &mut self,
        cx: f32,
        y: f32,
        value: &str,
        caption: &str,
        value_color: RgbColor,
        caption_color: RgbColor,
    ) {
        self.text(value, cx, y, 32.0, value_color, Weight::Bold, 255, Align::Center, 118.0);
        self.text(caption, cx, y + 25.0, 16.0, caption_color, Weight::Bold, 255, Align::Center, 118.0);
    }

    pub fn bar_row(&mut self, label: &str, fraction: Option<f64>, value: &str, y: f32, color: RgbColor, foreground: RgbColor) {
        self.text(label, 92.0, y + 8.0, 22.0, color, Weight::Bold, 255, Align::Left, 0.0);
        self.bar(152.0, y, 178.0, 14.0, fraction, color);
        self.text(value, 390.0, y + 8.0, 22.0, foreground, Weight::Bold, 255, Align::Right, 0.0);
    }

    /// Draws text. Values are the same as the Windows renderer: `y` is the
    /// baseline unless the theme centres a value, and `max_width` shrinks the
    /// font until the line fits.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: RgbColor,
        weight: Weight,
        alpha: u8,
        align: Align,
        max_width: f32,
    ) {
        if text.is_empty() || alpha == 0 {
            return;
        }
        let Some(font) = self.fonts.font(&self.font_family, weight) else {
            return;
        };
        let mut size = size * self.font_scale;
        if size < 1.0 {
            return;
        }
        let mut scale = PxScale::from(size);
        let mut width = measure(&font, scale, text);
        let mut limit = if align == Align::Center {
            visible_width(x, y, size)
        } else {
            0.0
        };
        if max_width > 0.0 {
            limit = if limit > 0.0 {
                limit.min(max_width)
            } else {
                max_width
            };
        }
        if limit > 0.0 && width > limit && width > 0.0 {
            size *= limit / width;
            scale = PxScale::from(size);
            width = measure(&font, scale, text);
        }
        let start = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        let scaled = font.as_scaled(scale);
        let mut pen_x = start;
        for character in text.chars() {
            let glyph_id = font.glyph_id(character);
            if character != ' ' {
                let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(pen_x, y));
                if let Some(outlined) = font.outline_glyph(glyph) {
                    let bounds = outlined.px_bounds();
                    let origin_x = bounds.min.x.floor() as i32;
                    let origin_y = bounds.min.y.floor() as i32;
                    outlined.draw(|gx, gy, coverage| {
                        if coverage > 0.0 {
                            let px = origin_x + gx as i32;
                            let py = origin_y + gy as i32;
                            blend_pixel(
                                &mut self.pixmap,
                                px,
                                py,
                                color,
                                alpha as f32 / 255.0 * coverage,
                            );
                        }
                    });
                }
            }
            pen_x += scaled.h_advance(glyph_id);
        }
    }

    /// Convenience for the common centred form used by the themes.
    #[allow(clippy::too_many_arguments)]
    pub fn center(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: RgbColor,
        weight: Weight,
        alpha: u8,
        max_width: f32,
    ) {
        self.text(text, x, y, size, color, weight, alpha, Align::Center, max_width);
    }

    /// Paints everything outside the round panel black.
    pub fn mask_circle(&mut self) {
        self.stroke_circle(CENTER, CENTER, CENTER + 50.0, 100.0, RgbColor::BLACK, 255);
    }

    pub fn draw_media(&mut self, media: &Pixmap) {
        let paint = PixmapPaint::default();
        self.pixmap.draw_pixmap(
            0,
            0,
            media.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
    }
}

fn round_rect_path(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<tiny_skia::Path> {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let mut path = PathBuilder::new();
    path.move_to(x + radius, y);
    path.line_to(x + width - radius, y);
    path.quad_to(x + width, y, x + width, y + radius);
    path.line_to(x + width, y + height - radius);
    path.quad_to(x + width, y + height, x + width - radius, y + height);
    path.line_to(x + radius, y + height);
    path.quad_to(x, y + height, x, y + height - radius);
    path.line_to(x, y + radius);
    path.quad_to(x, y, x + radius, y);
    path.close();
    path.finish()
}

fn measure(font: &FontVec, scale: PxScale, text: &str) -> f32 {
    let scaled = font.as_scaled(scale);
    text.chars()
        .map(|character| scaled.h_advance(font.glyph_id(character)))
        .sum()
}

/// Width a line centred on `x` can have at baseline `y` without leaving the
/// round panel's safe area.
fn visible_width(x: f32, y: f32, font_size: f32) -> f32 {
    let dy = (y - CENTER)
        .abs()
        .max((y - font_size * 0.72 - CENTER).abs());
    if dy >= SAFE_RADIUS {
        return 1.0;
    }
    let half = (SAFE_RADIUS * SAFE_RADIUS - dy * dy).sqrt();
    (2.0 * (x - (CENTER - half)).min((CENTER + half) - x)).max(1.0)
}

fn blend_pixel(pixmap: &mut Pixmap, x: i32, y: i32, color: RgbColor, coverage: f32) {
    if coverage <= 0.0 {
        return;
    }
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    if x < 0 || y < 0 || x >= width || y >= height {
        return;
    }
    let coverage = coverage.clamp(0.0, 1.0);
    let index = ((y * width + x) * 4) as usize;
    let data = pixmap.data_mut();
    if index + 3 >= data.len() {
        return;
    }
    let inv = 1.0 - coverage;
    data[index] = ((color.r as f32 * coverage) + data[index] as f32 * inv).round() as u8;
    data[index + 1] = ((color.g as f32 * coverage) + data[index + 1] as f32 * inv).round() as u8;
    data[index + 2] = ((color.b as f32 * coverage) + data[index + 2] as f32 * inv).round() as u8;
    data[index + 3] = ((255.0 * coverage) + data[index + 3] as f32 * inv)
        .round()
        .min(255.0) as u8;
}

/// Applies the panel brightness and rotation to a preview frame. The real
/// panel receives these as commands, so this is intentionally preview-only.
pub fn preview_transform(pixmap: &Pixmap, brightness: i32, rotation: i32) -> Pixmap {
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    if width == 0 || height == 0 {
        return pixmap.clone();
    }
    let source = pixmap.data();
    let mut output = vec![0u8; source.len()];
    let turns = matches!(rotation, 90 | 180 | 270);
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = if turns {
                match rotation {
                    90 => (height - 1 - y, x),
                    180 => (width - 1 - x, height - 1 - y),
                    270 => (y, width - 1 - x),
                    _ => (x, y),
                }
            } else {
                (x, y)
            };
            let src = (y * width + x) * 4;
            let dst = (dy * width + dx) * 4;
            output[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
        }
    }
    let factor = brightness.clamp(0, 100) as u32;
    if factor < 100 {
        for pixel in output.chunks_exact_mut(4) {
            pixel[0] = ((pixel[0] as u32 * factor) / 100) as u8;
            pixel[1] = ((pixel[1] as u32 * factor) / 100) as u8;
            pixel[2] = ((pixel[2] as u32 * factor) / 100) as u8;
        }
    }
    Pixmap::from_vec(
        output,
        tiny_skia::IntSize::from_wh(width as u32, height as u32).expect("frame size"),
    )
    .expect("preview pixmap")
}

/// Encodes a frame as baseline JPEG, stepping the quality down until it fits
/// the panel's payload limit.
pub fn encode_jpeg(pixmap: &Pixmap, max_bytes: usize) -> Vec<u8> {
    let rgba = pixmap.data();
    let mut rgb = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for pixel in rgba.chunks_exact(4) {
        // tiny-skia stores premultiplied RGBA; undo that for the JPEG encoder.
        let alpha = pixel[3];
        if alpha == 0 {
            rgb.extend_from_slice(&[0, 0, 0]);
        } else if alpha == 255 {
            rgb.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]);
        } else {
            let restore = |value: u8| ((value as u32 * 255) / alpha as u32).min(255) as u8;
            rgb.extend_from_slice(&[restore(pixel[0]), restore(pixel[1]), restore(pixel[2])]);
        }
    }
    let image = image::RgbImage::from_raw(SIZE, SIZE, rgb).expect("480x480 RGB frame");
    let mut quality = 88i32;
    loop {
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut bytes,
            quality.clamp(1, 100) as u8,
        );
        if encoder
            .encode_image(&image::DynamicImage::ImageRgb8(image.clone()))
            .is_err()
        {
            return Vec::new();
        }
        if bytes.len() <= max_bytes || quality <= 28 {
            return bytes;
        }
        quality -= 10;
    }
}

/// Encodes a PNG preview for the UI.
pub fn encode_png(pixmap: &Pixmap) -> Vec<u8> {
    let rgba = pixmap.data();
    let mut data = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            data.extend_from_slice(&[0, 0, 0, 0]);
        } else if alpha == 255 {
            data.extend_from_slice(pixel);
        } else {
            let restore = |value: u8| ((value as u32 * 255) / alpha as u32).min(255) as u8;
            data.extend_from_slice(&[restore(pixel[0]), restore(pixel[1]), restore(pixel[2]), alpha]);
        }
    }
    let image = image::RgbaImage::from_raw(SIZE, SIZE, data).expect("480x480 RGBA frame");
    let mut bytes = Vec::new();
    if image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return Vec::new();
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_frames_encode_to_a_valid_baseline_jpeg() {
        let mut fonts = FontCache::new();
        let mut canvas = Canvas::new(&mut fonts, "DejaVu Sans", 1.0, RgbColor::BLACK);
        canvas.mask_circle();
        let jpeg = encode_jpeg(&canvas.pixmap, 150_000);
        assert!(jpeg.len() > 100);
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9]);
    }

    #[test]
    fn png_preview_is_a_valid_png() {
        let mut fonts = FontCache::new();
        let canvas = Canvas::new(&mut fonts, "DejaVu Sans", 1.0, RgbColor::BLACK);
        let png = encode_png(&canvas.pixmap);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn text_shrinks_to_the_round_panel() {
        let mut fonts = FontCache::new();
        let mut canvas = Canvas::new(&mut fonts, "DejaVu Sans", 1.0, RgbColor::BLACK);
        canvas.center(
            "a very long line that must be reduced",
            CENTER,
            40.0,
            120.0,
            RgbColor::WHITE,
            Weight::Bold,
            255,
            0.0,
        );
        assert!(canvas.pixmap.data().iter().any(|value| *value > 0));
    }

    #[test]
    fn preview_rotation_turns_a_square_clockwise() {
        let mut pixmap = Pixmap::new(2, 2).unwrap();
        let colors = [
            (0, 0, [255, 0, 0, 255]),
            (1, 0, [0, 255, 0, 255]),
            (0, 1, [0, 0, 255, 255]),
            (1, 1, [255, 255, 255, 255]),
        ];
        for (x, y, color) in colors {
            let index = ((y * 2 + x) * 4) as usize;
            pixmap.data_mut()[index..index + 4].copy_from_slice(&color);
        }
        let turned = preview_transform(&pixmap, 100, 90);
        let pixel = |x: usize, y: usize| {
            let index = (y * 2 + x) * 4;
            [
                turned.data()[index],
                turned.data()[index + 1],
                turned.data()[index + 2],
                turned.data()[index + 3],
            ]
        };
        assert_eq!(pixel(1, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(1, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(0, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn preview_brightness_dims_the_frame() {
        let mut pixmap = Pixmap::new(1, 1).unwrap();
        pixmap.data_mut()[..4].copy_from_slice(&[200, 100, 50, 255]);
        let dimmed = preview_transform(&pixmap, 50, 0);
        assert_eq!(&dimmed.data()[..4], &[100, 50, 25, 255]);
    }

    #[test]
    fn arcs_and_bars_do_not_panic_at_the_edges() {
        let mut fonts = FontCache::new();
        let mut canvas = Canvas::new(&mut fonts, "DejaVu Sans", 1.0, RgbColor::BLACK);
        canvas.arc(CENTER, CENTER, 200.0, 18.0, Some(3.0), RgbColor::WHITE);
        canvas.bar(0.0, 0.0, 10.0, 10.0, Some(-1.0), RgbColor::WHITE);
        canvas.gauge(CENTER, CENTER, 100.0, Some(0.5), RgbColor::WHITE, true);
    }
}
