//! Small shared UI helpers used by the auxiliary windows.

use gpui_kit::prelude::*;
use gpui_kit::*;

/// Centers a window on the primary display, or falls back to a stable origin
/// when no display information is available.
pub fn centered_bounds(cx: &App, width: f32, height: f32) -> Bounds<Pixels> {
    match cx.primary_display() {
        Some(display) => {
            let center = display.bounds().center();
            Bounds {
                origin: point(center.x - px(width / 2.0), center.y - px(height / 2.0)),
                size: size(px(width), px(height)),
            }
        }
        None => Bounds {
            origin: point(px(80.0), px(80.0)),
            size: size(px(width), px(height)),
        },
    }
}

/// Compact text button used by the recording and scrolling bars.
pub fn flat_button<View: 'static>(
    id: &'static str,
    label: String,
    color: Rgba,
    cx: &mut Context<View>,
    action: fn(&mut View, &mut Context<View>),
) -> impl IntoElement + use<View> {
    let hover = rgba(0xFFFFFF1A);
    div()
        .id(id)
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .bg(rgba(0xFFFFFF14))
        .text_size(px(12.0))
        .text_color(color)
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
        .child(label)
}
