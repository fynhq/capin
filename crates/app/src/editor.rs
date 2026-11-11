use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use core_annotate::{
    Annotation, AnnotationDocument, AnnotationStyle, Shape, TextAlignment, rasterize,
};
use core_capture::{CaptureTarget, platform_backend};
use core_types::{
    CaptureRegion, MonitorId, PhysicalPoint, PhysicalRect, PhysicalSize, RgbaFrame,
    prefers_dark_foreground,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{
    WindowStyle, apply as apply_window_style, hide_native_border, system_accent_hex,
};

use crate::chrome_theme::ChromeTheme;
use crate::effects::RegionEffects;
use crate::logging::{log_error, log_info};
use crate::overlay::{
    ResizeHandle as RegionHandle, edge_resize_handle, handle_positions, resize_rect,
};
use crate::settings_state::AnnotationColorTool;
use crate::{i18n::t, icons, image_util, settings_state};

const COMPACT_TOOLBAR_HEIGHT: f32 = 40.0;
const COMPACT_TOOLBAR_WIDTH: f32 = 560.0;
const OPTIONS_PANEL_HEIGHT: f32 = 46.0;
const INFO_BAR_HEIGHT: f32 = 28.0;
const INFO_BAR_WIDTH: f32 = 360.0;
const TOOLBAR_SIZE: u32 = 30;

const STROKE_WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];
const FONT_SIZES: [f32; 4] = [14.0, 20.0, 28.0, 36.0];
const CORNER_RADII: [f32; 4] = [0.0, 8.0, 16.0, 24.0];
const NUMBER_SIZES: [u32; 4] = [24, 32, 40, 48];
const BLUR_RADII: [u32; 4] = [8, 16, 24, 32];
const SPOTLIGHT_INTENSITIES: [u8; 3] = [40, 60, 80];
const CANVAS_HISTORY_LIMIT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    /// Internal selection mode; clicking an existing object enters it without
    /// a dedicated toolbar button.
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

    fn from_shape(shape: &Shape) -> Option<Self> {
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

fn stroke_id(width: f32) -> &'static str {
    match width as u32 {
        2 => "editor-width-2",
        4 => "editor-width-4",
        _ => "editor-width-8",
    }
}

fn blur_image_index(radius: u32) -> usize {
    BLUR_RADII
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| candidate.abs_diff(radius))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

struct TooltipLabel {
    text: SharedString,
}

impl Render for TooltipLabel {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = ChromeTheme::for_window(window, None);
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(chrome.surface_strong)
            .border_1()
            .border_color(chrome.border)
            .shadow(chrome.shadow())
            .text_size(px(11.5))
            .text_color(chrome.text)
            .child(self.text.clone())
    }
}

pub(crate) fn tooltip_view(text: String, cx: &mut App) -> AnyView {
    cx.new(|_| TooltipLabel { text: text.into() }).into()
}

struct Draft {
    tool: Tool,
    start: PhysicalPoint,
    current: PhysicalPoint,
    /// Constrain rectangle-like tools to a square/circle while Shift is held.
    shift: bool,
    /// Rectangle tool drafts an ellipse.
    ellipse: bool,
}

struct MoveDraft {
    id: core_annotate::AnnotationId,
    start: PhysicalPoint,
    current: PhysicalPoint,
    original: Shape,
    /// The document records one undo snapshot for the whole drag. Until the
    /// first actual movement, no history entry is created.
    recorded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeHandle {
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

/// What the pointer is currently over inside the image, used to swap the
/// painted selection handles from the drawing crosshair to resize arrows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionHover {
    None,
    Body,
    Handle(ResizeHandle),
}

struct ResizeDraft {
    id: core_annotate::AnnotationId,
    handle: ResizeHandle,
    start: PhysicalPoint,
    current: PhysicalPoint,
    original: Shape,
    recorded: bool,
}

struct PanDraft {
    start: Point<Pixels>,
    start_pan: Point<Pixels>,
}

struct CropDraft {
    start: PhysicalPoint,
    current: PhysicalPoint,
}

struct PolylineDraft {
    points: Vec<PhysicalPoint>,
    current: PhysicalPoint,
}

thread_local! {
    /// Inline editors own full-frame textures but are not OverlayView
    /// instances. Track their window handles so app-driven close paths can
    /// release textures before the renderer is torn down.
    static EDITOR_VIEWS: std::cell::RefCell<Vec<(WeakEntity<EditorView>, AnyWindowHandle)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn register_view(view: &Entity<EditorView>, window: &Window) {
    let handle = window.window_handle();
    EDITOR_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|(weak, _)| weak.upgrade().is_some());
        views.push((view.downgrade(), handle));
    });
}

/// Releases editor textures for windows that are about to be removed. This
/// mirrors the overlay release path and closes the gap for standalone pin and
/// long-capture editors.
pub(crate) fn release_images_for_windows(
    cx: &mut App,
    handles: &[AnyWindowHandle],
    keep: Option<AnyWindowHandle>,
) {
    let entries: Vec<(Entity<EditorView>, AnyWindowHandle)> = EDITOR_VIEWS.with(|views| {
        views
            .borrow()
            .iter()
            .filter_map(|(weak, handle)| Some((weak.upgrade()?, *handle)))
            .collect()
    });
    for (view, handle) in entries {
        if keep.is_some_and(|keep| keep.window_id() == handle.window_id()) {
            continue;
        }
        if handles
            .iter()
            .any(|candidate| candidate.window_id() == handle.window_id())
        {
            view.update(cx, |editor, cx| editor.release_images(cx));
        }
    }
    EDITOR_VIEWS.with(|views| {
        views
            .borrow_mut()
            .retain(|(weak, _)| weak.upgrade().is_some());
    });
}

/// Pins the standalone inline editor (long capture) that owns the pin
/// shortcut. Overlay editors are handled by `overlay::pin_current` first;
/// pin editors have nothing new to pin.
pub(crate) fn has_open_editors() -> bool {
    EDITOR_VIEWS.with(|views| {
        views
            .borrow()
            .iter()
            .any(|(view, _)| view.upgrade().is_some())
    })
}

pub(crate) fn pin_current(cx: &mut App) -> bool {
    let views: Vec<Entity<EditorView>> = EDITOR_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|(weak, _)| weak.upgrade().is_some());
        views
            .iter()
            .filter_map(|(weak, _)| weak.upgrade())
            .collect()
    });
    for view in views.into_iter().rev() {
        let handled = view.update(cx, |editor, cx| {
            if editor.pin_target.is_some() {
                return false;
            }
            editor.pin(cx);
            true
        });
        if handled {
            return true;
        }
    }
    false
}

struct EditorSnapshot {
    frame: Arc<RgbaFrame>,
    image_size: PhysicalSize,
    document: AnnotationDocument,
}

/// Where an inline editor sits inside the full-screen capture overlay.
/// `image_rect` is already in the overlay window's logical coordinates, so the
/// editor can draw the annotated image exactly where the selection was instead
/// of opening a separate window.
#[derive(Clone, Copy)]
struct InlineHost {
    image_rect: Bounds<Pixels>,
    /// Logical origin of the surface that contains `image_rect`. Capture
    /// overlays use the monitor origin; pin/long-capture editors use their
    /// display origin, so the pin button can place output on the same screen.
    screen_origin: Point<Pixels>,
    capture: Option<InlineCapture>,
    /// Long captures fit the full image first and allow wheel zoom/pan;
    /// screenshot and pin editors must keep the captured pixel rect exact.
    zoomable: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct InlineCapture {
    pub(crate) region: CaptureRegion,
    pub(crate) target: CaptureTarget,
    pub(crate) scale: f32,
    pub(crate) monitor_origin: PhysicalPoint,
    pub(crate) effects: RegionEffects,
}

#[derive(Clone, Copy)]
struct RegionAdjustDraft {
    handle: RegionHandle,
    original: PhysicalRect,
    current: PhysicalRect,
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
    /// Whether the current pointer gesture is extending a text selection.
    selecting: bool,
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
            selecting: false,
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
            selecting: false,
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

    /// Replaces text as either an IME composition (`mark`) or committed text.
    ///
    /// A committed IME result must replace the in-flight composition instead
    /// of being appended after it, and only a composition may stay marked.
    fn replace(&mut self, range: Option<Range<usize>>, text: &str, mark: bool) {
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
        self.marked = (mark && !text.is_empty()).then_some(range.start..range.start + text.len());
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

/// One visual line produced by GPUI's text shaper. The editor and exporter use
/// different text engines, so hit-testing and the caret must follow the same
/// layout that the preview actually paints.
#[derive(Clone, Copy)]
struct TextLineBox {
    start: usize,
    end: usize,
    x: f32,
    width: f32,
    y: f32,
}

fn layout_annotation_text(
    window: &Window,
    text: &str,
    font_size: f32,
    width: f32,
) -> Option<WrappedLine> {
    if text.is_empty() || width <= 0.0 {
        return None;
    }
    let run = TextRun {
        len: text.len(),
        font: Font::default(),
        color: Hsla::from(rgb(0x000000FF)),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(
            SharedString::from(text.to_owned()),
            px(font_size.max(1.0)),
            &[run],
            Some(px(width)),
            None,
        )
        .ok()?
        .into_iter()
        .next()
}

fn text_line_boxes(layout: &WrappedLine, line_height: f32) -> Vec<TextLineBox> {
    let mut boxes = Vec::new();
    let mut start = 0usize;
    let mut start_x = 0.0f32;
    for boundary in layout.wrap_boundaries() {
        let run = &layout.runs()[boundary.run_ix];
        let glyph = &run.glyphs[boundary.glyph_ix];
        let end = glyph.index;
        let end_x = f32::from(glyph.position.x);
        boxes.push(TextLineBox {
            start,
            end,
            x: start_x,
            width: (end_x - start_x).max(0.0),
            y: boxes.len() as f32 * line_height,
        });
        start = end;
        start_x = end_x;
    }
    let end_x = f32::from(layout.unwrapped_layout.width);
    boxes.push(TextLineBox {
        start,
        end: layout.len(),
        x: start_x,
        width: (end_x - start_x).max(0.0),
        y: boxes.len() as f32 * line_height,
    });
    boxes
}

fn text_alignment_offset(alignment: TextAlignment, container_width: f32, line_width: f32) -> f32 {
    let remaining = (container_width - line_width).max(0.0);
    match alignment {
        TextAlignment::Left => 0.0,
        TextAlignment::Center => remaining / 2.0,
        TextAlignment::Right => remaining,
    }
}

fn text_caret_position(
    layout: Option<&WrappedLine>,
    boxes: &[TextLineBox],
    edit: &TextEdit,
    container_width: f32,
) -> Point<Pixels> {
    let cursor = edit.cursor_utf8();
    let Some(line) = boxes
        .iter()
        .find(|line| cursor <= line.end)
        .or_else(|| boxes.last())
    else {
        return point(
            px(text_alignment_offset(
                edit.style.text_alignment,
                container_width,
                0.0,
            )),
            px(0.0),
        );
    };
    let x = layout.map_or(0.0, |layout| {
        (f32::from(layout.unwrapped_layout.x_for_index(cursor)) - line.x).clamp(0.0, line.width)
    });
    let offset = text_alignment_offset(edit.style.text_alignment, container_width, line.width);
    point(px(x + offset), px(line.y))
}

fn text_selection_boxes(
    layout: &WrappedLine,
    boxes: &[TextLineBox],
    selection: &Range<usize>,
    alignment: TextAlignment,
    line_height: f32,
    container_width: f32,
) -> Vec<Bounds<Pixels>> {
    let mut rects = Vec::new();
    for line in boxes {
        let start = selection.start.max(line.start);
        let end = selection.end.min(line.end);
        if start >= end {
            continue;
        }
        let x1 =
            (f32::from(layout.unwrapped_layout.x_for_index(start)) - line.x).clamp(0.0, line.width);
        let x2 =
            (f32::from(layout.unwrapped_layout.x_for_index(end)) - line.x).clamp(0.0, line.width);
        if x2 <= x1 {
            continue;
        }
        let offset = text_alignment_offset(alignment, container_width, line.width);
        rects.push(Bounds {
            origin: point(px(x1 + offset), px(line.y)),
            size: size(px(x2 - x1), px(line_height)),
        });
    }
    rects
}

fn word_range_at(content: &str, index: usize) -> Range<usize> {
    let mut index = index.min(content.len());
    while index > 0 && !content.is_char_boundary(index) {
        index -= 1;
    }
    if content.is_empty() {
        return 0..0;
    }
    let mut start = if index == content.len() {
        previous_char_boundary(content, index)
    } else {
        index
    };
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let word = content[start..].chars().next().is_some_and(is_word);
    while start > 0 {
        let previous = previous_char_boundary(content, start);
        let ch = content[previous..start].chars().next().unwrap_or_default();
        if is_word(ch) == word {
            start = previous;
        } else {
            break;
        }
    }
    let mut end = if index == content.len() {
        content.len()
    } else {
        next_char_boundary(content, index)
    };
    while end < content.len() {
        let next = next_char_boundary(content, end);
        let ch = content[end..next].chars().next().unwrap_or_default();
        if is_word(ch) == word {
            end = next;
        } else {
            break;
        }
    }
    start..end
}

pub(crate) struct EditorView {
    image: Arc<RenderImage>,
    blur_images: [Option<Arc<RenderImage>>; BLUR_RADII.len()],
    images_released: bool,
    frame: Arc<RgbaFrame>,
    /// Frozen monitor image behind an inline screenshot editor. Present only
    /// for plain region captures, where the crop can be widened or shrunk
    /// again while no annotation object is selected. The BGRA render image is
    /// shared with the overlay, so widening a selection does not keep a second
    /// full RGBA monitor frame alive.
    region_source: Option<Arc<RenderImage>>,
    region_drag: Option<RegionAdjustDraft>,
    pin_target: Option<u64>,
    document: AnnotationDocument,
    image_size: PhysicalSize,
    tool: Tool,
    style: AnnotationStyle,
    font_size: f32,
    arrow_start: bool,
    arrow_end: bool,
    arrow_polyline: bool,
    text_background: bool,
    text_outline: bool,
    text_alignment: TextAlignment,
    /// Image pixels per logical point for the surface hosting this editor.
    /// Preset sizes (corner radius, badge size, spotlight radius) are stored
    /// in logical points and multiplied by this value when applied.
    ui_scale: f32,
    corner_radius: f32,
    number_size: u32,
    spotlight_radius: f32,
    /// Independent defaults for the two width-bearing tool families. The
    /// selected object's own width is shown and edited through `style`.
    rect_stroke_width: f32,
    arrow_stroke_width: f32,
    blur_radius: u32,
    rect_filled: bool,
    ellipse_filled: bool,
    shape_ellipse: bool,
    spotlight_intensity: u8,
    active_color_tool: AnnotationColorTool,
    draft: Option<Draft>,
    move_draft: Option<MoveDraft>,
    resize_draft: Option<ResizeDraft>,
    selection_hover: SelectionHover,
    pan_draft: Option<PanDraft>,
    view_zoom: f32,
    view_pan: Point<Pixels>,
    space_down: bool,
    crop_mode: bool,
    crop_draft: Option<CropDraft>,
    polyline_draft: Option<PolylineDraft>,
    canvas_undo: Vec<EditorSnapshot>,
    canvas_redo: Vec<EditorSnapshot>,
    annotation_clipboard: Option<Annotation>,
    custom_color_open: bool,
    custom_color_buffer: String,
    custom_color_fresh: bool,
    text_edit: Option<TextEdit>,
    text_edit_screen: Option<Bounds<Pixels>>,
    accent_hex: u32,
    chrome: ChromeTheme,
    appearance_subscription: Option<Subscription>,
    effects: RegionEffects,
    effects_open: bool,
    more_open: bool,
    toolbar_hidden: bool,
    hints_visible: bool,
    status: Option<String>,
    status_serial: u64,
    pub(crate) focus_handle: FocusHandle,
    inline: Option<InlineHost>,
}

impl EditorView {
    fn new(frame: Arc<RgbaFrame>, pin_target: Option<u64>, cx: &mut Context<Self>) -> Self {
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image_ref(&frame);
        // Blur previews are built lazily: most captures never use the blur
        // tool, and eagerly resizing four full frames cost startup CPU.
        let blur_images: [Option<Arc<RenderImage>>; BLUR_RADII.len()] =
            std::array::from_fn(|_| None);
        let accent_hex = system_accent_hex().unwrap_or(0x4C9AFF);
        let settings = settings_state::get();
        let active_color_tool = AnnotationColorTool::Rect;
        let style_color = settings.annotation_color_for(active_color_tool);
        let font_size = settings.annotation_font_size;
        let arrow_start = settings.annotation_arrow_start;
        let arrow_end = settings.annotation_arrow_end;
        let arrow_polyline = settings.annotation_arrow_polyline;
        let text_background = settings.annotation_text_background;
        let text_outline = settings.annotation_text_outline;
        let text_alignment = settings.annotation_text_alignment;
        let corner_radius = settings.annotation_corner_radius;
        let number_size = settings.annotation_number_size;
        let spotlight_radius = settings.annotation_spotlight_radius;
        let rect_stroke_width = settings.annotation_rect_stroke_width;
        let arrow_stroke_width = settings.annotation_arrow_stroke_width;
        let blur_radius = settings.annotation_blur_radius;
        let rect_filled = settings.annotation_rect_filled;
        let ellipse_filled = settings.annotation_ellipse_filled;
        let shape_ellipse = settings.annotation_rect_ellipse;
        let spotlight_intensity = settings.annotation_spotlight_intensity;
        cx.on_release(|this, cx| {
            if this.images_released {
                return;
            }
            this.images_released = true;
            cx.drop_image(this.image.clone(), None);
            for image in this.blur_images.iter().flatten() {
                cx.drop_image(image.clone(), None);
            }
        })
        .detach();
        Self {
            image,
            blur_images,
            images_released: false,
            frame,
            region_source: None,
            region_drag: None,
            pin_target,
            document: AnnotationDocument::new(image_size),
            image_size,
            tool: Tool::Select,
            style: AnnotationStyle {
                color: style_color,
                stroke_width: rect_stroke_width,
                font_size,
                corner_radius,
                blur_radius,
                filled: false,
                arrow_start,
                arrow_end,
                text_background,
                text_outline,
                text_alignment,
                spotlight_intensity,
                spotlight_radius,
            },
            font_size,
            arrow_start,
            arrow_end,
            arrow_polyline,
            text_background,
            text_outline,
            text_alignment,
            ui_scale: 1.0,
            corner_radius,
            number_size,
            spotlight_radius,
            rect_stroke_width,
            arrow_stroke_width,
            blur_radius,
            rect_filled,
            ellipse_filled,
            shape_ellipse,
            spotlight_intensity,
            active_color_tool,
            draft: None,
            move_draft: None,
            resize_draft: None,
            selection_hover: SelectionHover::None,
            pan_draft: None,
            view_zoom: 1.0,
            view_pan: point(px(0.0), px(0.0)),
            space_down: false,
            crop_mode: false,
            crop_draft: None,
            polyline_draft: None,
            canvas_undo: Vec::new(),
            canvas_redo: Vec::new(),
            annotation_clipboard: None,
            custom_color_open: false,
            custom_color_buffer: String::new(),
            custom_color_fresh: false,
            text_edit: None,
            text_edit_screen: None,
            accent_hex,
            chrome: ChromeTheme::for_appearance(false, Some(accent_hex)),
            appearance_subscription: None,
            effects: RegionEffects::default(),
            effects_open: false,
            more_open: false,
            toolbar_hidden: false,
            hints_visible: false,
            status: None,
            status_serial: 0,
            focus_handle: cx.focus_handle(),
            inline: None,
        }
    }

    pub(crate) fn observe_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance_subscription =
            Some(cx.observe_window_appearance(window, |this, window, cx| {
                this.chrome = ChromeTheme::for_window(window, Some(this.accent_hex));
                cx.notify();
            }));
    }

    /// Builds an editor that paints directly inside an existing overlay or a
    /// dedicated inline editor surface. The image stays in place; screenshot
    /// capture passes `capture: Some`, while pin and long-capture editing pass
    /// `None` and only need the image rect plus its screen origin.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_inline(
        frame: Arc<RgbaFrame>,
        region_source: Option<Arc<RenderImage>>,
        image_rect: Bounds<Pixels>,
        screen_origin: Point<Pixels>,
        initial_tool: Tool,
        capture: Option<InlineCapture>,
        pin_target: Option<u64>,
        zoomable: bool,
        document: Option<AnnotationDocument>,
        effects: RegionEffects,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(frame, pin_target, cx);
        view.ui_scale = if image_rect.size.width > px(0.0) {
            (view.image_size.width as f32 / f32::from(image_rect.size.width)).clamp(0.25, 8.0)
        } else {
            1.0
        };
        view.style.corner_radius = view.corner_radius * view.ui_scale;
        view.style.spotlight_radius = view.spotlight_radius * view.ui_scale;
        view.region_source = region_source;
        if let Some(document) = document
            && document.image_size() == view.image_size
        {
            view.document = document;
        }
        view.effects = effects;
        view.tool = initial_tool;
        if let Some(color_tool) = initial_tool.color_tool() {
            view.active_color_tool = color_tool;
            view.style.color = view.tool_color(color_tool);
        }
        match initial_tool {
            Tool::Rect => {
                view.style.corner_radius = view.corner_radius * view.ui_scale;
                view.style.stroke_width = view.rect_stroke_width;
                view.style.filled = if view.shape_ellipse {
                    view.ellipse_filled
                } else {
                    view.rect_filled
                };
            }
            Tool::Arrow => view.style.stroke_width = view.arrow_stroke_width,
            Tool::Blur => view.style.blur_radius = view.blur_radius,
            Tool::Spotlight => view.style.spotlight_intensity = view.spotlight_intensity,
            _ => {}
        }
        view.inline = Some(InlineHost {
            image_rect,
            screen_origin,
            capture,
            zoomable,
        });
        view
    }

    pub(crate) fn image_view_rect(&self, _window: &Window) -> Bounds<Pixels> {
        let Some(inline) = &self.inline else {
            return Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(0.0), px(0.0)),
            };
        };
        if !inline.zoomable {
            return inline.image_rect;
        }
        let base = inline.image_rect;
        let width = f32::from(base.size.width) * self.view_zoom;
        let height = f32::from(base.size.height) * self.view_zoom;
        let center_x = f32::from(base.origin.x) + f32::from(base.size.width) / 2.0;
        let center_y = f32::from(base.origin.y) + f32::from(base.size.height) / 2.0;
        Bounds {
            origin: point(
                px(center_x - width / 2.0 + f32::from(self.view_pan.x)),
                px(center_y - height / 2.0 + f32::from(self.view_pan.y)),
            ),
            size: size(px(width), px(height)),
        }
    }

