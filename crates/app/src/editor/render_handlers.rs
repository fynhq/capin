use super::*;

pub(super) fn object_layers(
    view: &mut EditorView,
    objects: &[core_annotate::Annotation],
    scale: f32,
    cx: &mut Context<EditorView>,
) -> Vec<AnyElement> {
    let mut object_layers: Vec<AnyElement> = Vec::with_capacity(objects.len());
    for annotation in objects {
        match &annotation.shape {
            Shape::Blur(rect) => {
                if rect.is_empty() {
                    continue;
                }
                let selection = view.full_selection();
                let state = &view.state;
                view.blur_cache
                    .ensure(&state.document, &view.frame, selection, cx);
                let Some((_, preview_rect, blur_image)) = view
                    .blur_cache
                    .previews()
                    .iter()
                    .find(|(id, _, _)| *id == annotation.id)
                else {
                    continue;
                };
                let left = preview_rect.origin.x as f32 * scale;
                let top = preview_rect.origin.y as f32 * scale;
                let width = px(preview_rect.size.width as f32 * scale);
                let height = px(preview_rect.size.height as f32 * scale);
                object_layers.push(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(top))
                        .w(width)
                        .h(height)
                        .child(
                            img(blur_image.clone())
                                .w(width)
                                .h(height)
                                .object_fit(ObjectFit::Fill),
                        )
                        .into_any_element(),
                );
            }
            Shape::Text { bounds, text } => {
                let color = (annotation.style.color >> 8) & 0x00FF_FFFF;
                let text_color = rgb(color);
                let text_align = match annotation.style.text_alignment {
                    TextAlignment::Left => TextAlign::Left,
                    TextAlignment::Center => TextAlign::Center,
                    TextAlignment::Right => TextAlign::Right,
                };
                let font_size = annotation.style.font_size.max(1.0) * scale;
                let mut layer = div()
                    .absolute()
                    .left(px(bounds.origin.x as f32 * scale))
                    .top(px(bounds.origin.y as f32 * scale))
                    .w(px(bounds.size.width as f32 * scale))
                    .text_size(px(font_size))
                    .line_height(px(font_size * 1.3))
                    .text_align(text_align);
                if annotation.style.text_background {
                    let background = if prefers_dark_foreground(color) {
                        rgba(0x000000A0)
                    } else {
                        rgba(0xFFFFFFBE)
                    };
                    layer = layer.rounded(px(font_size * 0.2)).bg(background);
                }
                if annotation.style.text_outline {
                    let outline = if prefers_dark_foreground(color) {
                        rgb(0x000000)
                    } else {
                        rgb(0xFFFFFF)
                    };
                    let stroke_offset = (annotation.style.stroke_width * 0.6).max(1.0).round();
                    let offset = (stroke_offset * scale).max(1.0) as i32;
                    for (dx, dy) in [
                        (-offset, 0),
                        (offset, 0),
                        (0, -offset),
                        (0, offset),
                        (-offset, -offset),
                        (offset, -offset),
                        (-offset, offset),
                        (offset, offset),
                    ] {
                        layer = layer.child(
                            div()
                                .absolute()
                                .left(px(dx as f32))
                                .top(px(dy as f32))
                                .w_full()
                                .text_size(px(font_size))
                                .line_height(px(font_size * 1.3))
                                .text_align(text_align)
                                .text_color(outline)
                                .child(text.clone()),
                        );
                    }
                }
                object_layers.push(
                    layer
                        .child(
                            div()
                                .w_full()
                                .line_height(px(font_size * 1.3))
                                .text_color(text_color)
                                .child(text.clone()),
                        )
                        .into_any_element(),
                );
            }
            Shape::Number { bounds, value } => {
                let color = (annotation.style.color >> 8) & 0x00FF_FFFF;
                let width = bounds.size.width as f32 * scale;
                let height = bounds.size.height as f32 * scale;
                let badge_foreground = core_annotate::badge_foreground(color);
                let foreground = rgb((badge_foreground >> 8) & 0x00FF_FFFF);
                let background = rgb(color);
                let radius = height.min(width) / 2.0;
                object_layers.push(
                    div()
                        .absolute()
                        .left(px(bounds.origin.x as f32 * scale))
                        .top(px(bounds.origin.y as f32 * scale))
                        .w(px(width))
                        .h(px(height))
                        .rounded(px(radius))
                        .bg(background)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px((bounds.size.height as f32 * 0.54).max(10.0) * scale))
                        .text_color(foreground)
                        .child(value.to_string())
                        .into_any_element(),
                );
            }
            shape => {
                let shape = shape.clone();
                let style = annotation.style;
                object_layers.push(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            paint_shape(window, bounds, scale, &shape, style);
                        },
                    )
                    .absolute()
                    .inset_0()
                    .size_full()
                    .into_any_element(),
                );
            }
        }
    }
    object_layers
}

