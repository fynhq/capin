use super::*;

pub(super) fn magnifier_panel(
    view: &OverlayView,
    selection: Option<Bounds<Pixels>>,
    language: Language,
    cx: &mut Context<OverlayView>,
) -> Option<Div> {
    let magnifier = view.magnifier.as_ref()?;
    let point = view.cursor?;
    let monitor = view.session.monitor_bounds();
    let (scale_x, scale_y) = view.frame_ratio();
    let local_x = (point.x - monitor.origin.x) as f32 / scale_x;
    let local_y = (point.y - monitor.origin.y) as f32 / scale_y;
    let window_width = f32::from(view.window_size.width);
    let window_height = f32::from(view.window_size.height);
    let mut left = local_x + 18.0;
    if left + MAGNIFIER_PANEL > window_width {
        left = local_x - MAGNIFIER_PANEL - 18.0;
    }
    let mut top = local_y + 18.0;
    if top + MAGNIFIER_PANEL_HEIGHT > window_height {
        top = local_y - MAGNIFIER_PANEL_HEIGHT - 18.0;
    }
    left = left.clamp(0.0, (window_width - MAGNIFIER_PANEL).max(0.0));
    top = top.clamp(0.0, (window_height - MAGNIFIER_PANEL_HEIGHT).max(0.0));

    let color = view.sample_color(point);
    let step = MAGNIFIER_SIZE as f32 / MAGNIFIER_SOURCE as f32;
    let mut lines: Vec<Div> = Vec::new();
    for index in 1..MAGNIFIER_SOURCE {
        let offset = step * index as f32;
        lines.push(
            div()
                .absolute()
                .left(px(offset))
                .top(px(0.0))
                .w(px(1.0))
                .h(px(MAGNIFIER_SIZE as f32))
                .bg(view.chrome.grid),
        );
        lines.push(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(offset))
                .w(px(MAGNIFIER_SIZE as f32))
                .h(px(1.0))
                .bg(view.chrome.grid),
        );
    }
    let centre = step * (MAGNIFIER_SOURCE as f32 / 2.0).floor();
    lines.push(
        div()
            .absolute()
            .left(px(centre))
            .top(px(centre))
            .w(px(step))
            .h(px(step))
            .border_1()
            .border_color(view.chrome.grid_strong),
    );

    let color_row = color.map(|(red, green, blue)| {
        let text = match view.color_format {
            ColorFormat::Hex => format!("#{red:02x}{green:02x}{blue:02x}"),
            ColorFormat::Rgb => format!("{red}, {green}, {blue}"),
        };
        let format_label = match view.color_format {
            ColorFormat::Hex => "HEX",
            ColorFormat::Rgb => "RGB",
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .w(px(12.0))
                    .h(px(12.0))
                    .rounded(px(3.0))
                    .bg(rgb((u32::from(red) << 16)
                        | (u32::from(green) << 8)
                        | u32::from(blue)))
                    .border_1()
                    .border_color(view.chrome.border),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(view.chrome.text)
                    .child(text),
            )
            .child(
                div()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded(px(4.0))
                    .bg(view.chrome.surface)
                    .border_1()
                    .border_color(view.chrome.border)
                    .text_size(px(9.0))
                    .text_color(view.chrome.muted)
                    .child(format_label),
            )
    });

    let size_row = selection.map(|selection| {
        let (scale_x, scale_y) = view.frame_ratio();
        let width = (f32::from(selection.size.width) * scale_x).round() as u32;
        let height = (f32::from(selection.size.height) * scale_y).round() as u32;
        div()
            .text_size(px(10.0))
            .text_color(view.chrome.muted)
            .child(format!("{width} x {height}"))
    });

    Some(
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(MAGNIFIER_PANEL))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(view.chrome.radius))
            .bg(view.chrome.surface_strong)
            .border_1()
            .border_color(view.chrome.border)
            .shadow(view.chrome.shadow())
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .relative()
                    .w(px(MAGNIFIER_SIZE as f32))
                    .h(px(MAGNIFIER_SIZE as f32))
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(view.chrome.border)
                    .child(
                        img(magnifier.image.clone())
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(ObjectFit::Fill),
                    )
                    .children(lines),
            )
            .children(color_row)
            .children(size_row)
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(view.chrome.muted)
                    .child(t(language, "overlay.magnifier_hint")),
            ),
    )
}

