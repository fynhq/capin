use super::*;

impl SettingsView {
    pub(super) fn save_location_row(
        &self,
        target: SaveTarget,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings().language;
        let custom = target.selected_directory(&self.settings()).is_some();
        let directory = target.directory(&self.settings()).display().to_string();
        let open = action_button(theme, t(language, "settings.open_folder").to_owned())
            .id(target.open_id())
            .role(Role::Button)
            .aria_label(t(language, "settings.open_folder"))
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(move |this, _, _, cx| this.open_directory(target, cx)));
        let change = action_button(theme, t(language, "settings.change_folder").to_owned())
            .id(target.change_id())
            .role(Role::Button)
            .aria_label(t(language, "settings.change_folder"))
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(move |this, _, _, cx| this.choose_directory(target, cx)));
        let reset = custom.then(|| {
            action_button(theme, t(language, "settings.use_default_folder").to_owned())
                .id(target.default_id())
                .role(Role::Button)
                .aria_label(t(language, "settings.use_default_folder"))
                .active(|style| style.opacity(0.88))
                .on_click(cx.listener(move |this, _, _, cx| this.reset_directory(target, cx)))
        });

        a11y_row(
            match target {
                SaveTarget::Screenshots => "settings-row-save-screenshots",
                SaveTarget::Recordings => "settings-row-save-recordings",
            },
            t(language, target.title_key()),
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::FOLDER,
                    t(language, target.title_key()),
                    "",
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_shrink_1()
                        .min_w(px(0.0))
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex_shrink_1()
                                .min_w(px(0.0))
                                .max_w(px(230.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis_middle()
                                .text_size(px(layout::CAPTION))
                                .text_color(theme.muted)
                                .child(directory),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(open)
                                .child(change)
                                .children(reset),
                        ),
                ),
        )
    }

    pub(super) fn filename_template_row(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings().language;
        self.filename_input.update(cx, |state, _| {
            state.set_editor_style(input_editor_style(theme));
        });
        let template = self.filename_input.read(cx).value().to_string();
        let preview = crate::storage::render_filename(&template, 1920, 1080);
        let focused = self
            .filename_input
            .read(cx)
            .presentation()
            .focus_handle()
            .is_focused(window);
        let field = div()
            .id("settings-filename-field")
            .w(px(260.0))
            .h(px(layout::BUTTON_HEIGHT))
            .flex_shrink_0()
            .px(px(9.0))
            .rounded(px(layout::BUTTON_RADIUS))
            .bg(theme.control)
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border })
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(layout::CONTROL_TEXT))
            .cursor_text()
            .child(Input::new(&self.filename_input))
            .on_mouse_down_out(cx.listener(|_, _: &MouseDownEvent, window, cx| window.blur(cx)))
            .on_click(cx.listener(|this, _, window, cx| {
                let focus = this
                    .filename_input
                    .read(cx)
                    .presentation()
                    .focus_handle()
                    .clone();
                window.focus(&focus, cx);
            }));

        a11y_row(
            "settings-row-filename-template",
            t(language, "settings.filename_template"),
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(layout::ROW_PAD_X))
                .py(px(layout::ROW_PAD_Y))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(24.0))
                        .child(row_label_with_icon(
                            theme,
                            icons::FILENAME,
                            t(language, "settings.filename_template"),
                            t(language, "settings.filename_template_desc"),
                        ))
                        .child(field),
                )
                .child(
                    div()
                        .pl(px(28.0))
                        .text_size(px(layout::CAPTION))
                        .text_color(theme.muted)
                        .child(format!(
                            "{} {}",
                            t(language, "settings.filename_template_preview"),
                            preview
                        )),
                ),
        )
    }

    pub(super) fn commit_filename_template(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = input.read(cx).value().to_string();
        let template: String = raw.trim().chars().take(120).collect();
        if template.is_empty() {
            let message = t(
                self.settings().language,
                "settings.filename_template_invalid",
            )
            .to_owned();
            self.show_status(message, cx);
            input.update(cx, |state, cx| {
                state.set_value(self.settings().filename_template.clone(), window, cx);
            });
            return;
        }
        if template != raw {
            input.update(cx, |state, cx| {
                state.set_value(template.clone(), window, cx);
            });
        }
        if template == self.settings().filename_template {
            return;
        }
        settings_state::update(|settings| settings.filename_template = template);
        let message = t(self.settings().language, "settings.filename_template_saved").to_owned();
        self.show_status(message, cx);
    }

    fn choose_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings().language;
        let prompt = t(language, "settings.choose_folder").to_owned();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        cx.spawn(async move |this, cx| {
            let (selected, error) = match receiver.await {
                Ok(Ok(Some(mut paths))) => (paths.pop(), None),
                Ok(Ok(None)) => (None, None),
                Ok(Err(error)) => (None, Some(error.to_string())),
                Err(_) => (None, None),
            };
            this.update(cx, |this, cx| {
                this.apply_directory_choice(target, selected, error, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_directory_choice(
        &mut self,
        target: SaveTarget,
        selected: Option<PathBuf>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings().language;
        if let Some(error) = error {
            self.show_status(
                format!("{} {}", t(language, "settings.status_picker_failed"), error),
                cx,
            );
            return;
        }
        let Some(directory) = selected else {
            return;
        };
        if let Err(error) = storage::ensure_directory(&directory) {
            self.show_status(
                format!("{} {}", t(language, "settings.status_folder_failed"), error),
                cx,
            );
            return;
        }
        let directory = Some(directory);
        settings_state::update(|settings| target.set_directory(settings, directory));
        self.show_status(t(language, "settings.status_folder_updated").to_owned(), cx);
    }

    fn reset_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings().language;
        settings_state::update(|settings| target.set_directory(settings, None));
        self.show_status(t(language, "settings.status_folder_default").to_owned(), cx);
    }

    fn open_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings().language;
        let directory = target.directory(&self.settings());
        if let Err(error) = storage::ensure_directory(&directory) {
            self.show_status(
                format!("{} {}", t(language, "settings.status_folder_failed"), error),
                cx,
            );
        } else {
            cx.open_with_system(&directory);
        }
    }
}
