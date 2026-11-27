use crate::i18n::{Language, t};
use crate::logging::log_debug;
use crate::settings_state::{self, PinDoubleClickAction};

use super::*;

impl PinView {
    pub(super) fn open_editor(&mut self, cx: &mut Context<Self>) {
        let edit_state = PINS.with(|pins| {
            pins.borrow()
                .get(&self.id)
                .and_then(|state| state.edit_state.clone())
        });
        let (frame, edit_state) = match edit_state {
            Some(state) => match state.base_frame() {
                Some(base) => (Some(base), Some(state)),
                None => (self.frame_for_action(), None),
            },
            None => (self.frame_for_action(), None),
        };
        let Some(frame) = frame else {
            return;
        };
        let bounds = PINS.with(|pins| pins.borrow().get(&self.id).map(|state| state.bounds));
        let Some(bounds) = bounds else {
            return;
        };
        if !crate::editor::open_pin_editor_overlay(
            self.id,
            frame,
            bounds,
            self.display,
            edit_state,
            cx,
        ) {
            let language = settings_state::get().language;
            self.show_notice(t(language, "pin.edit_failed").to_owned(), cx);
        }
    }

    pub(super) fn menu_items(&self, language: Language) -> Vec<(PinMenuAction, String)> {
        let mut items = Vec::new();
        if matches!(&self.content, PinContent::Image { .. }) {
            items.push(PinMenuAction::Rotate);
            items.push(PinMenuAction::Edit);
        }
        items.push(PinMenuAction::Settings);
        items.push(PinMenuAction::Through);
        items.push(PinMenuAction::Close);
        items
            .into_iter()
            .map(|action| (action, t(language, action.label_key()).to_owned()))
            .collect()
    }

    pub(super) fn body(&self, theme: &PinTheme, viewport: Size<Pixels>) -> Div {
        match &self.content {
            PinContent::Image { image, .. } => {
                let (width, height) = self.content.size(self.values.borrow().zoom);
                let viewport_width = f32::from(viewport.width);
                let viewport_height = f32::from(viewport.height);
                let fit = if viewport_width <= 0.0 || viewport_height <= 0.0 {
                    1.0
                } else {
                    (viewport_width / width.max(1.0))
                        .min(viewport_height / height.max(1.0))
                        .min(1.0)
                };
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(width * fit))
                            .h(px(height * fit))
                            .flex_shrink_0()
                            .child(img(image.clone()).size_full().object_fit(ObjectFit::Fill)),
                    )
            }
            PinContent::Text(text) => div()
                .size_full()
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(theme.card)
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .text_size(px(14.0))
                .text_color(theme.text)
                .child(text.clone()),
        }
    }
}

