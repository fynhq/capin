use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chrome_theme::ChromeTheme;
use crate::effects::RegionEffects;
use crate::i18n::{Language, t};
use crate::icons;
use crate::ui::{enter_slide_fade, tooltip_view};

const HINTS_PADDING: f32 = 14.0;
const HINTS_GAP: f32 = 7.0;
const HINTS_TITLE_FONT: f32 = 13.0;
const HINTS_KEY_FONT: f32 = 11.5;
const HINTS_TEXT_FONT: f32 = 12.5;
const HINTS_LINE_HEIGHT: f32 = 1.618;
const HINTS_TITLE_HEIGHT: f32 = HINTS_TITLE_FONT * HINTS_LINE_HEIGHT;
const HINTS_ROW_HEIGHT: f32 = HINTS_TEXT_FONT * HINTS_LINE_HEIGHT;

fn text_advance(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|ch| {
            if ch.is_whitespace() {
                font_size * 0.35
            } else if ch.is_ascii() {
                font_size * 0.62
            } else {
                font_size
            }
        })
        .sum()
}

pub(crate) fn effects_panel_width(language: Language) -> f32 {
    const FONT: f32 = 11.0;
    let chip = |text: &str| text_advance(text, FONT) + 12.0;
    let corner = 14.0
        + 2.0
        + chip(t(language, "overlay.effects_off"))
        + chip("8")
        + chip("16")
        + chip("24")
        + 8.0;
    let shadow = text_advance(t(language, "overlay.effects_shadow"), FONT) + 16.0;
    (16.0 + corner + 6.0 + 1.0 + 6.0 + shadow + 6.0).max(140.0)
}

pub(crate) fn hints_panel_width() -> f32 {
    360.0
}

pub(crate) fn hints_panel_height(rows: usize) -> f32 {
    HINTS_PADDING * 2.0
        + HINTS_TITLE_HEIGHT
        + rows as f32 * HINTS_ROW_HEIGHT
        + rows as f32 * HINTS_GAP
        + 2.0
}

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

pub(crate) fn hints_card(
    theme: ChromeTheme,
    language: Language,
    width: f32,
    rows: &[(String, String)],
    shadow: bool,
) -> Div {
    let mut panel = div()
        .w(px(width))
        .flex()
        .flex_col()
        .gap(px(HINTS_GAP))
        .p(px(HINTS_PADDING))
        .rounded(px(theme.radius))
        .bg(theme.surface_strong)
        .border_1()
        .border_color(theme.border)
        .when(shadow, |panel| panel.shadow(theme.shadow()))
        .child(
            div()
                .text_size(px(HINTS_TITLE_FONT))
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
                .gap(px(10.0))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(HINTS_KEY_FONT))
                        .text_color(theme.muted)
                        .child(key.clone()),
                )
                .child(
                    div()
                        .text_size(px(HINTS_TEXT_FONT))
                        .text_color(theme.text)
                        .child(description.clone()),
                ),
        );
    }
    panel
}

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
    let panel = hints_card(theme, language, f32::from(rect.size.width), &rows, true)
        .id(format!("{id_prefix}-hints-panel"))
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        );
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
