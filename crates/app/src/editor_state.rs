use std::ops::Range;

use core_annotate::{AnnotationDocument, AnnotationStyle, Shape, TextAlignment};
use core_types::{PhysicalPoint, PhysicalRect, PhysicalSize};

use crate::effects::RegionEffects;
use crate::icons;
use crate::selection_geometry::ResizeHandle as RegionHandle;
use crate::settings_state::{self, AnnotationColorTool};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Select,
    Rect,
    Arrow,
    Highlighter,
    Blur,
    Text,
    Number,
    Spotlight,
}

impl Tool {
    pub(crate) const ALL: [Self; 7] = [
        Self::Rect,
        Self::Arrow,
        Self::Highlighter,
        Self::Blur,
        Self::Text,
        Self::Number,
        Self::Spotlight,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Select => "editor-tool-select",
            Self::Rect => "editor-tool-rect",
            Self::Arrow => "editor-tool-arrow",
            Self::Highlighter => "editor-tool-highlighter",
            Self::Blur => "editor-tool-blur",
            Self::Text => "editor-tool-text",
            Self::Number => "editor-tool-number",
            Self::Spotlight => "editor-tool-spotlight",
        }
    }

    pub(crate) fn icon(self) -> &'static [u8] {
        match self {
            Self::Select => icons::SELECT,
            Self::Rect => icons::RECT,
            Self::Arrow => icons::ARROW,
            Self::Highlighter => icons::HIGHLIGHTER,
            Self::Blur => icons::BLUR,
            Self::Text => icons::TEXT,
            Self::Number => icons::NUMBER,
            Self::Spotlight => icons::SPOTLIGHT,
        }
    }

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Select => "editor.tool.select",
            Self::Rect => "editor.tool.rect",
            Self::Arrow => "editor.tool.arrow",
            Self::Highlighter => "editor.tool.highlighter",
            Self::Blur => "editor.tool.blur",
            Self::Text => "editor.tool.text",
            Self::Number => "editor.tool.number",
            Self::Spotlight => "editor.tool.spotlight",
        }
    }

    pub(crate) fn color_tool(self) -> Option<AnnotationColorTool> {
        match self {
            Self::Rect => Some(AnnotationColorTool::Rect),
            Self::Arrow => Some(AnnotationColorTool::Arrow),
            Self::Highlighter => Some(AnnotationColorTool::Highlighter),
            Self::Text => Some(AnnotationColorTool::Text),
            Self::Number => Some(AnnotationColorTool::Number),
            Self::Select | Self::Blur | Self::Spotlight => None,
        }
    }

    pub(crate) fn from_shape(shape: &Shape) -> Option<Self> {
        match shape {
            Shape::Rect(_) | Shape::Ellipse(_) => Some(Self::Rect),
            Shape::Arrow { .. } | Shape::Polyline { .. } => Some(Self::Arrow),
            Shape::HighlighterBox(_) => Some(Self::Highlighter),
            Shape::Blur(_) => Some(Self::Blur),
            Shape::Text { .. } => Some(Self::Text),
            Shape::Number { .. } => Some(Self::Number),
            Shape::Spotlight(_) => Some(Self::Spotlight),
        }
    }
}

pub(crate) struct Draft {
    pub(crate) tool: Tool,
    pub(crate) start: PhysicalPoint,
    pub(crate) current: PhysicalPoint,
    pub(crate) shift: bool,
    pub(crate) ellipse: bool,
}

pub(crate) struct MoveDraft {
    pub(crate) id: core_annotate::AnnotationId,
    pub(crate) start: PhysicalPoint,
    pub(crate) current: PhysicalPoint,
    pub(crate) original: Shape,
    pub(crate) recorded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResizeHandle {
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    ArrowStart,
    ArrowEnd,
}

pub(crate) struct ResizeDraft {
    pub(crate) id: core_annotate::AnnotationId,
    pub(crate) handle: ResizeHandle,
    pub(crate) start: PhysicalPoint,
    pub(crate) current: PhysicalPoint,
    pub(crate) original: Shape,
    pub(crate) recorded: bool,
}

pub(crate) struct CropDraft {
    pub(crate) start: PhysicalPoint,
    pub(crate) current: PhysicalPoint,
}

pub(crate) struct PolylineDraft {
    pub(crate) points: Vec<PhysicalPoint>,
    pub(crate) current: PhysicalPoint,
}

#[derive(Clone, Copy)]
pub(crate) struct RegionAdjustDraft {
    pub(crate) handle: RegionHandle,
    pub(crate) original: PhysicalRect,
    pub(crate) current: PhysicalRect,
}

#[derive(Clone, Copy)]
pub(crate) struct FullCanvas {
    pub(crate) adjustable: bool,
}

pub(crate) struct EditorSnapshot {
    pub(crate) frame: std::sync::Arc<core_types::RgbaFrame>,
    pub(crate) image_size: PhysicalSize,
    pub(crate) document: AnnotationDocument,
}

pub(crate) enum HistoryEntry {
    Document,
    Canvas(EditorSnapshot),
    ExpiredCanvas,
}

pub(crate) enum HistoryStep {
    Document,
    Canvas(EditorSnapshot),
}

pub(crate) struct TextEdit {
    pub(crate) target: Option<core_annotate::AnnotationId>,
    pub(crate) origin: PhysicalPoint,
    pub(crate) bounds: PhysicalRect,
    pub(crate) content: String,
    pub(crate) cursor: usize,
    pub(crate) anchor: usize,
    pub(crate) marked: Option<Range<usize>>,
    pub(crate) style: AnnotationStyle,
    pub(crate) selecting: bool,
}

impl TextEdit {
    pub(crate) fn start(
        origin: PhysicalPoint,
        image_size: PhysicalSize,
        style: AnnotationStyle,
    ) -> Self {
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
            selecting: false,
        }
    }