pub(super) fn info_bar(view: &OverlayView, cx: &mut Context<OverlayView>) -> Option<AnyElement> {
    let selected = view.selected.as_ref()?;
    let rect = view.info_rect()?;
    let language = settings_state::get().language;
    let mut bar = div()
        .id("overlay-info-bar")
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
        .gap(px(4.0))
        .px(px(7.0))
        .rounded(px(view.chrome.radius))
        .bg(view.chrome.surface_strong)
        .border_1()
        .border_color(view.chrome.border)
        .shadow(view.chrome.shadow())
        .cursor(CursorStyle::Arrow)
        .text_size(px(11.0));

    for field in [
        InfoField::X,
        InfoField::Y,
        InfoField::Width,
        InfoField::Height,
    ] {
        let label = match field {
            InfoField::X => "X",
            InfoField::Y => "Y",
            InfoField::Width => "W",
            InfoField::Height => "H",
        };
        let editing = selected
            .edit
            .as_ref()
            .is_some_and(|edit| edit.field == field);
        let value = if let Some(edit) = selected.edit.as_ref().filter(|_| editing) {
            edit.buffer.clone()
        } else {
            view.info_field_display(selected, field)
        };
        let mut value_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(1.0))
            .child(div().text_color(view.chrome.text).child(value));
        if editing {
            value_row = value_row.child(
                div()
                    .w(px(1.0))
                    .h(px(12.0))
                    .bg(view.chrome.accent)
                    .with_animation(
                        SharedString::from(format!("overlay-info-caret-{label}")),
                        crate::ui::caret_blink(),
                        |caret, delta| caret.opacity(delta),
                    ),
            );
        }
        bar = bar.child(
            div()
                .id(format!("overlay-info-{label}"))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(3.0))
                .px(px(4.0))
                .py(px(3.0))
                .rounded(px(5.0))
                .when(editing, |chip| chip.bg(view.chrome.selected))
                .when(!editing, |chip| {
                    chip.hover(|style| style.bg(view.chrome.hover))
                })
                .active(|style| style.opacity(0.85))
                .cursor_text()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.start_edit(field, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(div().text_color(view.chrome.muted).child(label))
                .child(value_row),
        );
    }

    let unit_label = selected.unit.label();
    let ratio_label = selected.ratio.label(language);
    bar = bar
        .child(
            div()
                .id("overlay-info-unit")
                .px(px(5.0))
                .py(px(3.0))
                .rounded(px(5.0))
                .text_color(view.chrome.muted)
                .hover(|style| style.bg(view.chrome.hover))
                .active(|style| style.opacity(0.85))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(selected) = this.selected.as_mut() {
                        selected.unit = selected.unit.next();
                        selected.edit = None;
                    }
                    cx.notify();
                }))
                .child(unit_label),
        )
        .child(div().w(px(1.0)).h(px(14.0)).bg(view.chrome.separator))
        .child(
            div()
                .id("overlay-info-ratio")
                .px(px(6.0))
                .py(px(3.0))
                .rounded(px(5.0))
                .text_color(view.chrome.text)
                .hover(|style| style.bg(view.chrome.hover))
                .active(|style| style.opacity(0.85))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.cycle_ratio(cx)))
                .child(ratio_label),
        );

    if !matches!(selected.target, CaptureTarget::Window(_)) {
        bar = bar.child(crate::capture_panels::effects_chip(
            "overlay-info-effects",
            view.chrome,
            language,
            selected.effects_open,
            view.effects.active(),
            cx,
            OverlayView::toggle_effects_panel,
        ));
    }

    bar = bar.child(crate::capture_panels::refresh_button(
        "overlay-info-refresh",
        view.chrome,
        language,
        cx,
        OverlayView::refresh_frame,
    ));

    Some(
        crate::ui::enter_slide_fade(
            bar,
            "overlay-info-enter",
            f32::from(rect.origin.y),
            4.0,
            Duration::from_millis(90),
        )
        .into_any_element(),
    )
}

