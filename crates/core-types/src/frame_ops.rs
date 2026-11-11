use crate::frame::RgbaFrame;
use crate::geometry::PhysicalRect;

impl RgbaFrame {
    /// Returns the frame cropped to `rect`, clamped to the frame bounds.
    ///
    /// Returns `None` when the intersection is empty. The result is packed at
    /// `width * 4` bytes per row; padded input rows are handled correctly.
    pub fn cropped(&self, rect: PhysicalRect) -> Option<Self> {
        let bounds = PhysicalRect::new(0, 0, self.width(), self.height());
        let overlap = bounds.intersection(rect)?;
        let width = overlap.size.width as usize;
        let height = overlap.size.height as usize;
        let left = overlap.origin.x as usize * 4;
        let row_bytes = width * 4;
        let mut pixels = Vec::with_capacity(row_bytes * height);
        for y in 0..overlap.size.height {
            let row = self.row(overlap.origin.y as u32 + y)?;
            pixels.extend_from_slice(row.get(left..left + row_bytes)?);
        }
        Self::from_pixels(width as u32, overlap.size.height, pixels).ok()
    }

    /// Crops an owned frame, reusing its buffer when the rows are tightly
    /// packed. This avoids allocating a second full image for the common
    /// one-pixel border trim and similar whole-width crops.
    pub fn into_cropped(self, rect: PhysicalRect) -> Option<Self> {
        let frame_width = self.width();
        let frame_height = self.height();
        let stride = self.stride();
        let bounds = PhysicalRect::new(0, 0, frame_width, frame_height);
        let overlap = bounds.intersection(rect)?;
        if overlap == bounds {
            return Some(self);
        }
        if stride != frame_width as usize * 4 {
            return self.cropped(overlap);
        }
        let width = overlap.size.width as usize;
        let height = overlap.size.height as usize;
        let top = overlap.origin.y as usize;
        let left = overlap.origin.x as usize;
        let row_bytes = width * 4;
        let mut pixels = self.into_pixels();
        for y in 0..height {
            let source = (top + y) * stride + left * 4;
            pixels.copy_within(source..source + row_bytes, y * row_bytes);
        }
        pixels.truncate(row_bytes * height);
        Self::from_pixels(width as u32, height as u32, pixels).ok()
    }