    pub(crate) fn edit_existing(
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
            selecting: false,
        }
    }

    pub(crate) fn cursor_utf8(&self) -> usize {
        self.cursor.min(self.content.len())
    }

    pub(crate) fn selection(&self) -> Range<usize> {
        let cursor = self.cursor_utf8();
        let anchor = self.anchor.min(self.content.len());
        if anchor <= cursor {
            anchor..cursor
        } else {
            cursor..anchor
        }
    }

    pub(crate) fn move_cursor(&mut self, cursor: usize, extend_selection: bool) {
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

    pub(crate) fn previous_boundary(&self) -> usize {
        previous_char_boundary(&self.content, self.cursor_utf8())
    }

    pub(crate) fn next_boundary(&self) -> usize {
        next_char_boundary(&self.content, self.cursor_utf8())
    }

    pub(crate) fn offset_from_utf16(&self, offset: usize) -> usize {
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

    pub(crate) fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = offset.min(self.content.len());
        self.content[..offset]
            .chars()
            .map(|ch| ch.len_utf16())
            .sum()
    }

    pub(crate) fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_from_utf16(range.start);
        let end = self.offset_from_utf16(range.end).max(start);
        start..end
    }

    pub(crate) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_to_utf16(range.start);
        let end = self.offset_to_utf16(range.end).max(start);
        start..end
    }

    pub(crate) fn replace(&mut self, range: Option<Range<usize>>, text: &str, mark: bool) {
        let selection = self.selection();
        let range = range
            .or_else(|| self.marked.take())
            .or_else(|| (!selection.is_empty()).then_some(selection))
            .unwrap_or(self.cursor_utf8()..self.cursor_utf8());
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.content.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.marked = (mark && !text.is_empty()).then_some(range.start..range.start + text.len());
    }

    pub(crate) fn finish_bounds(&self) -> PhysicalRect {
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

pub(crate) fn previous_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = cursor.min(content.len());
    let mut boundary = 0;
    for (index, _) in content.char_indices() {
        if index >= cursor {
            break;
        }
        boundary = index;
    }
    boundary
}

pub(crate) fn next_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = cursor.min(content.len());
    content
        .char_indices()
        .find(|(index, ch)| *index + ch.len_utf8() > cursor)
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(cursor)
}

pub(crate) struct EditorState {
    pub(crate) document: AnnotationDocument,
    pub(crate) image_size: PhysicalSize,
    pub(crate) tool: Tool,
    pub(crate) style: AnnotationStyle,
    pub(crate) font_size: f32,
    pub(crate) arrow_start: bool,
    pub(crate) arrow_end: bool,
    pub(crate) arrow_polyline: bool,
    pub(crate) text_background: bool,
    pub(crate) text_outline: bool,
    pub(crate) text_alignment: TextAlignment,
    pub(crate) ui_scale: f32,
    pub(crate) corner_radius: f32,
    pub(crate) highlighter_radius: f32,
    pub(crate) number_size: u32,
    pub(crate) spotlight_radius: f32,
    pub(crate) rect_stroke_width: f32,
    pub(crate) arrow_stroke_width: f32,
    pub(crate) blur_radius: u32,
    pub(crate) rect_filled: bool,
    pub(crate) ellipse_filled: bool,
    pub(crate) shape_ellipse: bool,
    pub(crate) spotlight_intensity: u8,
    pub(crate) active_color_tool: AnnotationColorTool,
    pub(crate) draft: Option<Draft>,
    pub(crate) move_draft: Option<MoveDraft>,
    pub(crate) resize_draft: Option<ResizeDraft>,
    pub(crate) crop_mode: bool,
    pub(crate) crop_draft: Option<CropDraft>,
    pub(crate) polyline_draft: Option<PolylineDraft>,
    pub(crate) undo: Vec<HistoryEntry>,
    pub(crate) redo: Vec<HistoryEntry>,
    pub(crate) annotation_clipboard: Option<core_annotate::Annotation>,
    pub(crate) custom_color_open: bool,
    pub(crate) custom_color_buffer: String,
    pub(crate) custom_color_fresh: bool,
    pub(crate) text_edit: Option<TextEdit>,
    pub(crate) effects: RegionEffects,
    pub(crate) effects_open: bool,
    pub(crate) more_open: bool,
    pub(crate) toolbar_hidden: bool,
    pub(crate) hints_visible: bool,
    pub(crate) status: Option<String>,
    pub(crate) status_serial: u64,
    pub(crate) region_drag: Option<RegionAdjustDraft>,
    pub(crate) full_canvas: Option<FullCanvas>,
}

