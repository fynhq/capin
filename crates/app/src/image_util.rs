use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use core_types::{PhysicalRect, PhysicalSize, RgbaFrame};
use gpui_kit::*;
use image::ImageEncoder;

/// Builds a GPUI image from an owned frame, swapping RGBA to BGRA in place.
/// Callers that need to keep the RGBA frame pass a clone instead of paying for
/// a second full-size buffer inside the conversion.
pub fn render_image(frame: RgbaFrame) -> Arc<RenderImage> {
    let width = frame.width();
    let height = frame.height();
    let mut pixels = frame.into_packed_pixels();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }

    // GPUI interprets RenderImage bytes as BGRA; only the byte order matters.
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)
        .expect("capture frames are validated before conversion");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// Copies a borrowed frame before converting it to a GPUI image.
pub fn render_image_ref(frame: &RgbaFrame) -> Arc<RenderImage> {
    render_image(frame.clone())
}

/// One transparent pixel, used by overlays whose editor already paints the
/// frozen monitor image and no longer needs a second full-screen background.
pub fn transparent_image() -> Arc<RenderImage> {
    render_image(
        RgbaFrame::from_pixels(1, 1, vec![0, 0, 0, 0])
            .expect("one transparent pixel is a valid frame"),
    )
}

/// Dimensions of the BGRA render image.
pub fn image_size(image: &RenderImage) -> PhysicalSize {
    let size = image.size(0);
    PhysicalSize::new(size.width.0.max(0) as u32, size.height.0.max(0) as u32)
}

/// Samples one pixel of a BGRA render image as `[red, green, blue, alpha]`.
pub fn image_pixel(image: &RenderImage, x: u32, y: u32) -> Option<[u8; 4]> {
    let size = image_size(image);
    if x >= size.width || y >= size.height {
        return None;
    }
    let bytes = image.as_bytes(0)?;
    let offset = (y as usize * size.width as usize + x as usize) * 4;
    let pixel = bytes.get(offset..offset + 4)?;
    Some([pixel[2], pixel[1], pixel[0], pixel[3]])
}

/// Crops a BGRA render image and returns a tightly packed RGBA frame.
///
/// The render image stays the only full-size CPU copy while an overlay is
/// open; only the selected region pays for an RGBA buffer.
pub fn cropped_rgba(image: &RenderImage, rect: PhysicalRect) -> Option<RgbaFrame> {
    let size = image_size(image);
    let bounds = PhysicalRect::new(0, 0, size.width, size.height);
    let overlap = bounds.intersection(rect)?;
    let bytes = image.as_bytes(0)?;
    let width = overlap.size.width as usize;
    let height = overlap.size.height as usize;
    let row_bytes = size.width as usize * 4;
    let left = overlap.origin.x as usize * 4;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row_start = (overlap.origin.y as usize + y) * row_bytes + left;
        let source = bytes.get(row_start..row_start + width * 4)?;
        pixels.extend(
            source
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]]),
        );
    }
    RgbaFrame::from_pixels(width as u32, height as u32, pixels).ok()
}

/// Builds a nearest-neighbour magnified crop around `center` for the overlay
/// magnifier. `center` is in image-local physical pixels.
pub fn magnifier_image(
    image: &RenderImage,
    center_x: i32,
    center_y: i32,
    source_size: u32,
    output_size: u32,
) -> Arc<RenderImage> {
    let output_size = output_size.max(1);
    let source_size = source_size.max(1);
    let half_source = source_size as i32 / 2;
    let mut pixels = vec![0u8; output_size as usize * output_size as usize * 4];
    let image_size = image_size(image);
    let bytes = image.as_bytes(0).unwrap_or(&[]);
    let source_columns: Vec<i32> = (0..output_size)
        .map(|output_x| {
            center_x - half_source
                + (i64::from(output_x) * i64::from(source_size) / i64::from(output_size)) as i32
        })
        .collect();

    for output_y in 0..output_size {
        let source_y = center_y - half_source
            + (i64::from(output_y) * i64::from(source_size) / i64::from(output_size)) as i32;
        if source_y < 0 || source_y as u32 >= image_size.height {
            continue;
        }
        for (output_x, &source_x) in source_columns.iter().enumerate() {
            if source_x < 0 || source_x as u32 >= image_size.width {
                continue;
            }
            let offset = (source_y as usize * image_size.width as usize + source_x as usize) * 4;
            let Some([blue, green, red, alpha]) = bytes
                .get(offset..offset + 4)
                .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
            else {
                continue;
            };
            let target = (output_y as usize * output_size as usize + output_x) * 4;
            pixels[target] = blue;
            pixels[target + 1] = green;
            pixels[target + 2] = red;
            pixels[target + 3] = alpha;
        }
    }

    let buffer =
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(output_size, output_size, pixels)
            .expect("magnifier buffer matches its dimensions");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

struct FrameView<'a>(&'a RgbaFrame);

impl image::GenericImageView for FrameView<'_> {
    type Pixel = image::Rgba<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.0.width(), self.0.height())
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let [red, green, blue, alpha] = self.0.pixel(x, y).unwrap_or_default();
        image::Rgba([red, green, blue, alpha])
    }
}

struct RenderFrameView<'a> {
    size: PhysicalSize,
    bytes: Option<&'a [u8]>,
}

impl<'a> RenderFrameView<'a> {
    fn new(image: &'a RenderImage) -> Self {
        Self {
            size: image_size(image),
            bytes: image.as_bytes(0),
        }
    }
}

