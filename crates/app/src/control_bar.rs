use core_capture::MonitorInfo;
use core_types::PhysicalRect;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style};

use crate::chrome_theme::ChromeTheme;

pub(crate) fn bounds_for_anchor(
    anchor: Option<PhysicalRect>,
    monitor: Option<&MonitorInfo>,
    width: f32,
    height: f32,
) -> Bounds<Pixels> {
    let span = crate::display::logical_monitor(monitor);
    let (anchor_left, anchor_top, anchor_width) = anchor
        .map(|rect| {
            (
                rect.origin.x as f32 / span.scale,
                rect.origin.y as f32 / span.scale,
                rect.size.width as f32 / span.scale,
            )
        })
        .unwrap_or((span.left, span.top, span.right - span.left));
    let x = (anchor_left + anchor_width / 2.0 - width / 2.0).clamp(
        span.left + 8.0,
        (span.right - width - 8.0).max(span.left + 8.0),
    );
    let above = anchor_top - height - 8.0;
    let y = if above >= span.top + 8.0 {
        above
    } else {
        (anchor_top + 8.0).min(span.bottom - height - 8.0)
    };
    Bounds {
        origin: point(px(x), px(y.max(span.top + 8.0))),
        size: size(px(width), px(height)),
    }
}

pub(crate) fn bar_badge(background: Rgba, radius: f32) -> Div {
    div()
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .rounded(px(radius))
        .bg(background)
        .flex()
        .items_center()
        .justify_center()
}

pub(crate) fn bar_title(theme: &ChromeTheme, text: String) -> Div {
    div()
        .text_size(px(crate::ui::FONT_TITLE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .child(text)
}

pub(crate) fn bar_shell(
    theme: &ChromeTheme,
    badge: impl IntoElement,
    title: impl IntoElement,
    status: String,
    buttons: impl IntoIterator<Item = AnyElement>,
) -> Div {
    div().size_full().flex().items_center().relative().child(
        div()
            .size_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(12.0))
            .rounded(px(theme.radius))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .shadow(theme.shadow())
            .text_color(theme.text)
            .child(badge)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .child(title)
                    .child(
                        div()
                            .text_size(px(crate::ui::FONT_MICRO))
                            .text_color(theme.muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(status),
                    ),
            )
            .children(buttons),
    )
}

pub(crate) fn open<T: Render + 'static>(
    id: &'static str,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App, bool) -> Entity<T> + 'static,
) -> Result<Entity<T>, String> {
    let options = crate::aux_window::popup_options(
        id,
        bounds,
        display_id,
        false,
        WindowKind::PopUp,
        WindowBackgroundAppearance::Transparent,
    );
    gpui_kit::open_window(options, cx, move |window, cx| {
        let styled = match apply_window_style(&*window, WindowStyle::CAPTURE_BAR) {
            Ok(()) => true,
            Err(error) => {
                crate::logging::log_error!("control-bar", "failed to style capture bar: {error}");
                false
            }
        };
        build(window, cx, styled)
    })
    .map(|(_, entity)| entity)
    .map_err(|error| error.to_string())
}
