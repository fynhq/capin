//! Native window behavior that GPUI does not expose: topmost, taskbar
//! visibility, activation policy, and capture exclusion.

mod accent;
mod autostart;
mod error;
mod style;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use accent::{system_accent_hex, system_uses_light_theme};
pub use autostart::{AutostartState, set_autostart};
pub use error::PlatformWindowError;
pub use style::WindowStyle;

use raw_window_handle::HasWindowHandle;

/// Hides the macOS Dock icon so the app behaves as a menu-bar utility.
/// No-op on other platforms.
pub fn set_macos_accessory_activation_policy() {
    #[cfg(target_os = "macos")]
    macos::set_accessory_activation_policy();
}

/// Returns the current user's Pictures folder.
///
/// On Windows this is the known folder, so redirected libraries (for example
/// OneDrive) resolve to their real location. On macOS it is `~/Pictures`.
pub fn pictures_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows::pictures_dir()
    }
    #[cfg(target_os = "macos")]
    {
        macos::pictures_dir()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        home_dir().map(|home| home.join("Pictures"))
    }
}

/// Returns the current user's video folder.
///
/// On Windows this is the known Videos folder; on macOS it is `~/Movies`.
pub fn videos_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows::videos_dir()
    }
    #[cfg(target_os = "macos")]
    {
        macos::videos_dir()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        home_dir().map(|home| home.join("Videos"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Enables or disables mouse pass-through for an existing window.
pub fn set_click_through(
    window: &impl HasWindowHandle,
    enabled: bool,
) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        windows::set_click_through(window, enabled)
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_click_through(window, enabled)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, enabled);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

/// Whether the platform's window capture already includes the native window
/// shadow. macOS 13's ScreenCaptureKit path does; macOS 14+ can ignore it.
pub fn window_capture_includes_native_shadow() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::window_capture_includes_native_shadow()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Returns the native window identifier used by the platform capture APIs.
///
/// On Windows this is the `HWND`; on macOS it is the `CGWindowID`/window
/// number. Returns `None` before the platform window has been created.
pub fn native_window_id(window: &impl HasWindowHandle) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        windows::native_window_id(window)
    }
    #[cfg(target_os = "macos")]
    {
        macos::native_window_id(window)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = window;
        None
    }
}

/// Applies the requested native window behavior to an existing window.
///
/// Call this after GPUI creates the window. Settings that the platform cannot
/// satisfy are ignored when they are not applicable (for example, macOS has no
/// per-window taskbar flag), while operations that report an OS error fail.
pub fn apply(window: &impl HasWindowHandle, style: WindowStyle) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        windows::apply(window, style)
    }
    #[cfg(target_os = "macos")]
    {
        macos::apply(window, style)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, style);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

/// Posts a scroll wheel event toward the window under a monitor-local point.
///
/// `monitor` is the platform monitor identifier and `(x, y)` is in that
/// monitor's physical pixel space. Using monitor-local coordinates avoids the
/// ambiguous global mapping on mixed-DPI macOS desktops.
///
/// `notches` is the number of wheel steps; positive scrolls up and negative
/// scrolls down. Returns `false` when the platform cannot synthesize input,
/// for example when macOS permission for posting events is missing.
pub fn scroll_wheel_at(monitor: u64, x: i32, y: i32, notches: i32) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::scroll_wheel_at(monitor, x, y, notches)
    }
    #[cfg(target_os = "macos")]
    {
        macos::scroll_wheel_at(monitor, x, y, notches)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (monitor, x, y, notches);
        false
    }
}
