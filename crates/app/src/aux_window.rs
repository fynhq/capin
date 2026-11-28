use std::sync::Mutex;
use std::time::{Duration, Instant};

use core_capture::{CaptureTarget, MonitorInfo};
use core_types::{PhysicalPoint, PhysicalRect};
use gpui_kit::*;
use platform_capture::platform_backend;

#[derive(Clone, Copy)]
struct MaterialState {
    transparency: bool,
    settings: bool,
    transient: bool,
}

static MATERIAL: Mutex<Option<(Instant, MaterialState)>> = Mutex::new(None);
const MATERIAL_TTL: Duration = Duration::from_secs(2);

fn material_state() -> MaterialState {
    let mut cached = MATERIAL.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some((at, state)) = *cached
        && at.elapsed() < MATERIAL_TTL
    {
        return state;
    }
    let transparency = platform_window::transparency_allowed();
    #[cfg(target_os = "windows")]
    let (settings, transient) = (
        transparency
            && platform_window::backdrop_supported(platform_window::BackdropMaterial::Mica),
        transparency
            && platform_window::backdrop_supported(platform_window::BackdropMaterial::Acrylic),
    );
    #[cfg(target_os = "macos")]
    let (settings, transient) = (transparency, transparency);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let (settings, transient) = (false, false);
    let state = MaterialState {
        transparency,
        settings,
        transient,
    };
    *cached = Some((Instant::now(), state));
    state
}

pub(crate) fn transparency_allowed() -> bool {
    material_state().transparency
}

pub(crate) fn settings_background() -> WindowBackgroundAppearance {
    #[cfg(target_os = "windows")]
    {
        if settings_material_active() {
            WindowBackgroundAppearance::MicaBackdrop
        } else {
            WindowBackgroundAppearance::Opaque
        }
    }
    #[cfg(target_os = "macos")]
    {
        if settings_material_active() {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        WindowBackgroundAppearance::Opaque
    }
}

pub(crate) fn settings_material_active() -> bool {
    material_state().settings
}

pub(crate) fn transient_background() -> WindowBackgroundAppearance {
    #[cfg(target_os = "macos")]
    {
        if transient_material_active() {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Transparent
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        WindowBackgroundAppearance::Transparent
    }
}

pub(crate) fn transient_material_active() -> bool {
    material_state().transient
}

pub(crate) fn apply_transient_backdrop(window: &Window) {
    #[cfg(target_os = "windows")]
    {
        if transient_material_active()
            && let Err(error) =
                platform_window::set_backdrop(window, platform_window::BackdropMaterial::Acrylic)
        {
            crate::logging::log_warn!("window", "failed to set acrylic backdrop: {error}");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
    }
}

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