impl EditorState {
    #[cfg(test)]
    pub(crate) fn for_test(document: AnnotationDocument, image_size: PhysicalSize) -> Self {
        Self {
            document,
            image_size,
            tool: Tool::Select,
            style: AnnotationStyle::default(),
            font_size: 20.0,
            arrow_start: false,
            arrow_end: true,
            arrow_polyline: false,
            text_background: false,
            text_outline: false,
            text_alignment: TextAlignment::Left,
            ui_scale: 1.0,
            corner_radius: 0.0,
            highlighter_radius: 0.0,
            number_size: 32,
            spotlight_radius: 0.0,
            rect_stroke_width: 3.0,
            arrow_stroke_width: 3.0,
            blur_radius: 16,
            rect_filled: false,
            ellipse_filled: false,
            shape_ellipse: false,
            spotlight_intensity: 40,
            active_color_tool: AnnotationColorTool::Rect,
            draft: None,
            move_draft: None,
            resize_draft: None,
            crop_mode: false,
            crop_draft: None,
            polyline_draft: None,
            undo: Vec::new(),
            redo: Vec::new(),
            annotation_clipboard: None,
            custom_color_open: false,
            custom_color_buffer: String::new(),
            custom_color_fresh: false,
            text_edit: None,
            effects: RegionEffects::default(),
            effects_open: false,
            more_open: false,
            toolbar_hidden: false,
            hints_visible: false,
            status: None,
            status_serial: 0,
            region_drag: None,
            full_canvas: None,
        }
    }
}

impl EditorState {
    pub(crate) fn effective_style(&self) -> AnnotationStyle {
        let mut style = self.style;
        if let Some(tool) = self.tool.color_tool() {
            style.color = (self.tool_color(tool) & 0xFFFFFF00) | (style.color & 0xFF);
        }
        match self.tool {
            Tool::Rect => style.stroke_width = self.rect_stroke_width,
            Tool::Arrow => style.stroke_width = self.arrow_stroke_width,
            _ => {}
        }
        if self.tool == Tool::Highlighter {
            style.color = (style.color & 0xFFFFFF00) | core_annotate::HIGHLIGHTER_ALPHA;
            style.stroke_width = 12.0;
        }
        style.filled = self.tool == Tool::Rect
            && if self.shape_ellipse {
                self.ellipse_filled
            } else {
                self.rect_filled
            };
        if self.tool == Tool::Rect && style.filled {
            style.color |= 0x000000FF;
        }
        if self.tool == Tool::Text {
            style.font_size = self.font_size;
            style.text_background = self.text_background;
            style.text_outline = self.text_outline;
            style.text_alignment = self.text_alignment;
        }
        if self.tool == Tool::Arrow {
            style.arrow_start = self.arrow_start;
            style.arrow_end = self.arrow_end;
        }
        match self.tool {
            Tool::Rect => style.corner_radius = self.corner_radius * self.ui_scale,
            Tool::Highlighter => {
                style.highlighter_radius = self.highlighter_radius * self.ui_scale;
            }
            Tool::Blur => style.blur_radius = self.blur_radius,
            Tool::Spotlight => {
                style.spotlight_intensity = self.spotlight_intensity;
                style.spotlight_radius = self.spotlight_radius * self.ui_scale;
            }
            _ => {}
        }
        style
    }

    pub(crate) fn tool_color(&self, tool: AnnotationColorTool) -> u32 {
        settings_state::get().annotation_color_for(tool)
    }

    pub(crate) fn palette_color_tool(&self) -> AnnotationColorTool {
        if let Some(tool) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .and_then(|annotation| Tool::from_shape(&annotation.shape))
            .and_then(Tool::color_tool)
        {
            return tool;
        }
        self.tool.color_tool().unwrap_or(self.active_color_tool)
    }

    pub(crate) fn config_tool(&self) -> Option<Tool> {
        if let Some(tool) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .and_then(|annotation| Tool::from_shape(&annotation.shape))
        {
            return Some(tool);
        }
        (self.tool != Tool::Select).then_some(self.tool)
    }