impl Render for PinView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PinTheme::for_window(window, self.accent_hex);
        let notice = self.toasts[NOTICE_TOAST].message.clone();
        let notice_serial = self.toasts[NOTICE_TOAST].serial;
        let notice_leaving = self.toasts[NOTICE_TOAST].leaving;
        let hud = self.toasts[HUD_TOAST].message.clone();
        let hud_serial = self.toasts[HUD_TOAST].serial;
        let hud_leaving = self.toasts[HUD_TOAST].leaving;
        let body = self
            .body(&theme, window.viewport_size())
            .opacity(self.values.borrow().opacity);
        let zoom = self.values.borrow().zoom;
        let text_layers: Vec<Div> = self
            .selection_rects()
            .into_iter()
            .map(|rect| {
                div()
                    .absolute()
                    .left(px(rect.origin.x as f32 * zoom))
                    .top(px(rect.origin.y as f32 * zoom))
                    .w(px(rect.size.width as f32 * zoom))
                    .h(px(rect.size.height as f32 * zoom))
                    .bg(theme.selection)
            })
            .collect();
        let flash = self.flash_active.then(|| {
            let outline = div()
                .absolute()
                .inset_0()
                .border_4()
                .border_color(theme.accent);
            if cx.reduce_motion() {
                outline.opacity(self.flash_alpha).into_any_element()
            } else {
                outline
                    .with_animation(
                        SharedString::from(format!("pin-flash-{}", self.flash_generation)),
                        Animation::new(Duration::from_millis(520)).with_easing(|delta| {
                            let pulse = (delta * std::f32::consts::TAU * 2.0).sin().abs();
                            (pulse * (1.0 - delta) * 1.15).min(1.0)
                        }),
                        |outline, delta| outline.opacity(delta),
                    )
                    .into_any_element()
            }
        });
        let content = div()
            .absolute()
            .inset_0()
            .child(body)
            .children(text_layers)
            .children(flash);
        let qr_overlay = self
            .qr()
            .filter(|_| {
                self.hover
                    && self.hover_qr
                    && !self.values.borrow().click_through
                    && self.drag.is_none()
            })
            .and_then(|qr| {
                let (left, top, width, height) = self.qr_display_rect(window.viewport_size())?;
                let language = settings_state::get().language;
                let url = pin_qr::qr_url(&qr.content);
                let accent = theme.accent;
                let dim = width.min(height).max(1.0);
                let radius = (dim * 0.08).clamp(3.0, 12.0);
                let open_size = (dim * 0.24).clamp(30.0, 44.0);
                let copy_size = (dim * 0.16).clamp(20.0, 26.0);
                let copy_inset = (copy_size * 0.25).clamp(4.0, 6.0);
                let mut overlay = div()
                    .id("pin-qr-overlay")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width.max(1.0)))
                    .h(px(height.max(1.0)))
                    .rounded(px(radius))
                    .overflow_hidden()
                    .bg(hsla(0.0, 0.0, 0.0, 0.40))
                    .cursor(CursorStyle::Arrow);
                if url.is_some() {
                    let open_tooltip = t(language, "pin.qr_open").to_owned();
                    overlay = overlay.child(
                        div()
                            .id("pin-qr-open")
                            .absolute()
                            .left(px((width - open_size) / 2.0))
                            .top(px((height - open_size) / 2.0))
                            .w(px(open_size))
                            .h(px(open_size))
                            .rounded_full()
                            .bg(accent)
                            .cursor_pointer()
                            .hover(move |style| style.opacity(0.85))
                            .tooltip(move |_, cx| crate::ui::tooltip_view(open_tooltip.clone(), cx))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    window.focus(&this.focus_handle, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let Some(code) = this.qr_code().map(str::to_owned) else {
                                    return;
                                };
                                if let Some(url) = pin_qr::qr_url(&code) {
                                    cx.open_url(&url);
                                }
                                cx.stop_propagation();
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icons::icon(
                                icons::ARROW_RIGHT,
                                open_size * 0.5,
                                rgb(0xFFFFFF),
                            )),
                    );
                }
                let copy_tooltip = t(language, "pin.qr_copy").to_owned();
                Some(
                    overlay
                        .child(
                            div()
                                .id("pin-qr-copy")
                                .absolute()
                                .top(px(copy_inset))
                                .right(px(copy_inset))
                                .w(px(copy_size))
                                .h(px(copy_size))
                                .rounded(px(copy_size * 0.3))
                                .bg(rgba(0xFFFFFFF2))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgb(0xFFFFFF)))
                                .tooltip(move |_, cx| {
                                    crate::ui::tooltip_view(copy_tooltip.clone(), cx)
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        window.focus(&this.focus_handle, cx);
                                        cx.stop_propagation();
                                    }),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.copy_qr(cx);
                                    cx.stop_propagation();
                                }))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icons::icon(icons::COPY, copy_size * 0.55, rgb(0x1F2328))),
                        )
                        .into_any_element(),
                )
            });
        let cursor = match &self.content {
            PinContent::Image { .. } if self.drag.is_some() => crate::ui::closed_hand_cursor(),
            PinContent::Image { .. } if self.hover_text => CursorStyle::IBeam,
            PinContent::Image { .. } => crate::ui::open_hand_cursor(),
            PinContent::Text(_) => CursorStyle::IBeam,
        };
        let root = div()
            .size_full()
            .id("pin-window")
            .cursor(cursor)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let command =
                    event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
                if command && key == "c" {
                    if this
                        .selection_range()
                        .is_some_and(|(start, end)| end > start)
                    {
                        this.copy_selected_text(cx);
                    } else if matches!(&this.content, PinContent::Image { .. }) {
                        this.copy_image(cx);
                    } else {
                        this.copy_content(cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                if command && key == "v" {
                    cx.defer(pin_clipboard::paste_clipboard);
                    cx.stop_propagation();
                    return;
                }
                if command && key == "s" {
                    if matches!(&this.content, PinContent::Image { .. }) {
                        this.save_image(cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                if command && key == "a" && this.select_all_text(cx) {
                    cx.stop_propagation();
                    return;
                }
                if command
                    && matches!(key, "=" | "+" | "-" | "0")
                    && this.content.image_size().is_some()
                {
                    let scale = window.scale_factor().max(0.1);
                    let target = if key == "0" {
                        1.0 / scale
                    } else if key == "-" {
                        this.zoom_target - PIN_ZOOM_STEP / scale
                    } else {
                        this.zoom_target + PIN_ZOOM_STEP / scale
                    };
                    this.zoom_toward(target, window, cx);
                    cx.stop_propagation();
                    return;
                }
                if key == "f1" {
                    let bounds = window.bounds();
                    let image = matches!(&this.content, PinContent::Image { .. });
                    let display = this.display;
                    cx.defer(move |cx| pin_help::toggle(bounds, display, image, cx));
                    cx.stop_propagation();
                    return;
                }
                if key == "escape" {
                    if this.values.borrow().click_through {
                        let id = this.id;
                        let window_handle =
                            PINS.with(|pins| pins.borrow().get(&id).map(|state| state.window));
                        if let Some(handle) = window_handle {
                            let _ = handle.update(cx, |_, window, _| {
                                let _ = set_click_through(&*window, false);
                            });
                        }
                        set_state_click_through(id, false);
                        log_info!("pin", "click-through released pin={id} reason=escape");
                        crate::tray::refresh_menu();
                        cx.notify();
                    } else {
                        let id = this.id;
                        cx.defer(move |cx| crate::pin::close_pin(id, cx));
                    }
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    log_debug!(
                        "pin",
                        "pin left down pin={} at={:.0},{:.0}",
                        this.id,
                        f32::from(event.position.x),
                        f32::from(event.position.y)
                    );
                    crate::pin::close_pin_menu(cx);
                    window.focus(&this.focus_handle, cx);
                    this.wake_ocr(cx);
                    this.hover_text = this.text_index_at(event.position).is_some();
                    if event.click_count >= 2 {
                        if settings_state::get().pin_double_click == PinDoubleClickAction::Close {
                            let id = this.id;
                            cx.defer(move |cx| crate::pin::close_pin(id, cx));
                        }
                        return;
                    }
                    let bounds = window.bounds().size;
                    let x = f32::from(event.position.x);
                    let y = f32::from(event.position.y);
                    let width = f32::from(bounds.width);
                    let height = f32::from(bounds.height);
                    let margin = 6.0;
                    let near_edge =
                        x < margin || y < margin || x > width - margin || y > height - margin;
                    let over_text = this.begin_text_selection(event.position);
                    if near_edge || event.modifiers.alt || !over_text {
                        this.selection = None;
                        this.selecting = false;
                        this.begin_window_drag(window);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let bounds = window.bounds();
                    let position = point(
                        px(f32::from(bounds.origin.x) + f32::from(event.position.x)),
                        px(f32::from(bounds.origin.y) + f32::from(event.position.y)),
                    );
                    let items = this.menu_items(settings_state::get().language);
                    let pin_id = this.id;
                    let display = this.display;
                    log_info!(
                        "pin",
                        "context menu requested pin={} items={} at={:.0},{:.0}",
                        pin_id,
                        items.len(),
                        f32::from(position.x),
                        f32::from(position.y)
                    );
                    cx.defer(move |cx| {
                        crate::pin::open_pin_menu(pin_id, position, display, items, cx)
                    });
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.drag.is_some() {
                    this.update_window_drag(window);
                    return;
                }
                if this.selecting {
                    this.update_text_selection(event.position);
                    cx.notify();
                    return;
                }
                let over_text = this.text_index_at(event.position).is_some();
                let over_qr = this.qr_hit(event.position, window.viewport_size());
                let mut changed = false;
                if !this.hover {
                    this.hover = true;
                    this.wake_ocr(cx);
                    changed = true;
                }
                if over_text != this.hover_text {
                    this.hover_text = over_text;
                    changed = true;
                }
                if over_qr != this.hover_qr {
                    this.hover_qr = over_qr;
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    let display = window.display(cx).map(|display| display.id());
                    this.display = display;
                    let over_qr = this.qr_hit(event.position, window.viewport_size());
                    let qr_changed = over_qr != this.hover_qr;
                    this.hover_qr = over_qr;
                    if this.drag.take().is_some() {
                        update_bounds(this.id, window.bounds(), display);
                        cx.notify();
                        return;
                    }
                    if this.selecting {
                        this.finish_text_selection(cx);
                    } else {
                        update_bounds(this.id, window.bounds(), display);
                    }
                    if qr_changed {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_exit(cx.listener(|this, _, window, cx| {
                let mut changed = false;
                if this.drag.take().is_some() {
                    let display = window.display(cx).map(|display| display.id());
                    this.display = display;
                    update_bounds(this.id, window.bounds(), display);
                    changed = true;
                }
                if this.selecting {
                    this.finish_text_selection(cx);
                }
                if this.hover_text {
                    this.hover_text = false;
                    changed = true;
                }
                if this.hover_qr {
                    this.hover_qr = false;
                    changed = true;
                }
                if this.hover {
                    this.hover = false;
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let lines = matches!(event.delta, ScrollDelta::Lines(_));
                let (delta_x, delta_y) = match event.delta {
                    ScrollDelta::Lines(delta) => (delta.x, delta.y),
                    ScrollDelta::Pixels(delta) => {
                        (f32::from(delta.x) / 20.0, f32::from(delta.y) / 20.0)
                    }
                };
                let primary = if delta_y.abs() > f32::EPSILON {
                    delta_y
                } else {
                    delta_x
                };
                if event.modifiers.shift || this.content.image_size().is_none() {
                    let step = if lines {
                        primary.signum() * 0.05
                    } else {
                        primary * 0.05
                    };
                    if step != 0.0 {
                        let opacity = this.values.borrow().opacity;
                        this.set_opacity(opacity + step, cx);
                    }
                } else {
                    let step = if lines {
                        delta_y.signum() * PIN_ZOOM_STEP
                    } else {
                        delta_y * PIN_ZOOM_STEP
                    };
                    if step != 0.0 {
                        let scale = window.scale_factor().max(0.1);
                        this.zoom_toward(this.zoom_target + step / scale, window, cx);
                    }
                }
            }))
            .child(content)
            .children(qr_overlay);

        root.children(hud.map(|message| {
            let duration = if hud_leaving {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(120)
            };
            div()
                .absolute()
                .top(px(6.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(5.0))
                        .bg(theme.menu)
                        .border_1()
                        .border_color(theme.menu_border)
                        .text_size(px(10.5))
                        .text_color(theme.text)
                        .child(message),
                )
                .with_animation(
                    SharedString::from(format!("pin-hud-{hud_serial}-{hud_leaving}")),
                    Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |hud, delta| {
                        if hud_leaving {
                            hud.opacity(1.0 - delta)
                        } else {
                            hud.opacity(delta).top(px(6.0 - (1.0 - delta) * 3.0))
                        }
                    },
                )
        }))
        .children(notice.map(|notice| {
            let duration = if notice_leaving {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(120)
            };
            div()
                .absolute()
                .bottom(px(8.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .bg(rgba(0x000000CC))
                        .text_size(px(10.0))
                        .text_color(rgb(0xFFFFFF))
                        .max_w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(notice),
                )
                .with_animation(
                    SharedString::from(format!("pin-notice-{notice_serial}-{notice_leaving}")),
                    Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |notice, delta| {
                        if notice_leaving {
                            notice.opacity(1.0 - delta)
                        } else {
                            notice.opacity(delta).bottom(px(8.0 - (1.0 - delta) * 4.0))
                        }
                    },
                )
        }))
    }
}
