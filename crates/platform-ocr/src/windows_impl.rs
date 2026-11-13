use std::sync::{Arc, Mutex, OnceLock};

use core_types::{PhysicalRect, RgbaFrame};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::merge_recognition_overlap;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::tiling::{Rect, TILE, merge_rects, sort_rects, tile_rects};
use crate::{OcrError, OcrLine, build_line, build_line_from_words};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use paddleocr_rs_onnx::{OcrEngine as PaddleEngine, TextRegion};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
static PADDLE: OnceLock<Result<Mutex<PaddleEngine>, String>> = OnceLock::new();

/// PaddleOCR confidence at or below this value is treated as icon noise. UI
/// text normally scores above 0.8; low-confidence boxes are usually toolbar
/// glyphs rather than copyable text.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const PADDLE_MIN_CONFIDENCE: f32 = 0.5;

/// PP-OCRv6 accepts a dynamic recognition width, but extremely wide lines
/// still produce a huge input tensor. Lines above this aspect ratio are
/// recognized in overlapping chunks, each close to the model's training width.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const PADDLE_REC_MAX_ASPECT: f32 = 64.0;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const PADDLE_REC_CHUNK_ASPECT: f32 = 48.0;

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

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn recognize_with_paddle(frame: &RgbaFrame) -> Option<Result<Vec<OcrLine>, OcrError>> {
    static DET: &[u8] = include_bytes!("../assets/ocr/PP-OCRv6_tiny_det.onnx");
    static REC: &[u8] = include_bytes!("../assets/ocr/PP-OCRv6_tiny_rec.onnx");
    static KEYS: &[u8] = include_bytes!("../assets/ocr/ppocrv6_dict.txt");

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
    // Small crops (window titles, file names) gain stroke detail when
    // upscaled. Large screenshots are tiled instead so the detector never
    // shrinks 4K UI text to a few pixels.
    let (width, height) = (image.width(), image.height());
    let tiled = width.max(height) > TILE;
    let (source, scale_x, scale_y) = if tiled {
        (image, 1.0, 1.0)
    } else {
        paddle_source(&image)
    };
    let Ok(engine) = engine.lock() else {
        return None;
    };
    let mut regions = match detect_regions(&engine, &source, tiled) {
        Ok(regions) => regions,
        Err(error) => return Some(Err(error)),
    };
    // A tight crop of a single line can have no room for the detector's
    // unclip margin. Keep the old whole-image recognition fallback for small
    // images; large images stay tiled and fall back to the system engine.
    if regions.is_empty() && !tiled {
        let (width, height) = (source.width() as f32, source.height() as f32);
        regions.push(TextRegion {
            bbox: [[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
            confidence: 1.0,
        });
    }
    let lines = recognize_regions(
        &engine,
        &source,
        regions,
        scale_x,
        scale_y,
        frame.width(),
        frame.height(),
    );
    Some(Ok(lines))
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn detect_regions(
    engine: &PaddleEngine,
    image: &image::DynamicImage,
    tiled: bool,
) -> Result<Vec<TextRegion>, OcrError> {
    let (width, height) = (image.width(), image.height());
    let regions = if tiled {
        detect_tiled(engine, image, width, height)?
    } else {
        engine
            .detect_text_regions(image)
            .map_err(|error| OcrError::Platform(error.to_string()))?
    };
    let mut rects: Vec<Rect> = regions.iter().map(region_to_rect).collect();
    merge_rects(&mut rects);
    sort_rects(&mut rects);
    Ok(rects
        .into_iter()
        .filter_map(|rect| rect_to_region(rect, width, height))
        .collect())
}

/// Runs detection on overlapping tiles and maps the boxes back to the source
/// image. A failed tile is skipped unless every tile fails, so one bad tile
/// does not discard the rest of the screenshot.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn detect_tiled(
    engine: &PaddleEngine,
    image: &image::DynamicImage,
    width: u32,
    height: u32,
) -> Result<Vec<TextRegion>, OcrError> {
    let tiles = tile_rects(width, height);
    let tile_count = tiles.len();
    let mut regions = Vec::new();
    let mut failures = 0u32;
    for (x, y, tile_width, tile_height) in tiles {
        let tile = image.crop_imm(x, y, tile_width, tile_height);
        match engine.detect_text_regions(&tile) {
            Ok(found) => {
                for region in found {
                    let mut bbox = region.bbox;
                    for point in &mut bbox {
                        point[0] += x as f32;
                        point[1] += y as f32;
                    }
                    regions.push(TextRegion {
                        bbox,
                        confidence: region.confidence,
                    });
                }
            }
            Err(_) => failures += 1,
        }
    }
    if failures == tile_count as u32 {
        return Err(OcrError::Platform(
            "text detection failed on every tile".to_owned(),
        ));
    }
    Ok(regions)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn recognize_regions(
    engine: &PaddleEngine,
    image: &image::DynamicImage,
    regions: Vec<TextRegion>,
    scale_x: f32,
    scale_y: f32,
    frame_width: u32,
    frame_height: u32,
) -> Vec<OcrLine> {
    let mut lines = Vec::with_capacity(regions.len());
    for region in regions {
        let Some((text, confidence)) = recognize_region(engine, image, &region) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() || confidence < PADDLE_MIN_CONFIDENCE {
            continue;
        }
        let Some(rect) = physical_rect(&region, scale_x, scale_y, frame_width, frame_height) else {
            continue;
        };
        lines.push(build_line(text.to_owned(), rect));
    }
    lines
}

/// Recognizes one region, splitting extremely wide lines into overlapping
/// chunks so the recognizer keeps a native character width.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn recognize_region(
    engine: &PaddleEngine,
    image: &image::DynamicImage,
    region: &TextRegion,
) -> Option<(String, f32)> {
    let rect = region_to_rect(region);
    let width = rect.width();
    let height = rect.height();
    if width < 2.0 || height < 2.0 {
        return None;
    }
    let aspect = width / height;
    if aspect <= PADDLE_REC_MAX_ASPECT {
        let decoded = engine.recognize_text(image, region).ok()?;
        return Some((decoded.text, decoded.score));
    }
    let chunks = ((aspect / PADDLE_REC_CHUNK_ASPECT).ceil() as usize).clamp(2, 16);
    // One character of overlap keeps a glyph cut by the chunk boundary whole.
    let overlap = (height * 1.2 / width).clamp(0.005, 0.2);
    let mut text = String::new();
    let mut weighted_score = 0.0_f32;
    let mut char_count = 0_u32;
    let mut decoded_chunks = 0_u32;
    for index in 0..chunks {
        let start = (index as f32 / chunks as f32 - overlap).max(0.0);
        let end = ((index as f32 + 1.0) / chunks as f32 + overlap).min(1.0);
        let chunk = TextRegion {
            bbox: [
                [rect.left + width * start, rect.top],
                [rect.left + width * end, rect.top],
                [rect.left + width * end, rect.bottom],
                [rect.left + width * start, rect.bottom],
            ],
            confidence: region.confidence,
        };
        let Ok(decoded) = engine.recognize_text(image, &chunk) else {
            continue;
        };
        let chunk_text = decoded.text;
        if chunk_text.is_empty() {
            continue;
        }
        let count = chunk_text.chars().count().max(1) as u32;
        text = if text.is_empty() {
            chunk_text
        } else {
            merge_recognition_overlap(&text, &chunk_text)
        };
        weighted_score += decoded.score * count as f32;
        char_count += count;
        decoded_chunks += 1;
    }
    if text.is_empty() || decoded_chunks == 0 {
        let decoded = engine.recognize_text(image, region).ok()?;
        return Some((decoded.text, decoded.score));
    }
    Some((text, weighted_score / char_count.max(1) as f32))
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn region_to_rect(region: &TextRegion) -> Rect {
    let (mut left, mut top) = (f32::INFINITY, f32::INFINITY);
    let (mut right, mut bottom) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for point in region.bbox {
        left = left.min(point[0]);
        top = top.min(point[1]);
        right = right.max(point[0]);
        bottom = bottom.max(point[1]);
    }
    Rect::new(left, top, right, bottom, region.confidence)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn rect_to_region(rect: Rect, width: u32, height: u32) -> Option<TextRegion> {
    let left = rect.left.max(0.0).min(width as f32);
    let top = rect.top.max(0.0).min(height as f32);
    let right = rect.right.max(0.0).min(width as f32);
    let bottom = rect.bottom.max(0.0).min(height as f32);
    if right - left < 2.0 || bottom - top < 2.0 {
        return None;
    }
    Some(TextRegion {
        bbox: [[left, top], [right, top], [right, bottom], [left, bottom]],
        confidence: rect.confidence,
    })
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn physical_rect(
    region: &TextRegion,
    scale_x: f32,
    scale_y: f32,
    frame_width: u32,
    frame_height: u32,
) -> Option<PhysicalRect> {
    let rect = region_to_rect(region);
    let width = frame_width as i32;
    let height = frame_height as i32;
    let left = (rect.left * scale_x).round() as i32;
    let top = (rect.top * scale_y).round() as i32;
    let right = (rect.right * scale_x).round() as i32;
    let bottom = (rect.bottom * scale_y).round() as i32;
    let left = left.clamp(0, width);
    let top = top.clamp(0, height);
    let right = right.clamp(left, width);
    let bottom = bottom.clamp(top, height);
    if right <= left || bottom <= top {
        return None;
    }
    Some(PhysicalRect::new(
        left,
        top,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}

/// Upscales small frames before detection and recognition. The detector caps
/// its input at a 960 px long side, so small crops still benefit from extra
/// pixels; large frames go through tiled detection instead.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
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
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
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

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
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
