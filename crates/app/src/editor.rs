use std::{ops::Range, sync::Arc, time::Duration};

use core_annotate::{AnnotationDocument, AnnotationStyle, Shape, TextAlignment, rasterize};
use core_capture::CaptureTarget;
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
use crate::editor_state::{
    BAND_HALF, CropDraft, Draft, EditorSnapshot, EditorState, FullCanvas, HANDLE_HIT, HistoryStep,
    MoveDraft, PolylineDraft, REGION_HANDLE_SIZE, ResizeDraft, ResizeHandle, SELECTION_HANDLE_HALF,
    TextEdit, annotation_handle_at, distance, finalize_shape, fit_ui_scale, handle_points,
    normalized_rect,
};
use crate::effects::RegionEffects;
use crate::logging::{log_error, log_info};
use crate::selection_geometry::{
    ResizeHandle as RegionHandle, corner_handle_offset, edge_resize_handle, handle_positions,
    region_handle_cursor, resize_rect,
};
use crate::settings_state::{AnnotationColorTool, DoubleClickAction};
use crate::ui::tooltip_view;
use crate::{i18n::t, icons, image_util, settings_state};

pub(crate) use crate::editor_state::Tool;

impl Tool {
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
}

mod blur_cache;
mod canvas;
mod export;
mod export_pipeline;
mod panels;
mod render;
mod render_handlers;
mod text;
mod toolbar;
mod view;

use blur_cache::BlurCache;

use canvas::push_preview_rounded_rect;

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
const SPOTLIGHT_INTENSITIES: [u8; 3] = [20, 40, 60];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionHover {
    None,
    Body,
    Handle(ResizeHandle),
    Region(RegionHandle),
}

struct PanDraft {
    start: Point<Pixels>,
    start_pan: Point<Pixels>,
}

