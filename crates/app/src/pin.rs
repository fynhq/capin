use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::i18n::t;
use crate::icons;
use crate::image_util;
use crate::logging::{log_error, log_info, log_warn};
use crate::settings_state;
use crate::ui::centered_bounds;
use core_annotate::AnnotationDocument;
use core_types::{DpiScale, PhysicalRect, PhysicalSize, RgbaFrame};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{
    WindowStyle, apply as apply_window_style, disable_native_shadow, enable_native_shadow,
    hide_native_border, place_on_monitor_deferred, set_click_through, system_accent_hex,
};

mod gallery;
mod manifest;
mod menu;
mod pin_base;
mod pin_clipboard;
mod pin_content;
mod pin_display;
mod pin_help;
mod pin_ocr;
mod pin_qr;
mod pin_view;
mod toast;
mod window;

use toast::PinToast;

const NOTICE_TOAST: usize = 0;
const HUD_TOAST: usize = 1;

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) use gallery::pin_last;
pub(crate) use gallery::{open_frame_at_with_hash, open_frame_with_hash};
use gallery::{open_history_pin, open_text_pin};
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) use menu::is_pin_menu_open;
pub(crate) use menu::{close_pin_menu, open_pin_menu};
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) use window::{click_through_count, flush_manifest, release_click_through};
pub(crate) use window::{
    close_pin, is_pin_window, refresh_recognition, replace_image_with_edit_state, restore,
    set_edit_state, set_export_padding,
};
use window::{
    close_pin_inner, next_pin_id, persist_rotated_image, save_manifest, save_pin_image,
    schedule_manifest_save, set_state_click_through, update_bounds, update_bounds_without_save,
};

use manifest::PersistedPin;
use pin_base::PinBase;
use pin_display::{
    clamp_to_display, fit_pin_to_display, pin_display_bounds, pin_open_display_bounds,
    saved_display_id,
};

use pin_clipboard::{clipboard_image_signature, clipboard_text_signature, text_signature};
use pin_content::PinContent;

const MAX_RESTORED_PINS: usize = 20;
const MAX_RESTORE_ESTIMATED_BYTES: f64 = 96.0 * 1024.0 * 1024.0;

const MIN_PIN_ZOOM: f32 = 0.2;
const MIN_PIN_FIT_ZOOM: f32 = 0.02;
const MAX_PIN_ZOOM: f32 = 4.0;
const PIN_ZOOM_STEP: f32 = 0.05;

fn pin_window_kind() -> WindowKind {
    #[cfg(target_os = "macos")]
    {
        WindowKind::Floating
    }
    #[cfg(not(target_os = "macos"))]
    {
        WindowKind::PopUp
    }
}
const CARD_WIDTH: f32 = 380.0;
const PIN_MENU_WIDTH: f32 = 190.0;
const PIN_MENU_ITEM_HEIGHT: f32 = 30.0;
const PIN_MENU_PADDING: f32 = 5.0;
const PIN_MENU_SEPARATOR: f32 = 9.0;

thread_local! {
    static PIN_CASCADE: Cell<u32> = const { Cell::new(0) };
    static PINS: RefCell<HashMap<u64, PinState>> = RefCell::new(HashMap::new());
    static PIN_MENU: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
    static MANIFEST_REVISION: Cell<u64> = const { Cell::new(0) };
    static LAST_PIN_TRIGGER: Cell<Option<u64>> = const { Cell::new(None) };
    static PIN_CLIPBOARD: Cell<Option<u64>> = const { Cell::new(None) };
    static PIN_CLIPBOARD_TEXT: Cell<Option<u64>> = const { Cell::new(None) };
    static PIN_WALK_GENERATION: Cell<Option<u64>> = const { Cell::new(None) };
    static PIN_WALK_EXHAUSTED: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_PINS: RefCell<Vec<PersistedPin>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PinMenuAction {
    Rotate,
    Edit,
    Settings,
    Through,
    Close,
}

impl PinMenuAction {
    fn id(self) -> &'static str {
        match self {
            Self::Rotate => "pin-menu-rotate",
            Self::Edit => "pin-menu-edit",
            Self::Settings => "pin-menu-settings",
            Self::Through => "pin-menu-through",
            Self::Close => "pin-menu-close",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Rotate => "pin.rotate",
            Self::Edit => "pin.edit",
            Self::Settings => "pin.settings",
            Self::Through => "pin.through",
            Self::Close => "pin.close",
        }
    }

    fn separated(self) -> bool {
        matches!(self, Self::Settings | Self::Through | Self::Close)
    }
}

fn estimated_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|ch| {
            if ch.is_whitespace() {
                font_size * 0.35
            } else if ch.is_ascii() {
                font_size * 0.6
            } else {
                font_size
            }
        })
        .sum()
}

fn wrapped_line_count(line: &str, width: f32, font_size: f32) -> usize {
    if line.is_empty() {
        return 1;
    }
    let mut lines = 1usize;
    let mut current = 0.0f32;
    for ch in line.chars() {
        let advance = if ch.is_whitespace() {
            font_size * 0.35
        } else if ch.is_ascii() {
            font_size * 0.6
        } else {
            font_size
        };
        if current + advance > width && current > 0.0 {
            lines += 1;
            current = 0.0;
        }
        current += advance;
    }
    lines
}

