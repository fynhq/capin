//! Window screenshot composition: rounded corners, soft shadows, and a
//! configurable background canvas.
//!
//! The input is a raw window frame in non-premultiplied RGBA. The output keeps
//! alpha so callers can save a transparent PNG or paste the image directly.

use std::sync::Arc;

use core_types::RgbaFrame;
use image::imageops::FilterType;
use thiserror::Error;

const MAX_PADDING: u32 = 2048;
const MAX_BLUR_RADIUS: u32 = 512;
const BLUR_PASSES: usize = 3;

/// A soft shadow placed behind the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// Vertical offset in physical pixels. Positive values move the shadow down.
    pub offset_y: i32,
    /// Blur radius in physical pixels.
    pub blur_radius: u32,
    /// Final opacity multiplier, clamped to `0.0..=1.0`.
    pub opacity: f32,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            offset_y: 12,
            blur_radius: 32,
            opacity: 0.35,
        }
    }
}

/// A border painted inside the window edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// Border width in physical pixels.
    pub width: u32,
    /// Non-premultiplied border color.
    pub color: [u8; 4],
}

/// The background painted behind the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    /// Leave the canvas transparent, producing a transparent PNG.
    Transparent,
    /// Paint every canvas pixel with this non-premultiplied RGBA color.
    Solid([u8; 4]),
    /// A vertical gradient between two non-premultiplied RGBA colors.
    VerticalGradient([u8; 4], [u8; 4]),
    /// A wallpaper image, scaled and center-cropped to cover the canvas.
    ///
    /// Shared so repeated window captures can reuse one decoded wallpaper
    /// without copying the full image into every canvas.
    Image(Arc<RgbaFrame>),
}

/// Layout and paint options for a window capture.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowCanvas {
    /// Transparent margin around the window, in physical pixels. The shadow is
    /// clipped to this margin.
    pub padding: u32,
    /// Corner radius in physical pixels. Use `0` for square corners.
    pub corner_radius: u32,
    pub shadow: Option<Shadow>,
    pub background: Background,
    pub border: Option<Border>,
}