    fn image_scale(&self, window: &Window) -> f32 {
        (f32::from(self.image_view_rect(window).size.width) / self.image_size.width.max(1) as f32)
            .max(0.01)
    }

    /// Recomputes the painted-handle hover state. Returns true when the cursor
    /// style has to be repainted.
    fn update_selection_hover(&mut self, point: PhysicalPoint, window: &Window) -> bool {
        let scale = self.image_scale(window);
        let handle = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .and_then(|annotation| annotation_handle_at(&annotation.shape, point, scale));
        let hover = if let Some(handle) = handle {
            SelectionHover::Handle(handle)
        } else if self.document.hit_test(point).is_some() {
            SelectionHover::Body
        } else {
            SelectionHover::None
        };
        if hover == self.selection_hover {
            return false;
        }
        self.selection_hover = hover;
        true
    }

    fn fit_scale(&self, window: &Window) -> f32 {
        (self.image_scale(window) / self.view_zoom.max(0.01)).max(0.01)
    }

    /// The selection area inside the displayed image. A composed window frame
    /// has transparent padding around the captured content, so the info bar and
    /// toolbar anchor to the content rect, not the padded frame.
    pub(crate) fn inline_selection_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let inline = self.inline?;
        let capture = inline.capture?;
        let image = self.image_view_rect(window);
        let scale = capture.scale.max(0.1);
        let width = capture.region.rect.size.width as f32 / scale;
        let height = capture.region.rect.size.height as f32 / scale;
        let center_x = f32::from(image.origin.x) + f32::from(image.size.width) / 2.0;
        let center_y = f32::from(image.origin.y) + f32::from(image.size.height) / 2.0;
        Some(Bounds {
            origin: point(px(center_x - width / 2.0), px(center_y - height / 2.0)),
            size: size(px(width), px(height)),
        })
    }

    fn toolbar_anchor(&self, window: &Window) -> Bounds<Pixels> {
        self.inline_selection_rect(window)
            .unwrap_or_else(|| self.image_view_rect(window))
    }

    fn toolbar_rect(&self, window: &Window) -> Bounds<Pixels> {
        let anchor = self.toolbar_anchor(window);
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = COMPACT_TOOLBAR_WIDTH.min((window_width - 16.0).max(120.0));
        let anchor_left = f32::from(anchor.origin.x);
        let anchor_top = f32::from(anchor.origin.y);
        let anchor_right = anchor_left + f32::from(anchor.size.width);
        let anchor_bottom = anchor_top + f32::from(anchor.size.height);
        let left = ((anchor_left + anchor_right) / 2.0 - width / 2.0)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let below = anchor_bottom + 8.0;
        let above = anchor_top - COMPACT_TOOLBAR_HEIGHT - 8.0;
        let top = if below + COMPACT_TOOLBAR_HEIGHT <= window_height - 8.0 {
            below
        } else if above >= 8.0 {
            above
        } else {
            (window_height - COMPACT_TOOLBAR_HEIGHT - 8.0).max(8.0)
        };
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(COMPACT_TOOLBAR_HEIGHT)),
        }
    }

    fn options_rect(&self, window: &Window, toolbar: Bounds<Pixels>) -> Bounds<Pixels> {
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 560.0_f32.min((window_width - 16.0).max(120.0));
        let left = f32::from(toolbar.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let toolbar_top = f32::from(toolbar.origin.y);
        let toolbar_bottom = toolbar_top + f32::from(toolbar.size.height);
        let anchor = self.toolbar_anchor(window);
        let anchor_bottom = f32::from(anchor.origin.y) + f32::from(anchor.size.height);
        // Keep the options on the same side as the toolbar, i.e. further away
        // from the capture. Placing them between the toolbar and the image
        // covered the bottom of small selections.
        let toolbar_below = toolbar_top >= anchor_bottom;
        let below = toolbar_bottom + 6.0;
        let above = toolbar_top - OPTIONS_PANEL_HEIGHT - 6.0;
        let (preferred, fallback) = if toolbar_below {
            (below, above)
        } else {
            (above, below)
        };
        let fits = |top: f32| top >= 8.0 && top + OPTIONS_PANEL_HEIGHT <= window_height - 8.0;
        let top = if fits(preferred) {
            preferred
        } else if fits(fallback) {
            fallback
        } else {
            preferred.clamp(8.0, (window_height - OPTIONS_PANEL_HEIGHT - 8.0).max(8.0))
        };
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(OPTIONS_PANEL_HEIGHT)),
        }
    }

    fn info_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let anchor = self.inline_selection_rect(window)?;
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = INFO_BAR_WIDTH.min((window_width - 16.0).max(120.0));
        let left = f32::from(anchor.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let toolbar = self.toolbar_rect(window);
        let anchor_top = f32::from(anchor.origin.y);
        let anchor_bottom = anchor_top + f32::from(anchor.size.height);
        let toolbar_top = f32::from(toolbar.origin.y);
        let toolbar_bottom = toolbar_top + f32::from(toolbar.size.height);
        let options_visible = self.config_tool().is_some() && !self.toolbar_hidden;
        let options = self.options_rect(window, toolbar);
        let options_top = f32::from(options.origin.y);
        let options_bottom = options_top + f32::from(options.size.height);
        let fits = |top: f32| top >= 8.0 && top + INFO_BAR_HEIGHT <= window_height - 8.0;
        let overlaps_toolbar = |top: f32| {
            !self.toolbar_hidden && top < toolbar_bottom && top + INFO_BAR_HEIGHT > toolbar_top
        };
        let above = anchor_top - INFO_BAR_HEIGHT - 8.0;
        let below = anchor_bottom + 8.0;
        let mut candidates = Vec::new();
        if fits(above) && !overlaps_toolbar(above) {
            candidates.push(above);
        }
        if toolbar_top >= anchor_bottom {
            let outside = if options_visible {
                options_bottom + 6.0
            } else {
                toolbar_bottom + 6.0
            };
            if fits(outside) {
                candidates.push(outside);
            }
        }
        if toolbar_bottom <= anchor_top {
            let outside = if options_visible {
                options_top - INFO_BAR_HEIGHT - 6.0
            } else {
                toolbar_top - INFO_BAR_HEIGHT - 6.0
            };
            if fits(outside) {
                candidates.push(outside);
            }
        }
        if fits(below) && !overlaps_toolbar(below) {
            candidates.push(below);
        }
        if fits(anchor_top + 8.0) {
            candidates.push(anchor_top + 8.0);
        }
        if fits(anchor_bottom - INFO_BAR_HEIGHT - 8.0) {
            candidates.push(anchor_bottom - INFO_BAR_HEIGHT - 8.0);
        }
        let top = candidates
            .first()
            .copied()
            .unwrap_or(8.0)
            .clamp(8.0, (window_height - INFO_BAR_HEIGHT - 8.0).max(8.0));
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(INFO_BAR_HEIGHT)),
        })
    }

    fn more_rect(&self, window: &Window, toolbar: Bounds<Pixels>) -> Bounds<Pixels> {
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 196.0;
        let height = if self.pin_target.is_some() {
            136.0
        } else {
            42.0
        };
        let left = (f32::from(toolbar.origin.x) + f32::from(toolbar.size.width) - width)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let mut top = f32::from(toolbar.origin.y) + f32::from(toolbar.size.height) + 6.0;
        if top + height > window_height - 8.0 {
            top = (f32::from(toolbar.origin.y) - height - 6.0).max(8.0);
        }
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        }
    }

    fn effects_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let info = self.info_rect(window)?;
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 250.0_f32.min((window_width - 16.0).max(120.0));
        let height = 86.0;
        let left = f32::from(info.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let top =
            crate::ui::popover_top(info, height, window_height, Some(self.toolbar_rect(window)));
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        })
    }

    fn custom_color_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        if !self.custom_color_open {
            return None;
        }
        let toolbar = self.toolbar_rect(window);
        let options = self.options_rect(window, toolbar);
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 236.0_f32.min((window_width - 16.0).max(120.0));
        let height = 116.0;
        let left = ((f32::from(options.origin.x) + f32::from(options.size.width) / 2.0)
            - width / 2.0)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let options_below_toolbar = f32::from(options.origin.y)
            >= f32::from(toolbar.origin.y) + f32::from(toolbar.size.height);
        let preferred = if options_below_toolbar {
            f32::from(options.origin.y) + f32::from(options.size.height) + 6.0
        } else {
            f32::from(options.origin.y) - height - 6.0
        };
        let fits = preferred >= 8.0 && preferred + height <= window_height - 8.0;
        let top = if fits {
            preferred
        } else {
            crate::ui::popover_top(options, height, window_height, Some(toolbar))
        };
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        })
    }

    fn custom_color_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.custom_color_open || self.toolbar_hidden {
            return None;
        }
        let rect = self.custom_color_rect(window)?;
        let language = settings_state::get().language;
        let palette_color = self.palette_color();
        let preview_rgb = self
            .custom_color_buffer
            .trim()
            .trim_start_matches('#')
            .chars()
            .take(6)
            .collect::<String>();
        let preview = u32::from_str_radix(&preview_rgb, 16)
            .ok()
            .filter(|_| preview_rgb.len() == 6)
            .map(|rgb| (rgb & 0x00FF_FFFF) << 8 | 0xFF)
            .unwrap_or(palette_color);
        let mut hue_row = div().flex().flex_row().items_center().gap(px(3.0));
        for index in 0..12u32 {
            let hue_color = crate::ui::hue_rgb(index as f32 * 30.0);
            let label = format!("#{hue_color:06x}");
            hue_row = hue_row.child(
                div()
                    .id(format!("editor-hue-{index}"))
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded(px(7.0))
                    .bg(rgb(hue_color))
                    .border_1()
                    .border_color(self.chrome.border)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.apply_color((hue_color << 8) | 0xFF);
                        this.custom_color_buffer = format!("{hue_color:06x}");
                        this.custom_color_fresh = false;
                        this.status = None;
                        cx.notify();
                    })),
            );
        }

        let panel = div()
            .id("editor-custom-color-panel")
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded(px(4.0))
                            .bg(rgba(preview))
                            .border_1()
                            .border_color(self.chrome.border),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .child(t(language, "editor.custom_color")),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(self.chrome.selected)
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .cursor_text()
                            .child({
                                let mut row =
                                    div().flex().flex_row().items_center().gap(px(1.0)).child(
                                        format!(
                                            "#{}",
                                            self.custom_color_buffer.to_ascii_uppercase()
                                        ),
                                    );
                                if !self.custom_color_fresh {
                                    row = row.child(
                                        div()
                                            .w(px(1.0))
                                            .h(px(11.0))
                                            .bg(self.chrome.accent)
                                            .with_animation(
                                                "editor-custom-color-caret",
                                                crate::ui::caret_blink(),
                                                |caret, delta| caret.opacity(delta),
                                            ),
                                    );
                                }
                                row
                            }),
                    ),
            )
            .child(hue_row)
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(self.chrome.muted)
                    .child(t(language, "editor.custom_color_hint")),
            );
        let panel = crate::ui::enter_slide_fade(
            panel,
            "editor-custom-color-enter",
            f32::from(rect.origin.y),
            4.0,
            Duration::from_millis(120),
        );

        Some(
            div()
                .id("editor-custom-color-layer")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_custom_color();
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    fn zoom_at(
        &mut self,
        factor: f32,
        anchor: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.image_view_rect(window);
        let before_scale = f32::from(before.size.width) / self.image_size.width.max(1) as f32;
        if before_scale <= 0.0 {
            return;
        }
        let image_x = (f32::from(anchor.x) - f32::from(before.origin.x)) / before_scale;
        let image_y = (f32::from(anchor.y) - f32::from(before.origin.y)) / before_scale;
        let zoom = (self.view_zoom * factor).clamp(0.25, 8.0);
        if (zoom - self.view_zoom).abs() < f32::EPSILON {
            return;
        }
        self.view_zoom = zoom;
        let after = self.image_view_rect(window);
        let after_scale = f32::from(after.size.width) / self.image_size.width.max(1) as f32;
        let desired_x = f32::from(anchor.x) - image_x * after_scale;
        let desired_y = f32::from(anchor.y) - image_y * after_scale;
        self.view_pan.x += px(desired_x - f32::from(after.origin.x));
        self.view_pan.y += px(desired_y - f32::from(after.origin.y));
        cx.notify();
    }

    fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.view_zoom = 1.0;
        self.view_pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    fn actual_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let fit = self.fit_scale(window);
        self.view_zoom = (1.0 / fit).clamp(0.25, 8.0);
        self.view_pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    fn push_canvas_snapshot(&mut self) {
        self.canvas_undo.push(EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        });
        if self.canvas_undo.len() > CANVAS_HISTORY_LIMIT {
            self.canvas_undo.remove(0);
        }
        self.canvas_redo.clear();
    }

    /// Rebuilds the GPUI images for the current frame and returns the old
    /// uploads to the window atlas. Direct `img(RenderImage)` entries stay in
    /// the atlas until dropped, so every canvas replacement has to release
    /// the previous set explicitly.
    fn refresh_frame_images(&mut self, cx: &mut Context<Self>) {
        let image = image_util::render_image_ref(&self.frame);
        let old_image = std::mem::replace(&mut self.image, image);
        let old_blur_images =
            std::mem::replace(&mut self.blur_images, std::array::from_fn(|_| None));
        self.images_released = false;
        // The update runs with the window removed from `App.windows`, so an
        // immediate `App::drop_image` cannot reach the editor's atlas. Defer
        // the drop until the update finished; otherwise every F5 refresh or
        // canvas replacement leaks the previous full image.
        let mut dropped = Vec::with_capacity(1 + old_blur_images.len());
        dropped.push(old_image);
        dropped.extend(old_blur_images.into_iter().flatten());
        cx.defer(move |cx| {
            for image in dropped {
                cx.drop_image(image, None);
            }
        });
    }

    /// Returns the cached blurred preview for `radius`, creating it on first
    /// use. The blur tool is rare, so the four previews no longer cost a
    /// resize and blur over the full frame on every editor open.
    fn blur_image(&mut self, radius: u32, _cx: &mut Context<Self>) -> Arc<RenderImage> {
        let index = blur_image_index(radius);
        if let Some(image) = &self.blur_images[index] {
            return image.clone();
        }
        let image = image_util::blurred_image(&self.frame, radius);
        self.blur_images[index] = Some(image.clone());
        image
    }

    /// Releases the editor's atlas entries before the window is removed.
    /// `on_release` runs after the window has left `App::windows`, so dropping
    /// only there cannot reach the retained Windows renderer.
    pub(crate) fn release_images(&mut self, cx: &mut Context<Self>) {
        if self.images_released {
            return;
        }
        self.images_released = true;
        cx.drop_image(self.image.clone(), None);
        for image in self.blur_images.iter().flatten() {
            cx.drop_image(image.clone(), None);
        }
    }

    /// Releases textures when the platform asks to close the window (Alt+F4,
    /// native close button, app quit) before the window leaves `App::windows`.
    pub(crate) fn watch_close(&self, window: &Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = view.update(cx, |this, cx| this.release_images(cx));
            true
        });
    }

    fn restore_canvas_snapshot(&mut self, snapshot: EditorSnapshot, cx: &mut Context<Self>) {
        self.frame = snapshot.frame;
        self.image_size = snapshot.image_size;
        self.document = snapshot.document;
        self.refresh_frame_images(cx);
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        self.pan_draft = None;
        self.crop_mode = false;
        self.crop_draft = None;
        self.reset_view(cx);
    }

    fn undo_canvas(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.canvas_undo.pop() else {
            return false;
        };
        self.canvas_redo.push(EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        });
        self.restore_canvas_snapshot(snapshot, cx);
        true
    }

    fn redo_canvas(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.canvas_redo.pop() else {
            return false;
        };
        self.canvas_undo.push(EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        });
        self.restore_canvas_snapshot(snapshot, cx);
        true
    }

    fn replace_canvas(&mut self, frame: RgbaFrame, cx: &mut Context<Self>) {
        self.frame = Arc::new(frame);
        self.image_size = PhysicalSize::new(self.frame.width(), self.frame.height());
        self.refresh_frame_images(cx);
        self.document = AnnotationDocument::new(self.image_size);
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        self.pan_draft = None;
        self.crop_mode = false;
        self.crop_draft = None;
        self.reset_view(cx);
    }

    fn region_capture(&self) -> Option<(CaptureRegion, PhysicalRect)> {
        let inline = self.inline.as_ref()?;
        if inline.zoomable {
            return None;
        }
        let capture = inline.capture.as_ref()?;
        let CaptureTarget::Region(region) = capture.target else {
            return None;
        };
        Some((region, region.rect))
    }

    fn can_adjust_region(&self) -> bool {
        self.tool == Tool::Select
            && self.region_source.is_some()
            && self.region_capture().is_some()
            && self.document.selected().is_none()
            && self.draft.is_none()
            && self.move_draft.is_none()
            && self.resize_draft.is_none()
            && self.polyline_draft.is_none()
            && self.text_edit.is_none()
            && !self.crop_mode
            && self.region_drag.is_none()
    }

    fn begin_region_drag(&mut self, handle: RegionHandle, cx: &mut Context<Self>) {
        let Some((_, rect)) = self.region_capture() else {
            return;
        };
        self.region_drag = Some(RegionAdjustDraft {
            handle,
            original: rect,
            current: rect,
        });
        self.status = None;
        cx.notify();
    }

    fn update_region_drag(
        &mut self,
        position: Point<Pixels>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.region_source.clone() else {
            return;
        };
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let Some(draft) = self.region_drag.as_ref().copied() else {
            return;
        };
        let scale = capture.scale.max(0.1);
        let cursor_point = PhysicalPoint::new(
            (f32::from(position.x) * scale).round() as i32,
            (f32::from(position.y) * scale).round() as i32,
        );
        let image_size = image_util::image_size(&source);
        let rect = resize_rect(
            draft.original,
            draft.handle,
            cursor_point,
            None,
            modifiers.shift,
            modifiers.alt,
            image_size,
        );
        if let Some(draft) = self.region_drag.as_mut() {
            draft.current = rect;
        }
        // Keep the captured image and annotations at their current size while
        // the crop frame moves; resizing the image element here stretched the
        // already-captured pixels until the pointer was released.
        cx.notify();
    }

    fn finish_region_drag(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.region_drag.take() else {
            return;
        };
        let Some(source) = self.region_source.clone() else {
            return;
        };
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let scale = capture.scale.max(0.1);
        if draft.current == draft.original {
            self.restore_region_image_rect(scale, cx);
            return;
        }
        let Some(cropped) = image_util::cropped_rgba(&source, draft.current) else {
            self.restore_region_image_rect(scale, cx);
            return;
        };
        let dx = draft.current.origin.x - draft.original.origin.x;
        let dy = draft.current.origin.y - draft.original.origin.y;
        let new_size = PhysicalSize::new(cropped.width(), cropped.height());
        let mut document = AnnotationDocument::new(new_size);
        for annotation in self.document.objects() {
            let translated = annotation.clone().translated(-dx, -dy);
            document.add(translated.shape, translated.style);
        }
        // `add` selects the newest object; a crop should leave the document
        // deselected instead of popping annotation handles the user did not
        // ask for.
        document.select(None);
        document.clear_history();
        self.frame = Arc::new(cropped);
        self.image_size = new_size;
        self.refresh_frame_images(cx);
        self.document = document;
        self.canvas_undo.clear();
        self.canvas_redo.clear();
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        self.polyline_draft = None;
        self.selection_hover = SelectionHover::None;
        self.view_zoom = 1.0;
        self.view_pan = point(px(0.0), px(0.0));
        if let Some(inline) = self.inline.as_mut() {
            inline.image_rect = Bounds {
                origin: point(
                    px(draft.current.origin.x as f32 / scale),
                    px(draft.current.origin.y as f32 / scale),
                ),
                size: size(
                    px(draft.current.size.width as f32 / scale),
                    px(draft.current.size.height as f32 / scale),
                ),
            };
            if let Some(capture) = inline.capture.as_mut() {
                capture.region = CaptureRegion::new(capture.region.monitor, draft.current);
                capture.target = CaptureTarget::Region(capture.region);
            }
        }
        self.show_status(
            t(settings_state::get().language, "editor.cropped").to_owned(),
            cx,
        );
    }

    fn restore_region_image_rect(&mut self, scale: f32, cx: &mut Context<Self>) {
        let Some((_, rect)) = self.region_capture() else {
            return;
        };
        if let Some(inline) = self.inline.as_mut() {
            inline.image_rect = Bounds {
                origin: point(
                    px(rect.origin.x as f32 / scale),
                    px(rect.origin.y as f32 / scale),
                ),
                size: size(
                    px(rect.size.width as f32 / scale),
                    px(rect.size.height as f32 / scale),
                ),
            };
        }
        cx.notify();
    }

    fn region_handles(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.can_adjust_region() {
            return Vec::new();
        }
        let rect = self.image_view_rect(window);
        let mut handles = Vec::new();
        for (handle, position) in handle_positions(rect) {
            handles.push(
                div()
                    .id(format!("editor-region-handle-{:?}", handle))
                    .absolute()
                    .left(px(f32::from(position.x) - 4.5))
                    .top(px(f32::from(position.y) - 4.5))
                    .w(px(9.0))
                    .h(px(9.0))
                    .rounded(px(1.5))
                    .bg(rgb(0xFFFFFF))
                    .border_1()
                    .border_color(rgba((self.accent_hex << 8) | 0xFF))
                    .cursor(region_handle_cursor(handle))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            let focus = this.focus_handle.clone();
                            window.focus(&focus, cx);
                            this.begin_region_drag(handle, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
        handles
    }

    /// Paints the crop frame while a region edge is being dragged. The image
    /// stays at its current size so the captured pixels are never stretched;
    /// the frame and its handles preview the rectangle that will be applied on
    /// release.
    fn region_drag_preview(&self) -> Option<AnyElement> {
        let draft = self.region_drag?;
        let capture = self.inline.as_ref()?.capture.as_ref()?;
        let scale = capture.scale.max(0.1);
        let rect = Bounds {
            origin: point(
                px(draft.current.origin.x as f32 / scale),
                px(draft.current.origin.y as f32 / scale),
            ),
            size: size(
                px(draft.current.size.width as f32 / scale),
                px(draft.current.size.height as f32 / scale),
            ),
        };
        let accent = rgba((self.accent_hex << 8) | 0xFF);
        let mut layer = div()
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .border_1()
            .border_color(accent);
        for (_, position) in handle_positions(rect) {
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(f32::from(position.x) - f32::from(rect.origin.x) - 4.5))
                    .top(px(f32::from(position.y) - f32::from(rect.origin.y) - 4.5))
                    .w(px(9.0))
                    .h(px(9.0))
                    .rounded(px(1.5))
                    .bg(rgb(0xFFFFFF))
                    .border_1()
                    .border_color(accent),
            );
        }
        Some(layer.into_any_element())
    }

    /// Selection outline and resize handles as regular elements. They sit
    /// above the image layer (so handles on the image edge are not clipped)
    /// and are rebuilt with every render, unlike the scene-painted canvas
    /// selection that could lag behind a click.
    fn selection_layer(&self, window: &Window) -> Option<AnyElement> {
        if self.text_edit.is_some() {
            return None;
        }
        let shape = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.shape.clone())?;
        let image_rect = self.image_view_rect(window);
        let scale = f32::from(image_rect.size.width) / self.image_size.width.max(1) as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let accent = rgba((self.accent_hex << 8) | 0xFF);
        let rect = shape.bounds();
        let left = f32::from(image_rect.origin.x) + rect.origin.x as f32 * scale;
        let top = f32::from(image_rect.origin.y) + rect.origin.y as f32 * scale;
        let mut layer = div().absolute().inset_0();
        layer = layer.child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px((rect.size.width as f32 * scale).max(0.0)))
                .h(px((rect.size.height as f32 * scale).max(0.0)))
                .border_1()
                .border_color(accent),
        );
        for (_, handle) in handle_points(&shape) {
            let centre_x = f32::from(image_rect.origin.x) + handle.x as f32 * scale;
            let centre_y = f32::from(image_rect.origin.y) + handle.y as f32 * scale;
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(centre_x - SELECTION_HANDLE_HALF))
                    .top(px(centre_y - SELECTION_HANDLE_HALF))
                    .w(px(SELECTION_HANDLE_HALF * 2.0))
                    .h(px(SELECTION_HANDLE_HALF * 2.0))
                    .bg(accent)
                    .child(
                        div()
                            .absolute()
                            .left(px(SELECTION_HANDLE_HALF - 3.0))
                            .top(px(SELECTION_HANDLE_HALF - 3.0))
                            .w(px(6.0))
                            .h(px(6.0))
                            .bg(rgb(0xFFFFFF)),
                    ),
            );
        }
        Some(layer.into_any_element())
    }

    fn rotate_canvas(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let language = settings_state::get().language;
        let Ok(output) = rasterize(&self.document, &self.frame) else {
            self.show_status(t(language, "editor.export_failed").to_owned(), cx);
            return;
        };
        self.push_canvas_snapshot();
        let rotated = output.rotated_cw();
        self.replace_canvas(rotated, cx);
        self.show_status(t(language, "editor.rotated").to_owned(), cx);
    }

    fn flip_canvas(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let language = settings_state::get().language;
        let Ok(output) = rasterize(&self.document, &self.frame) else {
            self.show_status(t(language, "editor.export_failed").to_owned(), cx);
            return;
        };
        self.push_canvas_snapshot();
        let flipped = output.flipped(horizontal);
        self.replace_canvas(flipped, cx);
        self.show_status(
            t(
                language,
                if horizontal {
                    "editor.flipped_horizontal"
                } else {
                    "editor.flipped_vertical"
                },
            )
            .to_owned(),
            cx,
        );
    }

    fn toggle_crop(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        self.crop_mode = !self.crop_mode;
        self.crop_draft = None;
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        if self.crop_mode {
            self.document.select(None);
            self.selection_hover = SelectionHover::None;
        }
        self.status = self
            .crop_mode
            .then(|| t(settings_state::get().language, "editor.crop_hint").to_owned());
        cx.notify();
    }

    fn apply_crop(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.crop_draft.take() else {
            return;
        };
        let rect = normalized_rect(draft.start, draft.current);
        if rect.size.width < 2 || rect.size.height < 2 {
            return;
        }
        let left = rect.origin.x.clamp(0, self.image_size.width as i32);
        let top = rect.origin.y.clamp(0, self.image_size.height as i32);
        let right =
            (rect.origin.x + rect.size.width as i32).clamp(left, self.image_size.width as i32);
        let bottom =
            (rect.origin.y + rect.size.height as i32).clamp(top, self.image_size.height as i32);
        let rect = PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32);
        if rect.size.width < 2 || rect.size.height < 2 {
            return;
        }
        let Some(cropped) = self.frame.as_ref().cropped(rect) else {
            return;
        };
        self.push_canvas_snapshot();
        let new_size = PhysicalSize::new(cropped.width(), cropped.height());
        let mut document = AnnotationDocument::new(new_size);
        for annotation in self.document.objects() {
            let translated = annotation
                .clone()
                .translated(-rect.origin.x, -rect.origin.y);
            document.add(translated.shape, translated.style);
        }
        document.clear_history();
        self.frame = Arc::new(cropped);
        self.image_size = new_size;
        self.refresh_frame_images(cx);
        self.document = document;
        self.crop_mode = false;
        self.crop_draft = None;
        self.text_edit = None;
        self.reset_view(cx);
        self.show_status(
            t(settings_state::get().language, "editor.cropped").to_owned(),
            cx,
        );
    }

    fn cancel_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = false;
        self.crop_draft = None;
        self.status = None;
        cx.notify();
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
            // Filled shapes are opaque by design; there is no opacity control.
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
            Tool::Blur => style.blur_radius = self.blur_radius,
            Tool::Spotlight => {
                style.spotlight_intensity = self.spotlight_intensity;
                style.spotlight_radius = self.spotlight_radius * self.ui_scale;
            }
            _ => {}
        }
        style
    }

    /// Current color for a tool. Reading the mode from the shared settings
    /// keeps an already-open editor in step when the setting is toggled.
    fn tool_color(&self, tool: AnnotationColorTool) -> u32 {
        settings_state::get().annotation_color_for(tool)
    }

    /// The tool whose color the toolbar palette edits: the selected object's
    /// tool when there is a selection, otherwise the active drawing tool.
    fn palette_color_tool(&self) -> AnnotationColorTool {
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

    /// The tool whose properties the toolbar should expose. The selected
    /// object wins over the drawing tool; `Select` alone has no properties.
    fn config_tool(&self) -> Option<Tool> {
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

    fn config_shape_ellipse(&self) -> bool {
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

    fn config_arrow_polyline(&self) -> bool {
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

    fn palette_color(&self) -> u32 {
        self.document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style.color)
            .unwrap_or_else(|| self.tool_color(self.palette_color_tool()))
    }

    fn open_custom_color(&mut self) {
        self.custom_color_buffer = format!("{:06x}", (self.palette_color() >> 8) & 0x00FF_FFFF);
        self.custom_color_fresh = true;
        self.custom_color_open = true;
    }

    fn close_custom_color(&mut self) {
        self.custom_color_open = false;
        self.custom_color_fresh = false;
    }

    /// Applies a typed `RRGGBB` value. Returns false while the buffer is not a
    /// complete color, so the panel can stay open.
    fn apply_custom_color(&mut self, cx: &mut Context<Self>) -> bool {
        let hex = self.custom_color_buffer.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        let Ok(rgb) = u32::from_str_radix(hex, 16) else {
            return false;
        };
        self.apply_color((rgb << 8) | 0xFF);
        self.close_custom_color();
        self.status = None;
        cx.notify();
        true
    }

    fn apply_stroke_width(&mut self, width: f32) {
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

    fn apply_font_size(&mut self, size: f32) {
        self.font_size = size;
        self.update_style(|style| style.font_size = size);
        if let Some(edit) = &mut self.text_edit {
            edit.style.font_size = size;
        }
        settings_state::update(|settings| settings.annotation_font_size = size);
    }

    fn apply_text_background(&mut self, enabled: bool) {
        self.text_background = enabled;
        self.update_style(|style| style.text_background = enabled);
        if let Some(edit) = &mut self.text_edit {
            edit.style.text_background = enabled;
        }
        settings_state::update(|settings| settings.annotation_text_background = enabled);
    }

    fn apply_text_outline(&mut self, enabled: bool) {
        self.text_outline = enabled;
        self.update_style(|style| style.text_outline = enabled);
        if let Some(edit) = &mut self.text_edit {
            edit.style.text_outline = enabled;
        }
        settings_state::update(|settings| settings.annotation_text_outline = enabled);
    }

    fn apply_text_alignment(&mut self, alignment: TextAlignment) {
        self.text_alignment = alignment;
        self.update_style(|style| style.text_alignment = alignment);
        if let Some(edit) = &mut self.text_edit {
            edit.style.text_alignment = alignment;
        }
        settings_state::update(|settings| settings.annotation_text_alignment = alignment);
    }

    fn apply_arrow_heads(&mut self, start: bool, end: bool) {
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

    fn apply_arrow_polyline(&mut self, polyline: bool) {
        self.arrow_polyline = polyline;
        self.polyline_draft = None;
        settings_state::update(|settings| settings.annotation_arrow_polyline = polyline);
    }

    fn apply_corner_radius(&mut self, radius: f32) {
        self.corner_radius = radius;
        let applied = radius * self.ui_scale;
        self.update_style(|style| style.corner_radius = applied);
        settings_state::update(|settings| settings.annotation_corner_radius = radius);
    }

    fn apply_number_size(&mut self, size: u32) {
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
        self.document.update_shape(
            id,
            Shape::Number {
                bounds: PhysicalRect::new(left, top, side, side),
                value,
            },
            annotation.style,
        );
    }

    fn apply_spotlight_radius(&mut self, radius: f32) {
        self.spotlight_radius = radius;
        let applied = radius * self.ui_scale;
        self.update_style(|style| style.spotlight_radius = applied);
        settings_state::update(|settings| settings.annotation_spotlight_radius = radius);
    }

    fn apply_blur_radius(&mut self, radius: u32) {
        self.blur_radius = radius;
        self.update_style(|style| style.blur_radius = radius);
        settings_state::update(|settings| settings.annotation_blur_radius = radius);
    }

    fn apply_fill(&mut self, filled: bool) {
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

    fn apply_shape_ellipse(&mut self, ellipse: bool) {
        self.shape_ellipse = ellipse;
        settings_state::update(|settings| settings.annotation_rect_ellipse = ellipse);
    }

    fn apply_spotlight_intensity(&mut self, intensity: u8) {
        let intensity = intensity.min(100);
        self.spotlight_intensity = intensity;
        self.update_style(|style| style.spotlight_intensity = intensity);
        settings_state::update(|settings| settings.annotation_spotlight_intensity = intensity);
    }

    fn apply_color(&mut self, color: u32) {
        let tool = self.palette_color_tool();
        // Keep the alpha the selected object already renders with. Reading
        // `self.style` here was wrong right after drawing: the draft uses
        // `effective_style` but `self.style` still holds the opaque palette
        // color, so the first color switch turned a highlighter box opaque.
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
            self.document.update_selected_style(style);
            self.style = style;
        }

        settings_state::update(|settings| settings.apply_annotation_color(tool, color));
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

    fn finish_polyline(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.polyline_draft.take() else {
            return;
        };
        let mut points = draft.points;
        if distance(*points.last().unwrap_or(&draft.current), draft.current) > 2.0 {
            points.push(draft.current);
        }
        if points.len() >= 2 {
            self.document
                .add(Shape::Polyline { points }, self.effective_style());
        }
        self.status = None;
        cx.notify();
    }

    fn cancel_polyline(&mut self, cx: &mut Context<Self>) {
        self.polyline_draft = None;
        self.status = None;
        cx.notify();
    }

    /// Applies a move gesture up to `point`. The shape is always rebuilt from
    /// the geometry captured on mouse-down, so a dropped mouse-up cannot make
    /// the object snap back to its pre-drag position: every move already
    /// updates the document. The first actual movement records one undo step.
    fn update_move(&mut self, point: PhysicalPoint) -> bool {
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
        self.document.update_shape_live(id, shape, record)
    }

    fn update_resize(&mut self, point: PhysicalPoint) -> bool {
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
        self.document.update_shape_live(id, shape, record)
    }

    fn finish_resize(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) {
        if self.resize_draft.is_none() {
            return;
        }
        self.update_resize(point);
        self.resize_draft = None;
        self.status = None;
        cx.notify();
    }

    fn select_annotation(&mut self, id: Option<core_annotate::AnnotationId>) {
        self.close_custom_color();
        self.document.select(id);
        if let Some(annotation) = id.and_then(|id| self.document.object(id)) {
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
            // Deselecting returns the toolbar to the drawing tool's own
            // independent defaults instead of keeping the object's width.
            self.style.stroke_width = match self.tool {
                Tool::Rect => self.rect_stroke_width,
                Tool::Arrow => self.arrow_stroke_width,
                _ => self.style.stroke_width,
            };
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
        self.select_annotation(Some(id));
        self.status =
            Some(t(settings_state::get().language, "editor.annotation_pasted").to_owned());
        cx.notify();
        true
    }

    fn delete_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.document.selected() else {
            log_info!("editor", "delete requested without a selected annotation");
            return false;
        };
        if !self.document.remove(id) {
            log_info!("editor", "delete failed for annotation {id:?}");
            return false;
        }
        log_info!("editor", "deleted annotation {id:?}");
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

    fn commit_text(&mut self, cx: &mut Context<Self>) {
        let Some(mut edit) = self.text_edit.take() else {
            return;
        };
        // Clicking elsewhere can commit while a composition is still in
        // flight; never persist the IME's pinyin buffer as user text.
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

    fn show_status(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = Some(message);
        self.status_serial = self.status_serial.wrapping_add(1);
        let serial = self.status_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            this.update(cx, |this, cx| {
                if this.status_serial == serial {
                    this.status = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn text_index_at(&self, window: &Window, position: Point<Pixels>) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let bounds = self.text_edit_screen?;
        let scale = self.image_scale(window);
        let font_size = (edit.style.font_size * scale).max(8.0);
        let width = (edit.bounds.size.width as f32 * scale).max(24.0);
        let Some(layout) = layout_annotation_text(window, &edit.content, font_size, width) else {
            return Some(0);
        };
        let line_height = font_size * 1.3;
        let boxes = text_line_boxes(&layout, line_height);
        let local_x = f32::from(position.x - bounds.origin.x).max(0.0);
        let local_y = f32::from(position.y - bounds.origin.y).max(0.0);
        let line = boxes
            .iter()
            .find(|line| local_y < line.y + line_height)
            .or_else(|| boxes.last());
        let offset = line.map_or(0.0, |line| {
            text_alignment_offset(edit.style.text_alignment, width, line.width)
        });
        let point = point(px((local_x - offset).max(0.0)), px(local_y));
        Some(
            layout
                .index_for_position(point, px(line_height))
                .unwrap_or_else(|index| index),
        )
    }

    fn move_text_vertical(
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
            let layout = layout_annotation_text(window, &edit.content, font_size, width)?;
            let boxes = text_line_boxes(&layout, line_height);
            let caret = text_caret_position(Some(&layout), &boxes, edit, width);
            let target_y =
                (f32::from(caret.y) + if up { -line_height } else { line_height }).max(0.0);
            let line = boxes
                .iter()
                .find(|line| target_y < line.y + line_height)
                .or_else(|| boxes.last())?;
            let offset = text_alignment_offset(edit.style.text_alignment, width, line.width);
            let x = (f32::from(caret.x) - offset).max(0.0);
            let y = if target_y >= line.y + line_height {
                line.y
            } else {
                target_y
            };
            Some(
                layout
                    .index_for_position(point(px(x), px(y)), px(line_height))
                    .unwrap_or_else(|index| index),
            )
        })() else {
            return;
        };
        self.move_text_cursor(index, extend, cx);
    }

    fn text_line_edge(&self, start: bool, window: &Window) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let scale = self.image_scale(window);
        let font_size = (edit.style.font_size * scale).max(8.0);
        let width = (edit.bounds.size.width as f32 * scale).max(24.0);
        let line_height = font_size * 1.3;
        let layout = layout_annotation_text(window, &edit.content, font_size, width)?;
        let boxes = text_line_boxes(&layout, line_height);
        let caret = text_caret_position(Some(&layout), &boxes, edit, width);
        let y = f32::from(caret.y);
        let line = boxes
            .iter()
            .find(|line| y < line.y + line_height)
            .or_else(|| boxes.last())?;
        Some(if start { line.start } else { line.end })
    }

    fn select_text_word(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        let range = word_range_at(&edit.content, index);
        edit.anchor = range.start;
        edit.cursor = range.end;
        edit.selecting = false;
        cx.notify();
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
        self.show_status(
            t(settings_state::get().language, "editor.copied").to_owned(),
            cx,
        );
    }

    fn toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>, Option<AnyElement>) {
        let language = settings_state::get().language;
        let chrome = self.chrome;
        let text = chrome.text;
        let muted = chrome.muted;
        let accent = chrome.accent;
        let accent_selected = chrome.accent_tint;
        let hover = chrome.hover;
        let border = chrome.border;

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
                .when(!selected, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.document.select(None);
                    this.close_custom_color();
                    this.crop_mode = false;
                    this.crop_draft = None;
                    this.polyline_draft = None;
                    this.tool = tool;
                    if let Some(color_tool) = tool.color_tool() {
                        this.active_color_tool = color_tool;
                        this.style.color = this.tool_color(color_tool);
                    }
                    match tool {
                        Tool::Rect => {
                            this.style.stroke_width = this.rect_stroke_width;
                            this.style.corner_radius = this.corner_radius * this.ui_scale;
                            this.style.filled = if this.shape_ellipse {
                                this.ellipse_filled
                            } else {
                                this.rect_filled
                            };
                        }
                        Tool::Arrow => this.style.stroke_width = this.arrow_stroke_width,
                        Tool::Blur => this.style.blur_radius = this.blur_radius,
                        Tool::Spotlight => {
                            this.style.spotlight_intensity = this.spotlight_intensity;
                            this.style.spotlight_radius = this.spotlight_radius * this.ui_scale;
                        }
                        _ => {}
                    }
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

        let config_tool = self.config_tool();
        let selected_style = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style);
        let config_style = selected_style.unwrap_or(self.style);
        let config_ellipse = self.config_shape_ellipse();
        let config_filled = match config_tool {
            Some(Tool::Rect) => selected_style.map_or(
                if config_ellipse {
                    self.ellipse_filled
                } else {
                    self.rect_filled
                },
                |style| style.filled,
            ),
            _ => false,
        };
        let config_intensity =
            selected_style.map_or(self.spotlight_intensity, |style| style.spotlight_intensity);
        let show_color = config_tool.and_then(Tool::color_tool).is_some();
        let show_fill = config_tool == Some(Tool::Rect);
        let show_width = match config_tool {
            Some(Tool::Arrow) => true,
            Some(Tool::Rect) => !config_filled,
            _ => false,
        };
        let show_font = config_tool == Some(Tool::Text);
        let show_text_style = config_tool == Some(Tool::Text);
        let show_arrow_heads = config_tool == Some(Tool::Arrow);
        let show_corner = config_tool == Some(Tool::Rect) && !config_ellipse;
        let show_number = config_tool == Some(Tool::Number);
        let show_blur = config_tool == Some(Tool::Blur);
        let show_spotlight = config_tool == Some(Tool::Spotlight);
        let size_scale = self.ui_scale.max(0.1);

        let mut groups: Vec<AnyElement> = Vec::new();
        if show_color {
            let palette_color = self.palette_color();
            let highlighter_palette = self.palette_color_tool() == AnnotationColorTool::Highlighter;
            let mut swatches = Vec::new();
            for (color, name, label_key) in settings_state::ANNOTATION_COLORS {
                let selected = (palette_color & 0xFFFFFF00) == (color & 0xFFFFFF00);
                let label = t(language, label_key).to_owned();
                let preview = if highlighter_palette {
                    core_annotate::highlighter_color(color)
                } else {
                    color
                };
                let swatch = div()
                    .id(name)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded(px(9.0))
                    .bg(rgb((preview >> 8) & 0x00FF_FFFF))
                    .border_2()
                    .border_color(if selected { text } else { rgba(0x00000000) })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_color(color);
                        this.status = None;
                        cx.notify();
                    }));
                swatches.push(swatch.into_any_element());
            }
            // The palette is not limited to the six presets: this swatch opens
            // an exact color panel with a hue strip and a hex field.
            let custom_label = t(language, "editor.custom_color").to_owned();
            let custom_active = self.custom_color_open
                || !settings_state::ANNOTATION_COLORS
                    .iter()
                    .any(|(color, _, _)| (palette_color & 0xFFFFFF00) == (color & 0xFFFFFF00));
            swatches.push(
                div()
                    .id("editor-color-custom")
                    .tooltip(move |_, cx| tooltip_view(custom_label.clone(), cx))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded(px(9.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if custom_active {
                        accent_selected
                    } else {
                        chrome.surface
                    })
                    .border_2()
                    .border_color(if custom_active { text } else { border })
                    .when(!custom_active, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.commit_text(cx);
                        if this.custom_color_open {
                            this.close_custom_color();
                        } else {
                            this.open_custom_color();
                            this.more_open = false;
                            this.effects_open = false;
                            this.hints_visible = false;
                        }
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icons::PALETTE,
                        12.0,
                        if custom_active { accent } else { muted },
                    ))
                    .into_any_element(),
            );
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .children(swatches)
                    .into_any_element(),
            );
        }

        if config_tool == Some(Tool::Rect) {
            let mut options = Vec::new();
            for (ellipse, id, label_key, icon) in [
                (false, "editor-shape-rect", "editor.tool.rect", icons::RECT),
                (
                    true,
                    "editor-shape-ellipse",
                    "editor.tool.ellipse",
                    icons::ELLIPSE,
                ),
            ] {
                let selected = config_ellipse == ellipse;
                let label = t(language, label_key).to_owned();
                let button = div()
                    .id(id)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.document.select(None);
                        this.apply_shape_ellipse(ellipse);
                        this.style.filled = if ellipse {
                            this.ellipse_filled
                        } else {
                            this.rect_filled
                        };
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icon,
                        16.0,
                        if selected { accent } else { muted },
                    ));
                options.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(options)
                    .into_any_element(),
            );
        }

        if show_fill {
            let mut options = Vec::new();
            for (filled, id, label_key, icon) in [
                (
                    false,
                    "editor-fill-outline",
                    "editor.outline",
                    icons::OUTLINE,
                ),
                (true, "editor-fill-solid", "editor.fill", icons::FILL),
            ] {
                let selected = config_filled == filled;
                let label = t(language, label_key).to_owned();
                let button = div()
                    .id(id)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_fill(filled);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icon,
                        16.0,
                        if selected { accent } else { muted },
                    ));
                options.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(options)
                    .into_any_element(),
            );
        }

        if show_width {
            let mut widths = Vec::new();
            for width in STROKE_WIDTHS {
                let selected = (config_style.stroke_width - width).abs() < 0.1;
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
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_stroke_width(width);
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
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(widths)
                    .into_any_element(),
            );
        }

        if show_font {
            let mut sizes = Vec::new();
            for size in FONT_SIZES {
                let selected = (config_style.font_size - size).abs() < 0.1;
                let label = format!("{size:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.font_size"));
                let button = div()
                    .id(format!("editor-font-{label}"))
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_font_size(size);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { accent } else { muted })
                            .child(label),
                    );
                sizes.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::TEXT, 14.0, muted))
                    .children(sizes)
                    .into_any_element(),
            );
        }

        if show_text_style {
            let background_enabled = config_style.text_background;
            let outline_enabled = config_style.text_outline;
            let alignment = config_style.text_alignment;
            let alignment_label = match alignment {
                TextAlignment::Left => t(language, "editor.align_left"),
                TextAlignment::Center => t(language, "editor.align_center"),
                TextAlignment::Right => t(language, "editor.align_right"),
            };
            let background_tooltip = t(language, "editor.text_background").to_owned();
            let outline_tooltip = t(language, "editor.text_outline").to_owned();
            let alignment_tooltip = t(language, "editor.text_alignment").to_owned();
            let background = div()
                .id("editor-text-background")
                .tooltip(move |_, cx| tooltip_view(background_tooltip.clone(), cx))
                .w(px(24.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .when(background_enabled, |button| button.bg(accent_selected))
                .when(!background_enabled, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.commit_text(cx);
                    this.apply_text_background(!this.text_background);
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    icons::FILL,
                    15.0,
                    if background_enabled { accent } else { muted },
                ))
                .into_any_element();
            let outline = div()
                .id("editor-text-outline")
                .tooltip(move |_, cx| tooltip_view(outline_tooltip.clone(), cx))
                .w(px(24.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .when(outline_enabled, |button| button.bg(accent_selected))
                .when(!outline_enabled, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.commit_text(cx);
                    this.apply_text_outline(!this.text_outline);
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    icons::OUTLINE,
                    15.0,
                    if outline_enabled { accent } else { muted },
                ))
                .into_any_element();
            let next_alignment = match alignment {
                TextAlignment::Left => TextAlignment::Center,
                TextAlignment::Center => TextAlignment::Right,
                TextAlignment::Right => TextAlignment::Left,
            };
            let alignment_button = div()
                .id("editor-text-alignment")
                .tooltip(move |_, cx| tooltip_view(alignment_tooltip.clone(), cx))
                .w(px(28.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.apply_text_alignment(next_alignment);
                    this.status = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(accent)
                        .child(alignment_label),
                )
                .into_any_element();
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(background)
                    .child(outline)
                    .child(alignment_button)
                    .into_any_element(),
            );
        }

        if show_arrow_heads {
            let mut buttons = Vec::new();
            for (is_start, enabled, label, tooltip_key, id) in [
                (
                    true,
                    config_style.arrow_start,
                    "←",
                    "editor.arrow_start",
                    "editor-arrow-start",
                ),
                (
                    false,
                    config_style.arrow_end,
                    "→",
                    "editor.arrow_end",
                    "editor-arrow-end",
                ),
            ] {
                let tooltip = t(language, tooltip_key).to_owned();
                let button = div()
                    .id(id)
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(enabled, |button| button.bg(accent_selected))
                    .when(!enabled, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let start = if is_start {
                            !enabled
                        } else {
                            config_style.arrow_start
                        };
                        let end = if is_start {
                            config_style.arrow_end
                        } else {
                            !enabled
                        };
                        this.apply_arrow_heads(start, end);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(if enabled { accent } else { muted })
                            .child(label),
                    );
                buttons.push(button.into_any_element());
            }
            let polyline = self.config_arrow_polyline();
            let tooltip = t(language, "editor.tool.polyline").to_owned();
            buttons.push(
                div()
                    .id("editor-arrow-polyline")
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(polyline, |button| button.bg(accent_selected))
                    .when(!polyline, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.document.select(None);
                        this.apply_arrow_polyline(!polyline);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icons::POLYLINE,
                        14.0,
                        if polyline { accent } else { muted },
                    ))
                    .into_any_element(),
            );
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(buttons)
                    .into_any_element(),
            );
        }

        if show_corner {
            let mut radii = Vec::new();
            for radius in CORNER_RADII {
                let applied = (radius * size_scale).round();
                let selected = (config_style.corner_radius - applied).abs() < 0.1;
                let label = format!("{radius:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.corner_radius"));
                let button = div()
                    .id(format!("editor-corner-{label}"))
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_corner_radius(radius);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { accent } else { muted })
                            .child(label),
                    );
                radii.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::RECT, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }

        if show_number {
            let current = self
                .document
                .selected()
                .and_then(|id| self.document.object(id))
                .and_then(|annotation| match annotation.shape {
                    Shape::Number { bounds, .. } => Some(bounds.size.height),
                    _ => None,
                })
                .unwrap_or(((self.number_size as f32 * size_scale).round() as u32).max(8));
            let mut sizes = Vec::new();
            for size in NUMBER_SIZES {
                let applied = ((size as f32 * size_scale).round() as u32).max(8);
                let selected = current.abs_diff(applied) <= 1;
                let label = format!("{size}");
                let tooltip = format!("{} {label}", t(language, "editor.number_size"));
                let button = div()
                    .id(format!("editor-number-{label}"))
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_number_size(size);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { accent } else { muted })
                            .child(label),
                    );
                sizes.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::NUMBER, 14.0, muted))
                    .children(sizes)
                    .into_any_element(),
            );
        }

        if show_blur {
            let mut radii = Vec::new();
            for radius in BLUR_RADII {
                let selected = config_style.blur_radius == radius;
                let label = format!("{radius}");
                let tooltip = format!("{} {label}", t(language, "editor.blur_radius"));
                let button = div()
                    .id(format!("editor-blur-{label}"))
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_blur_radius(radius);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { accent } else { muted })
                            .child(label),
                    );
                radii.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::BLUR, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }

        if show_spotlight {
            let mut levels = Vec::new();
            for intensity in SPOTLIGHT_INTENSITIES {
                let selected = config_intensity == intensity;
                let label = format!("{intensity}%");
                let tooltip = format!("{} {label}", t(language, "editor.intensity"));
                let button = div()
                    .id(format!("editor-spotlight-{intensity}"))
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(32.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_spotlight_intensity(intensity);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { accent } else { muted })
                            .child(label),
                    );
                levels.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::SPOTLIGHT, 14.0, muted))
                    .children(levels)
                    .into_any_element(),
            );

            let mut radii = Vec::new();
            for radius in CORNER_RADII {
                let applied = (radius * size_scale).round();
                let selected = (config_style.spotlight_radius - applied).abs() < 0.1;
                let label = format!("{radius:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.spotlight_radius"));
                let button = div()
                    .id(format!("editor-spotlight-radius-{label}"))
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_spotlight_radius(radius);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { accent } else { muted })
                            .child(label),
                    );
                radii.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::RECT, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }

        let mut config_children: Vec<AnyElement> = Vec::new();
        for (index, group) in groups.into_iter().enumerate() {
            if index > 0 {
                config_children.push(div().w(px(1.0)).h(px(18.0)).bg(border).into_any_element());
            }
            config_children.push(group);
        }

        let undo_label = t(language, "editor.undo").to_owned();
        let undo = if self.document.can_undo() || !self.canvas_undo.is_empty() {
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
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.document.undo() {
                        this.undo_canvas(cx);
                    }
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
        let redo = if self.document.can_redo() || !self.canvas_redo.is_empty() {
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
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.document.redo() {
                        this.redo_canvas(cx);
                    }
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
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.commit_text(cx);
                    this.delete_selected_annotation(cx);
                }))
                .child(icons::icon(icons::TRASH, 18.0, text))
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
        let pin = (self.pin_target.is_none()).then(|| {
            div()
                .id("editor-pin")
                .tooltip(move |_, cx| tooltip_view(pin_label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| this.pin(cx)))
                .child(icons::icon(icons::PIN, 18.0, text))
                .into_any_element()
        });

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
            .active(|style| style.opacity(0.85))
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
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|this, _, _, cx| this.export(false, cx)))
            .child(icons::icon(icons::SAVE, 18.0, text))
            .into_any_element();

        let capture_actions_enabled = self.inline.and_then(|inline| inline.capture).is_some();
        let mut capture_actions: Vec<AnyElement> = Vec::new();
        if capture_actions_enabled {
            let scroll_label = t(language, "menu.scroll_capture").to_owned();
            capture_actions.push(
                div()
                    .id("editor-scroll-capture")
                    .tooltip(move |_, cx| tooltip_view(scroll_label.clone(), cx))
                    .w(px(TOOLBAR_SIZE as f32))
                    .h(px(TOOLBAR_SIZE as f32))
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |style| style.bg(hover))
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(|this, _, _, cx| this.start_scroll_capture(cx)))
                    .child(icons::icon(icons::SCROLL, 18.0, muted))
                    .into_any_element(),
            );
            let record_label = t(language, "menu.record").to_owned();
            capture_actions.push(
                div()
                    .id("editor-record")
                    .tooltip(move |_, cx| tooltip_view(record_label.clone(), cx))
                    .w(px(TOOLBAR_SIZE as f32))
                    .h(px(TOOLBAR_SIZE as f32))
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |style| style.bg(hover))
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx)))
                    .child(icons::icon(icons::RECORDING, 18.0, muted))
                    .into_any_element(),
            );
        }

        let capture_page = capture_actions_enabled;
        let close_label = t(language, "editor.close").to_owned();
        let more_label = t(language, "editor.more").to_owned();
        let more = (!capture_page).then(|| {
            div()
                .id("editor-more")
                .tooltip(move |_, cx| tooltip_view(more_label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .when(self.more_open, |button| button.bg(chrome.selected))
                .hover(move |style| style.bg(hover))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.more_open = !this.more_open;
                    this.effects_open = false;
                    this.hints_visible = false;
                    cx.notify();
                }))
                .child(icons::icon(icons::MORE, 18.0, muted))
                .into_any_element()
        });

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
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|this, _, window, cx| {
                this.release_images(cx);
                if this.inline.is_some() {
                    let handles = cx.windows();
                    cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
                } else {
                    window.remove_window();
                }
            }))
            .child(icons::icon(icons::CLOSE, 18.0, muted))
            .into_any_element();

        let bar_rect = self.toolbar_rect(window);
        let options_rect = self.options_rect(window, bar_rect);
        let options_panel = (!config_children.is_empty() && !self.toolbar_hidden).then(|| {
            let top = f32::from(options_rect.origin.y);
            let offset = if top >= f32::from(bar_rect.origin.y) + f32::from(bar_rect.size.height) {
                4.0
            } else {
                -4.0
            };
            crate::ui::enter_slide_fade(
                div()
                    .id("editor-options-panel")
                    .absolute()
                    .left(options_rect.origin.x)
                    .top(options_rect.origin.y)
                    .max_w(options_rect.size.width)
                    .h(options_rect.size.height)
                    .px(px(10.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(chrome.radius))
                    .bg(chrome.surface_strong)
                    .border_1()
                    .border_color(border)
                    .shadow(chrome.shadow())
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .children(config_children),
                "editor-options-enter",
                top,
                offset,
                Duration::from_millis(120),
            )
            .into_any_element()
        });

        let more_panel = (self.more_open && !self.toolbar_hidden && !capture_page).then(|| {
            let more_rect = self.more_rect(window, bar_rect);
            let mut panel = div()
                .id("editor-more-panel")
                .absolute()
                .left(more_rect.origin.x)
                .top(more_rect.origin.y)
                .w(more_rect.size.width)
                .flex()
                .flex_col()
                .gap(px(2.0))
                .p(px(5.0))
                .rounded(px(chrome.radius))
                .bg(chrome.surface_strong)
                .border_1()
                .border_color(border)
                .shadow(chrome.shadow())
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                )
                .child(self.more_row(
                    "editor-more-crop",
                    icons::CROP,
                    t(language, "editor.crop"),
                    cx,
                    |this, cx| this.toggle_crop(cx),
                ));

            // Long captures only support cropping; rotation and flipping are
            // reserved for pins, which keep the full image transform set.
            if self.pin_target.is_some() {
                panel = panel
                    .child(self.more_row(
                        "editor-more-rotate",
                        icons::ROTATE,
                        t(language, "editor.rotate"),
                        cx,
                        |this, cx| this.rotate_canvas(cx),
                    ))
                    .child(self.more_row(
                        "editor-more-flip-horizontal",
                        icons::FLIP_H,
                        t(language, "editor.flip_horizontal"),
                        cx,
                        |this, cx| this.flip_canvas(true, cx),
                    ))
                    .child(self.more_row(
                        "editor-more-flip-vertical",
                        icons::FLIP_V,
                        t(language, "editor.flip_vertical"),
                        cx,
                        |this, cx| this.flip_canvas(false, cx),
                    ));
            }
            let top = f32::from(more_rect.origin.y);
            let offset = if top >= f32::from(bar_rect.origin.y) + f32::from(bar_rect.size.height) {
                4.0
            } else {
                -4.0
            };
            crate::ui::enter_slide_fade(
                panel.h(more_rect.size.height),
                "editor-more-enter",
                top,
                offset,
                Duration::from_millis(120),
            )
        });

        let more_panel = more_panel.map(|panel| {
            div()
                .id("editor-more-layer")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.more_open = false;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element()
        });

        if self.toolbar_hidden {
            return (div().into_any_element(), None, more_panel);
        }

        let bar = div()
            .id("editor-toolbar")
            .absolute()
            .left(bar_rect.origin.x)
            .top(bar_rect.origin.y)
            .w(bar_rect.size.width)
            .h(bar_rect.size.height)
            .px(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .rounded(px(chrome.radius))
            .bg(chrome.surface)
            .border_1()
            .border_color(border)
            .shadow(chrome.shadow())
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(1.0))
                    .children(tools)
                    .child(div().w(px(5.0)))
                    .child(div().w(px(1.0)).h(px(18.0)).bg(border))
                    .child(div().w(px(5.0)))
                    .child(undo)
                    .child(redo)
                    .child(delete),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(1.0))
                    .children(pin)
                    .child(copy)
                    .child(save)
                    .children(capture_actions)
                    .children(more)
                    .child(close),
            );
        (bar.into_any_element(), options_panel, more_panel)
    }

    fn more_row(
        &self,
        id: &'static str,
        icon: &'static [u8],
        label: &'static str,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(5.0))
            .hover(|style| style.bg(self.chrome.hover))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.more_open = false;
                action(this, cx);
                cx.notify();
            }))
            .child(icons::icon(icon, 15.0, self.chrome.icon))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(self.chrome.text)
                    .child(label),
            )
            .into_any_element()
    }

    fn region_effects_active(&self) -> bool {
        if !self.effects.active() {
            return false;
        }
        match self.inline.and_then(|inline| inline.capture) {
            Some(capture) => !matches!(capture.target, CaptureTarget::Window(_)),
            // Pin editors keep the capture effects without an inline capture
            // descriptor; long-capture editors never have region effects.
            None => self.pin_target.is_some(),
        }
    }

    /// Effects worth carrying into a pin document. Window captures already
    /// have their system frame and must not inherit a stale region radius.
    fn stored_region_effects(&self) -> RegionEffects {
        if self.region_effects_active() {
            self.effects
        } else {
            RegionEffects::default()
        }
    }

    fn region_effects_scale(&self) -> f32 {
        self.inline
            .and_then(|inline| inline.capture)
            .map(|capture| capture.scale)
            .unwrap_or(1.0)
            .max(0.1)
    }

    fn info_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let inline = self.inline?;
        let capture = inline.capture?;
        let rect = self.info_rect(window)?;
        let language = settings_state::get().language;
        let absolute_x = capture.monitor_origin.x + capture.region.rect.origin.x;
        let absolute_y = capture.monitor_origin.y + capture.region.rect.origin.y;
        let width = capture.region.rect.size.width;
        let height = capture.region.rect.size.height;
        let mut bar = div()
            .id("editor-info-bar")
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
            .gap(px(8.0))
            .px(px(8.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .text_size(px(11.0))
            .child(
                div()
                    .text_color(self.chrome.muted)
                    .child(format!("{absolute_x}, {absolute_y}")),
            )
            .child(div().w(px(1.0)).h(px(14.0)).bg(self.chrome.separator))
            .child(
                div()
                    .text_color(self.chrome.text)
                    .child(format!("{width} x {height} px")),
            );

        if !matches!(capture.target, CaptureTarget::Window(_)) {
            let effects_active = self.effects.active();
            bar = bar.child(
                div()
                    .id("editor-info-effects")
                    .px(px(6.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .when(self.effects_open, |chip| chip.bg(self.chrome.selected))
                    .hover(|style| style.bg(self.chrome.hover))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.effects_open = !this.effects_open;
                        this.more_open = false;
                        this.hints_visible = false;
                        cx.notify();
                    }))
                    .child(t(language, "overlay.info_effects"))
                    .when(effects_active, |chip| {
                        chip.child(
                            div()
                                .w(px(5.0))
                                .h(px(5.0))
                                .rounded(px(3.0))
                                .bg(self.chrome.accent),
                        )
                    }),
            );
        }

        bar = bar.child(
            div()
                .id("editor-info-refresh")
                .tooltip(move |_, cx| {
                    tooltip_view(t(language, "overlay.info_refresh").to_owned(), cx)
                })
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|style| style.bg(self.chrome.hover))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.refresh_capture(cx)))
                .child(icons::icon(icons::REFRESH, 14.0, self.chrome.icon)),
        );

        Some(bar.into_any_element())
    }

    fn effects_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.effects_open {
            return None;
        }
        let rect = self.effects_rect(window)?;
        let language = settings_state::get().language;
        let mut corner_row = div().flex().flex_row().items_center().gap(px(3.0));
        for radius in [0u32, 8, 16, 24] {
            let selected_radius = self.effects.corner_radius == radius;
            let label = if radius == 0 {
                t(language, "overlay.effects_off").to_owned()
            } else {
                radius.to_string()
            };
            corner_row = corner_row.child(
                div()
                    .id(format!("editor-radius-{radius}"))
                    .px(px(7.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .when(selected_radius, |chip| chip.bg(self.chrome.accent))
                    .when(!selected_radius, |chip| {
                        chip.hover(|style| style.bg(self.chrome.hover))
                    })
                    .text_size(px(10.5))
                    .text_color(if selected_radius {
                        self.chrome.accent_text
                    } else {
                        self.chrome.muted
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.effects.corner_radius = radius;
                        settings_state::update(|settings| {
                            settings.region_corner_radius = radius;
                        });
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        let panel = div()
            .id("editor-effects-panel")
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(self.chrome.muted)
                    .child(t(language, "overlay.effects_corner")),
            )
            .child(corner_row)
            .child({
                let shadow = self.effects.shadow;
                div()
                    .id("editor-effects-shadow")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(7.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .when(shadow, |chip| chip.bg(self.chrome.accent))
                    .when(!shadow, |chip| {
                        chip.hover(|style| style.bg(self.chrome.hover))
                    })
                    .text_size(px(10.5))
                    .text_color(if shadow {
                        self.chrome.accent_text
                    } else {
                        self.chrome.muted
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.effects.shadow = !this.effects.shadow;
                        let shadow = this.effects.shadow;
                        settings_state::update(|settings| settings.region_shadow = shadow);
                        cx.notify();
                    }))
                    .child(t(language, "overlay.effects_shadow"))
            });
        let panel = crate::ui::enter_slide_fade(
            panel,
            "editor-effects-enter",
            f32::from(rect.origin.y),
            4.0,
            Duration::from_millis(120),
        );

        Some(
            div()
                .id("editor-effects-layer")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.effects_open = false;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    fn hints_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.hints_visible {
            return None;
        }
        let language = settings_state::get().language;
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 340.0_f32.min((window_width - 16.0).max(120.0));
        let height = 220.0;
        let panel = div()
            .id("editor-hints-panel")
            .absolute()
            .left(px((window_width - width) / 2.0))
            .top(px((window_height - height) / 2.0))
            .w(px(width))
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(5.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(self.chrome.text)
                    .child(t(language, "editor.hints_title")),
            );
        let pin_hotkey = crate::hotkey::display_hotkey(&settings_state::get().pin_hotkey);
        let mut rows: Vec<(String, &'static str)> = vec![
            (
                t(language, "editor.hint_key_copy").to_owned(),
                t(language, "editor.hint_copy"),
            ),
            ("Cmd/Ctrl+S".to_owned(), t(language, "editor.hint_save")),
        ];
        if self.inline.and_then(|inline| inline.capture).is_some() {
            rows.push((pin_hotkey, t(language, "editor.hint_pin")));
        }
        rows.push(if self.inline.is_some_and(|inline| inline.zoomable) {
            ("Space".to_owned(), t(language, "editor.hint_pan"))
        } else {
            ("Space".to_owned(), t(language, "editor.hint_space"))
        });
        if self.inline.and_then(|inline| inline.capture).is_some() {
            rows.push(("F5".to_owned(), t(language, "editor.hint_refresh")));
        }
        rows.push(("Esc".to_owned(), t(language, "editor.hint_cancel")));
        let mut panel = panel;
        for (key, description) in rows {
            panel = panel.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(self.chrome.muted)
                            .child(key),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .child(description),
                    ),
            );
        }
        let panel = crate::ui::enter_slide_fade(
            panel,
            "editor-hints-enter",
            (window_height - height) / 2.0,
            4.0,
            Duration::from_millis(120),
        );
        Some(
            div()
                .id("editor-hints-layer")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.hints_visible = false;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    fn refresh_capture(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let mut backend = core_capture::platform_backend();
        let captured = if settings_state::get().capture_cursor {
            backend.capture_with_cursor(capture.target)
        } else {
            backend.capture(capture.target)
        };
        let frame = match captured {
            Ok(frame) => match capture.target {
                CaptureTarget::Window(id) => {
                    crate::window_frame::compose_window_capture(id, frame, capture.scale)
                }
                _ => frame,
            },
            Err(error) => {
                self.show_status(
                    format!(
                        "{} {}",
                        t(settings_state::get().language, "editor.refresh_failed"),
                        error
                    ),
                    cx,
                );
                return;
            }
        };
        if matches!(capture.target, CaptureTarget::Region(_))
            && let Ok(source) = core_capture::platform_backend()
                .capture(CaptureTarget::Monitor(capture.region.monitor))
        {
            self.region_source = Some(image_util::render_image(source));
        }
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        let size_changed = image_size != self.image_size;
        self.frame = Arc::new(frame);
        self.image_size = image_size;
        self.refresh_frame_images(cx);
        if size_changed {
            self.document = AnnotationDocument::new(image_size);
            self.canvas_undo.clear();
            self.canvas_redo.clear();
            self.text_edit = None;
            self.draft = None;
            self.move_draft = None;
            self.resize_draft = None;
            self.crop_mode = false;
            self.crop_draft = None;
            self.polyline_draft = None;
        }
        if let Some(inline) = self.inline.as_mut() {
            let scale = capture.scale.max(0.1);
            let center_x = (capture.region.rect.origin.x as f32
                + capture.region.rect.size.width as f32 / 2.0)
                / scale;
            let center_y = (capture.region.rect.origin.y as f32
                + capture.region.rect.size.height as f32 / 2.0)
                / scale;
            let image_width = self.image_size.width as f32 / scale;
            let image_height = self.image_size.height as f32 / scale;
            inline.image_rect = Bounds {
                origin: point(
                    px(center_x - image_width / 2.0),
                    px(center_y - image_height / 2.0),
                ),
                size: size(px(image_width), px(image_height)),
            };
        }
        self.show_status(
            t(settings_state::get().language, "editor.refreshed").to_owned(),
            cx,
        );
    }

    fn start_scroll_capture(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        self.commit_text(cx);
        let first_frame = match rasterize(&self.document, &self.frame) {
            Ok(frame) => frame,
            Err(error) => {
                self.show_status(
                    format!(
                        "{}{}",
                        t(settings_state::get().language, "editor.export_failed"),
                        error
                    ),
                    cx,
                );
                return;
            }
        };
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
            crate::scroll::open_session(capture.region, first_frame, cx);
        });
    }

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        self.commit_text(cx);
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
            crate::record::start(capture.target, cx);
        });
    }

    pub(crate) fn pin(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let inline_host = self.inline;
        match rasterize(&self.document, &self.frame) {
            Ok(output) => {
                let output = if self.region_effects_active() {
                    self.effects.apply(output, self.region_effects_scale())
                } else {
                    output
                };
                let hash = crate::capture_history::hash_frame(&output);
                let placement = inline_host.map(|inline| {
                    // The editor image may already be zoomed; use its displayed
                    // scale so the pin opens at the same size and center. This
                    // also keeps pins on the same monitor as the capture, long
                    // capture, or source pin.
                    let display_scale = if self.image_size.width == 0 {
                        1.0
                    } else {
                        f32::from(inline.image_rect.size.width) / self.image_size.width as f32
                    }
                    .max(0.01);
                    let width = output.width() as f32 * display_scale;
                    let height = output.height() as f32 * display_scale;
                    let center_x = f32::from(inline.screen_origin.x)
                        + f32::from(inline.image_rect.origin.x)
                        + f32::from(inline.image_rect.size.width) / 2.0;
                    let center_y = f32::from(inline.screen_origin.y)
                        + f32::from(inline.image_rect.origin.y)
                        + f32::from(inline.image_rect.size.height) / 2.0;
                    (
                        Bounds {
                            origin: point(px(center_x - width / 2.0), px(center_y - height / 2.0)),
                            size: size(px(width), px(height)),
                        },
                        display_scale,
                    )
                });
                let monitor = inline_host
                    .and_then(|inline| inline.capture)
                    .map(|capture| capture.region.monitor);
                // Window captures already include their composed edge shadow.
                // Region captures only bake one when the shadow effect is on;
                // long-capture results never do, so those pins use the native
                // window shadow instead. Rounded frames without a composed
                // shadow skip the native one: a rectangular DWM shadow around
                // transparent corners reads as square.
                let native_shadow = inline_host.is_some_and(|inline| match inline.capture {
                    Some(capture) => {
                        matches!(capture.target, CaptureTarget::Region(_))
                            && !self.effects.shadow
                            && self.effects.corner_radius == 0
                    }
                    None => self.pin_target.is_none(),
                });
                if inline_host.is_some() {
                    self.release_images(cx);
                }
                let handles = cx.windows();
                let edit_state = crate::pin::PinEditState::new(
                    self.frame.clone(),
                    self.document.clone(),
                    self.stored_region_effects(),
                );
                // Pin windows must not be created inside the toolbar's input
                // callback; close the editor first and open the pin next turn.
                cx.defer(move |cx| {
                    if inline_host.is_some() {
                        crate::windows::close_capture_auxiliary(cx, handles);
                    }
                    let id = if let Some((bounds, zoom)) = placement {
                        crate::pin::open_frame_at_with_hash(
                            output,
                            bounds,
                            zoom,
                            hash,
                            monitor,
                            native_shadow,
                            cx,
                        )
                    } else {
                        crate::pin::open_frame_with_hash(output, hash, native_shadow, cx)
                    };
                    if let Some(id) = id {
                        crate::pin::set_edit_state(id, edit_state);
                    }
                });
            }
            Err(error) => {
                let language = settings_state::get().language;
                self.show_status(
                    format!("{}{}", t(language, "editor.export_failed"), error),
                    cx,
                );
            }
        }
        log_info!("editor", "pin requested");
        cx.notify();
    }

    fn export(&mut self, copy_only: bool, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let language = settings_state::get().language;
        let mut messages = Vec::new();
        let mut copy_ok = false;
        let mut save_ok = false;
        let mut pin_ok = false;
        match rasterize(&self.document, &self.frame) {
            Ok(output) => {
                let output = if self.region_effects_active() {
                    self.effects.apply(output, self.region_effects_scale())
                } else {
                    output
                };
                let hash = crate::capture_history::hash_frame(&output);
                let record_history = self.pin_target.is_none();
                if copy_only || settings_state::get().copy_to_clipboard {
                    match image_util::encode_png(&output) {
                        Ok(bytes) => {
                            copy_ok = true;
                            if record_history {
                                crate::capture_history::record_png_bytes(&bytes, hash);
                            }
                            image_util::copy_png_bytes(bytes, cx);
                            messages.push(t(language, "editor.copied").to_owned());
                        }
                        Err(error) => {
                            messages.push(format!("{}{}", t(language, "editor.copy_failed"), error))
                        }
                    }
                }
                if !copy_only && self.pin_target.is_none() {
                    match image_util::save_png(&output) {
                        Ok(path) => {
                            save_ok = true;
                            crate::capture_history::record_png_file(&path, hash);
                            messages.push(format!(
                                "{} {}",
                                t(language, "editor.saved"),
                                path.display()
                            ));
                        }
                        Err(error) => messages.push(format!(
                            "{} {}",
                            t(language, "editor.save_failed"),
                            error
                        )),
                    }
                }
                if !copy_only && let Some(pin_id) = self.pin_target {
                    let edit_state = crate::pin::PinEditState::new(
                        self.frame.clone(),
                        self.document.clone(),
                        self.stored_region_effects(),
                    );
                    if crate::pin::replace_image_with_edit_state(pin_id, output, edit_state, cx) {
                        pin_ok = true;
                        messages.push(t(language, "editor.pin_updated").to_owned());
                    } else {
                        messages.push(t(language, "editor.pin_missing").to_owned());
                    }
                }
            }
            Err(error) => {
                messages.push(format!("{}{}", t(language, "editor.export_failed"), error))
            }
        }
        self.show_status(messages.join(" · "), cx);
        let close = if copy_only {
            copy_ok
        } else if self.pin_target.is_some() {
            pin_ok
        } else {
            save_ok
        };
        log_info!(
            "editor",
            "export finished: copy_only={copy_only} copy_ok={copy_ok} save_ok={save_ok} close={close}"
        );
        if close && self.inline.is_some() {
            self.release_images(cx);
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
        } else {
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
            // A committed IME result arrives with no range; `replace` then
            // consumes the marked composition range before inserting it.
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

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.chrome = ChromeTheme::for_window(window, Some(self.accent_hex));
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
        let objects = self.document.objects().to_vec();
        let crop_selection = self
            .crop_draft
            .as_ref()
            .map(|draft| normalized_rect(draft.start, draft.current));
        let preview = self.preview_shape();
        let preview_blur = preview.as_ref().and_then(|(shape, style)| match shape {
            Shape::Blur(rect) if !rect.is_empty() => Some((*rect, style.blur_radius)),
            _ => None,
        });
        let status = self.status.clone();
        let status_serial = self.status_serial;
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
                    let blur_image = self.blur_image(annotation.style.blur_radius, cx);
                    object_layers.push(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .w(width)
                            .h(height)
                            .overflow_hidden()
                            .child(
                                img(blur_image)
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
                    let color = (annotation.style.color >> 8) & 0x00FF_FFFF;
                    let text_color = rgb(color);
                    let text_align = match annotation.style.text_alignment {
                        TextAlignment::Left => TextAlign::Left,
                        TextAlignment::Center => TextAlign::Center,
                        TextAlignment::Right => TextAlign::Right,
                    };
                    let font_size = (annotation.style.font_size * scale).max(8.0);
                    let mut layer = div()
                        .absolute()
                        .left(px(bounds.origin.x as f32 * scale))
                        .top(px(bounds.origin.y as f32 * scale))
                        .w(px(bounds.size.width as f32 * scale))
                        .text_size(px(font_size))
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
                        let offset =
                            ((annotation.style.stroke_width * 0.6 * scale).max(1.0) as i32).min(3);
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
                                    .text_align(text_align)
                                    .text_color(outline)
                                    .child(text.clone()),
                            );
                        }
                    }
                    object_layers.push(
                        layer
                            .child(div().w_full().text_color(text_color).child(text.clone()))
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
                if let Some(rect) = crop_selection {
                    paint_crop_overlay(window, bounds, scale, rect, accent_hex);
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
            let line_height = font_size * 1.3;
            let color = edit.style.color;
            let text_rgb = (color >> 8) & 0x00FF_FFFF;
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
            let layout = layout_annotation_text(window, &edit.content, font_size, width);
            let boxes = layout
                .as_ref()
                .map(|layout| text_line_boxes(layout, line_height))
                .unwrap_or_default();
            let caret = text_caret_position(layout.as_ref(), &boxes, edit, width);
            let selection = edit.selection();
            let selection_boxes = layout
                .as_ref()
                .map(|layout| {
                    text_selection_boxes(
                        layout,
                        &boxes,
                        &selection,
                        edit.style.text_alignment,
                        line_height,
                        width,
                    )
                })
                .unwrap_or_default();
            let selection_color = rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x66);
            let mut layer = div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .min_h(px(line_height))
                .text_size(px(font_size))
                .text_color(if edit.content.is_empty() {
                    rgb(accent_hex)
                } else {
                    rgb(text_rgb)
                })
                .text_align(match edit.style.text_alignment {
                    TextAlignment::Left => TextAlign::Left,
                    TextAlignment::Center => TextAlign::Center,
                    TextAlignment::Right => TextAlign::Right,
                });
            if edit.style.text_background {
                layer =
                    layer
                        .rounded(px(font_size * 0.2))
                        .bg(if prefers_dark_foreground(text_rgb) {
                            rgba(0x000000A0)
                        } else {
                            rgba(0xFFFFFFBE)
                        });
            }
            for rect in selection_boxes {
                layer = layer.child(
                    div()
                        .absolute()
                        .left(rect.origin.x)
                        .top(rect.origin.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .bg(selection_color),
                );
            }
            if !edit.content.is_empty() {
                layer = layer.child(edit.content.clone());
            }
            layer = layer.child(
                div()
                    .absolute()
                    .left(caret.x)
                    .top(caret.y)
                    .w(px(1.5))
                    .h(px(line_height))
                    .bg(if edit.content.is_empty() {
                        rgb(accent_hex)
                    } else {
                        rgb(text_rgb)
                    })
                    .with_animation(
                        SharedString::from("editor-text-caret"),
                        crate::ui::caret_blink(),
                        |caret, delta| caret.opacity(delta),
                    ),
            );
            layer
                .child(input)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        let index = this.text_index_at(window, event.position).unwrap_or(0);
                        if event.click_count >= 2 {
                            this.select_text_word(index, cx);
                        } else if let Some(edit) = this.text_edit.as_mut() {
                            edit.move_cursor(index, false);
                            edit.selecting = true;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        if event.pressed_button.is_none()
                            && let Some(edit) = this.text_edit.as_mut()
                        {
                            edit.selecting = false;
                        }
                        return;
                    }
                    if !this.text_edit.as_ref().is_some_and(|edit| edit.selecting) {
                        return;
                    }
                    let index = this.text_index_at(window, event.position).unwrap_or(0);
                    if let Some(edit) = this.text_edit.as_mut() {
                        edit.move_cursor(index, true);
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        if let Some(edit) = this.text_edit.as_mut() {
                            edit.selecting = false;
                            if edit.selection().is_empty() {
                                edit.anchor = edit.cursor;
                            }
                        }
                        cx.notify();
                    }),
                )
                .into_any_element()
        });

        let preview_blur_layer = preview_blur.map(|(rect, radius)| {
            let left = rect.origin.x as f32 * scale;
            let top = rect.origin.y as f32 * scale;
            let blur_image = self.blur_image(radius, cx);
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(rect.size.width as f32 * scale))
                .h(px(rect.size.height as f32 * scale))
                .overflow_hidden()
                .border_1()
                .border_color(rgba((accent_hex << 8) | 0xCC))
                .child(
                    img(blur_image)
                        .absolute()
                        .left(px(-left))
                        .top(px(-top))
                        .w(image_rect.size.width)
                        .h(image_rect.size.height)
                        .object_fit(ObjectFit::Fill),
                )
                .into_any_element()
        });

        let effects_active = self.region_effects_active();
        let inline = self.inline.is_some();
        let image_cursor = if self.move_draft.is_some() {
            crate::ui::closed_hand_cursor()
        } else {
            match self.selection_hover {
                SelectionHover::Handle(handle) => handle_cursor(handle),
                // Any annotation under the pointer can be selected and moved,
                // even while a drawing tool is active.
                SelectionHover::Body => crate::ui::open_hand_cursor(),
                SelectionHover::None if self.tool == Tool::Select => CursorStyle::Arrow,
                SelectionHover::None => CursorStyle::Crosshair,
            }
        };
        let outside_backdrop = inline.then(|| {
            div().absolute().inset_0().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    // The image layer blocks this layer for clicks on the
                    // image; keep the bounds check as a safeguard for the
                    // first frame after the image rect changes.
                    if this.image_view_rect(window).contains(&event.position) {
                        cx.stop_propagation();
                        return;
                    }
                    if this.document.selected().is_some()
                        || this.more_open
                        || this.effects_open
                        || this.hints_visible
                    {
                        this.select_annotation(None);
                        this.more_open = false;
                        this.effects_open = false;
                        this.hints_visible = false;
                        this.selection_hover = SelectionHover::None;
                        cx.notify();
                    }
                }),
            )
        });
        let image_layer = div()
            .absolute()
            .left(image_rect.origin.x)
            .top(image_rect.origin.y)
            .w(image_rect.size.width)
            .h(image_rect.size.height)
            .cursor(image_cursor)
            .rounded(px(if effects_active {
                self.effects.corner_radius as f32
            } else {
                0.0
            }))
            .overflow_hidden()
            // Block the inline backdrop behind the image: without this the
            // same click selected an object and then reached the backdrop,
            // which immediately cleared the selection again.
            .occlude()
            .child(
                img(self.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .children(object_layers)
            .children(preview_blur_layer)
            .child(overlay_canvas)
            .children(text_overlay)
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let zoomable = this.inline.is_some_and(|inline| inline.zoomable);
                if this.inline.is_some() && !zoomable {
                    cx.stop_propagation();
                    return;
                }
                let (x, y) = match event.delta {
                    ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
                    ScrollDelta::Lines(delta) => (delta.x * 50.0, delta.y * 50.0),
                };
                if event.modifiers.control || event.modifiers.platform {
                    let steps = (y / 50.0).clamp(-4.0, 4.0);
                    this.zoom_at(1.15f32.powf(steps), event.position, window, cx);
                } else {
                    this.view_pan.x += px(x);
                    this.view_pan.y += px(y);
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    // Clicking the image must keep keyboard focus in the
                    // editor, otherwise Delete/arrow shortcuts stop working
                    // after a toolbar interaction.
                    let focus = this.focus_handle.clone();
                    window.focus(&focus, cx);
                    if this.space_down && this.text_edit.is_none() {
                        this.pan_draft = Some(PanDraft {
                            start: event.position,
                            start_pan: this.view_pan,
                        });
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    let Some(point) = this.to_image_point(event.position, window) else {
                        return;
                    };
                    // A previous gesture can outlive its mouse-up when the
                    // pointer leaves the window or the platform consumes the
                    // release. Every new press starts from a clean transform.
                    this.move_draft = None;
                    this.resize_draft = None;
                    // Selection handles stay live regardless of the active
                    // drawing tool: after drawing a shape the tool does not
                    // switch to Select, and dragging a painted handle should
                    // still resize instead of starting another draft.
                    let scale = this.image_scale(window);
                    if let Some(annotation) = this
                        .document
                        .selected()
                        .and_then(|id| this.document.object(id))
                        .cloned()
                        && let Some(handle) = annotation_handle_at(&annotation.shape, point, scale)
                    {
                        this.resize_draft = Some(ResizeDraft {
                            id: annotation.id,
                            handle,
                            start: point,
                            current: point,
                            original: annotation.shape,
                            recorded: false,
                        });
                        this.move_draft = None;
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    if this.crop_mode {
                        this.crop_draft = Some(CropDraft {
                            start: point,
                            current: point,
                        });
                        cx.notify();
                        return;
                    }
                    if this.text_edit.is_some() {
                        this.commit_text(cx);
                    }
                    let hit = this.document.hit_test(point);
                    // With nothing selected, the image edge is a live crop
                    // handle. Clicking an object still wins so edge-adjacent
                    // annotations remain selectable.
                    if hit.is_none()
                        && this.can_adjust_region()
                        && let Some(handle) = edge_resize_handle(
                            PhysicalRect::new(0, 0, this.image_size.width, this.image_size.height),
                            point,
                            (8.0 / scale).max(1.0),
                        )
                    {
                        this.begin_region_drag(handle, cx);
                        cx.stop_propagation();
                        return;
                    }
                    // Text edits are a double-click affordance on the object
                    // itself, not on the Select tool, so a just-created text
                    // annotation can be reopened without switching tools.
                    if event.click_count >= 2 && hit.is_some() && this.edit_text_at(point, cx) {
                        let focus = this.focus_handle.clone();
                        window.focus(&focus, cx);
                        cx.stop_propagation();
                        return;
                    }
                    // Clicking any existing object selects it and starts a
                    // move, even while a drawing tool is still active. Without
                    // this, only the shape that had just been drawn could be
                    // moved; older annotations could never be selected again.
                    if this.polyline_draft.is_none()
                        && let Some(id) = hit
                    {
                        log_info!(
                            "editor",
                            "annotation selected at image ({}, {}), {} object(s)",
                            point.x,
                            point.y,
                            this.document.objects().len()
                        );
                        this.draft = None;
                        this.resize_draft = None;
                        if this.document.selected() != Some(id) {
                            this.select_annotation(Some(id));
                        }
                        if let Some(original) =
                            this.document.object(id).map(|item| item.shape.clone())
                        {
                            this.move_draft = Some(MoveDraft {
                                id,
                                start: point,
                                current: point,
                                original,
                                recorded: false,
                            });
                        }
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    // Starting another annotation with a drawing tool drops
                    // the previous selection before the new draft begins.
                    if this.tool != Tool::Select && this.document.selected().is_some() {
                        this.document.select(None);
                        this.selection_hover = SelectionHover::None;
                    }
                    if event.click_count >= 2
                        && this.inline.is_some()
                        && this.polyline_draft.is_none()
                        && this.text_edit.is_none()
                        && this.document.hit_test(point).is_none()
                    {
                        this.export(true, cx);
                        return;
                    }
                    if this.tool == Tool::Arrow && this.arrow_polyline {
                        this.resize_draft = None;
                        this.move_draft = None;
                        if event.click_count >= 2 {
                            if let Some(draft) = &mut this.polyline_draft {
                                draft.current = point;
                            }
                            this.finish_polyline(cx);
                            return;
                        }
                        if let Some(draft) = &mut this.polyline_draft {
                            if distance(*draft.points.last().unwrap_or(&point), point) > 2.0 {
                                draft.points.push(point);
                            }
                            draft.current = point;
                        } else {
                            this.polyline_draft = Some(PolylineDraft {
                                points: vec![point],
                                current: point,
                            });
                        }
                        this.show_status(
                            t(settings_state::get().language, "editor.polyline_hint").to_owned(),
                            cx,
                        );
                        return;
                    }
                    if this.tool == Tool::Select {
                        let selected = hit;
                        this.resize_draft = None;
                        this.select_annotation(selected);
                        this.move_draft = selected.and_then(|id| {
                            let original = this.document.object(id)?.shape.clone();
                            Some(MoveDraft {
                                id,
                                start: point,
                                current: point,
                                original,
                                recorded: false,
                            })
                        });
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    this.resize_draft = None;
                    this.move_draft = None;
                    if this.tool == Tool::Number {
                        let side =
                            ((this.number_size as f32 * this.ui_scale).round() as u32).max(8);
                        let max_left = (this.image_size.width.saturating_sub(side)) as i32;
                        let max_top = (this.image_size.height.saturating_sub(side)) as i32;
                        let left = (point.x - side as i32 / 2).clamp(0, max_left);
                        let top = (point.y - side as i32 / 2).clamp(0, max_top);
                        let value = this.document.next_number();
                        this.document.add(
                            Shape::Number {
                                bounds: PhysicalRect::new(left, top, side, side),
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
                        shift: event.modifiers.shift,
                        ellipse: this.tool == Tool::Rect && this.shape_ellipse,
                    });
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some(pan_draft) = &this.pan_draft {
                    this.view_pan.x =
                        pan_draft.start_pan.x + (event.position.x - pan_draft.start.x);
                    this.view_pan.y =
                        pan_draft.start_pan.y + (event.position.y - pan_draft.start.y);
                    cx.notify();
                    return;
                }
                let Some(point) = this.to_image_point(event.position, window) else {
                    return;
                };
                // Transforms commit on every move and end on mouse-up or
                // mouse-exit. Do not cancel on `pressed_button == None`: some
                // Windows move events omit the button state, which made a
                // selected object impossible to drag.
                if this.move_draft.is_some() || this.resize_draft.is_some() {
                    let changed = if this.resize_draft.is_some() {
                        this.update_resize(point)
                    } else {
                        this.update_move(point)
                    };
                    if changed {
                        cx.notify();
                    }
                    return;
                }
                if this.update_selection_hover(point, window) {
                    cx.notify();
                }
                if this.crop_mode {
                    if let Some(crop_draft) = &mut this.crop_draft {
                        crop_draft.current = point;
                        cx.notify();
                    }
                    return;
                }
                if let Some(polyline) = &mut this.polyline_draft {
                    polyline.current = point;
                    cx.notify();
                    return;
                }
                let Some(draft) = &mut this.draft else {
                    return;
                };
                draft.current = point;
                draft.shift = event.modifiers.shift;
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.crop_mode {
                        if let Some(point) = this.to_image_point(event.position, window)
                            && let Some(crop_draft) = &mut this.crop_draft
                        {
                            crop_draft.current = point;
                        }
                        cx.notify();
                        return;
                    }
                    if this.polyline_draft.is_some() {
                        cx.notify();
                        return;
                    }
                    if this.pan_draft.take().is_some() {
                        cx.notify();
                        return;
                    }
                    if this.resize_draft.is_some() {
                        let point = this.resize_draft.as_ref().map(|draft| draft.current);
                        if let Some(point) = point {
                            this.finish_resize(point, cx);
                        } else {
                            this.resize_draft = None;
                            cx.notify();
                        }
                        return;
                    }
                    if this.move_draft.take().is_some() {
                        // The move is already committed by the last pointer
                        // move. Never reapply the mouse-up position: some
                        // platforms report it late or not at all, and using a
                        // stale value would send the object back or jump it.
                        this.status = None;
                        cx.notify();
                        return;
                    }

                    let Some(mut draft) = this.draft.take() else {
                        return;
                    };
                    let point = this
                        .to_image_point(event.position, window)
                        .unwrap_or(draft.current);
                    draft.current = point;
                    draft.shift = event.modifiers.shift;
                    if let Some(shape) = finalize_shape(&draft, point) {
                        this.document.add(shape, this.effective_style());
                        this.status = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_exit(cx.listener(|this, _, _, cx| {
                let hover_reset = this.selection_hover != SelectionHover::None;
                this.selection_hover = SelectionHover::None;
                // A drag that ends outside the image keeps the geometry it
                // already committed on the last move; only the gesture state
                // has to be cleared.
                let mut changed = hover_reset;
                if this.move_draft.take().is_some() {
                    changed = true;
                }
                if this.resize_draft.is_some() {
                    let point = this.resize_draft.as_ref().map(|resize| resize.current);
                    if let Some(point) = point {
                        this.update_resize(point);
                    }
                    this.resize_draft = None;
                    changed = true;
                }
                if this.draft.take().is_some() || this.pan_draft.take().is_some() {
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            }));

        let toolbar_rect = self.toolbar_rect(window);
        let region_handles = self.region_handles(window, cx);
        let selection_layer = self.selection_layer(window);
        let region_preview = self.region_drag_preview();
        let (toolbar, options_panel, more_panel) = self.toolbar(window, cx);
        let info_bar = if inline {
            self.info_bar(window, cx)
        } else {
            None
        };
        let effects_panel = if inline {
            self.effects_panel(window, cx)
        } else {
            None
        };
        let hints_panel = self.hints_panel(window, cx);
        let custom_color_panel = self.custom_color_panel(window, cx);
        let status_toast = status.map(|message| {
            let top = (f32::from(toolbar_rect.origin.y) - 38.0).max(0.0);
            div()
                .absolute()
                .right(px(16.0))
                .top(px(top))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(self.chrome.surface_strong)
                .border_1()
                .border_color(self.chrome.border)
                .shadow(self.chrome.shadow())
                .text_size(px(12.0))
                .text_color(self.chrome.text)
                .child(message)
                .with_animation(
                    SharedString::from(format!("editor-status-{status_serial}")),
                    Animation::new(Duration::from_millis(120))
                        .with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |toast, delta| toast.opacity(delta).top(px(top + (1.0 - delta) * 4.0)),
                )
        });

        div()
            .relative()
            .size_full()
            .when(!inline, |root| root.bg(self.chrome.canvas))
            .text_color(self.chrome.text)
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                if let Some(draft) = &mut this.draft {
                    draft.shift = event.modifiers.shift;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if this.region_drag.is_some() {
                    this.update_region_drag(event.position, event.modifiers, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if let Some(edit) = this.text_edit.as_mut() {
                        edit.selecting = false;
                    }
                    if this.region_drag.is_some() {
                        this.finish_region_drag(cx);
                    }
                    // Toolbars and panels overlap the image on small captures
                    // and consume the mouse-up before the image layer sees it.
                    // Finish any in-flight drag here; the geometry itself was
                    // already committed by the last move.
                    let mut changed = false;
                    if this.move_draft.take().is_some() {
                        changed = true;
                    }
                    if this.resize_draft.is_some() {
                        let point = this.resize_draft.as_ref().map(|draft| draft.current);
                        if let Some(point) = point {
                            this.finish_resize(point, cx);
                        } else {
                            this.resize_draft = None;
                        }
                        changed = true;
                    }
                    if let Some(mut draft) = this.draft.take() {
                        let point = this
                            .to_image_point(event.position, window)
                            .unwrap_or(draft.current);
                        draft.current = point;
                        draft.shift = event.modifiers.shift;
                        if let Some(shape) = finalize_shape(&draft, point) {
                            this.document.add(shape, this.effective_style());
                            this.status = None;
                        }
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, _window, cx| {
                if this.region_drag.is_some() {
                    this.finish_region_drag(cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    // Right click confirms the annotation in progress instead
                    // of tearing the capture down: commit text, finish a
                    // multi-segment arrow, apply a crop, or close a panel.
                    cx.stop_propagation();
                    if this.text_edit.is_some() {
                        this.commit_text(cx);
                        return;
                    }
                    if this.polyline_draft.is_some() {
                        this.finish_polyline(cx);
                        return;
                    }
                    if this.crop_mode {
                        if this.crop_draft.is_some() {
                            this.apply_crop(cx);
                        } else {
                            this.cancel_crop(cx);
                        }
                        return;
                    }
                    if this.document.selected().is_some()
                        || this.tool != Tool::Select
                        || this.more_open
                        || this.effects_open
                        || this.hints_visible
                    {
                        // Outside an in-progress annotation, right click leaves
                        // annotation mode like Escape: drop the selection,
                        // return to Select, and close the tool options bar.
                        this.select_annotation(None);
                        this.tool = Tool::Select;
                        this.draft = None;
                        this.move_draft = None;
                        this.resize_draft = None;
                        this.pan_draft = None;
                        this.more_open = false;
                        this.effects_open = false;
                        this.hints_visible = false;
                        this.selection_hover = SelectionHover::None;
                        this.status = None;
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let command =
                    event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
                if this.custom_color_open {
                    match key {
                        "escape" => {
                            this.close_custom_color();
                            cx.notify();
                            return;
                        }
                        "enter" => {
                            if this.apply_custom_color(cx) {
                                return;
                            }
                        }
                        "backspace" | "delete" => {
                            if this.custom_color_fresh {
                                this.custom_color_buffer.clear();
                                this.custom_color_fresh = false;
                            } else {
                                this.custom_color_buffer.pop();
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
                        if this.custom_color_fresh {
                            this.custom_color_buffer.clear();
                            this.custom_color_fresh = false;
                        }
                        if this.custom_color_buffer.len() < 6 {
                            this.custom_color_buffer.push(ch.to_ascii_lowercase());
                        }
                        cx.notify();
                        return;
                    }
                    return;
                }
                if this.text_edit.is_some() {
                    // While an IME composition is in flight, Enter, Esc,
                    // arrows and Backspace belong to the input method. Handling
                    // them here committed the pinyin composition together with
                    // the final Chinese text.
                    if this
                        .text_edit
                        .as_ref()
                        .is_some_and(|edit| edit.marked.is_some())
                    {
                        return;
                    }
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
                        "up" => {
                            this.move_text_vertical(true, extend, window, cx);
                            return;
                        }
                        "down" => {
                            this.move_text_vertical(false, extend, window, cx);
                            return;
                        }
                        "home" => {
                            let position = this.text_line_edge(true, window);
                            if let Some(position) = position {
                                this.move_text_cursor(position, extend, cx);
                            }
                            return;
                        }
                        "end" => {
                            let position = this.text_line_edge(false, window);
                            if let Some(position) = position {
                                this.move_text_cursor(position, extend, cx);
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
                    if command && key == "s" && !event.keystroke.modifiers.shift {
                        this.commit_text(cx);
                        this.export(false, cx);
                        return;
                    }
                    // While editing, swallow document-level shortcuts so
                    // Cmd/Ctrl+Z cannot undo an object out from under the
                    // in-flight text edit.
                    return;
                }
                if this.polyline_draft.is_some() {
                    if key == "enter" {
                        this.finish_polyline(cx);
                        return;
                    }
                    if key == "escape" {
                        this.cancel_polyline(cx);
                        return;
                    }
                }
                if this.crop_mode {
                    if key == "enter" {
                        this.apply_crop(cx);
                        return;
                    }
                    if key == "escape" {
                        this.cancel_crop(cx);
                        return;
                    }
                }
                if key == "escape" && (this.more_open || this.effects_open || this.hints_visible) {
                    this.more_open = false;
                    this.effects_open = false;
                    this.hints_visible = false;
                    cx.notify();
                    return;
                }
                if key == "f1" {
                    this.hints_visible = !this.hints_visible;
                    this.more_open = false;
                    this.effects_open = false;
                    cx.notify();
                    return;
                }
                if key == "space" {
                    if this.inline.is_some() {
                        if this.inline.is_some_and(|inline| inline.zoomable) {
                            this.space_down = true;
                        } else {
                            this.toolbar_hidden = !this.toolbar_hidden;
                            this.more_open = false;
                            this.effects_open = false;
                            this.hints_visible = false;
                        }
                    } else {
                        this.space_down = true;
                    }
                    cx.notify();
                    return;
                }
                if key == "f5" {
                    if !event.is_held {
                        this.refresh_capture(cx);
                    }
                    return;
                }
                if key == "enter" {
                    this.export(true, cx);
                    return;
                }
                if command && key == "0" {
                    this.reset_view(cx);
                    return;
                }
                if command && key == "1" {
                    this.actual_size(window, cx);
                    return;
                }
                if command && (key == "=" || key == "+" || key == "-") {
                    let view = this.image_view_rect(window);
                    let center = point(
                        px(f32::from(view.origin.x) + f32::from(view.size.width) / 2.0),
                        px(f32::from(view.origin.y) + f32::from(view.size.height) / 2.0),
                    );
                    let factor = if key == "-" { 1.0 / 1.2 } else { 1.2 };
                    this.zoom_at(factor, center, window, cx);
                    return;
                }
                if key == "escape" {
                    if this.region_drag.is_some() {
                        this.region_drag = None;
                        let scale = this
                            .inline
                            .and_then(|inline| inline.capture)
                            .map(|capture| capture.scale.max(0.1))
                            .unwrap_or(1.0);
                        this.restore_region_image_rect(scale, cx);
                        return;
                    }
                    // First Escape clears the current annotation selection or
                    // active drawing tool; only the next one leaves the editor.
                    if this.document.selected().is_some() || this.tool != Tool::Select {
                        this.select_annotation(None);
                        this.tool = Tool::Select;
                        this.draft = None;
                        this.move_draft = None;
                        this.resize_draft = None;
                        this.pan_draft = None;
                        this.selection_hover = SelectionHover::None;
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    this.release_images(cx);
                    if this.inline.is_some() {
                        let handles = cx.windows();
                        cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
                    } else {
                        window.remove_window();
                    }
                    return;
                }
                if command && matches!(key, "w" | "q") {
                    this.release_images(cx);
                    if this.inline.is_some() {
                        let handles = cx.windows();
                        cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
                    } else {
                        window.remove_window();
                    }
                    return;
                }
                if key == "delete" || key == "backspace" {
                    this.delete_selected_annotation(cx);
                    return;
                }
                if command && key == "c" && !event.keystroke.modifiers.shift {
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
                        this.select_annotation(Some(pasted));
                        this.tool = Tool::Select;
                        this.show_status(
                            t(settings_state::get().language, "editor.annotation_pasted")
                                .to_owned(),
                            cx,
                        );
                    }
                    return;
                }
                if command && key == "s" && !event.keystroke.modifiers.shift {
                    this.export(false, cx);
                    return;
                }
                if command && key == "z" {
                    if event.keystroke.modifiers.shift {
                        if !this.document.redo() {
                            this.redo_canvas(cx);
                        }
                    } else if !this.document.undo() {
                        this.undo_canvas(cx);
                    }
                    this.status = None;
                    cx.notify();
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if event.keystroke.key == "space" {
                    this.space_down = false;
                    cx.notify();
                }
            }))
            .children(outside_backdrop)
            .child(image_layer)
            .children(region_handles)
            .children(selection_layer)
            .children(region_preview)
            .children(info_bar)
            .child(toolbar)
            .children(options_panel)
            .children(more_panel)
            .children(effects_panel)
            .children(custom_color_panel)
            .children(hints_panel)
            .children(status_toast)
            .children((!inline).then(|| {
                div()
                    .absolute()
                    .left(px(12.0))
                    .bottom(px(8.0))
                    .text_size(px(10.5))
                    .text_color(self.chrome.muted)
                    .child(format!(
                        "{} x {} · {:.0}%",
                        self.image_size.width,
                        self.image_size.height,
                        self.image_scale(window) * 100.0
                    ))
            }))
            .children((inline && self.toolbar_hidden).then(|| {
                let language = settings_state::get().language;
                div()
                    .id("editor-toolbar-recall")
                    .absolute()
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(12.0))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("editor-toolbar-recall-chip")
                            .occlude()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(self.chrome.surface_strong)
                            .border_1()
                            .border_color(self.chrome.border)
                            .shadow(self.chrome.shadow())
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .cursor_pointer()
                            .hover(|style| style.bg(self.chrome.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toolbar_hidden = false;
                                cx.notify();
                            }))
                            .child(format!("{} · Space", t(language, "editor.show_toolbar"))),
                    )
            }))
    }
}

struct EditorOverlayHost {
    editor: Entity<EditorView>,
}

impl Render for EditorOverlayHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // The editor itself is transparent outside the image. A full-window
        // scrim keeps pin/long-capture editing modal without opening a second
        // editor page; the image and toolbar paint above it.
        div()
            .relative()
            .size_full()
            .bg(rgba(0x0000004d))
            .child(self.editor.clone())
    }
}

fn inline_editor_window_kind() -> WindowKind {
    #[cfg(target_os = "macos")]
    {
        WindowKind::Normal
    }
    #[cfg(not(target_os = "macos"))]
    {
        WindowKind::PopUp
    }
}

/// Edits a pin at its current screen position. The pin window remains alive
/// underneath; this surface only replaces the old standalone editor window.
pub fn open_pin_editor_overlay(
    pin_id: u64,
    frame: Arc<RgbaFrame>,
    pin_bounds: Bounds<Pixels>,
    edit_state: Option<crate::pin::PinEditState>,
    cx: &mut App,
) -> bool {
    // The caller already resolved the unannotated base (or the flattened pin
    // when a compact base could not be decoded); only the vector document and
    // effects come from the edit state here.
    let (document, effects) = match edit_state {
        Some(state) => (Some(state.document), state.effects),
        None => (None, RegionEffects::default()),
    };
    let center_x = f32::from(pin_bounds.origin.x) + f32::from(pin_bounds.size.width) / 2.0;
    let center_y = f32::from(pin_bounds.origin.y) + f32::from(pin_bounds.size.height) / 2.0;
    let display = cx
        .displays()
        .into_iter()
        .find(|display| {
            let bounds = display.bounds();
            center_x >= f32::from(bounds.origin.x)
                && center_x < f32::from(bounds.origin.x) + f32::from(bounds.size.width)
                && center_y >= f32::from(bounds.origin.y)
                && center_y < f32::from(bounds.origin.y) + f32::from(bounds.size.height)
        })
        .or_else(|| cx.primary_display());
    let Some(display) = display else {
        log_error!("editor", "no display available for pin editing");
        return false;
    };
    let display_bounds = display.bounds();
    let image_rect = Bounds {
        origin: point(
            px(f32::from(pin_bounds.origin.x) - f32::from(display_bounds.origin.x)),
            px(f32::from(pin_bounds.origin.y) - f32::from(display_bounds.origin.y)),
        ),
        size: pin_bounds.size,
    };
    open_inline_editor_window(
        frame,
        image_rect,
        display_bounds,
        Some(display.id()),
        display_bounds.origin,
        Some(pin_id),
        false,
        Tool::Select,
        document,
        effects,
        cx,
    )
}

/// Opens a stitched long capture in the same in-place editor surface. The
/// tall image is initially fit to the capture monitor; wheel zoom/pan keeps
/// full-resolution detail reachable without a separate editor page.
pub fn open_long_capture_inline(frame: RgbaFrame, monitor_id: MonitorId, cx: &mut App) -> bool {
    let frame = Arc::new(frame);
    let monitors = platform_backend().monitors().unwrap_or_default();
    let monitor = monitors.iter().find(|monitor| monitor.id == monitor_id);
    let (window_bounds, display_id, screen_origin) = match monitor {
        Some(monitor) => {
            let scale = monitor.scale.get().max(0.1);
            let global = Bounds {
                origin: point(
                    px(monitor.bounds.origin.x as f32 / scale),
                    px(monitor.bounds.origin.y as f32 / scale),
                ),
                size: size(
                    px(monitor.bounds.size.width as f32 / scale),
                    px(monitor.bounds.size.height as f32 / scale),
                ),
            };
            // `WindowOptions` uses display-local coordinates on macOS; passing
            // the virtual-desktop origin pushed the result off-screen on a
            // secondary display.
            (
                crate::display::window_bounds_on_monitor(monitor, global),
                Some(DisplayId::new(monitor.id.raw())),
                global.origin,
            )
        }
        None => match cx.primary_display() {
            Some(display) => {
                let bounds = display.bounds();
                (bounds, Some(display.id()), bounds.origin)
            }
            None => {
                log_error!("editor", "no display available for long capture");
                return false;
            }
        },
    };
    let image_rect = fit_long_capture_rect(&frame, window_bounds);
    open_inline_editor_window(
        frame,
        image_rect,
        window_bounds,
        display_id,
        screen_origin,
        None,
        true,
        Tool::Select,
        None,
        RegionEffects::default(),
        cx,
    )
}

fn fit_long_capture_rect(frame: &RgbaFrame, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    // Keep the result clear of the macOS menu bar (which can be up to 37 pt on
    // notched displays) and the window edges elsewhere, so a tall capture
    // never opens partially off-screen.
    const MARGIN: f32 = 44.0;
    const GAP: f32 = 8.0;
    let window_width = f32::from(bounds.size.width);
    let window_height = f32::from(bounds.size.height);
    let available_width = (window_width - MARGIN * 2.0).max(1.0);
    let available_height = (window_height - MARGIN * 2.0 - COMPACT_TOOLBAR_HEIGHT - GAP).max(1.0);
    let fit = (available_width / frame.width().max(1) as f32)
        .min(available_height / frame.height().max(1) as f32)
        .min(1.0);
    let width = frame.width() as f32 * fit;
    let height = frame.height() as f32 * fit;
    let x = (window_width - width) / 2.0;
    let y = MARGIN + (available_height - height) / 2.0;
    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    }
}

#[allow(clippy::too_many_arguments)]
fn open_inline_editor_window(
    frame: Arc<RgbaFrame>,
    image_rect: Bounds<Pixels>,
    window_bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    screen_origin: Point<Pixels>,
    pin_target: Option<u64>,
    zoomable: bool,
    initial_tool: Tool,
    document: Option<AnnotationDocument>,
    effects: RegionEffects,
    cx: &mut App,
) -> bool {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(window_bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: inline_editor_window_kind(),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-editor-inline".to_owned()),
        ..Default::default()
    };

    // Accessories are not brought forward by ordering a window front; make
    // sure the pin/long-capture editor can become key before opening it.
    cx.activate(true);
    match gpui_kit::open_window(options, cx, move |window, cx| {
        if let Err(error) = apply_window_style(&*window, WindowStyle::INLINE_EDITOR) {
            log_error!("editor", "failed to style inline editor: {error}");
        }
        hide_native_border(&*window);
        // The popup frame is only on the left, right and bottom, but GPUI
        // splits its height evenly when placing the window. Align the client
        // area so annotations land on the pixels they were drawn over.
        crate::ui::align_client_to_bounds(&*window, window_bounds);
        let editor = cx.new(|cx| {
            EditorView::new_inline(
                frame,
                None,
                image_rect,
                screen_origin,
                initial_tool,
                None,
                pin_target,
                zoomable,
                document,
                effects,
                cx,
            )
        });
        let focus = editor.read(cx).focus_handle.clone();
        register_view(&editor, window);
        editor.update(cx, |this, cx| {
            this.observe_appearance(window, cx);
            this.watch_close(window, cx);
        });
        window.defer(cx, move |window, cx| {
            crate::ui::align_client_to_bounds(window, window_bounds);
            window.focus(&focus, cx);
            window.activate_window();
        });
        cx.new(|_| EditorOverlayHost { editor })
    }) {
        Ok((handle, _)) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
            true
        }
        Err(error) => {
            log_error!("editor", "failed to open inline editor: {error}");
            false
        }
    }
}

fn finalize_shape(draft: &Draft, end: PhysicalPoint) -> Option<Shape> {
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

fn normalized_rect(start: PhysicalPoint, end: PhysicalPoint) -> PhysicalRect {
    let left = start.x.min(end.x);
    let top = start.y.min(end.y);
    let right = start.x.max(end.x);
    let bottom = start.y.max(end.y);
    PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32)
}

/// Shift keeps the drag direction but forces equal width and height, which
/// turns rectangles into squares and ellipses into circles.
fn constrained_rect(start: PhysicalPoint, end: PhysicalPoint, square: bool) -> PhysicalRect {
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

fn distance(a: PhysicalPoint, b: PhysicalPoint) -> f32 {
    let dx = (a.x - b.x) as f32;
    let dy = (a.y - b.y) as f32;
    (dx * dx + dy * dy).sqrt()
}

fn region_handle_cursor(handle: RegionHandle) -> CursorStyle {
    match handle {
        RegionHandle::NorthWest | RegionHandle::SouthEast => CursorStyle::ResizeUpLeftDownRight,
        RegionHandle::NorthEast | RegionHandle::SouthWest => CursorStyle::ResizeUpRightDownLeft,
        RegionHandle::North | RegionHandle::South => CursorStyle::ResizeUpDown,
        RegionHandle::East | RegionHandle::West => CursorStyle::ResizeLeftRight,
    }
}

fn handle_cursor(handle: ResizeHandle) -> CursorStyle {
    match handle {
        ResizeHandle::Nw | ResizeHandle::Se => CursorStyle::ResizeUpLeftDownRight,
        ResizeHandle::Ne | ResizeHandle::Sw => CursorStyle::ResizeUpRightDownLeft,
        ResizeHandle::N | ResizeHandle::S => CursorStyle::ResizeUpDown,
        ResizeHandle::E | ResizeHandle::W => CursorStyle::ResizeLeftRight,
        ResizeHandle::ArrowStart | ResizeHandle::ArrowEnd => CursorStyle::Crosshair,
    }
}

fn handle_points(shape: &Shape) -> Vec<(ResizeHandle, PhysicalPoint)> {
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

fn hit_resize_handle(shape: &Shape, point: PhysicalPoint, tolerance: f32) -> Option<ResizeHandle> {
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

/// Half-size of the painted selection handle in logical points. The hit area
/// must match it, otherwise the visible outer half of a handle starts a move
/// instead of a resize.
const SELECTION_HANDLE_HALF: f32 = 5.5;

/// Resolves a resize handle without stealing drags from the object body.
///
/// Handles win inside their painted square even when that square overlaps the
/// shape border; only a deeper interior point starts a body drag. Very thin
/// shapes keep the body-first rule so moving them stays possible.
fn annotation_handle_at(shape: &Shape, point: PhysicalPoint, scale: f32) -> Option<ResizeHandle> {
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

fn closed_shape_contains(shape: &Shape, point: PhysicalPoint, inset: f32) -> bool {
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

fn resize_shape(
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
        // Sequence badges stay circular: resizing any handle scales the
        // square uniformly instead of turning the chip into an ellipse.
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

fn paint_crop_overlay(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scale: f32,
    rect: PhysicalRect,
    accent_hex: u32,
) {
    let image_width = f32::from(bounds.size.width);
    let image_height = f32::from(bounds.size.height);
    let left = rect.origin.x as f32 * scale;
    let top = rect.origin.y as f32 * scale;
    let right = left + rect.size.width as f32 * scale;
    let bottom = top + rect.size.height as f32 * scale;
    let fill = rgba(0x00000099);
    let mut fill_rect = |x: f32, y: f32, width: f32, height: f32| {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let mut builder = PathBuilder::fill();
        builder.move_to(point(bounds.origin.x + px(x), bounds.origin.y + px(y)));
        builder.line_to(point(
            bounds.origin.x + px(x + width),
            bounds.origin.y + px(y),
        ));
        builder.line_to(point(
            bounds.origin.x + px(x + width),
            bounds.origin.y + px(y + height),
        ));
        builder.line_to(point(
            bounds.origin.x + px(x),
            bounds.origin.y + px(y + height),
        ));
        builder.close();
        if let Ok(path) = builder.build() {
            window.paint_path(path, fill);
        }
    };
    fill_rect(0.0, 0.0, image_width, top);
    fill_rect(0.0, bottom, image_width, image_height - bottom);
    fill_rect(0.0, top, left, bottom - top);
    fill_rect(right, top, image_width - right, bottom - top);

    let stroke = px((1.5 * scale).max(1.0));
    let mut builder = PathBuilder::stroke(stroke);
    builder.move_to(point(bounds.origin.x + px(left), bounds.origin.y + px(top)));
    builder.line_to(point(
        bounds.origin.x + px(right),
        bounds.origin.y + px(top),
    ));
    builder.line_to(point(
        bounds.origin.x + px(right),
        bounds.origin.y + px(bottom),
    ));
    builder.line_to(point(
        bounds.origin.x + px(left),
        bounds.origin.y + px(bottom),
    ));
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
    let color = if matches!(shape, Shape::HighlighterBox(_)) {
        rgba(core_annotate::highlighter_color(style.color))
    } else {
        rgba(style.color)
    };
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
            let radius =
                (style.corner_radius * scale).clamp(0.0, (right - left).min(bottom - top) / 2.0);
            let mut builder = if style.filled {
                PathBuilder::fill()
            } else {
                PathBuilder::stroke(stroke)
            };
            if radius < 0.5 {
                builder.move_to(to_point(left, top));
                builder.line_to(to_point(right, top));
                builder.line_to(to_point(right, bottom));
                builder.line_to(to_point(left, bottom));
            } else {
                builder.move_to(to_point(left + radius, top));
                builder.line_to(to_point(right - radius, top));
                builder.curve_to(to_point(right, top + radius), to_point(right, top));
                builder.line_to(to_point(right, bottom - radius));
                builder.curve_to(to_point(right - radius, bottom), to_point(right, bottom));
                builder.line_to(to_point(left + radius, bottom));
                builder.curve_to(to_point(left, bottom - radius), to_point(left, bottom));
                builder.line_to(to_point(left, top + radius));
                builder.curve_to(to_point(left + radius, top), to_point(left, top));
            }
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
            let mut builder = if style.filled {
                PathBuilder::fill()
            } else {
                PathBuilder::stroke(stroke)
            };
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
        Shape::Arrow { .. } | Shape::Polyline { .. } => {
            let Some(polygons) = core_annotate::stroke_outline(shape, &style) else {
                return;
            };
            for polygon in polygons {
                let Some(first) = polygon.first() else {
                    continue;
                };
                let mut builder = PathBuilder::fill();
                builder.move_to(to_point(first.x, first.y));
                for vertex in &polygon[1..] {
                    builder.line_to(to_point(vertex.x, vertex.y));
                }
                builder.close();
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }
        }
        Shape::HighlighterBox(rect) => {
            let left = rect.origin.x as f32;
            let top = rect.origin.y as f32;
            let right = left + rect.size.width as f32;
            let bottom = top + rect.size.height as f32;
            let mut builder = PathBuilder::fill();
            builder.move_to(to_point(left, top));
            builder.line_to(to_point(right, top));
            builder.line_to(to_point(right, bottom));
            builder.line_to(to_point(left, bottom));
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Blur(_) | Shape::Text { .. } | Shape::Number { .. } => {}
        Shape::Spotlight(rect) => paint_spotlight(
            window,
            bounds,
            scale,
            *rect,
            style.spotlight_intensity,
            style.spotlight_radius,
        ),
    }
}

fn paint_spotlight(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scale: f32,
    rect: PhysicalRect,
    intensity: u8,
    radius: f32,
) {
    let alpha = (u32::from(intensity.min(100)) * 255 / 100) as u8;
    let image_left = f32::from(bounds.origin.x);
    let image_top = f32::from(bounds.origin.y);
    let image_right = image_left + f32::from(bounds.size.width);
    let image_bottom = image_top + f32::from(bounds.size.height);
    let hole_left = (image_left + rect.origin.x as f32 * scale).clamp(image_left, image_right);
    let hole_top = (image_top + rect.origin.y as f32 * scale).clamp(image_top, image_bottom);
    let hole_right = (hole_left + rect.size.width as f32 * scale).clamp(hole_left, image_right);
    let hole_bottom = (hole_top + rect.size.height as f32 * scale).clamp(hole_top, image_bottom);
    if hole_right <= hole_left || hole_bottom <= hole_top {
        return;
    }

    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::EvenOdd),
    ));
    builder.move_to(point(px(image_left), px(image_top)));
    builder.line_to(point(px(image_right), px(image_top)));
    builder.line_to(point(px(image_right), px(image_bottom)));
    builder.line_to(point(px(image_left), px(image_bottom)));
    builder.close();
    let radius = (radius * scale).clamp(
        0.0,
        (hole_right - hole_left).min(hole_bottom - hole_top) / 2.0,
    );
    let hole = Bounds {
        origin: point(px(hole_left), px(hole_top)),
        size: size(px(hole_right - hole_left), px(hole_bottom - hole_top)),
    };
    crate::ui::push_rounded_rect(&mut builder, hole, radius);
    if let Ok(path) = builder.build() {
        window.paint_path(path, rgba(u32::from(alpha)));
    }
}

#[cfg(test)]
mod tests {
    use super::{TextEdit, word_range_at};
    use crate::ui::hue_rgb;
    use core_annotate::AnnotationStyle;
    use core_types::{PhysicalPoint, PhysicalSize};

    fn empty_edit() -> TextEdit {
        TextEdit::start(
            PhysicalPoint::new(0, 0),
            PhysicalSize::new(800, 600),
            AnnotationStyle::default(),
        )
    }

    #[test]
    fn committed_ime_result_replaces_the_composition() {
        let mut edit = empty_edit();
        edit.replace(None, "ni", true);
        assert_eq!(edit.content, "ni");
        assert_eq!(edit.marked, Some(0..2));

        edit.replace(None, "NI", false);
        assert_eq!(edit.content, "NI");
        assert_eq!(edit.marked, None);

        edit.replace(None, "ma", true);
        edit.replace(None, "MA", false);
        assert_eq!(edit.content, "NIMA");
        assert_eq!(edit.marked, None);
    }

    #[test]
    fn an_open_composition_can_be_cleared_without_touching_committed_text() {
        let mut edit = empty_edit();
        edit.replace(None, "hao", true);
        edit.replace(Some(0..3), "", false);
        assert_eq!(edit.content, "");
        assert_eq!(edit.marked, None);
    }

    #[test]
    fn word_selection_covers_latin_and_cjk_runs() {
        assert_eq!(word_range_at("hello world", 2), 0..5);
        assert_eq!(word_range_at("hello world", 8), 6..11);
        assert_eq!(word_range_at("hello world", 5), 5..6);
        assert_eq!(
            word_range_at("\u{622A}\u{56FE}\u{5DE5}\u{5177} Capin", 8),
            0..12
        );
        assert_eq!(
            word_range_at("\u{622A}\u{56FE}\u{5DE5}\u{5177} Capin", 16),
            13..18
        );
    }

    #[test]
    fn word_selection_falls_back_to_one_character() {
        assert_eq!(word_range_at("a, b", 1), 1..3);
        assert_eq!(word_range_at("a, b", 4), 3..4);
        assert_eq!(word_range_at("", 0), 0..0);
    }

    #[test]
    fn hue_strip_wraps_and_stays_saturated() {
        assert_eq!(hue_rgb(0.0), hue_rgb(360.0));
        assert_ne!(hue_rgb(0.0), hue_rgb(120.0));
        assert_ne!(hue_rgb(120.0), hue_rgb(240.0));
    }

    #[test]
    fn small_closed_annotations_drag_from_the_body() {
        let shape = super::Shape::Rect(core_types::PhysicalRect::new(0, 0, 12, 12));
        assert_eq!(
            super::annotation_handle_at(&shape, PhysicalPoint::new(6, 6), 0.5),
            None
        );
        assert!(
            super::annotation_handle_at(&shape, PhysicalPoint::new(0, 0), 0.5).is_some(),
            "the painted corner remains a resize handle"
        );
    }

    #[test]
    fn large_annotation_handles_cover_their_painted_square() {
        let shape = super::Shape::Rect(core_types::PhysicalRect::new(0, 0, 200, 100));
        assert_eq!(
            super::annotation_handle_at(&shape, PhysicalPoint::new(5, 5), 0.5),
            Some(super::ResizeHandle::Nw),
            "a point inside the painted handle still resizes"
        );
        assert_eq!(
            super::annotation_handle_at(&shape, PhysicalPoint::new(40, 40), 0.5),
            None,
            "a point away from the handles drags the body"
        );
    }

    #[test]
    fn arrow_body_drags_instead_of_resizing_from_an_endpoint() {
        let shape = super::Shape::Arrow {
            start: PhysicalPoint::new(10, 50),
            end: PhysicalPoint::new(210, 50),
        };
        assert_eq!(
            super::annotation_handle_at(&shape, PhysicalPoint::new(100, 50), 0.5),
            None
        );
        assert!(
            super::annotation_handle_at(&shape, PhysicalPoint::new(210, 50), 0.5).is_some(),
            "the tip remains a resize handle"
        );
    }

    #[test]
    fn number_resize_stays_square_from_corner_and_edge_handles() {
        use core_types::PhysicalRect;
        let original = super::Shape::Number {
            bounds: PhysicalRect::new(10, 10, 32, 32),
            value: 1,
        };
        let resized = super::resize_shape(
            &original,
            super::ResizeHandle::Se,
            PhysicalPoint::new(42, 42),
            PhysicalPoint::new(62, 52),
            4,
        );
        assert_eq!(
            resized,
            super::Shape::Number {
                bounds: PhysicalRect::new(10, 10, 52, 52),
                value: 1,
            }
        );

        let resized = super::resize_shape(
            &original,
            super::ResizeHandle::E,
            PhysicalPoint::new(42, 26),
            PhysicalPoint::new(62, 26),
            4,
        );
        assert_eq!(
            resized,
            super::Shape::Number {
                bounds: PhysicalRect::new(10, 0, 52, 52),
                value: 1,
            }
        );
    }

    #[test]
    fn long_capture_fit_keeps_room_for_the_menu_bar_and_toolbar() {
        use gpui_kit::{Bounds, point, px, size};
        let frame =
            core_types::RgbaFrame::from_pixels(400, 12_000, vec![0; 400 * 12_000 * 4]).unwrap();
        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(1440.0), px(900.0)),
        };
        let rect = super::fit_long_capture_rect(&frame, bounds);
        assert!(f32::from(rect.origin.y) >= 44.0);
        assert!(f32::from(rect.origin.y) + f32::from(rect.size.height) <= 900.0 - 44.0);
        assert!(f32::from(rect.origin.x) >= 44.0);
        assert!(f32::from(rect.size.width) <= 1440.0 - 88.0 + 0.5);
    }
}
