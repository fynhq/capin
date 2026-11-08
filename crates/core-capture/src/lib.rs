//! Platform-neutral capture API shared by the overlay, editor, and recorder.

mod backend;
mod error;
mod info;
mod selection;
mod target;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use backend::{CaptureBackend, platform_backend};
pub use error::{CaptureError, Result};
pub use info::{MonitorInfo, WindowInfo};
pub use selection::{
    DRAG_THRESHOLD_PX, SelectionAction, SelectionOutcome, SelectionSession, element_candidates,
    region_from_drag,
};
pub use target::CaptureTarget;

/// Returns best-effort bounds for UI elements inside a window.
///
/// Windows uses UI Automation; other platforms currently return an empty list
/// and fall back to whole-window selection.
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

/// Returns the window's effective corner radius in physical pixels.
///
/// Windows 11 reports its DWM corner preference; Windows 10 keeps square
/// corners. macOS uses the standard 10-point AppKit radius.
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
pub use macos::MacosCapture;
#[cfg(target_os = "windows")]
pub use windows::{WindowsCapture, init_process_dpi_awareness};
