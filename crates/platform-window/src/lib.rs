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

pub use accent::{system_accent_hex, system_taskbar_uses_light_theme, system_uses_light_theme};
pub use autostart::{AutostartState, set_autostart};
pub use error::PlatformWindowError;
pub use style::WindowStyle;

use raw_window_handle::HasWindowHandle;

/// Raises a native dropdown menu at the pointer and returns the chosen index.
/// macOS uses this for settings popups so the menu owns its vibrant surface
/// and keyboard/dismiss behavior; other platforms keep their in-window popover.
pub fn show_native_menu(items: &[String], selected: usize) -> Option<usize> {
    #[cfg(target_os = "macos")]
    {
        macos::show_native_menu(items, selected)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (items, selected);
        None
    }
}

/// Whether the process currently has macOS Accessibility permission.
pub fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Whether the process currently has macOS screen-recording permission.
pub fn screen_capture_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::screen_capture_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Asks macOS to list the app under Screen Recording. The system shows the
/// prompt only while the app is not yet listed; later calls just return the
/// current decision. No-op on other platforms.
pub fn request_screen_capture_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::request_screen_capture_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Hides the macOS Dock icon so the app behaves as a menu-bar utility.
/// No-op on other platforms.
pub fn set_macos_accessory_activation_policy() {
    #[cfg(target_os = "macos")]
    macos::set_accessory_activation_policy();
}

/// Shows a Dock icon while a full app window (the settings page) is open.
/// No-op on other platforms.
pub fn set_macos_regular_activation_policy() {
    #[cfg(target_os = "macos")]
    macos::set_regular_activation_policy();
}

/// Brings the process to the foreground. The capture overlay needs this on
/// macOS because accessories are not activated merely by ordering a window
/// front; without it the overlay never becomes the key window, so Esc and
/// letter shortcuts do nothing.
pub fn activate_application() {
    #[cfg(target_os = "macos")]
    macos::activate_application();
}

/// A macOS Privacy & Security pane that the settings page can open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivacyPane {
    ScreenCapture,
    Accessibility,
}

/// Opens the matching pane of System Settings. On macOS each pane first
/// triggers its own permission prompt so Capin is added to the list instead
/// of leaving the user with an empty toggle.
pub fn open_privacy_settings(pane: PrivacyPane) {
    #[cfg(target_os = "macos")]
    {
        match pane {
            PrivacyPane::ScreenCapture => {
                let _ = macos::request_screen_capture_permission();
            }
            PrivacyPane::Accessibility => {
                let _ = macos::request_accessibility_permission();
            }
        }
        macos::open_privacy_settings(pane);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pane;
    }
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

/// Physical screen position of the mouse cursor.
pub fn cursor_screen_position() -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows::cursor_screen_position()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Physical screen origin of a native window.
pub fn window_screen_origin(window: &impl HasWindowHandle) -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows::window_screen_origin(window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        None
    }
}

/// Moves a native window without resizing or changing its z-order.
pub fn move_window_to(
    window: &impl HasWindowHandle,
    x: i32,
    y: i32,
) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        windows::move_window_to(window, x, y)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, x, y);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

/// Hides or restores the system cursor for the current thread's window.
///
/// The capture overlay hides it while Ctrl window mode paints a camera glyph
/// at the pointer. No-op on platforms where GPUI already owns the cursor.
pub fn set_cursor_hidden(hidden: bool) {
    #[cfg(target_os = "windows")]
    {
        windows::set_cursor_hidden(hidden);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = hidden;
    }
}

/// Removes the platform-drawn outline around a borderless surface. Windows 11
/// still paints a 1px frame and rounded corners around top-level windows; a
/// pin is sized to its image and must not show anything outside it. No-op on
/// other platforms.
pub fn hide_native_border(window: &impl HasWindowHandle) {
    #[cfg(target_os = "windows")]
    {
        windows::hide_native_border(window);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
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

/// Places an existing window on a monitor using that monitor's physical
/// rectangle.
///
/// Full-screen capture overlays must cover exactly one monitor even on
/// mixed-DPI desktops. GPUI converts logical window bounds with the target
/// display's scale factor, but Windows may still resize the window through
/// `WM_DPICHANGED` while it moves from the monitor it was created on. Applying
/// the physical rectangle after creation removes that ambiguity. On macOS the
/// rectangle is converted back to AppKit points, so callers always pass
/// physical pixels.
pub fn place_on_monitor(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    scale: core_types::DpiScale,
) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        let _ = scale;
        windows::place_on_monitor(window, bounds)
    }
    #[cfg(target_os = "macos")]
    {
        macos::place_on_monitor(window, bounds, scale)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, bounds, scale);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

/// Returns the platform monitor that currently contains the window, as the
/// same raw identifier used by `MonitorInfo`.
pub fn monitor_for_window(window: &impl HasWindowHandle) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        windows::monitor_for_window(window)
    }
    #[cfg(not(target_os = "windows"))]
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
