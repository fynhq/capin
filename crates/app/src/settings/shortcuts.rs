use super::*;

impl SettingsView {
    pub(super) fn shortcuts(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings().language;
        let pin_hotkey = crate::hotkey::display_hotkey(&self.settings().pin_hotkey);
        let page = div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(layout::GROUP_HEADER_GAP))
                    .child(card(
                        theme,
                        vec![
                            self.shortcut_row(ShortcutSlot::Capture, theme, cx),
                            self.shortcut_row(ShortcutSlot::Pin, theme, cx),
                        ],
                    )),
            )
            .child(group(
                "settings-group-capture-keys",
                theme,
                t(language, "settings.group.capture_keys"),
                t(language, "settings.group.capture_keys_desc"),
                card(
                    theme,
                    vec![
                        info_row(
                            theme,
                            t(language, "shortcuts.capture_region"),
                            "",
                            t(language, "shortcuts.value.drag"),
                            Some(icons::SELECT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.window_capture"),
                            "",
                            t(language, "shortcuts.value.auto"),
                            Some(icons::WINDOW),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.window_mode"),
                            t(language, "shortcuts.window_mode_desc"),
                            "Ctrl",
                            Some(icons::CAMERA),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.adjust_target"),
                            t(language, "shortcuts.adjust_target_desc"),
                            t(language, "shortcuts.value.wheel"),
                            Some(icons::SCROLL),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.nudge_selection"),
                            t(language, "shortcuts.nudge_selection_desc"),
                            t(language, "shortcuts.value.arrow_keys"),
                            Some(icons::SELECT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_ratio"),
                            t(language, "shortcuts.toggle_ratio_desc"),
                            "R",
                            Some(icons::CROP),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_color_format"),
                            t(language, "shortcuts.toggle_color_format_desc"),
                            "Shift",
                            Some(icons::PALETTE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.copy_color"),
                            "",
                            "C",
                            Some(icons::CLIPBOARD),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.copy_selection"),
                            "",
                            "Enter",
                            Some(icons::COPY),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.save_capture"),
                            "",
                            t(language, "shortcuts.value.cmd_ctrl_s"),
                            Some(icons::SAVE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_current"),
                            "",
                            &pin_hotkey,
                            Some(icons::PIN),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_toolbar"),
                            "",
                            "Space",
                            Some(icons::MORE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.hints"),
                            "",
                            "F1",
                            Some(icons::SHORTCUTS),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.refresh"),
                            "",
                            "F5",
                            Some(icons::REFRESH),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.cancel"),
                            "",
                            "Esc",
                            Some(icons::CLOSE),
                        ),
                    ],
                ),
            ))
            .child(group(
                "settings-group-pin-keys",
                theme,
                t(language, "settings.group.pin_keys"),
                t(language, "settings.group.pin_keys_desc"),
                card(
                    theme,
                    vec![
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_move"),
                            t(language, "shortcuts.pin_move_desc"),
                            t(language, "shortcuts.value.drag"),
                            Some(icons::PIN),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_select_text"),
                            "",
                            t(language, "shortcuts.value.drag"),
                            Some(icons::TEXT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_word"),
                            "",
                            t(language, "shortcuts.value.double_click"),
                            Some(icons::TEXT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_copy_text"),
                            "",
                            t(language, "shortcuts.value.cmd_ctrl_c"),
                            Some(icons::COPY),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_select_all"),
                            "",
                            t(language, "shortcuts.value.cmd_ctrl_a"),
                            Some(icons::SELECT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_zoom"),
                            "",
                            t(language, "shortcuts.value.wheel"),
                            Some(icons::IMAGE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_opacity"),
                            "",
                            t(language, "shortcuts.value.shift_wheel"),
                            Some(icons::PALETTE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_menu"),
                            "",
                            t(language, "shortcuts.value.right_click"),
                            Some(icons::MORE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_close"),
                            "",
                            t(language, "shortcuts.value.esc_middle"),
                            Some(icons::CLOSE),
                        ),
                    ],
                ),
            ));
        page.when_some(crate::hotkey::registration_error(), |page, error| {
            page.child(note(
                theme,
                &format!("{} {error}", t(language, "shortcuts.unavailable")),
            ))
        })
    }

    fn shortcut_row(
        &self,
        slot: ShortcutSlot,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings().language;
        let recording = self.recording_shortcut == Some(slot);
        let display = if recording {
            t(language, "shortcuts.recording").to_owned()
        } else {
            crate::hotkey::display_hotkey(slot.value(&self.settings()))
        };
        let accent = theme.accent;
        let selected = theme.selected;
        let control = theme.control;
        let control_hover = theme.control_hover;
        let border = theme.border;
        let text = theme.text;

        a11y_row(
            slot.id(),
            t(language, slot.title_key()),
            div().child(
                row_shell()
                    .child(row_label_with_icon(
                        theme,
                        icons::SHORTCUTS,
                        t(language, slot.title_key()),
                        slot.description_key()
                            .map(|key| t(language, key))
                            .unwrap_or_default(),
                    ))
                    .child(
                        div()
                            .id(slot.id())
                            .h(px(layout::BUTTON_HEIGHT))
                            .px(px(12.0))
                            .rounded(px(layout::BUTTON_RADIUS))
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .cursor_pointer()
                            .bg(if recording { selected } else { control })
                            .border_1()
                            .border_color(if recording { accent } else { border })
                            .text_size(px(layout::CONTROL_TEXT))
                            .text_color(if recording { accent } else { text })
                            .when(!recording, |pill| {
                                pill.hover(move |style| style.bg(control_hover))
                            })
                            .active(move |style| style.opacity(0.88))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.recording_shortcut = Some(slot);
                                this.status = None;
                                window.focus(&this.focus_handle, cx);
                                cx.notify();
                            }))
                            .child(display),
                    ),
            ),
        )
    }

    pub(super) fn handle_shortcut_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(slot) = self.recording_shortcut else {
            return false;
        };
        let language = self.settings().language;
        if event.keystroke.key == "escape" {
            self.recording_shortcut = None;
            self.status = None;
            cx.notify();
            return true;
        }

        let modifiers = &event.keystroke.modifiers;
        if !(modifiers.control || modifiers.alt || modifiers.platform) {
            let message = t(language, "shortcuts.need_modifier").to_owned();
            self.show_status(message, cx);
            return true;
        }
        let Some(stored) = crate::hotkey::shortcut_from_key(
            &event.keystroke.key,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
            modifiers.platform,
        ) else {
            let message = t(language, "shortcuts.invalid").to_owned();
            self.show_status(message, cx);
            return true;
        };

        let settings = self.settings();
        let other = match slot {
            ShortcutSlot::Capture => &settings.pin_hotkey,
            ShortcutSlot::Pin => &settings.capture_hotkey,
        };
        if other == &stored {
            let message = t(language, "shortcuts.duplicate").to_owned();
            self.show_status(message, cx);
            return true;
        }

        let previous_capture = self.settings().capture_hotkey.clone();
        let previous_pin = self.settings().pin_hotkey.clone();
        match slot {
            ShortcutSlot::Capture => {
                settings_state::update(|settings| settings.capture_hotkey = stored.clone());
            }
            ShortcutSlot::Pin => {
                settings_state::update(|settings| settings.pin_hotkey = stored.clone());
            }
        }

        match crate::hotkey::refresh() {
            Ok(()) => {
                let message = t(language, "shortcuts.updated").to_owned();
                self.show_status(message, cx);
                self.recording_shortcut = None;
            }
            Err(error) => {
                settings_state::update(|settings| {
                    settings.capture_hotkey = previous_capture;
                    settings.pin_hotkey = previous_pin;
                });
                let _ = crate::hotkey::refresh();
                let message = format!("{}{}", t(language, "shortcuts.failed"), error);
                self.show_status(message, cx);
            }
        }
        cx.notify();
        true
    }
}
