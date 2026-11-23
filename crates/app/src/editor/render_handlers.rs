use super::*;

impl EditorView {
    pub(super) fn object_layers(
        &mut self,
        objects: &[core_annotate::Annotation],
        scale: f32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut object_layers: Vec<AnyElement> = Vec::with_capacity(objects.len());
        for annotation in objects {
            match &annotation.shape {
                Shape::Blur(rect) => {
                    if rect.is_empty() {
                        continue;
                    }
                    let selection = self.full_selection();
                    let state = &self.state;
                    self.blur_cache
                        .ensure(&state.document, &self.frame, selection, cx);
                    let Some((_, preview_rect, blur_image)) = self
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
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.focus_handle.clone();
        window.focus(&focus, cx);
        if self.text_edit.is_none()
            && (self.space_down || self.zoomable_pan_start(event.position, window))
        {
            self.pan_draft = Some(PanDraft {
                start: event.position,
                start_pan: self.view_pan,
            });
            self.status = None;
            cx.notify();
            return;
        }
        let Some(display_point) = self.to_display_point(event.position, window) else {
            return;
        };
        if self.region_drag.is_some() {
            self.finish_region_drag(cx);
        }
        self.cancel_move();
        self.cancel_resize();
        if self
            .full_selection()
            .is_some_and(|selection| !selection.contains(display_point))
        {
            if let Some(handle) = self.region_handle_at(display_point) {
                self.begin_region_drag(handle, cx);
                cx.stop_propagation();
            }
            return;
        }
        let point = display_point;
        let scale = self.image_scale(window);
        if let Some(annotation) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .cloned()
            && let Some(handle) = annotation_handle_at(&annotation.shape, point, scale)
        {
            self.resize_draft = Some(ResizeDraft {
                id: annotation.id,
                handle,
                start: point,
                current: point,
                original: annotation.shape,
            });
            self.cancel_move();
            self.status = None;
            cx.notify();
            return;
        }
        if self.crop_mode {
            self.crop_draft = Some(CropDraft {
                start: point,
                current: point,
            });
            cx.notify();
            return;
        }
        if self.text_edit.is_some() {
            self.commit_text(cx);
        }
        let hit = self.document.hit_test(point);
        if hit.is_none()
            && self.can_adjust_region()
            && let Some(handle) = self.region_handle_at(point)
        {
            self.begin_region_drag(handle, cx);
            cx.stop_propagation();
            return;
        }
        if event.click_count >= 2 && hit.is_some() && self.edit_text_at(point, cx) {
            let focus = self.focus_handle.clone();
            window.focus(&focus, cx);
            cx.stop_propagation();
            return;
        }
        if self.polyline_draft.is_none()
            && let Some(id) = hit
        {
            log_info!(
                "editor",
                "annotation selected at image ({}, {}), {} object(s)",
                point.x,
                point.y,
                self.document.objects().len()
            );
            self.draft = None;
            self.cancel_resize();
            if self.document.selected() != Some(id) {
                self.select_annotation(Some(id));
            }
            if let Some(original) = self.document.object(id).map(|item| item.shape.clone()) {
                self.move_draft = Some(MoveDraft {
                    id,
                    start: point,
                    current: point,
                    original,
                });
            }
            self.status = None;
            cx.notify();
            return;
        }
        if self.tool != Tool::Select && self.document.selected().is_some() {
            self.document.select(None);
            self.selection_hover = SelectionHover::None;
        }
        if event.click_count >= 2
            && self.inline.is_some()
            && self.polyline_draft.is_none()
            && self.text_edit.is_none()
            && self.document.hit_test(point).is_none()
        {
            if self.inline.and_then(|inline| inline.capture).is_some() {
                match settings_state::get().capture_double_click {
                    DoubleClickAction::NoAction => {}
                    DoubleClickAction::Copy => self.export(ExportAction::Copy, cx),
                    DoubleClickAction::Pin => self.pin(cx),
                }
            } else {
                self.export(ExportAction::Copy, cx);
            }
            return;
        }
        if self.tool == Tool::Arrow && self.arrow_polyline {
            self.cancel_resize();
            self.cancel_move();
            if event.click_count >= 2 {
                if let Some(draft) = &mut self.polyline_draft {
                    draft.current = point;
                }
                self.finish_polyline(cx);
                return;
            }
            if let Some(draft) = &mut self.polyline_draft {
                if distance(*draft.points.last().unwrap_or(&point), point) > 2.0 {
                    draft.points.push(point);
                }
                draft.current = point;
            } else {
                self.polyline_draft = Some(PolylineDraft {
                    points: vec![point],
                    current: point,
                });
            }
            self.show_status(
                t(settings_state::get().language, "editor.polyline_hint").to_owned(),
                cx,
            );
            return;
        }
        if self.tool == Tool::Select {
            let selected = hit;
            self.cancel_resize();
            self.select_annotation(selected);
            self.move_draft = selected.and_then(|id| {
                let original = self.document.object(id)?.shape.clone();
                Some(MoveDraft {
                    id,
                    start: point,
                    current: point,
                    original,
                })
            });
            self.status = None;
            cx.notify();
            return;
        }
        self.cancel_resize();
        self.cancel_move();
        if self.tool == Tool::Number {
            let side = ((self.number_size as f32 * self.ui_scale).round() as u32).max(8);
            let max_left = (self.image_size.width.saturating_sub(side)) as i32;
            let max_top = (self.image_size.height.saturating_sub(side)) as i32;
            let left = (point.x - side as i32 / 2).clamp(0, max_left);
            let top = (point.y - side as i32 / 2).clamp(0, max_top);
            let value = self.document.next_number();
            let style = self.effective_style();
            self.state.add_annotation(
                Shape::Number {
                    bounds: PhysicalRect::new(left, top, side, side),
                    value,
                },
                style,
            );
            self.status = None;
            cx.notify();
            return;
        }
        if self.tool == Tool::Text {
            self.text_edit = Some(TextEdit::start(
                point,
                self.image_size,
                self.effective_style(),
            ));
            self.status = None;
            let focus = self.focus_handle.clone();
            window.focus(&focus, cx);
            cx.notify();
            return;
        }
        self.draft = Some(Draft {
            tool: self.tool,
            start: point,
            current: point,
            shift: event.modifiers.shift,
            ellipse: self.tool == Tool::Rect && self.shape_ellipse,
        });
        cx.notify();
    }

    pub(super) fn handle_editor_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        if self.custom_color_open {
            match key {
                "escape" => {
                    self.close_custom_color();
                    cx.notify();
                    return;
                }
                "enter" => {
                    if self.apply_custom_color(cx) {
                        return;
                    }
                }
                "backspace" | "delete" => {
                    if self.custom_color_fresh {
                        self.custom_color_buffer.clear();
                        self.custom_color_fresh = false;
                    } else {
                        self.custom_color_buffer.pop();
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
                if self.custom_color_fresh {
                    self.custom_color_buffer.clear();
                    self.custom_color_fresh = false;
                }
                if self.custom_color_buffer.len() < 6 {
                    self.custom_color_buffer.push(ch.to_ascii_lowercase());
                }
                cx.notify();
                return;
            }
            return;
        }
        if self.text_edit.is_some() {
            if self
                .text_edit
                .as_ref()
                .is_some_and(|edit| edit.marked.is_some())
            {
                return;
            }
            let extend = event.keystroke.modifiers.shift;
            match key {
                "escape" => {
                    self.cancel_text(cx);
                    return;
                }
                "enter" => {
                    if event.keystroke.modifiers.shift {
                        if let Some(edit) = &mut self.text_edit {
                            edit.replace(None, "\n", false);
                        }
                        cx.notify();
                    } else {
                        self.commit_text(cx);
                    }
                    return;
                }
                "backspace" => {
                    self.backspace_text(cx);
                    return;
                }
                "delete" => {
                    self.delete_text(cx);
                    return;
                }
                "left" => {
                    let position = self.text_edit.as_ref().map(|edit| edit.previous_boundary());
                    if let Some(position) = position {
                        self.move_text_cursor(position, extend, cx);
                    }
                    return;
                }
                "right" => {
                    let position = self.text_edit.as_ref().map(|edit| edit.next_boundary());
                    if let Some(position) = position {
                        self.move_text_cursor(position, extend, cx);
                    }
                    return;
                }
                "up" => {
                    self.move_text_vertical(true, extend, window, cx);
                    return;
                }
                "down" => {
                    self.move_text_vertical(false, extend, window, cx);
                    return;
                }
                "home" => {
                    let position = self.text_line_edge(true, window);
                    if let Some(position) = position {
                        self.move_text_cursor(position, extend, cx);
                    }
                    return;
                }
                "end" => {
                    let position = self.text_line_edge(false, window);
                    if let Some(position) = position {
                        self.move_text_cursor(position, extend, cx);
                    }
                    return;
                }
                _ => {}
            }
            if command && key == "a" {
                self.select_all_text(cx);
                return;
            }
            if command && key == "c" {
                self.copy_text_selection(cx);
                return;
            }
            if command && key == "v" {
                self.paste_text(cx);
                return;
            }
            if command && key == "s" && !event.keystroke.modifiers.shift {
                self.commit_text(cx);
                self.export(ExportAction::Save, cx);
                return;
            }
            return;
        }
        if self.polyline_draft.is_some() {
            if key == "enter" {
                self.finish_polyline(cx);
                return;
            }
            if key == "escape" {
                self.cancel_polyline(cx);
                return;
            }
        }
        if self.crop_mode {
            if key == "enter" {
                self.apply_crop(cx);
                return;
            }
            if key == "escape" {
                self.cancel_crop(cx);
                return;
            }
        }
        if key == "escape" && (self.more_open || self.effects_open || self.hints_visible) {
            self.more_open = false;
            self.effects_open = false;
            self.hints_visible = false;
            cx.notify();
            return;
        }
        if key == "f1" {
            self.hints_visible = !self.hints_visible;
            self.more_open = false;
            self.effects_open = false;
            cx.notify();
            return;
        }
        if key == "space" {
            if self.inline.is_some() {
                if self.inline.is_some_and(|inline| inline.zoomable) {
                    self.space_down = true;
                } else {
                    self.toolbar_hidden = !self.toolbar_hidden;
                    self.more_open = false;
                    self.effects_open = false;
                    self.hints_visible = false;
                }
            } else {
                self.space_down = true;
            }
            cx.notify();
            return;
        }
        if key == "f5" {
            if !event.is_held {
                self.refresh_capture(cx);
            }
            return;
        }
        if key == "enter" {
            self.export(ExportAction::Copy, cx);
            return;
        }
        if command && key == "0" {
            self.reset_view(cx);
            return;
        }
        if command && key == "1" {
            self.actual_size(window, cx);
            return;
        }
        if command && (key == "=" || key == "+" || key == "-") {
            let view = self.image_view_rect(window);
            let center = point(
                px(f32::from(view.origin.x) + f32::from(view.size.width) / 2.0),
                px(f32::from(view.origin.y) + f32::from(view.size.height) / 2.0),
            );
            let factor = if key == "-" { 1.0 / 1.2 } else { 1.2 };
            self.zoom_at(factor, center, window, cx);
            return;
        }
        if key == "escape" {
            if self.region_drag.is_some() {
                self.region_drag = None;
                let scale = self
                    .inline
                    .and_then(|inline| inline.capture)
                    .map(|capture| capture.scale.max(0.1))
                    .unwrap_or(1.0);
                self.restore_region_image_rect(scale, cx);
                return;
            }
            if self.cancel_annotation_state(cx) {
                return;
            }
            self.release_images(cx);
            if self.inline.is_some() {
                let handles = cx.windows();
                cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
            } else {
                window.remove_window();
            }
            return;
        }
        if command && matches!(key, "w" | "q") {
            self.release_images(cx);
            if self.inline.is_some() {
                let handles = cx.windows();
                cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
            } else {
                window.remove_window();
            }
            return;
        }
        if key == "delete" || key == "backspace" {
            self.delete_selected_annotation(cx);
            return;
        }
        if command && key == "c" && !event.keystroke.modifiers.shift {
            if !self.copy_selected_annotation(cx) {
                self.export(ExportAction::Copy, cx);
            }
            return;
        }
        if command && key == "v" {
            self.paste_annotation(cx);
            return;
        }
        if command && key == "d" {
            if let Some(annotation) = self
                .document
                .selected()
                .and_then(|id| self.document.object(id))
                .cloned()
            {
                let pasted = self.state.paste_annotation(&annotation, 16, 16);
                self.select_annotation(Some(pasted));
                self.tool = Tool::Select;
                self.show_status(
                    t(settings_state::get().language, "editor.annotation_pasted").to_owned(),
                    cx,
                );
            }
            return;
        }
        if command && key == "s" && !event.keystroke.modifiers.shift {
            self.export(ExportAction::Save, cx);
            return;
        }
        if command && key == "z" {
            if event.keystroke.modifiers.shift {
                self.redo_editor(cx);
            } else {
                self.undo_editor(cx);
            }
            self.status = None;
            cx.notify();
        }
    }
}
