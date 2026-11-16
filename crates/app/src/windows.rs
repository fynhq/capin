use std::cell::RefCell;

use gpui_kit::*;

use crate::settings::SettingsView;

thread_local! {
    static SETTINGS_WINDOWS: RefCell<Vec<(AnyWindowHandle, WeakEntity<SettingsView>)>> =
        const { RefCell::new(Vec::new()) };
}

pub fn is_settings_window(handle: &AnyWindowHandle) -> bool {
    SETTINGS_WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .any(|(open, _)| open.window_id() == handle.window_id())
    })
}

pub(crate) fn live_settings_windows(
    cx: &mut App,
) -> Vec<(AnyWindowHandle, WeakEntity<SettingsView>)> {
    let windows = SETTINGS_WINDOWS.with(|windows| windows.borrow().clone());
    let mut live = Vec::with_capacity(windows.len());
    for window in windows {
        if window.0.update(cx, |_, _, _| {}).is_ok() {
            live.push(window);
        }
    }
    SETTINGS_WINDOWS.with(|slot| *slot.borrow_mut() = live.clone());
    live
}

pub fn has_open_settings(cx: &mut App) -> bool {
    !live_settings_windows(cx).is_empty()
}

pub(crate) fn register_settings_window(handle: AnyWindowHandle, view: WeakEntity<SettingsView>) {
    SETTINGS_WINDOWS.with(|windows| windows.borrow_mut().push((handle, view)));
}

pub(crate) fn unregister_settings_window(handle: &AnyWindowHandle) {
    SETTINGS_WINDOWS.with(|windows| {
        windows
            .borrow_mut()
            .retain(|(open, _)| open.window_id() != handle.window_id());
    });
}

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
            || (preserve_settings && is_settings_window(&handle))
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
