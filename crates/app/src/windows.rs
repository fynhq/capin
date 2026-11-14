//! Window lifecycle shared by the capture, editor, and utility windows.

use gpui_kit::*;

// Callers pass `cx.windows()` rather than `cx.window_stack()`: the Windows
// backend does not implement the stack query and returns `None`, which used to
// turn every close path (Esc, save-and-exit) into a no-op.

fn close_matching(
    cx: &mut App,
    handles: Vec<AnyWindowHandle>,
    keep: Option<AnyWindowHandle>,
    preserve_settings: bool,
    release_overlay: bool,
) {
    if release_overlay {
        crate::overlay::release_textures(cx);
    }
    crate::editor::release_images_for_windows(cx, &handles, keep);
    let keep_id = keep.map(|handle| handle.window_id());
    for handle in handles {
        if keep_id == Some(handle.window_id())
            || crate::pin::is_pin_window(&handle)
            || (preserve_settings && crate::settings::is_settings_window(&handle))
        {
            continue;
        }
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// Removes every window in `handles` except pinned images. Pins are the only
/// windows that deliberately survive a new capture or a mode switch.
pub fn close_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    close_matching(cx, handles, None, false, true);
}

/// Closes capture/editor surfaces without touching a settings window that was
/// open when the capture started. Capture is a screen freeze, not a mode
/// switch, so settings has to survive the overlay and editor lifecycles.
pub fn close_capture_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    close_matching(cx, handles, None, true, true);
}

pub fn close_capture_auxiliary_except(
    cx: &mut App,
    handles: Vec<AnyWindowHandle>,
    keep: AnyWindowHandle,
) {
    close_matching(cx, handles, Some(keep), true, false);
}
