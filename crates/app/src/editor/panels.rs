use super::*;

impl EditorView {
    pub(super) fn refresh_capture_action(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_capture(cx);
    }

    pub(super) fn show_status(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = Some(message);
        self.status_serial = self.status_serial.wrapping_add(1);
        let serial = self.status_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            this.update(cx, |this, cx| {
                if this.status_serial == serial {
                    this.status = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn toggle_effects_panel(&mut self, cx: &mut Context<Self>) {
        self.effects_open = !self.effects_open;
        self.more_open = false;
        self.hints_visible = false;
        cx.notify();
    }

    pub(super) fn set_region_radius(&mut self, radius: u32, cx: &mut Context<Self>) {
        self.effects.corner_radius = radius;
        settings_state::update(|settings| {
            settings.region_corner_radius = radius;
        });
        cx.notify();
    }

    pub(super) fn toggle_region_shadow(&mut self, cx: &mut Context<Self>) {
        self.effects.shadow = !self.effects.shadow;
        let shadow = self.effects.shadow;
        settings_state::update(|settings| settings.region_shadow = shadow);
        cx.notify();
    }

    pub(super) fn dismiss_effects(&mut self, cx: &mut Context<Self>) {
        self.effects_open = false;
        cx.notify();
    }

    pub(super) fn dismiss_hints(&mut self, cx: &mut Context<Self>) {
        self.hints_visible = false;
        cx.notify();
    }
}

impl EditorView {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn label_option_chip<F>(
        &self,
        id: String,
        label: String,
        tooltip: String,
        selected: bool,
        width: f32,
        cx: &mut Context<Self>,
        apply: F,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Context<Self>) + 'static,
    {
        let accent = self.chrome.accent;
        let accent_tint = self.chrome.accent_tint;
        let hover = self.chrome.hover;
        let muted = self.chrome.muted;
        div()
            .id(id)
            .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
            .w(px(width))
            .h(px(22.0))
            .rounded(px(5.0))
            .flex()
            .items_center()
            .justify_center()
            .when(selected, |button| button.bg(accent_tint))
            .when(!selected, move |button| {
                button.hover(move |style| style.bg(hover))
            })
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(move |this, _, _, cx| apply(this, cx)))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if selected { accent } else { muted })
                    .child(label),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn toolbar_button(
        &self,
        id: &'static str,
        label: String,
        icon: &'static [u8],
        enabled: bool,
        color: Rgba,
        cx: &mut Context<Self>,
        action: fn(&mut Self, &mut Context<Self>),
    ) -> AnyElement {
        let hover = self.chrome.hover;
        if !enabled {
            return div()
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.35)
                .child(icons::icon(icon, 18.0, self.chrome.muted))
                .into_any_element();
        }
        div()
            .id(id)
            .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
            .child(icons::icon(icon, 18.0, color))
            .into_any_element()
    }
}

impl EditorView {
    pub(super) fn more_row(
        &self,
        id: &'static str,
        icon: &'static [u8],
        label: &'static str,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(5.0))
            .hover(|style| style.bg(self.chrome.hover))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.more_open = false;
                action(this, cx);
                cx.notify();
            }))
            .child(icons::icon(icon, 15.0, self.chrome.icon))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(self.chrome.text)
                    .child(label),
            )
            .into_any_element()
    }

    pub(super) fn info_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let inline = self.inline?;
        let capture = inline.capture?;
        let rect = self.info_rect(window)?;
        let language = settings_state::get().language;
        let selection = self.live_selection_rect().unwrap_or(capture.region.rect);
        let absolute_x = capture.monitor_origin.x + selection.origin.x;
        let absolute_y = capture.monitor_origin.y + selection.origin.y;
        let width = selection.size.width;
        let height = selection.size.height;
        let mut bar = div()
            .id("editor-info-bar")
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .text_size(px(11.0))
            .child(
                div()
                    .text_color(self.chrome.muted)
                    .child(format!("{absolute_x}, {absolute_y}")),
            )
            .child(div().w(px(1.0)).h(px(14.0)).bg(self.chrome.separator))
            .child(
                div()
                    .text_color(self.chrome.text)
                    .child(format!("{width} x {height} px")),
            );

        if !matches!(capture.target, CaptureTarget::Window(_)) {
            bar = bar.child(crate::capture_panels::effects_chip(
                "editor-info-effects",
                self.chrome,
                language,
                self.effects_open,
                self.effects.active(),
                cx,
                Self::toggle_effects_panel,
            ));
        }

        bar = bar.child(crate::capture_panels::refresh_button(
            "editor-info-refresh",
            self.chrome,
            language,
            cx,
            Self::refresh_capture_action,
        ));

        Some(bar.into_any_element())
    }

    pub(super) fn effects_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.effects_open {
            return None;
        }
        let rect = self.effects_rect(window)?;
        let language = settings_state::get().language;
        Some(crate::capture_panels::effects_panel(
            "editor",
            self.chrome,
            language,
            rect,
            self.effects,
            4.0,
            cx,
            Self::set_region_radius,
            Self::toggle_region_shadow,
            Self::dismiss_effects,
        ))
    }

    pub(super) fn hints_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.hints_visible {
            return None;
        }
        let language = settings_state::get().language;
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width =
            crate::capture_panels::hints_panel_width().min((window_width - 16.0).max(120.0));
        let pin_hotkey = crate::hotkey::display_hotkey(&settings_state::get().pin_hotkey);
        let k = |key: &str| t(language, key).to_owned();
        let mut rows: Vec<(String, String)> = vec![
            (k("editor.hint_key_copy"), k("editor.hint_copy")),
            ("Cmd/Ctrl+S".to_owned(), k("editor.hint_save")),
        ];
        if self.inline.and_then(|inline| inline.capture).is_some() {
            rows.push((pin_hotkey, k("editor.hint_pin")));
        }
        rows.push((
            "Space".to_owned(),
            if self.inline.is_some_and(|inline| inline.zoomable) {
                k("editor.hint_pan")
            } else {
                k("editor.hint_space")
            },
        ));
        if self.inline.and_then(|inline| inline.capture).is_some() {
            rows.push(("F5".to_owned(), k("editor.hint_refresh")));
        }
        rows.push(("Esc".to_owned(), k("editor.hint_cancel")));
        let height = crate::capture_panels::hints_panel_height(rows.len());
        let rect = Bounds {
            origin: point(
                px((window_width - width) / 2.0),
                px((window_height - height) / 2.0),
            ),
            size: size(px(width), px(height)),
        };
        Some(crate::capture_panels::hints_panel(
            "editor",
            self.chrome,
            language,
            rect,
            rows,
            cx,
            Self::dismiss_hints,
        ))
    }
}
