//! Live demo-cursor disc shown while recording.
//!
//! The encoded frames get the same disc painted by the platform recorder, so
//! this window is excluded from capture and click-through: it exists only to
//! let the user see the highlight in real time.

use std::cell::RefCell;
use std::time::Duration;

use core_capture::{CaptureTarget, platform_backend};
use core_record::{CURSOR_HIGHLIGHT_ALPHA, CURSOR_HIGHLIGHT_RADIUS};
use core_types::PhysicalRect;
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{
    WindowStyle, apply as apply_window_style, hide_native_border, set_click_through,
};

thread_local! {
    static SPOTLIGHT: RefCell<Option<(AnyWindowHandle, Entity<SpotlightView>)>> =
        const { RefCell::new(None) };
}

pub fn close(cx: &mut App) {
    let entry = SPOTLIGHT.with(|slot| slot.borrow_mut().take());
    let Some((handle, view)) = entry else {
        return;
    };
    view.update(cx, |this, cx| {
        this.visible = false;
        cx.notify();
    });
    let _ = handle.update(cx, |_, window, _| window.remove_window());
}

pub fn open(target: CaptureTarget, color: u32, cx: &mut App) {
    close(cx);
    let Some((area, monitor)) = target_area(target) else {
        return;
    };
    let scale = monitor.scale.get().max(0.1);
    let logical = Bounds {
        origin: point(
            px(area.origin.x as f32 / scale),
            px(area.origin.y as f32 / scale),
        ),
        size: size(
            px(area.size.width as f32 / scale),
            px(area.size.height as f32 / scale),
        ),
    };
    let bounds = crate::display::window_bounds_on_monitor(&monitor, logical);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(DisplayId::new(monitor.id.raw())),
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-record-spotlight".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, move |window, cx| {
        let _ = apply_window_style(&*window, WindowStyle::RECORD_HIGHLIGHT);
        hide_native_border(&*window);
        let _ = set_click_through(&*window, true);
        cx.new(|cx| SpotlightView::new(area, scale, color, cx))
    }) {
        Ok((handle, view)) => {
            SPOTLIGHT.with(|slot| *slot.borrow_mut() = Some((handle, view)));
        }
        Err(error) => {
            crate::logging::log_error!("record", "failed to open the demo cursor window: {error}")
        }
    }
}

struct SpotlightView {
    area: PhysicalRect,
    scale: f32,
    color: u32,
    cursor: Option<Point<Pixels>>,
    visible: bool,
}

impl SpotlightView {
    fn new(area: PhysicalRect, scale: f32, color: u32, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        this.sync_cursor(cx);
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        Self {
            area,
            scale,
            color,
            cursor: None,
            visible: true,
        }
    }

    fn sync_cursor(&mut self, cx: &mut Context<Self>) {
        let radius = CURSOR_HIGHLIGHT_RADIUS;
        let width = self.area.size.width as f32 / self.scale;
        let height = self.area.size.height as f32 / self.scale;
        let next = platform_window::cursor_screen_position().and_then(|(x, y)| {
            let local_x = (x - self.area.origin.x) as f32 / self.scale;
            let local_y = (y - self.area.origin.y) as f32 / self.scale;
            (local_x >= -radius
                && local_y >= -radius
                && local_x <= width + radius
                && local_y <= height + radius)
                .then(|| point(px(local_x), px(local_y)))
        });
        if next != self.cursor {
            self.cursor = next;
            cx.notify();
        }
    }
}

impl Render for SpotlightView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let radius = CURSOR_HIGHLIGHT_RADIUS;
        let disc = self.visible.then(|| {
            self.cursor.map(|cursor| {
                div()
                    .absolute()
                    .left(px(f32::from(cursor.x) - radius))
                    .top(px(f32::from(cursor.y) - radius))
                    .w(px(radius * 2.0))
                    .h(px(radius * 2.0))
                    .rounded(px(radius))
                    .bg(rgba(
                        ((self.color & 0x00FF_FFFF) << 8) | u32::from(CURSOR_HIGHLIGHT_ALPHA),
                    ))
            })
        });
        div().size_full().children(disc.flatten())
    }
}

fn target_area(target: CaptureTarget) -> Option<(PhysicalRect, core_capture::MonitorInfo)> {
    let monitors = platform_backend().monitors().ok()?;
    let (area, monitor_id) = match target {
        CaptureTarget::Region(region) => {
            let monitor = monitors
                .iter()
                .find(|monitor| monitor.id == region.monitor)?;
            let area = PhysicalRect::new(
                monitor.bounds.origin.x + region.rect.origin.x,
                monitor.bounds.origin.y + region.rect.origin.y,
                region.rect.size.width,
                region.rect.size.height,
            );
            (area, monitor.id)
        }
        CaptureTarget::Window(id) => {
            let window = platform_backend()
                .windows()
                .ok()?
                .into_iter()
                .find(|window| window.id == id)?;
            let center = core_types::PhysicalPoint::new(
                window.bounds.origin.x + window.bounds.size.width as i32 / 2,
                window.bounds.origin.y + window.bounds.size.height as i32 / 2,
            );
            let monitor_id = window.monitor.or_else(|| {
                monitors
                    .iter()
                    .find(|monitor| monitor.bounds.contains(center))
                    .map(|monitor| monitor.id)
            })?;
            (window.bounds, monitor_id)
        }
        CaptureTarget::Monitor(id) => {
            let monitor = monitors.iter().find(|monitor| monitor.id == id)?;
            (monitor.bounds, monitor.id)
        }
    };
    let monitor = monitors
        .into_iter()
        .find(|monitor| monitor.id == monitor_id)?;
    Some((area, monitor))
}