fn text_pin_size(text: &str) -> (f32, f32) {
    const FONT: f32 = 14.0;
    const LINE_HEIGHT: f32 = 20.0;
    const PAD: f32 = 24.0;
    let widest = text
        .lines()
        .map(|line| estimated_text_width(line, FONT))
        .fold(0.0f32, f32::max);
    let width = (widest + PAD).clamp(140.0, CARD_WIDTH);
    let lines = text
        .lines()
        .map(|line| wrapped_line_count(line, width - PAD, FONT))
        .sum::<usize>()
        .max(1);
    let height = (lines as f32 * LINE_HEIGHT + PAD).clamp(44.0, 600.0);
    (width, height)
}

#[derive(Clone)]
enum PinKind {
    Image,
    Text(String),
}

#[derive(Clone)]
pub(crate) struct PinEditState {
    base: Arc<Mutex<PinBase>>,
    pub document: AnnotationDocument,
    pub effects: crate::effects::RegionEffects,
    pub shadow_always: bool,
    pub display_padding: u32,
    pub effects_scale: f32,
}

const PIN_BASE_KEEP_RAW_BYTES: usize = 8 * 1024 * 1024;

impl PinEditState {
    pub(crate) fn new(
        base: Arc<RgbaFrame>,
        document: AnnotationDocument,
        effects: crate::effects::RegionEffects,
        shadow_always: bool,
        display_padding: u32,
        effects_scale: f32,
    ) -> Self {
        let state = Arc::new(Mutex::new(PinBase::Frame(Arc::clone(&base))));
        let raw_bytes = base.width() as usize * base.height() as usize * 4;
        if raw_bytes > PIN_BASE_KEEP_RAW_BYTES {
            let shared = Arc::clone(&state);
            let size = PhysicalSize::new(base.width(), base.height());
            std::thread::Builder::new()
                .name("capin-pin-base".to_owned())
                .spawn(move || {
                    let Ok(bytes) = crate::image_util::encode_png(&base) else {
                        return;
                    };
                    if bytes.len() >= raw_bytes {
                        return;
                    }
                    if let Ok(mut state) = shared.lock() {
                        *state = PinBase::Encoded {
                            bytes: bytes.into(),
                            size,
                        };
                    }
                })
                .ok();
        }
        Self {
            base: state,
            document,
            effects,
            shadow_always,
            display_padding,
            effects_scale: effects_scale.max(0.1),
        }
    }

    pub(crate) fn base_frame(&self) -> Option<Arc<RgbaFrame>> {
        self.base.lock().ok()?.frame()
    }
}

#[derive(Clone, Copy)]
struct PinWindowState {
    zoom: f32,
    opacity: f32,
    click_through: bool,
}

type SharedPinWindowState = Rc<RefCell<PinWindowState>>;

struct PinState {
    kind: PinKind,
    entity: Entity<PinView>,
    window: AnyWindowHandle,
    image_path: Option<PathBuf>,
    content_hash: Option<u64>,
    bounds: Bounds<Pixels>,
    values: SharedPinWindowState,
    native_shadow: bool,
    display: Option<DisplayId>,
    edit_state: Option<PinEditState>,
    export_padding: u32,
}

#[allow(clippy::too_many_arguments)]
fn open_pin_inner(
    content: PinContent,
    id: Option<u64>,
    bounds: Bounds<Pixels>,
    zoom: f32,
    opacity: f32,
    click_through: bool,
    existing_image: Option<PathBuf>,
    persist: bool,
    native_shadow: bool,
    display_id: Option<DisplayId>,
    defer_ocr: bool,
    cx: &mut App,
) -> Option<u64> {
    let display_id = display_id.or_else(|| cx.primary_display().map(|display| display.id()));
    let display = pin_open_display_bounds(cx, bounds, display_id);
    let scale = pin_display::display_scale_for(display_id);
    let (bounds, zoom) = fit_pin_to_display(&content, bounds, zoom, display, scale);
    let values = Rc::new(RefCell::new(PinWindowState {
        zoom,
        opacity,
        click_through,
    }));
    let view_values = values.clone();
    let id = id.unwrap_or_else(next_pin_id);
    let kind = content.kind();
    let content_hash = content.hash();
    let had_existing_image = existing_image.is_some();
    let image_path = if let Some(path) = existing_image {
        Some(path)
    } else if let Some(frame) = content.frame() {
        save_pin_image(id, frame)
    } else {
        None
    };
    let initial_ocr_path = image_path.clone();
    let options = crate::aux_window::popup_options(
        "capin-pin",
        bounds,
        display_id,
        false,
        pin_window_kind(),
        WindowBackgroundAppearance::Transparent,
    );

    let content_for_view = content.clone();
    match gpui_kit::open_window(options, cx, move |window, cx| {
        if let Err(error) = apply_window_style(&*window, WindowStyle::PIN) {
            log_error!("pin", "failed to style pin window: {error}");
        }
        if native_shadow {
            enable_native_shadow(&*window);
        } else {
            disable_native_shadow(&*window);
        }
        hide_native_border(&*window);
        crate::ui::align_client_to_bounds(&*window, bounds);
        if click_through && let Err(error) = set_click_through(&*window, true) {
            crate::logging::log_warn!("pin", "failed to enable click-through: {error}");
        }
        let view = cx.new(|cx| {
            PinView::new(
                id,
                content_for_view,
                display_id,
                view_values,
                initial_ocr_path,
                defer_ocr,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            view.watch_close(window, cx);
            view.fit_display_to_size(
                window.scale_factor(),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                cx,
            );
        });
        window.defer(cx, move |window, _| {
            crate::ui::align_client_to_bounds(window, bounds)
        });
        view
    }) {
        Ok((window, entity)) => {
            log_info!(
                "pin",
                "opened pin={id} window={} at={:.0},{:.0} size={:.0}x{:.0} zoom={:.2}",
                window.window_id().as_u64(),
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                zoom
            );
            let view_for_release = entity.clone();
            let state = PinState {
                kind,
                entity,
                window,
                image_path,
                content_hash,
                bounds,
                values,
                native_shadow,
                display: display_id,
                edit_state: None,
                export_padding: 0,
            };
            PINS.with(|pins| {
                pins.borrow_mut().insert(id, state);
            });
            view_for_release.update(cx, |view, _| view.release_frame_if_persisted());
            if persist {
                refresh_clipboard_signatures(cx);
                save_manifest();
            }
            Some(id)
        }
        Err(error) => {
            log_error!("pin", "failed to open pin window: {error}");
            if !had_existing_image && let Some(path) = image_path {
                let _ = std::fs::remove_file(path);
            }
            None
        }
    }
}

pub(crate) fn refresh_clipboard_signatures(cx: &mut App) {
    PIN_CLIPBOARD.with(|slot| slot.set(clipboard_image_signature(cx)));
    PIN_CLIPBOARD_TEXT.with(|slot| slot.set(clipboard_text_signature(cx)));
}

struct PinTheme {
    card: Rgba,
    text: Rgba,
    border: Rgba,
    menu: Rgba,
    menu_hover: Rgba,
    menu_border: Rgba,
    accent: Rgba,
    selection: Rgba,
}

impl PinTheme {
    fn for_window(window: &Window, accent_hex: u32) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let palette = crate::theme::pin_palette(dark);
        Self {
            card: palette.card,
            text: palette.text,
            border: palette.border,
            menu: palette.menu,
            menu_hover: palette.menu_hover,
            menu_border: palette.menu_border,
            accent: rgb(accent_hex),
            selection: rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x66),
        }
    }
}