    /// Rotates the frame 90 degrees clockwise. Padded input rows are handled
    /// correctly; the result is tightly packed.
    pub fn rotated_cw(&self) -> Self {
        let width = self.width();
        let height = self.height();
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            let row = self.row(y).expect("row index is within the frame");
            for x in 0..width {
                let source = &row[x as usize * 4..x as usize * 4 + 4];
                let target_x = height - 1 - y;
                let target_y = x;
                let target = (target_y as usize * height as usize + target_x as usize) * 4;
                pixels[target..target + 4].copy_from_slice(source);
            }
        }
        Self::from_pixels(height, width, pixels).expect("rotated frame keeps valid dimensions")
    }

    /// Returns a horizontally or vertically flipped frame. Padded input rows
    /// are handled correctly; the result is tightly packed.
    pub fn flipped(&self, horizontal: bool) -> Self {
        let width = self.width();
        let height = self.height();
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            let row = self.row(y).expect("row index is within the frame");
            let target_y = if horizontal { y } else { height - 1 - y };
            for x in 0..width {
                let source = &row[x as usize * 4..x as usize * 4 + 4];
                let target_x = if horizontal { width - 1 - x } else { x };
                let target = (target_y as usize * width as usize + target_x as usize) * 4;
                pixels[target..target + 4].copy_from_slice(source);
            }
        }
        Self::from_pixels(width, height, pixels).expect("flipped frame keeps valid dimensions")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_frame() -> RgbaFrame {
        let pixels = (0..24).collect::<Vec<u8>>();
        RgbaFrame::from_pixels(3, 2, pixels).unwrap()
    }

    #[test]
    fn cropped_clamps_to_the_frame_bounds() {
        let frame = pixel_frame();
        let cropped = frame.cropped(PhysicalRect::new(1, 0, 2, 2)).unwrap();
        assert_eq!((cropped.width(), cropped.height()), (2, 2));
        assert_eq!(cropped.pixel(0, 0), Some([4, 5, 6, 7]));
        assert_eq!(cropped.pixel(1, 0), Some([8, 9, 10, 11]));
        assert_eq!(cropped.pixel(0, 1), Some([16, 17, 18, 19]));

        let clipped = frame.cropped(PhysicalRect::new(-1, -1, 2, 2)).unwrap();
        assert_eq!((clipped.width(), clipped.height()), (1, 1));
        assert_eq!(clipped.pixel(0, 0), Some([0, 1, 2, 3]));
        assert!(frame.cropped(PhysicalRect::new(3, 2, 2, 2)).is_none());
        assert!(frame.cropped(PhysicalRect::new(0, 0, 0, 4)).is_none());
    }

    #[test]
    fn cropped_handles_padded_rows() {
        let frame = RgbaFrame::new(
            2,
            2,
            12,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 99, 99, 99,
                99,
            ],
        )
        .unwrap();
        let cropped = frame.cropped(PhysicalRect::new(1, 0, 1, 2)).unwrap();
        assert_eq!(
            (cropped.width(), cropped.height(), cropped.stride()),
            (1, 2, 4)
        );
        assert_eq!(cropped.pixel(0, 0), Some([5, 6, 7, 8]));
        assert_eq!(cropped.pixel(0, 1), Some([13, 14, 15, 16]));
    }

    #[test]
    fn into_cropped_matches_borrowed_crop() {
        let pixels = (0..24).collect::<Vec<u8>>();
        let source = RgbaFrame::from_pixels(3, 2, pixels).unwrap();
        for rect in [
            PhysicalRect::new(0, 0, 3, 2),
            PhysicalRect::new(1, 0, 2, 2),
            PhysicalRect::new(0, 1, 3, 1),
            PhysicalRect::new(-1, -1, 2, 2),
            PhysicalRect::new(2, 0, 5, 5),
        ] {
            let borrowed = source.cropped(rect);
            let owned = source.clone().into_cropped(rect);
            assert_eq!(owned, borrowed, "crop mismatch for {rect:?}");
        }
        assert!(source.into_cropped(PhysicalRect::new(3, 2, 1, 1)).is_none());
    }

    #[test]
    fn into_cropped_falls_back_for_padded_rows() {
        let frame = RgbaFrame::new(
            2,
            2,
            12,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 99, 99, 99,
                99,
            ],
        )
        .unwrap();
        let expected = frame.cropped(PhysicalRect::new(1, 0, 1, 2)).unwrap();
        let cropped = frame.into_cropped(PhysicalRect::new(1, 0, 1, 2)).unwrap();
        assert_eq!(cropped, expected);
    }

    #[test]
    fn rotated_cw_maps_pixels_and_dimensions() {
        let rotated = pixel_frame().rotated_cw();
        assert_eq!(
            (rotated.width(), rotated.height(), rotated.stride()),
            (2, 3, 8)
        );
        assert_eq!(rotated.pixel(0, 0), Some([12, 13, 14, 15]));
        assert_eq!(rotated.pixel(1, 0), Some([0, 1, 2, 3]));
        assert_eq!(rotated.pixel(0, 1), Some([16, 17, 18, 19]));
        assert_eq!(rotated.pixel(1, 1), Some([4, 5, 6, 7]));
        assert_eq!(rotated.pixel(0, 2), Some([20, 21, 22, 23]));
        assert_eq!(rotated.pixel(1, 2), Some([8, 9, 10, 11]));
    }

    #[test]
    fn rotated_cw_handles_padded_rows() {
        let frame = RgbaFrame::new(
            2,
            2,
            12,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 99, 99, 99,
                99,
            ],
        )
        .unwrap();
        let rotated = frame.rotated_cw();
        assert_eq!(
            (rotated.width(), rotated.height(), rotated.stride()),
            (2, 2, 8)
        );
        assert_eq!(rotated.pixel(0, 0), Some([9, 10, 11, 12]));
        assert_eq!(rotated.pixel(1, 0), Some([1, 2, 3, 4]));
        assert_eq!(rotated.pixel(0, 1), Some([13, 14, 15, 16]));
        assert_eq!(rotated.pixel(1, 1), Some([5, 6, 7, 8]));
    }

    #[test]
    fn flipped_maps_pixels_in_both_axes() {
        let frame = pixel_frame();
        let horizontal = frame.flipped(true);
        assert_eq!(horizontal.pixel(0, 0), Some([8, 9, 10, 11]));
        assert_eq!(horizontal.pixel(1, 0), Some([4, 5, 6, 7]));
        assert_eq!(horizontal.pixel(2, 0), Some([0, 1, 2, 3]));
        assert_eq!(horizontal.pixel(0, 1), Some([20, 21, 22, 23]));

        let vertical = frame.flipped(false);
        assert_eq!(vertical.pixel(0, 0), Some([12, 13, 14, 15]));
        assert_eq!(vertical.pixel(2, 0), Some([20, 21, 22, 23]));
        assert_eq!(vertical.pixel(0, 1), Some([0, 1, 2, 3]));
        assert_eq!(vertical.pixel(2, 1), Some([8, 9, 10, 11]));
    }

    #[test]
    fn flipped_handles_padded_rows() {
        let frame = RgbaFrame::new(
            2,
            2,
            12,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 99, 99, 99,
                99,
            ],
        )
        .unwrap();
        let horizontal = frame.flipped(true);
        assert_eq!(horizontal.stride(), 8);
        assert_eq!(horizontal.pixel(0, 0), Some([5, 6, 7, 8]));
        assert_eq!(horizontal.pixel(1, 0), Some([1, 2, 3, 4]));
        assert_eq!(horizontal.pixel(0, 1), Some([13, 14, 15, 16]));
    }
}
