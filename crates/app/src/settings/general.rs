use super::*;

impl SettingsView {
    pub(super) fn general(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        #[cfg(target_os = "macos")]
        let language = self.settings().language;
        let content = card(
            theme,
            vec![
                self.language_row(theme, cx),
                self.auto_start_row(theme, cx),
                self.toggle_row(
                    theme,
                    cx,
                    ToggleRow {
                        id: "settings-hide-tray",
                        icon: icons::GENERAL,
                        title_key: "settings.hide_tray",
                        description_key: Some("settings.hide_tray_desc"),
                        value: self.settings().hide_tray,
                        set: |settings, value| {
                            settings.hide_tray = value;
                            crate::tray::set_visible(!value);
                        },
                        refresh_pin: false,
                    },
                ),
                self.logs_row(theme, cx),
                self.filename_template_row(theme, window, cx),
            ],
        );
        let page = div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(content);
        #[cfg(target_os = "macos")]
        let page = page.child(group(
            "settings-group-permissions",
            theme,
            t(language, "settings.group.permissions"),
            "",
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(self.permissions_card(theme, cx))
                .when(
                    !(self.permission_screen && self.permission_accessibility),
                    |body| body.child(self.permission_hint(theme, cx)),
                ),
        ));
        page
    }

    #[cfg(target_os = "macos")]
    fn permission_hint(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings().language;
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(note(theme, t(language, "settings.permission_hint"))),
            )
            .child(
                action_button(theme, t(language, "settings.permission_restart").to_owned())
                    .id("settings-permission-restart")
                    .role(Role::Button)
                    .aria_label(t(language, "settings.permission_restart"))
                    .active(|style| style.opacity(0.88))
                    .on_click(cx.listener(|_, _, _, cx| crate::tray::restart(cx))),
            )
    }

    #[cfg(target_os = "macos")]
    fn permissions_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings().language;
        card(
            theme,
            vec![
                self.permission_row(
                    theme,
                    t(language, "settings.permission_screen"),
                    t(language, "settings.permission_screen_desc"),
                    self.permission_screen,
                    platform_window::PrivacyPane::ScreenCapture,
                    cx,
                ),
                self.permission_row(
                    theme,
                    t(language, "settings.permission_accessibility"),
                    t(language, "settings.permission_accessibility_desc"),
                    self.permission_accessibility,
                    platform_window::PrivacyPane::Accessibility,
                    cx,
                ),
            ],
        )
    }

    #[cfg(target_os = "macos")]
    fn permission_row(
        &self,
        theme: &Theme,
        title: &str,
        description: &str,
        granted: bool,
        pane: platform_window::PrivacyPane,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings().language;
        let status = t(
            language,
            if granted {
                "settings.permission_granted"
            } else {
                "settings.permission_missing"
            },
        );
        a11y_row(
            match pane {
                platform_window::PrivacyPane::ScreenCapture => "settings-row-permission-screen",
                platform_window::PrivacyPane::Accessibility => {
                    "settings-row-permission-accessibility"
                }
            },
            title.to_owned(),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::SHIELD,
                    title,
                    description,
                ))
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(px(layout::CAPTION))
                                .text_color(if granted { theme.muted } else { theme.accent })
                                .child(status),
                        )
                        .child(
                            action_button(theme, t(language, "settings.open_settings").to_owned())
                                .id(match pane {
                                    platform_window::PrivacyPane::ScreenCapture => {
                                        "settings-open-screen-permission"
                                    }
                                    platform_window::PrivacyPane::Accessibility => {
                                        "settings-open-accessibility-permission"
                                    }
                                })
                                .role(Role::Button)
                                .aria_label(t(language, "settings.open_settings"))
                                .active(|style| style.opacity(0.88))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    platform_window::open_privacy_settings(pane);
                                    this.permission_screen =
                                        platform_window::screen_capture_trusted();
                                    this.permission_accessibility =
                                        platform_window::accessibility_trusted();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    fn language_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let accent = theme.accent;
        let mut control = segmented();
        for option in Language::ALL {
            let selected = language == option;
            let label = option.label();
            let code = option.code();
            let focus_id = match option {
                Language::Zh => "settings-language-zh",
                Language::En => "settings-language-en",
            };
            let focus = super::control_focus(self, focus_id, cx);
            let focus_for_click = focus.clone();
            control = control.child(
                segment(theme, label, selected)
                    .id(code)
                    .role(Role::Button)
                    .aria_label(label.to_owned())
                    .aria_selected(selected)
                    .track_focus(&focus)
                    .focus_visible(move |style| style.shadow(crate::ui::focus_ring(accent)))
                    .active(|style| style.opacity(0.9))
                    .on_key_down(cx.listener(move |this, event, window, cx| {
                        if super::handle_tab_key(this, event, window, cx) {
                            return;
                        }
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            set_language(this, option, window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&focus_for_click, cx);
                        set_language(this, option, window, cx);
                    })),
            );
        }

        a11y_row(
            "settings-row-language",
            t(language, "settings.language"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::LANGUAGE,
                    t(language, "settings.language"),
                    "",
                ))
                .child(control),
        )
    }

    fn auto_start_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let value = self.settings().auto_start;
        let focus = super::control_focus(self, "settings-auto-start", cx);
        let focus_for_click = focus.clone();
        let accent = theme.accent;

        a11y_row(
            "settings-row-auto-start",
            t(language, "settings.auto_start"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::STARTUP,
                    t(language, "settings.auto_start"),
                    "",
                ))
                .child(
                    div()
                        .id("settings-auto-start")
                        .role(Role::Switch)
                        .aria_label(t(language, "settings.auto_start"))
                        .aria_toggled(if value { Toggled::True } else { Toggled::False })
                        .track_focus(&focus)
                        .focus_visible(move |style| style.shadow(crate::ui::focus_ring(accent)))
                        .active(move |style| style.opacity(0.85))
                        .on_key_down(cx.listener(|this, event, window, cx| {
                            if super::handle_tab_key(this, event, window, cx) {
                                return;
                            }
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                toggle_auto_start(this, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            window.focus(&focus_for_click, cx);
                            toggle_auto_start(this, cx);
                        }))
                        .child(toggle(
                            theme,
                            value,
                            "settings-auto-start-toggle",
                            cx.reduce_motion(),
                        )),
                ),
        )
    }

    fn logs_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let language = self.settings().language;
        let open = action_button(theme, t(language, "settings.open_logs").to_owned())
            .id("settings-open-logs")
            .role(Role::Button)
            .aria_label(t(language, "settings.open_logs"))
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(|_, _, _, cx| {
                let directory = crate::logging::log_directory();
                if let Err(error) = std::fs::create_dir_all(&directory) {
                    log_warn!("settings", "failed to create the log directory: {error}");
                }
                log_info!("settings", "opening log directory");
                cx.open_with_system(&directory);
            }));
        let copy = action_button(theme, t(language, "settings.copy_diagnostics").to_owned())
            .id("settings-copy-diagnostics")
            .role(Role::Button)
            .aria_label(t(language, "settings.copy_diagnostics"))
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(|this, _, _, cx| {
                let report = diagnostics_report(&this.settings());
                cx.write_to_clipboard(ClipboardItem::new_string(report));
                let message = t(this.settings().language, "settings.diagnostics_copied").to_owned();
                this.show_status(message, cx);
            }));
        a11y_row(
            "settings-row-logs",
            t(language, "settings.logs"),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::LOGS,
                    t(language, "settings.logs"),
                    t(language, "settings.logs_desc"),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(open)
                        .child(copy),
                ),
        )
    }

    pub(super) fn open_project(&self, cx: &mut Context<Self>) {
        cx.open_url(PROJECT_URL);
    }
}

