use super::*;

impl SettingsView {
    pub(super) fn capture_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings().language;

        div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(group(
                "settings-group-selection",
                theme,
                t(language, "settings.group.selection"),
                "",
                card(
                    theme,
                    vec![
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-capture-cursor",
                                icon: icons::CURSOR,
                                title_key: "settings.capture_cursor",
                                description_key: None,
                                value: self.settings().capture_cursor,
                                set: |settings, value| settings.capture_cursor = value,
                                refresh_pin: false,
                            },
                        ),
                        self.double_click_row(theme, cx),
                        self.save_location_row(SaveTarget::Screenshots, theme, cx),
                    ],
                ),
            ))
            .child(group(
                "settings-group-window",
                theme,
                t(language, "settings.group.window"),
                "",
                card(
                    theme,
                    vec![
                        self.window_background_row(theme, cx),
                        self.window_padding_row(theme, cx),
                    ],
                ),
            ))
            .child(group(
                "settings-group-annotation",
                theme,
                t(language, "settings.group.annotation"),
                "",
                card(
                    theme,
                    vec![self.toggle_row(
                        theme,
                        cx,
                        ToggleRow {
                            id: "settings-annotation-uniform-color",
                            icon: icons::PALETTE,
                            title_key: "settings.annotation_uniform_color",
                            description_key: Some("settings.annotation_uniform_color_desc"),
                            value: self.settings().annotation_uniform_color,
                            set: |settings, value| settings.annotation_uniform_color = value,
                            refresh_pin: false,
                        },
                    )],
                ),
            ))
            .child(group(
                "settings-group-pin",
                theme,
                t(language, "settings.group.pin"),
                "",
                card(
                    theme,
                    vec![
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-pin-text-ocr",
                                icon: icons::TEXT,
                                title_key: "settings.pin_text_ocr",
                                description_key: Some("settings.pin_text_ocr_desc"),
                                value: self.settings().pin_text_ocr,
                                set: |settings, value| settings.pin_text_ocr = value,
                                refresh_pin: true,
                            },
                        ),
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-pin-qr",
                                icon: icons::QR,
                                title_key: "settings.pin_qr",
                                description_key: None,
                                value: self.settings().pin_qr,
                                set: |settings, value| settings.pin_qr = value,
                                refresh_pin: true,
                            },
                        ),
                        self.repeat_pin_row(theme, cx),
                        self.pin_double_click_row(theme, cx),
                    ],
                ),
            ))
    }

    pub(super) fn toggle_row(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
        row: ToggleRow,
    ) -> AnyElement {
        let language = self.settings().language;
        a11y_row(
            row.id,
            t(language, row.title_key),
            div().child(
                row_shell()
                    .child(row_label_with_icon(
                        theme,
                        row.icon,
                        t(language, row.title_key),
                        row.description_key
                            .map(|key| t(language, key))
                            .unwrap_or_default(),
                    ))
                    .child(
                        div()
                            .id(row.id)
                            .role(Role::Switch)
                            .aria_label(t(language, row.title_key))
                            .aria_toggled(if row.value {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .cursor_pointer()
                            .active(move |style| style.opacity(0.85))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let value = !row.value;
                                settings_state::update(|settings| (row.set)(settings, value));
                                crate::hotkey::refresh_yield();
                                if row.refresh_pin {
                                    cx.defer(crate::pin::refresh_recognition);
                                }
                                this.status.clear();
                                cx.notify();
                            }))
                            .child(toggle(theme, row.value, row.id)),
                    ),
            ),
        )
    }

    fn window_background_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let current = t(
            language,
            background_label_key(self.settings().window_background),
        );
        a11y_row(
            "settings-row-window-background",
            t(language, "settings.window_background"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::IMAGE,
                    t(language, "settings.window_background"),
                    t(language, "settings.window_background_desc"),
                ))
                .child(self.select_trigger(
                    theme,
                    current,
                    "settings-window-background",
                    SelectKind::WindowBackground,
                    cx,
                )),
        )
    }

    fn window_padding_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let current = self.settings().window_padding.to_string();
        a11y_row(
            "settings-row-window-padding",
            t(language, "settings.window_padding"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::PADDING,
                    t(language, "settings.window_padding"),
                    "",
                ))
                .child(self.select_trigger(
                    theme,
                    &current,
                    "settings-window-padding",
                    SelectKind::WindowPadding,
                    cx,
                )),
        )
    }

    fn repeat_pin_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let current = t(language, repeat_pin_key(self.settings().repeat_pin));
        a11y_row(
            "settings-row-repeat-pin",
            t(language, "settings.repeat_pin"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::PIN,
                    t(language, "settings.repeat_pin"),
                    t(language, "settings.repeat_pin_desc"),
                ))
                .child(self.select_trigger(
                    theme,
                    current,
                    "settings-repeat-pin",
                    SelectKind::RepeatPin,
                    cx,
                )),
        )
    }

    fn double_click_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let current = t(
            language,
            double_click_key(self.settings().capture_double_click),
        );
        a11y_row(
            "settings-row-double-click",
            t(language, "settings.double_click"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::CURSOR,
                    t(language, "settings.double_click"),
                    t(language, "settings.double_click_desc"),
                ))
                .child(self.select_trigger(
                    theme,
                    current,
                    "settings-double-click",
                    SelectKind::DoubleClick,
                    cx,
                )),
        )
    }

    fn pin_double_click_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let current = t(
            language,
            pin_double_click_key(self.settings().pin_double_click),
        );
        a11y_row(
            "settings-row-pin-double-click",
            t(language, "settings.pin_double_click"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::CURSOR,
                    t(language, "settings.pin_double_click"),
                    t(language, "settings.pin_double_click_desc"),
                ))
                .child(self.select_trigger(
                    theme,
                    current,
                    "settings-pin-double-click",
                    SelectKind::PinDoubleClick,
                    cx,
                )),
        )
    }

    fn select_entries(&self, kind: SelectKind) -> Vec<(String, u32, bool)> {
        let language = self.settings().language;
        let mut entries: Vec<(String, u32, bool)> = Vec::new();
        match kind {
            SelectKind::WindowBackground => {
                for (index, background) in WindowBackground::ALL.into_iter().enumerate() {
                    entries.push((
                        t(language, background_label_key(background)).to_owned(),
                        index as u32,
                        self.settings().window_background == background,
                    ));
                }
            }
            SelectKind::WindowPadding => {
                for padding in WINDOW_PADDINGS {
                    entries.push((
                        t(language, padding_key(padding)).to_owned(),
                        padding,
                        self.settings().window_padding == padding,
                    ));
                }
            }
            SelectKind::RepeatPin => {
                for (index, behavior) in RepeatPinBehavior::ALL.into_iter().enumerate() {
                    entries.push((
                        t(language, repeat_pin_key(behavior)).to_owned(),
                        index as u32,
                        self.settings().repeat_pin == behavior,
                    ));
                }
            }
            SelectKind::DoubleClick => {
                for (index, action) in DoubleClickAction::ALL.into_iter().enumerate() {
                    entries.push((
                        t(language, double_click_key(action)).to_owned(),
                        index as u32,
                        self.settings().capture_double_click == action,
                    ));
                }
            }
            SelectKind::PinDoubleClick => {
                for (index, action) in PinDoubleClickAction::ALL.into_iter().enumerate() {
                    entries.push((
                        t(language, pin_double_click_key(action)).to_owned(),
                        index as u32,
                        self.settings().pin_double_click == action,
                    ));
                }
            }
        }
        entries
    }

    fn apply_select(&mut self, kind: SelectKind, value: u32, cx: &mut Context<Self>) {
        match kind {
            SelectKind::WindowBackground => {
                let background = WindowBackground::ALL[value as usize];
                settings_state::update(|settings| settings.window_background = background);
            }
            SelectKind::WindowPadding => {
                if WINDOW_PADDINGS.contains(&value) {
                    settings_state::update(|settings| settings.window_padding = value);
                }
            }
            SelectKind::RepeatPin => {
                let behavior = RepeatPinBehavior::ALL[value as usize];
                settings_state::update(|settings| settings.repeat_pin = behavior);
            }
            SelectKind::DoubleClick => {
                let action = DoubleClickAction::ALL[value as usize];
                settings_state::update(|settings| settings.capture_double_click = action);
            }
            SelectKind::PinDoubleClick => {
                let action = PinDoubleClickAction::ALL[value as usize];
                settings_state::update(|settings| settings.pin_double_click = action);
            }
        }
        self.status.clear();
        cx.notify();
    }

    fn select_trigger(
        &self,
        theme: &Theme,
        label: &str,
        id: &'static str,
        kind: SelectKind,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let anchor: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let anchor_for_paint = anchor.clone();
        let probe = canvas(
            move |bounds, _, _| anchor_for_paint.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element();
        let hover = theme.control_hover;
        let chevron = theme.muted;
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.to_owned())
            .relative()
            .w(px(160.0))
            .h(px(layout::BUTTON_HEIGHT))
            .flex_shrink_0()
            .px(px(10.0))
            .rounded(px(layout::BUTTON_RADIUS))
            .bg(theme.control)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .text_size(px(layout::CONTROL_TEXT))
            .text_color(theme.text)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .active(move |style| style.opacity(0.9))
            .on_click(cx.listener(move |this, _, _, cx| {
                #[cfg(target_os = "macos")]
                {
                    let entries = this.select_entries(kind);
                    let selected = entries.iter().position(|(_, _, active)| *active);
                    let items: Vec<String> =
                        entries.iter().map(|(label, _, _)| label.clone()).collect();
                    let values: Vec<u32> = entries.iter().map(|(_, value, _)| *value).collect();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(1))
                            .await;
                        let Some(index) =
                            platform_window::show_native_menu(&items, selected.unwrap_or(0))
                        else {
                            return;
                        };
                        let Some(value) = values.get(index).copied() else {
                            return;
                        };
                        this.update(cx, |this, cx| this.apply_select(kind, value, cx))
                            .ok();
                    })
                    .detach();
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if let Some(anchor) = anchor.get() {
                        this.select_menu = Some(SelectMenu { kind, anchor });
                    }
                    cx.notify();
                }
            }))
            .child(label.to_owned())
            .child(icons::icon(icons::CHEVRON_DOWN, 10.0, chevron))
            .child(probe)
    }

    pub(super) fn select_popup(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.select_menu?;
        let entries = self.select_entries(menu.kind);

        let width = 160.0;
        let item_height = 26.0;
        let padding = 4.0;
        let height = entries.len() as f32 * item_height + padding * 2.0;
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let x = crate::ui::popover_left(
            menu.anchor,
            width,
            viewport_width,
            crate::ui::PopoverAlign::End,
        );
        let y = crate::ui::popover_top_with_gap(menu.anchor, height, viewport_height, None, 4.0);

        let mut popup = div()
            .id("settings-select-popup")
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(width))
            .p(px(padding))
            .rounded(px(6.0))
            .bg(theme.popover)
            .shadow(vec![
                BoxShadow::new(
                    px(0.0),
                    px(0.0),
                    Hsla::from(rgba((u32::from(theme.text) & 0xFFFFFF00) | 0x1A)),
                )
                .spread_radius(px(1.0)),
                BoxShadow::new(px(0.0), px(4.0), Hsla::from(rgba(0x0000001A)))
                    .blur_radius(px(3.0))
                    .spread_radius(px(-1.0)),
                BoxShadow::new(px(0.0), px(2.0), Hsla::from(rgba(0x0000001A)))
                    .blur_radius(px(2.0))
                    .spread_radius(px(-2.0)),
            ])
            .flex()
            .flex_col()
            .occlude();
        for (index, (label, value, selected)) in entries.into_iter().enumerate() {
            let selected_bg = theme.option_selected;
            let check = theme.text;
            let text = theme.text;
            let hover = theme.option_hover;
            popup = popup.child(
                div()
                    .id(("settings-select-item", index))
                    .h(px(item_height))
                    .px(px(7.0))
                    .rounded(px(6.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(px(layout::CONTROL_TEXT))
                    .when(selected, |item| item.bg(selected_bg).text_color(text))
                    .when(!selected, |item| {
                        item.text_color(text).hover(move |style| style.bg(hover))
                    })
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.select_menu = None;
                        this.apply_select(menu.kind, value, cx);
                    }))
                    .child(div().flex_1().min_w(px(0.0)).child(label))
                    .child(div().w(px(12.0)).flex_shrink_0().when(selected, |slot| {
                        slot.child(icons::icon(icons::CHECK, 12.0, check))
                    })),
            );
        }
        let base_top = y;
        Some(
            crate::ui::enter_slide_fade(
                popup,
                SharedString::from(format!("settings-select-popup-{}", menu.kind as u8)),
                base_top,
                -8.0,
                Duration::from_millis(150),
            )
            .into_any_element(),
        )
    }
}
