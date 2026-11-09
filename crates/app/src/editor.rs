use std::ops::Range;
use std::sync::Arc;

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
use platform_window::{WindowStyle, apply as apply_window_style, system_accent_hex};

use crate::chrome_theme::ChromeTheme;
use crate::effects::RegionEffects;
use crate::logging::{log_error, log_info};
use crate::settings_state::AnnotationColorTool;
use crate::{i18n::t, icons, image_util, settings_state};

const COMPACT_TOOLBAR_HEIGHT: f32 = 40.0;
const COMPACT_TOOLBAR_WIDTH: f32 = 700.0;
const OPTIONS_PANEL_HEIGHT: f32 = 46.0;
const INFO_BAR_HEIGHT: f32 = 28.0;
const INFO_BAR_WIDTH: f32 = 360.0;
const TOOLBAR_SIZE: u32 = 30;
const BADGE_SIZE: u32 = 32;

const STROKE_WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];
const FONT_SIZES: [f32; 4] = [14.0, 20.0, 28.0, 36.0];
const CORNER_RADII: [f32; 4] = [0.0, 8.0, 16.0, 24.0];
const BLUR_RADII: [u32; 4] = [8, 16, 24, 32];
const SPOTLIGHT_INTENSITIES: [u8; 3] = [40, 60, 80];
const CANVAS_HISTORY_LIMIT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Select,
    Rect,
    Ellipse,
    Arrow,
    Polyline,
    Highlighter,
    Blur,
    Text,
    Number,
    Spotlight,
}

