use std::ops::Range;
use std::sync::Arc;

use core_annotate::{Annotation, AnnotationDocument, AnnotationStyle, Shape, rasterize};
use core_types::{PhysicalPoint, PhysicalRect, PhysicalSize, RgbaFrame, prefers_dark_foreground};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::system_accent_hex;

use crate::{i18n::t, icons, image_util, settings_state};

const TOOLBAR_HEIGHT: f32 = 48.0;
const PADDING: f32 = 16.0;
const MAX_IMAGE_WIDTH: f32 = 1200.0;
const MAX_IMAGE_HEIGHT: f32 = 800.0;
const MIN_WINDOW_WIDTH: f32 = 860.0;
const BACKGROUND: u32 = 0x141414FF;
const TOOLBAR_BG: u32 = 0x202020FF;
const TEXT: u32 = 0xF2F2F2FF;
const MUTED: u32 = 0x9A9A9AFF;
const BORDER: u32 = 0xFFFFFF1A;
const HOVER: u32 = 0xFFFFFF14;
const TOOLBAR_SIZE: u32 = 32;
const BADGE_SIZE: u32 = 32;

const STROKE_WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Select,
    Rect,
    Ellipse,
    Arrow,
    Highlighter,
    Blur,
    Text,
    Number,
    Spotlight,
}

impl Tool {
    const ALL: [Self; 9] = [
        Self::Select,
        Self::Rect,
        Self::Ellipse,
        Self::Arrow,
        Self::Highlighter,
        Self::Blur,
        Self::Text,
        Self::Number,
        Self::Spotlight,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Select => "editor-tool-select",
            Self::Rect => "editor-tool-rect",
            Self::Ellipse => "editor-tool-ellipse",
            Self::Arrow => "editor-tool-arrow",
            Self::Highlighter => "editor-tool-highlighter",
            Self::Blur => "editor-tool-blur",
            Self::Text => "editor-tool-text",
            Self::Number => "editor-tool-number",
            Self::Spotlight => "editor-tool-spotlight",
        }
    }

    fn icon(self) -> &'static [u8] {
        match self {
            Self::Select => icons::SELECT,
            Self::Rect => icons::RECT,
            Self::Ellipse => icons::ELLIPSE,
            Self::Arrow => icons::ARROW,
            Self::Highlighter => icons::HIGHLIGHTER,
            Self::Blur => icons::BLUR,
            Self::Text => icons::TEXT,
            Self::Number => icons::NUMBER,
            Self::Spotlight => icons::SPOTLIGHT,
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Select => "editor.tool.select",
            Self::Rect => "editor.tool.rect",
            Self::Ellipse => "editor.tool.ellipse",
            Self::Arrow => "editor.tool.arrow",
            Self::Highlighter => "editor.tool.highlighter",
            Self::Blur => "editor.tool.blur",
            Self::Text => "editor.tool.text",
            Self::Number => "editor.tool.number",
            Self::Spotlight => "editor.tool.spotlight",
        }
    }
}

fn stroke_id(width: f32) -> &'static str {
    match width as u32 {
        2 => "editor-width-2",
        4 => "editor-width-4",
        _ => "editor-width-8",
    }
}

struct TooltipLabel {
    text: SharedString,
}

impl Render for TooltipLabel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(rgba(0x000000E6))
            .text_size(px(11.5))
            .text_color(rgb(0xFFFFFF))
            .child(self.text.clone())
    }
}

fn tooltip_view(text: String, cx: &mut App) -> AnyView {
    cx.new(|_| TooltipLabel { text: text.into() }).into()
}

struct Draft {
    tool: Tool,
    start: PhysicalPoint,
    current: PhysicalPoint,
    points: Vec<PhysicalPoint>,
}

struct MoveDraft {
    id: core_annotate::AnnotationId,
    start: PhysicalPoint,
    current: PhysicalPoint,
}

struct TextEdit {
    target: Option<core_annotate::AnnotationId>,
    origin: PhysicalPoint,
    bounds: PhysicalRect,
    content: String,
    /// UTF-8 byte offsets. `anchor` is the selection origin; `cursor` is the
    /// caret and the end of a selection.
    cursor: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    style: AnnotationStyle,
}

impl TextEdit {
    fn start(origin: PhysicalPoint, image_size: PhysicalSize, style: AnnotationStyle) -> Self {
        let width = (image_size.width as i32 - origin.x).clamp(24, 640) as u32;
        let height = (style.font_size * 1.6).ceil() as u32;
        Self {
            target: None,
            origin,
            bounds: PhysicalRect::new(origin.x, origin.y, width, height),
            content: String::new(),
            cursor: 0,
            anchor: 0,
            marked: None,
            style,
        }
    }

    fn edit_existing(
        id: core_annotate::AnnotationId,
        bounds: PhysicalRect,
        content: String,
        style: AnnotationStyle,
    ) -> Self {
        Self {
            target: Some(id),
            origin: bounds.origin,
            bounds,
            cursor: content.len(),
            anchor: content.len(),
            content,
            marked: None,
            style,
        }
    }

    fn cursor_utf8(&self) -> usize {
        self.cursor.min(self.content.len())
    }

    fn selection(&self) -> Range<usize> {
        let cursor = self.cursor_utf8();
        let anchor = self.anchor.min(self.content.len());
        if anchor <= cursor {
            anchor..cursor
        } else {
            cursor..anchor
        }
    }

