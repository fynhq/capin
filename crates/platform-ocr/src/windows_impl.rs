use std::sync::Arc;

use core_types::{PhysicalRect, RgbaFrame};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::{OcrError, OcrLine, build_line, build_line_from_words};

pub(super) fn recognize_text(frame: Arc<RgbaFrame>) -> Result<Vec<OcrLine>, OcrError> {
    std::thread::spawn(move || recognize_on_thread(&frame))
        .join()
        .map_err(|_| OcrError::Platform("text recognition thread panicked".to_owned()))?
}

fn recognize_on_thread(frame: &RgbaFrame) -> Result<Vec<OcrLine>, OcrError> {
    // SAFETY: initializing a fresh apartment on this worker thread is safe;
    // a pre-initialized apartment only changes the returned HRESULT.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(platform_error("TryCreateFromUserProfileLanguages"))?;
    // Windows OCR misreads small UI text (0/l/1, e/0, 5/6) at native scale.
    // Text crops are often only a few hundred pixels wide, so scale them up
    // before recognition and map the returned boxes back to the original
    // pixels. Keep the result inside the engine's maximum bitmap dimension.
    let upscale = ocr_upscale(
        frame.width(),
        frame.height(),
        OcrEngine::MaxImageDimension().unwrap_or(0),
    );
    let (bgra, width, height) = ocr_bitmap(frame, upscale);
    // Map boxes back with the actual bitmap ratio, not the nominal factor:
    // rounding the upscaled dimensions would otherwise shift every box.
    let scale_x = frame.width() as f32 / width.max(1) as f32;
    let scale_y = frame.height() as f32 / height.max(1) as f32;

    let buffer = CryptographicBuffer::CreateFromByteArray(&bgra)
        .map_err(platform_error("CreateFromByteArray"))?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)
            .map_err(platform_error("CreateCopyFromBuffer"))?;
    let operation = engine
        .RecognizeAsync(&bitmap)
        .map_err(platform_error("RecognizeAsync"))?;
    let result = pollster::block_on(operation).map_err(platform_error("RecognizeAsync"))?;
    let lines = result.Lines().map_err(platform_error("Lines"))?;
    let count = lines.Size().map_err(platform_error("Lines.Size"))?;

    let mut result = Vec::new();
    for index in 0..count {
        let line = lines.GetAt(index).map_err(platform_error("Lines.GetAt"))?;
        let value = line
            .Text()
            .map_err(platform_error("OcrLine.Text"))?
            .to_string();
        let words = line.Words().map_err(platform_error("OcrLine.Words"))?;
        let word_count = words.Size().map_err(platform_error("Words.Size"))?;
        let mut word_items: Vec<(String, PhysicalRect)> = Vec::new();
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for word_index in 0..word_count {
            let word = words
                .GetAt(word_index)
                .map_err(platform_error("Words.GetAt"))?;
            let rect = word
                .BoundingRect()
                .map_err(platform_error("OcrWord.BoundingRect"))?;
            let text = word
                .Text()
                .map_err(platform_error("OcrWord.Text"))?
                .to_string();
            let (left, top) = (rect.X * scale_x, rect.Y * scale_y);
            let (right, bottom) = (
                (rect.X + rect.Width) * scale_x,
                (rect.Y + rect.Height) * scale_y,
            );
            bounds = Some(match bounds {
                Some((x0, y0, x1, y1)) => {
                    (x0.min(left), y0.min(top), x1.max(right), y1.max(bottom))
                }
                None => (left, top, right, bottom),
            });
            word_items.push((
                text,
                PhysicalRect::new(
                    left.max(0.0).round() as i32,
                    top.max(0.0).round() as i32,
                    (right - left).max(1.0).round() as u32,
                    (bottom - top).max(1.0).round() as u32,
                ),
            ));
        }
        if let Some(line) = build_line_from_words(&word_items) {
            result.push(line);
        } else if let Some((left, top, right, bottom)) = bounds {
            let rect = PhysicalRect::new(
                left.max(0.0).round() as i32,
                top.max(0.0).round() as i32,
                (right - left).max(1.0).round() as u32,
                (bottom - top).max(1.0).round() as u32,
            );
            result.push(build_line(value, rect));
        }
    }
    Ok(result)
}

/// Picks an integer upscale factor for OCR. Full-screen captures are left
/// alone; smaller crops gain enough stroke detail for Windows OCR to separate
/// characters such as `0` from `e`, `l` from `1`, and `5` from `6`. When the
/// engine reports a maximum bitmap dimension, the factor is lowered so the
/// upscaled bitmap still fits.
fn ocr_upscale(width: u32, height: u32, max_dimension: u32) -> f32 {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels == 0 {
        return 1.0;
    }
    let preferred = if pixels <= 300_000 {
        4.0
    } else if pixels <= 1_200_000 {
        3.0
    } else if pixels <= 4_000_000 {
        2.0
    } else {
        1.0
    };
    if max_dimension == 0 || width == 0 || height == 0 {
        return preferred;
    }
    let fit = (max_dimension as f32 / width.max(1) as f32)
        .min(max_dimension as f32 / height.max(1) as f32);
    preferred.min(fit).max(1.0)
}

/// Returns BGRA bytes plus the bitmap dimensions after the upscale pass.
fn ocr_bitmap(frame: &RgbaFrame, upscale: f32) -> (Vec<u8>, i32, i32) {
    if upscale <= 1.0 {
        let mut bgra = frame.pixels().to_vec();
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        return (bgra, frame.width() as i32, frame.height() as i32);
    }
    let width = (frame.width() as f32 * upscale).round().max(1.0) as u32;
    let height = (frame.height() as f32 * upscale).round().max(1.0) as u32;
    let Some(rgba) =
        image::RgbaImage::from_raw(frame.width(), frame.height(), frame.pixels().to_vec())
    else {
        let mut bgra = frame.pixels().to_vec();
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        return (bgra, frame.width() as i32, frame.height() as i32);
    };
    let resized =
        image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Lanczos3);
    let mut bgra = resized.into_raw();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    (bgra, width as i32, height as i32)
}

fn platform_error(operation: &'static str) -> impl FnOnce(windows::core::Error) -> OcrError {
    move |error| OcrError::Platform(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::ocr_upscale;

    #[test]
    fn small_text_crops_are_upscaled() {
        assert_eq!(ocr_upscale(846, 62, 0), 4.0);
        assert_eq!(ocr_upscale(1323, 1020, 0), 3.0);
        assert_eq!(ocr_upscale(3840, 2160, 0), 1.0);
        assert_eq!(ocr_upscale(0, 0, 0), 1.0);
    }

    #[test]
    fn upscale_respects_engine_maximum_dimension() {
        let fitted = ocr_upscale(846, 62, 2000);
        assert!((fitted - 2000.0 / 846.0).abs() < 0.001);
        assert_eq!(ocr_upscale(9000, 5000, 10000), 1.0);
    }
}
