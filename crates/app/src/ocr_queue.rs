use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use core_types::RgbaFrame;
use platform_ocr::{OcrOptions, OcrOutput};

use crate::logging::log_debug;

static OCR_LOCK: Mutex<()> = Mutex::new(());

pub(crate) enum Source {
    Frame(Arc<RgbaFrame>),
    Path(PathBuf),
}

pub(crate) fn recognize(source: Source, options: OcrOptions) -> OcrOutput {
    let started = Instant::now();
    let source_label = match &source {
        Source::Frame(_) => "frame",
        Source::Path(_) => "path",
    };
    let _guard = OCR_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let frame = match source {
        Source::Frame(frame) => frame,
        Source::Path(path) => match crate::image_util::load_frame(&path) {
            Some(frame) => Arc::new(frame),
            None => {
                log_debug!(
                    "ocr",
                    "recognize source={source_label} duration_ms={} failed=reload",
                    started.elapsed().as_millis()
                );
                return OcrOutput {
                    error: Some("the pinned image could not be reloaded".to_owned()),
                    ..Default::default()
                };
            }
        },
    };
    let output = platform_ocr::recognize_shared(frame, options);
    log_debug!(
        "ocr",
        "recognize source={source_label} duration_ms={} lines={} qr={} failed={}",
        started.elapsed().as_millis(),
        output.lines.len(),
        output.qr_codes.len(),
        output.error.is_some()
    );
    output
}
