mod backend;
mod error;
mod info;
mod selection;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub mod macos_sck_support;
#[cfg(target_os = "windows")]
mod windows;

pub use backend::{CaptureBackend, platform_backend};
pub use core_types::CaptureTarget;
pub use error::{CaptureError, Result};
pub use info::{MonitorInfo, WindowInfo};
pub use selection::{
    DRAG_THRESHOLD_PX, SelectionAction, SelectionOutcome, SelectionSession, element_candidates,
    region_from_drag,
};

pub fn ui_elements(id: core_types::WindowId) -> Vec<core_types::PhysicalRect> {
    #[cfg(target_os = "windows")]
    {
        windows::ui_elements(id)
    }
    #[cfg(target_os = "macos")]
    {
        macos::ui_elements(id)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = id;
        Vec::new()
    }
}

pub fn ui_elements_at(
    id: core_types::WindowId,
    point: core_types::PhysicalPoint,
) -> Vec<core_types::PhysicalRect> {
    #[cfg(target_os = "windows")]
    {
        let _ = point;
        windows::ui_elements(id)
    }
    #[cfg(target_os = "macos")]
    {
        macos::ui_elements_at(id, point)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (id, point);
        Vec::new()
    }
}

pub fn window_corner_radius(id: core_types::WindowId, scale: f32) -> u32 {
    #[cfg(target_os = "windows")]
    {
        windows::window_corner_radius(id, scale)
    }
    #[cfg(target_os = "macos")]
    {
        macos::window_corner_radius(id, scale)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (id, scale);
        0
    }
}

#[cfg(target_os = "macos")]
pub use macos::{MacosCapture, display_backing_scale};
#[cfg(target_os = "windows")]
pub use windows::{MonitorStream, WindowsCapture, init_process_dpi_awareness};

#[cfg(target_os = "macos")]
pub fn capture_excluded_window_ids() -> Vec<u32> {
    macos::capture_excluded_window_ids()
}
