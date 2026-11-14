use std::borrow::Cow;
use std::fmt;

pub const BYTES_PER_PIXEL: usize = 4;

/// A non-premultiplied RGBA8 image, stored top-down with no color conversion.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaFrame {
    width: u32,
    height: u32,
    stride: usize,
    pixels: Vec<u8>,
}

impl RgbaFrame {
    pub fn new(
        width: u32,
        height: u32,
        stride: usize,
        pixels: Vec<u8>,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::EmptyDimensions { width, height });
        }

        let minimum_stride = (width as usize)
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(FrameError::SizeOverflow)?;
        if stride < minimum_stride {
            return Err(FrameError::StrideTooSmall {
                stride,
                minimum: minimum_stride,
            });
        }

        let expected = stride
            .checked_mul(height as usize)
            .ok_or(FrameError::SizeOverflow)?;
        if pixels.len() != expected {
            return Err(FrameError::BufferSizeMismatch {
                expected,
                actual: pixels.len(),
            });
        }

        Ok(Self {
            width,
            height,
            stride,
            pixels,
        })
    }

    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, FrameError> {
        let stride = (width as usize)
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(FrameError::SizeOverflow)?;
        Self::new(width, height, stride, pixels)
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Returns tightly packed `width * height * 4` pixels.
    ///
    /// Most caller paths expect a packed image; this helper borrows the
    /// existing buffer for ordinary frames and compacts padded rows only when
    /// a platform actually supplied a larger stride.
    pub fn packed_pixels(&self) -> Cow<'_, [u8]> {
        if self.stride == self.width as usize * BYTES_PER_PIXEL {
            Cow::Borrowed(&self.pixels)
        } else {
            Cow::Owned(self.packed_pixels_owned())
        }
    }

    fn packed_pixels_owned(&self) -> Vec<u8> {
        let row_bytes = self.width as usize * BYTES_PER_PIXEL;
        let mut packed = Vec::with_capacity(row_bytes * self.height as usize);
        for y in 0..self.height as usize {
            let start = y * self.stride;
            packed.extend_from_slice(&self.pixels[start..start + row_bytes]);
        }
        packed
    }

    /// Takes the owned buffer without copying.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    /// Takes the owned buffer and repacks padded rows to `width * 4` bytes.
    ///
    /// Tightly packed frames move their buffer; padded frames compact it once.
    pub fn into_packed_pixels(self) -> Vec<u8> {
        if self.stride == self.width as usize * BYTES_PER_PIXEL {
            return self.pixels;
        }
        self.packed_pixels_owned()
    }

    pub fn row(&self, y: u32) -> Option<&[u8]> {
        if y >= self.height {
            return None;
        }
        let start = y as usize * self.stride;
        Some(&self.pixels[start..start + self.stride])
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; BYTES_PER_PIXEL]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = y as usize * self.stride + x as usize * BYTES_PER_PIXEL;
        let pixel = &self.pixels[offset..offset + BYTES_PER_PIXEL];
        Some([pixel[0], pixel[1], pixel[2], pixel[3]])
    }
}

impl fmt::Debug for RgbaFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RgbaFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    EmptyDimensions { width: u32, height: u32 },
    StrideTooSmall { stride: usize, minimum: usize },
    BufferSizeMismatch { expected: usize, actual: usize },
    SizeOverflow,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDimensions { width, height } => {
                write!(f, "frame dimensions must be non-zero, got {width}x{height}")
            }
            Self::StrideTooSmall { stride, minimum } => {
                write!(f, "frame stride {stride} is smaller than {minimum}")
            }
            Self::BufferSizeMismatch { expected, actual } => {
                write!(f, "frame buffer has {actual} bytes, expected {expected}")
            }
            Self::SizeOverflow => f.write_str("frame dimensions overflow address space"),
        }
    }
}

impl std::error::Error for FrameError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_pixels_maps_rows_and_pixels() {
        let frame = RgbaFrame::from_pixels(
            2,
            2,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        )
        .unwrap();
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 2);
        assert_eq!(frame.stride(), 8);
        assert_eq!(frame.row(0), Some(&[1, 2, 3, 4, 5, 6, 7, 8][..]));
        assert_eq!(frame.row(1), Some(&[9, 10, 11, 12, 13, 14, 15, 16][..]));
        assert_eq!(frame.pixel(0, 0), Some([1, 2, 3, 4]));
        assert_eq!(frame.pixel(1, 0), Some([5, 6, 7, 8]));
        assert_eq!(frame.pixel(0, 1), Some([9, 10, 11, 12]));
        assert_eq!(frame.pixel(2, 0), None);
        assert_eq!(frame.row(2), None);
    }

    #[test]
    fn honors_strided_rows() {
        let frame = RgbaFrame::new(
            1,
            2,
            8,
            vec![1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(frame.pixel(0, 0), Some([1, 2, 3, 4]));
        assert_eq!(frame.pixel(0, 1), Some([5, 6, 7, 8]));
        assert_eq!(frame.row(1).unwrap().len(), 8);
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert_eq!(
            RgbaFrame::from_pixels(0, 1, Vec::new()),
            Err(FrameError::EmptyDimensions {
                width: 0,
                height: 1
            })
        );
    }

    #[test]
    fn rejects_short_stride() {
        assert_eq!(
            RgbaFrame::new(2, 1, 4, vec![0; 4]),
            Err(FrameError::StrideTooSmall {
                stride: 4,
                minimum: 8
            })
        );
    }

    #[test]
    fn rejects_wrong_buffer_length() {
        assert_eq!(
            RgbaFrame::from_pixels(2, 2, vec![0; 15]),
            Err(FrameError::BufferSizeMismatch {
                expected: 16,
                actual: 15
            })
        );
    }

    #[test]
    fn packed_pixels_compacts_padded_rows() {
        let frame = RgbaFrame::new(
            1,
            2,
            8,
            vec![1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(frame.packed_pixels().as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn packed_pixels_borrows_tight_rows() {
        let frame = RgbaFrame::from_pixels(1, 1, vec![1, 2, 3, 4]).unwrap();
        assert!(matches!(
            frame.packed_pixels(),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn debug_output_omits_pixels() {
        let frame = RgbaFrame::from_pixels(1, 1, vec![9, 9, 9, 9]).unwrap();
        let text = format!("{frame:?}");
        assert!(text.contains("width: 1"));
        assert!(!text.contains("9, 9, 9, 9"));
    }

    #[test]
    fn into_pixels_moves_the_buffer() {
        let pixels = vec![1, 2, 3, 4];
        let pointer = pixels.as_ptr();
        let frame = RgbaFrame::from_pixels(1, 1, pixels).unwrap();
        let moved = frame.into_pixels();
        assert_eq!(moved, vec![1, 2, 3, 4]);
        assert_eq!(moved.as_ptr(), pointer);
    }

    #[test]
    fn into_packed_pixels_compacts_padded_rows() {
        let frame = RgbaFrame::new(
            1,
            2,
            8,
            vec![1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(frame.into_packed_pixels(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn into_packed_pixels_leaves_tight_rows_untouched() {
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let pointer = pixels.as_ptr();
        let frame = RgbaFrame::from_pixels(2, 1, pixels).unwrap();
        let packed = frame.into_packed_pixels();
        assert_eq!(packed, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(packed.as_ptr(), pointer);
    }
}
