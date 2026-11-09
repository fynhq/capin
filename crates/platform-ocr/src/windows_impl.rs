use core_types::{PhysicalRect, RgbaFrame};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::{OcrError, OcrLine, build_line, build_line_from_words};

pub(super) fn recognize_text(frame: &RgbaFrame) -> Result<Vec<OcrLine>, OcrError> {
    let frame = frame.clone();
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

    let mut bgra = frame.pixels().to_vec();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }

    let buffer = CryptographicBuffer::CreateFromByteArray(&bgra)
        .map_err(platform_error("CreateFromByteArray"))?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        frame.width() as i32,
        frame.height() as i32,
    )
    .map_err(platform_error("CreateCopyFromBuffer"))?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(platform_error("TryCreateFromUserProfileLanguages"))?;
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
            let (left, top) = (rect.X, rect.Y);
            let (right, bottom) = (rect.X + rect.Width, rect.Y + rect.Height);
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

fn platform_error(operation: &'static str) -> impl FnOnce(windows::core::Error) -> OcrError {
    move |error| OcrError::Platform(format!("{operation}: {error}"))
}
