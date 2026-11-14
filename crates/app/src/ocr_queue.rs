//! Serializes pin OCR and QR work.
//!
//! Restoring many pins used to queue one full-image OCR job per pin at once.
//! Frames are shared now, and a pin whose CPU frame was released sends its
//! saved PNG path instead, so only one recognition and one decode run at a
//! time.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use core_types::RgbaFrame;
use platform_ocr::{OcrOptions, OcrOutput};

static OCR_LOCK: Mutex<()> = Mutex::new(());

pub(crate) enum Source {
    Frame(Arc<RgbaFrame>),
    Path(PathBuf),
}

pub(crate) fn recognize(source: Source, options: OcrOptions) -> OcrOutput {
    let _guard = OCR_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let frame = match source {
        Source::Frame(frame) => frame,
        Source::Path(path) => match crate::image_util::load_frame(&path) {
            Some(frame) => Arc::new(frame),
            None => {
                return OcrOutput {
                    error: Some("the pinned image could not be reloaded".to_owned()),
                    ..Default::default()
                };
            }
        },
    };
    platform_ocr::recognize_shared(frame, options)
}
