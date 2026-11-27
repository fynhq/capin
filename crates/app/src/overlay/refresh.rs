use std::time::{Duration, Instant};

use core_capture::{CaptureBackend, CaptureTarget, SelectionSession};
use core_types::{MonitorId, PhysicalRect, PhysicalSize, RgbaFrame, WindowId};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_capture::platform_backend;

use crate::i18n::t;
use crate::image_util;
use crate::logging::log_warn;
use crate::settings_state;

use super::{OverlayView, live_overlay_window_ids_excluding};

impl OverlayView {
    pub(super) fn refresh_frame_action(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_frame(cx);
    }

    pub(super) fn refresh_frame(&mut self, cx: &mut Context<Self>) {
        if self.refresh_in_flight
            || self
                .last_refresh
                .is_some_and(|last| last.elapsed() < Duration::from_millis(200))
        {
            return;
        }
        self.last_refresh = Some(Instant::now());
        self.refresh_in_flight = true;
        let monitor = self.session.monitor();
        let monitor_bounds = self.session.monitor_bounds();
        let with_cursor = settings_state::get().capture_cursor;
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let captured = cx
                .background_executor()
                .spawn(async move { capture_monitor_frame(monitor, monitor_bounds, with_cursor) })
                .await;
            crate::logging::log_debug!(
                "overlay",
                "refreshed monitor {} duration_ms={} failed={}",
                monitor.raw(),
                started.elapsed().as_millis(),
                captured.is_err()
            );
            let _ = this.update(cx, |this, cx| {
                this.refresh_in_flight = false;
                this.apply_refreshed_frame(monitor, monitor_bounds, captured, cx);
            });
        })
        .detach();
    }

    fn apply_refreshed_frame(
        &mut self,
        monitor: MonitorId,
        monitor_bounds: PhysicalRect,
        captured: Result<RgbaFrame, String>,
        cx: &mut Context<Self>,
    ) {
        let frame = match captured {
            Ok(frame) if super::capture::frame_matches_monitor(&frame, monitor_bounds) => frame,
            Ok(frame) => {
                log_warn!(
                    "overlay",
                    "refreshed frame {}x{} does not match {:?}",
                    frame.width(),
                    frame.height(),
                    monitor_bounds
                );
                self.show_refresh_failed(cx);
                return;
            }
            Err(error) => {
                log_warn!("overlay", "refresh failed: {error}");
                self.show_refresh_failed(cx);
                return;
            }
        };
        self.image_size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image(frame);
        let old_image = std::mem::replace(&mut self.image, image);
        cx.drop_image(old_image, None);
        if let Some(magnifier) = self.magnifier.take() {
            cx.drop_image(magnifier.image, None);
        }
        self.element_probe.reset();
        self.screen_targeting = false;
        let mut excluded = live_overlay_window_ids_excluding(cx, Some(cx.entity().entity_id()));
        if let Some(id) = self.native_id {
            excluded.push(WindowId::from_raw(id));
        }
        let mut backend: Box<dyn CaptureBackend> = platform_backend();
        if let Ok(windows) = backend.windows_excluding(&excluded) {
            self.session = SelectionSession::with_scale(
                monitor,
                monitor_bounds,
                windows,
                self.effective_scale(),
            );
            self.session.set_window_targeting(true);
            self.session.set_aspect_ratio(self.aspect.ratio());
        }
        cx.notify();
    }

    fn show_refresh_failed(&mut self, cx: &mut Context<Self>) {
        let language = settings_state::get().language;
        self.set_notice(t(language, "overlay.refresh_failed").to_owned(), cx);
    }

    pub(super) fn watch_close(&self, window: &Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(view) = view.upgrade() {
                view.update(cx, |this, cx| this.release_capture_resources(cx));
            }
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
            true
        });
    }
}

fn capture_monitor_frame(
    monitor: MonitorId,
    monitor_bounds: PhysicalRect,
    with_cursor: bool,
) -> Result<RgbaFrame, String> {
    let mut last_error = None;
    for _ in 0..2 {
        let mut backend: Box<dyn CaptureBackend> = platform_backend();
        let captured = if with_cursor {
            backend.capture_with_cursor(CaptureTarget::Monitor(monitor))
        } else {
            backend.capture(CaptureTarget::Monitor(monitor))
        };
        match captured {
            Ok(frame) if super::capture::frame_matches_monitor(&frame, monitor_bounds) => {
                return Ok(frame);
            }
            Ok(frame) => {
                last_error = Some(format!(
                    "refreshed frame {}x{} does not match {:?}",
                    frame.width(),
                    frame.height(),
                    monitor_bounds
                ));
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    Err(last_error.unwrap_or_else(|| "capture failed".to_owned()))
}
