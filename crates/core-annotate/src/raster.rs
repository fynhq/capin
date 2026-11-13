use std::cell::RefCell;

use core_types::{
    FrameError, PhysicalPoint, PhysicalRect, PhysicalSize, RgbaFrame, prefers_dark_foreground,
};
use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache,
};
use thiserror::Error;
use tiny_skia::{
    IntSize, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

use crate::{
    Annotation, AnnotationDocument, AnnotationId, AnnotationStyle, OutlinePoint, Shape,
    TextAlignment, highlighter_color, stroke_outline,
};

/// Label color for a sequence badge. Badges read as solid chips, so the
/// foreground only flips to dark on light fills; mid-tone colors such as the
/// default red keep white text instead of picking the slightly better contrast.
pub fn badge_foreground(color: u32) -> u32 {
    if core_types::relative_luminance(color & 0x00FF_FFFF) > 0.55 {
        0x111111FF
    } else {
        0xFFFFFFFF
    }
}

pub fn rasterize(
    document: &AnnotationDocument,
    frame: &RgbaFrame,
) -> Result<RgbaFrame, RasterError> {
    let expected = document.image_size();
    let actual = PhysicalSize::new(frame.width(), frame.height());
    if expected != actual {
        return Err(RasterError::SizeMismatch { expected, actual });
    }

    // Plain screenshots have no objects. Returning the frame directly avoids
    // building and premultiplying a full pixmap only to copy the pixels back.
    if document.objects().is_empty() {
        return Ok(frame.clone());
    }

    let mut pixels = frame.pixels().to_vec();
    premultiply(&mut pixels);
    let size = IntSize::from_wh(expected.width, expected.height).ok_or(RasterError::Pixmap)?;
    let mut pixmap = Pixmap::from_vec(pixels, size).ok_or(RasterError::Pixmap)?;

    for annotation in document.objects() {
        if let Shape::Blur(rect) = &annotation.shape {
            blur_region(
                pixmap.data_mut(),
                expected.width,
                expected.height,
                *rect,
                annotation.style.blur_radius,
                annotation.style.blur_feather,
            );
            continue;
        }
        if matches!(annotation.shape, Shape::Text { .. }) {
            draw_text(&mut pixmap, annotation);
            continue;
        }
        if matches!(annotation.shape, Shape::Number { .. }) {
            draw_number(&mut pixmap, annotation);
            continue;
        }
        if let Shape::Spotlight(rect) = annotation.shape {
            draw_spotlight(
                &mut pixmap,
                expected,
                rect,
                annotation.style.spotlight_intensity,
                annotation.style.spotlight_radius,
            );
            continue;
        }

        let color = if matches!(annotation.shape, Shape::HighlighterBox(_)) {
            highlighter_color(annotation.style.color)
        } else {
            annotation.style.color
        };
        let (red, green, blue, alpha) = unpack(color);
        let mut paint = Paint::default();
        paint.set_color_rgba8(red, green, blue, alpha);
        if let Some(polygons) = stroke_outline(&annotation.shape, &annotation.style) {
            for polygon in polygons {
                let Some(path) = polygon_path(&polygon) else {
                    continue;
                };
                pixmap.as_mut().fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
            continue;
        }

        let path = build_path(annotation)?;
        let mut target = pixmap.as_mut();
        if fills_shape(annotation) {
            target.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        } else {
            let stroke = Stroke {
                width: annotation.style.stroke_width.max(1.0),
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            };
            target.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    let mut pixels = pixmap.take();
    unpremultiply(&mut pixels);
    Ok(RgbaFrame::from_pixels(
        expected.width,
        expected.height,
        pixels,
    )?)
}

fn polygon_path(polygon: &[OutlinePoint]) -> Option<Path> {
    let first = polygon.first()?;
    if polygon.len() < 3 {
        return None;
    }
    let mut builder = PathBuilder::new();
    builder.move_to(first.x, first.y);
    for point in &polygon[1..] {
        builder.line_to(point.x, point.y);
    }
    builder.close();
    builder.finish()
}

fn fills_shape(annotation: &Annotation) -> bool {
    match &annotation.shape {
        Shape::HighlighterBox(_) => true,
        Shape::Rect(_) | Shape::Ellipse(_) => annotation.style.filled,
        _ => false,
    }
}

fn build_path(annotation: &Annotation) -> Result<Path, RasterError> {
    match &annotation.shape {
        Shape::Rect(rect) => rounded_rect_path(*rect, annotation.style.corner_radius),
        Shape::Ellipse(rect) => {
            let mut builder = PathBuilder::new();
            builder.push_oval(to_skia_rect(*rect)?);
            builder.finish().ok_or(RasterError::InvalidGeometry)
        }
        Shape::HighlighterBox(rect) => {
            rounded_rect_path(*rect, annotation.style.highlighter_radius)
        }
        Shape::Arrow { .. } | Shape::Polyline { .. } => {
            Err(RasterError::Unsupported(annotation.id))
        }
        Shape::Blur(_) => Err(RasterError::Unsupported(annotation.id)),
        Shape::Spotlight(_) => Err(RasterError::Unsupported(annotation.id)),
        Shape::Text { .. } => Err(RasterError::Unsupported(annotation.id)),
        Shape::Number { .. } => Err(RasterError::Unsupported(annotation.id)),
    }
}

/// Measures the rendered height of wrapped text using the same layout engine
/// as the exporter. The editor uses this for hit-testing and selection boxes so
/// CJK and long wrapped lines do not drift from the exported bitmap.
pub fn measure_text_height(text: &str, font_size: f32, width: u32) -> u32 {
    if text.is_empty() || width == 0 {
        return 0;
    }
    let font_size = font_size.max(1.0);
    TEXT_ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        let engine = engine.get_or_insert_with(|| TextEngine {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        });
        if engine.font_system.db_mut().is_empty() {
            return (font_size * 1.3).ceil() as u32;
        }
        let mut buffer = Buffer::new(
            &mut engine.font_system,
            Metrics::new(font_size, font_size * 1.3),
        );
        buffer.set_size(Some(width as f32), None);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        let mut height = font_size * 1.3;
        for run in buffer.layout_runs() {
            height = height.max(run.line_top + run.line_height);
        }
        height.ceil().max(font_size * 1.3) as u32
    })
}

fn draw_text(pixmap: &mut Pixmap, annotation: &Annotation) {
    let Shape::Text { bounds, text } = &annotation.shape else {
        return;
    };
    if text.is_empty() || bounds.size.width == 0 || bounds.size.height == 0 {
        return;
    }

    if annotation.style.text_background {
        draw_text_background(pixmap, *bounds, annotation.style);
    }

    TEXT_ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        let engine = engine.get_or_insert_with(|| TextEngine {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        });
        if engine.font_system.db_mut().is_empty() {
            return;
        }
        let font_size = annotation.style.font_size.max(1.0);
        let mut buffer = Buffer::new(
            &mut engine.font_system,
            Metrics::new(font_size, font_size * 1.3),
        );
        buffer.set_size(Some(bounds.size.width as f32), None);
        let alignment = match annotation.style.text_alignment {
            TextAlignment::Left => Align::Left,
            TextAlignment::Center => Align::Center,
            TextAlignment::Right => Align::Right,
        };
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            Some(alignment),
        );

        let (red, green, blue, alpha) = unpack(annotation.style.color);
        let rgb = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
        if annotation.style.text_outline {
            let outline = if prefers_dark_foreground(rgb) {
                (0, 0, 0, 255)
            } else {
                (255, 255, 255, 255)
            };
            let radius = (annotation.style.stroke_width * 0.6).max(1.0).round() as i32;
            for dx in [-radius, 0, radius] {
                for dy in [-radius, 0, radius] {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    draw_text_buffer(
                        pixmap,
                        &mut engine.font_system,
                        &mut engine.swash_cache,
                        &mut buffer,
                        bounds.origin,
                        outline,
                        (dx, dy),
                    );
                }
            }
        }
        draw_text_buffer(
            pixmap,
            &mut engine.font_system,
            &mut engine.swash_cache,
            &mut buffer,
            bounds.origin,
            (red, green, blue, alpha),
            (0, 0),
        );
    });
}

