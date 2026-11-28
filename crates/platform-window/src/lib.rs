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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackdropMaterial {
    Mica,
    Acrylic,
}

pub fn transparency_allowed() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::transparency_allowed()
    }
    #[cfg(target_os = "macos")]
    {
        macos::transparency_allowed()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        true
    }
}

pub fn backdrop_supported(material: BackdropMaterial) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::backdrop_supported(material)
    }
    #[cfg(target_os = "macos")]
    {
        macos::backdrop_supported(material)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = material;
        false
    }
}

pub fn set_backdrop(
    window: &impl HasWindowHandle,
    material: BackdropMaterial,
) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        windows::set_backdrop(window, material)
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_backdrop(window, material)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, material);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

pub fn trim_working_set() {
    #[cfg(target_os = "windows")]
    {
        windows::trim_working_set();
    }
}

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

pub fn request_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::request_accessibility_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

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

pub fn set_macos_accessory_activation_policy() {
    #[cfg(target_os = "macos")]
    macos::set_accessory_activation_policy();
}

pub fn set_macos_regular_activation_policy() {
    #[cfg(target_os = "macos")]
    macos::set_regular_activation_policy();
}

pub fn activate_application() {
    #[cfg(target_os = "macos")]
    macos::activate_application();
}

pub fn control_modifier_down() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::control_modifier_down()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub fn frontmost_other_application_pid() -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        macos::frontmost_other_application_pid()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub fn activate_application_by_pid(pid: i32) {
    #[cfg(target_os = "macos")]
    macos::activate_application_by_pid(pid);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivacyPane {
    ScreenCapture,
    Accessibility,
}

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

pub fn cursor_screen_position() -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows::cursor_screen_position()
    }
    #[cfg(target_os = "macos")]
    {
        macos::cursor_screen_position()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

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

pub fn disable_native_shadow(window: &impl HasWindowHandle) {
    #[cfg(target_os = "windows")]
    {
        windows::disable_native_shadow(window);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
    }
}

pub fn enable_native_shadow(window: &impl HasWindowHandle) {
    #[cfg(target_os = "windows")]
    {
        windows::enable_native_shadow(window);
    }
    #[cfg(target_os = "macos")]
    {
        macos::enable_native_shadow(window);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = window;
    }
}

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

pub fn place_on_monitor_deferred(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    scale: core_types::DpiScale,
) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        let _ = scale;
        windows::place_on_monitor_deferred(window, bounds)
    }
    #[cfg(target_os = "macos")]
    {
        macos::place_on_monitor_deferred(window, bounds, scale)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, bounds, scale);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

pub fn set_image_clipboard(
    width: u32,
    height: u32,
    rgba: &[u8],
    png: Option<&[u8]>,
) -> Result<(), PlatformWindowError> {
    #[cfg(target_os = "windows")]
    {
        windows::set_image_clipboard(width, height, rgba, png)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (width, height, rgba, png);
        Err(PlatformWindowError::UnsupportedPlatform)
    }
}

pub fn reduce_motion() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::reduce_motion()
    }
    #[cfg(target_os = "macos")]
    {
        macos::reduce_motion()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

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