impl image::GenericImageView for RenderFrameView<'_> {
    type Pixel = image::Rgba<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.size.width, self.size.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        if x >= self.size.width || y >= self.size.height {
            return image::Rgba([0, 0, 0, 0]);
        }
        let Some(bytes) = self.bytes else {
            return image::Rgba([0, 0, 0, 0]);
        };
        let offset = (y as usize * self.size.width as usize + x as usize) * 4;
        let Some([blue, green, red, alpha]) = bytes
            .get(offset..offset + 4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
        else {
            return image::Rgba([0, 0, 0, 0]);
        };
        image::Rgba([red, green, blue, alpha])
    }
}

/// Builds the preview for a box-blur radius. The frame is downscaled by two
/// before the Gaussian pass so the preview keeps the exported radius; the
/// previous eighth-resolution preview compressed every strength into the same
/// frosted look. The downscale reads the source frame directly instead of
/// cloning a full RGBA image first.
pub fn blurred_image(frame: &RgbaFrame, radius: u32) -> Arc<RenderImage> {
    blur_from_view(&FrameView(frame), frame.width(), frame.height(), radius)
}

/// Blur preview for editors that display a BGRA render image directly.
pub fn blurred_image_from_render(image: &RenderImage, radius: u32) -> Arc<RenderImage> {
    let view = RenderFrameView::new(image);
    blur_from_view(&view, view.size.width, view.size.height, radius)
}

fn blur_from_view(
    view: &impl image::GenericImageView<Pixel = image::Rgba<u8>>,
    width: u32,
    height: u32,
    radius: u32,
) -> Arc<RenderImage> {
    let small_width = (width / 2).max(1);
    let small_height = (height / 2).max(1);
    let small = image::imageops::resize(
        view,
        small_width,
        small_height,
        image::imageops::FilterType::Triangle,
    );
    let sigma = (radius.max(1) as f32 / 2.0).max(0.5);
    let blurred = image::imageops::blur(&small, sigma);
    let mut pixels = blurred.into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer =
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(small_width, small_height, pixels)
            .expect("blurred image size is valid");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

pub fn encode_png(frame: &RgbaFrame) -> Result<Vec<u8>, image::ImageError> {
    let mut cursor = Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new(&mut cursor).write_image(
        frame.pixels(),
        frame.width(),
        frame.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(cursor.into_inner())
}

pub fn copy_png_bytes(bytes: Vec<u8>, cx: &mut App) {
    let image = Image::from_bytes(ImageFormat::Png, bytes);
    cx.write_to_clipboard(ClipboardItem::new_image(&image));
}

pub fn save_png(frame: &RgbaFrame) -> Result<PathBuf, std::io::Error> {
    crate::storage::save_png(frame)
}

#[cfg(test)]
mod tests {
    use super::{
        blurred_image, blurred_image_from_render, cropped_rgba, image_pixel, image_size,
        render_image, render_image_ref,
    };
    use core_types::{PhysicalRect, PhysicalSize, RgbaFrame};

    fn checker_frame(width: u32, height: u32) -> RgbaFrame {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let red = (x * 17 + y * 3) as u8;
                let green = (x * 5 + y * 11) as u8;
                let blue = (x * 7 + y * 13) as u8;
                let alpha = 200 + ((x + y) % 55) as u8;
                pixels.extend_from_slice(&[red, green, blue, alpha]);
            }
        }
        RgbaFrame::from_pixels(width, height, pixels).unwrap()
    }

    #[test]
    fn render_image_stores_bgra_without_losing_pixels() {
        let frame = RgbaFrame::from_pixels(1, 1, vec![10, 20, 30, 40]).unwrap();
        let image = render_image(frame);
        assert_eq!(image.as_bytes(0), Some(&[30, 20, 10, 40][..]));
        assert_eq!(image_size(&image), PhysicalSize::new(1, 1));
        assert_eq!(image_pixel(&image, 0, 0), Some([10, 20, 30, 40]));
    }

    #[test]
    fn cropped_rgba_matches_the_source_frame() {
        let frame = checker_frame(5, 4);
        let image = render_image_ref(&frame);
        for rect in [
            PhysicalRect::new(0, 0, 5, 4),
            PhysicalRect::new(1, 0, 3, 4),
            PhysicalRect::new(2, 1, 2, 2),
            PhysicalRect::new(-2, -1, 4, 3),
            PhysicalRect::new(4, 3, 6, 6),
        ] {
            let cropped = cropped_rgba(&image, rect);
            assert_eq!(cropped, frame.cropped(rect), "crop mismatch for {rect:?}");
        }
        assert!(cropped_rgba(&image, PhysicalRect::new(5, 4, 1, 1)).is_none());
    }

    #[test]
    fn blurred_image_matches_the_full_copy_pipeline() {
        let frame = checker_frame(8, 6);
        let image = render_image_ref(&frame);
        for radius in [8, 16, 24, 32] {
            let expected_small = image::imageops::resize(
                &image::RgbaImage::from_raw(8, 6, frame.pixels().to_vec()).unwrap(),
                4,
                3,
                image::imageops::FilterType::Triangle,
            );
            let expected = image::imageops::blur(&expected_small, (radius as f32 / 2.0).max(0.5));
            let mut expected = expected.into_raw();
            for pixel in expected.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            let actual = blurred_image(&frame, radius);
            assert_eq!(
                actual.as_bytes(0),
                Some(expected.as_slice()),
                "radius {radius}"
            );
            assert_eq!(
                blurred_image_from_render(&image, radius).as_bytes(0),
                actual.as_bytes(0),
                "render-image radius {radius}"
            );
        }
    }
}
