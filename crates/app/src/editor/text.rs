use super::*;

impl EditorView {
    pub(super) fn edit_text_at(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) -> bool {
        let changed = self.state.edit_text_at(point);
        if changed {
            cx.notify();
        }
        changed
    }

    pub(super) fn commit_text(&mut self, cx: &mut Context<Self>) {
        if self.state.commit_text() {
            cx.notify();
        }
    }

    pub(super) fn cancel_text(&mut self, cx: &mut Context<Self>) {
        if self.state.cancel_text() {
            cx.notify();
        }
    }

    pub(super) fn backspace_text(&mut self, cx: &mut Context<Self>) {
        if self.state.backspace_text() {
            cx.notify();
        }
    }

    pub(super) fn delete_text(&mut self, cx: &mut Context<Self>) {
        if self.state.delete_text() {
            cx.notify();
        }
    }

    pub(super) fn move_text_cursor(&mut self, cursor: usize, extend: bool, cx: &mut Context<Self>) {
        if self.state.move_text_cursor(cursor, extend) {
            cx.notify();
        }
    }
}

impl EditorView {
    pub(super) fn text_index_at(&self, window: &Window, position: Point<Pixels>) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let bounds = self.text_edit_screen?;
        let scale = self.image_scale(window);
        let font_size = (edit.style.font_size * scale).max(8.0);
        let width = (edit.bounds.size.width as f32 * scale).max(24.0);
        let Some(layouts) = layout_annotation_text(window, &edit.content, font_size, width) else {
            return Some(0);
        };
        let line_height = font_size * 1.3;
        let boxes = text_line_boxes(&layouts, line_height);
        let local_x = f32::from(position.x - bounds.origin.x).max(0.0);
        let local_y = f32::from(position.y - bounds.origin.y).max(0.0);
        let line = boxes
            .iter()
            .find(|line| local_y < line.y + line_height)
            .or_else(|| boxes.last())?;
        let layout = layouts.get(line.hard_line)?;
        let offset = text_alignment_offset(edit.style.text_alignment, width, line.width);
        let point = point(
            px((local_x - offset).max(0.0)),
            px((local_y - line.y).max(0.0)),
        );
        let index = layout
            .index_for_position(point, px(line_height))
            .unwrap_or_else(|index| index)
            .min(layout.len());
        Some(line.start + index)
    }

    pub(super) fn move_text_vertical(
        &mut self,
        up: bool,
        extend: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = (|| {
            let edit = self.text_edit.as_ref()?;
            let scale = self.image_scale(window);
            let font_size = (edit.style.font_size * scale).max(8.0);
            let width = (edit.bounds.size.width as f32 * scale).max(24.0);
            let line_height = font_size * 1.3;
            let layouts = layout_annotation_text(window, &edit.content, font_size, width)?;
            let boxes = text_line_boxes(&layouts, line_height);
            let caret = text_caret_position(Some(&layouts), &boxes, edit, width);
            let target_y =
                (f32::from(caret.y) + if up { -line_height } else { line_height }).max(0.0);
            let line = boxes
                .iter()
                .find(|line| target_y < line.y + line_height)
                .or_else(|| boxes.last())?;
            let layout = layouts.get(line.hard_line)?;
            let offset = text_alignment_offset(edit.style.text_alignment, width, line.width);
            let x = (f32::from(caret.x) - offset).max(0.0);
            let y = (target_y - line.y).max(0.0);
            let index = layout
                .index_for_position(point(px(x), px(y)), px(line_height))
                .unwrap_or_else(|index| index)
                .min(layout.len());
            Some(line.start + index)
        })() else {
            return;
        };
        self.move_text_cursor(index, extend, cx);
    }

    pub(super) fn text_line_edge(&self, start: bool, window: &Window) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let scale = self.image_scale(window);
        let font_size = (edit.style.font_size * scale).max(8.0);
        let width = (edit.bounds.size.width as f32 * scale).max(24.0);
        let line_height = font_size * 1.3;
        let layouts = layout_annotation_text(window, &edit.content, font_size, width)?;
        let boxes = text_line_boxes(&layouts, line_height);
        let caret = text_caret_position(Some(&layouts), &boxes, edit, width);
        let y = f32::from(caret.y);
        let line = boxes
            .iter()
            .find(|line| y < line.y + line_height)
            .or_else(|| boxes.last())?;
        Some(if start { line.start } else { line.end })
    }

    pub(super) fn select_text_word(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        let range = word_range_at(&edit.content, index);
        edit.anchor = range.start;
        edit.cursor = range.end;
        edit.selecting = false;
        cx.notify();
    }

    pub(super) fn select_all_text(&mut self, cx: &mut Context<Self>) {
        if self.state.select_all_text() {
            cx.notify();
        }
    }

    pub(super) fn copy_text_selection(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        let selection = edit.selection();
        if selection.is_empty() {
            return;
        }
        let text = edit.content[selection].to_owned();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_status(
            t(settings_state::get().language, "editor.copied").to_owned(),
            cx,
        );
    }

    pub(super) fn paste_text(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| {
            item.entries().iter().find_map(|entry| match entry {
                ClipboardEntry::String(string) => Some(string.text.clone()),
                _ => None,
            })
        }) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(edit) = &mut self.text_edit {
            edit.replace(None, &text, false);
            cx.notify();
        }
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let edit = self.text_edit.as_ref()?;
        let range = edit.range_from_utf16(&range_utf16);
        actual_range.replace(edit.range_to_utf16(&range));
        Some(edit.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let edit = self.text_edit.as_ref()?;
        let selection = edit.selection();
        Some(UTF16Selection {
            range: edit.range_to_utf16(&selection),
            reversed: edit.anchor > edit.cursor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let edit = self.text_edit.as_ref()?;
        edit.marked.as_ref().map(|range| edit.range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.text_edit {
            edit.marked = None;
            window.notify_text_input_changed(&self.focus_handle);
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = &mut self.text_edit {
            let range = range_utf16
                .as_ref()
                .map(|range| edit.range_from_utf16(range));
            edit.replace(range, new_text, false);
            window.notify_text_input_changed(&self.focus_handle);
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = &mut self.text_edit {
            let range = range_utf16
                .as_ref()
                .map(|range| edit.range_from_utf16(range));
            edit.replace(range, new_text, true);
            window.notify_text_input_changed(&self.focus_handle);
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.text_edit.as_ref()?;
        Some(self.text_edit_screen.unwrap_or(element_bounds))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let index = self.text_index_at(window, point).unwrap_or(0);
        Some(edit.offset_to_utf16(index))
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        Some(edit.offset_to_utf16(edit.content.len()))
    }
}
