use std::sync::{Arc, Mutex, OnceLock};

use core_types::{PhysicalRect, RgbaFrame};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::{OcrError, OcrLine, build_line, build_line_from_words, merge_recognition_overlap};

#[cfg(target_arch = "x86_64")]
use paddleocr_rs_onnx::{OcrEngine as PaddleEngine, OrderBy, TextRegion};

#[cfg(target_arch = "x86_64")]
static PADDLE: OnceLock<Result<Mutex<PaddleEngine>, String>> = OnceLock::new();

/// PaddleOCR confidence at or below this value is treated as icon noise. UI
/// text normally scores above 0.8; low-confidence boxes are usually toolbar
/// glyphs rather than copyable text.
#[cfg(target_arch = "x86_64")]
const PADDLE_MIN_CONFIDENCE: f32 = 0.5;

/// The bundled recognition model has a fixed 48x320 input, so a line wider
/// than about 6.7:1 is squeezed horizontally and its punctuation degrades.
/// Wide blocks are recognized in overlapping chunks instead.
#[cfg(target_arch = "x86_64")]
const PADDLE_REC_ASPECT: f32 = 320.0 / 48.0;

pub(super) fn recognize_text(frame: Arc<RgbaFrame>) -> Result<Vec<OcrLine>, OcrError> {
    std::thread::spawn(move || recognize_on_thread(&frame))
        .join()
        .map_err(|_| OcrError::Platform("text recognition thread panicked".to_owned()))?
}

fn recognize_on_thread(frame: &RgbaFrame) -> Result<Vec<OcrLine>, OcrError> {
    // Windows OCR splits Chinese characters into radicals at the scales this
    // app used to feed it. PaddleOCR's PP-OCR models keep whole glyphs and
    // preserve mixed Latin runs such as `eSIM`; fall back to the system engine
    // on unsupported architectures or when the bundled models cannot load.
    if let Some(Ok(lines)) = recognize_with_paddle(frame)
        && !lines.is_empty()
    {
        return Ok(lines);
    }
    recognize_with_windows(frame)
}

#[cfg(target_arch = "x86_64")]
fn recognize_with_paddle(frame: &RgbaFrame) -> Option<Result<Vec<OcrLine>, OcrError>> {
    static DET: &[u8] = include_bytes!("../assets/ocr/ch_PP-OCRv3_det_infer.onnx");
    static REC: &[u8] = include_bytes!("../assets/ocr/ch_PP-OCRv3_rec_static.onnx");
    static KEYS: &[u8] = include_bytes!("../assets/ocr/ppocr_keys_v1.txt");

    let engine = PADDLE.get_or_init(|| {
        PaddleEngine::new(DET, REC, KEYS)
            .map(Mutex::new)
            .map_err(|error| error.to_string())
    });
    let engine = match engine {
        Ok(engine) => engine,
        Err(_) => return None,
    };
    let image = image::RgbaImage::from_raw(frame.width(), frame.height(), frame.pixels().to_vec())?;
    let image = image::DynamicImage::ImageRgba8(image);
    // Small crops (window titles, file names) lose punctuation detail when the
    // recognizer resizes them to its 48 px input height. Upscaling first keeps
    // strokes such as `.` and `_` separable.
    let (image, scale_x, scale_y) = paddle_source(&image);
    let Ok(engine) = engine.lock() else {
        return None;
    };
    let blocks = match engine.recognize_all(&image, OrderBy::Horizontal) {
        Ok(blocks) => blocks,
        Err(error) => return Some(Err(OcrError::Platform(error.to_string()))),
    };
    let mut lines = Vec::new();
    for block in blocks {
        if block.text.trim().is_empty() {
            continue;
        }
        let (text, confidence) = paddle_block_text(&engine, &image, &block);
        if confidence < PADDLE_MIN_CONFIDENCE || text.trim().is_empty() {
            continue;
        }
        let width = frame.width() as i32;
        let height = frame.height() as i32;
        let left = (block.x * scale_x).round() as i32;
        let top = (block.y * scale_y).round() as i32;
        let right = ((block.x + block.width) * scale_x).round() as i32;
        let bottom = ((block.y + block.height) * scale_y).round() as i32;
        let left = left.clamp(0, width);
        let top = top.clamp(0, height);
        let right = right.clamp(left, width);
        let bottom = bottom.clamp(top, height);
        if right <= left || bottom <= top {
            continue;
        }
        lines.push(build_line(
            text,
            PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32),
        ));
    }
    Some(Ok(lines))
}