    pub(crate) fn config_shape_ellipse(&self) -> bool {
        if let Some(shape) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| &annotation.shape)
        {
            return matches!(shape, Shape::Ellipse(_));
        }
        self.shape_ellipse
    }

    pub(crate) fn config_arrow_polyline(&self) -> bool {
        if let Some(shape) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| &annotation.shape)
        {
            return matches!(shape, Shape::Polyline { .. });
        }
        self.arrow_polyline
    }

    pub(crate) fn palette_color(&self) -> u32 {
        self.document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style.color)
            .unwrap_or_else(|| self.tool_color(self.palette_color_tool()))
    }

    pub(crate) fn open_custom_color(&mut self) {
        self.custom_color_buffer = format!("{:06x}", (self.palette_color() >> 8) & 0x00FF_FFFF);
        self.custom_color_fresh = true;
        self.custom_color_open = true;
    }

    pub(crate) fn close_custom_color(&mut self) {
        self.custom_color_open = false;
        self.custom_color_fresh = false;
    }

    pub(crate) fn apply_stroke_width(&mut self, width: f32) {
        self.update_style(|style| style.stroke_width = width);
        match self.config_tool() {
            Some(Tool::Rect) => {
                self.rect_stroke_width = width;
                settings_state::update(|settings| settings.annotation_rect_stroke_width = width);
            }
            Some(Tool::Arrow) => {
                self.arrow_stroke_width = width;
                settings_state::update(|settings| settings.annotation_arrow_stroke_width = width);
            }
            _ => {}
        }
    }

    pub(crate) fn apply_font_size(&mut self, size: f32) {
        self.font_size = size;
        self.update_style(|style| style.font_size = size);
        if let Some(edit) = &mut self.text_edit {
            edit.style.font_size = size;
        }
        settings_state::update(|settings| settings.annotation_font_size = size);
    }

    pub(crate) fn apply_text_background(&mut self, enabled: bool) {
        self.text_background = enabled;
        self.update_style(|style| style.text_background = enabled);
        if let Some(edit) = &mut self.text_edit {
            edit.style.text_background = enabled;
        }
        settings_state::update(|settings| settings.annotation_text_background = enabled);
    }

    pub(crate) fn apply_text_outline(&mut self, enabled: bool) {
        self.text_outline = enabled;
        self.update_style(|style| style.text_outline = enabled);
        if let Some(edit) = &mut self.text_edit {
            edit.style.text_outline = enabled;
        }
        settings_state::update(|settings| settings.annotation_text_outline = enabled);
    }

    pub(crate) fn apply_text_alignment(&mut self, alignment: TextAlignment) {
        self.text_alignment = alignment;
        self.update_style(|style| style.text_alignment = alignment);
        if let Some(edit) = &mut self.text_edit {
            edit.style.text_alignment = alignment;
        }
        settings_state::update(|settings| settings.annotation_text_alignment = alignment);
    }

    pub(crate) fn apply_arrow_heads(&mut self, start: bool, end: bool) {
        self.arrow_start = start;
        self.arrow_end = end;
        self.update_style(|style| {
            style.arrow_start = start;
            style.arrow_end = end;
        });
        settings_state::update(|settings| {
            settings.annotation_arrow_start = start;
            settings.annotation_arrow_end = end;
        });
    }

    pub(crate) fn apply_arrow_polyline(&mut self, polyline: bool) {
        self.arrow_polyline = polyline;
        self.polyline_draft = None;
        settings_state::update(|settings| settings.annotation_arrow_polyline = polyline);
    }

    pub(crate) fn apply_corner_radius(&mut self, radius: f32) {
        self.corner_radius = radius;
        let applied = radius * self.ui_scale;
        self.update_style(|style| style.corner_radius = applied);
        settings_state::update(|settings| settings.annotation_corner_radius = radius);
    }

    pub(crate) fn apply_highlighter_radius(&mut self, radius: f32) {
        self.highlighter_radius = radius;
        let applied = radius * self.ui_scale;
        self.update_style(|style| style.highlighter_radius = applied);
        settings_state::update(|settings| settings.annotation_highlighter_radius = radius);
    }

    pub(crate) fn apply_number_size(&mut self, size: u32) {
        self.number_size = size;
        settings_state::update(|settings| settings.annotation_number_size = size);
        let Some(id) = self.document.selected() else {
            return;
        };
        let Some(annotation) = self.document.object(id).cloned() else {
            return;
        };
        let Shape::Number { bounds, value } = annotation.shape else {
            return;
        };
        let side = ((size as f32 * self.ui_scale).round() as u32).max(8);
        let centre_x = bounds.origin.x + bounds.size.width as i32 / 2;
        let centre_y = bounds.origin.y + bounds.size.height as i32 / 2;
        let left = (centre_x - side as i32 / 2)
            .clamp(0, self.image_size.width.saturating_sub(side) as i32);
        let top = (centre_y - side as i32 / 2)
            .clamp(0, self.image_size.height.saturating_sub(side) as i32);
        let shape = Shape::Number {
            bounds: PhysicalRect::new(left, top, side, side),
            value,
        };
        self.edit_document(|document| document.update_shape(id, shape, annotation.style));
    }

    pub(crate) fn apply_spotlight_radius(&mut self, radius: f32) {
        self.spotlight_radius = radius;
        let applied = radius * self.ui_scale;
        self.update_style(|style| style.spotlight_radius = applied);
        settings_state::update(|settings| settings.annotation_spotlight_radius = radius);
    }

    pub(crate) fn apply_blur_radius(&mut self, radius: u32) {
        self.blur_radius = radius;
        self.update_style(|style| style.blur_radius = radius);
        settings_state::update(|settings| settings.annotation_blur_radius = radius);
    }

    pub(crate) fn apply_fill(&mut self, filled: bool) {
        let Some(tool) = self.config_tool() else {
            return;
        };
        match tool {
            Tool::Rect => {
                if self.config_shape_ellipse() {
                    self.ellipse_filled = filled;
                    settings_state::update(|settings| settings.annotation_ellipse_filled = filled);
                } else {
                    self.rect_filled = filled;
                    settings_state::update(|settings| settings.annotation_rect_filled = filled);
                }
            }
            _ => return,
        }
        self.update_style(|style| style.filled = filled);
    }

    pub(crate) fn apply_shape_ellipse(&mut self, ellipse: bool) {
        self.shape_ellipse = ellipse;
        settings_state::update(|settings| settings.annotation_rect_ellipse = ellipse);
    }

    pub(crate) fn apply_spotlight_intensity(&mut self, intensity: u8) {
        let intensity = intensity.min(100);
        self.spotlight_intensity = intensity;
        self.update_style(|style| style.spotlight_intensity = intensity);
        settings_state::update(|settings| settings.annotation_spotlight_intensity = intensity);
    }

    pub(crate) fn apply_color(&mut self, color: u32) {
        let tool = self.palette_color_tool();
        let alpha = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style.color & 0xFF)
            .unwrap_or_else(|| {
                if tool == AnnotationColorTool::Highlighter {
                    core_annotate::HIGHLIGHTER_ALPHA
                } else {
                    self.style.color & 0xFF
                }
            });
        let applied = (color & 0xFFFFFF00) | alpha;
        self.style.color = (self.style.color & 0xFFFFFF00) | alpha;

        if let Some(mut style) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style)
        {
            style.color = applied;
            self.edit_document(|document| document.update_selected_style(style));
            self.style = style;
        }

        settings_state::update(|settings| settings.apply_annotation_color(tool, color));
    }

    pub(crate) fn update_style(&mut self, update: impl FnOnce(&mut AnnotationStyle)) {
        let selected_style = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style);
        if let Some(mut style) = selected_style {
            update(&mut style);
            self.edit_document(|document| document.update_selected_style(style));
            self.style = style;
            return;
        }
        update(&mut self.style);
    }

    pub(crate) fn select_annotation(&mut self, id: Option<core_annotate::AnnotationId>) {
        let annotation = id.and_then(|id| self.document.object(id)).cloned();
        self.close_custom_color();
        self.document.select(id);
        if let Some(annotation) = annotation {
            self.style = annotation.style;
            if let Some(tool) = Tool::from_shape(&annotation.shape) {
                if let Some(color_tool) = tool.color_tool() {
                    self.active_color_tool = color_tool;
                }
                match tool {
                    Tool::Rect => {
                        self.shape_ellipse = matches!(&annotation.shape, Shape::Ellipse(_));
                        self.corner_radius = annotation.style.corner_radius / self.ui_scale;
                        if self.shape_ellipse {
                            self.ellipse_filled = annotation.style.filled;
                        } else {
                            self.rect_filled = annotation.style.filled;
                        }
                    }
                    Tool::Text => {
                        self.font_size = annotation.style.font_size;
                        self.text_background = annotation.style.text_background;
                        self.text_outline = annotation.style.text_outline;
                        self.text_alignment = annotation.style.text_alignment;
                    }
                    Tool::Highlighter => {
                        self.highlighter_radius =
                            annotation.style.highlighter_radius / self.ui_scale;
                    }
                    Tool::Number => {
                        if let Shape::Number { bounds, .. } = &annotation.shape {
                            self.number_size =
                                ((bounds.size.width as f32 / self.ui_scale).round() as u32).max(1);
                        }
                    }
                    Tool::Arrow => {
                        self.arrow_start = annotation.style.arrow_start;
                        self.arrow_end = annotation.style.arrow_end;
                        self.arrow_polyline = matches!(&annotation.shape, Shape::Polyline { .. });
                    }
                    Tool::Blur => self.blur_radius = annotation.style.blur_radius,
                    Tool::Spotlight => {
                        self.spotlight_intensity = annotation.style.spotlight_intensity;
                        self.spotlight_radius = annotation.style.spotlight_radius / self.ui_scale;
                    }
                    _ => {}
                }
            }
        } else {
            self.style.stroke_width = match self.tool {
                Tool::Rect => self.rect_stroke_width,
                Tool::Arrow => self.arrow_stroke_width,
                _ => self.style.stroke_width,
            };
        }
    }

    pub(crate) fn update_move(&mut self, point: PhysicalPoint) -> bool {
        let Some(draft) = self.move_draft.as_mut() else {
            return false;
        };
        if point == draft.current {
            return false;
        }
        draft.current = point;
        let dx = point.x - draft.start.x;
        let dy = point.y - draft.start.y;
        if dx == 0 && dy == 0 && !draft.recorded {
            return false;
        }
        let shape = draft.original.clone().translated(dx, dy);
        let record = !draft.recorded;
        draft.recorded = true;
        let id = draft.id;
        self.edit_document(|document| document.update_shape_live(id, shape, record))
    }

    pub(crate) fn update_resize(&mut self, point: PhysicalPoint) -> bool {
        let Some(draft) = self.resize_draft.as_mut() else {
            return false;
        };
        if point == draft.current {
            return false;
        }
        draft.current = point;
        if point == draft.start && !draft.recorded {
            return false;
        }
        let shape = resize_shape(&draft.original, draft.handle, draft.start, point, 4);
        let record = !draft.recorded;
        draft.recorded = true;
        let id = draft.id;
        self.edit_document(|document| document.update_shape_live(id, shape, record))
    }

    pub(crate) fn preview_shape(&self) -> Option<(Shape, AnnotationStyle)> {
        if let Some(draft) = &self.polyline_draft {
            let mut points = draft.points.clone();
            if distance(*points.last().unwrap_or(&draft.current), draft.current) > 2.0 {
                points.push(draft.current);
            }
            if points.len() >= 2 {
                return Some((Shape::Polyline { points }, self.effective_style()));
            }
            return None;
        }
        let draft = self.draft.as_ref()?;
        finalize_shape(draft, draft.current).map(|shape| (shape, self.effective_style()))
    }
}

