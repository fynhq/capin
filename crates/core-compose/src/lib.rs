use core_types::RgbaFrame;
use thiserror::Error;

const MAX_PADDING: u32 = 2048;
const MAX_BLUR_RADIUS: u32 = 512;
const BLUR_PASSES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset_y: i32,
    pub blur_radius: u32,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub width: u32,
    pub color: [u8; 4],
    pub outside: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    Transparent,
    Solid([u8; 4]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowCanvas {
    pub padding: u32,
    pub corner_radius: u32,
    pub shadow: Option<Shadow>,
    pub background: Background,
    pub border: Option<Border>,
    pub edge_bleed: bool,
    pub edge_feather: bool,
}

impl Default for WindowCanvas {
    fn default() -> Self {
        Self {
            padding: 48,
            corner_radius: 12,
            shadow: Some(Shadow::default()),
            background: Background::Transparent,
            border: None,
            edge_bleed: false,
            edge_feather: false,
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

pub fn estimate_corner_radius(frame: &RgbaFrame, max_radius: u32) -> Option<u32> {
    let width = frame.width();
    let height = frame.height();
    let max_radius = max_radius.min(width.min(height) / 2);
    if max_radius < 4 || width < 8 || height < 8 {
        return None;
    }
    let stride = frame.stride();
    let pixels = frame.pixels();
    let rows = (max_radius as usize / 2).max(4);
    let mut estimates = Vec::new();
    for row in 0..rows {
        let mut first = None;
        for x in 0..max_radius as usize {
            if pixels[row * stride + x * 4 + 3] >= 128 {
                first = Some(x);
                break;
            }
        }
        let Some(x) = first else {
            continue;
        };
        if x == 0 {
            if !estimates.is_empty() {
                break;
            }
            continue;
        }
        let d = row as f64 + 0.5;
        let x = x as f64;
        let radius = x + d + (2.0 * x * d).sqrt();
        if radius >= 2.0 && radius <= f64::from(max_radius) + 2.0 {
            estimates.push(radius);
        }
    }
    if estimates.len() < 2 {
        return None;
    }
    estimates.sort_by(|left, right| left.total_cmp(right));
    Some((estimates[estimates.len() / 2].round() as u32).clamp(1, max_radius))
}

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

    paint_window(
        &mut pixels,
        width,
        padding,
        window,
        canvas.corner_radius,
        canvas.edge_bleed,
        canvas.edge_feather,
    );
    if let Some(border) = &canvas.border {
        paint_border(
            &mut pixels,
            width,
            padding,
            window,
            canvas.corner_radius,
            border,
            canvas.edge_feather,
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
    edge_bleed: bool,
    edge_feather: bool,
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
            let coverage = if edge_feather {
                feathered_coverage(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    width as f32,
                    height as f32,
                    corner_radius as f32,
                )
            } else {
                rounded_coverage(x, y, width, height, corner_radius)
            };
            if coverage <= 0.0 {
                continue;
            }
            let alpha = (f32::from(source[3]) * coverage).round().clamp(0.0, 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let color = if edge_bleed {
                bleeded_color(
                    window,
                    x,
                    y,
                    width,
                    height,
                    corner_radius,
                    [source[0], source[1], source[2]],
                )
            } else {
                [source[0], source[1], source[2]]
            };
            let target_x = padding + x;
            let target_y = padding + y;
            let target_offset = (target_y as usize * canvas_width as usize + target_x as usize) * 4;
            if alpha == 255 {
                pixels[target_offset..target_offset + 4]
                    .copy_from_slice(&[color[0], color[1], color[2], 255]);
            } else {
                blend_pixel(
                    &mut pixels[target_offset..target_offset + 4],
                    [color[0], color[1], color[2], alpha],
                );
            }
        }
    }
}

const EDGE_BLEED_BAND: f32 = 2.0;

fn bleeded_color(
    window: &RgbaFrame,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    corner_radius: u32,
    fallback: [u8; 3],
) -> [u8; 3] {
    if corner_radius == 0 || width < 3 || height < 3 {
        return fallback;
    }
    let radius = corner_radius.min(width.min(height) / 2) as f32;
    let point_x = x as f32 + 0.5;
    let point_y = y as f32 + 0.5;
    let half_width = width as f32 / 2.0;
    let half_height = height as f32 / 2.0;
    let in_corner = (point_x < radius || point_x > width as f32 - radius)
        && (point_y < radius || point_y > height as f32 - radius);
    if !in_corner {
        return fallback;
    }

    let distance = rounded_distance_point(point_x, point_y, width as f32, height as f32, radius);
    if distance < -EDGE_BLEED_BAND {
        return fallback;
    }

    let step = EDGE_BLEED_BAND + 1.0;
    let sample_x = if point_x < half_width {
        point_x + step
    } else {
        point_x - step
    };
    let sample_y = if point_y < half_height {
        point_y + step
    } else {
        point_y - step
    };
    let sample_x = (sample_x.round() as i64).clamp(0, i64::from(width) - 1) as u32;
    let sample_y = (sample_y.round() as i64).clamp(0, i64::from(height) - 1) as u32;
    window
        .pixel(sample_x, sample_y)
        .map(|sample| [sample[0], sample[1], sample[2]])
        .unwrap_or(fallback)
}

fn paint_border(
    pixels: &mut [u8],
    canvas_width: u32,
    padding: u32,
    window: &RgbaFrame,
    corner_radius: u32,
    border: &Border,
    edge_feather: bool,
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
            let outer = if edge_feather {
                feathered_coverage(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    window_width as f32,
                    window_height as f32,
                    corner_radius as f32,
                )
            } else {
                rounded_coverage(x, y, window_width, window_height, corner_radius)
            };
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

const EDGE_FEATHER_WIDTH: f32 = 1.0;

fn rounded_distance_point(point_x: f32, point_y: f32, width: f32, height: f32, radius: f32) -> f32 {
    let radius = radius.max(0.0).min(width.min(height) / 2.0);
    let half_width = width / 2.0;
    let half_height = height / 2.0;
    let qx = (point_x - half_width).abs() - (half_width - radius);
    let qy = (point_y - half_height).abs() - (half_height - radius);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius
}

fn feathered_coverage(point_x: f32, point_y: f32, width: f32, height: f32, radius: f32) -> f32 {
    if width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    let distance = rounded_distance_point(point_x, point_y, width, height, radius);
    let t = (0.5 - distance / EDGE_FEATHER_WIDTH).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

    if qx <= 0.0 || qy <= 0.0 {
        return if outside_box { 0.0 } else { 1.0 };
    }
    if (qx * qx + qy * qy).sqrt() - radius > 1.0 {
        return 0.0;
    }

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
            edge_bleed: false,
            edge_feather: false,
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
            edge_bleed: false,
            edge_feather: false,
        };
        let output = compose_window(&window, &options).unwrap();
        assert_eq!(output.pixel(1, 6).unwrap(), [0, 255, 0, 255]);
        assert_eq!(output.pixel(2, 6).unwrap(), [200, 50, 50, 255]);
        assert_eq!(output.pixel(13, 6).unwrap()[3], 0);
    }

    #[test]
    fn edge_feather_softens_the_rounded_boundary() {
        let width = 20.0;
        let height = 20.0;
        let radius = 5.0;
        let outside = feathered_coverage(10.0, -0.25, width, height, radius);
        let inside = feathered_coverage(10.0, 0.25, width, height, radius);
        assert!(outside > 0.0 && outside < 0.5);
        assert!(inside > 0.5 && inside < 1.0);
        assert_eq!(feathered_coverage(10.0, -0.75, width, height, radius), 0.0);
        assert_eq!(feathered_coverage(10.0, 0.75, width, height, radius), 1.0);
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
            edge_bleed: false,
            edge_feather: false,
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
    fn edge_bleed_removes_wallpaper_color_from_rounded_corners() {
        let width = 24;
        let height = 24;
        let radius = 8;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let inside = rounded_coverage(x, y, width, height, radius) >= 0.99;
                let color = if inside {
                    [20, 20, 20, 255]
                } else {
                    [240, 240, 240, 255]
                };
                let offset = (y as usize * width as usize + x as usize) * 4;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        let window = RgbaFrame::from_pixels(width, height, pixels).unwrap();
        let mut options = WindowCanvas {
            padding: 0,
            corner_radius: radius,
            shadow: None,
            background: Background::Transparent,
            border: None,
            edge_bleed: false,
            edge_feather: false,
        };
        let plain = compose_window(&window, &options).unwrap();
        options.edge_bleed = true;
        let bled = compose_window(&window, &options).unwrap();

        let mut checked = false;
        for y in 0..radius {
            for x in 0..radius {
                let source = window.pixel(x, y).unwrap();
                let plain = plain.pixel(x, y).unwrap();
                let bled = bled.pixel(x, y).unwrap();
                if source[0] == 240 && plain[3] > 0 && plain[3] < 255 {
                    checked = true;
                    assert!(
                        plain[0] > 200,
                        "plain composition should keep the wallpaper fringe: {plain:?}"
                    );
                    assert!(
                        bled[0] < 80,
                        "edge bleed should use the window color: {bled:?}"
                    );
                }
            }
        }
        assert!(
            checked,
            "the corner test needs a partially covered edge pixel"
        );
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
            edge_bleed: false,
            edge_feather: false,
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
            edge_bleed: false,
            edge_feather: false,
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
            edge_bleed: false,
            edge_feather: false,
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(0, 0).unwrap()[3], 0);
        let border = output.pixel(2, 1).unwrap();
        assert_eq!(border[3], 255);
        assert!(border[0] < 200, "border should darken the edge pixel");
        assert_eq!(output.pixel(6, 6).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn zero_padding_skips_the_shadow_and_keeps_the_canvas_size() {
        let window = solid_frame(10, 10, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 0,
            corner_radius: 4,
            shadow: Some(Shadow {
                offset_y: 0,
                blur_radius: 8,
                opacity: 1.0,
            }),
            background: Background::Solid([10, 20, 30, 255]),
            border: None,
            edge_bleed: false,
            edge_feather: false,
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.width(), 10);
        assert_eq!(output.height(), 10);
        assert_eq!(output.pixel(0, 0).unwrap(), [10, 20, 30, 255]);
        assert_eq!(output.pixel(5, 5).unwrap(), [200, 50, 50, 255]);
    }

    #[test]
    fn shadow_blends_over_a_solid_background() {
        let window = solid_frame(10, 10, [200, 50, 50, 255]);
        let options = WindowCanvas {
            padding: 12,
            corner_radius: 0,
            shadow: Some(Shadow {
                offset_y: 0,
                blur_radius: 6,
                opacity: 1.0,
            }),
            background: Background::Solid([10, 20, 30, 255]),
            border: None,
            edge_bleed: false,
            edge_feather: false,
        };
        let output = compose_window(&window, &options).unwrap();

        assert_eq!(output.pixel(0, 0).unwrap(), [10, 20, 30, 255]);
        assert_eq!(output.pixel(12, 12).unwrap(), [200, 50, 50, 255]);
        let shadowed = output.pixel(8, 12).unwrap();
        assert_eq!(shadowed[3], 255);
        assert!(
            shadowed[0] < 10 && shadowed[1] < 20 && shadowed[2] < 30,
            "the shadow should darken the solid background"
        );
    }

    fn rounded_frame(width: u32, height: u32, radius: u32) -> RgbaFrame {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let coverage = rounded_coverage(x, y, width, height, radius);
                let index = (y as usize * width as usize + x as usize) * 4;
                pixels[index] = 200;
                pixels[index + 1] = 50;
                pixels[index + 2] = 50;
                pixels[index + 3] = (coverage * 255.0).round() as u8;
            }
        }
        RgbaFrame::from_pixels(width, height, pixels).expect("test frame is valid")
    }

    #[test]
    fn corner_radius_is_measured_from_the_alpha_channel() {
        for radius in [12, 20, 40] {
            let frame = rounded_frame(160, 120, radius);
            let estimated =
                estimate_corner_radius(&frame, 64).expect("rounded alpha is measurable");
            assert!(
                estimated.abs_diff(radius) <= 2,
                "expected about {radius}, measured {estimated}"
            );
        }
    }

    #[test]
    fn opaque_frames_report_no_corner_radius() {
        let frame = solid_frame(80, 60, [200, 50, 50, 255]);
        assert_eq!(estimate_corner_radius(&frame, 64), None);
    }
}
