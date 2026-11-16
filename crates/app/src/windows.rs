use gpui_kit::*;

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

pub fn close_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    close_matching(cx, handles, None, false, true);
}

pub fn close_capture_auxiliary(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    close_matching(cx, handles, None, true, true);
    crate::overlay::restore_previous_application(cx);
}

pub fn close_capture_auxiliary_except(
    cx: &mut App,
    handles: Vec<AnyWindowHandle>,
    keep: AnyWindowHandle,
) {
    close_matching(cx, handles, Some(keep), true, false);
}