pub(crate) fn finalize_shape(draft: &Draft, end: PhysicalPoint) -> Option<Shape> {
    match draft.tool {
        Tool::Rect => {
            let rect = constrained_rect(draft.start, end, draft.shift);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(if draft.ellipse {
                Shape::Ellipse(rect)
            } else {
                Shape::Rect(rect)
            })
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
            let rect = normalized_rect(draft.start, end);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::HighlighterBox(rect))
        }
        Tool::Text | Tool::Select | Tool::Number => None,
    }
}

pub(crate) fn normalized_rect(start: PhysicalPoint, end: PhysicalPoint) -> PhysicalRect {
    let left = start.x.min(end.x);
    let top = start.y.min(end.y);
    let right = start.x.max(end.x);
    let bottom = start.y.max(end.y);
    PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32)
}

pub(crate) fn constrained_rect(
    start: PhysicalPoint,
    end: PhysicalPoint,
    square: bool,
) -> PhysicalRect {
    if !square {
        return normalized_rect(start, end);
    }
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let side = dx.abs().max(dy.abs());
    let sign_x = if dx == 0 { 1 } else { dx.signum() };
    let sign_y = if dy == 0 { 1 } else { dy.signum() };
    let corner = PhysicalPoint::new(start.x + side * sign_x, start.y + side * sign_y);
    normalized_rect(start, corner)
}

