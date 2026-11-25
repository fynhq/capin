use super::*;

impl SettingsView {
    fn content(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        let top = 0.0;
        #[cfg(not(target_os = "macos"))]
        let top = layout::TITLEBAR_HEIGHT;
        let content_width = (f32::from(window.bounds().size.width)
            - layout::SIDEBAR_WIDTH
            - layout::CONTENT_PAD_X * 2.0)
            .clamp(240.0, layout::CONTENT_MAX_WIDTH);
        let header = div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(layout::CONTENT_PAD_X))
            .pt(px(layout::CONTENT_PAD_TOP))
            .pb(px(layout::HEADER_GAP))
            .child(div().w(px(content_width)).child(self.section_header(theme)));
        #[cfg(target_os = "macos")]
        let top_block = self
            .macos_drag_area(
                "settings-content-drag-strip",
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(div().w_full().h(px(layout::TITLEBAR_HEIGHT)))
                    .child(header),
                cx,
            )
            .into_any_element();
        #[cfg(not(target_os = "macos"))]
        let top_block = header.into_any_element();
        div()
            .id("settings-content")
            .role(Role::Main)
            .aria_label(t(self.settings().language, "settings.a11y_content"))
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(top))
            .when_some(theme.page, |content, page| content.bg(page))
            .child(top_block)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .px(px(layout::CONTENT_PAD_X))
                            .pb(px(layout::CONTENT_PAD_BOTTOM))
                            .id(format!("settings-content-{}", self.section.id()))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                            .child(
                                div()
                                    .w(px(content_width))
                                    .flex()
                                    .flex_col()
                                    .gap(px(layout::CONTENT_GAP))
                                    .child(match self.section {
                                        SettingsSection::General => self.general(theme, window, cx),
                                        SettingsSection::Capture => {
                                            self.capture_settings(theme, cx)
                                        }
                                        SettingsSection::Recording => self.recording(theme, cx),
                                        SettingsSection::Shortcuts => {
                                            self.shortcuts(theme, window, cx)
                                        }
                                    })
                                    .with_animation(
                                        SharedString::from(format!(
                                            "settings-page-{}",
                                            self.section.id()
                                        )),
                                        Animation::new(Duration::from_millis(120))
                                            .with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                                        |page, delta| page.opacity(delta),
                                    ),
                            ),
                    )
                    .child(settings_scroll::scrollbar(theme, &self.scroll_handle)),
            )
    }

    fn section_header(&self, theme: &Theme) -> Div {
        let title = self.section.title(self.settings().language);
        div().flex().flex_col().child(
            div()
                .id("settings-section-heading")
                .role(Role::Heading)
                .aria_label(title)
                .aria_level(1)
                .text_size(px(layout::PAGE_TITLE))
                .text_color(theme.text)
                .child(title),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.settings().record_cursor_highlight {
            self.record_color_picker = None;
        }
        let theme = Theme::for_window(window, self.accent);
        let status = self.status.message().cloned();
        let status_serial = self.status.serial();
        let status_leaving = self.status.leaving();
        #[cfg(target_os = "windows")]
        let titlebar = Some(self.titlebar(&theme, window, cx));
        #[cfg(not(target_os = "windows"))]
        let titlebar: Option<Div> = None;
        let color_backdrop = self.record_color_picker.as_ref().map(|_| {
            div()
                .id("settings-record-color-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.record_color_picker = None;
                        cx.notify();
                    }),
                )
        });
        let select_backdrop = self.select_menu.map(|_| {
            div()
                .id("settings-select-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.select_menu = None;
                        cx.notify();
                    }),
                )
        });
        let select_popup = self.select_popup(&theme, window, cx);
        let record_color_popup = self.record_color_popup(&theme, window, cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .text_color(theme.text)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let command =
                    event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
                if this.record_color_picker.is_some() && this.handle_record_color_key(event, cx) {
                    return;
                }
                if this.select_menu.is_some() && event.keystroke.key == "escape" {
                    this.select_menu = None;
                    cx.notify();
                    return;
                }
                if command && matches!(event.keystroke.key.as_str(), "w" | "q") {
                    platform_window::set_macos_accessory_activation_policy();
                    let input = this.filename_input.clone();
                    this.commit_filename_template(&input, window, cx);
                    window.remove_window();
                    return;
                }
                this.handle_shortcut_key(event, window, cx);
            }))
            .child(self.sidebar(&theme, cx))
            .child(self.content(&theme, window, cx))
            .children(titlebar)
            .children(color_backdrop)
            .children(select_backdrop)
            .children(select_popup)
            .children(record_color_popup)
            .children(status.map(|message| {
                let duration = if status_leaving {
                    Duration::from_millis(150)
                } else {
                    Duration::from_millis(120)
                };
                div()
                    .absolute()
                    .bottom(px(16.0))
                    .right(px(16.0))
                    .max_w(px(480.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(layout::BUTTON_RADIUS))
                    .bg(rgba(0x000000CC))
                    .text_size(px(layout::CONTROL_TEXT))
                    .text_color(rgb(0xFFFFFF))
                    .child(message)
                    .with_animation(
                        SharedString::from(format!(
                            "settings-status-{status_serial}-{status_leaving}"
                        )),
                        Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                        move |toast, delta| {
                            if status_leaving {
                                toast.opacity(1.0 - delta)
                            } else {
                                toast.opacity(delta).bottom(px(16.0 - (1.0 - delta) * 4.0))
                            }
                        },
                    )
            }))
    }
}
