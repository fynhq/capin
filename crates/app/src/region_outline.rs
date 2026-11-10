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
    static OUTLINE: std::cell::RefCell<Option<AnyWindowHandle>> =
        const { std::cell::RefCell::new(None) };
}

pub fn close(cx: &mut App) {
    let handle = OUTLINE.with(|slot| slot.borrow_mut().take());
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
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
    let bounds = Bounds {
        origin: point(px(left), px(top)),
        size: size(px(width), px(height)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(DisplayId::new(monitor_id.raw())),
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-region-outline".to_owned()),
        ..Default::default()
    };
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
        Ok((handle, _)) => {
            OUTLINE.with(|slot| *slot.borrow_mut() = Some(handle));
        }
        Err(error) => {
            crate::logging::log_error!("outline", "failed to open region outline: {error}")
        }
    }
}

fn target_rect(target: CaptureTarget) -> Option<(core_types::PhysicalRect, core_types::MonitorId)> {
    let monitors = platform_backend().monitors().ok()?;
    match target {
        CaptureTarget::Region(region) => {
            let monitor = monitors
                .iter()
                .find(|monitor| monitor.id == region.monitor)?;
            Some((
                core_types::PhysicalRect::new(
                    monitor.bounds.origin.x + region.rect.origin.x,
                    monitor.bounds.origin.y + region.rect.origin.y,
                    region.rect.size.width,
                    region.rect.size.height,
                ),
                region.monitor,
            ))
        }
        CaptureTarget::Window(id) => {
            let window = platform_backend()
                .windows()
                .ok()?
                .into_iter()
                .find(|window| window.id == id)?;
            let monitor = window.monitor.or_else(|| {
                let center = core_types::PhysicalPoint::new(
                    window.bounds.origin.x + window.bounds.size.width as i32 / 2,
                    window.bounds.origin.y + window.bounds.size.height as i32 / 2,
                );
                monitors
                    .iter()
                    .find(|monitor| monitor.bounds.contains(center))
                    .map(|monitor| monitor.id)
            })?;
            Some((window.bounds, monitor))
        }
        // A full-screen recording needs no frame; the screen edge already
        // shows the recorded area.
        CaptureTarget::Monitor(_) => None,
    }
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