pub(crate) fn distance(a: PhysicalPoint, b: PhysicalPoint) -> f32 {
    let dx = (a.x - b.x) as f32;
    let dy = (a.y - b.y) as f32;
    (dx * dx + dy * dy).sqrt()
}

pub(crate) fn handle_points(shape: &Shape) -> Vec<(ResizeHandle, PhysicalPoint)> {
    if let Shape::Polyline { .. } = shape {
        return Vec::new();
    }
    if let Shape::Arrow { start, end } = shape {
        return vec![
            (ResizeHandle::ArrowStart, *start),
            (ResizeHandle::ArrowEnd, *end),
        ];
    }
    let rect = shape.bounds();
    let left = rect.origin.x;
    let top = rect.origin.y;
    let right = left + rect.size.width as i32;
    let bottom = top + rect.size.height as i32;
    let mid_x = left + (right - left) / 2;
    let mid_y = top + (bottom - top) / 2;
    vec![
        (ResizeHandle::Nw, PhysicalPoint::new(left, top)),
        (ResizeHandle::N, PhysicalPoint::new(mid_x, top)),
        (ResizeHandle::Ne, PhysicalPoint::new(right, top)),
        (ResizeHandle::E, PhysicalPoint::new(right, mid_y)),
        (ResizeHandle::Se, PhysicalPoint::new(right, bottom)),
        (ResizeHandle::S, PhysicalPoint::new(mid_x, bottom)),
        (ResizeHandle::Sw, PhysicalPoint::new(left, bottom)),
        (ResizeHandle::W, PhysicalPoint::new(left, mid_y)),
    ]
}

