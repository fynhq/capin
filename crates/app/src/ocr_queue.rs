use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use core_types::RgbaFrame;
use platform_ocr::{OcrOptions, OcrOutput};

use crate::logging::log_debug;

static OCR_LOCK: Mutex<()> = Mutex::new(());

#[cfg(target_os = "windows")]
mod idle_release {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::logging::log_debug;

    static LAST_OCR_MS: AtomicU64 = AtomicU64::new(0);
    static RELEASE_WATCH: OnceLock<()> = OnceLock::new();

    const OCR_IDLE_RELEASE: Duration = Duration::from_secs(300);
    const OCR_RELEASE_CHECK: Duration = Duration::from_secs(30);

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    pub(super) fn touch() {
        LAST_OCR_MS.store(now_ms(), Ordering::SeqCst);
        RELEASE_WATCH.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("capin-ocr-idle".to_owned())
                .spawn(|| {
                    loop {
                        std::thread::sleep(OCR_RELEASE_CHECK);
                        let last = LAST_OCR_MS.load(Ordering::SeqCst);
                        if last != 0
                            && now_ms().saturating_sub(last) >= OCR_IDLE_RELEASE.as_millis() as u64
                            && platform_ocr::release_idle_engine()
                        {
                            log_debug!("ocr", "released idle engine");
                        }
                    }
                });
        });
    }
}

#[cfg(target_os = "windows")]
use idle_release::touch as touch_ocr;

#[cfg(not(target_os = "windows"))]
fn touch_ocr() {}

pub(crate) enum Source {
    Frame(Arc<RgbaFrame>),
    Path(PathBuf),
}

pub(crate) fn recognize(source: Source, options: OcrOptions) -> OcrOutput {
    touch_ocr();
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
