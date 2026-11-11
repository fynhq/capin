//! Window screenshot composition: rounded corners, soft shadows, and a
//! configurable background canvas.
//!
//! The input is a raw window frame in non-premultiplied RGBA. The output keeps
//! alpha so callers can save a transparent PNG or paste the image directly.

use core_types::RgbaFrame;
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
    /// Paint the ring on the canvas outside the window instead of inside its
    /// content. Window captures use this so the border covers the first
    /// shadow pixel; an inside border leaves a light 1px gap outside it.
    pub outside: bool,
}

/// The background painted behind the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    /// Leave the canvas transparent, producing a transparent PNG.
    Transparent,
    /// Paint every canvas pixel with this non-premultiplied RGBA color.
    Solid([u8; 4]),
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
    paint_background(&mut pixels, &canvas.background);

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

fn paint_background(pixels: &mut [u8], background: &Background) {
    if let Background::Solid(color) = background {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            *pixel = *color;
        }
    }
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
    if border.outside {
        paint_border_outside(pixels, canvas_width, padding, window, corner_radius, border);
        return;
    }
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

/// Paints the border into the padding around the window. The ring is the
/// difference between a rounded rectangle expanded by `width` and the window
/// itself, so it follows the corners and covers the first shadow pixels.
fn paint_border_outside(
    pixels: &mut [u8],
    canvas_width: u32,
    padding: u32,
    window: &RgbaFrame,
    corner_radius: u32,
    border: &Border,
) {
    let width = border.width;
    let color_alpha = f32::from(border.color[3]);
    if width == 0 || color_alpha <= 0.0 || padding < width {
        return;
    }
    let window_width = window.width();
    let window_height = window.height();
    if window_width == 0 || window_height == 0 {
        return;
    }
    let outer_width = window_width + width * 2;
    let outer_height = window_height + width * 2;
    let outer_radius = corner_radius + width;
    let origin_x = padding - width;
    let origin_y = padding - width;
    let right = (padding + window_width + width).min(canvas_width);
    let canvas_height = (pixels.len() / 4 / canvas_width as usize) as u32;
    let bottom = (padding + window_height + width).min(canvas_height);

    for y in origin_y..bottom {
        for x in origin_x..right {
            let point_x = x as f32 + 0.5;
            let point_y = y as f32 + 0.5;
            let outer = rounded_coverage_point(
                point_x - origin_x as f32,
                point_y - origin_y as f32,
                outer_width as f32,
                outer_height as f32,
                outer_radius as f32,
            );
            if outer <= 0.0 {
                continue;
            }
            let inner = rounded_coverage_point(
                point_x - padding as f32,
                point_y - padding as f32,
                window_width as f32,
                window_height as f32,
                corner_radius as f32,
            );
            let coverage = (outer * (1.0 - inner)).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let target_offset = (y as usize * canvas_width as usize + x as usize) * 4;
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
    rounded_coverage_point(
        x as f32 + 0.5,
        y as f32 + 0.5,
        width as f32,
        height as f32,
        radius as f32,
    )
}

/// Coverage of a rounded rectangle at an arbitrary (possibly out-of-bounds)
/// point. `point_x`/`point_y` are absolute canvas coordinates; the rectangle
/// starts at the origin.
fn rounded_coverage_point(point_x: f32, point_y: f32, width: f32, height: f32, radius: f32) -> f32 {
    if width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    let radius = radius.max(0.0).min(width.min(height) / 2.0);
    let half_width = width / 2.0;
    let half_height = height / 2.0;
    if radius <= 0.0 {
        let inside = point_x >= 0.0 && point_y >= 0.0 && point_x <= width && point_y <= height;
        return if inside { 1.0 } else { 0.0 };
    }
    let qx = (point_x - half_width).abs() - (half_width - radius);
    let qy = (point_y - half_height).abs() - (half_height - radius);

    let outside_box = point_x < 0.0 || point_y < 0.0 || point_x > width || point_y > height;

    // Every interior row/column is a straight edge; only the four corner
    // quadrants need the rounded-distance sqrt. This keeps 4K window
    // captures from evaluating the corner math for millions of pixels.
    if qx <= 0.0 || qy <= 0.0 {
        return if outside_box { 0.0 } else { 1.0 };
    }
    if (qx * qx + qy * qy).sqrt() - radius > 1.0 {
        return 0.0;
    }

    // Supersample the arc instead of estimating a one-pixel distance band.
    const STEPS: u32 = 8;
    let mut covered = 0u32;
    for step_y in 0..STEPS {
        let sample_y = point_y - 0.5 + (step_y as f32 + 0.5) / STEPS as f32;
        let corner_y = if sample_y < half_height {
            radius
        } else {
            height - radius
        };
        for step_x in 0..STEPS {
            let sample_x = point_x - 0.5 + (step_x as f32 + 0.5) / STEPS as f32;
            let inside = if (radius..=width - radius).contains(&sample_x)
                || (radius..=height - radius).contains(&sample_y)
            {
                true
            } else {
                let corner_x = if sample_x < half_width {
                    radius
                } else {
                    width - radius
                };
                let dx = sample_x - corner_x;
                let dy = sample_y - corner_y;
                dx * dx + dy * dy <= radius * radius
            };
            if inside {
                covered += 1;
            }
        }
    }
    covered as f32 / (STEPS * STEPS) as f32
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
    fn outside_border_covers_the_padding_edge() {
        let window = solid_frame(10, 10, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 2,
            corner_radius: 4,
            shadow: None,
            background: Background::Transparent,
            border: Some(Border {
                width: 1,
                color: [0, 255, 0, 255],
                outside: true,
            }),
        };
        let output = compose_window(&window, &options).unwrap();
        // The ring sits at x/y 1..3, outside the window content at 2..12.
        assert_eq!(output.pixel(1, 6).unwrap(), [0, 255, 0, 255]);
        assert_eq!(output.pixel(2, 6).unwrap(), [200, 50, 50, 255]);
        assert_eq!(output.pixel(13, 6).unwrap()[3], 0);
    }

    #[test]
    fn rounded_corners_are_supersampled() {
        let window = solid_frame(24, 24, [20, 20, 20, 255]);
        let options = WindowCanvas {
            padding: 0,
            corner_radius: 10,
            shadow: None,
            background: Background::Transparent,
            border: None,
        };
        let output = compose_window(&window, &options).unwrap();
        let mut levels = std::collections::BTreeSet::new();
        for y in 0..10 {
            for x in 0..10 {
                levels.insert(output.pixel(x, y).unwrap()[3]);
            }
        }
        assert!(
            levels.iter().any(|alpha| *alpha > 0 && *alpha < 255),
            "corner arc needs intermediate alpha: {levels:?}"
        );
        assert!(levels.len() >= 5, "corner arc is too steppy: {levels:?}");
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
                outside: false,
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
                outside: false,
            }),
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(0, 0).unwrap()[3], 0);
        let border = output.pixel(2, 1).unwrap();
        assert_eq!(border[3], 255);
        assert!(border[0] < 200, "border should darken the edge pixel");
        assert_eq!(output.pixel(6, 6).unwrap(), [200, 50, 50, 255]);
    }
}