impl Tool {
    pub(crate) const ALL: [Self; 10] = [
        Self::Select,
        Self::Rect,
        Self::Ellipse,
        Self::Arrow,
        Self::Polyline,
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
            Self::Ellipse => "editor-tool-ellipse",
            Self::Arrow => "editor-tool-arrow",
            Self::Polyline => "editor-tool-polyline",
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
            Self::Ellipse => icons::ELLIPSE,
            Self::Arrow => icons::ARROW,
            Self::Polyline => icons::POLYLINE,
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
            Self::Ellipse => "editor.tool.ellipse",
            Self::Arrow => "editor.tool.arrow",
            Self::Polyline => "editor.tool.polyline",
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
            Self::Ellipse => Some(AnnotationColorTool::Ellipse),
            Self::Arrow | Self::Polyline => Some(AnnotationColorTool::Arrow),
            Self::Highlighter => Some(AnnotationColorTool::Highlighter),
            Self::Text => Some(AnnotationColorTool::Text),
            Self::Number => Some(AnnotationColorTool::Number),
            Self::Select | Self::Blur | Self::Spotlight => None,
        }
    }

    fn from_shape(shape: &Shape) -> Option<Self> {
        match shape {
            Shape::Rect(_) => Some(Self::Rect),
            Shape::Ellipse(_) => Some(Self::Ellipse),
            Shape::Arrow { .. } => Some(Self::Arrow),
            Shape::Polyline { .. } => Some(Self::Polyline),
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
}

struct MoveDraft {
    id: core_annotate::AnnotationId,
    start: PhysicalPoint,
    current: PhysicalPoint,
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

pub(crate) struct EditorView {
    image: Arc<RenderImage>,
    blur_images: [Option<Arc<RenderImage>>; BLUR_RADII.len()],
    images_released: bool,
    frame: Arc<RgbaFrame>,
    pin_target: Option<u64>,
    document: AnnotationDocument,
    image_size: PhysicalSize,
    tool: Tool,
    style: AnnotationStyle,
    font_size: f32,
    arrow_start: bool,
    arrow_end: bool,
    text_background: bool,
    text_outline: bool,
    text_alignment: TextAlignment,
    corner_radius: f32,
    blur_radius: u32,
    rect_filled: bool,
    ellipse_filled: bool,
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
    pub(crate) focus_handle: FocusHandle,
    inline: Option<InlineHost>,
}

impl EditorView {
    fn new(frame: Arc<RgbaFrame>, pin_target: Option<u64>, cx: &mut Context<Self>) -> Self {
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image(&frame);
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
        let text_background = settings.annotation_text_background;
        let text_outline = settings.annotation_text_outline;
        let text_alignment = settings.annotation_text_alignment;
        let corner_radius = settings.annotation_corner_radius;
        let blur_radius = settings.annotation_blur_radius;
        let rect_filled = settings.annotation_rect_filled;
        let ellipse_filled = settings.annotation_ellipse_filled;
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
            pin_target,
            document: AnnotationDocument::new(image_size),
            image_size,
            tool: Tool::Select,
            style: AnnotationStyle {
                color: style_color,
                stroke_width: settings.annotation_stroke_width,
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
            },
            font_size,
            arrow_start,
            arrow_end,
            text_background,
            text_outline,
            text_alignment,
            corner_radius,
            blur_radius,
            rect_filled,
            ellipse_filled,
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
        image_rect: Bounds<Pixels>,
        screen_origin: Point<Pixels>,
        initial_tool: Tool,
        capture: Option<InlineCapture>,
        pin_target: Option<u64>,
        zoomable: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(frame, pin_target, cx);
        if let Some(capture) = capture {
            view.effects = capture.effects;
        }
        view.tool = initial_tool;
        if let Some(color_tool) = initial_tool.color_tool() {
            view.active_color_tool = color_tool;
            view.style.color = view.tool_color(color_tool);
        }
        match initial_tool {
            Tool::Rect => {
                view.style.corner_radius = view.corner_radius;
                view.style.filled = view.rect_filled;
            }
            Tool::Ellipse => view.style.filled = view.ellipse_filled,
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
        let hover = match self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
        {
            Some(annotation) => {
                if let Some(handle) = hit_resize_handle(&annotation.shape, point, 9.0 / scale) {
                    SelectionHover::Handle(handle)
                } else if self.document.hit_test(point) == Some(annotation.id) {
                    SelectionHover::Body
                } else {
                    SelectionHover::None
                }
            }
            None => SelectionHover::None,
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
        let width = 560.0_f32.min((window_width - 16.0).max(120.0));
        let left = f32::from(toolbar.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let mut top = f32::from(toolbar.origin.y) - OPTIONS_PANEL_HEIGHT - 6.0;
        if top < 8.0 {
            top = f32::from(toolbar.origin.y) + f32::from(toolbar.size.height) + 6.0;
        }
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
        let mut top = f32::from(anchor.origin.y) - INFO_BAR_HEIGHT - 8.0;
        let toolbar = self.toolbar_rect(window);
        let inside_top = f32::from(anchor.origin.y) + 8.0;
        let below = f32::from(anchor.origin.y) + f32::from(anchor.size.height) + 8.0;
        if top < 8.0 {
            top = inside_top;
        } else if f32::from(toolbar.origin.y) < f32::from(anchor.origin.y) {
            top = if below + INFO_BAR_HEIGHT <= window_height - 8.0 {
                below
            } else {
                inside_top
            };
        }
        top = top.clamp(8.0, (window_height - INFO_BAR_HEIGHT - 8.0).max(8.0));
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
        let height = 84.0;
        let left = f32::from(info.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let top =
            crate::ui::popover_top(info, height, window_height, Some(self.toolbar_rect(window)));
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        })
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
        let image = image_util::render_image(&self.frame);
        let old_image = std::mem::replace(&mut self.image, image);
        let old_blur_images =
            std::mem::replace(&mut self.blur_images, std::array::from_fn(|_| None));
        self.images_released = false;
        cx.drop_image(old_image, None);
        for image in old_blur_images.into_iter().flatten() {
            cx.drop_image(image, None);
        }
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

    fn rotate_canvas(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let language = settings_state::get().language;
        let Ok(output) = rasterize(&self.document, &self.frame) else {
            self.status = Some(t(language, "editor.export_failed").to_owned());
            cx.notify();
            return;
        };
        self.push_canvas_snapshot();
        let rotated = output.rotated_cw();
        self.replace_canvas(rotated, cx);
        self.status = Some(t(language, "editor.rotated").to_owned());
        cx.notify();
    }

    fn flip_canvas(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let language = settings_state::get().language;
        let Ok(output) = rasterize(&self.document, &self.frame) else {
            self.status = Some(t(language, "editor.export_failed").to_owned());
            cx.notify();
            return;
        };
        self.push_canvas_snapshot();
        let flipped = output.flipped(horizontal);
        self.replace_canvas(flipped, cx);
        self.status = Some(
            t(
                language,
                if horizontal {
                    "editor.flipped_horizontal"
                } else {
                    "editor.flipped_vertical"
                },
            )
            .to_owned(),
        );
        cx.notify();
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
        self.status = Some(t(settings_state::get().language, "editor.cropped").to_owned());
        cx.notify();
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
        if self.tool == Tool::Highlighter {
            style.color = (style.color & 0xFFFFFF00) | 0x80;
            style.stroke_width = 12.0;
        }
        style.filled = (self.tool == Tool::Rect && self.rect_filled)
            || (self.tool == Tool::Ellipse && self.ellipse_filled);
        if self.tool == Tool::Rect && style.filled {
            // Rectangles are opaque by design; there is no opacity control.
            style.color |= 0x000000FF;
        }
        if self.tool == Tool::Text {
            style.font_size = self.font_size;
            style.text_background = self.text_background;
            style.text_outline = self.text_outline;
            style.text_alignment = self.text_alignment;
        }
        if matches!(self.tool, Tool::Arrow | Tool::Polyline) {
            style.arrow_start = self.arrow_start;
            style.arrow_end = self.arrow_end;
        }
        match self.tool {
            Tool::Rect => style.corner_radius = self.corner_radius,
            Tool::Blur => style.blur_radius = self.blur_radius,
            Tool::Spotlight => style.spotlight_intensity = self.spotlight_intensity,
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

    fn apply_stroke_width(&mut self, width: f32) {
        self.update_style(|style| style.stroke_width = width);
        settings_state::update(|settings| settings.annotation_stroke_width = width);
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

    fn apply_corner_radius(&mut self, radius: f32) {
        self.corner_radius = radius;
        self.update_style(|style| style.corner_radius = radius);
        settings_state::update(|settings| settings.annotation_corner_radius = radius);
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
                self.rect_filled = filled;
                settings_state::update(|settings| settings.annotation_rect_filled = filled);
            }
            Tool::Ellipse => {
                self.ellipse_filled = filled;
                settings_state::update(|settings| settings.annotation_ellipse_filled = filled);
            }
            _ => return,
        }
        self.update_style(|style| style.filled = filled);
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
        // `effective_style` (highlighter alpha 0x80) but `self.style` still
        // holds the opaque palette color, so the first color switch turned a
        // highlighter box fully opaque.
        let alpha = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style.color & 0xFF)
            .unwrap_or_else(|| {
                if tool == AnnotationColorTool::Highlighter {
                    0x80
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

    fn finish_resize(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) {
        let Some(draft) = self.resize_draft.take() else {
            return;
        };
        if point == draft.start {
            cx.notify();
            return;
        }
        let Some(style) = self
            .document
            .object(draft.id)
            .map(|annotation| annotation.style)
        else {
            return;
        };
        let shape = resize_shape(&draft.original, draft.handle, draft.start, point, 4);
        self.document.update_shape(draft.id, shape, style);
        self.status = None;
        cx.notify();
    }

    fn select_annotation(&mut self, id: Option<core_annotate::AnnotationId>) {
        self.document.select(id);
        if let Some(annotation) = id.and_then(|id| self.document.object(id)) {
            self.style = annotation.style;
            if let Some(tool) = Tool::from_shape(&annotation.shape) {
                if let Some(color_tool) = tool.color_tool() {
                    self.active_color_tool = color_tool;
                }
                match tool {
                    Tool::Rect => {
                        self.corner_radius = annotation.style.corner_radius;
                        self.rect_filled = annotation.style.filled;
                    }
                    Tool::Ellipse => self.ellipse_filled = annotation.style.filled,
                    Tool::Text => {
                        self.font_size = annotation.style.font_size;
                        self.text_background = annotation.style.text_background;
                        self.text_outline = annotation.style.text_outline;
                        self.text_alignment = annotation.style.text_alignment;
                    }
                    Tool::Number => {}
                    Tool::Arrow => {
                        self.arrow_start = annotation.style.arrow_start;
                        self.arrow_end = annotation.style.arrow_end;
                    }
                    Tool::Blur => self.blur_radius = annotation.style.blur_radius,
                    Tool::Spotlight => {
                        self.spotlight_intensity = annotation.style.spotlight_intensity
                    }
                    _ => {}
                }
            }
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
                .hover(move |style| style.bg(hover))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.document.select(None);
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
                            this.style.corner_radius = this.corner_radius;
                            this.style.filled = this.rect_filled;
                        }
                        Tool::Ellipse => this.style.filled = this.ellipse_filled,
                        Tool::Blur => this.style.blur_radius = this.blur_radius,
                        Tool::Spotlight => {
                            this.style.spotlight_intensity = this.spotlight_intensity
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
        let config_filled = match config_tool {
            Some(Tool::Rect) => selected_style.map_or(self.rect_filled, |style| style.filled),
            Some(Tool::Ellipse) => selected_style.map_or(self.ellipse_filled, |style| style.filled),
            _ => false,
        };
        let config_intensity =
            selected_style.map_or(self.spotlight_intensity, |style| style.spotlight_intensity);
        let show_color = config_tool.and_then(Tool::color_tool).is_some();
        let show_fill = matches!(config_tool, Some(Tool::Rect | Tool::Ellipse));
        let show_width = match config_tool {
            Some(Tool::Arrow) => true,
            Some(Tool::Rect | Tool::Ellipse) => !config_filled,
            _ => false,
        };
        let show_font = config_tool == Some(Tool::Text);
        let show_text_style = config_tool == Some(Tool::Text);
        let show_arrow_heads = config_tool == Some(Tool::Arrow);
        let show_corner = config_tool == Some(Tool::Rect);
        let show_blur = config_tool == Some(Tool::Blur);
        let show_spotlight = config_tool == Some(Tool::Spotlight);

        let mut groups: Vec<AnyElement> = Vec::new();
        if show_color {
            let palette_color = self
                .document
                .selected()
                .and_then(|id| self.document.object(id))
                .map(|annotation| annotation.style.color)
                .unwrap_or_else(|| self.tool_color(self.palette_color_tool()));
            let mut swatches = Vec::new();
            for (color, name, label_key) in settings_state::ANNOTATION_COLORS {
                let selected = (palette_color & 0xFFFFFF00) == (color & 0xFFFFFF00);
                let label = t(language, label_key).to_owned();
                let swatch = div()
                    .id(name)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded(px(9.0))
                    .bg(rgb((color >> 8) & 0x00FF_FFFF))
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
                    .hover(move |style| style.bg(hover))
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
                    .hover(move |style| style.bg(hover))
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
                    .hover(move |style| style.bg(hover))
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
                .hover(move |style| style.bg(hover))
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
                .hover(move |style| style.bg(hover))
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
                    .hover(move |style| style.bg(hover))
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
                let selected = (config_style.corner_radius - radius).abs() < 0.1;
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
                    .hover(move |style| style.bg(hover))
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
                    .hover(move |style| style.bg(hover))
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
                    .hover(move |style| style.bg(hover))
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
        let options_right = f32::from(window.viewport_size().width)
            - (f32::from(bar_rect.origin.x) + f32::from(bar_rect.size.width));
        let options_panel = (!config_children.is_empty() && !self.toolbar_hidden).then(|| {
            div()
                .id("editor-options-panel")
                .absolute()
                .right(px(options_right.max(8.0)))
                .top(options_rect.origin.y)
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
                .children(config_children)
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
            panel
        });

        let more_panel = more_panel.map(|panel| {
            let more_rect = self.more_rect(window, bar_rect);
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
                .child(panel.h(more_rect.size.height))
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
                    .children(capture_actions)
                    .children(pin)
                    .child(copy)
                    .child(save)
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
        self.inline
            .and_then(|inline| inline.capture)
            .is_some_and(|capture| {
                !matches!(capture.target, CaptureTarget::Window(_)) && self.effects.active()
            })
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
                    .hover(|style| style.bg(self.chrome.hover))
                    .text_size(px(10.5))
                    .text_color(if selected_radius {
                        self.chrome.accent_text
                    } else {
                        self.chrome.muted
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.effects.corner_radius = radius;
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        let shadow = self.effects.shadow;
        let border = self.effects.border;
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
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(4.0))
                    .child(
                        div()
                            .id("editor-effects-shadow")
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(5.0))
                            .when(shadow, |chip| chip.bg(self.chrome.accent))
                            .hover(|style| style.bg(self.chrome.hover))
                            .text_size(px(10.5))
                            .text_color(if shadow {
                                self.chrome.accent_text
                            } else {
                                self.chrome.muted
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.effects.shadow = !this.effects.shadow;
                                cx.notify();
                            }))
                            .child(t(language, "overlay.effects_shadow")),
                    )
                    .child(
                        div()
                            .id("editor-effects-border")
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(5.0))
                            .when(border, |chip| chip.bg(self.chrome.accent))
                            .hover(|style| style.bg(self.chrome.hover))
                            .text_size(px(10.5))
                            .text_color(if border {
                                self.chrome.accent_text
                            } else {
                                self.chrome.muted
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.effects.border = !this.effects.border;
                                cx.notify();
                            }))
                            .child(t(language, "overlay.effects_border")),
                    ),
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
                    crate::window_style::compose_window_capture(id, frame, capture.scale)
                }
                _ => frame,
            },
            Err(error) => {
                self.status = Some(format!(
                    "{} {}",
                    t(settings_state::get().language, "editor.refresh_failed"),
                    error
                ));
                cx.notify();
                return;
            }
        };
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
        self.status = Some(t(settings_state::get().language, "editor.refreshed").to_owned());
        cx.notify();
    }

    fn start_scroll_capture(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        self.commit_text(cx);
        let first_frame =
            rasterize(&self.document, &self.frame).unwrap_or_else(|_| self.frame.as_ref().clone());
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
        match rasterize(&self.document, &self.frame) {
            Ok(output) => {
                let output = self.effects.apply(output, self.region_effects_scale());
                let hash = crate::capture_history::hash_frame(&output);
                let placement = self.inline.map(|inline| {
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
                if let Some((bounds, zoom)) = placement {
                    crate::pin::open_frame_at_with_hash(output, bounds, zoom, hash, cx);
                } else {
                    crate::pin::open_frame_with_hash(output, hash, cx);
                }
            }
            Err(error) => {
                let language = settings_state::get().language;
                self.status = Some(format!("{}{}", t(language, "editor.export_failed"), error));
            }
        }
        if self.inline.is_some() {
            self.release_images(cx);
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
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
                let output = self.effects.apply(output, self.region_effects_scale());
                let hash = crate::capture_history::hash_frame(&output);
                let record_history = self.pin_target.is_none();
                if copy_only || settings_state::get().copy_to_clipboard {
                    match image_util::copy_png(&output, cx) {
                        Ok(bytes) => {
                            copy_ok = true;
                            if record_history {
                                crate::capture_history::record_png_bytes(&bytes, hash);
                            }
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
                    if crate::pin::replace_image(pin_id, output, cx) {
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
        self.status = Some(messages.join(" · "));
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
        if let Some(resize_draft) = &self.resize_draft
            && let Some(annotation) = objects
                .iter_mut()
                .find(|annotation| annotation.id == resize_draft.id)
        {
            annotation.shape = resize_shape(
                &resize_draft.original,
                resize_draft.handle,
                resize_draft.start,
                resize_draft.current,
                4,
            );
        }
        let selected_shape = self
            .document
            .selected()
            .and_then(|id| objects.iter().find(|annotation| annotation.id == id))
            .map(|annotation| annotation.shape.clone());
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
                if let Some(shape) = &selected_shape {
                    paint_selection(window, bounds, scale, shape, accent_hex);
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
            let text_rgb = (color >> 8) & 0x00FF_FFFF;
            let mut layer = div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
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
            layer
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
        let effects_scale = self.region_effects_scale();
        let inline = self.inline.is_some();
        let image_cursor = if self.move_draft.is_some() {
            CursorStyle::ClosedHand
        } else {
            match self.selection_hover {
                SelectionHover::Handle(handle) => handle_cursor(handle),
                SelectionHover::Body if self.tool == Tool::Select => CursorStyle::OpenHand,
                SelectionHover::Body => CursorStyle::Crosshair,
                SelectionHover::None if self.tool == Tool::Select => CursorStyle::Arrow,
                SelectionHover::None => CursorStyle::Crosshair,
            }
        };
        let outside_backdrop = inline.then(|| {
            div().absolute().inset_0().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
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
                self.effects.corner_radius as f32 * effects_scale
            } else {
                0.0
            }))
            .when(effects_active && self.effects.border, |layer| {
                layer.border_1().border_color(rgba(0xFFFFFFCC))
            })
            .when(effects_active && self.effects.shadow, |layer| {
                layer.shadow(vec![
                    BoxShadow::new(px(0.0), px(8.0 * effects_scale), hsla(0.0, 0.0, 0.0, 0.35))
                        .blur_radius(px(20.0 * effects_scale)),
                ])
            })
            .overflow_hidden()
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
                        && let Some(handle) =
                            hit_resize_handle(&annotation.shape, point, 9.0 / scale)
                    {
                        this.resize_draft = Some(ResizeDraft {
                            id: annotation.id,
                            handle,
                            start: point,
                            current: point,
                            original: annotation.shape,
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
                        this.draft = None;
                        this.resize_draft = None;
                        if this.document.selected() != Some(id) {
                            this.select_annotation(Some(id));
                        }
                        this.move_draft = Some(MoveDraft {
                            id,
                            start: point,
                            current: point,
                        });
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
                    if this.tool == Tool::Polyline {
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
                        this.status = Some(
                            t(settings_state::get().language, "editor.polyline_hint").to_owned(),
                        );
                        cx.notify();
                        return;
                    }
                    if this.tool == Tool::Select {
                        let selected = hit;
                        this.resize_draft = None;
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
                    this.resize_draft = None;
                    this.move_draft = None;
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
                        shift: event.modifiers.shift,
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
                if let Some(resize_draft) = &mut this.resize_draft {
                    resize_draft.current = point;
                    cx.notify();
                    return;
                }
                if let Some(move_draft) = &mut this.move_draft {
                    move_draft.current = point;
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
                        let point = this
                            .to_image_point(event.position, window)
                            .or_else(|| this.resize_draft.as_ref().map(|draft| draft.current));
                        if let Some(point) = point {
                            this.finish_resize(point, cx);
                        } else {
                            this.resize_draft = None;
                            cx.notify();
                        }
                        return;
                    }
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
                if hover_reset
                    || this.draft.take().is_some()
                    || this.move_draft.take().is_some()
                    || this.resize_draft.take().is_some()
                    || this.pan_draft.take().is_some()
                {
                    cx.notify();
                }
            }));

        let toolbar_rect = self.toolbar_rect(window);
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
        let status_toast = status.map(|message| {
            div()
                .absolute()
                .right(px(16.0))
                .top(px((f32::from(toolbar_rect.origin.y) - 38.0).max(0.0)))
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
                    this.refresh_capture(cx);
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
                if key == "escape" || (command && matches!(key, "w" | "q")) {
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
                        this.status = Some(
                            t(settings_state::get().language, "editor.annotation_pasted")
                                .to_owned(),
                        );
                        cx.notify();
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
            .children(info_bar)
            .child(toolbar)
            .children(options_panel)
            .children(more_panel)
            .children(effects_panel)
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
    cx: &mut App,
) -> bool {
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
    let (window_bounds, display_id) = match monitor {
        Some(monitor) => {
            let scale = monitor.scale.get().max(0.1);
            (
                Bounds {
                    origin: point(
                        px(monitor.bounds.origin.x as f32 / scale),
                        px(monitor.bounds.origin.y as f32 / scale),
                    ),
                    size: size(
                        px(monitor.bounds.size.width as f32 / scale),
                        px(monitor.bounds.size.height as f32 / scale),
                    ),
                },
                Some(DisplayId::new(monitor.id.raw())),
            )
        }
        None => match cx.primary_display() {
            Some(display) => (display.bounds(), Some(display.id())),
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
        window_bounds.origin,
        None,
        true,
        Tool::Select,
        cx,
    )
}

fn fit_long_capture_rect(frame: &RgbaFrame, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    const MARGIN: f32 = 24.0;
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
        let editor = cx.new(|cx| {
            EditorView::new_inline(
                frame,
                image_rect,
                screen_origin,
                initial_tool,
                None,
                pin_target,
                zoomable,
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
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::Rect(rect))
        }
        Tool::Ellipse => {
            let rect = constrained_rect(draft.start, end, draft.shift);
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
            let rect = normalized_rect(draft.start, end);
            (rect.size.width >= 2 && rect.size.height >= 2).then_some(Shape::HighlighterBox(rect))
        }
        Tool::Text | Tool::Select | Tool::Number | Tool::Polyline => None,
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

fn paint_selection(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scale: f32,
    shape: &Shape,
    accent_hex: u32,
) {
    let rect = shape.bounds();
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

    let draw_handle = |window: &mut Window, handle_pos: PhysicalPoint| {
        let center = to_point(handle_pos.x as f32, handle_pos.y as f32);
        for (half, color) in [
            (px(4.5), rgba((accent_hex << 8) | 0xFF)),
            (px(2.5), rgb(0xFFFFFF)),
        ] {
            let mut builder = PathBuilder::fill();
            builder.move_to(point(center.x - half, center.y - half));
            builder.line_to(point(center.x + half, center.y - half));
            builder.line_to(point(center.x + half, center.y + half));
            builder.line_to(point(center.x - half, center.y + half));
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
    };
    match shape {
        Shape::Arrow { start, end } => {
            draw_handle(window, *start);
            draw_handle(window, *end);
        }
        _ => {
            for (_, handle) in handle_points(shape) {
                draw_handle(window, handle);
            }
        }
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
        Shape::Arrow { start, end } => {
            let mut builder = PathBuilder::stroke(stroke);
            builder.move_to(to_point(start.x as f32, start.y as f32));
            builder.line_to(to_point(end.x as f32, end.y as f32));

            let dx = (end.x - start.x) as f32;
            let dy = (end.y - start.y) as f32;
            let angle = dy.atan2(dx);
            let head = (style.stroke_width * 3.5).max(10.0);
            let spread = 0.55;
            let push_head = |builder: &mut PathBuilder, tip_x: f32, tip_y: f32, angle: f32| {
                let left = (
                    tip_x - head * (angle - spread).cos(),
                    tip_y - head * (angle - spread).sin(),
                );
                let right = (
                    tip_x - head * (angle + spread).cos(),
                    tip_y - head * (angle + spread).sin(),
                );
                builder.move_to(to_point(left.0, left.1));
                builder.line_to(to_point(tip_x, tip_y));
                builder.line_to(to_point(right.0, right.1));
            };
            if style.arrow_end {
                push_head(&mut builder, end.x as f32, end.y as f32, angle);
            }
            if style.arrow_start {
                push_head(
                    &mut builder,
                    start.x as f32,
                    start.y as f32,
                    (-dy).atan2(-dx),
                );
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Polyline { points } => {
            if points.len() < 2 {
                return;
            }
            let mut builder = PathBuilder::stroke(stroke);
            builder.move_to(to_point(points[0].x as f32, points[0].y as f32));
            for point in &points[1..] {
                builder.line_to(to_point(point.x as f32, point.y as f32));
            }
            let head = (style.stroke_width * 3.5).max(10.0);
            let spread = 0.55;
            let push_head = |builder: &mut PathBuilder, tip: PhysicalPoint, angle: f32| {
                let left = (
                    tip.x as f32 - head * (angle - spread).cos(),
                    tip.y as f32 - head * (angle - spread).sin(),
                );
                let right = (
                    tip.x as f32 - head * (angle + spread).cos(),
                    tip.y as f32 - head * (angle + spread).sin(),
                );
                builder.move_to(to_point(left.0, left.1));
                builder.line_to(to_point(tip.x as f32, tip.y as f32));
                builder.line_to(to_point(right.0, right.1));
            };
            if style.arrow_end {
                let last = points[points.len() - 1];
                let previous = points[points.len() - 2];
                push_head(
                    &mut builder,
                    last,
                    ((last.y - previous.y) as f32).atan2((last.x - previous.x) as f32),
                );
            }
            if style.arrow_start {
                let first = points[0];
                let next = points[1];
                push_head(
                    &mut builder,
                    first,
                    ((first.y - next.y) as f32).atan2((first.x - next.x) as f32),
                );
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
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
        Shape::Spotlight(rect) => {
            paint_spotlight(window, bounds, scale, *rect, style.spotlight_intensity)
        }
    }
}

fn paint_spotlight(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scale: f32,
    rect: PhysicalRect,
    intensity: u8,
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
            window.paint_path(path, rgba(u32::from(alpha)));
        }
    }
}
