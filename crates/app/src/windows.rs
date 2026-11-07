//! Window lifecycle shared by the capture, editor, and utility windows.

use gpui_kit::*;

/// Removes every window in `handles` except pinned images. Pins are the only
/// windows that deliberately survive a new capture or a mode switch.
pub fn close_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    for handle in handles {
        if !crate::pin::is_pin_window(&handle) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}
