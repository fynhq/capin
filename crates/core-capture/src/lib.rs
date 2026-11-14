//! Cross-platform capture API shared by the overlay, editor, and recorder.
//!
//! The public surface is platform-neutral; the Windows/macOS backends live in
//! gated modules inside this crate. OS service adapters that are not part of
//! the capture abstraction belong in the `platform-*` crates.

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
pub use macos::{MacosCapture, display_backing_scale};
#[cfg(target_os = "windows")]
pub use windows::{WindowsCapture, init_process_dpi_awareness};

/// Window ids macOS marks as helper surfaces (overlay, capture bars, region
/// outline, demo cursor, pin menus). Recording reuses this list so pins and
/// the settings page stay visible in the output, matching screenshots and the
/// Windows recorder.
#[cfg(target_os = "macos")]
pub fn capture_excluded_window_ids() -> Vec<u32> {
    macos::capture_excluded_window_ids()
}
