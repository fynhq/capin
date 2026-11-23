use core_capture::{CaptureBackend, CaptureError, CaptureTarget, MonitorInfo};
use core_types::{MonitorId, RgbaFrame};

pub(super) fn capture_monitors(
    monitors: Vec<MonitorInfo>,
    with_cursor: bool,
) -> Vec<(MonitorInfo, Result<RgbaFrame, CaptureError>)> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = monitors
            .iter()
            .cloned()
            .map(|monitor| {
                scope.spawn(move || {
                    let frame = capture_monitor_checked(&monitor, with_cursor);
                    (monitor, frame)
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(monitors)
            .map(|(handle, monitor)| {
                handle.join().unwrap_or_else(|_| {
                    (
                        monitor,
                        Err(CaptureError::Platform {
                            operation: "overlay capture",
                            message: "capture thread panicked".to_owned(),
                        }),
                    )
                })
            })
            .collect()
    })
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