fn draw_text_buffer(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    buffer: &mut Buffer,
    origin: PhysicalPoint,
    color: (u8, u8, u8, u8),
    offset: (i32, i32),
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let data = pixmap.data_mut();
    buffer.draw(
        font_system,
        swash_cache,
        TextColor::rgba(color.0, color.1, color.2, color.3),
        |x, y, rect_width, rect_height, glyph| {
            let mask = f32::from((glyph.0 >> 24) as u8) / 255.0;
            let coverage = mask * (f32::from(color.3) / 255.0);
            if coverage <= 0.0 {
                return;
            }
            let source_red = f32::from(((glyph.0 >> 16) & 0xFF) as u8);
            let source_green = f32::from(((glyph.0 >> 8) & 0xFF) as u8);
            let source_blue = f32::from((glyph.0 & 0xFF) as u8);
            let inverse = 1.0 - coverage;
            for row in 0..rect_height as i32 {
                let pixel_y = origin.y + y + row + offset.1;
                if !(0..height).contains(&pixel_y) {
                    continue;
                }
                for column in 0..rect_width as i32 {
                    let pixel_x = origin.x + x + column + offset.0;
                    if !(0..width).contains(&pixel_x) {
                        continue;
                    }
                    let index = ((pixel_y * width + pixel_x) * 4) as usize;
                    data[index] = blend_channel(source_red, coverage, data[index], inverse);
                    data[index + 1] =
                        blend_channel(source_green, coverage, data[index + 1], inverse);
                    data[index + 2] =
                        blend_channel(source_blue, coverage, data[index + 2], inverse);
                    data[index + 3] = blend_channel(255.0, coverage, data[index + 3], inverse);
                }
            }
        },
    );
}

