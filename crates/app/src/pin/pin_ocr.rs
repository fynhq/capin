use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use super::*;
use crate::logging::{log_debug, log_error};
use crate::ocr_queue::SubmitError;

impl PinView {
    fn ocr_source(&mut self) -> Option<crate::ocr_queue::Source> {
        if let Some(path) = self.ocr_path.clone() {
            return Some(crate::ocr_queue::Source::Path(path));
        }
        if let Some(frame) = self.content.frame_arc() {
            Some(crate::ocr_queue::Source::Frame(frame))
        } else {
            self.persisted_path().map(crate::ocr_queue::Source::Path)
        }
    }

    pub(super) fn release_frame_if_persisted(&mut self) {
        if self.persisted_path().is_some() {
            self.content.drop_frame();
        }
    }

    pub(super) fn wake_ocr(&mut self, cx: &mut Context<Self>) {
        if self.ocr_deferred {
            self.start_ocr(cx);
        }
    }

    pub(super) fn start_ocr(&mut self, cx: &mut Context<Self>) {
        self.ocr_deferred = false;
        let settings = settings_state::get();
        if !settings.pin_text_ocr && !settings.pin_qr {
            self.ocr = None;
            self.release_frame_if_persisted();
            return;
        }
        let Some(source) = self.ocr_source() else {
            self.ocr = None;
            return;
        };
        let options = platform_ocr::OcrOptions {
            text: settings.pin_text_ocr,
            qr: settings.pin_qr,
        };
        let generation = self.content_generation;
        match crate::ocr_queue::submit(source, options) {
            Ok(receiver) => {
                self.ocr_path = None;
                cx.spawn(async move |this, cx| {
                    loop {
                        match receiver.try_recv() {
                            Ok(output) => {
                                this.update_in(cx, |this, window, cx| {
                                    if this.content_generation == generation {
                                        this.ocr = Some(output);
                                        this.hover_qr = this.hover
                                            && this.qr_hit(
                                                window.mouse_position(),
                                                window.viewport_size(),
                                            );
                                        this.release_frame_if_persisted();
                                        cx.notify();
                                    }
                                })
                                .ok();
                                return;
                            }
                            Err(TryRecvError::Empty) => {
                                let current = this
                                    .update(cx, |this, _| this.content_generation == generation)
                                    .unwrap_or(false);
                                if !current {
                                    return;
                                }
                                cx.background_executor()
                                    .timer(Duration::from_millis(50))
                                    .await;
                            }
                            Err(TryRecvError::Disconnected) => {
                                log_error!("ocr", "the OCR queue disconnected before replying");
                                return;
                            }
                        }
                    }
                })
                .detach();
                self.release_frame_if_persisted();
            }
            Err(SubmitError::Busy) => {
                log_debug!("ocr", "OCR queue is busy; deferring pin {}", self.id);
                self.ocr = None;
                self.ocr_deferred = true;
                self.release_frame_if_persisted();
            }
            Err(SubmitError::Unavailable) => {
                log_error!("ocr", "the OCR queue is unavailable");
                self.ocr = None;
                self.release_frame_if_persisted();
            }
        }
    }
}
