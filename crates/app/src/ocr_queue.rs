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
