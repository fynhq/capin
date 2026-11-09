use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use core_types::{PhysicalRect, PhysicalSize, RgbaFrame};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{
    WindowStyle, apply as apply_window_style, set_click_through, system_accent_hex,
};
use serde::{Deserialize, Serialize};

use crate::i18n::{Language, t};
use crate::image_util;
use crate::logging::{log_error, log_info, log_warn};
use crate::settings_state;
use crate::storage;
use crate::ui::centered_bounds;

const MAX_RESTORED_PINS: usize = 20;

/// macOS `WindowKind::PopUp` is an `NSPanel` with `NonactivatingPanel`, which
/// can never take keyboard focus; pins need key status for the system copy
/// shortcut on selected OCR text. Windows keeps the popup tool window.
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PinMenuAction {
    Rotate,
    Edit,
    Copy,
    Save,
    CopyQr,
    Through,
    Close,
}

impl PinMenuAction {
    fn id(self) -> &'static str {
        match self {
            Self::Rotate => "pin-menu-rotate",
            Self::Edit => "pin-menu-edit",
            Self::Copy => "pin-menu-copy",
            Self::Save => "pin-menu-save",
            Self::CopyQr => "pin-menu-copy-qr",
            Self::Through => "pin-menu-through",
            Self::Close => "pin-menu-close",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Rotate => "pin.rotate",
            Self::Edit => "pin.edit",
            Self::Copy => "pin.copy",
            Self::Save => "pin.save",
            Self::CopyQr => "pin.copy_qr",
            Self::Through => "pin.through",
            Self::Close => "pin.close",
        }
    }

    fn separated(self) -> bool {
        matches!(self, Self::Through | Self::Close)
    }
}

#[derive(Clone)]
enum PinContent {
    Image {
        frame: Option<Arc<RgbaFrame>>,
        image: Arc<RenderImage>,
        size: PhysicalSize,
        hash: u64,
    },
    Text(String),
}

impl PinContent {
    fn from_frame(frame: RgbaFrame) -> Self {
        let hash = crate::capture_history::hash_frame(&frame);
        Self::from_frame_with_hash(frame, hash)
    }

    fn from_frame_with_hash(frame: RgbaFrame, hash: u64) -> Self {
        Self::from_frame_shared(Arc::new(frame), hash)
    }

    fn from_frame_shared(frame: Arc<RgbaFrame>, hash: u64) -> Self {
        let size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image(&frame);
        Self::Image {
            frame: Some(frame),
            image,
            size,
            hash,
        }
    }

    fn frame(&self) -> Option<&RgbaFrame> {
        match self {
            Self::Image { frame, .. } => frame.as_deref(),
            _ => None,
        }
    }

    fn frame_arc(&self) -> Option<Arc<RgbaFrame>> {
        match self {
            Self::Image { frame, .. } => frame.as_ref().map(Arc::clone),
            _ => None,
        }
    }

    /// Drops the CPU copy after OCR has finished; the render image and the
    /// persisted PNG remain available. Pin actions reload it on demand.
    fn drop_frame(&mut self) {
        if let Self::Image { frame, .. } = self {
            *frame = None;
        }
    }

    fn hash(&self) -> Option<u64> {
        match self {
            Self::Image { hash, .. } => Some(*hash),
            _ => None,
        }
    }

    fn image_size(&self) -> Option<PhysicalSize> {
        match self {
            Self::Image { size, .. } => Some(*size),
            _ => None,
        }
    }

    fn clipboard_text(&self) -> Option<String> {
        match self {
            Self::Image { .. } => None,
            Self::Text(text) => Some(text.clone()),
        }
    }

    fn kind(&self) -> PinKind {
        match self {
            Self::Image { .. } => PinKind::Image,
            Self::Text(text) => PinKind::Text(text.clone()),
        }
    }

    fn preferred_zoom(&self) -> f32 {
        match self {
            Self::Image { size, .. } => (900.0 / size.width as f32)
                .min(700.0 / size.height as f32)
                .min(1.0),
            Self::Text(_) => 1.0,
        }
    }

