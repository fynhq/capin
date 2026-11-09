use std::sync::Arc;

use core_types::{PhysicalRect, RgbaFrame};
use objc2::{AnyThread, ClassType};
use objc2_foundation::{NSArray, NSData, NSDictionary};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};

use crate::{OcrError, OcrLine, build_line};

pub(super) fn recognize_text(frame: Arc<RgbaFrame>) -> Result<Vec<OcrLine>, OcrError> {
    let image = image::RgbaImage::from_raw(frame.width(), frame.height(), frame.pixels().to_vec())
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
            let left = bounds.origin.x * frame_width;
            let top = (1.0 - bounds.origin.y - bounds.size.height) * frame_height;
            let width = bounds.size.width * frame_width;
            let height = bounds.size.height * frame_height;
            if width < 1.0 || height < 1.0 {
                continue;
            }
            lines.push(build_line(
                candidate.string().to_string(),
                PhysicalRect::new(
                    left.round() as i32,
                    top.round() as i32,
                    width.round() as u32,
                    height.round() as u32,
                ),
            ));
        }
    }
    lines.sort_by_key(|line| (line.rect.origin.y, line.rect.origin.x));
    Ok(lines)
}
