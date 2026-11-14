use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use core_annotate::{AnnotationDocument, AnnotationStyle, Shape, TextAlignment, rasterize};
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
use crate::editor_state::{
    BAND_HALF, CropDraft, Draft, EditorSnapshot, EditorState, FullCanvas, HANDLE_HIT, MoveDraft,
    PolylineDraft, REGION_HANDLE_SIZE, RegionAdjustDraft, ResizeDraft, ResizeHandle,
    SELECTION_HANDLE_HALF, TextEdit, annotation_handle_at, distance, finalize_shape, handle_points,
    next_char_boundary, normalized_rect, previous_char_boundary,
};
use crate::effects::RegionEffects;
use crate::logging::{log_error, log_info};
use crate::selection_geometry::{
    ResizeHandle as RegionHandle, corner_handle_offset, edge_resize_handle, handle_positions,
    region_handle_cursor, resize_rect,
};
use crate::settings_state::AnnotationColorTool;
use crate::ui::tooltip_view;
use crate::{i18n::t, icons, image_util, settings_state};

pub(crate) use crate::editor_state::Tool;

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
const CANVAS_HISTORY_LIMIT: usize = 3;

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

/// What the pointer is currently over inside the image, used to swap the
/// painted selection handles from the drawing crosshair to resize arrows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionHover {
    None,
    Body,
    Handle(ResizeHandle),
    /// Edge of the adjustable export frame in a full-canvas region editor.
    Region(RegionHandle),
}

struct PanDraft {
    start: Point<Pixels>,
    start_pan: Point<Pixels>,
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
    /// Client area used to fit the image. Set for zoomable editors so a crop
    /// or undo can re-fit the new canvas instead of stretching it into the
    /// original tall rect.
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

/// One visual line produced by GPUI's text shaper. The editor and exporter use
/// different text engines, so hit-testing and the caret must follow the same
/// layout that the preview actually paints.
#[derive(Clone, Copy)]
struct TextLineBox {
    /// UTF-8 byte offsets in the edited content. The newline that ends a hard
    /// line is not part of any box, so the next hard line starts one byte later.
    start: usize,
    end: usize,
    x: f32,
    width: f32,
    y: f32,
    hard_line: usize,
}

/// Shapes every hard line. `shape_text` splits on `\n` and returns one
/// `WrappedLine` per hard line; keeping only the first one made multi-line
/// text hit-test and paint its caret against the first line.
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
    pin_target: Option<u64>,
    /// The pin being edited came from a region capture, which always displays
    /// the composed shadow even when the export setting leaves it off.
    pin_shadow_always: bool,
    /// Capture scale of a pin's stored effects; pin editors have no inline
    /// capture descriptor to read it from.
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
    /// Pure editing state. Field access goes through `Deref` so the split can
    /// land incrementally without rewriting every call site.
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
    fn new(frame: Arc<RgbaFrame>, pin_target: Option<u64>, cx: &mut Context<Self>) -> Self {
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image_ref(&frame);
        Self::new_with_image(frame, image, image_size, pin_target, cx)
    }