fn draw_text_background(pixmap: &mut Pixmap, rect: PhysicalRect, style: AnnotationStyle) {
    let (red, green, blue, _) = unpack(style.color);
    let rgb = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
    let color = if prefers_dark_foreground(rgb) {
        (0, 0, 0, 160)
    } else {
        (255, 255, 255, 190)
    };
    let Ok(path) = rounded_rect_path(rect, style.font_size * 0.2) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, color.3);
    pixmap.as_mut().fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn blend_channel(source: f32, coverage: f32, destination: u8, inverse: f32) -> u8 {
    (source * coverage + f32::from(destination) * inverse)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn rounded_rect_path(rect: PhysicalRect, radius: f32) -> Result<Path, RasterError> {
    let mut builder = PathBuilder::new();
    push_rounded_rect(&mut builder, rect, radius)?;
    builder.finish().ok_or(RasterError::InvalidGeometry)
}

/// Appends one closed rounded rectangle to `builder`. The radius is clamped to
/// half of the shorter side so extreme corner values still produce a valid
/// path.
fn push_rounded_rect(
    builder: &mut PathBuilder,
    rect: PhysicalRect,
    radius: f32,
) -> Result<(), RasterError> {
    let rect = to_skia_rect(rect)?;
    let radius = radius.max(0.0).min(rect.width().min(rect.height()) / 2.0);
    if radius < 0.5 {
        builder.push_rect(rect);
        return Ok(());
    }
    let left = rect.left();
    let top = rect.top();
    let right = rect.right();
    let bottom = rect.bottom();
    builder.move_to(left + radius, top);
    builder.line_to(right - radius, top);
    builder.quad_to(right, top, right, top + radius);
    builder.line_to(right, bottom - radius);
    builder.quad_to(right, bottom, right - radius, bottom);
    builder.line_to(left + radius, bottom);
    builder.quad_to(left, bottom, left, bottom - radius);
    builder.line_to(left, top + radius);
    builder.quad_to(left, top, left + radius, top);
    builder.close();
    Ok(())
}

#[cfg(test)]
mod badge_tests {
    use super::badge_foreground;

    #[test]
    fn sequence_badges_default_to_white_on_mid_tones() {
        assert_eq!(badge_foreground(0xFF3B30), 0xFFFFFFFF);
        assert_eq!(badge_foreground(0x0067C0), 0xFFFFFFFF);
    }

    #[test]
    fn sequence_badges_flip_to_dark_only_on_light_fills() {
        assert_eq!(badge_foreground(0xFFD60A), 0x111111FF);
        assert_eq!(badge_foreground(0xFFFFFF), 0x111111FF);
    }
}

fn draw_number(pixmap: &mut Pixmap, annotation: &Annotation) {
    let Shape::Number { bounds, value } = &annotation.shape else {
        return;
    };
    if bounds.size.width < 2 || bounds.size.height < 2 {
        return;
    }
    let (red, green, blue, alpha) = unpack(annotation.style.color);
    let rgb = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
    let foreground = badge_foreground(rgb);

    let background = to_skia_rect(*bounds).ok().and_then(|rect| {
        let mut builder = PathBuilder::new();
        builder.push_oval(rect);
        builder.finish()
    });
    if let Some(path) = background {
        let mut paint = Paint::default();
        paint.set_color_rgba8(red, green, blue, alpha);
        pixmap.as_mut().fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    let font_size = (bounds.size.height as f32 * 0.54).max(10.0);
    let text = value.to_string();
    let estimated_width = (font_size * 0.62 * text.chars().count() as f32).max(font_size);
    let text_width = estimated_width.ceil() as u32 + 2;
    // Center the padded glyph box on the badge and draw the text centered
    // inside it. Left-in-box alignment pushed narrow digits such as "1" a few
    // pixels off the circle centre in the exported image.
    let text_bounds = PhysicalRect::new(
        bounds.origin.x + ((bounds.size.width as f32 - text_width as f32) / 2.0).round() as i32,
        bounds.origin.y + ((bounds.size.height as f32 - font_size * 1.3) / 2.0).round() as i32,
        text_width,
        (font_size * 1.3).ceil() as u32,
    );
    let text_annotation = Annotation {
        id: annotation.id,
        shape: Shape::Text {
            bounds: text_bounds,
            text,
        },
        style: AnnotationStyle {
            color: foreground,
            stroke_width: 0.0,
            font_size,
            text_alignment: TextAlignment::Center,
            ..Default::default()
        },
    };
    draw_text(pixmap, &text_annotation);
}

fn draw_spotlight(
    pixmap: &mut Pixmap,
    size: PhysicalSize,
    rect: PhysicalRect,
    intensity: u8,
    radius: f32,
) {
    let image_width = size.width as i64;
    let image_height = size.height as i64;
    let left = i64::from(rect.origin.x).clamp(0, image_width);
    let top = i64::from(rect.origin.y).clamp(0, image_height);
    let right = rect.right().clamp(0, image_width);
    let bottom = rect.bottom().clamp(0, image_height);
    if right <= left || bottom <= top {
        return;
    }
    let hole = PhysicalRect::new(
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    );

    let Some(canvas) = Rect::from_xywh(0.0, 0.0, image_width as f32, image_height as f32) else {
        return;
    };
    let mut builder = PathBuilder::new();
    builder.push_rect(canvas);
    if push_rounded_rect(&mut builder, hole, radius).is_err() {
        return;
    }
    let Some(path) = builder.finish() else {
        return;
    };

    let alpha = (u32::from(intensity.min(100)) * 255 / 100) as u8;
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, alpha);
    pixmap.as_mut().fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::EvenOdd,
        Transform::identity(),
        None,
    );
}

