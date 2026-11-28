use super::*;

impl SettingsView {
    #[cfg(target_os = "windows")]
    pub(super) fn titlebar(
        &self,
        theme: &Theme,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Div {
        let leading = div()
            .id("settings-titlebar-drag")
            .flex_1()
            .h_full()
            .pl(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.text)
                    .child(t(self.settings().language, "settings.title")),
            );

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(layout::TITLEBAR_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .child(leading)
            .child(window_control_button(
                theme,
                "settings-window-minimize",
                icons::MINIMIZE,
                WindowControlArea::Min,
                false,
            ))
            .child(window_control_button(
                theme,
                "settings-window-maximize",
                if window.is_maximized() {
                    icons::RESTORE
                } else {
                    icons::MAXIMIZE
                },
                WindowControlArea::Max,
                false,
            ))
            .child(window_control_button(
                theme,
                "settings-window-close",
                icons::WINDOW_CLOSE,
                WindowControlArea::Close,
                true,
            ))
    }
}

#[cfg(target_os = "windows")]
fn window_control_button(
    theme: &Theme,
    id: &'static str,
    icon: &'static [u8],
    area: WindowControlArea,
    close: bool,
) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Button)
        .w(px(layout::CAPTION_BUTTON_WIDTH))
        .h(px(layout::CAPTION_BUTTON_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .when(close, |button| {
            button
                .group("settings-window-close")
                .hover(move |style| style.bg(theme.close_hover))
        })
        .when(!close, |button| {
            button.hover(move |style| style.bg(theme.selected))
        })
        .child(
            svg()
                .data(icon)
                .w(px(10.0))
                .h(px(10.0))
                .text_color(theme.text)
                .when(close, |glyph| {
                    glyph.group_hover("settings-window-close", |style| {
                        style.text_color(rgb(0xFFFFFF))
                    })
                }),
        )
}
