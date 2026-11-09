//! Window lifecycle shared by the capture, editor, and utility windows.

use gpui_kit::*;

// Callers pass `cx.windows()` rather than `cx.window_stack()`: the Windows
// backend does not implement the stack query and returns `None`, which used to
// turn every close path (Esc, save-and-exit) into a no-op.

/// Removes every window in `handles` except pinned images. Pins are the only
/// windows that deliberately survive a new capture or a mode switch.
pub fn close_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    crate::overlay::release_textures(cx);
    crate::editor::release_images_for_windows(cx, &handles, None);
    for handle in handles {
        if !crate::pin::is_pin_window(&handle) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// Closes capture/editor surfaces without touching a settings window that was
/// open when the capture started. Capture is a screen freeze, not a mode
/// switch, so settings has to survive the overlay and editor lifecycles.
pub fn close_capture_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    crate::overlay::release_textures(cx);
    crate::editor::release_images_for_windows(cx, &handles, None);
    for handle in handles {
        if !crate::pin::is_pin_window(&handle) && !crate::settings::is_settings_window(&handle) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

pub fn close_capture_auxiliary_except(
    cx: &mut App,
    handles: Vec<AnyWindowHandle>,
    keep: AnyWindowHandle,
) {
    crate::editor::release_images_for_windows(cx, &handles, Some(keep));
    for handle in handles {
        if handle != keep
            && !crate::pin::is_pin_window(&handle)
            && !crate::settings::is_settings_window(&handle)
        {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}
