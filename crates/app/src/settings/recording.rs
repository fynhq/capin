use super::*;

impl SettingsView {
    pub(super) fn recording(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings().language;
        let mut rows = vec![self.toggle_row(
            theme,
            cx,
            ToggleRow {
                id: "settings-record-cursor-highlight",
                icon: icons::CURSOR,
                title_key: "settings.record_cursor_highlight",
                description_key: Some("settings.record_cursor_highlight_desc"),
                value: self.settings().record_cursor_highlight,
                set: |settings, value| settings.record_cursor_highlight = value,
                refresh_pin: false,
            },
        )];
        if self.settings().record_cursor_highlight {
            rows.push(self.record_highlight_color_row(theme, cx));
        }
        rows.push(self.save_location_row(SaveTarget::Recordings, theme, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_HEADER_GAP))
            .child(note(theme, t(language, "settings.recording_note")))
            .child(card(theme, rows))
    }

    fn record_highlight_color_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let current = self.settings().record_cursor_highlight_color;
        let mut swatches = Vec::new();
        for (index, color) in settings_state::RECORD_HIGHLIGHT_COLORS
            .into_iter()
            .enumerate()
        {
            let selected = current == color;
            let swatch = div()
                .id(("settings-record-highlight-color", index))
                .w(px(28.0))
                .h(px(28.0))
                .flex_shrink_0()
                .rounded(px(14.0))
                .p(px(3.0))
                .border_1()
                .when(selected, |item| item.border_2().border_color(theme.accent))
                .when(!selected, |item| item.border_color(theme.border))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.record_color_picker = None;
                    settings_state::update(|settings| {
                        settings.record_cursor_highlight_color = color
                    });
                    this.status.clear();
                    cx.notify();
                }))
                .child(
                    div()
                        .size_full()
                        .rounded(px(11.0))
                        .bg(rgb(color))
                        .border_1()
                        .border_color(theme.card_border),
                );
            swatches.push(swatch.into_any_element());
        }

        let anchor: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let anchor_for_paint = anchor.clone();
        let probe = canvas(
            move |bounds, _, _| anchor_for_paint.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element();
        let custom_selected = !settings_state::RECORD_HIGHLIGHT_COLORS.contains(&current);
        let picker_open = self.record_color_picker.is_some();
        let custom_tooltip = t(language, "editor.custom_color").to_owned();
        let custom = div()
            .id("settings-record-highlight-custom")
            .tooltip(move |_, cx| crate::ui::tooltip_view(custom_tooltip.clone(), cx))
            .relative()
            .w(px(28.0))
            .h(px(28.0))
            .flex_shrink_0()
            .rounded(px(14.0))
            .p(px(3.0))
            .border_1()
            .when(custom_selected || picker_open, |item| {
                item.border_2().border_color(theme.accent)
            })
            .when(!custom_selected && !picker_open, |item| {
                item.border_color(theme.border)
            })
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(anchor) = anchor.get() {
                    this.open_record_color_picker(anchor, window, cx);
                }
            }))
            .child(
                div()
                    .size_full()
                    .rounded(px(11.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(custom_selected, |item| item.bg(rgb(current)))
                    .when(!custom_selected, |item| {
                        item.bg(theme.control)
                            .child(icons::icon(icons::PALETTE, 14.0, theme.muted))
                    })
                    .border_1()
                    .border_color(theme.card_border),
            )
            .child(probe);
        swatches.push(custom.into_any_element());

        a11y_row(
            "settings-row-highlight-color",
            t(language, "settings.record_cursor_highlight_color"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::PALETTE,
                    t(language, "settings.record_cursor_highlight_color"),
                    t(language, "settings.record_cursor_highlight_color_desc"),
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .children(swatches),
                ),
        )
    }

    fn open_record_color_picker(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record_color_picker = Some(RecordColorPicker {
            anchor,
            buffer: format!("{:06x}", self.settings().record_cursor_highlight_color),
            fresh: true,
        });
        self.select_menu = None;
        self.status.clear();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn apply_record_color_buffer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.record_color_picker.as_mut() else {
            return false;
        };
        let hex = picker.buffer.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        let Ok(rgb) = u32::from_str_radix(hex, 16) else {
            return false;
        };
        let color = rgb & 0x00FF_FFFF;
        settings_state::update(|settings| settings.record_cursor_highlight_color = color);
        self.status.clear();
        cx.notify();
        true
    }

    pub(super) fn handle_record_color_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = self.record_color_picker.as_mut() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.record_color_picker = None;
                cx.notify();
                return true;
            }
            "enter" => {
                self.apply_record_color_buffer(cx);
                return true;
            }
            "backspace" | "delete" => {
                if picker.fresh {
                    picker.buffer.clear();
                    picker.fresh = false;
                } else {
                    picker.buffer.pop();
                }
                cx.notify();
                return true;
            }
            _ => {}
        }
        let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        if !command
            && let Some(ch) = event.keystroke.key.chars().next()
            && ch.is_ascii_hexdigit()
        {
            if picker.fresh {
                picker.buffer.clear();
                picker.fresh = false;
            }
            if picker.buffer.len() < 6 {
                picker.buffer.push(ch.to_ascii_lowercase());
            }
            cx.notify();
            return true;
        }
        !command
    }

    pub(super) fn record_color_popup(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.record_color_picker.as_ref()?;
        let language = self.settings().language;
        let width = 240.0;
        let height = 84.0;
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let x = crate::ui::popover_left(
            picker.anchor,
            width,
            viewport_width,
            crate::ui::PopoverAlign::Center,
        );
        let y = crate::ui::popover_top_with_gap(picker.anchor, height, viewport_height, None, 4.0);

        let preview_rgb = picker.buffer.trim().trim_start_matches('#');
        let preview = u32::from_str_radix(preview_rgb, 16)
            .ok()
            .filter(|_| preview_rgb.len() == 6)
            .map(|rgb| (rgb & 0x00FF_FFFF) << 8 | 0xFF)
            .unwrap_or((self.settings().record_cursor_highlight_color << 8) | 0xFF);
        let mut hue_row = div().flex().flex_row().items_center().gap(px(3.0));
        for index in 0..12u32 {
            let hue_color = crate::ui::hue_rgb(index as f32 * 30.0);
            hue_row = hue_row.child(
                div()
                    .id(("settings-record-hue", index))
                    .w(px(15.0))
                    .h(px(15.0))
                    .rounded(px(8.0))
                    .bg(rgb(hue_color))
                    .border_1()
                    .border_color(theme.border)
                    .hover(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let color = hue_color;
                        settings_state::update(|settings| {
                            settings.record_cursor_highlight_color = color
                        });
                        if let Some(picker) = this.record_color_picker.as_mut() {
                            picker.buffer = format!("{color:06x}");
                            picker.fresh = false;
                        }
                        this.status.clear();
                        cx.notify();
                    })),
            );
        }

        Some(
            crate::ui::enter_slide_fade(
                div()
                    .id("settings-record-color-popup")
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(width))
                    .p(px(8.0))
                    .rounded(px(6.0))
                    .bg(theme.popover)
                    .shadow(vec![
                        BoxShadow::new(px(0.0), px(0.0), Hsla::from(theme.border))
                            .spread_radius(px(1.0)),
                        BoxShadow::new(px(0.0), px(4.0), Hsla::from(rgba(0x0000001F)))
                            .blur_radius(px(3.0))
                            .spread_radius(px(-1.0)),
                    ])
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .occlude()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .w(px(18.0))
                                    .h(px(18.0))
                                    .rounded(px(4.0))
                                    .bg(rgba(preview))
                                    .border_1()
                                    .border_color(theme.border),
                            )
                            .child(
                                div()
                                    .text_size(px(crate::ui::FONT_CAPTION))
                                    .text_color(theme.text)
                                    .child(t(language, "editor.custom_color")),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(theme.control)
                                    .text_size(px(crate::ui::FONT_CAPTION))
                                    .text_color(theme.text)
                                    .child({
                                        let mut row = div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(1.0))
                                            .child(format!(
                                                "#{}",
                                                picker.buffer.to_ascii_uppercase()
                                            ));
                                        if !picker.fresh {
                                            row = row.child(
                                                div()
                                                    .w(px(1.0))
                                                    .h(px(11.0))
                                                    .bg(theme.accent)
                                                    .with_animation(
                                                        "settings-record-color-caret",
                                                        crate::ui::caret_blink(),
                                                        |caret, delta| caret.opacity(delta),
                                                    ),
                                            );
                                        }
                                        row
                                    }),
                            ),
                    )
                    .child(hue_row)
                    .child(
                        div()
                            .text_size(px(crate::ui::FONT_MICRO))
                            .text_color(theme.muted)
                            .child(t(language, "editor.custom_color_hint")),
                    ),
                "settings-record-color-enter",
                y,
                0.0,
                Duration::from_millis(50),
                cx.reduce_motion(),
            )
            .into_any_element(),
        )
    }
}