fn diagnostics_report(settings: &AppSettings) -> String {
    let mut report = String::new();
    report.push_str(&crate::build_info::version_label());
    report.push('\n');
    report.push_str(&format!(
        "OS: {} {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    report.push_str(&format!("Language: {:?}\n", settings.language));
    report.push_str(&format!("Capture hotkey: {}\n", settings.capture_hotkey));
    report.push_str(&format!("Pin hotkey: {}\n", settings.pin_hotkey));
    report.push_str(&format!("Autostart: {}\n", settings.auto_start));
    report.push_str(&format!(
        "Filename template: {}\n",
        settings.filename_template
    ));
    report.push_str("Recent log:\n");
    for line in crate::logging::recent_lines(40) {
        report.push_str(&line);
        report.push('\n');
    }
    report
}

pub(super) fn settings_bounds(cx: &App) -> Bounds<Pixels> {
    let settings = settings_state::get();
    let displays = cx.displays();
    let primary = cx.primary_display();
    let display = settings
        .settings_window_display
        .and_then(|id| {
            displays
                .iter()
                .find(|display| u64::from(display.id()) == id)
                .cloned()
        })
        .or(primary)
        .or_else(|| displays.first().cloned());
    let Some(display) = display else {
        return Bounds {
            origin: point(px(80.0), px(80.0)),
            size: size(px(layout::WINDOW_WIDTH), px(layout::WINDOW_HEIGHT)),
        };
    };
    let visible = display.visible_bounds();
    let available_width = (f32::from(visible.size.width) - 32.0).max(1.0);
    let available_height = (f32::from(visible.size.height) - 32.0).max(1.0);
    let (stored_width, stored_height, stored_origin) = match settings.settings_window_frame {
        Some([left, top, width, height]) => (width, height, Some(point(px(left), px(top)))),
        None => (layout::WINDOW_WIDTH, layout::WINDOW_HEIGHT, None),
    };
    let width = stored_width
        .min(available_width)
        .max(layout::MIN_WIDTH.min(available_width));
    let height = stored_height
        .min(available_height)
        .max(layout::MIN_HEIGHT.min(available_height));
    let left = f32::from(visible.origin.x);
    let top = f32::from(visible.origin.y);
    let default_x = left + (f32::from(visible.size.width) - width) / 2.0;
    let default_y = top + (f32::from(visible.size.height) - height) / 2.0;
    let x = stored_origin
        .map(|origin| f32::from(origin.x))
        .unwrap_or(default_x)
        .clamp(
            left + 8.0,
            (left + f32::from(visible.size.width) - width - 8.0).max(left + 8.0),
        );
    let y = stored_origin
        .map(|origin| f32::from(origin.y))
        .unwrap_or(default_y)
        .clamp(
            top + 8.0,
            (top + f32::from(visible.size.height) - height - 8.0).max(top + 8.0),
        );
    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    }
}

fn set_language(
    view: &mut SettingsView,
    option: Language,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    settings_state::update(|settings| settings.language = option);
    crate::tray::refresh_menu();
    window.set_window_title(t(option, "settings.title"));
    view.status.clear();
    cx.notify();
}

fn toggle_auto_start(view: &mut SettingsView, cx: &mut Context<SettingsView>) {
    let language = view.settings().language;
    let desired = !view.settings().auto_start;
    match set_autostart(desired) {
        Ok(AutostartState::Unsupported) => {
            view.show_status(
                t(language, "settings.status_autostart_failed").to_owned(),
                cx,
            );
        }
        Ok(state) => {
            settings_state::update(|settings| {
                settings.auto_start = desired;
                settings.auto_start_explicit = true;
            });
            let message = match state {
                AutostartState::Enabled => t(language, "settings.status_autostart_enabled"),
                AutostartState::Disabled => t(language, "settings.status_autostart_disabled"),
                AutostartState::RequiresApproval => {
                    t(language, "settings.status_autostart_approval")
                }
                AutostartState::Unsupported => t(language, "settings.status_autostart_failed"),
            };
            view.show_status(message.to_owned(), cx);
        }
        Err(error) => {
            view.show_status(
                format!(
                    "{}{}",
                    t(language, "settings.status_autostart_failed"),
                    error
                ),
                cx,
            );
        }
    }
    cx.notify();
}