    /// Builds an editor whose display image and coordinate space can differ
    /// from the CPU frame used for export. Region screenshots pass the frozen
    /// monitor BGRA image plus a cropped selection copy; pin and long-capture
    /// editors pass the image built from their frame.
    fn new_with_image(
        frame: Arc<RgbaFrame>,
        image: Arc<RenderImage>,
        image_size: PhysicalSize,
        pin_target: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Self {
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
            state: EditorState {
                document: AnnotationDocument::new(image_size),
                image_size,
                tool: Tool::Select,
                style: AnnotationStyle {
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
                highlighter_radius,
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
                effects: RegionEffects::default(),
                effects_open: false,
                more_open: false,
                toolbar_hidden: false,
                hints_visible: false,
                status: None,
                status_serial: 0,
                region_drag: None,
                full_canvas: None,
            },
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
        full_canvas: Option<FullCanvasInit>,
        cx: &mut Context<Self>,
    ) -> Self {
        let full_setting = full_canvas.as_ref().map(|full| FullCanvas {
            adjustable: full.adjustable,
        });
        let mut view = match full_canvas {
            Some(full) => Self::new_with_image(frame, full.image, full.image_size, pin_target, cx),
            None => Self::new(frame, pin_target, cx),
        };
        view.full_canvas = full_setting;
        view.ui_scale = if image_rect.size.width > px(0.0) {
            (view.image_size.width as f32 / f32::from(image_rect.size.width)).clamp(0.25, 32.0)
        } else {
            1.0
        };
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
            zoomable,
            fit_area: None,
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
        let hit = self.document.hit_test(point);
        let hover = if let Some(handle) = handle {
            SelectionHover::Handle(handle)
        } else if hit.is_some() {
            SelectionHover::Body
        } else if let Some(handle) = self.region_handle_at(point) {
            SelectionHover::Region(handle)
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
        if self.full_canvas.is_some() {
            // Full-canvas editors show the whole monitor; the selection is an
            // arbitrary rect inside it, not the image boundary.
            let selection = self.live_selection_rect().unwrap_or(capture.region.rect);
            return full_selection_bounds(image, self.image_size, selection);
        }
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

    /// The selection frame the user currently sees. While the export frame is
    /// being dragged this is the live preview rectangle, so the dim mask,
    /// border, handles and size readout follow the pointer in real time the
    /// way the initial capture selection does.
    fn live_selection_rect(&self) -> Option<PhysicalRect> {
        self.region_drag
            .filter(|_| self.full_canvas.is_some())
            .map(|draft| draft.current)
            .or_else(|| self.full_selection())
    }

    /// Selection frame in full-monitor physical pixels.
    fn full_selection(&self) -> Option<PhysicalRect> {
        self.full_canvas.as_ref()?;
        Some(self.inline?.capture?.region.rect)
    }

    /// Builds the annotation document for the current export frame. Full-canvas
    /// editors keep objects in monitor coordinates, so export translates them
    /// into the cropped output instead of cropping the live document.
    fn action_document(&self) -> AnnotationDocument {
        let Some(selection) = self.full_selection() else {
            return self.document.clone();
        };
        cropped_document(&self.document, selection)
    }

    /// Selection frame in window logical coordinates for pin placement.
    fn full_selection_screen_bounds(&self) -> Option<Bounds<Pixels>> {
        let selection = self.full_selection()?;
        let inline = self.inline?;
        full_selection_bounds(inline.image_rect, self.image_size, selection)
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
        let width = 216.0_f32.min((window_width - 16.0).max(120.0));
        let height = 40.0;
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
        let zoom = (self.view_zoom * factor).clamp(0.25, 32.0);
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

    /// Re-fits the canvas after its pixel size changed. Without this a crop or
    /// an undo kept the original tall display rect, which stretched the new
    /// image to the old aspect ratio.
    fn refit_zoomable_view(&mut self) {
        let Some(mut inline) = self.inline else {
            return;
        };
        if !inline.zoomable {
            return;
        }
        let Some(area) = inline.fit_area else {
            return;
        };
        inline.image_rect = fit_frame_rect(self.image_size, area);
        let width = f32::from(inline.image_rect.size.width).max(1.0);
        self.ui_scale = (self.image_size.width as f32 / width).clamp(0.25, 32.0);
        self.inline = Some(inline);
    }

    fn actual_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let fit = self.fit_scale(window);
        self.view_zoom = (1.0 / fit).clamp(0.25, 32.0);
        self.view_pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    fn push_canvas_snapshot(&mut self) {
        let snapshot = EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        };
        self.canvas_undo.push(snapshot);
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
        let image = if self.full_canvas.is_some() {
            image_util::blurred_image_from_render(&self.image, radius)
        } else {
            image_util::blurred_image(&self.frame, radius)
        };
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
        self.refit_zoomable_view();
        self.reset_view(cx);
    }

    fn undo_canvas(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.canvas_undo.pop() else {
            return false;
        };
        let current = EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        };
        self.canvas_redo.push(current);
        self.restore_canvas_snapshot(snapshot, cx);
        true
    }

    fn redo_canvas(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.canvas_redo.pop() else {
            return false;
        };
        let current = EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        };
        self.canvas_undo.push(current);
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
        self.refit_zoomable_view();
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
        let has_source = self.region_source.is_some()
            || self
                .full_canvas
                .is_some_and(|full_canvas| full_canvas.adjustable);
        self.tool == Tool::Select
            && has_source
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
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let Some(draft) = self.region_drag.as_ref().copied() else {
            return;
        };
        let image_size = if self.full_canvas.is_some() {
            self.image_size
        } else {
            let Some(source) = self.region_source.clone() else {
                return;
            };
            image_util::image_size(&source)
        };
        let scale = capture.scale.max(0.1);
        let cursor_point = PhysicalPoint::new(
            (f32::from(position.x) * scale).round() as i32,
            (f32::from(position.y) * scale).round() as i32,
        );
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
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        if self.full_canvas.is_some() {
            if draft.current == draft.original {
                return;
            }
            let Some(cropped) = image_util::cropped_rgba(&self.image, draft.current) else {
                return;
            };
            self.frame = Arc::new(cropped);
            if let Some(inline) = self.inline.as_mut()
                && let Some(capture) = inline.capture.as_mut()
            {
                capture.region = CaptureRegion::new(capture.region.monitor, draft.current);
                capture.target = CaptureTarget::Region(capture.region);
            }
            // Keep the object data, but do not leave a hidden object selected
            // outside the new export frame.
            if let Some(id) = self.document.selected()
                && self
                    .document
                    .object(id)
                    .is_none_or(|object| !object.shape.bounds().intersects(draft.current))
            {
                self.document.select(None);
                self.selection_hover = SelectionHover::None;
            }
            self.status = None;
            cx.notify();
            return;
        }
        let Some(source) = self.region_source.clone() else {
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
        if self.full_canvas.is_some() {
            return;
        }
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

    /// Leaves the current annotation state exactly like the first Escape
    /// press: cancel the text or polyline in progress, close the tool
    /// panels, or drop the selection and return to Select. Returns false
    /// when the editor is already idle, so the caller can decide whether
    /// Escape should close the editor.
    fn cancel_annotation_state(&mut self, cx: &mut Context<Self>) -> bool {
        if self.text_edit.is_some() {
            self.cancel_text(cx);
            return true;
        }
        if self.polyline_draft.is_some() {
            self.cancel_polyline(cx);
            return true;
        }
        if self.more_open || self.effects_open || self.hints_visible {
            self.more_open = false;
            self.effects_open = false;
            self.hints_visible = false;
            cx.notify();
            return true;
        }
        if self.document.selected().is_some() || self.tool != Tool::Select {
            self.select_annotation(None);
            self.tool = Tool::Select;
            self.draft = None;
            self.move_draft = None;
            self.resize_draft = None;
            self.pan_draft = None;
            self.selection_hover = SelectionHover::None;
            self.status = None;
            cx.notify();
            return true;
        }
        false
    }

    /// Right click never leaves the capture. While annotating it behaves like
    /// Escape and leaves the tool; an annotation already in progress is
    /// confirmed first (text commit, polyline finish, crop apply). The image
    /// layer carries this handler as well, because its opaque hitbox blocks
    /// events from reaching handlers on the editor root.
    fn right_click(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.text_edit.is_some() {
            self.commit_text(cx);
            return;
        }
        if self.polyline_draft.is_some() {
            self.finish_polyline(cx);
            return;
        }
        if self.crop_mode {
            if self.crop_draft.is_some() {
                self.apply_crop(cx);
            } else {
                self.cancel_crop(cx);
            }
            return;
        }
        let _ = self.cancel_annotation_state(cx);
    }

    fn region_handles(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.can_adjust_region() {
            return Vec::new();
        }
        let rect = self
            .inline_selection_rect(window)
            .unwrap_or_else(|| self.image_view_rect(window));
        let radius = if self.region_effects_active() {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let mut handles = Vec::new();
        for (handle, position) in handle_positions(rect) {
            let (dx, dy) = corner_handle_offset(handle, radius);
            handles.push(
                div()
                    .id(format!("editor-region-handle-{:?}", handle))
                    .absolute()
                    .left(px(f32::from(position.x) + dx - HANDLE_HIT / 2.0))
                    .top(px(f32::from(position.y) + dy - HANDLE_HIT / 2.0))
                    .w(px(HANDLE_HIT))
                    .h(px(HANDLE_HIT))
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
                    .child(
                        div()
                            .absolute()
                            .left(px((HANDLE_HIT - REGION_HANDLE_SIZE) / 2.0))
                            .top(px((HANDLE_HIT - REGION_HANDLE_SIZE) / 2.0))
                            .w(px(REGION_HANDLE_SIZE))
                            .h(px(REGION_HANDLE_SIZE))
                            .rounded(px(1.5))
                            .bg(rgb(0xFFFFFF))
                            .border_1()
                            .border_color(rgba((self.accent_hex << 8) | 0xFF)),
                    )
                    .into_any_element(),
            );
        }
        handles
    }

    /// Transparent grab bands on the export frame edges and corners. They sit
    /// above the image layer so a frame edge stays draggable even when an
    /// annotation runs along it, and they make the 9px handles easier to grab.
    fn region_edge_bands(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.can_adjust_region() {
            return Vec::new();
        }
        let Some(rect) = self.inline_selection_rect(window) else {
            return Vec::new();
        };
        let left = f32::from(rect.origin.x);
        let top = f32::from(rect.origin.y);
        let width = f32::from(rect.size.width);
        let height = f32::from(rect.size.height);
        let half = BAND_HALF;
        let mut bands = Vec::new();
        {
            let mut band = |handle: RegionHandle, x: f32, y: f32, w: f32, h: f32| {
                bands.push(
                    div()
                        .id(format!("editor-region-band-{:?}", handle))
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w.max(1.0)))
                        .h(px(h.max(1.0)))
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
            };
            band(RegionHandle::North, left, top - half, width, half * 2.0);
            band(
                RegionHandle::South,
                left,
                top + height - half,
                width,
                half * 2.0,
            );
            band(RegionHandle::West, left - half, top, half * 2.0, height);
            band(
                RegionHandle::East,
                left + width - half,
                top,
                half * 2.0,
                height,
            );
            let corner = half * 2.0;
            band(
                RegionHandle::NorthWest,
                left - half,
                top - half,
                corner,
                corner,
            );
            band(
                RegionHandle::NorthEast,
                left + width - half,
                top - half,
                corner,
                corner,
            );
            band(
                RegionHandle::SouthWest,
                left - half,
                top + height - half,
                corner,
                corner,
            );
            band(
                RegionHandle::SouthEast,
                left + width - half,
                top + height - half,
                corner,
                corner,
            );
        }
        bands
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
        if self.full_canvas.is_some() {
            return;
        }
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
        if self.full_canvas.is_some() {
            return;
        }
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
        if self.full_canvas.is_some() {
            return;
        }
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
        if self.full_canvas.is_some() {
            return;
        }
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
        self.refit_zoomable_view();
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

    fn to_display_point(&self, position: Point<Pixels>, window: &Window) -> Option<PhysicalPoint> {
        let view = self.image_view_rect(window);
        let scale = f32::from(view.size.width) / self.image_size.width as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let x = ((f32::from(position.x) - f32::from(view.origin.x)) / scale).floor() as i32;
        let y = ((f32::from(position.y) - f32::from(view.origin.y)) / scale).floor() as i32;
        if x < 0 || y < 0 || x >= self.image_size.width as i32 || y >= self.image_size.height as i32
        {
            return None;
        }
        Some(PhysicalPoint::new(x, y))
    }

    /// Whether a left press should start panning a zoomable long-capture
    /// preview instead of drawing or selecting. Empty image area pans; an
    /// annotation body or its resize handle keeps its editing behavior.
    fn zoomable_pan_start(&self, position: Point<Pixels>, window: &Window) -> bool {
        let Some(inline) = self.inline else {
            return false;
        };
        if !inline.zoomable || self.tool != Tool::Select || self.text_edit.is_some() {
            return false;
        }
        let Some(point) = self.to_display_point(position, window) else {
            return false;
        };
        if self.document.hit_test(point).is_some() {
            return false;
        }
        let scale = self.image_scale(window);
        if let Some(annotation) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            && annotation_handle_at(&annotation.shape, point, scale).is_some()
        {
            return false;
        }
        true
    }

    fn to_image_point(&self, position: Point<Pixels>, window: &Window) -> Option<PhysicalPoint> {
        let point = self.to_display_point(position, window)?;
        if self
            .full_selection()
            .is_some_and(|selection| !selection.contains(point))
        {
            return None;
        }
        Some(point)
    }

    /// Export-frame edge under `point`. Full-canvas region editors keep the
    /// whole monitor on screen, so these handles must follow the selection
    /// rect instead of the monitor boundary.
    fn region_handle_at(&self, point: PhysicalPoint) -> Option<RegionHandle> {
        if !self.can_adjust_region() {
            return None;
        }
        let rect = self.full_selection().unwrap_or_else(|| {
            PhysicalRect::new(0, 0, self.image_size.width, self.image_size.height)
        });
        let scale = self
            .inline
            .map(|inline| {
                f32::from(inline.image_rect.size.width) / self.image_size.width.max(1) as f32
            })
            .unwrap_or(1.0)
            .max(0.01);
        edge_resize_handle(rect, point, (8.0 / scale).max(1.0))
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

    fn finish_polyline(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.polyline_draft.take() else {
            return;
        };
        let mut points = draft.points;
        if distance(*points.last().unwrap_or(&draft.current), draft.current) > 2.0 {
            points.push(draft.current);
        }
        if points.len() >= 2 {
            let style = self.effective_style();
            self.document.add(Shape::Polyline { points }, style);
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
        if self.resize_draft.is_none() {
            return;
        }
        self.update_resize(point);
        self.resize_draft = None;
        self.status = None;
        cx.notify();
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
        self.show_status(
            t(settings_state::get().language, "editor.annotation_copied").to_owned(),
            cx,
        );
        true
    }

    fn paste_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(annotation) = self.annotation_clipboard.clone() else {
            return false;
        };
        let id = self.document.paste(&annotation, 16, 16);
        self.tool = Tool::Select;
        self.select_annotation(Some(id));
        self.show_status(
            t(settings_state::get().language, "editor.annotation_pasted").to_owned(),
            cx,
        );
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
        self.show_status(
            t(settings_state::get().language, "editor.annotation_deleted").to_owned(),
            cx,
        );
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

    fn refresh_capture_action(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_capture(cx);
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

    fn toggle_effects_panel(&mut self, cx: &mut Context<Self>) {
        self.effects_open = !self.effects_open;
        self.more_open = false;
        self.hints_visible = false;
        cx.notify();
    }

    fn set_region_radius(&mut self, radius: u32, cx: &mut Context<Self>) {
        self.effects.corner_radius = radius;
        settings_state::update(|settings| {
            settings.region_corner_radius = radius;
        });
        cx.notify();
    }

    fn toggle_region_shadow(&mut self, cx: &mut Context<Self>) {
        self.effects.shadow = !self.effects.shadow;
        let shadow = self.effects.shadow;
        settings_state::update(|settings| settings.region_shadow = shadow);
        cx.notify();
    }

    fn dismiss_effects(&mut self, cx: &mut Context<Self>) {
        self.effects_open = false;
        cx.notify();
    }

    fn dismiss_hints(&mut self, cx: &mut Context<Self>) {
        self.hints_visible = false;
        cx.notify();
    }

    fn text_index_at(&self, window: &Window, position: Point<Pixels>) -> Option<usize> {
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

    fn text_line_edge(&self, start: bool, window: &Window) -> Option<usize> {
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

    /// Inserts clipboard text at the caret. Multi-line text keeps its line
    /// breaks so pasted notes match what the source application showed.
    fn paste_text(&mut self, cx: &mut Context<Self>) {
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

    #[allow(clippy::too_many_arguments)]
    fn label_option_chip<F>(
        &self,
        id: String,
        label: String,
        tooltip: String,
        selected: bool,
        width: f32,
        cx: &mut Context<Self>,
        apply: F,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Context<Self>) + 'static,
    {
        let accent = self.chrome.accent;
        let accent_tint = self.chrome.accent_tint;
        let hover = self.chrome.hover;
        let muted = self.chrome.muted;
        div()
            .id(id)
            .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
            .w(px(width))
            .h(px(22.0))
            .rounded(px(5.0))
            .flex()
            .items_center()
            .justify_center()
            .when(selected, |button| button.bg(accent_tint))
            .when(!selected, move |button| {
                button.hover(move |style| style.bg(hover))
            })
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(move |this, _, _, cx| apply(this, cx)))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if selected { accent } else { muted })
                    .child(label),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn toolbar_button(
        &self,
        id: &'static str,
        label: String,
        icon: &'static [u8],
        enabled: bool,
        color: Rgba,
        cx: &mut Context<Self>,
        action: fn(&mut Self, &mut Context<Self>),
    ) -> AnyElement {
        let hover = self.chrome.hover;
        if !enabled {
            return div()
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.35)
                .child(icons::icon(icon, 18.0, self.chrome.muted))
                .into_any_element();
        }
        div()
            .id(id)
            .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
            .child(icons::icon(icon, 18.0, color))
            .into_any_element()
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
                        Tool::Highlighter => {
                            this.style.highlighter_radius = this.highlighter_radius * this.ui_scale;
                        }
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
        let show_highlighter = config_tool == Some(Tool::Highlighter);
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
                sizes.push(self.label_option_chip(
                    format!("editor-font-{label}"),
                    label,
                    tooltip,
                    selected,
                    24.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_font_size(size);
                        this.status = None;
                        cx.notify();
                    },
                ));
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
                    let enabled = !this.text_background;
                    this.apply_text_background(enabled);
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
                    let enabled = !this.text_outline;
                    this.apply_text_outline(enabled);
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
                radii.push(self.label_option_chip(
                    format!("editor-corner-{label}"),
                    label,
                    tooltip,
                    selected,
                    22.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_corner_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
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

        if show_highlighter {
            let mut radii = Vec::new();
            for radius in CORNER_RADII {
                let applied = (radius * size_scale).round();
                let selected = (config_style.highlighter_radius - applied).abs() < 0.1;
                let label = format!("{radius:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.highlighter_radius"));
                radii.push(self.label_option_chip(
                    format!("editor-highlighter-radius-{label}"),
                    label,
                    tooltip,
                    selected,
                    22.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_highlighter_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::HIGHLIGHTER, 14.0, muted))
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
                sizes.push(self.label_option_chip(
                    format!("editor-number-{label}"),
                    label,
                    tooltip,
                    selected,
                    24.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_number_size(size);
                        this.status = None;
                        cx.notify();
                    },
                ));
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
                radii.push(self.label_option_chip(
                    format!("editor-blur-{label}"),
                    label,
                    tooltip,
                    selected,
                    24.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_blur_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
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
                levels.push(self.label_option_chip(
                    format!("editor-spotlight-{intensity}"),
                    label,
                    tooltip,
                    selected,
                    32.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_spotlight_intensity(intensity);
                        this.status = None;
                        cx.notify();
                    },
                ));
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
                radii.push(self.label_option_chip(
                    format!("editor-spotlight-radius-{label}"),
                    label,
                    tooltip,
                    selected,
                    22.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_spotlight_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
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
        let undo = self.toolbar_button(
            "editor-undo",
            undo_label,
            icons::UNDO,
            self.document.can_undo() || !self.canvas_undo.is_empty(),
            muted,
            cx,
            |this, cx| {
                if !this.document.undo() {
                    this.undo_canvas(cx);
                }
                this.status = None;
                cx.notify();
            },
        );

        let redo_label = t(language, "editor.redo").to_owned();
        let redo = self.toolbar_button(
            "editor-redo",
            redo_label,
            icons::REDO,
            self.document.can_redo() || !self.canvas_redo.is_empty(),
            muted,
            cx,
            |this, cx| {
                if !this.document.redo() {
                    this.redo_canvas(cx);
                }
                this.status = None;
                cx.notify();
            },
        );

        let delete_label = t(language, "editor.delete").to_owned();
        let delete = self.toolbar_button(
            "editor-delete",
            delete_label,
            icons::TRASH,
            self.document.selected().is_some(),
            text,
            cx,
            |this, cx| {
                this.commit_text(cx);
                this.delete_selected_annotation(cx);
            },
        );

        let pin_label = t(language, "editor.pin").to_owned();
        let pin = (self.pin_target.is_none()).then(|| {
            self.toolbar_button(
                "editor-pin",
                pin_label,
                icons::PIN,
                true,
                text,
                cx,
                |this, cx| this.pin(cx),
            )
        });

        let copy_label = t(language, "editor.copy").to_owned();
        let copy = self.toolbar_button(
            "editor-copy",
            copy_label,
            icons::COPY,
            true,
            text,
            cx,
            |this, cx| this.export(true, cx),
        );

        let save_label = t(language, "editor.save").to_owned();
        let save = self.toolbar_button(
            "editor-save",
            save_label,
            icons::SAVE,
            true,
            text,
            cx,
            |this, cx| this.export(false, cx),
        );

        let capture_actions_enabled = self.inline.and_then(|inline| inline.capture).is_some();
        let mut capture_actions: Vec<AnyElement> = Vec::new();
        if capture_actions_enabled {
            capture_actions.push(self.toolbar_button(
                "editor-scroll-capture",
                t(language, "menu.scroll_capture").to_owned(),
                icons::SCROLL,
                true,
                muted,
                cx,
                |this, cx| this.start_scroll_capture(cx),
            ));
            capture_actions.push(self.toolbar_button(
                "editor-record",
                t(language, "menu.record").to_owned(),
                icons::RECORDING,
                true,
                muted,
                cx,
                |this, cx| this.start_recording(cx),
            ));
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

    /// Effects applied to a pin created from this editor. Region pins always
    /// compose the shadow; copy/save still follow the configured effects.
    fn pin_effects(&self) -> RegionEffects {
        let mut effects = self.stored_region_effects();
        if self.region_pin() {
            effects = effects.with_shadow();
        }
        effects
    }

    /// Whether the pin being created or edited keeps the shadow regardless of
    /// the export setting.
    fn region_pin(&self) -> bool {
        self.pin_shadow_always
            || self
                .inline
                .and_then(|inline| inline.capture)
                .is_some_and(|capture| matches!(capture.target, CaptureTarget::Region(_)))
    }

    /// Long-capture results have no composed shadow and rely on the native
    /// window shadow. Region pins now compose their own, and window/pin
    /// editors already carry the captured edge shadow.
    fn pin_native_shadow(&self) -> bool {
        self.inline.is_some()
            && self.inline.and_then(|inline| inline.capture).is_none()
            && self.pin_target.is_none()
    }

    fn region_effects_scale(&self) -> f32 {
        self.inline
            .and_then(|inline| inline.capture)
            .map(|capture| capture.scale)
            .unwrap_or(self.pin_effects_scale)
            .max(0.1)
    }

    fn info_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let inline = self.inline?;
        let capture = inline.capture?;
        let rect = self.info_rect(window)?;
        let language = settings_state::get().language;
        let selection = self.live_selection_rect().unwrap_or(capture.region.rect);
        let absolute_x = capture.monitor_origin.x + selection.origin.x;
        let absolute_y = capture.monitor_origin.y + selection.origin.y;
        let width = selection.size.width;
        let height = selection.size.height;
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
            bar = bar.child(crate::capture_panels::effects_chip(
                "editor-info-effects",
                self.chrome,
                language,
                self.effects_open,
                self.effects.active(),
                cx,
                Self::toggle_effects_panel,
            ));
        }

        bar = bar.child(crate::capture_panels::refresh_button(
            "editor-info-refresh",
            self.chrome,
            language,
            cx,
            Self::refresh_capture_action,
        ));

        Some(bar.into_any_element())
    }

    fn effects_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.effects_open {
            return None;
        }
        let rect = self.effects_rect(window)?;
        let language = settings_state::get().language;
        Some(crate::capture_panels::effects_panel(
            "editor",
            self.chrome,
            language,
            rect,
            self.effects,
            4.0,
            cx,
            Self::set_region_radius,
            Self::toggle_region_shadow,
            Self::dismiss_effects,
        ))
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
        let rect = Bounds {
            origin: point(
                px((window_width - width) / 2.0),
                px((window_height - height) / 2.0),
            ),
            size: size(px(width), px(height)),
        };
        let pin_hotkey = crate::hotkey::display_hotkey(&settings_state::get().pin_hotkey);
        let mut rows: Vec<(String, String)> = vec![
            (
                t(language, "editor.hint_key_copy").to_owned(),
                t(language, "editor.hint_copy").to_owned(),
            ),
            (
                "Cmd/Ctrl+S".to_owned(),
                t(language, "editor.hint_save").to_owned(),
            ),
        ];
        if self.inline.and_then(|inline| inline.capture).is_some() {
            rows.push((pin_hotkey, t(language, "editor.hint_pin").to_owned()));
        }
        rows.push(if self.inline.is_some_and(|inline| inline.zoomable) {
            (
                "Space".to_owned(),
                t(language, "editor.hint_pan").to_owned(),
            )
        } else {
            (
                "Space".to_owned(),
                t(language, "editor.hint_space").to_owned(),
            )
        });
        if self.inline.and_then(|inline| inline.capture).is_some() {
            rows.push((
                "F5".to_owned(),
                t(language, "editor.hint_refresh").to_owned(),
            ));
        }
        rows.push((
            "Esc".to_owned(),
            t(language, "editor.hint_cancel").to_owned(),
        ));
        Some(crate::capture_panels::hints_panel(
            "editor",
            self.chrome,
            language,
            rect,
            rows,
            cx,
            Self::dismiss_hints,
        ))
    }

    fn refresh_full_canvas(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let monitor = capture.region.monitor;
        let region_target = matches!(capture.target, CaptureTarget::Region(_));
        let mut backend = core_capture::platform_backend();
        let captured = if settings_state::get().capture_cursor {
            backend.capture_with_cursor(CaptureTarget::Monitor(monitor))
        } else {
            backend.capture(CaptureTarget::Monitor(monitor))
        };
        let frame = match captured {
            Ok(frame) => frame,
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
        let new_size = PhysicalSize::new(frame.width(), frame.height());
        let size_changed = new_size != self.image_size;
        let image = image_util::render_image(frame);
        let old_image = std::mem::replace(&mut self.image, image);
        let old_blur_images =
            std::mem::replace(&mut self.blur_images, std::array::from_fn(|_| None));
        self.images_released = false;
        cx.defer(move |cx| {
            cx.drop_image(old_image, None);
            for image in old_blur_images.into_iter().flatten() {
                cx.drop_image(image, None);
            }
        });
        if size_changed {
            self.image_size = new_size;
            self.document = AnnotationDocument::new(new_size);
            self.canvas_undo.clear();
            self.canvas_redo.clear();
            self.text_edit = None;
            self.draft = None;
            self.move_draft = None;
            self.resize_draft = None;
            self.crop_mode = false;
            self.crop_draft = None;
            self.polyline_draft = None;
            self.region_drag = None;
            self.selection_hover = SelectionHover::None;
            self.document.select(None);
            if let Some(inline) = self.inline.as_mut()
                && let Some(capture) = inline.capture.as_mut()
            {
                if region_target {
                    let width = capture.region.rect.size.width.min(new_size.width);
                    let height = capture.region.rect.size.height.min(new_size.height);
                    let x = capture
                        .region
                        .rect
                        .origin
                        .x
                        .clamp(0, (new_size.width - width) as i32);
                    let y = capture
                        .region
                        .rect
                        .origin
                        .y
                        .clamp(0, (new_size.height - height) as i32);
                    capture.region = CaptureRegion::new(
                        capture.region.monitor,
                        PhysicalRect::new(x, y, width, height),
                    );
                } else {
                    capture.region = CaptureRegion::new(
                        capture.region.monitor,
                        PhysicalRect::new(0, 0, new_size.width, new_size.height),
                    );
                }
                capture.target = if region_target {
                    CaptureTarget::Region(capture.region)
                } else {
                    CaptureTarget::Monitor(monitor)
                };
            }
            if let Some(inline) = &self.inline
                && f32::from(inline.image_rect.size.width) > 0.0
            {
                self.ui_scale = (new_size.width as f32 / f32::from(inline.image_rect.size.width))
                    .clamp(0.25, 8.0);
            }
        }
        if let Some(selection) = self.full_selection()
            && let Some(cropped) = image_util::cropped_rgba(&self.image, selection)
        {
            self.frame = Arc::new(cropped);
        }
        self.show_status(
            t(settings_state::get().language, "editor.refreshed").to_owned(),
            cx,
        );
    }

    fn refresh_capture(&mut self, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            self.refresh_full_canvas(cx);
            return;
        }
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
        let image_size = self.image_size;
        if let Some(inline) = self.inline.as_mut() {
            let scale = capture.scale.max(0.1);
            let center_x = (capture.region.rect.origin.x as f32
                + capture.region.rect.size.width as f32 / 2.0)
                / scale;
            let center_y = (capture.region.rect.origin.y as f32
                + capture.region.rect.size.height as f32 / 2.0)
                / scale;
            let image_width = image_size.width as f32 / scale;
            let image_height = image_size.height as f32 / scale;
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
        let document = self.action_document();
        let first_frame = match rasterize(&document, &self.frame) {
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
        let selection_bounds = self.full_selection_screen_bounds();
        let pin_effects = self.pin_effects();
        let pin_shadow_always = self.region_pin();
        let effects_scale = self.region_effects_scale();
        let pin_padding = pin_effects.padding(effects_scale);
        let native_shadow = self.pin_native_shadow();
        let document = self.action_document();
        match rasterize(&document, &self.frame) {
            Ok(raw) => {
                let export = if self.region_effects_active() {
                    self.effects.apply(raw.clone(), effects_scale)
                } else {
                    raw.clone()
                };
                let output = if pin_effects.active() {
                    pin_effects.apply(raw, effects_scale)
                } else {
                    raw
                };
                let hash = crate::capture_history::hash_frame(&export);
                let record_history = self.pin_target.is_none();
                let pin_shadow = (record_history && self.region_pin() && !self.effects.shadow)
                    .then_some(crate::capture_history::HistoryPinShadow {
                        corner_radius: self.effects.corner_radius,
                        scale: effects_scale,
                    });
                let export_padding = if self.effects.shadow { 0 } else { pin_padding };
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
                });
                let monitor = inline_host
                    .and_then(|inline| inline.capture)
                    .map(|capture| capture.region.monitor);
                if inline_host.is_some() {
                    self.release_images(cx);
                }
                let handles = cx.windows();
                let edit_state = crate::pin::PinEditState::new(
                    self.frame.clone(),
                    document,
                    self.stored_region_effects(),
                    pin_shadow_always,
                    pin_padding,
                    effects_scale,
                );
                // Pin windows must not be created inside the toolbar's input
                // callback; close the editor first and open the pin next turn.
                cx.defer(move |cx| {
                    if inline_host.is_some() {
                        crate::windows::close_capture_auxiliary(cx, handles);
                    }
                    // Encoding blocks for a few milliseconds; run it after the
                    // editor closes instead of inside the toolbar callback.
                    if record_history && let Ok(bytes) = image_util::encode_png(&export) {
                        crate::capture_history::record_png_bytes(
                            &bytes,
                            hash,
                            pin_shadow,
                            Some(effects_scale),
                        );
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
                        crate::pin::set_export_padding(id, export_padding);
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
        let mut history_recorded = false;
        let document = self.action_document();
        match rasterize(&document, &self.frame) {
            Ok(raw) => {
                let pin_update = (!copy_only).then_some(self.pin_target).flatten();
                let scale = self.region_effects_scale();
                let pin_effects = self.pin_effects();
                let pin_padding = pin_effects.padding(scale);
                // Pin editors display region captures with a composed shadow
                // even when the export setting is off; copy/save keep following
                // the configured effects.
                let (output, pin_output) = if pin_update.is_some() {
                    let copy = if self.region_effects_active() {
                        self.effects.apply(raw.clone(), scale)
                    } else {
                        raw.clone()
                    };
                    let pin = if pin_effects.active() {
                        pin_effects.apply(raw, scale)
                    } else {
                        raw
                    };
                    (copy, Some(pin))
                } else {
                    let copy = if self.region_effects_active() {
                        self.effects.apply(raw, scale)
                    } else {
                        raw
                    };
                    (copy, None)
                };
                let hash = crate::capture_history::hash_frame(&output);
                let record_history = self.pin_target.is_none();
                let pin_shadow = (record_history && self.region_pin() && !self.effects.shadow)
                    .then_some(crate::capture_history::HistoryPinShadow {
                        corner_radius: self.effects.corner_radius,
                        scale,
                    });
                if copy_only || settings_state::get().copy_to_clipboard {
                    match image_util::encode_png(&output) {
                        Ok(bytes) => {
                            copy_ok = true;
                            if record_history {
                                history_recorded = crate::capture_history::record_png_bytes(
                                    &bytes,
                                    hash,
                                    pin_shadow,
                                    Some(scale),
                                );
                            }
                            image_util::copy_png_bytes(bytes, &output, cx);
                            messages.push(t(language, "editor.copied").to_owned());
                        }
                        Err(error) => {
                            messages.push(format!("{}{}", t(language, "editor.copy_failed"), error))
                        }
                    }
                }
                if !copy_only && self.pin_target.is_none() {
                    match crate::storage::save_png(&output) {
                        Ok(path) => {
                            save_ok = true;
                            if record_history && !history_recorded {
                                crate::capture_history::record_png_file(
                                    &path,
                                    hash,
                                    pin_shadow,
                                    Some(scale),
                                );
                            }
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
                        document,
                        self.stored_region_effects(),
                        self.region_pin(),
                        pin_padding,
                        scale,
                    );
                    let pin_image = pin_output.unwrap_or(output);
                    if crate::pin::replace_image_with_edit_state(pin_id, pin_image, edit_state, cx)
                    {
                        let export_padding = if self.effects.shadow { 0 } else { pin_padding };
                        crate::pin::set_export_padding(pin_id, export_padding);
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
            let layouts = layout_annotation_text(window, &edit.content, font_size, width);
            let boxes = layouts
                .as_ref()
                .map(|layouts| text_line_boxes(layouts, line_height))
                .unwrap_or_default();
            let caret = text_caret_position(layouts.as_deref(), &boxes, edit, width);
            let selection = edit.selection();
            let selection_boxes = layouts
                .as_ref()
                .map(|layouts| {
                    text_selection_boxes(
                        layouts,
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
                .line_height(px(line_height))
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
                SelectionHover::Region(handle) => region_handle_cursor(handle),
                // Any annotation under the pointer can be selected and moved,
                // even while a drawing tool is active.
                SelectionHover::Body => crate::ui::open_hand_cursor(),
                SelectionHover::None
                    if self.tool == Tool::Select
                        && self.inline.is_some_and(|inline| inline.zoomable) =>
                {
                    crate::ui::open_hand_cursor()
                }
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
        let full_frame = self.live_selection_rect();
        let show_full_frame = self
            .full_canvas
            .is_some_and(|full_canvas| full_canvas.adjustable);
        let selection_clip = show_full_frame
            .then_some(full_frame)
            .flatten()
            .map(|rect| Bounds {
                origin: point(
                    px(rect.origin.x as f32 * scale),
                    px(rect.origin.y as f32 * scale),
                ),
                size: size(
                    px(rect.size.width as f32 * scale),
                    px(rect.size.height as f32 * scale),
                ),
            });
        let image_radius = if effects_active && !show_full_frame {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let frame_radius = if effects_active {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let frame_overlay = selection_clip.map(|clip| {
            // Paint in window coordinates: the canvas is anchored to the image
            // layer, while the mask and its accent ring share one outline.
            let origin = image_rect.origin;
            let hole = Bounds {
                origin: point(
                    px(f32::from(clip.origin.x) + f32::from(origin.x)),
                    px(f32::from(clip.origin.y) + f32::from(origin.y)),
                ),
                size: clip.size,
            };
            let accent = rgba((accent_hex << 8) | 0xFF);
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    crate::ui::paint_rounded_mask(
                        window,
                        bounds,
                        hole,
                        frame_radius,
                        rgba(0x00000066),
                        Some((accent, 3.0)),
                    );
                },
            )
            .absolute()
            .inset_0()
            .size_full()
        });
        let annotation_content = div()
            .absolute()
            .left(px(
                selection_clip.map_or(0.0, |clip| -f32::from(clip.origin.x))
            ))
            .top(px(
                selection_clip.map_or(0.0, |clip| -f32::from(clip.origin.y))
            ))
            .w(image_rect.size.width)
            .h(image_rect.size.height)
            .children(object_layers)
            .children(preview_blur_layer)
            .child(overlay_canvas)
            .children(text_overlay);
        let annotation_layer = if let Some(clip) = selection_clip {
            div()
                .absolute()
                .left(clip.origin.x)
                .top(clip.origin.y)
                .w(clip.size.width)
                .h(clip.size.height)
                .overflow_hidden()
                .child(annotation_content)
        } else {
            annotation_content
        };
        let image_layer = div()
            .absolute()
            .left(image_rect.origin.x)
            .top(image_rect.origin.y)
            .w(image_rect.size.width)
            .h(image_rect.size.height)
            .cursor(image_cursor)
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
                    .rounded(px(image_radius))
                    .object_fit(ObjectFit::Fill),
            )
            .child(annotation_layer)
            .children(frame_overlay)
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
                // Long-capture preview: the wheel zooms directly, anchored at
                // the pointer. Panning is left-drag only, so Ctrl is optional.
                if zoomable {
                    let delta = if y.abs() >= x.abs() { y } else { x };
                    let steps = (delta / 50.0).clamp(-4.0, 4.0);
                    if steps != 0.0 {
                        this.zoom_at(1.15f32.powf(steps), event.position, window, cx);
                    }
                } else if event.modifiers.control || event.modifiers.platform {
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
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.right_click(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    // Clicking the image must keep keyboard focus in the
                    // editor, otherwise Delete/arrow shortcuts stop working
                    // after a toolbar interaction.
                    let focus = this.focus_handle.clone();
                    window.focus(&focus, cx);
                    if this.text_edit.is_none()
                        && (this.space_down || this.zoomable_pan_start(event.position, window))
                    {
                        this.pan_draft = Some(PanDraft {
                            start: event.position,
                            start_pan: this.view_pan,
                        });
                        this.status = None;
                        cx.notify();
                        return;
                    }
                    let Some(display_point) = this.to_display_point(event.position, window) else {
                        return;
                    };
                    // A previous gesture can outlive its mouse-up when the
                    // pointer leaves the window or the platform consumes the
                    // release. Every new press starts from a clean transform.
                    if this.region_drag.is_some() {
                        this.finish_region_drag(cx);
                    }
                    this.move_draft = None;
                    this.resize_draft = None;
                    if this
                        .full_selection()
                        .is_some_and(|selection| !selection.contains(display_point))
                    {
                        // Pressing just outside the export frame still grabs the
                        // nearest frame edge; the pointer may sit on the border
                        // itself while the selection is the real target.
                        if let Some(handle) = this.region_handle_at(display_point) {
                            this.begin_region_drag(handle, cx);
                            cx.stop_propagation();
                        }
                        return;
                    }
                    let point = display_point;
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
                        && let Some(handle) = this.region_handle_at(point)
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
                        let style = this.effective_style();
                        this.document.add(
                            Shape::Number {
                                bounds: PhysicalRect::new(left, top, side, side),
                                value,
                            },
                            style,
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
                // The export-frame grab bands sit above this layer; their drag
                // must keep following the pointer even though it moves over
                // the opaque image, which never reaches the editor root.
                if this.region_drag.is_some() {
                    this.update_region_drag(event.position, event.modifiers, cx);
                    return;
                }
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
                    if this.region_drag.is_some() {
                        this.finish_region_drag(cx);
                        cx.stop_propagation();
                        return;
                    }
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
                        let style = this.effective_style();
                        this.document.add(shape, style);
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
            }))
            // A release over a toolbar or panel never reaches this image
            // layer's mouse-up handler; finish the export-frame drag from the
            // outside path so the frame cannot stay stuck mid-resize.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    if this.region_drag.is_some() {
                        this.update_region_drag(event.position, event.modifiers, cx);
                        this.finish_region_drag(cx);
                    }
                }),
            );

        let region_bands = self.region_edge_bands(window, cx);
        let region_handles = self.region_handles(window, cx);
        let selection_layer = self.selection_layer(window);
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
            div()
                .absolute()
                .bottom(px(46.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(self.chrome.surface_strong)
                        .border_1()
                        .border_color(self.chrome.border)
                        .shadow(self.chrome.shadow())
                        .text_size(px(11.0))
                        .text_color(self.chrome.text)
                        .child(message),
                )
                .with_animation(
                    SharedString::from(format!("editor-status-{status_serial}")),
                    Animation::new(Duration::from_millis(120))
                        .with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |toast, delta| toast.opacity(delta).bottom(px(46.0 - (1.0 - delta) * 4.0)),
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
                            let style = this.effective_style();
                            this.document.add(shape, style);
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
                    this.right_click(cx);
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
                            if event.keystroke.modifiers.shift {
                                if let Some(edit) = &mut this.text_edit {
                                    edit.replace(None, "\n", false);
                                }
                                cx.notify();
                            } else {
                                this.commit_text(cx);
                            }
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
                    if command && key == "v" {
                        this.paste_text(cx);
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
                    if this.cancel_annotation_state(cx) {
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
            .children(region_bands)
            .children(region_handles)
            .children(selection_layer)
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
    // Pins with composed effects display the base inset by the shadow padding;
    // open the editor on that inner rect so the base is neither stretched nor
    // re-shadowed.
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

/// Opens a stitched long capture in the same in-place editor surface. The
/// tall image is initially fit to the capture monitor; wheel zoom/pan keeps
/// full-resolution detail reachable without a separate editor page.
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
    // Keep the result clear of the macOS menu bar (which can be up to 37 pt on
    // notched displays) and the window edges elsewhere, so a tall capture
    // never opens partially off-screen.
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
            if zoomable && let Some(inline) = editor.inline.as_mut() {
                inline.fit_area = Some(window_bounds.size);
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
    let mut cropped = AnnotationDocument::new(selection.size);
    for annotation in document.objects() {
        let translated = annotation
            .clone()
            .translated(-selection.origin.x, -selection.origin.y);
        cropped.add(translated.shape, translated.style);
    }
    cropped.select(None);
    cropped.clear_history();
    cropped
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

fn push_preview_rounded_rect(
    builder: &mut PathBuilder,
    to_point: &impl Fn(f32, f32) -> Point<Pixels>,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) {
    if radius < 0.5 {
        builder.move_to(to_point(left, top));
        builder.line_to(to_point(right, top));
        builder.line_to(to_point(right, bottom));
        builder.line_to(to_point(left, bottom));
        return;
    }
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