pub(super) fn handle_canvas_mouse_down(
    view: &mut EditorView,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    let focus = view.focus_handle.clone();
    window.focus(&focus, cx);
    if view.text_edit.is_none()
        && (view.space_down || view.zoomable_pan_start(event.position, window))
    {
        view.pan_draft = Some(PanDraft {
            start: event.position,
            start_pan: view.view_pan,
        });
        view.status = None;
        cx.notify();
        return;
    }
    let Some(display_point) = view.to_display_point(event.position, window) else {
        return;
    };
    if view.region_adjust.draft.is_some() {
        view.finish_region_drag(cx);
    }
    view.cancel_move();
    view.cancel_resize();
    if view
        .full_selection()
        .is_some_and(|selection| !selection.contains(display_point))
    {
        if let Some(handle) = view.region_handle_at(display_point) {
            view.begin_region_drag(handle, cx);
            cx.stop_propagation();
        }
        return;
    }
    let point = display_point;
    let scale = view.image_scale(window);
    if let Some(annotation) = view
        .document
        .selected()
        .and_then(|id| view.document.object(id))
        .cloned()
        && let Some(handle) = annotation_handle_at(&annotation.shape, point, scale)
    {
        view.resize_draft = Some(ResizeDraft {
            id: annotation.id,
            handle,
            start: point,
            current: point,
            original: annotation.shape,
        });
        view.cancel_move();
        view.status = None;
        cx.notify();
        return;
    }
    if view.crop_mode {
        view.crop_draft = Some(CropDraft {
            start: point,
            current: point,
        });
        cx.notify();
        return;
    }
    if view.text_edit.is_some() {
        view.commit_text(cx);
    }
    let hit = view.document.hit_test(point);
    if hit.is_none()
        && canvas::can_adjust_region(view)
        && let Some(handle) = view.region_handle_at(point)
    {
        view.begin_region_drag(handle, cx);
        cx.stop_propagation();
        return;
    }
    if event.click_count >= 2 && hit.is_some() && view.edit_text_at(point, cx) {
        let focus = view.focus_handle.clone();
        window.focus(&focus, cx);
        cx.stop_propagation();
        return;
    }
    if view.polyline_draft.is_none()
        && let Some(id) = hit
    {
        log_info!(
            "editor",
            "annotation selected at image ({}, {}), {} object(s)",
            point.x,
            point.y,
            view.document.objects().len()
        );
        view.draft = None;
        view.cancel_resize();
        if view.document.selected() != Some(id) {
            view.select_annotation(Some(id));
        }
        if let Some(original) = view.document.object(id).map(|item| item.shape.clone()) {
            view.move_draft = Some(MoveDraft {
                id,
                start: point,
                current: point,
                original,
            });
        }
        view.status = None;
        cx.notify();
        return;
    }
    if view.tool != Tool::Select && view.document.selected().is_some() {
        view.document.select(None);
        view.selection_hover = SelectionHover::None;
    }
    if event.click_count >= 2
        && view.inline.is_some()
        && view.polyline_draft.is_none()
        && view.text_edit.is_none()
        && view.document.hit_test(point).is_none()
    {
        if view.inline.and_then(|inline| inline.capture).is_some() {
            match settings_state::get().capture_double_click {
                DoubleClickAction::NoAction => {}
                DoubleClickAction::Copy => view.export(ExportAction::Copy, cx),
                DoubleClickAction::Pin => view.pin(cx),
            }
        } else {
            view.export(ExportAction::Copy, cx);
        }
        return;
    }
    if view.tool == Tool::Arrow && view.arrow_polyline {
        view.cancel_resize();
        view.cancel_move();
        if event.click_count >= 2 {
            if let Some(draft) = &mut view.polyline_draft {
                draft.current = point;
            }
            view.finish_polyline(cx);
            return;
        }
        if let Some(draft) = &mut view.polyline_draft {
            if distance(*draft.points.last().unwrap_or(&point), point) > 2.0 {
                draft.points.push(point);
            }
            draft.current = point;
        } else {
            view.polyline_draft = Some(PolylineDraft {
                points: vec![point],
                current: point,
            });
        }
        view.show_status(
            t(settings_state::get().language, "editor.polyline_hint").to_owned(),
            cx,
        );
        return;
    }
    if view.tool == Tool::Select {
        let selected = hit;
        view.cancel_resize();
        view.select_annotation(selected);
        view.move_draft = selected.and_then(|id| {
            let original = view.document.object(id)?.shape.clone();
            Some(MoveDraft {
                id,
                start: point,
                current: point,
                original,
            })
        });
        view.status = None;
        cx.notify();
        return;
    }
    view.cancel_resize();
    view.cancel_move();
    if view.tool == Tool::Number {
        let side = ((view.number_size as f32 * view.ui_scale).round() as u32).max(8);
        let max_left = (view.image_size.width.saturating_sub(side)) as i32;
        let max_top = (view.image_size.height.saturating_sub(side)) as i32;
        let left = (point.x - side as i32 / 2).clamp(0, max_left);
        let top = (point.y - side as i32 / 2).clamp(0, max_top);
        let value = view.document.next_number();
        let style = view.effective_style();
        view.state.add_annotation(
            Shape::Number {
                bounds: PhysicalRect::new(left, top, side, side),
                value,
            },
            style,
        );
        view.status = None;
        cx.notify();
        return;
    }
    if view.tool == Tool::Text {
        view.text_edit = Some(TextEdit::start(
            point,
            view.image_size,
            view.effective_style(),
        ));
        view.status = None;
        let focus = view.focus_handle.clone();
        window.focus(&focus, cx);
        cx.notify();
        return;
    }
    view.draft = Some(Draft {
        tool: view.tool,
        start: point,
        current: point,
        shift: event.modifiers.shift,
        ellipse: view.tool == Tool::Rect && view.shape_ellipse,
    });
    cx.notify();
}

