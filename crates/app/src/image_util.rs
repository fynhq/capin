use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use core_types::RgbaFrame;
use gpui_kit::*;
use image::ImageEncoder;

pub fn render_image(frame: &RgbaFrame) -> Arc<RenderImage> {
    let width = frame.width();
    let height = frame.height();
    let mut pixels = frame.pixels().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }

    // GPUI interprets RenderImage bytes as BGRA; only the byte order matters.
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)
        .expect("capture frames are validated before conversion");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// Decodes an embedded PNG (the app icon) into a GPUI render image. The SVG
/// artwork uses masks, filters and gradients that GPUI's SVG renderer does not
/// implement, so settings must use the raster asset.
pub fn render_png(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// Builds a nearest-neighbour magnified crop around `center` for the overlay
/// magnifier. `center` is in frame-local physical pixels.
pub fn magnifier_image(
    frame: &RgbaFrame,
    center_x: i32,
    center_y: i32,
    source_size: u32,
    output_size: u32,
) -> Arc<RenderImage> {
    let output_size = output_size.max(1);
    let source_size = source_size.max(1);
    let half_source = source_size as i32 / 2;
    let mut pixels = vec![0u8; output_size as usize * output_size as usize * 4];

    for output_y in 0..output_size {
        let source_y = center_y - half_source
            + (i64::from(output_y) * i64::from(source_size) / i64::from(output_size)) as i32;
        for output_x in 0..output_size {
            let source_x = center_x - half_source
                + (i64::from(output_x) * i64::from(source_size) / i64::from(output_size)) as i32;
            let target = (output_y as usize * output_size as usize + output_x as usize) * 4;
            let Some([red, green, blue, alpha]) = frame
                .pixel(source_x.max(0) as u32, source_y.max(0) as u32)
                .filter(|_| source_x >= 0 && source_y >= 0)
            else {
                continue;
            };
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

/// Builds the downscaled preview for a box-blur radius. A radius of 16 maps
/// to the historical preview sigma of 2.0 at one eighth resolution.
pub fn blurred_image(frame: &RgbaFrame, radius: u32) -> Arc<RenderImage> {
    let width = frame.width();
    let height = frame.height();
    let source = image::RgbaImage::from_raw(width, height, frame.pixels().to_vec())
        .expect("capture frames are validated before conversion");
    let small_width = (width / 8).max(1);
    let small_height = (height / 8).max(1);
    let small = image::imageops::resize(
        &source,
        small_width,
        small_height,
        image::imageops::FilterType::Triangle,
    );
    let sigma = (radius.max(1) as f32 / 8.0).max(0.5);
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

pub fn copy_png(frame: &RgbaFrame, cx: &mut App) -> Result<(), image::ImageError> {
    let bytes = encode_png(frame)?;
    let image = Image::from_bytes(ImageFormat::Png, bytes);
    cx.write_to_clipboard(ClipboardItem::new_image(&image));
    Ok(())
}

pub fn save_png(frame: &RgbaFrame) -> Result<PathBuf, std::io::Error> {
    crate::storage::save_png(frame)
}