    fn move_cursor(&mut self, cursor: usize, extend_selection: bool) {
        let cursor = cursor.min(self.content.len());
        let cursor = if self.content.is_char_boundary(cursor) {
            cursor
        } else {
            previous_char_boundary(&self.content, cursor)
        };
        self.cursor = cursor;
        if !extend_selection {
            self.anchor = cursor;
        }
    }

    fn previous_boundary(&self) -> usize {
        previous_char_boundary(&self.content, self.cursor_utf8())
    }

    fn next_boundary(&self) -> usize {
        next_char_boundary(&self.content, self.cursor_utf8())
    }

    fn index_at_local(&self, local_x: f32, local_y: f32, width_logical: f32) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let scale = (width_logical / self.bounds.size.width.max(1) as f32).max(0.01);
        let x = local_x / scale;
        let y = local_y / scale;
        let font_size = self.style.font_size.max(1.0);
        let line_height = font_size * 1.3;
        let target_line = (y / line_height).floor().max(0.0) as usize;
        let width = self.bounds.size.width as f32;

        let mut line = 0usize;
        let mut line_width = 0.0f32;
        for (index, ch) in self.content.char_indices() {
            let advance = estimated_char_width(ch, font_size);
            if line_width + advance > width && line_width > 0.0 {
                if line == target_line {
                    return index;
                }
                line += 1;
                line_width = 0.0;
            }
            line_width += advance;
            if line == target_line && x <= line_width - advance / 2.0 {
                return index;
            }
        }
        self.content.len()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = offset.min(self.content.len());
        self.content[..offset]
            .chars()
            .map(|ch| ch.len_utf16())
            .sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_from_utf16(range.start);
        let end = self.offset_from_utf16(range.end).max(start);
        start..end
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_to_utf16(range.start);
        let end = self.offset_to_utf16(range.end).max(start);
        start..end
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str) {
        let selection = self.selection();
        let range = range
            .or_else(|| self.marked.take())
            .or_else(|| (!selection.is_empty()).then_some(selection))
            .unwrap_or(self.cursor_utf8()..self.cursor_utf8());
        let text: String = text
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect();
        self.content.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.marked = (!text.is_empty()).then_some(range.start..range.start + text.len());
    }

    fn finish_bounds(&self) -> PhysicalRect {
        let measured = core_annotate::measure_text_height(
            &self.content,
            self.style.font_size,
            self.bounds.size.width,
        );
        PhysicalRect::new(
            self.bounds.origin.x,
            self.bounds.origin.y,
            self.bounds.size.width,
            measured.max(self.bounds.size.height),
        )
    }
}

fn previous_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = cursor.min(content.len());
    content[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = cursor.min(content.len());
    content[cursor..]
        .chars()
        .next()
        .map(|ch| cursor + ch.len_utf8())
        .unwrap_or(cursor)
}

fn estimated_char_width(ch: char, font_size: f32) -> f32 {
    if ch.is_whitespace() {
        font_size * 0.35
    } else if ch.is_ascii() {
        font_size * 0.6
    } else {
        font_size
    }
}

struct EditorView {
    image: Arc<RenderImage>,
    blur_image: Arc<RenderImage>,
    frame: RgbaFrame,
    pin_target: Option<u64>,
    document: AnnotationDocument,
    image_size: PhysicalSize,
    tool: Tool,
    style: AnnotationStyle,
    draft: Option<Draft>,
    move_draft: Option<MoveDraft>,
    annotation_clipboard: Option<Annotation>,
    text_edit: Option<TextEdit>,
    text_edit_screen: Option<Bounds<Pixels>>,
    accent: Rgba,
    accent_hex: u32,
    accent_selected: Rgba,
    status: Option<String>,
    focus_handle: FocusHandle,
}

impl EditorView {
    fn new(frame: RgbaFrame, pin_target: Option<u64>, cx: &mut Context<Self>) -> Self {
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image(&frame);
        let blur_image = image_util::blurred_image(&frame);
        let accent_hex = system_accent_hex().unwrap_or(0x4C9AFF);
        Self {
            image,
            blur_image,
            frame,
            pin_target,
            document: AnnotationDocument::new(image_size),
            image_size,
            tool: Tool::Select,
            style: AnnotationStyle {
                color: settings_state::get().annotation_color,
                stroke_width: 4.0,
                font_size: 20.0,
            },
            draft: None,
            move_draft: None,
            annotation_clipboard: None,
            text_edit: None,
            text_edit_screen: None,
            accent: rgb(accent_hex),
            accent_hex,
            accent_selected: rgba((accent_hex << 8) | 0x33),
            status: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn image_view_rect(&self, window: &Window) -> Bounds<Pixels> {
        let viewport = window.viewport_size();
        let available_width = (f32::from(viewport.width) - PADDING * 2.0).max(1.0);
        let available_height =
            (f32::from(viewport.height) - TOOLBAR_HEIGHT - PADDING * 2.0).max(1.0);
        let image_width = self.image_size.width as f32;
        let image_height = self.image_size.height as f32;
        let scale = (available_width / image_width)
            .min(available_height / image_height)
            .min(1.0);
        let display_width = image_width * scale;
        let display_height = image_height * scale;
        Bounds {
            origin: point(
                px(PADDING + (available_width - display_width) / 2.0),
                px(TOOLBAR_HEIGHT + PADDING + (available_height - display_height) / 2.0),
            ),
            size: size(px(display_width), px(display_height)),
        }
    }

    fn to_image_point(&self, position: Point<Pixels>, window: &Window) -> Option<PhysicalPoint> {
        let view = self.image_view_rect(window);
        let scale = f32::from(view.size.width) / self.image_size.width as f32;
        let x = ((f32::from(position.x) - f32::from(view.origin.x)) / scale).floor() as i32;
        let y = ((f32::from(position.y) - f32::from(view.origin.y)) / scale).floor() as i32;
        if x < 0 || y < 0 || x >= self.image_size.width as i32 || y >= self.image_size.height as i32
        {
            return None;
        }
        Some(PhysicalPoint::new(x, y))
    }

    fn effective_style(&self) -> AnnotationStyle {
        let mut style = self.style;
        if self.tool == Tool::Highlighter {
            style.color = (style.color & 0xFFFFFF00) | 0x80;
            style.stroke_width = 12.0;
        }
        style
    }

    fn update_style(&mut self, update: impl FnOnce(&mut AnnotationStyle)) {
        let selected_style = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style);
        if let Some(mut style) = selected_style {
            update(&mut style);
            self.document.update_selected_style(style);
            self.style = style;
            return;
        }
        update(&mut self.style);
    }

    fn select_annotation(&mut self, id: Option<core_annotate::AnnotationId>) {
        self.document.select(id);
        if let Some(annotation) = id.and_then(|id| self.document.object(id)) {
            self.style = annotation.style;
        }
    }

    fn copy_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(annotation) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .cloned()
        else {
            return false;
        };
        self.annotation_clipboard = Some(annotation);
        self.status =
            Some(t(settings_state::get().language, "editor.annotation_copied").to_owned());
        cx.notify();
        true
    }