pub(super) fn handle_editor_key(
    view: &mut EditorView,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    let key = event.keystroke.key.as_str();
    let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
    if view.toolbar.custom_color_open {
        match key {
            "escape" => {
                view.close_custom_color();
                cx.notify();
                return;
            }
            "enter" => {
                if view.apply_custom_color(cx) {
                    return;
                }
            }
            "backspace" | "delete" => {
                if view.toolbar.custom_color_fresh {
                    view.toolbar.custom_color_buffer.clear();
                    view.toolbar.custom_color_fresh = false;
                } else {
                    view.toolbar.custom_color_buffer.pop();
                }
                cx.notify();
                return;
            }
            _ => {}
        }
        if !command
            && let Some(ch) = event.keystroke.key.chars().next()
            && ch.is_ascii_hexdigit()
        {
            if view.toolbar.custom_color_fresh {
                view.toolbar.custom_color_buffer.clear();
                view.toolbar.custom_color_fresh = false;
            }
            if view.toolbar.custom_color_buffer.len() < 6 {
                view.toolbar
                    .custom_color_buffer
                    .push(ch.to_ascii_lowercase());
            }
            cx.notify();
            return;
        }
        return;
    }
    if view.text_edit.is_some() {
        if view
            .text_edit
            .as_ref()
            .is_some_and(|edit| edit.marked.is_some())
        {
            return;
        }
        let extend = event.keystroke.modifiers.shift;
        match key {
            "escape" => {
                view.cancel_text(cx);
                return;
            }
            "enter" => {
                if event.keystroke.modifiers.shift {
                    if let Some(edit) = &mut view.text_edit {
                        edit.replace(None, "\n", false);
                    }
                    cx.notify();
                } else {
                    view.commit_text(cx);
                }
                return;
            }
            "backspace" => {
                view.backspace_text(cx);
                return;
            }
            "delete" => {
                view.delete_text(cx);
                return;
            }
            "left" => {
                let position = view.text_edit.as_ref().map(|edit| edit.previous_boundary());
                if let Some(position) = position {
                    view.move_text_cursor(position, extend, cx);
                }
                return;
            }
            "right" => {
                let position = view.text_edit.as_ref().map(|edit| edit.next_boundary());
                if let Some(position) = position {
                    view.move_text_cursor(position, extend, cx);
                }
                return;
            }
            "up" => {
                view.move_text_vertical(true, extend, window, cx);
                return;
            }
            "down" => {
                view.move_text_vertical(false, extend, window, cx);
                return;
            }
            "home" => {
                let position = view.text_line_edge(true, window);
                if let Some(position) = position {
                    view.move_text_cursor(position, extend, cx);
                }
                return;
            }
            "end" => {
                let position = view.text_line_edge(false, window);
                if let Some(position) = position {
                    view.move_text_cursor(position, extend, cx);
                }
                return;
            }
            _ => {}
        }
        if command && key == "a" {
            view.select_all_text(cx);
            return;
        }
        if command && key == "c" {
            view.copy_text_selection(cx);
            return;
        }
        if command && key == "v" {
            view.paste_text(cx);
            return;
        }
        if command && key == "s" && !event.keystroke.modifiers.shift {
            view.commit_text(cx);
            view.export(ExportAction::Save, cx);
            return;
        }
        return;
    }
    if view.polyline_draft.is_some() {
        if key == "enter" {
            view.finish_polyline(cx);
            return;
        }
        if key == "escape" {
            view.cancel_polyline(cx);
            return;
        }
    }
    if view.crop_mode {
        if key == "enter" {
            view.apply_crop(cx);
            return;
        }
        if key == "escape" {
            view.cancel_crop(cx);
            return;
        }
    }
    if key == "escape"
        && (view.toolbar.more_open || view.toolbar.effects_open || view.toolbar.hints_visible)
    {
        view.toolbar.more_open = false;
        view.toolbar.effects_open = false;
        view.toolbar.hints_visible = false;
        cx.notify();
        return;
    }
    if key == "f1" {
        view.toolbar.hints_visible = !view.toolbar.hints_visible;
        view.toolbar.more_open = false;
        view.toolbar.effects_open = false;
        cx.notify();
        return;
    }
    if key == "space" {
        if view.inline.is_some() {
            if view.inline.is_some_and(|inline| inline.zoomable) {
                view.space_down = true;
            } else {
                view.toolbar.hidden = !view.toolbar.hidden;
                view.toolbar.more_open = false;
                view.toolbar.effects_open = false;
                view.toolbar.hints_visible = false;
            }
        } else {
            view.space_down = true;
        }
        cx.notify();
        return;
    }
    if key == "f5" {
        if !event.is_held {
            view.refresh_capture(cx);
        }
        return;
    }
    if key == "enter" {
        view.export(ExportAction::Copy, cx);
        return;
    }
    if command && key == "0" {
        view.reset_view(cx);
        return;
    }
    if command && key == "1" {
        view.actual_size(window, cx);
        return;
    }
    if command && (key == "=" || key == "+" || key == "-") {
        let rect = view.image_view_rect(window);
        let center = point(
            px(f32::from(rect.origin.x) + f32::from(rect.size.width) / 2.0),
            px(f32::from(rect.origin.y) + f32::from(rect.size.height) / 2.0),
        );
        let factor = if key == "-" { 1.0 / 1.2 } else { 1.2 };
        view.zoom_at(factor, center, window, cx);
        return;
    }
    if key == "escape" {
        if view.region_adjust.draft.is_some() {
            view.region_adjust.cancel();
            let scale = view
                .inline
                .and_then(|inline| inline.capture)
                .map(|capture| capture.scale.max(0.1))
                .unwrap_or(1.0);
            canvas::restore_region_image_rect(view, scale, cx);
            return;
        }
        if canvas::cancel_annotation_state(view, cx) {
            return;
        }
        view.release_images(cx);
        if view.inline.is_some() {
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
        } else {
            window.remove_window();
        }
        return;
    }
    if command && matches!(key, "w" | "q") {
        view.release_images(cx);
        if view.inline.is_some() {
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
        } else {
            window.remove_window();
        }
        return;
    }
    if key == "delete" || key == "backspace" {
        view.delete_selected_annotation(cx);
        return;
    }
    if command && key == "c" && !event.keystroke.modifiers.shift {
        if !view.copy_selected_annotation(cx) {
            view.export(ExportAction::Copy, cx);
        }
        return;
    }
    if command && key == "v" {
        view.paste_annotation(cx);
        return;
    }
    if command && key == "d" {
        if let Some(annotation) = view
            .document
            .selected()
            .and_then(|id| view.document.object(id))
            .cloned()
        {
            let pasted = view.state.paste_annotation(&annotation, 16, 16);
            view.select_annotation(Some(pasted));
            view.tool = Tool::Select;
            view.show_status(
                t(settings_state::get().language, "editor.annotation_pasted").to_owned(),
                cx,
            );
        }
        return;
    }
    if command && key == "s" && !event.keystroke.modifiers.shift {
        view.export(ExportAction::Save, cx);
        return;
    }
    if command && key == "z" {
        if event.keystroke.modifiers.shift {
            view.redo_editor(cx);
        } else {
            view.undo_editor(cx);
        }
        view.status = None;
        cx.notify();
    }
}
