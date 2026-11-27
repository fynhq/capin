use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use core_types::RgbaFrame;
use platform_ocr::{OcrOptions, OcrOutput};

use crate::logging::{log_debug, log_error};

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

const OCR_QUEUE_CAPACITY: usize = 2;

pub(crate) enum Source {
    Frame(Arc<RgbaFrame>),
    Path(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubmitError {
    Busy,
    Unavailable,
}

struct OcrJob {
    source: Source,
    options: OcrOptions,
    reply: mpsc::Sender<OcrOutput>,
}

static OCR_WORKER: OnceLock<Option<SyncSender<OcrJob>>> = OnceLock::new();

pub(crate) fn submit(
    source: Source,
    options: OcrOptions,
) -> Result<Receiver<OcrOutput>, SubmitError> {
    let worker = worker()?;
    let (reply, receiver) = mpsc::channel();
    match worker.try_send(OcrJob {
        source,
        options,
        reply,
    }) {
        Ok(()) => Ok(receiver),
        Err(TrySendError::Full(_)) => Err(SubmitError::Busy),
        Err(TrySendError::Disconnected(_)) => Err(SubmitError::Unavailable),
    }
}

fn worker() -> Result<&'static SyncSender<OcrJob>, SubmitError> {
    OCR_WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<OcrJob>(OCR_QUEUE_CAPACITY);
            match std::thread::Builder::new()
                .name("capin-ocr-queue".to_owned())
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        let output = recognize(job.source, job.options);
                        let _ = job.reply.send(output);
                    }
                }) {
                Ok(_) => Some(sender),
                Err(error) => {
                    log_error!("ocr", "failed to start the OCR queue: {error}");
                    None
                }
            }
        })
        .as_ref()
        .ok_or(SubmitError::Unavailable)
}

fn recognize(source: Source, options: OcrOptions) -> OcrOutput {
    touch_ocr();
    let started = Instant::now();
    let source_label = match &source {
        Source::Frame(_) => "frame",
        Source::Path(_) => "path",
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_returns_a_result_without_blocking_the_caller() {
        let frame = Arc::new(RgbaFrame::from_pixels(2, 2, vec![0; 16]).unwrap());
        let receiver = submit(
            Source::Frame(frame),
            OcrOptions {
                text: false,
                qr: false,
            },
        )
        .expect("OCR queue should accept the first job");
        let output = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("OCR worker should answer");
        assert!(output.error.is_none());
        assert!(output.lines.is_empty());
    }
}