/// Upscales small frames before PaddleOCR runs. Full-screen captures are left
/// alone: the detector already resizes to a 960 px long side, and the extra
/// copy would only cost memory.
#[cfg(target_arch = "x86_64")]
fn paddle_upscale(width: u32, height: u32) -> f32 {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels == 0 {
        return 1.0;
    }
    if pixels <= 1_200_000 {
        2.0
    } else if pixels <= 4_000_000 {
        1.5
    } else {
        1.0
    }
}

/// Returns the (possibly upscaled) image plus the factors that map its
/// coordinates back to the original frame.
#[cfg(target_arch = "x86_64")]
fn paddle_source(image: &image::DynamicImage) -> (image::DynamicImage, f32, f32) {
    let upscale = paddle_upscale(image.width(), image.height());
    if upscale <= 1.0 {
        return (image.clone(), 1.0, 1.0);
    }
    let width = ((image.width() as f32 * upscale).round() as u32).max(1);
    let height = ((image.height() as f32 * upscale).round() as u32).max(1);
    let resized =
        image::imageops::resize(image, width, height, image::imageops::FilterType::Lanczos3);
    (
        image::DynamicImage::ImageRgba8(resized),
        image.width() as f32 / width as f32,
        image.height() as f32 / height as f32,
    )
}

/// Recognizes one detected block. Blocks much wider than the recognizer's
/// fixed aspect ratio are cut into overlapping chunks; each chunk keeps a
/// near-native character width instead of being squeezed into 320 px.
#[cfg(target_arch = "x86_64")]
fn paddle_block_text(
    engine: &PaddleEngine,
    image: &image::DynamicImage,
    block: &paddleocr_rs_onnx::OcrBlock,
) -> (String, f32) {
    let width = block.width.max(1.0);
    let height = block.height.max(1.0);
    let aspect = width / height;
    if aspect <= PADDLE_REC_ASPECT * 1.05 || height < 8.0 {
        return (block.text.clone(), block.confidence);
    }
    let chunks = (aspect / PADDLE_REC_ASPECT).ceil().clamp(2.0, 8.0) as usize;
    // Overlap is one character wide (CJK glyphs are roughly square) so a
    // glyph cut by the chunk boundary still appears whole in one of them.
    let overlap = (height * 1.2 / width).clamp(0.01, 0.2);
    let mut text = String::new();
    let mut score_sum = 0.0_f32;
    let mut score_count = 0_u32;
    for index in 0..chunks {
        let start = (index as f32 / chunks as f32 - overlap).max(0.0);
        let end = ((index as f32 + 1.0) / chunks as f32 + overlap).min(1.0);
        let left = block.x + width * start;
        let right = block.x + width * end;
        let region = TextRegion {
            bbox: [
                [left, block.y],
                [right, block.y],
                [right, block.y + height],
                [left, block.y + height],
            ],
            confidence: block.confidence,
        };
        let Ok(decoded) = engine.recognize_text(image, &region) else {
            continue;
        };
        if decoded.text.is_empty() {
            continue;
        }
        text = if text.is_empty() {
            decoded.text
        } else {
            merge_recognition_overlap(&text, &decoded.text)
        };
        score_sum += decoded.score;
        score_count += 1;
    }
    if text.is_empty() || score_count == 0 {
        (block.text.clone(), block.confidence)
    } else {
        (text, score_sum / score_count as f32)
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn recognize_with_paddle(_frame: &RgbaFrame) -> Option<Result<Vec<OcrLine>, OcrError>> {
    None
}

fn recognize_with_windows(frame: &RgbaFrame) -> Result<Vec<OcrLine>, OcrError> {
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
