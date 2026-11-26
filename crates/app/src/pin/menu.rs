use super::*;

struct PinMenuView {
    pin_id: u64,
    items: Vec<(PinMenuAction, String)>,
    accent_hex: u32,
    selected: Option<usize>,
    focus_handle: FocusHandle,
    _blur_subscription: Option<gpui_kit::Subscription>,
}

impl Render for PinMenuView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PinTheme::for_window(window, self.accent_hex);
        let pin_id = self.pin_id;
        let actions: Vec<PinMenuAction> = self.items.iter().map(|(action, _)| *action).collect();
        let mut menu = div()
            .size_full()
            .p(px(PIN_MENU_PADDING))
            .rounded(px(8.0))
            .bg(theme.menu)
            .border_1()
            .border_color(theme.menu_border)
            .shadow(vec![
                BoxShadow::new(px(0.0), px(6.0), hsla(0.0, 0.0, 0.0, 0.28)).blur_radius(px(18.0)),
            ])
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        log_info!(
                            "pin",
                            "context menu closed pin={} reason=escape",
                            this.pin_id
                        );
                        forget_pin_menu();
                        window.remove_window();
                        cx.stop_propagation();
                    }
                    "up" => {
                        this.selected = Some(match this.selected {
                            Some(index) => index.saturating_sub(1),
                            None => actions.len().saturating_sub(1),
                        });
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.selected = Some(match this.selected {
                            Some(index) => (index + 1).min(actions.len().saturating_sub(1)),
                            None => 0,
                        });
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "enter" => {
                        let action = this.selected.and_then(|index| actions.get(index).copied());
                        if let Some(action) = action {
                            log_info!(
                                "pin",
                                "context menu closed pin={} reason=keyboard action={}",
                                pin_id,
                                action.id()
                            );
                            forget_pin_menu();
                            window.remove_window();
                            cx.defer(move |cx| perform_pin_action(pin_id, action, cx));
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }));
        for (index, (action, label)) in self.items.iter().enumerate() {
            let action = *action;
            let label = label.clone();
            let selected = self.selected == Some(index);
            if index > 0 && action.separated() {
                menu = menu.child(
                    div()
                        .w_full()
                        .py(px(4.0))
                        .child(div().h(px(1.0)).mx(px(4.0)).bg(theme.menu_border)),
                );
            }
            menu = menu.child(
                div()
                    .id(action.id())
                    .occlude()
                    .h(px(PIN_MENU_ITEM_HEIGHT))
                    .px(px(10.0))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .text_size(px(12.5))
                    .text_color(theme.text)
                    .when(selected, |item| item.bg(theme.menu_hover))
                    .hover(move |style| style.bg(theme.menu_hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(move |_, _, window, cx| {
                        log_info!(
                            "pin",
                            "context menu closed pin={} reason=select action={}",
                            pin_id,
                            action.id()
                        );
                        forget_pin_menu();
                        window.remove_window();
                        cx.defer(move |cx| perform_pin_action(pin_id, action, cx));
                    }))
                    .child(label),
            );
        }
        crate::ui::enter_fade(menu, "pin-menu-enter", Duration::from_millis(90))
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn is_pin_menu_open() -> bool {
    PIN_MENU.with(|slot| slot.borrow().is_some())
}

fn forget_pin_menu() {
    PIN_MENU.with(|slot| {
        slot.borrow_mut().take();
    });
}

pub(crate) fn close_pin_menu(cx: &mut App) {
    let handle = PIN_MENU.with(|slot| slot.borrow_mut().take());
    if let Some(handle) = handle {
        log_info!(
            "pin",
            "context menu closed window={} reason=external",
            handle.window_id().as_u64()
        );
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

pub(crate) fn open_pin_menu(
    pin_id: u64,
    position: Point<Pixels>,
    display_id: Option<DisplayId>,
    items: Vec<(PinMenuAction, String)>,
    cx: &mut App,
) {
    close_pin_menu(cx);
    let width = PIN_MENU_WIDTH;
    let item_count = items.len();
    let separators = items
        .iter()
        .enumerate()
        .filter(|(index, (action, _))| *index > 0 && action.separated())
        .count() as f32;
    let height = PIN_MENU_PADDING * 2.0
        + PIN_MENU_ITEM_HEIGHT * items.len() as f32
        + PIN_MENU_SEPARATOR * separators
        + 2.0;
    let position_x = f32::from(position.x);
    let position_y = f32::from(position.y);
    let display = display_id
        .and_then(|id| cx.displays().into_iter().find(|display| display.id() == id))
        .or_else(|| cx.primary_display());
    let (display_id, area) = match display {
        Some(display) => (Some(display.id()), display.bounds()),
        None => (None, Bounds::default()),
    };
    let left = f32::from(area.origin.x);
    let top = f32::from(area.origin.y);
    let right = left + f32::from(area.size.width);
    let bottom = top + f32::from(area.size.height);
    let origin = point(
        px(position_x.clamp(left, (right - width).max(left))),
        px(position_y.clamp(top, (bottom - height).max(top))),
    );
    let options = crate::aux_window::popup_options(
        "capin-pin-menu",
        Bounds {
            origin,
            size: size(px(width), px(height)),
        },
        display_id,
        true,
        pin_window_kind(),
        WindowBackgroundAppearance::Transparent,
    );
    let accent_hex = system_accent_hex().unwrap_or_else(|| crate::theme::fallback_accent(false));

    match gpui_kit::open_window(options, cx, move |window, cx| {
        let _ = apply_window_style(&*window, WindowStyle::PIN_MENU);
        let view = cx.new(|cx| PinMenuView {
            pin_id,
            items,
            accent_hex,
            selected: None,
            focus_handle: cx.focus_handle(),
            _blur_subscription: None,
        });
        let focus = view.read(cx).focus_handle.clone();
        view.update(cx, |view, cx| {
            view._blur_subscription = Some(cx.on_blur(&focus, window, move |_, window, _| {
                log_info!("pin", "context menu closed pin={} reason=blur", pin_id);
                forget_pin_menu();
                window.remove_window();
            }));
        });
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, _)) => {
            log_info!(
                "pin",
                "context menu opened pin={} window={} items={} at={:.0},{:.0} size={:.0}x{:.0}",
                pin_id,
                handle.window_id().as_u64(),
                item_count,
                f32::from(origin.x),
                f32::from(origin.y),
                width,
                height
            );
            PIN_MENU.with(|slot| *slot.borrow_mut() = Some(handle));
        }
        Err(error) => log_error!("pin", "failed to open context menu pin={pin_id}: {error}"),
    }
}

fn pin_handles(id: u64) -> Option<(Entity<PinView>, AnyWindowHandle)> {
    PINS.with(|pins| {
        pins.borrow()
            .get(&id)
            .map(|state| (state.entity.clone(), state.window))
    })
}

fn perform_pin_action(id: u64, action: PinMenuAction, cx: &mut App) {
    let Some((entity, window_handle)) = pin_handles(id) else {
        log_warn!(
            "pin",
            "menu action dropped pin={id} action={} (pin is gone)",
            action.id()
        );
        return;
    };
    log_info!("pin", "menu action pin={id} action={}", action.id());
    match action {
        PinMenuAction::Rotate => rotate_pin(id, &entity, window_handle, cx),
        PinMenuAction::Edit => {
            entity.update(cx, |view, cx| view.open_editor(cx));
        }
        PinMenuAction::Settings => {
            crate::settings::open_settings(cx);
        }
        PinMenuAction::Through => {
            enable_click_through(id, &entity, window_handle, cx);
        }
        PinMenuAction::Close => {
            close_pin(id, cx);
        }
    }
}

fn rotate_pin(id: u64, entity: &Entity<PinView>, window_handle: AnyWindowHandle, cx: &mut App) {
    let (frame, zoom) = {
        let view = entity.read(cx);
        (view.frame_for_action(), view.values.borrow().zoom)
    };
    let Some(frame) = frame else {
        return;
    };
    let rotated = frame.rotated_cw();
    let hash = crate::capture_history::hash_frame(&rotated);
    let width = rotated.width() as f32 * zoom;
    let height = rotated.height() as f32 * zoom;
    let Ok(origin) = window_handle.update(cx, |_, window, _| window.bounds().origin) else {
        return;
    };
    let bounds = Bounds {
        origin,
        size: size(px(width), px(height)),
    };
    if !persist_rotated_image(id, &rotated, hash, bounds) {
        entity.update(cx, |view, cx| {
            let language = settings_state::get().language;
            view.show_notice(t(language, "pin.rotate_failed").to_owned(), cx);
        });
        return;
    }
    let _ = window_handle.update(cx, |_, window, _| {
        window.resize(size(px(width), px(height)))
    });
    entity.update(cx, |view, cx| {
        view.replace_image(Arc::new(rotated), hash, cx)
    });
}

fn enable_click_through(
    id: u64,
    entity: &Entity<PinView>,
    window_handle: AnyWindowHandle,
    cx: &mut App,
) {
    let applied = window_handle
        .update(cx, |_, window, _| set_click_through(&*window, true).is_ok())
        .unwrap_or(false);
    if !applied {
        log_warn!("pin", "failed to enable click-through pin={id}");
        return;
    }
    log_info!("pin", "click-through enabled pin={id}");
    entity.update(cx, |_, cx| {
        set_state_click_through(id, true);
        cx.notify();
    });
    crate::tray::refresh_menu();
}