thread_local! {
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

pub(crate) fn has_open_editors() -> bool {
    EDITOR_VIEWS.with(|views| {
        views
            .borrow()
            .iter()
            .any(|(view, _)| view.upgrade().is_some())
    })
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
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

#[derive(Clone, Copy)]
struct InlineHost {
    image_rect: Bounds<Pixels>,
    screen_origin: Point<Pixels>,
    capture: Option<InlineCapture>,
    display: Option<DisplayId>,
    zoomable: bool,
    fit_area: Option<Size<Pixels>>,
}

#[derive(Clone, Copy)]
pub(crate) struct InlineCapture {
    pub(crate) region: CaptureRegion,
    pub(crate) target: CaptureTarget,
    pub(crate) scale: f32,
    pub(crate) monitor_origin: PhysicalPoint,
    pub(crate) effects: RegionEffects,
}

#[derive(Clone)]
pub(crate) struct FullCanvasInit {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) image_size: PhysicalSize,
    pub(crate) adjustable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportAction {
    Copy,
    Save,
}

struct RenderedExport {
    document: AnnotationDocument,
    output: RgbaFrame,
    pin_output: Option<RgbaFrame>,
    hash: u64,
    record_history: bool,
    pin_shadow: Option<crate::capture_history::HistoryPinShadow>,
    scale: f32,
    pin_padding: u32,
    stored_effects: RegionEffects,
    pin_shadow_always: bool,
}

#[derive(Clone, Copy)]
struct TextLineBox {
    start: usize,
    end: usize,
    x: f32,
    width: f32,
    y: f32,
    hard_line: usize,
}

fn layout_annotation_text(
    window: &Window,
    text: &str,
    font_size: f32,
    width: f32,
) -> Option<Vec<WrappedLine>> {
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
        .ok()
        .map(|lines| lines.into_iter().collect())
}

fn text_line_boxes(layouts: &[WrappedLine], line_height: f32) -> Vec<TextLineBox> {
    let mut boxes = Vec::new();
    let mut base = 0usize;
    for (hard_line, layout) in layouts.iter().enumerate() {
        let mut start = 0usize;
        let mut start_x = 0.0f32;
        for boundary in layout.wrap_boundaries() {
            let run = &layout.runs()[boundary.run_ix];
            let glyph = &run.glyphs[boundary.glyph_ix];
            let end = glyph.index;
            let end_x = f32::from(glyph.position.x);
            boxes.push(TextLineBox {
                start: base + start,
                end: base + end,
                x: start_x,
                width: (end_x - start_x).max(0.0),
                y: boxes.len() as f32 * line_height,
                hard_line,
            });
            start = end;
            start_x = end_x;
        }
        let end_x = f32::from(layout.unwrapped_layout.width);
        boxes.push(TextLineBox {
            start: base + start,
            end: base + layout.len(),
            x: start_x,
            width: (end_x - start_x).max(0.0),
            y: boxes.len() as f32 * line_height,
            hard_line,
        });
        base += layout.len() + 1;
    }
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
    layouts: Option<&[WrappedLine]>,
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
    let local = cursor.saturating_sub(line.start);
    let x = layouts
        .and_then(|layouts| layouts.get(line.hard_line))
        .map_or(0.0, |layout| {
            let local = local.min(layout.len());
            (f32::from(layout.unwrapped_layout.x_for_index(local)) - line.x).clamp(0.0, line.width)
        });
    let offset = text_alignment_offset(edit.style.text_alignment, container_width, line.width);
    point(px(x + offset), px(line.y))
}

fn text_selection_boxes(
    layouts: &[WrappedLine],
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
        let Some(layout) = layouts.get(line.hard_line) else {
            continue;
        };
        let local_start = (start - line.start).min(layout.len());
        let local_end = (end - line.start).min(layout.len());
        let x1 = (f32::from(layout.unwrapped_layout.x_for_index(local_start)) - line.x)
            .clamp(0.0, line.width);
        let x2 = (f32::from(layout.unwrapped_layout.x_for_index(local_end)) - line.x)
            .clamp(0.0, line.width);
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
    let chars: Vec<char> = content.chars().collect();
    let mut offsets = Vec::with_capacity(chars.len() + 1);
    offsets.extend(content.char_indices().map(|(offset, _)| offset));
    offsets.push(content.len());
    let char_index = offsets.partition_point(|offset| *offset < index);
    let Some(range) =
        crate::text_scan::word_range(&chars, char_index, |ch| ch.is_alphanumeric() || ch == '_')
    else {
        return 0..0;
    };
    offsets[range.start]..offsets[range.end]
}

pub(crate) struct EditorView {
    image: Arc<RenderImage>,
    blur_cache: BlurCache,
    images_released: bool,
    frame: Arc<RgbaFrame>,
    region_source: Option<Arc<RenderImage>>,
    pin_target: Option<u64>,
    pin_shadow_always: bool,
    pin_effects_scale: f32,
    selection_hover: SelectionHover,
    pan_draft: Option<PanDraft>,
    view_zoom: f32,
    view_pan: Point<Pixels>,
    space_down: bool,
    text_edit_screen: Option<Bounds<Pixels>>,
    accent_hex: u32,
    chrome: ChromeTheme,
    appearance_subscription: Option<Subscription>,
    pub(crate) focus_handle: FocusHandle,
    inline: Option<InlineHost>,
    pub(crate) state: EditorState,
}

impl std::ops::Deref for EditorView {
    type Target = EditorState;

    fn deref(&self) -> &EditorState {
        &self.state
    }
}

impl std::ops::DerefMut for EditorView {
    fn deref_mut(&mut self) -> &mut EditorState {
        &mut self.state
    }
}

impl EditorView {
    fn new_with_image(
        frame: Arc<RgbaFrame>,
        image: Arc<RenderImage>,
        image_size: PhysicalSize,
        pin_target: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Self {
        let accent_hex =
            system_accent_hex().unwrap_or_else(|| crate::theme::fallback_accent(false));
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
        let highlighter_radius = settings.annotation_highlighter_radius;
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
            this.blur_cache.drop_images_now(cx);
        })
        .detach();
        let mut state = EditorState::base(AnnotationDocument::new(image_size), image_size);
        state.style = AnnotationStyle {
            color: style_color,
            stroke_width: rect_stroke_width,
            font_size,
            corner_radius,
            highlighter_radius,
            blur_radius,
            filled: false,
            arrow_start,
            arrow_end,
            text_background,
            text_outline,
            text_alignment,
            spotlight_intensity,
            spotlight_radius,
        };
        state.font_size = font_size;
        state.arrow_start = arrow_start;
        state.arrow_end = arrow_end;
        state.arrow_polyline = arrow_polyline;
        state.text_background = text_background;
        state.text_outline = text_outline;
        state.text_alignment = text_alignment;
        state.corner_radius = corner_radius;
        state.highlighter_radius = highlighter_radius;
        state.number_size = number_size;
        state.spotlight_radius = spotlight_radius;
        state.rect_stroke_width = rect_stroke_width;
        state.arrow_stroke_width = arrow_stroke_width;
        state.blur_radius = blur_radius;
        state.rect_filled = rect_filled;
        state.ellipse_filled = ellipse_filled;
        state.shape_ellipse = shape_ellipse;
        state.spotlight_intensity = spotlight_intensity;
        state.active_color_tool = active_color_tool;
        state.palette = if settings.annotation_uniform_color {
            [settings.annotation_color; crate::settings_state::ANNOTATION_COLOR_TOOL_COUNT]
        } else {
            settings.annotation_colors
        };
        state.uniform_color = settings.annotation_uniform_color;
        Self {
            image,
            blur_cache: BlurCache::default(),
            images_released: false,
            frame,
            region_source: None,
            pin_target,
            pin_shadow_always: false,
            pin_effects_scale: 1.0,
            selection_hover: SelectionHover::None,
            pan_draft: None,
            view_zoom: 1.0,
            view_pan: point(px(0.0), px(0.0)),
            space_down: false,
            text_edit_screen: None,
            accent_hex,
            chrome: ChromeTheme::for_appearance(false, Some(accent_hex)),
            appearance_subscription: None,
            focus_handle: cx.focus_handle(),
            inline: None,
            state,
        }
    }

    pub(crate) fn observe_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance_subscription =
            Some(cx.observe_window_appearance(window, |this, window, cx| {
                this.chrome = ChromeTheme::for_window(window, Some(this.accent_hex));
                cx.notify();
            }));
    }

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
        full_canvas: Option<FullCanvasInit>,
        cx: &mut Context<Self>,
    ) -> Self {
        let full_setting = full_canvas.as_ref().map(|full| FullCanvas {
            adjustable: full.adjustable,
        });
        let mut view = match full_canvas {
            Some(full) => Self::new_with_image(frame, full.image, full.image_size, pin_target, cx),
            None => {
                let image_size = PhysicalSize::new(frame.width(), frame.height());
                let image = image_util::render_image_ref(&frame);
                Self::new_with_image(frame, image, image_size, pin_target, cx)
            }
        };
        view.full_canvas = full_setting;
        view.ui_scale = fit_ui_scale(view.image_size.width, f32::from(image_rect.size.width));
        view.style.corner_radius = view.corner_radius * view.ui_scale;
        view.style.highlighter_radius = view.highlighter_radius * view.ui_scale;
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
            display: None,
            zoomable,
            fit_area: None,
        });
        view
    }

    pub(crate) fn set_display(&mut self, display: Option<DisplayId>) {
        if let Some(inline) = self.inline.as_mut() {
            inline.display = display;
        }
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
}

