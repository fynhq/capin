//! Thin border-only window that keeps the recorded or scroll-captured region
//! visible while the control bar is on screen. It is click-through and always
//! excluded from capture, so the red frame never ends up in the output.

use core_capture::{CaptureTarget, platform_backend};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style, hide_native_border};

const BORDER_WIDTH: f32 = 3.0;
const BORDER_COLOR: u32 = 0xFF3B30;

thread_local! {
    static OUTLINE: std::cell::RefCell<Option<(AnyWindowHandle, Entity<RegionOutline>)>> =
        const { std::cell::RefCell::new(None) };
}

pub fn close(cx: &mut App) {
    let entry = OUTLINE.with(|slot| slot.borrow_mut().take());
    let Some((handle, view)) = entry else {
        return;
    };
    // Native window teardown is asynchronous on Windows; hide the border now
    // so stopping or leaving the completion prompt removes the frame at once.
    view.update(cx, |this, cx| {
        this.visible = false;
        cx.notify();
    });
    let _ = handle.update(cx, |_, window, _| window.remove_window());
}

pub fn open(target: CaptureTarget, cx: &mut App) {
    close(cx);
    let Some((rect, monitor_id)) = target_rect(target) else {
        return;
    };
    let monitors = platform_backend().monitors().unwrap_or_default();
    let Some(monitor) = monitors.iter().find(|monitor| monitor.id == monitor_id) else {
        return;
    };
    let scale = monitor.scale.get().max(0.1);
    let left = rect.origin.x as f32 / scale - BORDER_WIDTH;
    let top = rect.origin.y as f32 / scale - BORDER_WIDTH;
    let width = rect.size.width as f32 / scale + BORDER_WIDTH * 2.0;
    let height = rect.size.height as f32 / scale + BORDER_WIDTH * 2.0;
    let logical = Bounds {
        origin: point(px(left), px(top)),
        size: size(px(width), px(height)),
    };
    // macOS accepts display-local points in `WindowOptions`; Windows accepts
    // virtual-desktop logical coordinates. The shared helper applies the
    // platform-specific translation, matching the recording/scroll bars.
    let bounds = crate::display::window_bounds_on_monitor(monitor, logical);
    let options = crate::aux_window::popup_options(
        "capin-region-outline",
        bounds,
        Some(DisplayId::new(monitor_id.raw())),
        false,
        WindowKind::PopUp,
        WindowBackgroundAppearance::Transparent,
    );
    match gpui_kit::open_window(options, cx, move |window, cx| {
        let visible = match apply_window_style(&*window, WindowStyle::CAPTURE_OUTLINE) {
            Ok(()) => true,
            Err(error) => {
                crate::logging::log_error!("outline", "failed to style region outline: {error}");
                false
            }
        };
        hide_native_border(&*window);
        let _ = platform_window::set_click_through(&*window, true);
        cx.new(|_| RegionOutline { visible })
    }) {
        Ok((handle, view)) => {
            OUTLINE.with(|slot| *slot.borrow_mut() = Some((handle, view)));
        }
        Err(error) => {
            crate::logging::log_error!("outline", "failed to open region outline: {error}")
        }
    }
}

fn target_rect(target: CaptureTarget) -> Option<(core_types::PhysicalRect, core_types::MonitorId)> {
    if matches!(target, CaptureTarget::Monitor(_)) {
        return None;
    }
    crate::aux_window::target_area(target).map(|(area, monitor)| (area, monitor.id))
}

struct RegionOutline {
    visible: bool,
}

impl Render for RegionOutline {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().when(self.visible, |root| {
            root.border_3().border_color(rgb(BORDER_COLOR))
        })
    }
}
