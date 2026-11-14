use std::sync::Arc;

use core_types::{PhysicalRect, RgbaFrame};
use objc2::{AnyThread, ClassType};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSRange};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRequest,
    VNRequestTextRecognitionLevel,
};

use crate::{OcrChar, OcrError, OcrLine, build_line};

/// Per-character Vision boxes are only fetched for reasonably sized lines.
/// Longer paragraphs keep the distributed layout so OCR does not spend more
/// time on UI boxes than on recognition.
const MAX_VISION_CHARS: usize = 1024;

pub(super) fn recognize_text(frame: Arc<RgbaFrame>) -> Result<Vec<OcrLine>, OcrError> {
    let image = image::RgbaImage::from_raw(
        frame.width(),
        frame.height(),
        frame.packed_pixels().into_owned(),
    )
    .ok_or_else(|| OcrError::Platform("invalid image buffer".to_owned()))?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| OcrError::Platform(error.to_string()))?;

    let data = NSData::with_bytes(&png);
    let options = NSDictionary::new();
    let handler = VNImageRequestHandler::initWithData_options(
        VNImageRequestHandler::alloc(),
        &data,
        &options,
    );

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
            // SAFETY: the observation is a live result returned by Vision.
            let bounds = unsafe { observation.boundingBox() };
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

/// Vision has returned line-sized boxes for every character on some systems.
/// Detect that shape and fall back to the distributed layout instead of
/// breaking text selection.
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
