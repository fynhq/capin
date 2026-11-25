use core_capture::{CaptureBackend, CaptureError, CaptureTarget, MonitorInfo};
use core_types::{MonitorId, RgbaFrame};

#[cfg(target_os = "windows")]
pub(super) type CaptureStream = platform_capture::MonitorStream;
#[cfg(not(target_os = "windows"))]
pub(super) type CaptureStream = std::convert::Infallible;

pub(super) struct MonitorCapture {
    pub(super) monitor: MonitorInfo,
    pub(super) frame: Result<RgbaFrame, CaptureError>,
    pub(super) stream: Option<CaptureStream>,
}

pub(super) fn capture_monitors(
    monitors: Vec<MonitorInfo>,
    with_cursor: bool,
) -> Vec<MonitorCapture> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = monitors
            .iter()
            .cloned()
            .map(|monitor| {
                scope.spawn(move || {
                    let (frame, stream) = capture_monitor_with_cache(&monitor, with_cursor);
                    MonitorCapture {
                        monitor,
                        frame,
                        stream,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(monitors)
            .map(|(handle, monitor)| {
                handle.join().unwrap_or_else(|_| MonitorCapture {
                    monitor,
                    frame: Err(CaptureError::Platform {
                        operation: "overlay capture",
                        message: "capture thread panicked".to_owned(),
                    }),
                    stream: None,
                })
            })
            .collect()
    })
}

fn capture_monitor_with_cache(
    monitor: &MonitorInfo,
    with_cursor: bool,
) -> (Result<RgbaFrame, CaptureError>, Option<CaptureStream>) {
    #[cfg(target_os = "windows")]
    {
        let mut stream = platform_capture::take_monitor_stream(monitor.id);
        let mut warm_frame = None;
        let mut keep_stream = true;
        if !with_cursor && let Some(cached) = stream.as_mut() {
            match cached.refresh() {
                Ok(Some(frame)) if frame_matches_monitor(&frame, monitor.bounds) => {
                    crate::logging::log_info!(
                        "overlay",
                        "monitor {} reused warm capture",
                        monitor.id.raw()
                    );
                    warm_frame = Some(frame);
                }
                Ok(Some(_)) => {
                    crate::logging::log_warn!(
                        "overlay",
                        "monitor {} warm stream returned a mismatched frame",
                        monitor.id.raw()
                    );
                    keep_stream = false;
                }
                Ok(None) => {}
                Err(error) => {
                    crate::logging::log_warn!(
                        "overlay",
                        "monitor {} warm stream failed: {error}",
                        monitor.id.raw()
                    );
                    keep_stream = false;
                }
            }
        }
        if !keep_stream {
            stream = None;
        }
        if let Some(frame) = warm_frame {
            return (Ok(frame), stream);
        }
        let frame = capture_monitor_checked(monitor, with_cursor);
        if !with_cursor && stream.is_none() && frame.is_ok() {
            platform_capture::seed_monitor_stream(monitor.id);
        }
        (frame, stream)
    }
    #[cfg(not(target_os = "windows"))]
    {
        (capture_monitor_checked(monitor, with_cursor), None)
    }
}

fn capture_monitor(
    backend: &mut dyn CaptureBackend,
    id: MonitorId,
    with_cursor: bool,
) -> Result<RgbaFrame, CaptureError> {
    let target = CaptureTarget::Monitor(id);
    if with_cursor {
        backend.capture_with_cursor(target)
    } else {
        backend.capture(target)
    }
}

fn capture_monitor_checked(
    monitor: &MonitorInfo,
    with_cursor: bool,
) -> Result<RgbaFrame, CaptureError> {
    let mut backend = platform_capture::platform_backend();
    let mut frame = capture_monitor(&mut *backend, monitor.id, with_cursor)?;
    if !frame_matches_monitor(&frame, monitor.bounds) {
        crate::logging::log_error!(
            "overlay",
            "monitor {} frame {}x{} does not match {:?}; retrying",
            monitor.id.raw(),
            frame.width(),
            frame.height(),
            monitor.bounds
        );
        frame = capture_monitor(&mut *backend, monitor.id, with_cursor)?;
    }
    if !frame_matches_monitor(&frame, monitor.bounds) {
        return Err(CaptureError::Platform {
            operation: "overlay capture",
            message: format!(
                "monitor {} kept returning {}x{} instead of {:?}",
                monitor.id.raw(),
                frame.width(),
                frame.height(),
                monitor.bounds
            ),
        });
    }
    Ok(frame)
}

pub(super) fn frame_matches_monitor(frame: &RgbaFrame, bounds: core_types::PhysicalRect) -> bool {
    frame.width().abs_diff(bounds.size.width) <= 1
        && frame.height().abs_diff(bounds.size.height) <= 1
}
