use std::ptr::NonNull;
use std::sync::Arc;

use core_types::{PhysicalRect, RgbaFrame};
use objc2::rc::Retained;
use objc2::{AnyThread, ClassType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSArray, NSDictionary, NSRange};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRequest,
    VNRequestTextRecognitionLevel,
};

use crate::{OcrChar, OcrError, OcrLine, build_line};

const MAX_VISION_CHARS: usize = 1024;
const MIN_LINE_CONFIDENCE: f32 = 0.5;

fn pixel_buffer(frame: &RgbaFrame) -> Result<Retained<CVPixelBuffer>, OcrError> {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: the output pointer is valid for the call; the returned buffer is +1 retained.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            width,
            height,
            kCVPixelFormatType_32BGRA,
            None,
            NonNull::from(&mut out),
        )
    };
    if status != 0 || out.is_null() {
        return Err(OcrError::Platform(format!(
            "CVPixelBufferCreate failed with status {status}"
        )));
    }
    // SAFETY: creation transferred ownership of the buffer to us.
    let buffer = unsafe { Retained::from_raw(out) }
        .ok_or_else(|| OcrError::Platform("invalid pixel buffer".to_owned()))?;
    // SAFETY: the buffer is exclusively owned and unlocked during the copy.
    unsafe {
        if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()) != 0 {
            return Err(OcrError::Platform(
                "failed to lock the pixel buffer".to_owned(),
            ));
        }
        let row_bytes = CVPixelBufferGetBytesPerRow(&buffer);
        let base = CVPixelBufferGetBaseAddress(&buffer) as *mut u8;
        let packed = frame.packed_pixels();
        for y in 0..height {
            let target = base.add(y * row_bytes);
            for x in 0..width {
                let source = &packed[(y * width + x) * 4..(y * width + x) * 4 + 4];
                *target.add(x * 4) = source[2];
                *target.add(x * 4 + 1) = source[1];
                *target.add(x * 4 + 2) = source[0];
                *target.add(x * 4 + 3) = source[3];
            }
        }
        CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
    }
    Ok(buffer)
}

pub(super) fn recognize_text(frame: Arc<RgbaFrame>) -> Result<Vec<OcrLine>, OcrError> {
    let buffer = pixel_buffer(&frame)?;
    let options = NSDictionary::new();
    let handler = unsafe {
        VNImageRequestHandler::initWithCVPixelBuffer_options(
            VNImageRequestHandler::alloc(),
            &buffer,
            &options,
        )
    };

    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    request.setAutomaticallyDetectsLanguage(true);
    let requests = NSArray::from_slice(&[request.as_super() as &VNRequest]);
    handler
        .performRequests_error(&requests)
        .map_err(|error| OcrError::Platform(error.localizedDescription().to_string()))?;

    let mut lines = Vec::new();
    let frame_width = f64::from(frame.width());
    let frame_height = f64::from(frame.height());
    if let Some(results) = request.results() {
        for observation in results.iter() {
            let Some(candidate) = observation.topCandidates(1).iter().next() else {
                continue;
            };
            // SAFETY: both accessors read from a live result returned by Vision.
            let confidence = unsafe { observation.confidence() };
            let bounds = unsafe { observation.boundingBox() };
            if confidence < MIN_LINE_CONFIDENCE {
                continue;
            }
            if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
                continue;
            }
            let rect = normalized_rect(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
                frame_width,
                frame_height,
            );
            lines.push(build_vision_line(
                &candidate,
                candidate.string().to_string(),
                rect,
                frame_width,
                frame_height,
            ));
        }
    }
    lines.sort_by_key(|line| (line.rect.origin.y, line.rect.origin.x));
    Ok(lines)
}

fn normalized_rect(
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    frame_width: f64,
    frame_height: f64,
) -> PhysicalRect {
    let left = origin_x * frame_width;
    let top = (1.0 - origin_y - height) * frame_height;
    PhysicalRect::new(
        left.round() as i32,
        top.round() as i32,
        (width * frame_width).round().max(1.0) as u32,
        (height * frame_height).round().max(1.0) as u32,
    )
}

fn build_vision_line(
    candidate: &VNRecognizedText,
    text: String,
    rect: PhysicalRect,
    frame_width: f64,
    frame_height: f64,
) -> OcrLine {
    let text = text.trim_end();
    if text.is_empty() || text.chars().count() > MAX_VISION_CHARS {
        return build_line(text.to_owned(), rect);
    }
    let mut chars = Vec::with_capacity(text.chars().count());
    let mut location = 0_usize;
    for ch in text.chars() {
        let range = NSRange::new(location, ch.len_utf16());
        location += ch.len_utf16();
        // SAFETY: the range covers one character of the candidate string.
        let Ok(observation) = (unsafe { candidate.boundingBoxForRange_error(range) }) else {
            return build_line(text.to_owned(), rect);
        };
        // SAFETY: the observation is a live result returned by Vision.
        let bounds = unsafe { observation.boundingBox() };
        let char_rect = normalized_rect(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bounds.size.height,
            frame_width,
            frame_height,
        );
        chars.push(OcrChar {
            ch,
            rect: char_rect,
        });
    }
    if !vision_chars_are_usable(&chars, rect) {
        return build_line(text.to_owned(), rect);
    }
    OcrLine {
        text: text.to_owned(),
        rect,
        chars,
    }
}

fn vision_chars_are_usable(chars: &[OcrChar], line: PhysicalRect) -> bool {
    let visible: Vec<&OcrChar> = chars
        .iter()
        .filter(|item| !item.ch.is_whitespace())
        .collect();
    if visible.len() <= 1 {
        return true;
    }
    let min_x = visible
        .iter()
        .map(|item| item.rect.origin.x)
        .min()
        .unwrap_or(0);
    let max_x = visible
        .iter()
        .map(|item| item.rect.origin.x + item.rect.size.width as i32)
        .max()
        .unwrap_or(0);
    if max_x <= min_x {
        return false;
    }
    let line_width = line.size.width.max(1) as f32;
    let widest = visible
        .iter()
        .map(|item| item.rect.size.width as f32)
        .fold(0.0_f32, f32::max);
    widest <= line_width * 0.9
}