impl Default for WindowCanvas {
    fn default() -> Self {
        Self {
            padding: 48,
            corner_radius: 12,
            shadow: Some(Shadow::default()),
            background: Background::Transparent,
            border: None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ComposeError {
    #[error("window frame is empty")]
    EmptyWindow,
    #[error("composed canvas is too large")]
    SizeOverflow,
    #[error("background image is empty")]
    EmptyBackground,
}

/// Composites `window` onto a new canvas according to `canvas`.
///
/// The returned frame is larger than the input by `canvas.padding` on every
/// side. Alpha is preserved for transparent backgrounds and outside rounded
/// corners.
pub fn compose_window(
    window: &RgbaFrame,
    canvas: &WindowCanvas,
) -> Result<RgbaFrame, ComposeError> {
    if window.width() == 0 || window.height() == 0 {
        return Err(ComposeError::EmptyWindow);
    }

    let padding = canvas.padding.min(MAX_PADDING);
    let doubled = padding.checked_mul(2).ok_or(ComposeError::SizeOverflow)?;
    let width = window
        .width()
        .checked_add(doubled)
        .ok_or(ComposeError::SizeOverflow)?;
    let height = window
        .height()
        .checked_add(doubled)
        .ok_or(ComposeError::SizeOverflow)?;
    let buffer_len = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ComposeError::SizeOverflow)?;

    let mut pixels = vec![0u8; buffer_len];
    paint_background(&mut pixels, width, height, &canvas.background)?;

    if let Some(shadow) = &canvas.shadow {
        paint_shadow(
            &mut pixels,
            CanvasLayout {
                width,
                height,
                padding,
                window_width: window.width(),
                window_height: window.height(),
                corner_radius: canvas.corner_radius,
            },
            shadow,
        );
    }

    paint_window(&mut pixels, width, padding, window, canvas.corner_radius);
    if let Some(border) = &canvas.border {
        paint_border(
            &mut pixels,
            width,
            padding,
            window,
            canvas.corner_radius,
            border,
        );
    }

    RgbaFrame::from_pixels(width, height, pixels).map_err(|_| ComposeError::SizeOverflow)
}

fn paint_background(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    background: &Background,
) -> Result<(), ComposeError> {
    match background {
        Background::Transparent => {}
        Background::Solid(color) => {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                *pixel = *color;
            }
        }
        Background::VerticalGradient(top, bottom) => {
            for y in 0..height {
                let t = if height > 1 {
                    y as f32 / (height - 1) as f32
                } else {
                    0.0
                };
                let color = lerp_color(*top, *bottom, t);
                let row = y as usize * width as usize * 4;
                for x in 0..width as usize {
                    let offset = row + x * 4;
                    pixels[offset..offset + 4].copy_from_slice(&color);
                }
            }
        }
        Background::Image(frame) => paint_background_image(pixels, width, height, frame)?,
    }
    Ok(())
}

fn paint_background_image(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    frame: &RgbaFrame,
) -> Result<(), ComposeError> {
    if frame.width() == 0 || frame.height() == 0 {
        return Err(ComposeError::EmptyBackground);
    }

    let mut raw = Vec::with_capacity(frame.width() as usize * frame.height() as usize * 4);
    for y in 0..frame.height() {
        let row = frame.row(y).ok_or(ComposeError::EmptyBackground)?;
        let end = frame.width() as usize * 4;
        if row.len() < end {
            return Err(ComposeError::EmptyBackground);
        }
        raw.extend_from_slice(&row[..end]);
    }
    let image = image::RgbaImage::from_raw(frame.width(), frame.height(), raw)
        .ok_or(ComposeError::EmptyBackground)?;

    // Crop to the target aspect ratio before scaling. Resizing a very wide or
    // tall wallpaper directly to "cover" can allocate an enormous temporary
    // image (a 1x10000 source would be scaled to tens of millions of pixels).
    let target_aspect = f64::from(width) / f64::from(height);
    let source_width = f64::from(frame.width());
    let source_height = f64::from(frame.height());
    let source_aspect = source_width / source_height;
    let (crop_width, crop_height) = if source_aspect > target_aspect {
        let crop_width = (source_height * target_aspect).round().max(1.0) as u32;
        (crop_width.min(frame.width()), frame.height())
    } else {
        let crop_height = (source_width / target_aspect).round().max(1.0) as u32;
        (frame.width(), crop_height.min(frame.height()))
    };
    let crop_x = (frame.width().saturating_sub(crop_width)) / 2;
    let crop_y = (frame.height().saturating_sub(crop_height)) / 2;
    let cropped = image::imageops::crop_imm(&image, crop_x, crop_y, crop_width, crop_height);
    let resized = image::imageops::resize(&cropped.to_image(), width, height, FilterType::Triangle);

    for y in 0..height as usize {
        let start = y * width as usize * 4;
        let end = start + width as usize * 4;
        let source = resized
            .as_raw()
            .get(start..end)
            .ok_or(ComposeError::SizeOverflow)?;
        pixels[start..end].copy_from_slice(source);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct CanvasLayout {
    width: u32,
    height: u32,
    padding: u32,
    window_width: u32,
    window_height: u32,
    corner_radius: u32,
}

fn paint_shadow(pixels: &mut [u8], layout: CanvasLayout, shadow: &Shadow) {
    let opacity = shadow.opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 || layout.padding == 0 {
        return;
    }

    // Three box passes spread the shadow roughly `3 * radius` pixels beyond
    // the window. Clamp the radius to the padding on the tighter side (the
    // vertical offset eats into one edge) so the canvas never cuts a partially
    // transparent shadow into a hard line.
    let available = layout
        .padding
        .saturating_sub(shadow.offset_y.unsigned_abs());
    let max_radius = available / BLUR_PASSES as u32;
    if max_radius == 0 {
        return;
    }
    let blur_radius = shadow.blur_radius.min(max_radius);

    let origin_x = layout.padding as i64;
    let origin_y = layout.padding as i64 + i64::from(shadow.offset_y);
    let left = origin_x.max(0);
    let top = origin_y.max(0);
    let right = (origin_x + i64::from(layout.window_width)).min(i64::from(layout.width));
    let bottom = (origin_y + i64::from(layout.window_height)).min(i64::from(layout.height));
    if right <= left || bottom <= top {
        return;
    }

    let mut mask = vec![0u8; layout.width as usize * layout.height as usize];
    for y in top as u32..bottom as u32 {
        for x in left as u32..right as u32 {
            let local_x = (i64::from(x) - origin_x) as u32;
            let local_y = (i64::from(y) - origin_y) as u32;
            let coverage = rounded_coverage(
                local_x,
                local_y,
                layout.window_width,
                layout.window_height,
                layout.corner_radius,
            );
            if coverage > 0.0 {
                mask[y as usize * layout.width as usize + x as usize] =
                    (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    if blur_radius > 0 {
        blur_alpha(&mut mask, layout.width, layout.height, blur_radius);
    }

    for (index, coverage) in mask.into_iter().enumerate() {
        if coverage == 0 {
            continue;
        }
        let alpha = (f32::from(coverage) * opacity).round().clamp(0.0, 255.0) as u8;
        if alpha == 0 {
            continue;
        }
        let offset = index * 4;
        blend_pixel(&mut pixels[offset..offset + 4], [0, 0, 0, alpha]);
    }
}

fn paint_window(
    pixels: &mut [u8],
    canvas_width: u32,
    padding: u32,
    window: &RgbaFrame,
    corner_radius: u32,
) {
    let width = window.width();
    let height = window.height();
    for y in 0..height {
        let Some(row) = window.row(y) else {
            continue;
        };
        for x in 0..width {
            let source_offset = x as usize * 4;
            let Some(source) = row.get(source_offset..source_offset + 4) else {
                continue;
            };
            let coverage = rounded_coverage(x, y, width, height, corner_radius);
            if coverage <= 0.0 {
                continue;
            }
            let alpha = (f32::from(source[3]) * coverage).round().clamp(0.0, 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let target_x = padding + x;
            let target_y = padding + y;
            let target_offset = (target_y as usize * canvas_width as usize + target_x as usize) * 4;
            if alpha == 255 {
                // Opaque window pixels are the common case. Copying avoids
                // the float alpha blend for every interior pixel.
                pixels[target_offset..target_offset + 4].copy_from_slice(source);
            } else {
                blend_pixel(
                    &mut pixels[target_offset..target_offset + 4],
                    [source[0], source[1], source[2], alpha],
                );
            }
        }
    }
}

fn paint_border(
    pixels: &mut [u8],
    canvas_width: u32,
    padding: u32,
    window: &RgbaFrame,
    corner_radius: u32,
    border: &Border,
) {
    let width = border.width;
    let color_alpha = f32::from(border.color[3]);
    if width == 0 || color_alpha <= 0.0 {
        return;
    }
    let window_width = window.width();
    let window_height = window.height();
    let inner_width = window_width.saturating_sub(width.saturating_mul(2));
    let inner_height = window_height.saturating_sub(width.saturating_mul(2));
    if inner_width == 0 || inner_height == 0 {
        return;
    }
    let inner_radius = corner_radius.saturating_sub(width);

    for y in 0..window_height {
        for x in 0..window_width {
            let outer = rounded_coverage(x, y, window_width, window_height, corner_radius);
            if outer <= 0.0 {
                continue;
            }
            let inner = if x >= width
                && y >= width
                && x < width + inner_width
                && y < width + inner_height
            {
                rounded_coverage(
                    x - width,
                    y - width,
                    inner_width,
                    inner_height,
                    inner_radius,
                )
            } else {
                0.0
            };
            let coverage = (outer - inner).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let target_x = padding + x;
            let target_y = padding + y;
            let target_offset = (target_y as usize * canvas_width as usize + target_x as usize) * 4;
            let alpha = (color_alpha * coverage).round().clamp(0.0, 255.0) as u8;
            blend_pixel(
                &mut pixels[target_offset..target_offset + 4],
                [border.color[0], border.color[1], border.color[2], alpha],
            );
        }
    }
}

fn rounded_coverage(x: u32, y: u32, width: u32, height: u32, radius: u32) -> f32 {
    if width == 0 || height == 0 {
        return 0.0;
    }
    let radius = radius.min(width / 2).min(height / 2) as f32;
    if radius <= 0.0 {
        return 1.0;
    }

    let half_width = width as f32 / 2.0;
    let half_height = height as f32 / 2.0;
    let point_x = x as f32 + 0.5;
    let point_y = y as f32 + 0.5;
    let qx = (point_x - half_width).abs() - (half_width - radius);
    let qy = (point_y - half_height).abs() - (half_height - radius);

    // Every interior row/column is a straight edge; only the four corner
    // quadrants need the rounded-distance sqrt. This keeps 4K window
    // captures from evaluating the corner math for millions of pixels.
    if qx <= 0.0 || qy <= 0.0 {
        return 1.0;
    }

    let outside = (qx * qx + qy * qy).sqrt();
    let distance = outside - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

fn blur_alpha(mask: &mut [u8], width: u32, height: u32, radius: u32) {
    let radius = radius.min(MAX_BLUR_RADIUS) as usize;
    if radius == 0 {
        return;
    }

    let width = width as usize;
    let height = height as usize;
    let mut scratch = vec![0u8; mask.len()];
    for _ in 0..BLUR_PASSES {
        blur_horizontal(mask, &mut scratch, width, height, radius);
        blur_vertical(&scratch, mask, width, height, radius);
    }
}

fn blur_horizontal(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let mut prefix = vec![0u32; width + 1];
    for y in 0..height {
        let row = y * width;
        prefix[0] = 0;
        for x in 0..width {
            prefix[x + 1] = prefix[x] + u32::from(src[row + x]);
        }
        for x in 0..width {
            let left = x.saturating_sub(radius);
            let right = (x + radius).min(width - 1);
            let count = (right - left + 1) as u32;
            dst[row + x] = ((prefix[right + 1] - prefix[left]) / count) as u8;
        }
    }
}

fn blur_vertical(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let mut prefix = vec![0u32; height + 1];
    for x in 0..width {
        prefix[0] = 0;
        for y in 0..height {
            prefix[y + 1] = prefix[y] + u32::from(src[y * width + x]);
        }
        for y in 0..height {
            let top = y.saturating_sub(radius);
            let bottom = (y + radius).min(height - 1);
            let count = (bottom - top + 1) as u32;
            dst[y * width + x] = ((prefix[bottom + 1] - prefix[top]) / count) as u8;
        }
    }
}

fn blend_pixel(target: &mut [u8], source: [u8; 4]) {
    let source_alpha = f32::from(source[3]) / 255.0;
    if source_alpha <= 0.0 {
        return;
    }
    let target_alpha = f32::from(target[3]) / 255.0;
    let out_alpha = source_alpha + target_alpha * (1.0 - source_alpha);
    if out_alpha <= 0.0 {
        target.fill(0);
        return;
    }

    for channel in 0..3 {
        let value = (f32::from(source[channel]) * source_alpha
            + f32::from(target[channel]) * target_alpha * (1.0 - source_alpha))
            / out_alpha;
        target[channel] = value.round().clamp(0.0, 255.0) as u8;
    }
    target[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn lerp_color(from: [u8; 4], to: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let mut color = [0u8; 4];
    for channel in 0..4 {
        let value =
            f32::from(from[channel]) + (f32::from(to[channel]) - f32::from(from[channel])) * t;
        color[channel] = value.round().clamp(0.0, 255.0) as u8;
    }
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_frame(width: u32, height: u32, color: [u8; 4]) -> RgbaFrame {
        let pixels = color
            .iter()
            .copied()
            .cycle()
            .take(width as usize * height as usize * 4)
            .collect();
        RgbaFrame::from_pixels(width, height, pixels).expect("test frame is valid")
    }

    fn canvas(background: Background) -> WindowCanvas {
        WindowCanvas {
            padding: 3,
            corner_radius: 4,
            shadow: None,
            background,
            border: None,
        }
    }

    #[test]
    fn transparent_canvas_rounds_corners_and_keeps_alpha() {
        let window = solid_frame(12, 10, [200, 50, 50, 255]);
        let output = compose_window(&window, &canvas(Background::Transparent)).unwrap();

        assert_eq!(output.width(), 18);
        assert_eq!(output.height(), 16);
        assert_eq!(output.pixel(0, 0).unwrap()[3], 0);
        assert_eq!(output.pixel(3, 3).unwrap()[3], 0);
        assert_eq!(output.pixel(3, 8).unwrap(), [200, 50, 50, 255]);
        assert_eq!(output.pixel(9, 8).unwrap()[3], 255);
    }

    #[test]
    fn solid_background_fills_the_whole_canvas() {
        let window = solid_frame(8, 8, [200, 50, 50, 255]);
        let output =
            compose_window(&window, &canvas(Background::Solid([10, 20, 30, 255]))).unwrap();

        assert_eq!(output.pixel(0, 0).unwrap(), [10, 20, 30, 255]);
        assert_eq!(output.pixel(7, 7).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn shadow_draws_alpha_outside_the_window() {
        let window = solid_frame(10, 10, [200, 50, 50, 255]);
        let mut options = canvas(Background::Transparent);
        options.padding = 8;
        options.shadow = Some(Shadow {
            offset_y: 2,
            blur_radius: 2,
            opacity: 1.0,
        });
        let output = compose_window(&window, &options).unwrap();

        assert!(output.pixel(13, 19).unwrap()[3] > 0);
        assert_eq!(output.pixel(13, 12).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn zero_radius_keeps_square_corners() {
        let window = solid_frame(6, 6, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 2,
            corner_radius: 0,
            shadow: None,
            background: Background::Transparent,
            border: None,
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(2, 2).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn border_paints_inside_the_window_edges() {
        let window = solid_frame(10, 10, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 0,
            corner_radius: 0,
            shadow: None,
            background: Background::Transparent,
            border: Some(Border {
                width: 2,
                color: [0, 255, 0, 255],
            }),
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(1, 5).unwrap(), [0, 255, 0, 255]);
        assert_eq!(output.pixel(5, 5).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn rounded_border_keeps_transparent_corners() {
        let window = solid_frame(12, 12, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 0,
            corner_radius: 4,
            shadow: None,
            background: Background::Transparent,
            border: Some(Border {
                width: 1,
                color: [0, 0, 0, 255],
            }),
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(0, 0).unwrap()[3], 0);
        let border = output.pixel(2, 1).unwrap();
        assert_eq!(border[3], 255);
        assert!(border[0] < 200, "border should darken the edge pixel");
        assert_eq!(output.pixel(6, 6).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn vertical_gradient_uses_both_ends() {
        let window = solid_frame(4, 4, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 2,
            corner_radius: 0,
            shadow: None,
            background: Background::VerticalGradient([10, 20, 30, 255], [40, 50, 60, 255]),
            border: None,
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(0, 0).unwrap(), [10, 20, 30, 255]);
        let height = output.height();
        assert_eq!(output.pixel(0, height - 1).unwrap(), [40, 50, 60, 255]);
    }

    #[test]
    fn background_image_is_cover_scaled() {
        let window = solid_frame(2, 2, [200, 50, 50, 255]);
        let background = solid_frame(2, 1, [10, 20, 30, 255]);
        let options = WindowCanvas {
            padding: 1,
            corner_radius: 0,
            shadow: None,
            background: Background::Image(Arc::new(background)),
            border: None,
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.width(), 4);
        assert_eq!(output.height(), 4);
        assert_eq!(output.pixel(0, 0).unwrap()[3], 255);
    }
}
