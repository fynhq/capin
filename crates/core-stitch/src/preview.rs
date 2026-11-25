use core_types::RgbaFrame;

use super::{BYTES_PER_PIXEL, Stitcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewAnchor {
    Top,
    Bottom,
}

impl Stitcher {
    pub fn thumbnail_anchored(
        &self,
        max_width: u32,
        max_height: u32,
        anchor: PreviewAnchor,
    ) -> Option<RgbaFrame> {
        if self.height == 0 || self.width == 0 || max_width == 0 || max_height == 0 {
            return None;
        }
        let scale = (f64::from(max_width) / f64::from(self.width)).min(1.0);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let width = (f64::from(self.width) * scale).round().max(1.0) as u32;
        let max_rows = (f64::from(max_height) / scale).floor().max(1.0) as u32;
        let rows = self.height.min(max_rows);
        let start = match anchor {
            PreviewAnchor::Top => 0,
            PreviewAnchor::Bottom => self.height - rows,
        };
        let height = (f64::from(rows) * scale).round().max(1.0) as u32;
        let mut pixels = vec![0u8; width as usize * height as usize * BYTES_PER_PIXEL];
        let src_row = self.width as usize * BYTES_PER_PIXEL;
        for y in 0..height {
            let source_y =
                start as usize + (u64::from(y) * u64::from(rows) / u64::from(height)) as usize;
            for x in 0..width {
                let source_x = (u64::from(x) * u64::from(self.width) / u64::from(width)) as usize;
                let source = source_y * src_row + source_x * BYTES_PER_PIXEL;
                let target = (y as usize * width as usize + x as usize) * BYTES_PER_PIXEL;
                pixels[target..target + BYTES_PER_PIXEL]
                    .copy_from_slice(&self.pixels[source..source + BYTES_PER_PIXEL]);
            }
        }
        RgbaFrame::from_pixels(width, height, pixels).ok()
    }
}