pub(super) fn effects_panel(
    view: &OverlayView,
    cx: &mut Context<OverlayView>,
) -> Option<AnyElement> {
    let selected = view.selected.as_ref()?;
    if !selected.effects_open {
        return None;
    }
    let info = view.info_rect()?;
    let language = settings_state::get().language;
    let width = crate::capture_panels::effects_panel_width(language)
        .min((f32::from(view.window_size.width) - 16.0).max(120.0));
    let height = 40.0;
    let left = f32::from(info.origin.x).clamp(
        8.0,
        (f32::from(view.window_size.width) - width - 8.0).max(8.0),
    );
    let top = crate::ui::popover_top(
        info,
        height,
        f32::from(view.window_size.height),
        view.toolbar_rect(),
    );
    let rect = Bounds {
        origin: point(px(left), px(top)),
        size: size(px(width), px(height)),
    };
    let offset = if top >= f32::from(info.origin.y) {
        5.0
    } else {
        -5.0
    };
    Some(crate::capture_panels::effects_panel(
        "overlay",
        view.chrome,
        language,
        rect,
        view.effects,
        offset,
        cx,
        OverlayView::set_region_radius,
        OverlayView::toggle_region_shadow,
        OverlayView::dismiss_effects,
    ))
}

fn action_button(
    view: &OverlayView,
    id: &'static str,
    label: String,
    icon: &'static [u8],
    color: Rgba,
    cx: &mut Context<OverlayView>,
    action: fn(&mut OverlayView, &mut Context<OverlayView>),
) -> AnyElement {
    div()
        .id(id)
        .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
        .w(px(30.0))
        .h(px(30.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(view.chrome.hover))
        .active(|style| style.opacity(0.85))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
        .child(icons::icon(icon, 17.0, color))
        .into_any_element()
}

pub(super) fn selection_toolbar(
    view: &OverlayView,
    cx: &mut Context<OverlayView>,
) -> Option<AnyElement> {
    let selected = view.selected.as_ref()?;
    if selected.space_down {
        return None;
    }
    let rect = view.toolbar_rect()?;
    let language = settings_state::get().language;
    let mut tools = div().flex().flex_row().items_center().gap(px(1.0));
    for tool in Tool::ALL {
        let label = t(language, tool.label_key()).to_owned();
        tools = tools.child(
            div()
                .id(format!("overlay-tool-{}", tool.id()))
                .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                .w(px(30.0))
                .h(px(30.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|style| style.bg(view.chrome.hover))
                .active(|style| style.opacity(0.85))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_editor_for_selected(tool, window, cx);
                }))
                .child(icons::icon(tool.icon(), 17.0, view.chrome.icon)),
        );
    }

    let mut actions = div().flex().flex_row().items_center().gap(px(1.0));
    actions = actions.child(action_button(
        view,
        "overlay-action-pin",
        t(language, "editor.pin").to_owned(),
        icons::PIN,
        view.chrome.icon,
        cx,
        OverlayView::pin_selected,
    ));
    actions = actions.child(action_button(
        view,
        "overlay-action-copy",
        t(language, "editor.copy").to_owned(),
        icons::COPY,
        view.chrome.icon,
        cx,
        OverlayView::copy_selected,
    ));
    actions = actions.child(action_button(
        view,
        "overlay-action-save",
        t(language, "editor.save").to_owned(),
        icons::SAVE,
        view.chrome.icon,
        cx,
        OverlayView::save_selected,
    ));
    if crate::scroll::supports_target(selected.target) {
        actions = actions.child(action_button(
            view,
            "overlay-action-scroll",
            t(language, "menu.scroll_capture").to_owned(),
            icons::SCROLL,
            view.chrome.icon,
            cx,
            OverlayView::scroll_selected,
        ));
    }
    actions = actions.child(action_button(
        view,
        "overlay-action-record",
        t(language, "menu.record").to_owned(),
        icons::RECORDING,
        view.chrome.icon,
        cx,
        OverlayView::record_selected,
    ));
    actions = actions.child(action_button(
        view,
        "overlay-action-close",
        t(language, "editor.close").to_owned(),
        icons::CLOSE,
        view.chrome.muted,
        cx,
        OverlayView::close_overlays,
    ));

    Some(
        crate::ui::enter_slide_fade(
            div()
                .id("overlay-toolbar")
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
                .justify_between()
                .px(px(8.0))
                .rounded(px(view.chrome.radius))
                .bg(view.chrome.surface)
                .border_1()
                .border_color(view.chrome.border)
                .cursor(CursorStyle::Arrow)
                .shadow(view.chrome.shadow())
                .child(tools)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(1.0))
                        .child(actions),
                ),
            "overlay-toolbar-enter",
            f32::from(rect.origin.y),
            4.0,
            Duration::from_millis(90),
        )
        .into_any_element(),
    )
}

