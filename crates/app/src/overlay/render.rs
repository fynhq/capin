use super::*;

struct OverlayScene {
    layers: Vec<AnyElement>,
    handles: Vec<AnyElement>,
    info_bar: Option<AnyElement>,
    toolbar: Option<AnyElement>,
    magnifier: Option<Div>,
    size_tag: Option<Div>,
    window_mode_hint: Option<Div>,
    capture_hint: Option<Div>,
    notice: Option<AnyElement>,
    effects_panel: Option<AnyElement>,
    hints_panel: Option<AnyElement>,
}

impl OverlayView {
    pub(super) fn render_inner(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.chrome = ChromeTheme::for_window(window, Some(self.accent_hex));
        if self.window_only
            && self.cursor.is_some()
            && self.selected.is_none()
            && self.editor.is_none()
        {
            platform_window::set_cursor_hidden(true);
        }
        if let Some(editor) = self.editor.clone() {
            let rect = Some(editor.read(cx).image_view_rect(window));
            let layers = self.dim_layers(rect);
            return div()
                .relative()
                .size_full()
                .child(
                    img(self.image.clone())
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                )
                .children(layers)
                .child(editor)
                .into_any_element();
        }

        let scene = self.render_scene(cx);
        self.render_root(scene, cx)
    }

    fn render_scene(&mut self, cx: &mut Context<Self>) -> OverlayScene {
        let language = settings_state::get().language;
        let selected_bounds = self.selection_bounds();
        let selected = self.selected.is_some();
        let drag_selection = self
            .session
            .selection_rect()
            .and_then(|rect| self.local_rect(rect));
        let hovered_window = self
            .session
            .hovered_window()
            .and_then(|window| self.local_bounds(window.bounds));
        let hovered_element = (!self.window_only)
            .then(|| {
                self.session
                    .hovered_element()
                    .and_then(|element| self.local_bounds(element))
            })
            .flatten();
        let hovered = hovered_element.or(hovered_window);
        let desktop_hover = !selected
            && !self.session.has_drag()
            && self.cursor.is_some()
            && hovered.is_none()
            && self.editing_rect.is_none();
        let screen_bounds = desktop_hover.then(|| Bounds {
            origin: point(px(0.0), px(0.0)),
            size: self.window_size,
        });
        let hole = selected_bounds
            .or(drag_selection)
            .or(hovered)
            .or(screen_bounds)
            .or(self.editing_rect);

        let magnifier = (!selected && !self.session.is_dragging())
            .then(|| panels::magnifier_panel(self, drag_selection, language, cx))
            .flatten();
        let size_tag = panels::selection_size_tag(self, language);
        let window_mode_hint = (!selected && self.window_only).then(|| {
            div()
                .absolute()
                .top(px(16.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(7.0))
                        .rounded(px(self.chrome.radius))
                        .bg(self.chrome.surface)
                        .border_1()
                        .border_color(rgba((ELEMENT_HOVER << 8) | 0xAA))
                        .shadow(self.chrome.shadow())
                        .text_size(px(crate::ui::FONT_TITLE))
                        .text_color(self.chrome.text)
                        .child(t(language, "overlay.window_mode_hint")),
                )
        });
        let capture_hint = (!selected && self.cursor.is_none()).then(|| {
            div()
                .absolute()
                .bottom(px(16.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(11.0))
                        .py(px(5.0))
                        .rounded(px(crate::ui::RADIUS_CONTROL))
                        .bg(self.chrome.surface)
                        .border_1()
                        .border_color(self.chrome.border)
                        .shadow(self.chrome.shadow())
                        .text_size(px(crate::ui::FONT_MICRO))
                        .text_color(self.chrome.muted)
                        .child(t(language, "overlay.capture_hint")),
                )
        });
        let notice_serial = self.notice_serial;
        let notice_leaving = self.notice_leaving;
        let notice = self.notice.clone().map(|message| {
            let duration = if cx.reduce_motion() {
                Duration::ZERO
            } else if notice_leaving {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(120)
            };
            div()
                .absolute()
                .bottom(px(46.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(crate::ui::RADIUS_CONTROL))
                        .bg(self.chrome.surface_strong)
                        .border_1()
                        .border_color(self.chrome.border)
                        .shadow(self.chrome.shadow())
                        .text_size(px(crate::ui::FONT_BODY))
                        .text_color(self.chrome.text)
                        .child(message),
                )
                .with_animation(
                    SharedString::from(format!("overlay-notice-{notice_serial}-{notice_leaving}")),
                    Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |toast, delta| {
                        if notice_leaving {
                            toast.opacity(1.0 - delta)
                        } else {
                            toast.opacity(delta).bottom(px(46.0 - (1.0 - delta) * 4.0))
                        }
                    },
                )
        });

        let screen_color = rgba((SCREEN_SELECTION << 8) | 0xFF);
        let selection_radius = self.selection_effects_radius();
        let drag_radius = if self.selected.is_none() && drag_selection.is_some() {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let radius = selection_radius.max(drag_radius);
        let mut layers = self.dim_layers_rounded(hole, radius);
        if let Some(rect) = selected_bounds {
            if radius <= 0.5 {
                layers.push(border(rect, self.accent, radius).into_any_element());
            }
        } else if let Some(rect) = drag_selection {
            if radius <= 0.5 {
                layers.push(border(rect, self.accent, 0.0).into_any_element());
            }
        } else if let Some(rect) = hovered_element {
            layers.push(element_border(rect, rgba((ELEMENT_HOVER << 8) | 0xFF)).into_any_element());
        } else if let Some(rect) = hovered_window {
            layers.push(border(rect, self.accent, 0.0).into_any_element());
        } else if let Some(rect) = screen_bounds {
            layers.push(screen_border(rect, screen_color).into_any_element());
        }

        if self.selected.is_none()
            && self.window_only
            && let Some(cursor) = self.cursor
        {
            let monitor = self.session.monitor_bounds();
            let (scale_x, scale_y) = self.frame_ratio();
            let x = (cursor.x - monitor.origin.x) as f32 / scale_x;
            let y = (cursor.y - monitor.origin.y) as f32 / scale_y;
            let width = f32::from(self.window_size.width);
            let height = f32::from(self.window_size.height);
            if x >= 0.0 && y >= 0.0 && x <= width && y <= height {
                layers.push(camera_cursor(x, y, self.chrome.dark).into_any_element());
            }
        }

        if !selected
            && let Some(window) = self.session.hovered_window()
            && let Some(rect) = hovered
        {
            layers.push(
                query::window_label(
                    &self.chrome,
                    language,
                    window,
                    rect,
                    hovered_element.and(self.session.hovered_element()),
                )
                .into_any_element(),
            );
        }
        if !selected && desktop_hover {
            let size = self.session.monitor_bounds().size;
            layers.push(query::screen_label(&self.chrome, language, size).into_any_element());
        }

        let handles = if selected {
            panels::selection_handles(self, cx)
        } else {
            Vec::new()
        };
        let info_bar = panels::info_bar(self, cx);
        let toolbar = panels::selection_toolbar(self, cx);
        let effects_panel = panels::effects_panel(self, cx);
        let hints_panel = panels::hints_panel(self, cx);

        OverlayScene {
            layers,
            handles,
            info_bar,
            toolbar,
            magnifier,
            size_tag,
            window_mode_hint,
            capture_hint,
            effects_panel,
            hints_panel,
            notice: notice.map(|notice| notice.into_any_element()),
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let command = event.keystroke.modifiers.platform || event.keystroke.modifiers.control;

        if key == "shift" && !self.shift_held && self.selected.is_none() {
            self.toggle_color_format(cx);
        }
        if key == "shift" {
            self.shift_held = true;
        }

        if let Some(selected) = self.selected.as_mut() {
            if selected.edit.is_some() {
                match key {
                    "escape" => {
                        self.cancel_edit(cx);
                        return;
                    }
                    "enter" => {
                        self.commit_edit(cx);
                        return;
                    }
                    "backspace" | "delete" => {
                        if let Some(selected) = self.selected.as_mut()
                            && let Some(edit) = selected.edit.as_mut()
                        {
                            edit.fresh = false;
                            edit.buffer.pop();
                        }
                        cx.notify();
                        return;
                    }
                    _ => {}
                }
                if let Some(ch) = event.keystroke.key.chars().next()
                    && (ch.is_ascii_digit() || ch == '-')
                {
                    if let Some(selected) = self.selected.as_mut()
                        && let Some(edit) = selected.edit.as_mut()
                    {
                        if edit.fresh {
                            edit.buffer.clear();
                            edit.fresh = false;
                        }
                        edit.buffer.push(ch);
                    }
                    cx.notify();
                    return;
                }
                if !command {
                    return;
                }
            }
            if key == "escape" {
                if selected.hints_open || selected.effects_open {
                    selected.hints_open = false;
                    selected.effects_open = false;
                    hints_window::close(cx);
                    cx.notify();
                    return;
                }
                self.close_overlays(cx);
                return;
            }
            if command && matches!(key, "w" | "q") {
                self.close_overlays(cx);
                return;
            }
            if key == "enter" || (command && key == "c" && !event.keystroke.modifiers.shift) {
                self.copy_selected(cx);
                return;
            }
            if command && key == "s" && !event.keystroke.modifiers.shift {
                self.save_selected(cx);
                return;
            }
            if key == "f5" {
                if !event.is_held {
                    self.refresh_frame(cx);
                }
                return;
            }
            if key == "f1" {
                self.toggle_hints(window, cx);
                return;
            }
            if key.eq_ignore_ascii_case("r") {
                self.cycle_ratio(cx);
                return;
            }
            if key == "space" {
                if let Some(selected) = self.selected.as_mut() {
                    selected.space_down = true;
                    selected.effects_open = false;
                    selected.hints_open = false;
                }
                hints_window::close(cx);
                cx.notify();
                return;
            }
            if key == "delete" || key == "backspace" {
                self.selected = None;
                self.screen_targeting = false;
                self.session.cancel();
                hints_window::close(cx);
                cx.notify();
                return;
            }
            if matches!(key, "left" | "right" | "up" | "down") {
                self.on_arrow_key(key, event.keystroke.modifiers.shift, command, cx);
                return;
            }
            return;
        }

        if key == "escape" || (command && matches!(key, "w" | "q")) {
            self.screen_targeting = false;
            self.session.cancel();
            self.close_overlays(cx);
            return;
        }
        if key.eq_ignore_ascii_case("r") {
            self.cycle_aspect(cx);
        } else if key.eq_ignore_ascii_case("c") {
            self.copy_color(cx);
        } else if key == "f5" && !event.is_held {
            self.refresh_frame(cx);
        }
        let _ = window;
    }

    fn render_root(&mut self, scene: OverlayScene, cx: &mut Context<Self>) -> AnyElement {
        div()
            .relative()
            .size_full()
            .cursor(self.cursor_style)
            .track_focus(&self.focus_handle)
            .key_context(OVERLAY_KEY_CONTEXT)
            .on_action(cx.listener(Self::on_widen_target))
            .on_action(cx.listener(Self::on_narrow_target))
            .on_modifiers_changed(cx.listener(
                |this, event: &ModifiersChangedEvent, _window, cx| {
                    let shift = event.modifiers.shift;
                    if shift && !this.shift_held && this.selected.is_none() {
                        this.toggle_color_format(cx);
                    }
                    this.shift_held = shift;

                    this.set_window_only(control_down(&event.modifiers), cx);
                    cx.notify();
                },
            ))
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                if event.keystroke.key == "shift" {
                    this.shift_held = false;
                    cx.notify();
                }
                if event.keystroke.key == "space"
                    && let Some(selected) = this.selected.as_mut()
                {
                    selected.space_down = false;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let point = this.to_physical(event.position);
                if this.cursor != Some(point) {
                    this.element_probe.scroll.reset();
                }
                this.cursor = Some(point);
                this.set_window_only(control_down(&event.modifiers), cx);
                if this.window_only && this.selected.is_none() {
                    platform_window::set_cursor_hidden(true);
                }
                if this
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.drag.is_some())
                {
                    this.cursor_style = crate::ui::closed_hand_cursor();
                    if event.pressed_button.is_none() {
                        this.finish_region_drag(cx);
                    } else {
                        this.update_region_drag(point, cx);
                    }
                    return;
                }
                if let Some(selected) = this.selected.as_ref() {
                    let monitor = this.session.monitor_bounds();
                    let local =
                        PhysicalPoint::new(point.x - monitor.origin.x, point.y - monitor.origin.y);
                    let edge_tolerance = 8.0 * this.effective_scale();
                    let inside = local.x >= selected.rect.origin.x
                        && local.y >= selected.rect.origin.y
                        && local.x <= selected.rect.origin.x + selected.rect.size.width as i32
                        && local.y <= selected.rect.origin.y + selected.rect.size.height as i32;
                    this.cursor_style = if let Some(handle) =
                        edge_resize_handle(selected.rect, local, edge_tolerance)
                    {
                        region_handle_cursor(handle)
                    } else if inside {
                        crate::ui::open_hand_cursor()
                    } else {
                        CursorStyle::Crosshair
                    };
                    cx.notify();
                    return;
                }
                this.cursor_style = CursorStyle::Crosshair;
                if this.session.has_drag() && event.pressed_button.is_none() {
                    this.screen_targeting = false;
                    this.session.cancel();
                } else {
                    this.session.pointer_moved(point);
                    if !this.session.has_drag() {
                        this.refresh_auto_target(point, cx);
                    }
                }
                this.update_magnifier(window);
                cx.notify();
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _window, cx| {
                if this.selected.is_some() {
                    return;
                }
                let Some(direction) =
                    this.element_probe
                        .scroll
                        .step(Instant::now(), event.delta, event.touch_phase)
                else {
                    return;
                };
                this.adjust_auto_target(direction);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if cfg!(target_os = "macos")
                        && this.selected.is_none()
                        && this.editor.is_none()
                        && (this.window_only || control_down(&event.modifiers))
                    {
                        this.ctrl_click_primary = true;
                        this.primary_pointer_down(event, true, cx);
                        cx.stop_propagation();
                        return;
                    }
                    if this
                        .selected
                        .as_ref()
                        .is_some_and(|selected| selected.edit.is_some())
                    {
                        this.commit_edit(cx);
                        return;
                    }
                    if let Some(selected) = this.selected.as_mut() {
                        selected.hints_open = false;
                        selected.effects_open = false;
                    }
                    hints_window::close(cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.ctrl_click_primary {
                        this.ctrl_click_primary = false;
                        this.primary_pointer_up(event, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if this.ctrl_click_primary {
                        return;
                    }
                    this.primary_pointer_down(event, false, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.ctrl_click_primary {
                        this.ctrl_click_primary = false;
                        this.primary_pointer_up(event, window, cx);
                        return;
                    }
                    this.primary_pointer_up(event, window, cx);
                }),
            )
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, window, cx| {
                if this
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.drag.is_some())
                {
                    this.finish_region_drag(cx);
                }
                this.cursor = None;
                this.clear_magnifier(window);
                this.cursor_style = CursorStyle::Crosshair;
                this.ctrl_click_primary = false;
                this.element_probe.clear_hover();
                this.screen_targeting = false;
                this.session.pointer_left();
                cx.notify();
            }))
            .child(
                img(self.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .children(scene.layers)
            .children(scene.handles)
            .children(scene.info_bar)
            .children(scene.toolbar)
            .children(scene.magnifier)
            .children(scene.size_tag)
            .children(scene.window_mode_hint)
            .children(scene.capture_hint)
            .children(scene.notice)
            .children(scene.effects_panel)
            .children(scene.hints_panel)
            .into_any_element()
    }
}
