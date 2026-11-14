//! Shared construction for auxiliary popup windows and capture targets.

use core_capture::{CaptureTarget, MonitorInfo, platform_backend};
use core_types::{PhysicalPoint, PhysicalRect};
use gpui_kit::*;

/// Common options for borderless, fixed-size helper windows.
#[allow(clippy::too_many_arguments)]
pub(crate) fn popup_options(
    app_id: &'static str,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    focus: bool,
    kind: WindowKind,
    background: WindowBackgroundAppearance,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus,
        show: true,
        kind,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: background,
        app_id: Some(app_id.to_owned()),
        ..Default::default()
    }
}

/// Resolves a capture target to its monitor-local physical area and monitor.
///
/// Region and window targets use the same mapping as the capture overlay; a
/// monitor target maps to the monitor's full bounds.
pub(crate) fn target_area(target: CaptureTarget) -> Option<(PhysicalRect, MonitorInfo)> {
    let monitors = platform_backend().monitors().ok()?;
    let (area, monitor_id) = match target {
        CaptureTarget::Region(region) => {
            let monitor = monitors
                .iter()
                .find(|monitor| monitor.id == region.monitor)?;
            let area = PhysicalRect::new(
                monitor.bounds.origin.x + region.rect.origin.x,
                monitor.bounds.origin.y + region.rect.origin.y,
                region.rect.size.width,
                region.rect.size.height,
            );
            (area, monitor.id)
        }
        CaptureTarget::Window(id) => {
            let window = platform_backend()
                .windows()
                .ok()?
                .into_iter()
                .find(|window| window.id == id)?;
            let center = PhysicalPoint::new(
                window.bounds.origin.x + window.bounds.size.width as i32 / 2,
                window.bounds.origin.y + window.bounds.size.height as i32 / 2,
            );
            let monitor_id = window.monitor.or_else(|| {
                monitors
                    .iter()
                    .find(|monitor| monitor.bounds.contains(center))
                    .map(|monitor| monitor.id)
            })?;
            (window.bounds, monitor_id)
        }
        CaptureTarget::Monitor(id) => {
            let monitor = monitors.iter().find(|monitor| monitor.id == id)?;
            (monitor.bounds, monitor.id)
        }
    };
    let monitor = monitors
        .into_iter()
        .find(|monitor| monitor.id == monitor_id)?;
    Some((area, monitor))
}