    fn size(&self, zoom: f32) -> (f32, f32) {
        match self {
            Self::Image { size, .. } => (size.width as f32 * zoom, size.height as f32 * zoom),
            Self::Text(text) => text_pin_size(text),
        }
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

/// Text pins hug their content instead of taking a card-width block: the width
/// follows the longest line up to a readable cap, and the height only counts
/// the lines that actually wrap at that width.
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

struct PinState {
    kind: PinKind,
    entity: Entity<PinView>,
    window: AnyWindowHandle,
    image_path: Option<PathBuf>,
    /// Hash of the pinned image, used to find the pin when the same image is
    /// triggered again.
    content_hash: Option<u64>,
    bounds: Bounds<Pixels>,
    opacity: f32,
    zoom: f32,
    click_through: bool,
}

#[derive(Serialize, Deserialize)]
struct PersistedPin {
    id: u64,
    kind: String,
    image: Option<String>,
    text: Option<String>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    opacity: f32,
    zoom: f32,
    #[serde(default)]
    click_through: bool,
}

pub fn pin_last(cx: &mut App) {
    if let Some(item) = cx.read_from_clipboard() {
        for entry in item.entries() {
            let ClipboardEntry::Image(image) = entry else {
                continue;
            };
            let Ok(image) = image::load_from_memory(&image.bytes) else {
                continue;
            };
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            if let Ok(frame) = RgbaFrame::from_pixels(width, height, rgba.into_raw()) {
                pin_image(frame, cx);
                return;
            }
        }
        for entry in item.entries() {
            let ClipboardEntry::String(text) = entry else {
                continue;
            };
            let text = text.text.trim_end();
            if text.is_empty() {
                continue;
            }
            open_pin_content(PinContent::Text(text.to_owned()), cx);
            return;
        }
    }

    if let Some(frame) = crate::capture_history::latest() {
        pin_image(frame, cx);
    } else {
        log_warn!("pin", "no clipboard content or recent capture to pin");
    }
}

/// Pins an image, or applies the configured repeat behavior when the same
/// screenshot is triggered again without a new clipboard source.
fn pin_image(frame: RgbaFrame, cx: &mut App) {
    let hash = crate::capture_history::hash_frame(&frame);
    let repeated = LAST_PIN_TRIGGER.with(|slot| slot.get() == Some(hash))
        || PINS.with(|pins| {
            pins.borrow()
                .values()
                .any(|state| state.content_hash == Some(hash))
        });
    if repeated {
        let behavior = settings_state::get().repeat_pin;
        if behavior == settings_state::RepeatPinBehavior::RestoreHistory
            && crate::capture_history::contains(hash)
            && let Some(history_frame) = crate::capture_history::cycle(hash)
        {
            open_pin_content(PinContent::from_frame(history_frame), cx);
            // Keep the trigger hash stable so another press advances the
            // history cursor instead of pinning the source again.
            LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
            return;
        }
        // Flash an on-screen pin with the same image. When every matching pin
        // was closed, fall through so the shortcut still reopens the image.
        if flash_existing_pin(hash, cx) {
            return;
        }
    }
    open_image_pin(frame, hash, cx);
}

/// Brings the pin holding `hash` to the front and flashes its outline.
fn flash_existing_pin(hash: u64, cx: &mut App) -> bool {
    let target = PINS.with(|pins| {
        pins.borrow()
            .values()
            .find(|state| state.content_hash == Some(hash))
            .map(|state| (state.entity.clone(), state.window))
    });
    let Some((entity, window)) = target else {
        return false;
    };
    entity.update(cx, |view, cx| view.flash(cx));
    let _ = window.update(cx, |_, window, _| window.activate_window());
    true
}

pub fn open_frame_with_hash(frame: RgbaFrame, hash: u64, cx: &mut App) {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    let content = PinContent::from_frame_with_hash(frame, hash);
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    let bounds = default_bounds(cx, width, height);
    let _ = open_pin_content_at(content, bounds, zoom, true, cx);
}

/// Pins a capture at a screen rectangle using a hash already computed by the
/// caller. Capture images are physical pixels, so `zoom` is the inverse
/// monitor scale; that keeps the pin at the same apparent size instead of
/// growing on high-DPI screens. Reusing the hash avoids a second full-image
/// pass.
pub fn open_frame_at_with_hash(
    frame: RgbaFrame,
    bounds: Bounds<Pixels>,
    zoom: f32,
    hash: u64,
    cx: &mut App,
) -> Option<u64> {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    open_pin_content_at(
        PinContent::from_frame_with_hash(frame, hash),
        bounds,
        zoom,
        true,
        cx,
    )
}

fn open_image_pin(frame: RgbaFrame, hash: u64, cx: &mut App) {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    open_pin_content(PinContent::from_frame_with_hash(frame, hash), cx);
}

fn open_pin_content(content: PinContent, cx: &mut App) {
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    log_info!("pin", "opening pin {}x{}", width.round(), height.round());
    let bounds = default_bounds(cx, width, height);
    open_pin_inner(
        content, None, bounds, zoom, 1.0, false, None, true, false, cx,
    );
}

fn open_pin_content_at(
    content: PinContent,
    bounds: Bounds<Pixels>,
    zoom: f32,
    record_history: bool,
    cx: &mut App,
) -> Option<u64> {
    let zoom = zoom.clamp(0.1, 4.0);
    let size = bounds.size;
    log_info!(
        "pin",
        "opening pin at {}x{}",
        f32::from(size.width).round(),
        f32::from(size.height).round()
    );
    open_pin_inner(
        content,
        None,
        bounds,
        zoom,
        1.0,
        false,
        None,
        true,
        record_history,
        cx,
    )
}

fn default_bounds(cx: &App, width: f32, height: f32) -> Bounds<Pixels> {
    let mut bounds = centered_bounds(cx, width, height);
    let cascade = PIN_CASCADE.with(|index| {
        let value = index.get();
        index.set((value + 1) % 8);
        value as f32 * 24.0
    });
    bounds.origin = point(bounds.origin.x + px(cascade), bounds.origin.y + px(cascade));
    bounds
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
    record_history: bool,
    cx: &mut App,
) -> Option<u64> {
    let id = id.unwrap_or_else(next_pin_id);
    let kind = content.kind();
    let content_hash = content.hash();
    let had_existing_image = existing_image.is_some();
    let initial_ocr_path = existing_image.clone();
    let image_path = if let Some(path) = existing_image {
        Some(path)
    } else if let Some(frame) = content.frame() {
        save_pin_image(id, frame)
    } else {
        None
    };
    let display_id = display_for_bounds(cx, bounds);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: pin_window_kind(),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-pin".to_owned()),
        ..Default::default()
    };

    let content_for_view = content.clone();
    match gpui_kit::open_window(options, cx, move |window, cx| {
        if let Err(error) = apply_window_style(&*window, WindowStyle::PIN) {
            log_error!("pin", "failed to style pin window: {error}");
        }
        if click_through {
            let _ = set_click_through(&*window, true);
        }
        let view = cx.new(|cx| {
            PinView::new(
                id,
                content_for_view,
                zoom,
                opacity,
                click_through,
                initial_ocr_path,
                cx,
            )
        });
        view.update(cx, |view, cx| view.watch_close(window, cx));
        view
    }) {
        Ok((window, entity)) => {
            if record_history
                && let (Some(hash), Some(path)) = (content_hash, image_path.as_deref())
            {
                // The pin file is already encoded; copy it into history
                // instead of encoding the same frame again.
                crate::capture_history::record_png_file(path, hash);
            }
            let view_for_release = entity.clone();
            let state = PinState {
                kind,
                entity,
                window,
                image_path,
                content_hash,
                bounds,
                opacity,
                zoom,
                click_through,
            };
            PINS.with(|pins| {
                pins.borrow_mut().insert(id, state);
            });
            // OCR already holds its own Arc source; drop the pin's CPU frame
            // now that the PNG path is recorded. Actions reload it lazily.
            view_for_release.update(cx, |view, _| view.release_frame_if_persisted());
            if persist {
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

pub fn restore(cx: &mut App) {
    if PINS.with(|pins| !pins.borrow().is_empty()) {
        return;
    }
    let Some(directory) = storage::pins_dir() else {
        return;
    };
    let manifest_path = directory.join("pins.json");
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    let Ok(entries) = serde_json::from_str::<Vec<PersistedPin>>(&text) else {
        return;
    };

    for entry in entries.into_iter().take(MAX_RESTORED_PINS) {
        if !entry.x.is_finite()
            || !entry.y.is_finite()
            || !entry.width.is_finite()
            || !entry.height.is_finite()
            || entry.width < 1.0
            || entry.height < 1.0
        {
            continue;
        }
        let content = match entry.kind.as_str() {
            "image" => {
                let Some(name) = entry
                    .image
                    .as_deref()
                    .and_then(|name| Path::new(name).file_name().map(|name| name.to_owned()))
                else {
                    continue;
                };
                let path = directory.join(name);
                let Some(frame) = load_pin_image(&path) else {
                    continue;
                };
                PinContent::from_frame(frame)
            }
            "text" => {
                let Some(text) = entry.text.filter(|text| !text.is_empty()) else {
                    continue;
                };
                PinContent::Text(text)
            }
            _ => continue,
        };
        let zoom = entry.zoom.clamp(0.1, 4.0);
        let (width, height) = content.size(zoom);
        let bounds = ensure_visible(
            cx,
            Bounds {
                origin: point(px(entry.x), px(entry.y)),
                // Content size is authoritative: a previous zoom might not
                // have been written together with the window bounds.
                size: size(px(width), px(height)),
            },
        );
        let existing_image = match &content {
            PinContent::Image { .. } => entry
                .image
                .as_deref()
                .and_then(|name| Path::new(name).file_name())
                .map(|name| directory.join(name)),
            _ => None,
        };
        let _ = open_pin_inner(
            content,
            Some(entry.id),
            bounds,
            zoom,
            entry.opacity.clamp(0.2, 1.0),
            entry.click_through,
            existing_image,
            false,
            false,
            cx,
        );
    }
    save_manifest();
}

pub fn replace_image(id: u64, frame: RgbaFrame, cx: &mut App) -> bool {
    let hash = crate::capture_history::hash_frame(&frame);
    let state = PINS.with(|pins| {
        let pins = pins.borrow();
        let state = pins.get(&id)?;
        Some((
            state.entity.clone(),
            state.window,
            state.image_path.clone(),
            state.zoom,
            state.bounds.origin,
        ))
    });
    let Some((entity, window_handle, image_path, zoom, origin)) = state else {
        return false;
    };
    let path = if let Some(path) = image_path {
        if image::save_buffer(
            &path,
            frame.pixels(),
            frame.width(),
            frame.height(),
            image::ColorType::Rgba8,
        )
        .is_err()
        {
            return false;
        }
        Some(path)
    } else {
        save_pin_image(id, &frame)
    };
    let width = frame.width() as f32 * zoom;
    let height = frame.height() as f32 * zoom;
    entity.update(cx, |view, cx| {
        view.replace_image(Arc::new(frame), hash, cx);
    });
    let _ = window_handle.update(cx, |_, window, _| {
        window.resize(size(px(width), px(height)));
    });
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.kind = PinKind::Image;
            state.content_hash = Some(hash);
            state.image_path = path;
            state.bounds = Bounds {
                origin,
                size: size(px(width), px(height)),
            };
        }
    });
    save_manifest();
    true
}

pub fn close_pin(id: u64) {
    let image_path = PINS.with(|pins| {
        pins.borrow_mut()
            .remove(&id)
            .and_then(|state| state.image_path)
    });
    if let Some(path) = image_path {
        let _ = std::fs::remove_file(path);
    }
    save_manifest();
}

fn persist_rotated_image(id: u64, frame: &RgbaFrame, hash: u64, bounds: Bounds<Pixels>) -> bool {
    let path = PINS.with(|pins| {
        pins.borrow()
            .get(&id)
            .and_then(|state| state.image_path.clone())
    });
    let saved = match path {
        Some(path) => {
            if image::save_buffer(
                &path,
                frame.pixels(),
                frame.width(),
                frame.height(),
                image::ColorType::Rgba8,
            )
            .is_err()
            {
                return false;
            }
            Some(path)
        }
        None => save_pin_image(id, frame),
    };
    if saved.is_none() {
        return false;
    }
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.kind = PinKind::Image;
            state.content_hash = Some(hash);
            state.image_path = saved;
            state.bounds = bounds;
        }
    });
    save_manifest();
    true
}