    fn paste_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(annotation) = self.annotation_clipboard.clone() else {
            return false;
        };
        let id = self.document.paste(&annotation, 16, 16);
        self.tool = Tool::Select;
        if let Some(annotation) = self.document.object(id) {
            self.style = annotation.style;
        }
        self.status =
            Some(t(settings_state::get().language, "editor.annotation_pasted").to_owned());
        cx.notify();
        true
    }

    fn delete_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.document.selected() else {
            return false;
        };
        if !self.document.remove(id) {
            return false;
        }
        self.status =
            Some(t(settings_state::get().language, "editor.annotation_deleted").to_owned());
        cx.notify();
        true
    }

    fn edit_text_at(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.document.hit_test(point) else {
            return false;
        };
        let Some(annotation) = self.document.object(id) else {
            return false;
        };
        let Shape::Text { bounds, text } = &annotation.shape else {
            return false;
        };
        let bounds = *bounds;
        let text = text.clone();
        let style = annotation.style;
        self.document.select(Some(id));
        self.text_edit = Some(TextEdit::edit_existing(id, bounds, text, style));
        self.style = style;
        self.status = None;
        cx.notify();
        true
    }

    fn preview_shape(&self) -> Option<(Shape, AnnotationStyle)> {
        let draft = self.draft.as_ref()?;
        finalize_shape(draft, draft.current).map(|shape| (shape, self.effective_style()))
    }

    fn commit_text(&mut self, cx: &mut Context<Self>) {
        let Some(mut edit) = self.text_edit.take() else {
            return;
        };
        let text = edit.content.trim_end().to_owned();
        if text.is_empty() {
            if let Some(id) = edit.target {
                self.document.remove(id);
                self.status = None;
            }
            cx.notify();
            return;
        }
        edit.content = text.clone();
        let bounds = edit.finish_bounds();
        let shape = Shape::Text { bounds, text };
        if let Some(id) = edit.target {
            self.document.update_shape(id, shape, edit.style);
        } else {
            self.document.add(shape, edit.style);
        }
        self.style = edit.style;
        self.status = None;
        cx.notify();
    }

    fn cancel_text(&mut self, cx: &mut Context<Self>) {
        if self.text_edit.take().is_some() {
            cx.notify();
        }
    }

    fn backspace_text(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        let selection = edit.selection();
        let range = edit.marked.take().or_else(|| {
            if !selection.is_empty() {
                Some(selection)
            } else {
                let cursor = edit.cursor_utf8();
                (cursor > 0).then(|| edit.previous_boundary()..cursor)
            }
        });
        if let Some(range) = range {
            edit.content.replace_range(range.clone(), "");
            edit.cursor = range.start;
            edit.anchor = range.start;
        }
        cx.notify();
    }

    fn delete_text(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        let selection = edit.selection();
        let range = if !selection.is_empty() {
            Some(selection)
        } else {
            let cursor = edit.cursor_utf8();
            (cursor < edit.content.len()).then(|| cursor..edit.next_boundary())
        };
        if let Some(range) = range {
            edit.content.replace_range(range.clone(), "");
            edit.cursor = range.start;
            edit.anchor = range.start;
        }
        cx.notify();
    }

    fn move_text_cursor(&mut self, cursor: usize, extend: bool, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.text_edit {
            edit.move_cursor(cursor, extend);
            cx.notify();
        }
    }

    fn select_all_text(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.text_edit {
            edit.anchor = 0;
            edit.cursor = edit.content.len();
            cx.notify();
        }
    }

    fn copy_text_selection(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        let selection = edit.selection();
        if selection.is_empty() {
            return;
        }
        let text = edit.content[selection].to_owned();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status = Some(t(settings_state::get().language, "editor.copied").to_owned());
        cx.notify();
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let language = settings_state::get().language;
        let text = rgb(TEXT);
        let muted = rgb(MUTED);
        let accent = self.accent;
        let accent_selected = self.accent_selected;
        let hover = rgba(HOVER);
        let border = rgba(BORDER);

        let mut tools = Vec::new();
        for tool in Tool::ALL {
            let selected = self.tool == tool;
            let label = t(language, tool.label_key()).to_owned();
            let button = div()
                .id(tool.id())
                .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |button| button.bg(accent_selected))
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.tool = tool;
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    tool.icon(),
                    18.0,
                    if selected { accent } else { muted },
                ));
            tools.push(button.into_any_element());
        }

        let mut swatches = Vec::new();
        for (color, name, label_key) in settings_state::ANNOTATION_COLORS {
            let selected = (self.style.color & 0xFFFFFF00) == (color & 0xFFFFFF00);
            let label = t(language, label_key).to_owned();
            let swatch = div()
                .id(name)
                .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                .w(px(18.0))
                .h(px(18.0))
                .rounded(px(9.0))
                .bg(rgb((color >> 8) & 0x00FF_FFFF))
                .border_2()
                .border_color(if selected {
                    rgb(0xFFFFFF)
                } else {
                    rgba(0x00000000)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.update_style(|style| {
                        style.color = (color & 0xFFFFFF00) | (style.color & 0xFF);
                    });
                    this.status = None;
                    cx.notify();
                }));
            swatches.push(swatch.into_any_element());
        }

        let mut widths = Vec::new();
        for width in STROKE_WIDTHS {
            let selected = (self.style.stroke_width - width).abs() < 0.1;
            let label = format!("{width:.0}");
            let button = div()
                .id(stroke_id(width))
                .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |button| button.bg(accent_selected))
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.update_style(|style| style.stroke_width = width);
                    this.status = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .w(px(14.0))
                        .h(px(width))
                        .rounded(px(width / 2.0))
                        .bg(if selected { accent } else { muted }),
                );
            widths.push(button.into_any_element());
        }

        let undo_label = t(language, "editor.undo").to_owned();
        let undo = if self.document.can_undo() {
            div()
                .id("editor-undo")
                .tooltip(move |_, cx| tooltip_view(undo_label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.document.undo();
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(icons::UNDO, 18.0, muted))
                .into_any_element()
        } else {
            div()
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.35)
                .child(icons::icon(icons::UNDO, 18.0, muted))
                .into_any_element()
        };

        let redo_label = t(language, "editor.redo").to_owned();
        let redo = if self.document.can_redo() {
            div()
                .id("editor-redo")
                .tooltip(move |_, cx| tooltip_view(redo_label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.document.redo();
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(icons::REDO, 18.0, muted))
                .into_any_element()
        } else {
            div()
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.35)
                .child(icons::icon(icons::REDO, 18.0, muted))
                .into_any_element()
        };

        let delete_label = t(language, "editor.delete").to_owned();
        let delete = if self.document.selected().is_some() {
            div()
                .id("editor-delete")
                .tooltip(move |_, cx| tooltip_view(delete_label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.commit_text(cx);
                    this.delete_selected_annotation(cx);
                }))
                .child(icons::icon(icons::TRASH, 18.0, muted))
                .into_any_element()
        } else {
            div()
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.35)
                .child(icons::icon(icons::TRASH, 18.0, muted))
                .into_any_element()
        };

        let pin_label = t(language, "editor.pin").to_owned();
        let pin = div()
            .id("editor-pin")
            .tooltip(move |_, cx| tooltip_view(pin_label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(|this, _, _, cx| this.pin(cx)))
            .child(icons::icon(icons::PIN, 18.0, text))
            .into_any_element();

        let copy_label = t(language, "editor.copy").to_owned();
        let copy = div()
            .id("editor-copy")
            .tooltip(move |_, cx| tooltip_view(copy_label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(|this, _, _, cx| this.export(true, cx)))
            .child(icons::icon(icons::COPY, 18.0, text))
            .into_any_element();

        let save_label = t(language, "editor.save").to_owned();
        let save = div()
            .id("editor-save")
            .tooltip(move |_, cx| tooltip_view(save_label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(|this, _, _, cx| this.export(false, cx)))
            .child(icons::icon(icons::SAVE, 18.0, text))
            .into_any_element();

        let close_label = t(language, "editor.close").to_owned();
        let close = div()
            .id("editor-close")
            .tooltip(move |_, cx| tooltip_view(close_label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(|_, _, window, _| window.remove_window()))
            .child(icons::icon(icons::CLOSE, 18.0, muted))
            .into_any_element();

        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .w_full()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(12.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .bg(rgb(TOOLBAR_BG))
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .children(tools),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .children(swatches)
                    .child(div().w(px(1.0)).h(px(18.0)).bg(border))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.0))
                            .children(widths),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(undo)
                    .child(redo)
                    .child(delete)
                    .child(div().w(px(6.0)))
                    .child(pin)
                    .child(copy)
                    .child(save)
                    .child(close),
            )
    }

    fn pin(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        match rasterize(&self.document, &self.frame) {
            Ok(output) => crate::pin::open_frame(output, cx),
            Err(error) => {
                let language = settings_state::get().language;
                self.status = Some(format!("{}{}", t(language, "editor.export_failed"), error));
            }
        }
        cx.notify();
    }

    fn export(&mut self, copy_only: bool, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let language = settings_state::get().language;
        let mut messages = Vec::new();
        match rasterize(&self.document, &self.frame) {
            Ok(output) => {
                crate::pin::remember(&output);
                if copy_only || settings_state::get().copy_to_clipboard {
                    match image_util::copy_png(&output, cx) {
                        Ok(()) => messages.push(t(language, "editor.copied").to_owned()),
                        Err(error) => {
                            messages.push(format!("{}{}", t(language, "editor.copy_failed"), error))
                        }
                    }
                }
                if !copy_only {
                    if let Some(pin_id) = self.pin_target {
                        if crate::pin::replace_image(pin_id, &output, cx) {
                            messages.push(t(language, "editor.pin_updated").to_owned());
                        } else {
                            messages.push(t(language, "editor.pin_missing").to_owned());
                        }
                    } else {
                        match image_util::save_png(&output) {
                            Ok(path) => messages.push(format!(
                                "{} {}",
                                t(language, "editor.saved"),
                                path.display()
                            )),
                            Err(error) => messages.push(format!(
                                "{} {}",
                                t(language, "editor.save_failed"),
                                error
                            )),
                        }
                    }
                }
            }
            Err(error) => {
                messages.push(format!("{}{}", t(language, "editor.export_failed"), error))
            }
        }
        self.status = Some(messages.join(" · "));
        cx.notify();
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
            edit.marked = None;
            edit.replace(range, new_text);
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
            edit.replace(range, new_text);
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
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let bounds = self.text_edit_screen?;
        let local_x = (f32::from(point.x - bounds.origin.x)).max(0.0);
        let local_y = (f32::from(point.y - bounds.origin.y)).max(0.0);
        Some(edit.offset_to_utf16(edit.index_at_local(
            local_x,
            local_y,
            f32::from(bounds.size.width),
        )))
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

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let image_rect = self.image_view_rect(window);
        let scale = f32::from(image_rect.size.width) / self.image_size.width as f32;
        self.text_edit_screen = self.text_edit.as_ref().map(|edit| Bounds {
            origin: point(
                image_rect.origin.x + px(edit.bounds.origin.x as f32 * scale),
                image_rect.origin.y + px(edit.bounds.origin.y as f32 * scale),
            ),
            size: size(
                px((edit.bounds.size.width as f32 * scale).max(24.0)),
                px((edit.bounds.size.height as f32 * scale).max(12.0)),
            ),
        });
        let mut objects = self.document.objects().to_vec();
        if let Some(move_draft) = &self.move_draft {
            let dx = move_draft.current.x - move_draft.start.x;
            let dy = move_draft.current.y - move_draft.start.y;
            if (dx != 0 || dy != 0)
                && let Some(annotation) = objects
                    .iter_mut()
                    .find(|annotation| annotation.id == move_draft.id)
            {
                *annotation = annotation.clone().translated(dx, dy);
            }
        }
        let selected_bounds = self
            .document
            .selected()
            .and_then(|id| objects.iter().find(|annotation| annotation.id == id))
            .map(|annotation| annotation.bounds());
        let preview = self.preview_shape();
        let status = self.status.clone();
        let accent_hex = self.accent_hex;

        // Build one layer per annotation in document order. Rendering every
        // vector shape in a single canvas put all of them above text and
        // numbers, so the preview no longer matched the exported z-order.
        let mut object_layers: Vec<AnyElement> = Vec::with_capacity(objects.len());
        for annotation in &objects {
            match &annotation.shape {
                Shape::Blur(rect) => {
                    if rect.is_empty() {
                        continue;
                    }
                    let left = rect.origin.x as f32 * scale;
                    let top = rect.origin.y as f32 * scale;
                    let width = px(rect.size.width as f32 * scale);
                    let height = px(rect.size.height as f32 * scale);
                    object_layers.push(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .w(width)
                            .h(height)
                            .overflow_hidden()
                            .child(
                                img(self.blur_image.clone())
                                    .absolute()
                                    .left(px(-left))
                                    .top(px(-top))
                                    .w(image_rect.size.width)
                                    .h(image_rect.size.height)
                                    .object_fit(ObjectFit::Fill),
                            )
                            .into_any_element(),
                    );
                }
                Shape::Text { bounds, text } => {
                    object_layers.push(
                        div()
                            .absolute()
                            .left(px(bounds.origin.x as f32 * scale))
                            .top(px(bounds.origin.y as f32 * scale))
                            .w(px(bounds.size.width as f32 * scale))
                            .text_size(px(annotation.style.font_size * scale))
                            .text_color(rgb((annotation.style.color >> 8) & 0x00FF_FFFF))
                            .child(text.clone())
                            .into_any_element(),
                    );
                }
                Shape::Number { bounds, value } => {
                    let color = (annotation.style.color >> 8) & 0x00FF_FFFF;
                    let foreground = if prefers_dark_foreground(color) {
                        rgb(0x111111)
                    } else {
                        rgb(0xFFFFFF)
                    };
                    let width = bounds.size.width as f32 * scale;
                    let height = bounds.size.height as f32 * scale;
                    object_layers.push(
                        div()
                            .absolute()
                            .left(px(bounds.origin.x as f32 * scale))
                            .top(px(bounds.origin.y as f32 * scale))
                            .w(px(width))
                            .h(px(height))
                            .rounded(px(height.min(width) / 2.0))
                            .bg(rgb(color))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px((height * 0.54).max(8.0)))
                            .font_weight(FontWeight::SEMIBOLD)
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

        let overlay_canvas = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if let Some((shape, style)) = &preview {
                    paint_shape(window, bounds, scale, shape, *style);
                }
                if let Some(rect) = selected_bounds {
                    paint_selection(window, bounds, scale, rect, accent_hex);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let text_overlay = self.text_edit.as_ref().map(|edit| {
            let left = edit.origin.x as f32 * scale;
            let top = edit.origin.y as f32 * scale;
            let font_size = (edit.style.font_size * scale).max(8.0);
            let width = (edit.bounds.size.width as f32 * scale).max(24.0);
            let color = edit.style.color;
            let entity = cx.entity();
            let focus = self.focus_handle.clone();
            let input = canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    window.handle_input(
                        &focus,
                        ElementInputHandler::new(bounds, entity.clone()),
                        cx,
                    );
                },
            )
            .absolute()
            .inset_0();
            let cursor = edit.cursor_utf8();
            let text = if edit.content.is_empty() {
                "|".to_owned()
            } else {
                format!("{}|{}", &edit.content[..cursor], &edit.content[cursor..])
            };
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .px(px(3.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(rgba(0x00000080))
                .border_1()
                .border_color(rgba((accent_hex << 8) | 0xB3))
                .text_size(px(font_size))
                .text_color(rgb((color >> 8) & 0x00FF_FFFF))
                .child(text)
                .child(input)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                        if let (Some(bounds), Some(edit)) =
                            (this.text_edit_screen, this.text_edit.as_ref())
                        {
                            let local_x = f32::from(event.position.x - bounds.origin.x).max(0.0);
                            let local_y = f32::from(event.position.y - bounds.origin.y).max(0.0);
                            let index =
                                edit.index_at_local(local_x, local_y, f32::from(bounds.size.width));
                            if let Some(edit) = this.text_edit.as_mut() {
                                edit.move_cursor(index, false);
                            }
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        });

        let image_layer = div()
            .absolute()
            .left(image_rect.origin.x)
            .top(image_rect.origin.y)
            .w(image_rect.size.width)
            .h(image_rect.size.height)
            .child(
                img(self.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .children(object_layers)
            .child(overlay_canvas)
            .children(text_overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let Some(point) = this.to_image_point(event.position, window) else {
                        return;
                    };
                    if this.text_edit.is_some() {
                        this.commit_text(cx);
                    }
                    if this.tool == Tool::Select {
                        if event.click_count >= 2 && this.edit_text_at(point, cx) {
                            let focus = this.focus_handle.clone();
                            window.focus(&focus, cx);
                            return;
                        }
                        let selected = this.document.hit_test(point);
                        this.select_annotation(selected);
                        this.move_draft = selected.map(|id| MoveDraft {
                            id,
                            start: point,
                            current: point,
                        });
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    if this.tool == Tool::Number {
                        let max_left = (this.image_size.width.saturating_sub(BADGE_SIZE)) as i32;
                        let max_top = (this.image_size.height.saturating_sub(BADGE_SIZE)) as i32;
                        let left = (point.x - BADGE_SIZE as i32 / 2).clamp(0, max_left);
                        let top = (point.y - BADGE_SIZE as i32 / 2).clamp(0, max_top);
                        let value = this.document.next_number();
                        this.document.add(
                            Shape::Number {
                                bounds: PhysicalRect::new(left, top, BADGE_SIZE, BADGE_SIZE),
                                value,
                            },
                            this.effective_style(),
                        );
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    if this.tool == Tool::Text {
                        this.text_edit = Some(TextEdit::start(
                            point,
                            this.image_size,
                            this.effective_style(),
                        ));
                        this.status = None;
                        let focus = this.focus_handle.clone();
                        window.focus(&focus, cx);
                        cx.notify();
                        return;
                    }
                    this.draft = Some(Draft {
                        tool: this.tool,
                        start: point,
                        current: point,
                        points: vec![point],
                    });
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let Some(point) = this.to_image_point(event.position, window) else {
                    return;
                };
                if let Some(move_draft) = &mut this.move_draft {
                    move_draft.current = point;
                    cx.notify();
                    return;
                }
                let Some(draft) = &mut this.draft else {
                    return;
                };
                draft.current = point;
                if draft.tool == Tool::Highlighter {
                    let last = draft.points.last().copied().unwrap_or(point);
                    if distance(last, point) >= 2.0 {
                        draft.points.push(point);
                    }
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if let Some(move_draft) = this.move_draft.take() {
                        let point = this
                            .to_image_point(event.position, window)
                            .unwrap_or(move_draft.current);
                        let dx = point.x - move_draft.start.x;
                        let dy = point.y - move_draft.start.y;
                        if dx != 0 || dy != 0 {
                            this.document.move_selected(dx, dy);
                            this.status = None;
                        }
                        cx.notify();
                        return;
                    }

                    let Some(draft) = this.draft.take() else {
                        return;
                    };
                    let point = this
                        .to_image_point(event.position, window)
                        .unwrap_or(draft.current);
                    if let Some(shape) = finalize_shape(&draft, point) {
                        this.document.add(shape, this.effective_style());
                        this.status = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_exit(cx.listener(|this, _, _, cx| {
                if this.draft.take().is_some() || this.move_draft.take().is_some() {
                    cx.notify();
                }
            }));

        let status_toast = status.map(|message| {
            div()
                .absolute()
                .bottom(px(16.0))
                .right(px(16.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(rgba(0x000000CC))
                .text_size(px(12.0))
                .text_color(rgb(0xFFFFFF))
                .child(message)
        });

        div()
            .relative()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let command =
                    event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
                if this.text_edit.is_some() {
                    let extend = event.keystroke.modifiers.shift;
                    match key {
                        "escape" => {
                            this.cancel_text(cx);
                            return;
                        }
                        "enter" => {
                            this.commit_text(cx);
                            return;
                        }
                        "backspace" => {
                            this.backspace_text(cx);
                            return;
                        }
                        "delete" => {
                            this.delete_text(cx);
                            return;
                        }
                        "left" => {
                            let position =
                                this.text_edit.as_ref().map(|edit| edit.previous_boundary());
                            if let Some(position) = position {
                                this.move_text_cursor(position, extend, cx);
                            }
                            return;
                        }
                        "right" => {
                            let position = this.text_edit.as_ref().map(|edit| edit.next_boundary());
                            if let Some(position) = position {
                                this.move_text_cursor(position, extend, cx);
                            }
                            return;
                        }
                        "home" => {
                            this.move_text_cursor(0, extend, cx);
                            return;
                        }
                        "end" => {
                            let end = this.text_edit.as_ref().map(|edit| edit.content.len());
                            if let Some(end) = end {
                                this.move_text_cursor(end, extend, cx);
                            }
                            return;
                        }
                        _ => {}
                    }
                    if command && key == "a" {
                        this.select_all_text(cx);
                        return;
                    }
                    if command && key == "c" {
                        this.copy_text_selection(cx);
                        return;
                    }
                    if command && key == "s" {
                        this.commit_text(cx);
                        this.export(false, cx);
                        return;
                    }
                    // While editing, swallow document-level shortcuts so
                    // Cmd/Ctrl+Z cannot undo an object out from under the
                    // in-flight text edit.
                    return;
                }
                if key == "escape" {
                    window.remove_window();
                    return;
                }
                if key == "delete" || key == "backspace" {
                    this.delete_selected_annotation(cx);
                    return;
                }
                if command && key == "c" {
                    if !this.copy_selected_annotation(cx) {
                        this.export(true, cx);
                    }
                    return;
                }
                if command && key == "v" {
                    this.paste_annotation(cx);
                    return;
                }
                if command && key == "d" {
                    if let Some(id) = this.document.selected()
                        && let Some(pasted) = this.document.duplicate(id, 16, 16)
                    {
                        if let Some(annotation) = this.document.object(pasted) {
                            this.style = annotation.style;
                        }
                        this.tool = Tool::Select;
                        this.status = Some(
                            t(settings_state::get().language, "editor.annotation_pasted")
                                .to_owned(),
                        );
                        cx.notify();
                    }
                    return;
                }
                if command && key == "s" {
                    this.export(false, cx);
                    return;
                }
                if command && key == "z" {
                    if event.keystroke.modifiers.shift {
                        this.document.redo();
                    } else {
                        this.document.undo();
                    }
                    this.status = None;
                    cx.notify();
                }
            }))
            .child(self.toolbar(cx))
            .child(image_layer)
            .children(status_toast)
            .child(
                div()
                    .absolute()
                    .left(px(12.0))
                    .bottom(px(8.0))
                    .text_size(px(10.5))
                    .text_color(rgba(0xFFFFFF66))
                    .child(format!(
                        "{} x {}",
                        self.image_size.width, self.image_size.height
                    )),
            )
    }
}

pub fn open_editor(frame: RgbaFrame, cx: &mut App) {
    open_editor_with_pin(frame, None, cx);
}

pub fn open_pin_editor(frame: RgbaFrame, pin_id: u64, cx: &mut App) {
    open_editor_with_pin(frame, Some(pin_id), cx);
}

fn open_editor_with_pin(frame: RgbaFrame, pin_target: Option<u64>, cx: &mut App) {
    crate::pin::remember(&frame);
    let image_width = frame.width() as f32;
    let image_height = frame.height() as f32;
    let scale = (MAX_IMAGE_WIDTH / image_width)
        .min(MAX_IMAGE_HEIGHT / image_height)
        .min(1.0);
    let content_width = (image_width * scale).max(320.0);
    let content_height = (image_height * scale).max(240.0);
    let window_width = (content_width + PADDING * 2.0).max(MIN_WINDOW_WIDTH);
    let window_height = (content_height + TOOLBAR_HEIGHT + PADDING * 2.0).max(480.0);
    let bounds = crate::ui::centered_bounds(cx, window_width, window_height);
    let display_id = cx.primary_display().map(|display| display.id());
    let language = settings_state::get().language;

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(t(language, "editor.title").into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        display_id,
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(360.0))),
        app_id: Some("capin-editor".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, move |window, cx| {
        let view = cx.new(|cx| EditorView::new(frame, pin_target, cx));
        let focus = view.read(cx).focus_handle.clone();
        window.defer(cx, move |window, cx| {
            window.focus(&focus, cx);
            window.activate_window();
        });
        view
    }) {
        Ok((handle, _)) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
        }
        Err(error) => eprintln!("capin: failed to open editor: {error}"),
    }
}

fn finalize_shape(draft: &Draft, end: PhysicalPoint) -> Option<Shape> {
    match draft.tool {
        Tool::Rect => {
            let rect = normalized_rect(draft.start, end);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::Rect(rect))
        }
        Tool::Ellipse => {
            let rect = normalized_rect(draft.start, end);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::Ellipse(rect))
        }
        Tool::Blur => {
            let rect = normalized_rect(draft.start, end);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::Blur(rect))
        }
        Tool::Spotlight => {
            let rect = normalized_rect(draft.start, end);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::Spotlight(rect))
        }
        Tool::Arrow => (distance(draft.start, end) >= 2.0).then_some(Shape::Arrow {
            start: draft.start,
            end,
        }),
        Tool::Highlighter => {
            let mut points = draft.points.clone();
            if points.last().is_none_or(|last| distance(*last, end) >= 2.0) {
                points.push(end);
            }
            (points.len() >= 2).then_some(Shape::Highlighter { points })
        }
        Tool::Text | Tool::Select | Tool::Number => None,
    }
}

fn normalized_rect(start: PhysicalPoint, end: PhysicalPoint) -> PhysicalRect {
    let left = start.x.min(end.x);
    let top = start.y.min(end.y);
    let right = start.x.max(end.x);
    let bottom = start.y.max(end.y);
    PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32)
}

fn distance(a: PhysicalPoint, b: PhysicalPoint) -> f32 {
    let dx = (a.x - b.x) as f32;
    let dy = (a.y - b.y) as f32;
    (dx * dx + dy * dy).sqrt()
}

fn paint_selection(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scale: f32,
    rect: PhysicalRect,
    accent_hex: u32,
) {
    let left = rect.origin.x as f32;
    let top = rect.origin.y as f32;
    let right = left + rect.size.width as f32;
    let bottom = top + rect.size.height as f32;
    let stroke = px((1.5 * scale).max(1.0));
    let to_point = |x: f32, y: f32| {
        point(
            bounds.origin.x + px(x * scale),
            bounds.origin.y + px(y * scale),
        )
    };
    let mut builder = PathBuilder::stroke(stroke);
    builder.move_to(to_point(left, top));
    builder.line_to(to_point(right, top));
    builder.line_to(to_point(right, bottom));
    builder.line_to(to_point(left, bottom));
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, rgba((accent_hex << 8) | 0xFF));
    }
}

fn paint_shape(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scale: f32,
    shape: &Shape,
    style: AnnotationStyle,
) {
    let color = rgba(style.color);
    let stroke = px((style.stroke_width * scale).max(1.0));
    let to_point = |x: f32, y: f32| {
        point(
            bounds.origin.x + px(x * scale),
            bounds.origin.y + px(y * scale),
        )
    };

    match shape {
        Shape::Rect(rect) => {
            let left = rect.origin.x as f32;
            let top = rect.origin.y as f32;
            let right = left + rect.size.width as f32;
            let bottom = top + rect.size.height as f32;
            let mut builder = PathBuilder::stroke(stroke);
            builder.move_to(to_point(left, top));
            builder.line_to(to_point(right, top));
            builder.line_to(to_point(right, bottom));
            builder.line_to(to_point(left, bottom));
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Ellipse(rect) => {
            let center_x = rect.origin.x as f32 + rect.size.width as f32 / 2.0;
            let center_y = rect.origin.y as f32 + rect.size.height as f32 / 2.0;
            let radius_x = rect.size.width as f32 / 2.0;
            let radius_y = rect.size.height as f32 / 2.0;
            let mut builder = PathBuilder::stroke(stroke);
            builder.move_to(to_point(center_x + radius_x, center_y));
            builder.arc_to(
                point(px(radius_x * scale), px(radius_y * scale)),
                px(0.0),
                false,
                false,
                to_point(center_x - radius_x, center_y),
            );
            builder.arc_to(
                point(px(radius_x * scale), px(radius_y * scale)),
                px(0.0),
                false,
                false,
                to_point(center_x + radius_x, center_y),
            );
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Arrow { start, end } => {
            let mut builder = PathBuilder::stroke(stroke);
            builder.move_to(to_point(start.x as f32, start.y as f32));
            builder.line_to(to_point(end.x as f32, end.y as f32));

            let dx = (end.x - start.x) as f32;
            let dy = (end.y - start.y) as f32;
            let angle = dy.atan2(dx);
            let head = (style.stroke_width * 3.5).max(10.0);
            let spread = 0.55;
            let left = (
                end.x as f32 - head * (angle - spread).cos(),
                end.y as f32 - head * (angle - spread).sin(),
            );
            let right = (
                end.x as f32 - head * (angle + spread).cos(),
                end.y as f32 - head * (angle + spread).sin(),
            );
            builder.move_to(to_point(left.0, left.1));
            builder.line_to(to_point(end.x as f32, end.y as f32));
            builder.line_to(to_point(right.0, right.1));
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Highlighter { points } => {
            let Some(first) = points.first() else {
                return;
            };
            let mut builder = PathBuilder::stroke(stroke);
            builder.move_to(to_point(first.x as f32, first.y as f32));
            for next in &points[1..] {
                builder.line_to(to_point(next.x as f32, next.y as f32));
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Blur(_) | Shape::Text { .. } | Shape::Number { .. } => {}
        Shape::Spotlight(rect) => paint_spotlight(window, bounds, scale, *rect),
    }
}

fn paint_spotlight(window: &mut Window, bounds: Bounds<Pixels>, scale: f32, rect: PhysicalRect) {
    let image_left = f32::from(bounds.origin.x);
    let image_top = f32::from(bounds.origin.y);
    let image_right = image_left + f32::from(bounds.size.width);
    let image_bottom = image_top + f32::from(bounds.size.height);
    let hole_left = (image_left + rect.origin.x as f32 * scale).clamp(image_left, image_right);
    let hole_top = (image_top + rect.origin.y as f32 * scale).clamp(image_top, image_bottom);
    let hole_right = (hole_left + rect.size.width as f32 * scale).clamp(hole_left, image_right);
    let hole_bottom = (hole_top + rect.size.height as f32 * scale).clamp(hole_top, image_bottom);

    let pieces = [
        (image_left, image_top, image_right, hole_top),
        (image_left, hole_bottom, image_right, image_bottom),
        (image_left, hole_top, hole_left, hole_bottom),
        (hole_right, hole_top, image_right, hole_bottom),
    ];
    for (left, top, right, bottom) in pieces {
        if right <= left || bottom <= top {
            continue;
        }
        let mut builder = PathBuilder::fill();
        builder.add_polygon(
            &[
                point(px(left), px(top)),
                point(px(right), px(top)),
                point(px(right), px(bottom)),
                point(px(left), px(bottom)),
            ],
            true,
        );
        if let Ok(path) = builder.build() {
            window.paint_path(path, rgba(0x0000008C));
        }
    }
}
