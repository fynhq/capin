//! Shared capture chrome: effect options, keyboard hints, and compact action
//! buttons used by both the frozen overlay and the inline editor.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chrome_theme::ChromeTheme;
use crate::effects::RegionEffects;
use crate::i18n::{Language, t};
use crate::icons;
use crate::ui::{enter_slide_fade, tooltip_view};

/// Effect options popover shared by the overlay and inline editor.
///
/// The caller owns placement and animation direction; the popover itself only
/// renders the corner/shadow controls and reports changes through callbacks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn effects_panel<View: 'static>(
    id_prefix: &'static str,
    theme: ChromeTheme,
    language: Language,
    rect: Bounds<Pixels>,
    effects: RegionEffects,
    offset: f32,
    cx: &mut Context<View>,
    on_radius: fn(&mut View, u32, &mut Context<View>),
    on_shadow: fn(&mut View, &mut Context<View>),
    on_dismiss: fn(&mut View, &mut Context<View>),
) -> AnyElement {
    let mut corner_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .child(icons::icon(icons::RECT, 14.0, theme.muted));
    for radius in [0u32, 8, 16, 24] {
        let selected = effects.corner_radius == radius;
        let label = if radius == 0 {
            t(language, "overlay.effects_off").to_owned()
        } else {
            radius.to_string()
        };
        let tooltip = format!("{} {label}", t(language, "overlay.effects_corner"));
        corner_row = corner_row.child(
            div()
                .id(format!("{id_prefix}-radius-{radius}"))
                .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                .h(px(22.0))
                .px(px(6.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |chip| chip.bg(theme.accent_tint))
                .when(!selected, |chip| {
                    chip.hover(move |style| style.bg(theme.hover))
                })
                .active(|style| style.opacity(0.85))
                .text_size(px(11.0))
                .text_color(if selected { theme.accent } else { theme.muted })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| on_radius(this, radius, cx)))
                .child(label),
        );
    }

    let shadow = effects.shadow;
    let shadow_label = t(language, "overlay.effects_shadow").to_owned();
    let shadow_chip = div()
        .id(format!("{id_prefix}-effects-shadow"))
        .tooltip({
            let label = shadow_label.clone();
            move |_, cx| tooltip_view(label.clone(), cx)
        })
        .h(px(22.0))
        .px(px(8.0))
        .rounded(px(5.0))
        .flex()
        .items_center()
        .justify_center()
        .when(shadow, |chip| chip.bg(theme.accent_tint))
        .when(!shadow, |chip| {
            chip.hover(move |style| style.bg(theme.hover))
        })
        .active(|style| style.opacity(0.85))
        .text_size(px(11.0))
        .text_color(if shadow { theme.accent } else { theme.muted })
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| on_shadow(this, cx)))
        .child(shadow_label);

    let panel = div()
        .id(format!("{id_prefix}-effects-panel"))
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .w(rect.size.width)
        .h(rect.size.height)
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .rounded(px(theme.radius))
        .bg(theme.surface_strong)
        .border_1()
        .border_color(theme.border)
        .shadow(theme.shadow())
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .child(corner_row)
        .child(div().w(px(1.0)).h(px(18.0)).bg(theme.separator))
        .child(shadow_chip);
    let panel = enter_slide_fade(
        panel,
        format!("{id_prefix}-effects-enter"),
        f32::from(rect.origin.y),
        offset,
        Duration::from_millis(120),
    );

    div()
        .id(format!("{id_prefix}-effects-layer"))
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                on_dismiss(this, cx);
                cx.stop_propagation();
            }),
        )
        .child(panel)
        .into_any_element()
}

/// Keyboard hint card shared by the overlay and inline editor.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hints_panel<View: 'static>(
    id_prefix: &'static str,
    theme: ChromeTheme,
    language: Language,
    rect: Bounds<Pixels>,
    rows: Vec<(String, String)>,
    cx: &mut Context<View>,
    on_dismiss: fn(&mut View, &mut Context<View>),
) -> AnyElement {
    let mut panel = div()
        .id(format!("{id_prefix}-hints-panel"))
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .w(rect.size.width)
        .flex()
        .flex_col()
        .gap(px(5.0))
        .p(px(12.0))
        .rounded(px(theme.radius))
        .bg(theme.surface_strong)
        .border_1()
        .border_color(theme.border)
        .shadow(theme.shadow())
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme.text)
                .child(t(language, "editor.hints_title")),
        );
    for (key, description) in rows {
        panel = panel.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(div().text_size(px(10.5)).text_color(theme.muted).child(key))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.text)
                        .child(description),
                ),
        );
    }
    let panel = enter_slide_fade(
        panel,
        format!("{id_prefix}-hints-enter"),
        f32::from(rect.origin.y),
        4.0,
        Duration::from_millis(120),
    );
    div()
        .id(format!("{id_prefix}-hints-layer"))
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                on_dismiss(this, cx);
                cx.stop_propagation();
            }),
        )
        .child(panel)
        .into_any_element()
}

/// The "effects" chip in the selection info bar.
pub(crate) fn effects_chip<View: 'static>(
    id: &'static str,
    theme: ChromeTheme,
    language: Language,
    selected: bool,
    active: bool,
    cx: &mut Context<View>,
    on_click: fn(&mut View, &mut Context<View>),
) -> AnyElement {
    div()
        .id(id)
        .px(px(6.0))
        .py(px(3.0))
        .rounded(px(5.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .text_color(theme.text)
        .when(selected, |chip| chip.bg(theme.selected))
        .when(!selected, |chip| {
            chip.hover(move |style| style.bg(theme.hover))
        })
        .active(|style| style.opacity(0.85))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(t(language, "overlay.info_effects"))
        .when(active, |chip| {
            chip.child(
                div()
                    .w(px(5.0))
                    .h(px(5.0))
                    .rounded(px(3.0))
                    .bg(theme.accent),
            )
        })
        .into_any_element()
}

/// The refresh button in the selection info bar.
pub(crate) fn refresh_button<View: 'static>(
    id: &'static str,
    theme: ChromeTheme,
    language: Language,
    cx: &mut Context<View>,
    on_click: fn(&mut View, &mut Window, &mut Context<View>),
) -> AnyElement {
    let label = t(language, "overlay.info_refresh").to_owned();
    div()
        .id(id)
        .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
        .w(px(22.0))
        .h(px(22.0))
        .rounded(px(5.0))
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(theme.hover))
        .active(|style| style.opacity(0.85))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .child(icons::icon(icons::REFRESH, 14.0, theme.icon))
        .into_any_element()
}
