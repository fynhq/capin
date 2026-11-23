use std::time::{Duration, Instant};

use core_capture::{CaptureBackend, CaptureTarget, SelectionSession};
use core_types::{MonitorId, PhysicalSize, RgbaFrame, WindowId};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_capture::platform_backend;

use crate::image_util;

use super::{OverlayView, live_overlay_window_ids_excluding};

impl OverlayView {
    pub(super) fn refresh_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .last_refresh
            .is_some_and(|last| last.elapsed() < Duration::from_millis(200))
        {
            return;
        }
        self.last_refresh = Some(Instant::now());
        let mut backend = platform_backend();
        let monitor = self.session.monitor();
        let mut frame = match self.refresh_capture(&mut *backend, monitor) {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(error) => {
                self.set_notice(error.to_string(), cx);
                return;
            }
        };
        let monitor_bounds = self.session.monitor_bounds();
        if !super::capture::frame_matches_monitor(&frame, monitor_bounds) {
            crate::logging::log_warn!(
                "overlay",
                "refreshed frame {}x{} does not match {:?}; retrying",
                frame.width(),
                frame.height(),
                monitor_bounds
            );
            match backend.capture(CaptureTarget::Monitor(monitor)) {
                Ok(retry) if super::capture::frame_matches_monitor(&retry, monitor_bounds) => {
                    frame = retry;
                }
                _ => {
                    self.set_notice(
                        "refreshed frame does not match the monitor size".to_owned(),
                        cx,
                    );
                    return;
                }
            }
        }
        self.image_size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image(frame);
        let old_image = std::mem::replace(&mut self.image, image);
        if let Err(error) = window.drop_image(old_image) {
            crate::logging::log_warn!("overlay", "failed to release refreshed frame: {error}");
        }
        if let Some(magnifier) = self.magnifier.take()
            && let Err(error) = window.drop_image(magnifier.image)
        {
            crate::logging::log_warn!("overlay", "failed to release magnifier frame: {error}");
        }
        self.element_probe.reset();
        self.screen_targeting = false;
        let mut excluded = live_overlay_window_ids_excluding(cx, Some(cx.entity().entity_id()));
        if let Some(id) = self.native_id {
            excluded.push(WindowId::from_raw(id));
        }
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

    #[cfg(target_os = "windows")]
    pub(super) fn refresh_capture(
        &mut self,
        backend: &mut dyn CaptureBackend,
        monitor: MonitorId,
    ) -> core_capture::Result<Option<RgbaFrame>> {
        let started = self.monitor_stream.is_none();
        if started {
            match platform_capture::MonitorStream::start(monitor) {
                Ok(stream) => self.monitor_stream = Some(stream),
                Err(error) => {
                    crate::logging::log_warn!("overlay", "monitor stream unavailable: {error}");
                }
            }
        }
        if let Some(stream) = self.monitor_stream.as_mut() {
            match stream.refresh() {
                Ok(Some(frame)) => return Ok(Some(frame)),
                Ok(None) if !started => return Ok(None),
                Ok(None) => {}
                Err(error) => {
                    crate::logging::log_warn!("overlay", "monitor stream refresh failed: {error}");
                    self.monitor_stream = None;
                }
            }
        }
        backend.capture(CaptureTarget::Monitor(monitor)).map(Some)
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn refresh_capture(
        &mut self,
        backend: &mut dyn CaptureBackend,
        monitor: MonitorId,
    ) -> core_capture::Result<Option<RgbaFrame>> {
        backend.capture(CaptureTarget::Monitor(monitor)).map(Some)
    }

    pub(super) fn watch_close(&self, window: &Window, cx: &mut Context<Self>) {
        window.on_window_should_close(cx, move |_, cx| {
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
            true
        });
    }
}