pub(super) fn hints_panel(view: &OverlayView, cx: &mut Context<OverlayView>) -> Option<AnyElement> {
    let selected = view.selected.as_ref()?;
    if !selected.hints_open {
        return None;
    }
    let language = settings_state::get().language;
    let width = crate::capture_panels::hints_panel_width()
        .min((f32::from(view.window_size.width) - 16.0).max(120.0));
    let pin_hotkey = crate::hotkey::display_hotkey(&settings_state::get().pin_hotkey);
    let k = |key: &str| t(language, key).to_owned();
    let rows = vec![
        (k("editor.hint_key_copy"), k("editor.hint_copy")),
        ("Cmd/Ctrl+S".to_owned(), k("editor.hint_save")),
        (pin_hotkey, k("editor.hint_pin")),
        ("Space".to_owned(), k("editor.hint_space")),
        ("R / F5".to_owned(), k("editor.hint_ratio_refresh")),
        ("Esc".to_owned(), k("editor.hint_cancel")),
    ];
    let height = crate::capture_panels::hints_panel_height(rows.len());
    let left = (f32::from(view.window_size.width) - width) / 2.0;
    let top = (f32::from(view.window_size.height) - height) / 2.0;
    let rect = Bounds {
        origin: point(px(left), px(top)),
        size: size(px(width), px(height)),
    };
    Some(crate::capture_panels::hints_panel(
        "overlay",
        view.chrome,
        language,
        rect,
        rows,
        cx,
        OverlayView::dismiss_hints,
    ))
}

pub(super) fn selection_handles(
    view: &OverlayView,
    cx: &mut Context<OverlayView>,
) -> Vec<AnyElement> {
    let Some(selection) = view.selection_bounds() else {
        return Vec::new();
    };
    let mut handles = Vec::new();
    let radius = view.selection_effects_radius();
    for (handle, position) in handle_positions(selection) {
        let (dx, dy) = corner_handle_offset(handle, radius);
        let cursor = region_handle_cursor(handle);
        handles.push(
            div()
                .id(format!("overlay-handle-{:?}", handle))
                .absolute()
                .left(px(f32::from(position.x) + dx - HANDLE_SIZE / 2.0))
                .top(px(f32::from(position.y) + dy - HANDLE_SIZE / 2.0))
                .w(px(HANDLE_SIZE))
                .h(px(HANDLE_SIZE))
                .rounded(px(1.5))
                .bg(view.chrome.handle_fill)
                .border_1()
                .border_color(view.chrome.handle_border)
                .shadow(vec![
                    BoxShadow::new(px(0.0), px(1.0), hsla(0.0, 0.0, 0.0, 0.28))
                        .blur_radius(px(2.0)),
                ])
                .cursor(cursor)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        let point = this.to_physical(event.position);
                        if let Some(selected) = this.selected.as_mut() {
                            selected.drag = Some(RegionDrag {
                                kind: RegionDragKind::Resize(handle),
                                start: point,
                                original: selected.rect,
                            });
                            selected.edit = None;
                            selected.effects_open = false;
                            selected.hints_open = false;
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .into_any_element(),
        );
    }
    handles
}

pub(super) fn selection_size_tag(view: &OverlayView, language: Language) -> Option<Div> {
    if view.selected.is_some() {
        return None;
    }
    let rect = view
        .session
        .selection_rect()
        .and_then(|rect| view.local_rect(rect))?;
    let (scale_x, scale_y) = view.frame_ratio();
    let width = (f32::from(rect.size.width) * scale_x).round() as u32;
    let height = (f32::from(rect.size.height) * scale_y).round() as u32;
    let ratio = view.aspect.ratio().map(|_| view.aspect.label(language));
    let text = match ratio {
        Some(ratio) => format!("{width} x {height}  {ratio}"),
        None => format!("{width} x {height}"),
    };
    let top = (f32::from(rect.origin.y) - 26.0).max(0.0);
    Some(
        div()
            .absolute()
            .left(rect.origin.x)
            .top(px(top))
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(view.chrome.surface)
            .border_1()
            .border_color(view.chrome.border)
            .shadow(view.chrome.shadow())
            .text_size(px(11.0))
            .text_color(view.chrome.text)
            .child(text),
    )
}