pub fn is_pin_window(handle: &AnyWindowHandle) -> bool {
    PINS.with(|pins| {
        pins.borrow()
            .values()
            .any(|state| state.window.window_id() == handle.window_id())
    })
}

pub fn refresh_recognition(cx: &mut App) {
    let entities: Vec<Entity<PinView>> = PINS.with(|pins| {
        pins.borrow()
            .values()
            .map(|state| state.entity.clone())
            .collect()
    });
    for entity in entities {
        entity.update(cx, |view, cx| {
            view.ocr = None;
            view.selection = None;
            view.selecting = false;
            view.content_generation = view.content_generation.wrapping_add(1);
            view.start_ocr(cx);
            cx.notify();
        });
    }
}

pub fn click_through_count() -> usize {
    PINS.with(|pins| {
        pins.borrow()
            .values()
            .filter(|state| state.click_through)
            .count()
    })
}

pub fn release_click_through(cx: &mut App) -> usize {
    let states: Vec<(u64, Entity<PinView>, AnyWindowHandle)> = PINS.with(|pins| {
        pins.borrow()
            .iter()
            .filter(|(_, state)| state.click_through)
            .map(|(id, state)| (*id, state.entity.clone(), state.window))
            .collect()
    });
    let count = states.len();
    for (id, entity, window) in states {
        let _ = window.update(cx, |_, window, _| {
            let _ = set_click_through(&*window, false);
        });
        entity.update(cx, |view, cx| {
            view.click_through = false;
            cx.notify();
        });
        PINS.with(|pins| {
            if let Some(state) = pins.borrow_mut().get_mut(&id) {
                state.click_through = false;
            }
        });
    }
    save_manifest();
    crate::tray::refresh_menu();
    count
}

