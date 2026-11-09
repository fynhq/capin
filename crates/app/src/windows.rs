//! Window lifecycle shared by the capture, editor, and utility windows.

use gpui_kit::*;

// Callers pass `cx.windows()` rather than `cx.window_stack()`: the Windows
// backend does not implement the stack query and returns `None`, which used to
// turn every close path (Esc, right-click, save-and-exit) into a no-op.

/// Removes every window in `handles` except pinned images. Pins are the only
/// windows that deliberately survive a new capture or a mode switch.
pub fn close_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    for handle in handles {
        if !crate::pin::is_pin_window(&handle) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// Removes every window in `handles` except pinned images and `keep`.
/// The inline editor keeps the overlay window that is turning into the editor
/// while the other monitors' overlays must disappear.
pub fn close_auxiliary_except(cx: &mut App, handles: Vec<AnyWindowHandle>, keep: AnyWindowHandle) {
    for handle in handles {
        if handle != keep && !crate::pin::is_pin_window(&handle) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}
