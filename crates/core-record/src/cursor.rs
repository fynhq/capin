//! Cursor highlight compositing for recorded frames.
//!
//! Screen capture APIs composite the system cursor themselves, so the demo
//! highlight is painted into the captured frame at the cursor position. The
//! circle is translucent and leaves the pointer itself readable.

/// Radius of the highlight disc in logical points.
pub const CURSOR_HIGHLIGHT_RADIUS: f32 = 24.0;

/// Opacity of the highlight disc, applied over the configured color.
pub const CURSOR_HIGHLIGHT_ALPHA: u8 = 0x66;

/// Paints a translucent filled circle into a top-down BGRA frame.
///
/// `center_x`/`center_y` are physical pixels from the top-left corner. The
/// edge is anti-aliased over one pixel; pixels outside the disc are untouched.
/// `color` is packed `0xRRGGBB`.
#[allow(clippy::too_many_arguments)]
pub fn paint_cursor_highlight_bgra(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
    radius: f32,
    color: u32,
    alpha: u8,
) {
    if width == 0
        || height == 0
        || !center_x.is_finite()
        || !center_y.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || alpha == 0
    {
        return;
    }
    if pixels.len() < width as usize * height as usize * 4 {
        return;
    }

    let red = ((color >> 16) & 0xFF) as f32;
    let green = ((color >> 8) & 0xFF) as f32;
    let blue = (color & 0xFF) as f32;
    let source_alpha = f32::from(alpha) / 255.0;

    let left = (center_x - radius - 1.0).floor().max(0.0) as u32;
    let top = (center_y - radius - 1.0).floor().max(0.0) as u32;
    let right = ((center_x + radius + 1.0).ceil().max(0.0) as u32).min(width);
    let bottom = ((center_y + radius + 1.0).ceil().max(0.0) as u32).min(height);
    if left >= right || top >= bottom {
        return;
    }

    for y in top..bottom {
        let dy = y as f32 + 0.5 - center_y;
        for x in left..right {
            let dx = x as f32 + 0.5 - center_x;
            let distance = (dx * dx + dy * dy).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let opacity = source_alpha * coverage;
            let index = (y as usize * width as usize + x as usize) * 4;
            let target = &mut pixels[index..index + 4];
            target[0] = blend(target[0], blue, opacity);
            target[1] = blend(target[1], green, opacity);
            target[2] = blend(target[2], red, opacity);
        }
    }
}

fn blend(destination: u8, source: f32, opacity: f32) -> u8 {
    let value = source * opacity + f32::from(destination) * (1.0 - opacity);
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> Vec<u8> {
        vec![0u8; width as usize * height as usize * 4]
    }

    #[test]
    fn paints_the_center_and_leaves_the_outside_untouched() {
        let mut pixels = frame(100, 100);
        paint_cursor_highlight_bgra(&mut pixels, 100, 100, 50.0, 50.0, 20.0, 0xFFCC00, 255);

        let center = (50 * 100 + 50) * 4;
        assert_eq!(&pixels[center..center + 3], &[0x00, 0xCC, 0xFF]);
        let corner = 0;
        assert_eq!(&pixels[corner..corner + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn applies_partial_alpha() {
        let mut pixels = frame(20, 20);
        paint_cursor_highlight_bgra(&mut pixels, 20, 20, 10.0, 10.0, 5.0, 0xFFFFFF, 128);
        let center = (10 * 20 + 10) * 4;
        assert!((120..=136).contains(&center_pixel(&pixels, center)[0]));
    }

    fn center_pixel(pixels: &[u8], index: usize) -> &[u8] {
        &pixels[index..index + 4]
    }

    #[test]
    fn clips_a_circle_that_crosses_the_frame_edge() {
        let mut pixels = frame(10, 10);
        paint_cursor_highlight_bgra(&mut pixels, 10, 10, 0.0, 0.0, 6.0, 0xFF0000, 255);
        assert_eq!(&pixels[0..3], &[0, 0, 255]);
    }

    #[test]
    fn ignores_empty_frames_and_zero_radius() {
        let mut pixels = frame(4, 4);
        paint_cursor_highlight_bgra(&mut pixels, 4, 4, 2.0, 2.0, 0.0, 0xFFFFFF, 255);
        paint_cursor_highlight_bgra(&mut [], 0, 0, 0.0, 0.0, 4.0, 0xFFFFFF, 255);
        assert!(pixels.iter().all(|byte| *byte == 0));
    }
}