fn update_bounds(id: u64, bounds: Bounds<Pixels>) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.bounds = bounds;
        }
    });
    save_manifest();
}

fn update_meta(id: u64, zoom: f32, opacity: f32) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.zoom = zoom;
            state.opacity = opacity;
        }
    });
}

fn update_bounds_state(id: u64, bounds: Bounds<Pixels>) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.bounds = bounds;
        }
    });
}

/// Debounces manifest writes while the user scrolls to zoom or change opacity.
fn schedule_manifest_save(cx: &mut Context<PinView>) {
    let revision = MANIFEST_REVISION.with(|revision| {
        let next = revision.get().wrapping_add(1);
        revision.set(next);
        next
    });
    cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        let _ = this.update(cx, |_, _| {
            if MANIFEST_REVISION.with(|current| current.get()) == revision {
                save_manifest();
            }
        });
    })
    .detach();
}

fn set_state_click_through(id: u64, enabled: bool) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.click_through = enabled;
        }
    });
    save_manifest();
}

fn next_pin_id() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let mut id = millis.max(1);
    PINS.with(|pins| {
        while pins.borrow().contains_key(&id) {
            id += 1;
        }
    });
    id
}

fn save_pin_image(id: u64, frame: &RgbaFrame) -> Option<PathBuf> {
    let directory = storage::pins_dir()?;
    let path = directory.join(format!("pin-{id}.png"));
    image::save_buffer(
        &path,
        frame.pixels(),
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
    )
    .ok()?;
    Some(path)
}

fn load_pin_image(path: &Path) -> Option<RgbaFrame> {
    let image = image::open(path).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    RgbaFrame::from_pixels(width, height, image.into_raw()).ok()
}

fn save_manifest() {
    let Some(directory) = storage::pins_dir() else {
        return;
    };
    let path = directory.join("pins.json");
    let entries: Vec<PersistedPin> = PINS.with(|pins| {
        let mut entries: Vec<PersistedPin> = pins
            .borrow()
            .iter()
            .filter_map(|(id, state)| {
                let (kind, image, text) = match &state.kind {
                    PinKind::Image => {
                        let image = state
                            .image_path
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy().to_string())?;
                        ("image", Some(image), None)
                    }
                    PinKind::Text(text) => ("text", None, Some(text.clone())),
                };
                Some(PersistedPin {
                    id: *id,
                    kind: kind.to_owned(),
                    image,
                    text,
                    x: f32::from(state.bounds.origin.x),
                    y: f32::from(state.bounds.origin.y),
                    width: f32::from(state.bounds.size.width),
                    height: f32::from(state.bounds.size.height),
                    opacity: state.opacity,
                    zoom: state.zoom,
                    click_through: state.click_through,
                })
            })
            .collect();
        entries.sort_by_key(|entry| entry.id);
        entries
    });
    if entries.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(path, text);
    }
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
    fn for_window(window: &Window) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let accent_hex = system_accent_hex().unwrap_or(if cfg!(target_os = "macos") {
            0x0A84FF
        } else {
            0x0067C0
        });
        let accent = rgb(accent_hex);
        let selection = rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x66);
        if dark {
            Self {
                card: rgba(0x1E1E1EF2),
                text: rgb(0xF2F2F2),
                border: rgba(0xFFFFFF24),
                menu: rgba(0x2B2B2BFA),
                menu_hover: rgba(0xFFFFFF16),
                menu_border: rgba(0xFFFFFF22),
                accent,
                selection,
            }
        } else {
            Self {
                card: rgba(0xFFFFFFF2),
                text: rgb(0x1A1A1A),
                border: rgba(0x0000001A),
                menu: rgba(0xF9F9F9FA),
                menu_hover: rgba(0x00000010),
                menu_border: rgba(0x0000001A),
                accent,
                selection,
            }
        }
    }
}

struct PinView {
    id: u64,
    content: PinContent,
    zoom: f32,
    opacity: f32,
    click_through: bool,
    notice: Option<String>,
    notice_serial: u64,
    hud: Option<String>,
    hud_serial: u64,
    ocr: Option<platform_ocr::OcrOutput>,
    /// Restored pins start OCR from their saved PNG instead of keeping a
    /// full-frame Arc queued for every restored window.
    ocr_path: Option<PathBuf>,
    selection: Option<(usize, usize)>,
    selecting: bool,
    /// Window-local press position while a text click may still turn into a
    /// window drag.
    press: Option<Point<Pixels>>,
    content_generation: u64,
    flash_alpha: f32,
    flash_generation: u64,
    focus_handle: FocusHandle,
}