pub(crate) fn hit_resize_handle(
    shape: &Shape,
    point: PhysicalPoint,
    tolerance: f32,
) -> Option<ResizeHandle> {
    let tolerance_sq = tolerance * tolerance;
    handle_points(shape)
        .into_iter()
        .filter_map(|(handle, handle_point)| {
            let dx = (handle_point.x - point.x) as f32;
            let dy = (handle_point.y - point.y) as f32;
            let distance_sq = dx * dx + dy * dy;
            (distance_sq <= tolerance_sq).then_some((handle, distance_sq))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(handle, _)| handle)
}

pub(crate) const SELECTION_HANDLE_HALF: f32 = 5.5;

pub(crate) const REGION_HANDLE_SIZE: f32 = 9.0;
pub(crate) const HANDLE_HIT: f32 = 18.0;
pub(crate) const BAND_HALF: f32 = 6.0;

pub(crate) fn annotation_handle_at(
    shape: &Shape,
    point: PhysicalPoint,
    scale: f32,
) -> Option<ResizeHandle> {
    let scale = scale.max(0.01);
    let bounds = shape.bounds();
    let min_pt = (bounds.size.width.min(bounds.size.height) as f32) * scale;
    if !matches!(
        shape,
        Shape::Arrow { .. } | Shape::Polyline { .. } | Shape::Number { .. }
    ) && min_pt < SELECTION_HANDLE_HALF * 4.0
        && closed_shape_contains(shape, point, 1.0 / scale)
    {
        return None;
    }
    hit_resize_handle(shape, point, SELECTION_HANDLE_HALF / scale)
}

pub(crate) fn closed_shape_contains(shape: &Shape, point: PhysicalPoint, inset: f32) -> bool {
    match shape {
        Shape::Arrow { .. } | Shape::Polyline { .. } => false,
        _ => {
            let rect = shape.bounds();
            let left = rect.origin.x as f32 + inset;
            let top = rect.origin.y as f32 + inset;
            let right = rect.right() as f32 - inset;
            let bottom = rect.bottom() as f32 - inset;
            left < right
                && top < bottom
                && (point.x as f32) >= left
                && (point.y as f32) >= top
                && (point.x as f32) <= right
                && (point.y as f32) <= bottom
        }
    }
}

pub(crate) fn resize_shape(
    original: &Shape,
    handle: ResizeHandle,
    start: PhysicalPoint,
    current: PhysicalPoint,
    min_size: i32,
) -> Shape {
    if let Shape::Arrow {
        start: original_start,
        end: original_end,
    } = original
    {
        return Shape::Arrow {
            start: if handle == ResizeHandle::ArrowStart {
                current
            } else {
                *original_start
            },
            end: if handle == ResizeHandle::ArrowEnd {
                current
            } else {
                *original_end
            },
        };
    }

    let rect = original.bounds();
    let mut left = rect.origin.x;
    let mut top = rect.origin.y;
    let mut right = left + rect.size.width as i32;
    let mut bottom = top + rect.size.height as i32;
    let dx = current.x - start.x;
    let dy = current.y - start.y;
    match handle {
        ResizeHandle::Nw => {
            left += dx;
            top += dy;
        }
        ResizeHandle::N => top += dy,
        ResizeHandle::Ne => {
            right += dx;
            top += dy;
        }
        ResizeHandle::E => right += dx,
        ResizeHandle::Se => {
            right += dx;
            bottom += dy;
        }
        ResizeHandle::S => bottom += dy,
        ResizeHandle::Sw => {
            left += dx;
            bottom += dy;
        }
        ResizeHandle::W => left += dx,
        ResizeHandle::ArrowStart | ResizeHandle::ArrowEnd => {}
    }
    if right - left < min_size {
        if matches!(
            handle,
            ResizeHandle::Nw | ResizeHandle::W | ResizeHandle::Sw
        ) {
            left = right - min_size;
        } else {
            right = left + min_size;
        }
    }
    if bottom - top < min_size {
        if matches!(
            handle,
            ResizeHandle::Nw | ResizeHandle::N | ResizeHandle::Ne
        ) {
            top = bottom - min_size;
        } else {
            bottom = top + min_size;
        }
    }
    if matches!(original, Shape::Number { .. }) {
        let width = right - left;
        let height = bottom - top;
        let (size, anchor_x, anchor_y) = match handle {
            ResizeHandle::N | ResizeHandle::S => {
                (height.max(min_size), Some((left + right) / 2), None)
            }
            ResizeHandle::E | ResizeHandle::W => {
                (width.max(min_size), None, Some((top + bottom) / 2))
            }
            _ => (width.max(height).max(min_size), None, None),
        };
        if let Some(centre) = anchor_x {
            left = centre - size / 2;
            right = left + size;
        } else if matches!(
            handle,
            ResizeHandle::Nw | ResizeHandle::W | ResizeHandle::Sw
        ) {
            left = right - size;
        } else {
            right = left + size;
        }
        if let Some(centre) = anchor_y {
            top = centre - size / 2;
            bottom = top + size;
        } else if matches!(
            handle,
            ResizeHandle::Nw | ResizeHandle::N | ResizeHandle::Ne
        ) {
            top = bottom - size;
        } else {
            bottom = top + size;
        }
    }
    let rect = PhysicalRect::new(
        left,
        top,
        (right - left).max(0) as u32,
        (bottom - top).max(0) as u32,
    );
    match original {
        Shape::Rect(_) => Shape::Rect(rect),
        Shape::Ellipse(_) => Shape::Ellipse(rect),
        Shape::HighlighterBox(_) => Shape::HighlighterBox(rect),
        Shape::Blur(_) => Shape::Blur(rect),
        Shape::Spotlight(_) => Shape::Spotlight(rect),
        Shape::Text { text, .. } => Shape::Text {
            bounds: rect,
            text: text.clone(),
        },
        Shape::Number { value, .. } => Shape::Number {
            bounds: rect,
            value: *value,
        },
        Shape::Arrow { .. } => unreachable!("arrow handled above"),
        Shape::Polyline { .. } => original.clone(),
    }
}

impl EditorState {
    pub(crate) fn finish_polyline(&mut self) -> bool {
        let Some(draft) = self.polyline_draft.take() else {
            return false;
        };
        let mut points = draft.points;
        if distance(*points.last().unwrap_or(&draft.current), draft.current) > 2.0 {
            points.push(draft.current);
        }
        if points.len() >= 2 {
            let style = self.effective_style();
            self.edit_document(|document| document.add(Shape::Polyline { points }, style));
        }
        self.status = None;
        true
    }

    pub(crate) fn cancel_polyline(&mut self) {
        self.polyline_draft = None;
        self.status = None;
    }

    pub(crate) fn finish_resize(&mut self, point: PhysicalPoint) -> bool {
        if self.resize_draft.is_none() {
            return false;
        }
        self.update_resize(point);
        self.resize_draft = None;
        self.status = None;
        true
    }

    pub(crate) fn edit_text_at(&mut self, point: PhysicalPoint) -> bool {
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
        true
    }

    pub(crate) fn commit_text(&mut self) -> bool {
        let Some(mut edit) = self.text_edit.take() else {
            return false;
        };
        if let Some(range) = edit.marked.take()
            && range.end <= edit.content.len()
        {
            edit.content.replace_range(range.clone(), "");
            edit.cursor = range.start;
            edit.anchor = range.start;
        }
        let text = edit.content.trim_end().to_owned();
        if text.is_empty() {
            if let Some(id) = edit.target {
                self.remove_annotation(id);
                self.status = None;
            }
            return true;
        }
        edit.content = text.clone();
        let bounds = edit.finish_bounds();
        let shape = Shape::Text { bounds, text };
        if let Some(id) = edit.target {
            self.edit_document(|document| document.update_shape(id, shape, edit.style));
        } else {
            self.add_annotation(shape, edit.style);
        }
        self.style = edit.style;
        self.status = None;
        true
    }

    pub(crate) fn cancel_text(&mut self) -> bool {
        self.text_edit.take().is_some()
    }

    pub(crate) fn backspace_text(&mut self) -> bool {
        let Some(edit) = &mut self.text_edit else {
            return false;
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
        true
    }

    pub(crate) fn delete_text(&mut self) -> bool {
        let Some(edit) = &mut self.text_edit else {
            return false;
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
        true
    }

    pub(crate) fn move_text_cursor(&mut self, cursor: usize, extend: bool) -> bool {
        if let Some(edit) = &mut self.text_edit {
            edit.move_cursor(cursor, extend);
            true
        } else {
            false
        }
    }

    pub(crate) fn select_all_text(&mut self) -> bool {
        if let Some(edit) = &mut self.text_edit {
            edit.anchor = 0;
            edit.cursor = edit.content.len();
            true
        } else {
            false
        }
    }
}

pub(crate) const CANVAS_HISTORY_LIMIT: usize = 3;

const LARGE_CANVAS_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const HISTORY_LIMIT: usize = 128 + CANVAS_HISTORY_LIMIT;

impl EditorState {
    fn push_undo(&mut self, entry: HistoryEntry) {
        self.undo.push(entry);
        while self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    fn note_document_edit(&mut self) {
        self.push_undo(HistoryEntry::Document);
        self.redo.clear();
    }

    pub(crate) fn edit_document<T>(
        &mut self,
        edit: impl FnOnce(&mut AnnotationDocument) -> T,
    ) -> T {
        let serial = self.document.history_serial();
        let result = edit(&mut self.document);
        if self.document.history_serial() != serial {
            self.note_document_edit();
        }
        result
    }

    pub(crate) fn add_annotation(
        &mut self,
        shape: Shape,
        style: AnnotationStyle,
    ) -> core_annotate::AnnotationId {
        self.edit_document(|document| document.add(shape, style))
    }

    pub(crate) fn paste_annotation(
        &mut self,
        annotation: &core_annotate::Annotation,
        dx: i32,
        dy: i32,
    ) -> core_annotate::AnnotationId {
        self.edit_document(|document| document.paste(annotation, dx, dy))
    }

    pub(crate) fn remove_annotation(&mut self, id: core_annotate::AnnotationId) -> bool {
        self.edit_document(|document| document.remove(id))
    }

    pub(crate) fn record_canvas_snapshot(&mut self, snapshot: EditorSnapshot) {
        let bytes = snapshot.image_size.width as usize * snapshot.image_size.height as usize * 4;
        let limit = if bytes > LARGE_CANVAS_SNAPSHOT_BYTES {
            1
        } else {
            CANVAS_HISTORY_LIMIT
        };
        self.push_undo(HistoryEntry::Canvas(snapshot));
        let mut canvases = self
            .undo
            .iter()
            .filter(|entry| matches!(entry, HistoryEntry::Canvas(_)))
            .count();
        while canvases > limit {
            let Some(entry) = self
                .undo
                .iter_mut()
                .find(|entry| matches!(entry, HistoryEntry::Canvas(_)))
            else {
                break;
            };
            *entry = HistoryEntry::ExpiredCanvas;
            canvases -= 1;
        }
        self.redo.clear();
    }

    pub(crate) fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.document.clear_history();
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.undo.iter().any(|entry| match entry {
            HistoryEntry::Document => self.document.can_undo(),
            HistoryEntry::Canvas(_) => true,
            HistoryEntry::ExpiredCanvas => false,
        })
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.redo.iter().any(|entry| match entry {
            HistoryEntry::Document => self.document.can_redo(),
            HistoryEntry::Canvas(_) => true,
            HistoryEntry::ExpiredCanvas => false,
        })
    }

    pub(crate) fn take_undo(&mut self, current: EditorSnapshot) -> Option<HistoryStep> {
        let mut current = Some(current);
        loop {
            match self.undo.pop()? {
                HistoryEntry::Document => {
                    if self.document.undo() {
                        self.redo.push(HistoryEntry::Document);
                        return Some(HistoryStep::Document);
                    }
                }
                HistoryEntry::Canvas(snapshot) => {
                    if let Some(current) = current.take() {
                        self.redo.push(HistoryEntry::Canvas(current));
                    }
                    return Some(HistoryStep::Canvas(snapshot));
                }
                HistoryEntry::ExpiredCanvas => {}
            }
        }
    }

    pub(crate) fn take_redo(&mut self, current: EditorSnapshot) -> Option<HistoryStep> {
        let mut current = Some(current);
        loop {
            match self.redo.pop()? {
                HistoryEntry::Document => {
                    if self.document.redo() {
                        self.push_undo(HistoryEntry::Document);
                        return Some(HistoryStep::Document);
                    }
                }
                HistoryEntry::Canvas(snapshot) => {
                    if let Some(current) = current.take() {
                        self.push_undo(HistoryEntry::Canvas(current));
                    }
                    return Some(HistoryStep::Canvas(snapshot));
                }
                HistoryEntry::ExpiredCanvas => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "editor_state_tests.rs"]
mod tests;