struct PinDrag {
    cursor_start: (i32, i32),
    window_start: (i32, i32),
}

struct PinView {
    id: u64,
    content: PinContent,
    display: Option<DisplayId>,
    accent_hex: u32,
    values: SharedPinWindowState,
    zoom_target: f32,
    zoom_serial: u64,
    zoom_animating: bool,
    zoom_center: Option<(f32, f32)>,
    zoom_hud_at: Option<Instant>,
    toasts: [PinToast; 2],
    ocr: Option<platform_ocr::OcrOutput>,
    ocr_path: Option<PathBuf>,
    ocr_deferred: bool,
    selection: Option<(usize, usize)>,
    selecting: bool,
    drag: Option<PinDrag>,
    hover_text: bool,
    hover_qr: bool,
    hover: bool,
    content_generation: u64,
    flash_alpha: f32,
    flash_active: bool,
    flash_generation: u64,
    focus_handle: FocusHandle,
}

impl PinView {
    fn new(
        id: u64,
        content: PinContent,
        display: Option<DisplayId>,
        values: SharedPinWindowState,
        ocr_path: Option<PathBuf>,
        defer_ocr: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let zoom = values.borrow().zoom;
        let accent_hex =
            system_accent_hex().unwrap_or_else(|| crate::theme::fallback_accent(false));
        let mut view = Self {
            id,
            content,
            display,
            accent_hex,
            values,
            zoom_target: zoom,
            zoom_serial: 0,
            zoom_animating: false,
            zoom_center: None,
            zoom_hud_at: None,
            ocr_path,
            ocr_deferred: defer_ocr,
            toasts: [
                PinToast::new(Duration::from_millis(2200)),
                PinToast::new(Duration::from_millis(900)),
            ],
            ocr: None,
            selection: None,
            selecting: false,
            drag: None,
            hover_text: false,
            hover_qr: false,
            hover: false,
            content_generation: 1,
            flash_alpha: 0.0,
            flash_active: false,
            flash_generation: 0,
            focus_handle: cx.focus_handle(),
        };
        cx.on_release(|this, cx| {
            if let PinContent::Image { image, .. } = &this.content {
                cx.drop_image(image.clone(), None);
            }
        })
        .detach();
        if !defer_ocr {
            view.start_ocr(cx);
        }
        view
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn flash(&mut self, cx: &mut Context<Self>) {
        self.flash_generation = self.flash_generation.wrapping_add(1);
        self.flash_active = true;
        self.flash_alpha = 1.0;
        let generation = self.flash_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(520))
                .await;
            this.update(cx, |this, cx| {
                if this.flash_generation == generation {
                    this.flash_active = false;
                    this.flash_alpha = 0.0;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn persisted_path(&self) -> Option<PathBuf> {
        PINS.with(|pins| {
            pins.borrow()
                .get(&self.id)
                .and_then(|state| state.image_path.clone())
        })
    }

    fn frame_for_action(&self) -> Option<Arc<RgbaFrame>> {
        self.content.frame_arc().or_else(|| {
            self.persisted_path()
                .and_then(|path| image_util::load_frame(&path))
                .map(Arc::new)
        })
    }

    fn qr(&self) -> Option<&platform_ocr::OcrQrCode> {
        if !settings_state::get().pin_qr {
            return None;
        }
        self.ocr.as_ref().and_then(|output| output.qr_codes.first())
    }

    fn qr_code(&self) -> Option<&str> {
        self.qr().map(|qr| qr.content.as_str())
    }

    fn image_layout(&self, zoom: f32, viewport: Size<Pixels>) -> (f32, f32, f32) {
        let (width, height) = self.content.size(zoom);
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let fit = if viewport_width <= 0.0 || viewport_height <= 0.0 {
            1.0
        } else {
            (viewport_width / width.max(1.0))
                .min(viewport_height / height.max(1.0))
                .min(1.0)
        };
        (
            fit,
            (viewport_width - width * fit) / 2.0,
            (viewport_height - height * fit) / 2.0,
        )
    }

    fn qr_display_rect(&self, viewport: Size<Pixels>) -> Option<(f32, f32, f32, f32)> {
        let rect = self.qr()?.rect;
        let zoom = self.values.borrow().zoom;
        let (fit, offset_x, offset_y) = self.image_layout(zoom, viewport);
        let scale = zoom * fit;
        Some((
            offset_x + rect.origin.x as f32 * scale,
            offset_y + rect.origin.y as f32 * scale,
            rect.size.width as f32 * scale,
            rect.size.height as f32 * scale,
        ))
    }

    fn qr_hit(&self, position: Point<Pixels>, viewport: Size<Pixels>) -> bool {
        let Some((left, top, width, height)) = self.qr_display_rect(viewport) else {
            return false;
        };
        let x = f32::from(position.x);
        let y = f32::from(position.y);
        x >= left && y >= top && x <= left + width && y <= top + height
    }

    fn text_lines(&self) -> Option<&[platform_ocr::OcrLine]> {
        self.ocr
            .as_ref()
            .filter(|output| output.error.is_none())
            .map(|output| output.lines.as_slice())
            .filter(|lines| !lines.is_empty())
    }

    fn text_index_at(&self, position: Point<Pixels>) -> Option<usize> {
        let lines = self.text_lines()?;
        let zoom = self.values.borrow().zoom;
        let x = f32::from(position.x) / zoom;
        let y = f32::from(position.y) / zoom;
        let mut base = 0usize;
        let mut best: Option<(f32, usize)> = None;
        for line in lines {
            let top = line.rect.origin.y as f32;
            let bottom = top + line.rect.size.height as f32;
            if y < top - 2.0 || y > bottom + 2.0 {
                base += line.chars.len();
                continue;
            }
            let line_left = line
                .chars
                .first()
                .map(|ch| ch.rect.origin.x as f32)
                .unwrap_or(0.0);
            let line_right = line
                .chars
                .last()
                .map(|ch| (ch.rect.origin.x + ch.rect.size.width as i32) as f32)
                .unwrap_or(line_left);
            if x < line_left - 3.0 || x > line_right + 3.0 {
                base += line.chars.len();
                continue;
            }
            for (index, ch) in line.chars.iter().enumerate() {
                let left = ch.rect.origin.x as f32;
                let right = left + ch.rect.size.width as f32;
                let horizontal = if x < left {
                    left - x
                } else if x > right {
                    x - right
                } else {
                    0.0
                };
                if best.is_none_or(|(best_distance, _)| horizontal < best_distance) {
                    best = Some((horizontal, base + index));
                }
            }
            base += line.chars.len();
        }
        best.map(|(_, index)| index)
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let (anchor, focus) = self.selection?;
        Some((anchor.min(focus), anchor.max(focus)))
    }

    fn selected_text(&self) -> Option<String> {
        let lines = self.text_lines()?;
        let (start, end) = self.selection_range()?;
        let block_right = lines
            .iter()
            .map(|line| line.rect.origin.x + line.rect.size.width as i32)
            .max()
            .unwrap_or(0);
        type PreviousLine<'a> = (&'a platform_ocr::OcrLine, bool, Option<(u16, u16)>);
        let mut text = String::new();
        let mut base = 0usize;
        let table = self.ocr.as_ref().and_then(|output| output.table.as_ref());
        let mut previous: Option<PreviousLine<'_>> = None;
        for (index, line) in lines.iter().enumerate() {
            let length = line.chars.len();
            if end < base || start >= base + length {
                base += length;
                continue;
            }
            let from = start.saturating_sub(base).min(length);
            let to = (end.saturating_sub(base) + 1).min(length);
            let full_line = to == length;
            let cell = table.and_then(|table| table.line_cells.get(index).copied().flatten());
            let separator = match previous {
                Some((previous_line, true, previous_cell)) if from == 0 => {
                    match (previous_cell, cell) {
                        (Some((previous_row, previous_column)), Some((row, column)))
                            if previous_row == row && previous_column != column =>
                        {
                            "\t"
                        }
                        (Some((previous_row, _)), Some((row, _))) if row > previous_row => "\n",
                        (Some(_), Some(_)) => " ",
                        _ => platform_ocr::line_join_separator(previous_line, line, block_right),
                    }
                }
                Some(_) => "\n",
                None => "",
            };
            text.push_str(separator);
            if let Some(chars) = line.chars.get(from..to) {
                for ch in chars {
                    text.push(ch.ch);
                }
            }
            previous = Some((line, full_line, cell));
            base += length;
        }
        Some(text.trim_end().to_owned())
    }

    fn selection_rects(&self) -> Vec<PhysicalRect> {
        let Some(lines) = self.text_lines() else {
            return Vec::new();
        };
        let Some((start, end)) = self.selection_range() else {
            return Vec::new();
        };
        let mut rects = Vec::new();
        let mut base = 0usize;
        for line in lines {
            let length = line.chars.len();
            if end < base || start >= base + length {
                base += length;
                continue;
            }
            let from = start.saturating_sub(base).min(length);
            let to = (end.saturating_sub(base) + 1).min(length);
            if from < to
                && let (Some(first), Some(last)) = (line.chars.get(from), line.chars.get(to - 1))
            {
                let left = first.rect.origin.x as f32;
                let right = (last.rect.origin.x + last.rect.size.width as i32) as f32;
                rects.push(PhysicalRect::new(
                    left as i32,
                    line.rect.origin.y,
                    (right - left).max(1.0) as u32,
                    line.rect.size.height,
                ));
            }
            base += length;
        }
        rects
    }

    fn copy_selected_text(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.selected_text() else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let language = settings_state::get().language;
        self.show_notice(t(language, "pin.text_copied").to_owned(), cx);
    }

    fn begin_text_selection(&mut self, position: Point<Pixels>) -> bool {
        let Some(index) = self.text_index_at(position) else {
            self.selection = None;
            return false;
        };
        self.selection = Some((index, index));
        self.selecting = true;
        true
    }

    fn update_text_selection(&mut self, position: Point<Pixels>) {
        let Some((anchor, _)) = self.selection else {
            return;
        };
        if let Some(index) = self.text_index_at(position) {
            self.selection = Some((anchor, index));
        }
    }

    fn finish_text_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        let Some((start, end)) = self.selection_range() else {
            return;
        };
        if start == end {
            self.selection = None;
        }
        cx.notify();
    }

    fn select_all_text(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(total) = self
            .text_lines()
            .map(|lines| lines.iter().map(|line| line.chars.len()).sum::<usize>())
        else {
            return false;
        };
        if total == 0 {
            return false;
        }
        self.selection = Some((0, total - 1));
        self.selecting = false;
        cx.notify();
        true
    }

    fn apply_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.image_size().is_none() {
            return;
        }
        let fit = self.fitted_zoom_limit(window, cx);
        let zoom = zoom.clamp(pin_display::pin_zoom_floor(fit, window.scale_factor()), fit);
        let (width, height) = self.content.size(zoom);
        let bounds = window.bounds();
        let (center_x, center_y) = self.zoom_center.unwrap_or_else(|| {
            let stored = PINS.with(|pins| {
                pins.borrow().get(&self.id).map(|state| {
                    (
                        f32::from(state.bounds.origin.x) + f32::from(state.bounds.size.width) / 2.0,
                        f32::from(state.bounds.origin.y)
                            + f32::from(state.bounds.size.height) / 2.0,
                    )
                })
            });
            stored.unwrap_or((
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
                f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
            ))
        });
        let mut origin_x = center_x - width / 2.0;
        let mut origin_y = center_y - height / 2.0;
        if let Some(screen) = pin_display_bounds(window, cx) {
            let screen_left = f32::from(screen.origin.x);
            let screen_top = f32::from(screen.origin.y);
            let screen_width = f32::from(screen.size.width);
            let screen_height = f32::from(screen.size.height);
            if width <= screen_width {
                origin_x = origin_x.clamp(screen_left, screen_left + screen_width - width);
            }
            if height <= screen_height {
                origin_y = origin_y.clamp(screen_top, screen_top + screen_height - height);
            }
        }
        let origin = point(px(origin_x), px(origin_y));
        let scale = window.scale_factor().max(0.1);
        let dpi = DpiScale::new(scale).unwrap_or_default();
        let physical = PhysicalRect::new(
            (f32::from(origin.x) * scale).round() as i32,
            (f32::from(origin.y) * scale).round() as i32,
            (width * scale).ceil().max(1.0) as u32,
            (height * scale).ceil().max(1.0) as u32,
        );
        self.values.borrow_mut().zoom = zoom;
        let settle_bounds = Bounds {
            origin,
            size: size(px(width), px(height)),
        };
        if place_on_monitor_deferred(&*window, physical, dpi).is_err() {
            window.resize(size(px(width), px(height)));
        }
        window.defer(cx, move |window, cx| {
            crate::ui::align_client_to_bounds(window, settle_bounds);
            window.bounds_changed(cx);
            window.refresh();
        });
        #[cfg(target_os = "windows")]
        {
            let handle = window.window_handle();
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(32))
                    .await;
                let _ = handle.update(cx, |_, window, _| window.refresh());
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
        let display = window.display(cx).map(|display| display.id());
        self.display = display;
        update_bounds_without_save(self.id, settle_bounds, display);
        cx.notify();
    }

    fn step_zoom(&mut self, serial: u64, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if serial != self.zoom_serial {
            return true;
        }
        let fit = self.fitted_zoom_limit(window, cx);
        let scale = window.scale_factor();
        self.zoom_target = self
            .zoom_target
            .clamp(pin_display::pin_zoom_floor(fit, scale), fit);
        let difference = self.zoom_target - self.values.borrow().zoom;
        if difference.abs() <= 0.001 {
            if (self.zoom_target - self.values.borrow().zoom).abs() > f32::EPSILON {
                self.apply_zoom(self.zoom_target, window, cx);
            }
            self.ensure_display_resolution(window);
            self.zoom_center = None;
            self.zoom_animating = false;
            self.zoom_target = self.values.borrow().zoom;
            schedule_manifest_save(cx);
            let percent = pin_display::zoom_percent(self.zoom_target, scale);
            self.show_zoom_hud(percent, cx, true);
            return true;
        }
        let next = self.values.borrow().zoom + difference * 0.5;
        self.apply_zoom(next, window, cx);
        false
    }

    fn zoom_toward(&mut self, target: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.image_size().is_none() {
            return;
        }
        let fit = self.fitted_zoom_limit(window, cx);
        let scale = window.scale_factor();
        self.zoom_target = target.clamp(pin_display::pin_zoom_floor(fit, scale), fit);
        let percent = pin_display::zoom_percent(self.zoom_target, scale);
        if cx.reduce_motion() || cfg!(target_os = "windows") {
            self.zoom_serial = self.zoom_serial.wrapping_add(1);
            self.zoom_animating = false;
            self.zoom_center = None;
            let target = self.zoom_target;
            self.apply_zoom(target, window, cx);
            self.zoom_target = self.values.borrow().zoom;
            self.ensure_display_resolution(window);
            schedule_manifest_save(cx);
            let percent =
                pin_display::zoom_percent(self.values.borrow().zoom, window.scale_factor());
            self.show_zoom_hud(percent, cx, true);
            return;
        }
        self.show_zoom_hud(percent, cx, false);
        if self.zoom_center.is_none() {
            let bounds = window.bounds();
            self.zoom_center = Some((
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
                f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
            ));
        }
        if self.zoom_animating {
            return;
        }
        self.zoom_animating = true;
        self.zoom_serial = self.zoom_serial.wrapping_add(1);
        let serial = self.zoom_serial;
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let done = handle
                    .update(cx, |_, window, cx| {
                        this.update(cx, |this, cx| this.step_zoom(serial, window, cx))
                            .unwrap_or(true)
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    fn set_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        self.values.borrow_mut().opacity = opacity.clamp(0.2, 1.0);
        schedule_manifest_save(cx);
        let language = settings_state::get().language;
        let percent = (self.values.borrow().opacity * 100.0).round() as i32;
        self.show_hud(format!("{} {percent}%", t(language, "pin.opacity_hud")), cx);
        cx.notify();
    }

    fn begin_window_drag(&mut self, window: &mut Window) {
        #[cfg(target_os = "windows")]
        {
            let cursor = platform_window::cursor_screen_position();
            let origin = platform_window::window_screen_origin(&*window);
            if let (Some(cursor), Some(origin)) = (cursor, origin) {
                self.drag = Some(PinDrag {
                    cursor_start: cursor,
                    window_start: origin,
                });
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            window.start_window_move();
        }
    }

    fn update_window_drag(&mut self, window: &mut Window) {
        let Some(drag) = &self.drag else {
            return;
        };
        let Some((cursor_x, cursor_y)) = platform_window::cursor_screen_position() else {
            return;
        };
        let x = drag.window_start.0 + cursor_x - drag.cursor_start.0;
        let y = drag.window_start.1 + cursor_y - drag.cursor_start.1;
        let _ = platform_window::move_window_to(&*window, x, y);
    }

    fn watch_close(&self, window: &Window, cx: &mut Context<Self>) {
        let id = self.id;
        window.on_window_should_close(cx, move |_, cx| {
            log_info!("pin", "pin window close requested pin={id}");
            close_pin_inner(id, cx, false);
            true
        });
    }

    fn replace_image(&mut self, frame: Arc<RgbaFrame>, hash: u64, cx: &mut Context<Self>) {
        PINS.with(|pins| {
            if let Some(state) = pins.borrow_mut().get_mut(&self.id) {
                state.edit_state = None;
            }
        });
        let previous = std::mem::replace(
            &mut self.content,
            PinContent::from_frame_shared(frame, hash),
        );
        self.zoom_target = self.values.borrow().zoom;
        self.zoom_center = None;
        self.zoom_animating = false;
        self.zoom_serial = self.zoom_serial.wrapping_add(1);
        if let PinContent::Image { image, .. } = previous {
            cx.defer(move |cx| cx.drop_image(image, None));
        }
        self.content_generation = self.content_generation.wrapping_add(1);
        self.ocr = None;
        self.ocr_deferred = false;
        self.selection = None;
        self.selecting = false;
        let window_handle = PINS.with(|pins| pins.borrow().get(&self.id).map(|state| state.window));
        if let Some(handle) = window_handle
            && let Ok(scale) = handle.update(cx, |_, window, _| window.scale_factor())
        {
            let (width, height) = self.content.size(self.values.borrow().zoom);
            self.fit_display_to_size(scale, width, height, cx);
        }
        self.start_ocr(cx);
        cx.notify();
    }

    fn show_toast(&mut self, slot: usize, message: String, cx: &mut Context<Self>) {
        let toast = &mut self.toasts[slot];
        toast.message = Some(message);
        toast.leaving = false;
        toast.serial = toast.serial.wrapping_add(1);
        let serial = toast.serial;
        let linger = toast.linger;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(linger).await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.toasts[slot].serial != serial {
                        return false;
                    }
                    this.toasts[slot].leaving = true;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !alive {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            this.update(cx, |this, cx| {
                if this.toasts[slot].serial == serial {
                    this.toasts[slot].message = None;
                    this.toasts[slot].leaving = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn show_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.show_toast(NOTICE_TOAST, message, cx);
    }

    fn show_hud(&mut self, message: String, cx: &mut Context<Self>) {
        self.show_toast(HUD_TOAST, message, cx);
    }

    fn show_zoom_hud(&mut self, percent: i32, cx: &mut Context<Self>, force: bool) {
        let language = settings_state::get().language;
        let message = format!("{} {percent}%", t(language, "pin.zoom_hud"));
        let toast = &self.toasts[HUD_TOAST];
        if !toast.leaving && toast.message.as_deref() == Some(message.as_str()) {
            return;
        }
        let now = Instant::now();
        if !force
            && self
                .zoom_hud_at
                .is_some_and(|at| now.duration_since(at) < Duration::from_millis(50))
        {
            return;
        }
        self.zoom_hud_at = Some(now);
        self.show_hud(message, cx);
    }

    fn copy_content(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.content.clipboard_text() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let language = settings_state::get().language;
        self.show_notice(t(language, "pin.copied").to_owned(), cx);
    }

    fn copy_qr(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.qr_code().map(str::to_owned) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(code));
        let language = settings_state::get().language;
        self.show_notice(t(language, "pin.copied").to_owned(), cx);
    }

    fn export_frame(&self) -> Option<Arc<RgbaFrame>> {
        let edit_state = PINS.with(|pins| {
            pins.borrow()
                .get(&self.id)
                .and_then(|state| state.edit_state.clone())
        });
        if let Some(state) = edit_state
            && let Some(base) = state.base_frame()
            && let Ok(raw) = core_annotate::rasterize(&state.document, &base)
        {
            let frame = if state.effects.active() {
                state.effects.apply(raw, state.effects_scale)
            } else {
                raw
            };
            return Some(Arc::new(frame));
        }
        let frame = self.frame_for_action()?;
        let padding = self.export_padding();
        if padding > 0 {
            let width = frame.width().saturating_sub(padding.saturating_mul(2));
            let height = frame.height().saturating_sub(padding.saturating_mul(2));
            if width > 0 && height > 0 {
                let inner = PhysicalRect::new(padding as i32, padding as i32, width, height);
                return frame.cropped(inner).map(Arc::new);
            }
        }
        Some(frame)
    }

    fn export_padding(&self) -> u32 {
        PINS.with(|pins| {
            pins.borrow()
                .get(&self.id)
                .map_or(0, |state| state.export_padding)
        })
    }

    fn copy_image(&mut self, cx: &mut Context<Self>) {
        let language = settings_state::get().language;
        let Some(frame) = self.export_frame() else {
            return;
        };
        if let Ok(bytes) = image_util::encode_png(&frame) {
            image_util::copy_png_bytes(bytes, &frame, cx);
            self.show_notice(t(language, "pin.copied").to_owned(), cx);
        }
    }

    fn save_image(&mut self, cx: &mut Context<Self>) {
        let language = settings_state::get().language;
        let Some(frame) = self.export_frame() else {
            return;
        };
        match crate::storage::save_png(&frame) {
            Ok(path) => {
                self.show_notice(
                    format!("{} {}", t(language, "pin.saved"), path.display()),
                    cx,
                );
            }
            Err(error) => {
                self.show_notice(format!("{}{}", t(language, "pin.save_failed"), error), cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::manifest::{self, PersistedPin};
    use super::{PinBase, PinEditState, text_pin_size};
    use crate::effects::RegionEffects;
    use core_annotate::AnnotationDocument;
    use core_types::{PhysicalSize, RgbaFrame};
    use std::sync::Arc;

    fn persisted(id: u64) -> PersistedPin {
        PersistedPin {
            id,
            kind: "text".to_owned(),
            image: None,
            text: Some(format!("pin-{id}")),
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 80.0,
            opacity: 1.0,
            zoom: 1.0,
            click_through: false,
            shadow: false,
            export_padding: 0,
            display: None,
        }
    }

    #[test]
    fn restore_prefers_the_newest_pins() {
        let mut entries = vec![persisted(1), persisted(30), persisted(12), persisted(25)];
        manifest::sort_restore_entries(&mut entries);
        let ids: Vec<u64> = entries.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, vec![30, 25, 12, 1]);
    }

    #[test]
    fn zoom_limit_keeps_the_pin_inside_the_display() {
        assert!(
            (super::pin_display::fit_zoom_limit(1000, 500, 1200.0, 800.0, 1.0) - 1.2).abs() < 0.001
        );
        assert!(
            (super::pin_display::fit_zoom_limit(200, 100, 1920.0, 1080.0, 1.0)
                - super::MAX_PIN_ZOOM)
                .abs()
                < 0.001
        );
        assert!(
            (super::pin_display::fit_zoom_limit(4000, 3000, 1920.0, 1080.0, 1.0) - 0.36).abs()
                < 0.001
        );
        assert!(
            (super::pin_display::fit_zoom_limit(3000, 12000, 1920.0, 1080.0, 1.0) - 0.09).abs()
                < 0.001
        );
    }

    #[test]
    fn fit_limit_is_normalized_to_physical_pixels() {
        assert!(
            (super::pin_display::fit_zoom_limit(1920, 1200, 1536.0, 960.0, 1.25) - 0.8).abs()
                < 0.001
        );
        assert!(
            (super::pin_display::fit_zoom_limit(200, 100, 1536.0, 960.0, 1.25) - 3.2).abs() < 0.001
        );
    }

    #[test]
    fn zoom_percent_matches_physical_pixels() {
        assert_eq!(super::pin_display::zoom_percent(0.8, 1.25), 100);
        assert_eq!(super::pin_display::zoom_percent(0.8, 1.0), 80);
        assert_eq!(super::pin_display::zoom_percent(0.16, 1.25), 20);
    }

    #[test]
    fn user_zoom_stops_at_the_floor_unless_the_fit_is_lower() {
        assert!((super::pin_display::pin_zoom_floor(0.5, 1.0) - super::MIN_PIN_ZOOM).abs() < 0.001);
        assert!((super::pin_display::pin_zoom_floor(0.09, 1.0) - 0.09).abs() < 0.001);
        assert!((super::pin_display::pin_zoom_floor(0.5, 1.25) - 0.16).abs() < 0.001);
        assert!((super::pin_display::pin_zoom_floor(0.1, 1.25) - 0.1).abs() < 0.001);
    }

    #[test]
    fn raw_zoom_limits_follow_the_display_scale() {
        assert!((super::pin_display::min_raw_zoom(1.25) - 0.16).abs() < 0.001);
        assert!((super::pin_display::max_raw_zoom(1.25) - 3.2).abs() < 0.001);
    }

    #[test]
    fn available_saved_display_is_kept() {
        let primary = super::DisplayId::new(1);
        let secondary = super::DisplayId::new(2);
        let displays = [primary, secondary];
        assert_eq!(
            super::pin_display::resolve_display_id(Some(2), &displays, Some(primary))
                .map(u64::from),
            Some(2)
        );
        assert_eq!(
            super::pin_display::resolve_display_id(Some(9), &displays, Some(primary))
                .map(u64::from),
            Some(1)
        );
        assert_eq!(
            super::pin_display::resolve_display_id(None, &displays, Some(primary)).map(u64::from),
            Some(1)
        );
    }

    #[test]
    fn clamping_keeps_the_pin_inside_the_saved_display() {
        use gpui_kit::{Bounds, point, px, size};

        let secondary = Bounds {
            origin: point(px(1920.0), px(0.0)),
            size: size(px(1920.0), px(1080.0)),
        };
        let pin = Bounds {
            origin: point(px(1900.0), px(1000.0)),
            size: size(px(100.0), px(100.0)),
        };
        let clamped = super::pin_display::clamp_bounds_to_display(pin, Some(secondary));
        assert_eq!(f32::from(clamped.origin.x), 1920.0);
        assert_eq!(f32::from(clamped.origin.y), 980.0);
        assert_eq!(f32::from(clamped.size.width), 100.0);
        assert_eq!(f32::from(clamped.size.height), 100.0);

        let unclamped = super::pin_display::clamp_bounds_to_display(pin, None);
        assert_eq!(f32::from(unclamped.origin.x), 1900.0);
        assert_eq!(f32::from(unclamped.origin.y), 1000.0);
    }

    #[test]
    fn opening_pins_fit_the_image_inside_the_display() {
        let frame = RgbaFrame::from_pixels(400, 300, vec![0; 400 * 300 * 4]).unwrap();
        let content = super::PinContent::from_frame(frame);
        let display = gpui_kit::Bounds {
            origin: gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(0.0)),
            size: gpui_kit::size(gpui_kit::px(200.0), gpui_kit::px(200.0)),
        };
        let bounds = gpui_kit::Bounds {
            origin: gpui_kit::point(gpui_kit::px(-100.0), gpui_kit::px(-100.0)),
            size: gpui_kit::size(gpui_kit::px(400.0), gpui_kit::px(300.0)),
        };
        let (fitted, zoom) = super::fit_pin_to_display(&content, bounds, 1.0, Some(display), 1.0);
        assert!((zoom - 0.5).abs() < 0.001, "zoom was {zoom}");
        assert_eq!(f32::from(fitted.size.width), 200.0);
        assert_eq!(f32::from(fitted.size.height), 150.0);
        assert_eq!(f32::from(fitted.origin.x), 0.0);
        assert_eq!(f32::from(fitted.origin.y), 0.0);
    }

    #[test]
    fn opening_pins_clamp_origins_into_the_display() {
        let frame = RgbaFrame::from_pixels(100, 100, vec![0; 100 * 100 * 4]).unwrap();
        let content = super::PinContent::from_frame(frame);
        let display = gpui_kit::Bounds {
            origin: gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(0.0)),
            size: gpui_kit::size(gpui_kit::px(1920.0), gpui_kit::px(1080.0)),
        };
        let bounds = gpui_kit::Bounds {
            origin: gpui_kit::point(gpui_kit::px(1900.0), gpui_kit::px(1000.0)),
            size: gpui_kit::size(gpui_kit::px(100.0), gpui_kit::px(100.0)),
        };
        let (fitted, zoom) = super::fit_pin_to_display(&content, bounds, 1.0, Some(display), 1.0);
        assert!((zoom - 1.0).abs() < 0.001);
        assert_eq!(f32::from(fitted.origin.x), 1820.0);
        assert_eq!(f32::from(fitted.origin.y), 980.0);
    }

    #[test]
    fn text_pins_hug_short_content() {
        let (width, height) = text_pin_size("Hi");
        assert!(
            width < 200.0,
            "short text should not take card width: {width}"
        );
        assert!(
            height <= 48.0,
            "one line should stay near one row: {height}"
        );
    }

    #[test]
    fn text_pins_grow_with_wrapping_and_cap_the_width() {
        let (width, height) = text_pin_size(&"wide ".repeat(80));
        assert!(width <= super::CARD_WIDTH);
        assert!(height > 48.0);
    }

    #[test]
    fn small_edit_bases_stay_in_memory() {
        let frame = Arc::new(RgbaFrame::from_pixels(16, 16, vec![7; 16 * 16 * 4]).unwrap());
        let state = PinEditState::new(
            frame,
            AnnotationDocument::new(PhysicalSize::new(16, 16)),
            RegionEffects::default(),
            false,
            0,
            1.0,
        );
        assert!(matches!(
            *state.base.lock().expect("base lock"),
            PinBase::Frame(_)
        ));
    }

    #[test]
    fn large_edit_bases_are_compressed_and_round_trip() {
        let width = 2049;
        let height = 1024;
        let pixels = vec![0x5A; width as usize * height as usize * 4];
        let frame = Arc::new(RgbaFrame::from_pixels(width, height, pixels).unwrap());
        let state = PinEditState::new(
            frame.clone(),
            AnnotationDocument::new(PhysicalSize::new(width, height)),
            RegionEffects::default(),
            false,
            0,
            1.0,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !matches!(
            *state.base.lock().expect("base lock"),
            PinBase::Encoded { .. }
        ) {
            assert!(
                std::time::Instant::now() < deadline,
                "pin base compression timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let decoded = state.base_frame().expect("compressed base decodes");
        assert_eq!(decoded.pixels(), frame.pixels());
    }
}