impl PinView {
    fn new(
        id: u64,
        content: PinContent,
        zoom: f32,
        opacity: f32,
        click_through: bool,
        ocr_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            id,
            content,
            zoom,
            opacity,
            click_through,
            ocr_path,
            notice: None,
            notice_serial: 0,
            hud: None,
            hud_serial: 0,
            ocr: None,
            selection: None,
            selecting: false,
            press: None,
            content_generation: 1,
            flash_alpha: 0.0,
            flash_generation: 0,
            focus_handle: cx.focus_handle(),
        };
        cx.on_release(|this, cx| {
            if let PinContent::Image { image, .. } = &this.content {
                cx.drop_image(image.clone(), None);
            }
        })
        .detach();
        view.start_ocr(cx);
        view
    }

    /// Pulses a bright outline so a repeated pin shortcut makes the existing
    /// pin easy to find instead of stacking another copy on top.
    fn flash(&mut self, cx: &mut Context<Self>) {
        self.flash_generation = self.flash_generation.wrapping_add(1);
        let generation = self.flash_generation;
        self.flash_alpha = 1.0;
        cx.notify();
        cx.spawn(async move |this, cx| {
            for step in 0..5 {
                cx.background_executor()
                    .timer(Duration::from_millis(110))
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.flash_generation != generation {
                            return false;
                        }
                        this.flash_alpha = if step % 2 == 0 { 0.2 } else { 1.0 };
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    return;
                }
            }
            this.update(cx, |this, cx| {
                if this.flash_generation == generation {
                    this.flash_alpha = 0.0;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn persisted_path(&self) -> Option<PathBuf> {
        PINS.with(|pins| {
            pins.borrow()
                .get(&self.id)
                .and_then(|state| state.image_path.clone())
        })
    }

    /// Returns the CPU frame for an explicit user action (copy/save/edit/
    /// rotate). After OCR the frame is dropped and reloaded from the pinned
    /// PNG only when needed.
    fn frame_for_action(&self) -> Option<Arc<RgbaFrame>> {
        self.content.frame_arc().or_else(|| {
            self.persisted_path()
                .and_then(|path| load_pin_image(&path))
                .map(Arc::new)
        })
    }

    fn ocr_source(&mut self) -> Option<crate::ocr_queue::Source> {
        if let Some(path) = self.ocr_path.take() {
            return Some(crate::ocr_queue::Source::Path(path));
        }
        if let Some(frame) = self.content.frame_arc() {
            Some(crate::ocr_queue::Source::Frame(frame))
        } else {
            self.persisted_path().map(crate::ocr_queue::Source::Path)
        }
    }

    fn release_frame_if_persisted(&mut self) {
        if self.persisted_path().is_some() {
            self.content.drop_frame();
        }
    }

    fn start_ocr(&mut self, cx: &mut Context<Self>) {
        let settings = settings_state::get();
        if !settings.pin_text_ocr && !settings.pin_qr {
            self.ocr = None;
            self.release_frame_if_persisted();
            return;
        }
        let Some(source) = self.ocr_source() else {
            self.ocr = None;
            return;
        };
        let options = platform_ocr::OcrOptions {
            text: settings.pin_text_ocr,
            qr: settings.pin_qr,
        };
        let generation = self.content_generation;
        cx.spawn(async move |this, cx| {
            let current = this
                .update(cx, |this, _| this.content_generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            // Sources are either a shared frame or a saved PNG path; queued
            // jobs therefore hold no duplicate pixel buffers, and the queue
            // serializes the expensive decode/recognition work.
            let output = cx
                .background_executor()
                .spawn(async move { crate::ocr_queue::recognize(source, options) })
                .await;
            this.update(cx, |this, cx| {
                if this.content_generation == generation {
                    this.ocr = Some(output);
                    this.release_frame_if_persisted();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        // The OCR task holds its own Arc source; actions reload from disk.
        self.release_frame_if_persisted();
    }

    fn qr_code(&self) -> Option<&str> {
        if !settings_state::get().pin_qr {
            return None;
        }
        self.ocr
            .as_ref()
            .and_then(|output| output.qr_codes.first())
            .map(String::as_str)
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
        let zoom = self.zoom.max(0.1);
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
            // Only a click inside the line's own glyph run selects text.
            // Falling back to the nearest character anywhere in the image made
            // every drag over a text screenshot start a selection, so the pin
            // could not be moved by its body.
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

    fn word_range_at(&self, index: usize) -> Option<(usize, usize)> {
        let lines = self.text_lines()?;
        let mut base = 0usize;
        for line in lines {
            let length = line.chars.len();
            if index < base + length {
                let local = index - base;
                // CJK runs no longer carry the OCR word separator, so treat
                // one glyph as the double-click unit instead of selecting the
                // entire line.
                if !line.chars[local].ch.is_ascii() {
                    return Some((base + local, base + local));
                }
                let mut start = local;
                while start > 0 && !line.chars[start - 1].ch.is_whitespace() {
                    start -= 1;
                }
                let mut end = local;
                while end + 1 < length && !line.chars[end + 1].ch.is_whitespace() {
                    end += 1;
                }
                return Some((base + start, base + end));
            }
            base += length;
        }
        None
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let (anchor, focus) = self.selection?;
        Some((anchor.min(focus), anchor.max(focus)))
    }

    fn selected_text(&self) -> Option<String> {
        let lines = self.text_lines()?;
        let (start, end) = self.selection_range()?;
        let mut text = String::new();
        let mut base = 0usize;
        let mut first_line = true;
        for line in lines {
            let length = line.chars.len();
            if end < base || start >= base + length {
                base += length;
                continue;
            }
            if !first_line {
                text.push('\n');
            }
            let from = start.saturating_sub(base).min(length);
            let to = (end.saturating_sub(base) + 1).min(length);
            if let Some(chars) = line.chars.get(from..to) {
                for ch in chars {
                    text.push(ch.ch);
                }
            }
            first_line = false;
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

    fn select_word_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.text_index_at(position) else {
            return false;
        };
        let Some((start, end)) = self.word_range_at(index) else {
            return false;
        };
        self.selection = Some((start, end));
        self.selecting = false;
        cx.notify();
        true
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

    fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.image_size().is_none() {
            return;
        }
        self.zoom = zoom.clamp(0.15, 4.0);
        let (width, height) = self.content.size(self.zoom);
        window.resize(size(px(width), px(height)));
        let origin = window.bounds().origin;
        update_bounds_state(
            self.id,
            Bounds {
                origin,
                size: size(px(width), px(height)),
            },
        );
        update_meta(self.id, self.zoom, self.opacity);
        schedule_manifest_save(cx);
        let language = settings_state::get().language;
        let percent = (self.zoom * 100.0).round() as i32;
        self.show_hud(format!("{} {percent}%", t(language, "pin.zoom_hud")), cx);
        cx.notify();
    }

    fn set_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        self.opacity = opacity.clamp(0.2, 1.0);
        update_meta(self.id, self.zoom, self.opacity);
        schedule_manifest_save(cx);
        let language = settings_state::get().language;
        let percent = (self.opacity * 100.0).round() as i32;
        self.show_hud(format!("{} {percent}%", t(language, "pin.opacity_hud")), cx);
        cx.notify();
    }

    fn begin_window_drag(&mut self, window: &mut Window) {
        #[cfg(target_os = "windows")]
        {
            // GPUI's Windows backend leaves `start_window_move` as a no-op, so
            // ask the native title-bar move loop directly.
            let _ = platform_window::begin_window_drag(&*window);
            update_bounds(self.id, window.bounds());
        }
        #[cfg(not(target_os = "windows"))]
        {
            window.start_window_move();
        }
    }

    fn release_image(&mut self, cx: &mut Context<Self>) {
        if let PinContent::Image { image, .. } = &self.content {
            cx.drop_image(image.clone(), None);
        }
    }

    /// Releases the pin texture before a native window-close request removes
    /// the renderer. App-driven close already calls `release_image` explicitly.
    fn watch_close(&self, window: &Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = view.update(cx, |this, cx| this.release_image(cx));
            true
        });
    }

    fn replace_image(&mut self, frame: Arc<RgbaFrame>, hash: u64, cx: &mut Context<Self>) {
        let previous = std::mem::replace(
            &mut self.content,
            PinContent::from_frame_shared(frame, hash),
        );
        if let PinContent::Image { image, .. } = previous {
            cx.drop_image(image, None);
        }
        self.content_generation = self.content_generation.wrapping_add(1);
        self.ocr = None;
        self.selection = None;
        self.selecting = false;
        self.start_ocr(cx);
        cx.notify();
    }

    fn show_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = Some(message);
        self.notice_serial = self.notice_serial.wrapping_add(1);
        let serial = self.notice_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            this.update(cx, |this, cx| {
                if this.notice_serial == serial {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Short-lived numeric feedback for wheel gestures. Kept separate from
    /// `notice` so a zoom tick does not hide a copy/save message.
    fn show_hud(&mut self, message: String, cx: &mut Context<Self>) {
        self.hud = Some(message);
        self.hud_serial = self.hud_serial.wrapping_add(1);
        let serial = self.hud_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            this.update(cx, |this, cx| {
                if this.hud_serial == serial {
                    this.hud = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
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

    fn copy_image(&mut self, cx: &mut Context<Self>) {
        // The persisted PNG is already encoded; copying its bytes avoids
        // decoding the pin and re-encoding it just to reach the clipboard.
        if let Some(path) = self.persisted_path()
            && let Ok(bytes) = std::fs::read(path)
        {
            image_util::copy_png_bytes(&bytes, cx);
            let language = settings_state::get().language;
            self.show_notice(t(language, "pin.copied").to_owned(), cx);
            return;
        }
        let Some(frame) = self.frame_for_action() else {
            return;
        };
        if image_util::copy_png(&frame, cx).is_ok() {
            let language = settings_state::get().language;
            self.show_notice(t(language, "pin.copied").to_owned(), cx);
        }
    }

    fn save_image(&mut self, cx: &mut Context<Self>) {
        let language = settings_state::get().language;
        let result = if let Some(path) = self.persisted_path() {
            crate::storage::save_png_file(&path)
        } else {
            let Some(frame) = self.frame_for_action() else {
                return;
            };
            image_util::save_png(&frame)
        };
        match result {
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

    fn open_editor(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.frame_for_action() else {
            return;
        };
        let bounds = PINS.with(|pins| pins.borrow().get(&self.id).map(|state| state.bounds));
        let Some(bounds) = bounds else {
            return;
        };
        if !crate::editor::open_pin_editor_overlay(self.id, frame, bounds, cx) {
            let language = settings_state::get().language;
            self.show_notice(t(language, "pin.edit_failed").to_owned(), cx);
        }
    }

    fn menu_items(&self, language: Language) -> Vec<(PinMenuAction, String)> {
        let mut items = Vec::new();
        match &self.content {
            PinContent::Image { .. } => {
                items.push(PinMenuAction::Rotate);
                items.push(PinMenuAction::Edit);
                items.push(PinMenuAction::Copy);
                items.push(PinMenuAction::Save);
                if self.qr_code().is_some() {
                    items.push(PinMenuAction::CopyQr);
                }
            }
            PinContent::Text(_) => {
                items.push(PinMenuAction::Copy);
            }
        }
        items.push(PinMenuAction::Through);
        items.push(PinMenuAction::Close);
        items
            .into_iter()
            .map(|action| (action, t(language, action.label_key()).to_owned()))
            .collect()
    }

    fn body(&self, theme: &PinTheme, _language: Language) -> Div {
        match &self.content {
            PinContent::Image { image, .. } => div()
                .size_full()
                .child(img(image.clone()).size_full().object_fit(ObjectFit::Fill)),
            PinContent::Text(text) => div()
                .size_full()
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(theme.card)
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .text_size(px(14.0))
                .text_color(theme.text)
                .child(text.clone()),
        }
    }
}

impl Render for PinView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PinTheme::for_window(window);
        let language = settings_state::get().language;
        let notice = self.notice.clone();
        let hud = self.hud.clone();
        let body = self.body(&theme, language).opacity(self.opacity);
        let zoom = self.zoom.max(0.1);
        let text_layers: Vec<Div> = self
            .selection_rects()
            .into_iter()
            .map(|rect| {
                div()
                    .absolute()
                    .left(px(rect.origin.x as f32 * zoom))
                    .top(px(rect.origin.y as f32 * zoom))
                    .w(px(rect.size.width as f32 * zoom))
                    .h(px(rect.size.height as f32 * zoom))
                    .bg(theme.selection)
            })
            .collect();
        let flash = (self.flash_alpha > 0.001).then(|| {
            div()
                .absolute()
                .inset_0()
                .border_4()
                .border_color(theme.accent)
                .opacity(self.flash_alpha)
        });
        let cursor = match &self.content {
            PinContent::Image { .. } => CursorStyle::OpenHand,
            PinContent::Text(_) => CursorStyle::IBeam,
        };
        let root = div()
            .size_full()
            .id("pin-window")
            .cursor(cursor)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let key = event.keystroke.key.as_str();
                let command =
                    event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
                if command && key == "c" {
                    this.copy_selected_text(cx);
                    cx.stop_propagation();
                    return;
                }
                if command && key == "a" && this.select_all_text(cx) {
                    cx.stop_propagation();
                    return;
                }
                if key == "escape" {
                    this.selection = None;
                    this.selecting = false;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    crate::pin::close_pin_menu(cx);
                    window.focus(&this.focus_handle, cx);
                    if event.click_count >= 2 {
                        this.select_word_at(event.position, cx);
                        cx.notify();
                        return;
                    }
                    let bounds = window.bounds().size;
                    let x = f32::from(event.position.x);
                    let y = f32::from(event.position.y);
                    let margin = 6.0;
                    let near_edge = x < margin
                        || y < margin
                        || x > f32::from(bounds.width) - margin
                        || y > f32::from(bounds.height) - margin;
                    if near_edge
                        || event.modifiers.alt
                        || !this.begin_text_selection(event.position)
                    {
                        this.selection = None;
                        this.selecting = false;
                        this.press = None;
                        this.begin_window_drag(window);
                    } else {
                        this.press = Some(event.position);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let bounds = window.bounds();
                    let position = point(
                        px(f32::from(bounds.origin.x) + f32::from(event.position.x)),
                        px(f32::from(bounds.origin.y) + f32::from(event.position.y)),
                    );
                    let items = this.menu_items(settings_state::get().language);
                    crate::pin::open_pin_menu(this.id, position, items, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.selecting {
                    let moved = this.press.is_some_and(|press| {
                        let dx = f32::from(event.position.x) - f32::from(press.x);
                        let dy = f32::from(event.position.y) - f32::from(press.y);
                        dx * dx + dy * dy > 25.0
                    });
                    if moved && !event.modifiers.shift {
                        // Plain drag moves the pin even when it starts on a
                        // text run; Shift keeps the drag as text selection.
                        this.selection = None;
                        this.selecting = false;
                        this.press = None;
                        this.begin_window_drag(window);
                        cx.notify();
                        return;
                    }
                    this.update_text_selection(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.press = None;
                    if this.selecting {
                        this.finish_text_selection(cx);
                    } else {
                        update_bounds(this.id, window.bounds());
                    }
                }),
            )
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let delta = match event.delta {
                    ScrollDelta::Lines(delta) => delta.y,
                    ScrollDelta::Pixels(delta) => f32::from(delta.y) / 20.0,
                };
                if event.modifiers.shift || this.content.image_size().is_none() {
                    this.set_opacity(this.opacity - delta * 0.05, cx);
                } else {
                    this.set_zoom(this.zoom + delta * 0.1, window, cx);
                }
            }))
            .child(body)
            .children(text_layers)
            .children(flash);

        root.children(hud.map(|message| {
            div()
                .absolute()
                .top(px(6.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(5.0))
                        .bg(theme.menu)
                        .border_1()
                        .border_color(theme.menu_border)
                        .text_size(px(10.5))
                        .text_color(theme.text)
                        .child(message),
                )
        }))
        .children(notice.map(|notice| {
            div()
                .absolute()
                .bottom(px(8.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .bg(rgba(0x000000CC))
                        .text_size(px(10.0))
                        .text_color(rgb(0xFFFFFF))
                        .child(notice),
                )
        }))
    }
}

struct PinMenuView {
    pin_id: u64,
    items: Vec<(PinMenuAction, String)>,
    focus_handle: FocusHandle,
    _blur_subscription: Option<gpui_kit::Subscription>,
}

impl Render for PinMenuView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PinTheme::for_window(window);
        let mut menu = div()
            .size_full()
            .p(px(PIN_MENU_PADDING))
            .rounded(px(8.0))
            .bg(theme.menu)
            .border_1()
            .border_color(theme.menu_border)
            .shadow(vec![
                BoxShadow::new(px(0.0), px(6.0), hsla(0.0, 0.0, 0.0, 0.28)).blur_radius(px(18.0)),
            ])
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    forget_pin_menu();
                    window.remove_window();
                    cx.stop_propagation();
                }
            }));
        for (index, (action, label)) in self.items.iter().enumerate() {
            let action = *action;
            let label = label.clone();
            let pin_id = self.pin_id;
            if index > 0 && action.separated() {
                menu = menu.child(
                    div()
                        .w_full()
                        .py(px(4.0))
                        .child(div().h(px(1.0)).mx(px(4.0)).bg(theme.menu_border)),
                );
            }
            menu = menu.child(
                div()
                    .id(action.id())
                    .occlude()
                    .h(px(PIN_MENU_ITEM_HEIGHT))
                    .px(px(10.0))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .text_size(px(12.5))
                    .text_color(theme.text)
                    .hover(move |style| style.bg(theme.menu_hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(move |_, _, window, cx| {
                        forget_pin_menu();
                        perform_pin_action(pin_id, action, cx);
                        window.remove_window();
                    }))
                    .child(label),
            );
        }
        menu
    }
}

fn forget_pin_menu() {
    PIN_MENU.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn close_pin_menu(cx: &mut App) {
    let handle = PIN_MENU.with(|slot| slot.borrow_mut().take());
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

fn open_pin_menu(
    pin_id: u64,
    position: Point<Pixels>,
    items: Vec<(PinMenuAction, String)>,
    cx: &mut App,
) {
    close_pin_menu(cx);
    let width = PIN_MENU_WIDTH;
    let separators = items
        .iter()
        .enumerate()
        .filter(|(index, (action, _))| *index > 0 && action.separated())
        .count() as f32;
    let height = PIN_MENU_PADDING * 2.0
        + PIN_MENU_ITEM_HEIGHT * items.len() as f32
        + PIN_MENU_SEPARATOR * separators
        + 2.0;
    let position_x = f32::from(position.x);
    let position_y = f32::from(position.y);
    let display = cx.displays().into_iter().find(|display| {
        let bounds = display.bounds();
        let left = f32::from(bounds.origin.x);
        let top = f32::from(bounds.origin.y);
        position_x >= left
            && position_x < left + f32::from(bounds.size.width)
            && position_y >= top
            && position_y < top + f32::from(bounds.size.height)
    });
    let (display_id, area) = match display {
        Some(display) => (Some(display.id()), display.bounds()),
        None => match cx.primary_display() {
            Some(display) => (Some(display.id()), display.bounds()),
            None => (None, Bounds::default()),
        },
    };
    let left = f32::from(area.origin.x);
    let top = f32::from(area.origin.y);
    let right = left + f32::from(area.size.width);
    let bottom = top + f32::from(area.size.height);
    let origin = point(
        px(position_x.clamp(left, (right - width).max(left))),
        px(position_y.clamp(top, (bottom - height).max(top))),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin,
            size: size(px(width), px(height)),
        })),
        titlebar: None,
        focus: true,
        show: true,
        kind: pin_window_kind(),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-pin-menu".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, move |window, cx| {
        let _ = apply_window_style(
            &*window,
            WindowStyle {
                always_on_top: true,
                skip_taskbar: true,
                no_activate: false,
                exclude_from_capture: true,
                borderless: false,
                opaque: false,
            },
        );
        let view = cx.new(|cx| PinMenuView {
            pin_id,
            items,
            focus_handle: cx.focus_handle(),
            _blur_subscription: None,
        });
        let focus = view.read(cx).focus_handle.clone();
        view.update(cx, |view, cx| {
            view._blur_subscription = Some(cx.on_blur(&focus, window, |_, window, _| {
                forget_pin_menu();
                window.remove_window();
            }));
        });
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, _)) => {
            PIN_MENU.with(|slot| *slot.borrow_mut() = Some(handle));
        }
        Err(error) => log_error!("pin", "failed to open pin menu: {error}"),
    }
}

fn pin_handles(id: u64) -> Option<(Entity<PinView>, AnyWindowHandle)> {
    PINS.with(|pins| {
        pins.borrow()
            .get(&id)
            .map(|state| (state.entity.clone(), state.window))
    })
}

fn perform_pin_action(id: u64, action: PinMenuAction, cx: &mut App) {
    let Some((entity, window_handle)) = pin_handles(id) else {
        return;
    };
    match action {
        PinMenuAction::Rotate => rotate_pin(id, &entity, window_handle, cx),
        PinMenuAction::Edit => {
            entity.update(cx, |view, cx| view.open_editor(cx));
        }
        PinMenuAction::Copy => {
            entity.update(cx, |view, cx| {
                if matches!(view.content, PinContent::Image { .. }) {
                    view.copy_image(cx);
                } else {
                    view.copy_content(cx);
                }
            });
        }
        PinMenuAction::Save => {
            entity.update(cx, |view, cx| view.save_image(cx));
        }
        PinMenuAction::CopyQr => {
            entity.update(cx, |view, cx| view.copy_qr(cx));
        }
        PinMenuAction::Through => {
            enable_click_through(id, &entity, window_handle, cx);
        }
        PinMenuAction::Close => {
            entity.update(cx, |view, cx| view.release_image(cx));
            close_pin(id);
            let _ = window_handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

fn rotate_pin(id: u64, entity: &Entity<PinView>, window_handle: AnyWindowHandle, cx: &mut App) {
    let (frame, zoom) = {
        let view = entity.read(cx);
        (view.frame_for_action(), view.zoom)
    };
    let Some(frame) = frame else {
        return;
    };
    let rotated = frame.rotated_cw();
    let hash = crate::capture_history::hash_frame(&rotated);
    let width = rotated.width() as f32 * zoom;
    let height = rotated.height() as f32 * zoom;
    let Ok(origin) = window_handle.update(cx, |_, window, _| window.bounds().origin) else {
        return;
    };
    let bounds = Bounds {
        origin,
        size: size(px(width), px(height)),
    };
    if !persist_rotated_image(id, &rotated, hash, bounds) {
        entity.update(cx, |view, cx| {
            let language = settings_state::get().language;
            view.show_notice(t(language, "pin.rotate_failed").to_owned(), cx);
        });
        return;
    }
    let _ = window_handle.update(cx, |_, window, _| {
        window.resize(size(px(width), px(height)))
    });
    entity.update(cx, |view, cx| {
        view.replace_image(Arc::new(rotated), hash, cx)
    });
}

fn enable_click_through(
    id: u64,
    entity: &Entity<PinView>,
    window_handle: AnyWindowHandle,
    cx: &mut App,
) {
    let applied = window_handle
        .update(cx, |_, window, _| set_click_through(&*window, true).is_ok())
        .unwrap_or(false);
    if !applied {
        return;
    }
    entity.update(cx, |view, cx| {
        view.click_through = true;
        set_state_click_through(id, true);
        cx.notify();
    });
    crate::tray::refresh_menu();
}

fn display_for_bounds(cx: &App, bounds: Bounds<Pixels>) -> Option<DisplayId> {
    let center_x = bounds.origin.x + bounds.size.width / 2.0;
    let center_y = bounds.origin.y + bounds.size.height / 2.0;
    cx.displays()
        .into_iter()
        .find(|display| {
            let display = display.bounds();
            center_x >= display.origin.x
                && center_x < display.origin.x + display.size.width
                && center_y >= display.origin.y
                && center_y < display.origin.y + display.size.height
        })
        .map(|display| display.id())
        .or_else(|| cx.primary_display().map(|display| display.id()))
}

fn ensure_visible(cx: &App, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);
    for display in cx.displays() {
        let display = display.bounds();
        let display_left = f32::from(display.origin.x);
        let display_top = f32::from(display.origin.y);
        let display_right = display_left + f32::from(display.size.width);
        let display_bottom = display_top + f32::from(display.size.height);
        if right > display_left
            && left < display_right
            && bottom > display_top
            && top < display_bottom
        {
            return bounds;
        }
    }
    centered_bounds(
        cx,
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

#[cfg(test)]
mod tests {
    use super::text_pin_size;

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
}