struct EditorOverlayHost {
    editor: Entity<EditorView>,
}

impl Render for EditorOverlayHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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

pub fn open_pin_editor_overlay(
    pin_id: u64,
    frame: Arc<RgbaFrame>,
    pin_bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    edit_state: Option<crate::pin::PinEditState>,
    cx: &mut App,
) -> bool {
    let (document, effects, shadow_always, display_padding, effects_scale) = match edit_state {
        Some(state) => (
            Some(state.document),
            state.effects,
            state.shadow_always,
            state.display_padding,
            state.effects_scale,
        ),
        None => (None, RegionEffects::default(), false, 0, 1.0),
    };
    let display = display_id
        .and_then(|id| cx.displays().into_iter().find(|display| display.id() == id))
        .or_else(|| cx.primary_display());
    let Some(display) = display else {
        log_error!("editor", "no display available for pin editing");
        return false;
    };
    let display_bounds = display.bounds();
    let output_width = frame.width() as f32 + display_padding as f32 * 2.0;
    let display_scale = f32::from(pin_bounds.size.width) / output_width.max(1.0);
    let inset = display_padding as f32 * display_scale;
    let image_rect = Bounds {
        origin: point(
            px(f32::from(pin_bounds.origin.x) - f32::from(display_bounds.origin.x) + inset),
            px(f32::from(pin_bounds.origin.y) - f32::from(display_bounds.origin.y) + inset),
        ),
        size: size(
            px((f32::from(pin_bounds.size.width) - inset * 2.0).max(1.0)),
            px((f32::from(pin_bounds.size.height) - inset * 2.0).max(1.0)),
        ),
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
        shadow_always,
        effects_scale,
        None,
        cx,
    )
}