thread_local! {
    static TEXT_ENGINE: RefCell<Option<TextEngine>> = const { RefCell::new(None) };
}

struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

fn blur_region(
    data: &mut [u8],
    width: u32,
    height: u32,
    rect: PhysicalRect,
    radius: u32,
    feather: u32,
) {
    if rect.is_empty() {
        return;
    }
    let x0 = rect.origin.x.max(0) as usize;
    let y0 = rect.origin.y.max(0) as usize;
    let x1 = rect.right().clamp(0, i64::from(width)) as usize;
    let y1 = rect.bottom().clamp(0, i64::from(height)) as usize;
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    let image_width = width as usize;
    let region_width = x1 - x0;
    let region_height = y1 - y0;
    let radius = (radius as usize)
        .max(1)
        .min(region_width.min(region_height) / 2)
        .max(1);
    let mut blurred = Vec::with_capacity(region_width * region_height * 4);
    for row in 0..region_height {
        let start = ((y0 + row) * image_width + x0) * 4;
        blurred.extend_from_slice(&data[start..start + region_width * 4]);
    }
    let mut scratch = vec![0u8; region_width * region_height * 4];

    // Three box passes are a close, separable approximation of a Gaussian blur.
    for _ in 0..3 {
        box_blur_horizontal(&blurred, &mut scratch, region_width, region_height, radius);
        box_blur_vertical(&scratch, &mut blurred, region_width, region_height, radius);
    }

    let feather = (feather as usize).min(region_width.min(region_height) / 2);
    for row in 0..region_height {
        for column in 0..region_width {
            let target = ((y0 + row) * image_width + x0 + column) * 4;
            let source = (row * region_width + column) * 4;
            let alpha = if feather == 0 {
                1.0
            } else {
                let distance = column
                    .min(region_width - 1 - column)
                    .min(row.min(region_height - 1 - row)) as f32;
                let t = (distance / feather as f32).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            if alpha >= 1.0 {
                data[target..target + 4].copy_from_slice(&blurred[source..source + 4]);
            } else if alpha > 0.0 {
                for channel in 0..4 {
                    let original = f32::from(data[target + channel]);
                    let mixed = f32::from(blurred[source + channel]);
                    data[target + channel] = (original + (mixed - original) * alpha).round() as u8;
                }
            }
        }
    }
}

fn box_blur_horizontal(
    source: &[u8],
    target: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
) {
    for row in 0..height {
        let source_row = row * width;
        let mut sums = [0u32; 4];
        for column in 0..=radius.min(width - 1) {
            let index = (source_row + column) * 4;
            for channel in 0..4 {
                sums[channel] += u32::from(source[index + channel]);
            }
        }
        for column in 0..width {
            if column > 0 {
                let remove = column as isize - radius as isize - 1;
                if remove >= 0 {
                    let index = (source_row + remove as usize) * 4;
                    for channel in 0..4 {
                        sums[channel] -= u32::from(source[index + channel]);
                    }
                }
                let add = column + radius;
                if add < width {
                    let index = (source_row + add) * 4;
                    for channel in 0..4 {
                        sums[channel] += u32::from(source[index + channel]);
                    }
                }
            }
            let low = column.saturating_sub(radius);
            let high = (column + radius).min(width - 1);
            let count = (high - low + 1) as u32;
            let target_index = (row * width + column) * 4;
            for channel in 0..4 {
                target[target_index + channel] = (sums[channel] / count) as u8;
            }
        }
    }
}

fn box_blur_vertical(source: &[u8], target: &mut [u8], width: usize, height: usize, radius: usize) {
    for column in 0..width {
        let mut sums = [0u32; 4];
        for row in 0..=radius.min(height - 1) {
            let index = (row * width + column) * 4;
            for channel in 0..4 {
                sums[channel] += u32::from(source[index + channel]);
            }
        }
        for row in 0..height {
            if row > 0 {
                let remove = row as isize - radius as isize - 1;
                if remove >= 0 {
                    let index = (remove as usize * width + column) * 4;
                    for channel in 0..4 {
                        sums[channel] -= u32::from(source[index + channel]);
                    }
                }
                let add = row + radius;
                if add < height {
                    let index = (add * width + column) * 4;
                    for channel in 0..4 {
                        sums[channel] += u32::from(source[index + channel]);
                    }
                }
            }
            let low = row.saturating_sub(radius);
            let high = (row + radius).min(height - 1);
            let count = (high - low + 1) as u32;
            let target_index = (row * width + column) * 4;
            for channel in 0..4 {
                target[target_index + channel] = (sums[channel] / count) as u8;
            }
        }
    }
}

fn to_skia_rect(rect: PhysicalRect) -> Result<Rect, RasterError> {
    Rect::from_xywh(
        rect.origin.x as f32,
        rect.origin.y as f32,
        rect.size.width as f32,
        rect.size.height as f32,
    )
    .ok_or(RasterError::InvalidGeometry)
}

fn unpack(color: u32) -> (u8, u8, u8, u8) {
    (
        ((color >> 24) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

fn premultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel.fill(0);
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
}

fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum RasterError {
    #[error("frame size mismatch: expected {expected:?}, got {actual:?}")]
    SizeMismatch {
        expected: PhysicalSize,
        actual: PhysicalSize,
    },
    #[error("annotation {0:?} cannot be rasterized yet")]
    Unsupported(AnnotationId),
    #[error("annotation geometry cannot be rasterized")]
    InvalidGeometry,
    #[error("failed to allocate a raster pixmap")]
    Pixmap,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[cfg(test)]
mod tests {
    use core_types::PhysicalPoint;

    use super::*;
    use crate::{AnnotationDocument, AnnotationStyle, Shape};

    fn black_frame(width: u32, height: u32) -> RgbaFrame {
        RgbaFrame::from_pixels(
            width,
            height,
            [0, 0, 0, 255].repeat((width * height) as usize),
        )
        .unwrap()
    }

    fn pixel(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 4] {
        frame.pixel(x, y).unwrap()
    }

    fn document(width: u32, height: u32) -> AnnotationDocument {
        AnnotationDocument::new(PhysicalSize::new(width, height))
    }

    #[test]
    fn no_annotations_preserves_pixels() {
        let frame = RgbaFrame::from_pixels(4, 2, (0..32).collect()).unwrap();
        let document = document(4, 2);
        let output = rasterize(&document, &frame).unwrap();
        assert_eq!(output.pixels(), frame.pixels());
    }

    #[test]
    fn rectangle_outline_is_drawn() {
        let frame = black_frame(100, 100);
        let mut document = document(100, 100);
        document.add(
            Shape::Rect(PhysicalRect::new(10, 10, 80, 80)),
            AnnotationStyle {
                color: 0xFF0000FF,
                stroke_width: 4.0,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        let border = pixel(&output, 10, 50);
        assert!(border[0] > 120, "border should be red: {border:?}");
        assert_eq!(pixel(&output, 50, 50), [0, 0, 0, 255]);
    }

    #[test]
    fn filled_rectangle_covers_the_interior() {
        let frame = black_frame(100, 100);
        let mut document = document(100, 100);
        document.add(
            Shape::Rect(PhysicalRect::new(10, 10, 80, 80)),
            AnnotationStyle {
                color: 0xFF0000FF,
                filled: true,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        let center = pixel(&output, 50, 50);
        assert!(
            center[0] > 200,
            "filled rectangle should be red: {center:?}"
        );
        assert_eq!(pixel(&output, 2, 2), [0, 0, 0, 255]);
    }

    #[test]
    fn filled_ellipse_covers_the_center() {
        let frame = black_frame(80, 80);
        let mut document = document(80, 80);
        document.add(
            Shape::Ellipse(PhysicalRect::new(10, 10, 60, 60)),
            AnnotationStyle {
                color: 0x00FF00FF,
                filled: true,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        let center = pixel(&output, 40, 40);
        assert!(
            center[1] > 200,
            "filled ellipse should be green: {center:?}"
        );
        assert_eq!(pixel(&output, 12, 12), [0, 0, 0, 255]);
    }

    #[test]
    fn rectangle_corner_radius_keeps_the_corner_clear() {
        let frame = black_frame(100, 100);
        let mut document = document(100, 100);
        document.add(
            Shape::Rect(PhysicalRect::new(10, 10, 80, 80)),
            AnnotationStyle {
                color: 0xFF0000FF,
                stroke_width: 4.0,
                corner_radius: 30.0,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        assert_eq!(pixel(&output, 10, 10), [0, 0, 0, 255]);
        assert!(pixel(&output, 50, 10)[0] > 120);
    }

    #[test]
    fn blur_radius_controls_the_spread() {
        let width = 100;
        let height = 80;
        let mut pixels = [0, 0, 0, 255].repeat((width * height) as usize);
        for y in 30..50 {
            for x in 40..60 {
                let index = (y * width + x) as usize * 4;
                pixels[index..index + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let frame = RgbaFrame::from_pixels(width, height, pixels).unwrap();

        let blurred_sample = |radius: u32| {
            let mut document = document(width, height);
            document.add(
                Shape::Blur(PhysicalRect::new(10, 10, 80, 60)),
                AnnotationStyle {
                    blur_radius: radius,
                    ..Default::default()
                },
            );
            let output = rasterize(&document, &frame).unwrap();
            pixel(&output, 80, 40)[0]
        };

        assert!(
            blurred_sample(4) < blurred_sample(32),
            "stronger blur should spread the source block further"
        );
    }

    #[test]
    fn blur_feather_softens_the_region_edge() {
        let width = 40;
        let height = 40;
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _y in 0..height {
            for x in 0..width {
                let value = if x < width / 2 { 255 } else { 0 };
                pixels.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let frame = RgbaFrame::from_pixels(width, height, pixels).unwrap();

        let sample = |feather: u32| {
            let mut document = document(width, height);
            document.add(
                Shape::Blur(PhysicalRect::new(0, 0, width, height)),
                AnnotationStyle {
                    blur_radius: 8,
                    blur_feather: feather,
                    ..Default::default()
                },
            );
            let output = rasterize(&document, &frame).unwrap();
            (pixel(&output, 0, 20)[0], pixel(&output, 20, 20)[0])
        };

        let (hard_edge, hard_center) = sample(0);
        let (feathered_edge, feathered_center) = sample(12);
        assert!(
            hard_edge < 255,
            "a hard-edged blur must change the boundary pixel"
        );
        assert_eq!(
            feathered_edge, 255,
            "the feather band must leave the outer edge untouched"
        );
        assert!(
            feathered_center < 255 && hard_center < 255,
            "the blur center must stay blurred with a feather band"
        );
    }

    #[test]
    fn polyline_arrow_renders_without_error() {
        let frame = black_frame(100, 100);
        let mut document = document(100, 100);
        document.add(
            Shape::Polyline {
                points: vec![
                    PhysicalPoint::new(10, 80),
                    PhysicalPoint::new(40, 30),
                    PhysicalPoint::new(70, 70),
                    PhysicalPoint::new(90, 20),
                ],
            },
            AnnotationStyle {
                arrow_start: true,
                arrow_end: true,
                ..Default::default()
            },
        );
        assert!(rasterize(&document, &frame).is_ok());
    }

    #[test]
    fn arrow_heads_can_be_rendered_at_both_ends() {
        let frame = black_frame(80, 80);
        let mut document = document(80, 80);
        document.add(
            Shape::Arrow {
                start: PhysicalPoint::new(10, 40),
                end: PhysicalPoint::new(70, 40),
            },
            AnnotationStyle {
                arrow_start: true,
                arrow_end: true,
                ..Default::default()
            },
        );
        assert!(rasterize(&document, &frame).is_ok());
    }

    #[test]
    fn ellipse_and_arrow_render_without_error() {
        let frame = black_frame(80, 80);
        let mut document = document(80, 80);
        document.add(
            Shape::Ellipse(PhysicalRect::new(10, 10, 50, 50)),
            AnnotationStyle::default(),
        );
        document.add(
            Shape::Arrow {
                start: PhysicalPoint::new(10, 70),
                end: PhysicalPoint::new(70, 20),
            },
            AnnotationStyle::default(),
        );
        let output = rasterize(&document, &frame).unwrap();
        assert_eq!(output.width(), 80);
    }

    #[test]
    fn highlighter_box_fills_with_translucent_color() {
        let frame = black_frame(60, 40);
        let mut document = document(60, 40);
        document.add(
            Shape::HighlighterBox(PhysicalRect::new(10, 10, 40, 20)),
            AnnotationStyle {
                color: 0xFFFF0080,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        let center = pixel(&output, 30, 20);
        assert!(
            center[0] > 80 && center[1] > 80 && center[3] == 255,
            "highlighter box should tint the background: {center:?}"
        );
        assert_eq!(
            center[2], 0,
            "the highlighter must not mix white into its color: {center:?}"
        );
        assert_eq!(pixel(&output, 2, 2), [0, 0, 0, 255]);
    }

    #[test]
    fn highlighter_box_honors_its_corner_radius() {
        let frame = black_frame(60, 40);
        let mut document = document(60, 40);
        document.add(
            Shape::HighlighterBox(PhysicalRect::new(10, 10, 40, 20)),
            AnnotationStyle {
                color: 0xFFFF0080,
                highlighter_radius: 10.0,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        assert!(
            pixel(&output, 30, 20)[0] > 80,
            "the wash center must keep its color"
        );
        assert_eq!(
            pixel(&output, 11, 11),
            [0, 0, 0, 255],
            "a rounded corner leaves the top-left pixel untouched"
        );
    }

    #[test]
    fn size_mismatch_is_rejected() {
        let frame = black_frame(20, 20);
        let document = document(30, 20);
        assert!(matches!(
            rasterize(&document, &frame),
            Err(RasterError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn blur_region_blends_pixels() {
        let width = 30;
        let height = 20;
        let mut pixels = [0, 0, 0, 255].repeat((width * height) as usize);
        let center = (10 * width + 10) as usize * 4;
        pixels[center..center + 4].copy_from_slice(&[255, 0, 0, 255]);
        let frame = RgbaFrame::from_pixels(width, height, pixels).unwrap();

        let mut document = document(width, height);
        document.add(
            Shape::Blur(PhysicalRect::new(5, 5, 20, 10)),
            AnnotationStyle::default(),
        );
        let output = rasterize(&document, &frame).unwrap();
        let blurred = pixel(&output, 10, 10);
        assert!(
            blurred[0] > 0 && blurred[0] < 255,
            "pixel was not blurred: {blurred:?}"
        );
        assert_eq!(pixel(&output, 0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn empty_text_is_a_noop() {
        let frame = black_frame(20, 20);
        let mut document = document(20, 20);
        document.add(
            Shape::Text {
                bounds: PhysicalRect::new(0, 0, 10, 10),
                text: String::new(),
            },
            AnnotationStyle::default(),
        );
        let output = rasterize(&document, &frame).unwrap();
        assert_eq!(pixel(&output, 0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn text_is_rasterized_without_error() {
        let frame = black_frame(120, 60);
        let mut document = document(120, 60);
        document.add(
            Shape::Text {
                bounds: PhysicalRect::new(4, 4, 110, 40),
                text: "Capin text".to_owned(),
            },
            AnnotationStyle::default(),
        );
        let output = rasterize(&document, &frame).unwrap();
        assert_eq!(output.width(), 120);
    }

    #[test]
    fn text_styles_render_without_error() {
        let frame = black_frame(120, 60);
        let mut document = document(120, 60);
        document.add(
            Shape::Text {
                bounds: PhysicalRect::new(4, 4, 110, 40),
                text: "Capin text".to_owned(),
            },
            AnnotationStyle {
                text_background: true,
                text_outline: true,
                text_alignment: TextAlignment::Center,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        assert_eq!(output.width(), 120);
    }

    #[test]
    fn number_badge_paints_its_circle() {
        let frame = black_frame(80, 80);
        let mut document = document(80, 80);
        document.add(
            Shape::Number {
                bounds: PhysicalRect::new(20, 20, 36, 36),
                value: 1,
            },
            AnnotationStyle {
                color: 0xFF0000FF,
                ..Default::default()
            },
        );
        let output = rasterize(&document, &frame).unwrap();
        let center = pixel(&output, 38, 38);
        assert!(center[0] > 120, "badge should be red: {center:?}");
        assert_eq!(pixel(&output, 2, 2), [0, 0, 0, 255]);
    }

    #[test]
    fn spotlight_dims_outside_and_keeps_the_hole_clear() {
        let frame = RgbaFrame::from_pixels(60, 60, [255, 255, 255, 255].repeat(60 * 60)).unwrap();
        let mut document = document(60, 60);
        document.add(
            Shape::Spotlight(PhysicalRect::new(20, 20, 20, 20)),
            AnnotationStyle::default(),
        );
        let output = rasterize(&document, &frame).unwrap();
        let outside = pixel(&output, 5, 5);
        assert!(
            outside[0] < 180,
            "spotlight should dim the outside: {outside:?}"
        );
        assert_eq!(pixel(&output, 30, 30), [255, 255, 255, 255]);
    }

    #[test]
    fn spotlight_radius_rounds_the_hole_corners() {
        let frame = RgbaFrame::from_pixels(60, 60, [255, 255, 255, 255].repeat(60 * 60)).unwrap();
        let mut rounded = document(60, 60);
        rounded.add(
            Shape::Spotlight(PhysicalRect::new(20, 20, 20, 20)),
            AnnotationStyle {
                spotlight_intensity: 100,
                spotlight_radius: 8.0,
                ..Default::default()
            },
        );
        let output = rasterize(&rounded, &frame).unwrap();
        // The sharp hole corner is dimmed once the hole gets a radius, while
        // the centre stays fully clear.
        assert!(pixel(&output, 21, 21)[0] < 100);
        assert_eq!(pixel(&output, 30, 30), [255, 255, 255, 255]);
        let mut square = document(60, 60);
        square.add(
            Shape::Spotlight(PhysicalRect::new(20, 20, 20, 20)),
            AnnotationStyle {
                spotlight_intensity: 100,
                ..Default::default()
            },
        );
        let output = rasterize(&square, &frame).unwrap();
        assert_eq!(pixel(&output, 21, 21), [255, 255, 255, 255]);
    }

    #[test]
    fn spotlight_intensity_controls_the_dim_level() {
        let frame = RgbaFrame::from_pixels(60, 60, [255, 255, 255, 255].repeat(60 * 60)).unwrap();
        let outside_for = |intensity: u8| {
            let mut document = document(60, 60);
            document.add(
                Shape::Spotlight(PhysicalRect::new(20, 20, 20, 20)),
                AnnotationStyle {
                    spotlight_intensity: intensity,
                    ..Default::default()
                },
            );
            let output = rasterize(&document, &frame).unwrap();
            pixel(&output, 5, 5)[0]
        };

        assert!(
            outside_for(20) > outside_for(80),
            "higher intensity should dim the outside more"
        );
    }
}
