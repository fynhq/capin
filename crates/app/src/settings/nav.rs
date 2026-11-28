use super::*;

impl SettingsView {
    pub(super) fn sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav: Vec<AnyElement> = Vec::new();
        let nav_focus = self.nav_focus.clone();
        for (index, section) in SettingsSection::ALL.into_iter().enumerate() {
            let selected = self.section == section;
            let hover = theme.hover;
            let accent = theme.accent;
            let muted = theme.muted;
            let text = theme.text;
            let selected_bg = theme.selected;
            let item = div()
                .w_full()
                .h(px(layout::NAV_ITEM_HEIGHT))
                .px(px(layout::NAV_ITEM_PAD_X))
                .rounded(px(layout::NAV_ITEM_RADIUS))
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(layout::NAV_GAP))
                .when(selected, |item| item.bg(selected_bg))
                .when(!selected, |item| item.hover(move |style| style.bg(hover)))
                .id(section.id())
                .role(Role::Button)
                .aria_label(section.title(self.settings().language))
                .aria_selected(selected)
                .active(move |style| style.bg(selected_bg))
                .track_focus(&nav_focus[index])
                .focus_visible(move |style| style.shadow(crate::ui::focus_ring(accent)))
                .on_key_down(cx.listener(move |this, event, window, cx| {
                    if super::handle_tab_key(this, event, window, cx) {
                        return;
                    }
                    match event.keystroke.key.as_str() {
                        "up" if index > 0 => {
                            window.focus(&this.nav_focus[index - 1], cx);
                            cx.notify();
                            cx.stop_propagation();
                        }
                        "down" if index + 1 < this.nav_focus.len() => {
                            window.focus(&this.nav_focus[index + 1], cx);
                            cx.notify();
                            cx.stop_propagation();
                        }
                        "enter" | "space" => {
                            super::select_section(this, section, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.focus(&this.nav_focus[index], cx);
                    super::select_section(this, section, cx);
                }))
                .child(
                    div()
                        .id(format!("settings-nav-icon-{}", section.id()))
                        .role(Role::Image)
                        .aria_label(section.title(self.settings().language))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icons::icon(
                            section.icon(selected),
                            layout::NAV_ICON,
                            if selected { accent } else { muted },
                        )),
                )
                .child(
                    div()
                        .text_size(px(layout::NAV_FONT))
                        .text_color(text)
                        .child(section.title(self.settings().language)),
                );
            nav.push(item.into_any_element());
        }

        let selected_index = SettingsSection::ALL
            .iter()
            .position(|candidate| *candidate == self.section)
            .unwrap_or(0) as f32;
        let indicator_top = selected_index * (layout::NAV_ITEM_HEIGHT + 2.0)
            + (layout::NAV_ITEM_HEIGHT - 16.0) / 2.0;
        let indicator = div()
            .absolute()
            .left(px(0.0))
            .top(px(indicator_top))
            .w(px(3.0))
            .h(px(16.0))
            .rounded(px(1.5))
            .bg(theme.accent);
        let nav = div()
            .id("settings-nav")
            .role(Role::Navigation)
            .aria_label(t(self.settings().language, "settings.a11y_sidebar"))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(nav)
            .child(indicator);
        let mut sidebar = div()
            .id("settings-sidebar")
            .role(Role::Complementary)
            .aria_label(t(self.settings().language, "settings.a11y_sidebar"))
            .w(px(layout::SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(px(layout::SIDEBAR_PAD))
            .pb(px(layout::SIDEBAR_PAD));

        #[cfg(target_os = "macos")]
        {
            sidebar = sidebar
                .child(self.macos_drag_area(
                    "settings-sidebar-drag-strip",
                    div().w_full().h(px(layout::TITLEBAR_HEIGHT)),
                    cx,
                ))
                .child(self.macos_drag_area(
                    "settings-sidebar-drag-title",
                    self.sidebar_title(theme),
                    cx,
                ))
                .child(div().w_full().mt(px(12.0)).child(nav));
        }
        #[cfg(not(target_os = "macos"))]
        {
            sidebar = sidebar.pt(px(layout::TITLEBAR_HEIGHT + 8.0)).child(nav);
        }

        sidebar = sidebar
            .child(self.sidebar_footer(theme, cx))
            .bg(theme.sidebar);
        if layout::SIDEBAR_DIVIDER {
            sidebar.border_r_1().border_color(theme.border)
        } else {
            sidebar
        }
    }

    #[cfg(target_os = "macos")]
    fn sidebar_title(&self, theme: &Theme) -> Div {
        div()
            .w_full()
            .h(px(24.0))
            .px(px(layout::NAV_ITEM_PAD_X))
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.text)
                    .child(t(self.settings().language, "settings.title")),
            )
    }

    #[cfg(target_os = "macos")]
    pub(super) fn macos_drag_area(
        &self,
        id: &'static str,
        area: Div,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        area.id(id)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _| {
                    if event.click_count == 2 {
                        this.titlebar_drag = false;
                        window.titlebar_double_click();
                    } else {
                        this.titlebar_drag = true;
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.titlebar_drag = false),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| this.titlebar_drag = false))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.titlebar_drag {
                    this.titlebar_drag = false;
                    window.start_window_move();
                }
            }))
    }

    fn sidebar_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let accent = theme.accent;
        let version = crate::build_info::version_label();
        div()
            .mt_auto()
            .w_full()
            .pt(px(16.0))
            .pb(px(2.0))
            .pl(px(layout::NAV_ITEM_PAD_X))
            .flex()
            .justify_start()
            .child(
                div()
                    .id("settings-version-link")
                    .role(Role::Button)
                    .aria_label(version.clone())
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(accent))
                    .on_click(cx.listener(|this, _, _, cx| this.open_project(cx)))
                    .child(version),
            )
    }
}