pub fn open_long_capture_inline(
    frame: RgbaFrame,
    monitor_id: MonitorId,
    missed_frames: u32,
    cx: &mut App,
) -> bool {
    let frame = Arc::new(frame);
    let language = settings_state::get().language;
    let notice = (missed_frames > 0).then(|| {
        format!(
            "{} ({})",
            t(language, "scroll.missed_summary"),
            missed_frames
        )
    });
    let monitors = platform_capture::platform_backend()
        .monitors()
        .unwrap_or_default();
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
    let image_rect = fit_frame_rect(
        PhysicalSize::new(frame.width(), frame.height()),
        window_bounds.size,
    );
    let effects_scale = monitor.map(|monitor| monitor.scale.get()).unwrap_or(1.0);
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
        false,
        effects_scale,
        notice,
        cx,
    )
}

fn fit_frame_rect(image_size: PhysicalSize, area: Size<Pixels>) -> Bounds<Pixels> {
    const MARGIN: f32 = 44.0;
    const GAP: f32 = 8.0;
    let area_width = f32::from(area.width);
    let area_height = f32::from(area.height);
    let available_width = (area_width - MARGIN * 2.0).max(1.0);
    let available_height = (area_height - MARGIN * 2.0 - COMPACT_TOOLBAR_HEIGHT - GAP).max(1.0);
    let fit = (available_width / image_size.width.max(1) as f32)
        .min(available_height / image_size.height.max(1) as f32)
        .min(1.0);
    let width = image_size.width as f32 * fit;
    let height = image_size.height as f32 * fit;
    let x = (area_width - width) / 2.0;
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
    pin_shadow_always: bool,
    pin_effects_scale: f32,
    notice: Option<String>,
    cx: &mut App,
) -> bool {
    let options = crate::aux_window::popup_options(
        "capin-editor-inline",
        window_bounds,
        display_id,
        true,
        inline_editor_window_kind(),
        WindowBackgroundAppearance::Transparent,
    );

    cx.activate(true);
    match gpui_kit::open_window(options, cx, move |window, cx| {
        if let Err(error) = apply_window_style(&*window, WindowStyle::INLINE_EDITOR) {
            log_error!("editor", "failed to style inline editor: {error}");
        }
        hide_native_border(&*window);
        crate::ui::align_client_to_bounds(&*window, window_bounds);
        let editor = cx.new(|cx| {
            let mut editor = EditorView::new_inline(
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
                None,
                cx,
            );
            editor.pin_shadow_always = pin_shadow_always;
            editor.pin_effects_scale = pin_effects_scale.max(0.1);
            if let Some(inline) = editor.inline.as_mut() {
                inline.display = display_id;
                if zoomable {
                    inline.fit_area = Some(window_bounds.size);
                }
            }
            editor
        });
        if let Some(message) = notice {
            editor.update(cx, |this, cx| this.show_status(message, cx));
        }
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

fn full_selection_bounds(
    image_rect: Bounds<Pixels>,
    image_size: PhysicalSize,
    selection: PhysicalRect,
) -> Option<Bounds<Pixels>> {
    let scale = f32::from(image_rect.size.width) / image_size.width.max(1) as f32;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some(Bounds {
        origin: point(
            px(f32::from(image_rect.origin.x) + selection.origin.x as f32 * scale),
            px(f32::from(image_rect.origin.y) + selection.origin.y as f32 * scale),
        ),
        size: size(
            px(selection.size.width as f32 * scale),
            px(selection.size.height as f32 * scale),
        ),
    })
}

fn cropped_document(document: &AnnotationDocument, selection: PhysicalRect) -> AnnotationDocument {
    document.crop_to(selection.origin, selection.size)
}

fn finish_pointer_gestures(
    view: &mut EditorView,
    event: &MouseUpEvent,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) -> bool {
    let mut changed = false;
    if view.region_adjust.draft.is_some() {
        view.finish_region_drag(cx);
        changed = true;
    }
    if view.move_draft.is_some() {
        let point = view
            .to_image_point(event.position, window)
            .or_else(|| view.move_draft.as_ref().map(|draft| draft.current));
        if let Some(point) = point {
            view.state.finish_move(point);
        } else {
            view.state.cancel_move();
        }
        view.status = None;
        changed = true;
    }
    if view.resize_draft.is_some() {
        if let Some(point) = view.resize_draft.as_ref().map(|draft| draft.current) {
            view.finish_resize(point, cx);
        } else {
            view.state.cancel_resize();
        }
        changed = true;
    }
    if let Some(mut draft) = view.draft.take() {
        let point = view
            .to_image_point(event.position, window)
            .unwrap_or(draft.current);
        draft.current = point;
        draft.shift = event.modifiers.shift;
        if let Some(shape) = finalize_shape(&draft, point) {
            let style = view.effective_style();
            view.state.add_annotation(shape, style);
            view.status = None;
        }
        changed = true;
    }
    changed
}

fn pin_placement(
    inline: Option<InlineHost>,
    selection_bounds: Option<Bounds<Pixels>>,
    image_size: PhysicalSize,
    width: u32,
    height: u32,
) -> Option<(Bounds<Pixels>, f32)> {
    inline.map(|inline| {
        let display_scale = if image_size.width == 0 {
            1.0
        } else {
            f32::from(inline.image_rect.size.width) / image_size.width as f32
        }
        .max(0.01);
        let width = width as f32 * display_scale;
        let height = height as f32 * display_scale;
        let (center_x, center_y) = if let Some(selection) = selection_bounds {
            (
                f32::from(inline.screen_origin.x)
                    + f32::from(selection.origin.x)
                    + f32::from(selection.size.width) / 2.0,
                f32::from(inline.screen_origin.y)
                    + f32::from(selection.origin.y)
                    + f32::from(selection.size.height) / 2.0,
            )
        } else {
            (
                f32::from(inline.screen_origin.x)
                    + f32::from(inline.image_rect.origin.x)
                    + f32::from(inline.image_rect.size.width) / 2.0,
                f32::from(inline.screen_origin.y)
                    + f32::from(inline.image_rect.origin.y)
                    + f32::from(inline.image_rect.size.height) / 2.0,
            )
        };
        (
            Bounds {
                origin: point(px(center_x - width / 2.0), px(center_y - height / 2.0)),
                size: size(px(width), px(height)),
            },
            display_scale,
        )
    })
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
            push_preview_rounded_rect(&mut builder, &to_point, left, top, right, bottom, radius);
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
            let radius = (style.highlighter_radius * scale)
                .clamp(0.0, (right - left).min(bottom - top) / 2.0);
            let mut builder = PathBuilder::fill();
            push_preview_rounded_rect(&mut builder, &to_point, left, top, right, bottom, radius);
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
    let hole = Bounds {
        origin: point(px(hole_left), px(hole_top)),
        size: size(px(hole_right - hole_left), px(hole_bottom - hole_top)),
    };
    crate::ui::paint_rounded_mask(
        window,
        bounds,
        hole,
        (radius * scale).max(0.0),
        rgba(u32::from(alpha)),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::{TextEdit, cropped_document, word_range_at};
    use crate::ui::hue_rgb;
    use core_annotate::{AnnotationDocument, AnnotationStyle, Shape};
    use core_types::{PhysicalPoint, PhysicalRect, PhysicalSize};

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
        let resized = crate::editor_state::resize_shape(
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

        let resized = crate::editor_state::resize_shape(
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
        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(1440.0), px(900.0)),
        };
        let rect = super::fit_frame_rect(core_types::PhysicalSize::new(400, 12_000), bounds.size);
        assert!(f32::from(rect.origin.y) >= 44.0);
        assert!(f32::from(rect.origin.y) + f32::from(rect.size.height) <= 900.0 - 44.0);
        assert!(f32::from(rect.origin.x) >= 44.0);
        assert!(f32::from(rect.size.width) <= 1440.0 - 88.0 + 0.5);
    }

    #[test]
    fn refit_uses_the_new_canvas_size() {
        use gpui_kit::{px, size};
        let area = size(px(1440.0), px(900.0));
        let tall = super::fit_frame_rect(core_types::PhysicalSize::new(400, 12_000), area);
        let wide = super::fit_frame_rect(core_types::PhysicalSize::new(400, 200), area);
        assert!(f32::from(wide.size.width) > f32::from(tall.size.width));
        assert!(f32::from(wide.size.height) < 900.0 - 44.0);
        assert!(
            (f32::from(wide.size.width) / f32::from(wide.size.height) - 2.0).abs() < 0.01,
            "the fitted rect must keep the new canvas aspect ratio"
        );
    }

    #[test]
    fn cropped_document_translates_objects_and_keeps_outside_data() {
        let mut document = AnnotationDocument::new(PhysicalSize::new(100, 100));
        document.add(
            Shape::Rect(PhysicalRect::new(10, 20, 30, 40)),
            AnnotationStyle::default(),
        );
        document.add(
            Shape::Rect(PhysicalRect::new(80, 80, 10, 10)),
            AnnotationStyle::default(),
        );

        let cropped = cropped_document(&document, PhysicalRect::new(20, 10, 50, 50));
        assert_eq!(cropped.image_size(), PhysicalSize::new(50, 50));
        assert_eq!(cropped.objects().len(), 2);
        assert_eq!(
            cropped.objects()[0].shape.bounds(),
            PhysicalRect::new(-10, 10, 30, 40)
        );
        assert_eq!(
            cropped.objects()[1].shape.bounds(),
            PhysicalRect::new(60, 70, 10, 10)
        );
        assert!(cropped.selected().is_none());
    }

    #[test]
    fn full_selection_bounds_offset_the_frame_inside_the_monitor() {
        use gpui_kit::{Bounds, point, px, size};
        let image = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(200.0), px(100.0)),
        };
        let bounds = super::full_selection_bounds(
            image,
            PhysicalSize::new(400, 200),
            PhysicalRect::new(40, 20, 100, 60),
        )
        .unwrap();
        assert_eq!(f32::from(bounds.origin.x), 20.0);
        assert_eq!(f32::from(bounds.origin.y), 10.0);
        assert_eq!(f32::from(bounds.size.width), 50.0);
        assert_eq!(f32::from(bounds.size.height), 30.0);
    }
}
