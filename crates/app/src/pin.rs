use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use core_annotate::AnnotationDocument;
use core_types::{DpiScale, PhysicalRect, PhysicalSize, RgbaFrame};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{
    WindowStyle, apply as apply_window_style, disable_native_shadow, enable_native_shadow,
    hide_native_border, place_on_monitor_deferred, set_click_through, system_accent_hex,
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
    /// Native-shadow choice of the pin that set `LAST_PIN_TRIGGER`, so the
    /// first shortcut press after Esc reopens the same image with the same
    /// floating edge.
    static LAST_PIN_SHADOW: Cell<bool> = const { Cell::new(false) };
    /// Signature of the clipboard image seen when the current pin sequence
    /// started. A changed clipboard starts a fresh sequence; an unchanged one
    /// keeps walking the screenshot history even after the previous pin was
    /// closed.
    static PIN_CLIPBOARD: Cell<Option<u64>> = const { Cell::new(None) };
    static PIN_CLIPBOARD_TEXT: Cell<Option<u64>> = const { Cell::new(None) };
}

fn clipboard_signature_of(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Cheap content signature for the first clipboard image. Bytes are hashed
/// without decoding, so checking whether the clipboard changed costs one read.
fn clipboard_image_signature(cx: &mut App) -> Option<u64> {
    let item = cx.read_from_clipboard()?;
    for entry in item.entries() {
        let ClipboardEntry::Image(image) = entry else {
            continue;
        };
        return Some(clipboard_signature_of(&image.bytes));
    }
    None
}

fn clipboard_text_signature(cx: &mut App) -> Option<u64> {
    let item = cx.read_from_clipboard()?;
    for entry in item.entries() {
        let ClipboardEntry::String(text) = entry else {
            continue;
        };
        let text = text.text.trim_end();
        if text.is_empty() {
            continue;
        }
        return Some(clipboard_signature_of(text.as_bytes()));
    }
    None
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

/// Vector annotations kept alongside a pinned image so reopening the pin
/// editor does not force the user to edit an already-rasterized image.
#[derive(Clone)]
pub(crate) struct PinEditState {
    pub base: Arc<RgbaFrame>,
    pub document: AnnotationDocument,
    pub effects: crate::effects::RegionEffects,
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
    /// Region and long-capture pins have no composed edge shadow, so the
    /// window itself supplies one. Window captures already bake it in.
    native_shadow: bool,
    /// Present after the pin has been edited in this session. Rotating or
    /// replacing the image from another source clears it.
    edit_state: Option<PinEditState>,
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
    #[serde(default)]
    shadow: bool,
}

pub fn pin_last(cx: &mut App) {
    let mut clipboard_frame = None;
    let mut clipboard_signature = None;
    let mut clipboard_text = None;
    let mut clipboard_text_signature = None;
    if let Some(item) = cx.read_from_clipboard() {
        for entry in item.entries() {
            let ClipboardEntry::Image(image) = entry else {
                continue;
            };
            clipboard_signature = Some(clipboard_signature_of(&image.bytes));
            let Ok(image) = image::load_from_memory(&image.bytes) else {
                continue;
            };
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            if let Ok(frame) = RgbaFrame::from_pixels(width, height, rgba.into_raw()) {
                clipboard_frame = Some(frame);
                break;
            }
        }
        if clipboard_frame.is_none() {
            for entry in item.entries() {
                let ClipboardEntry::String(text) = entry else {
                    continue;
                };
                let text = text.text.trim_end();
                if text.is_empty() {
                    continue;
                }
                clipboard_text_signature = Some(clipboard_signature_of(text.as_bytes()));
                clipboard_text = Some(text.to_owned());
                break;
            }
        }
    }

    if let Some(frame) = clipboard_frame {
        let previous_clipboard = PIN_CLIPBOARD.get();
        // A clipboard image that has not changed since the sequence started
        // must not hijack the walk: pressing the shortcut after closing a pin
        // continues upward through the screenshots.
        let clipboard_changed = match (previous_clipboard, clipboard_signature) {
            (Some(previous), Some(current)) => previous != current,
            // No image existed when the sequence started, so a new one is
            // user-provided content rather than part of the walk.
            (None, Some(_)) => true,
            _ => false,
        };
        if LAST_PIN_TRIGGER.with(|slot| slot.get()).is_none() || clipboard_changed {
            pin_image(frame, cx);
            return;
        }
        // The sequence is already walking the history. Even when it is
        // exhausted, reopening the clipboard image would show the image the
        // user just dismissed.
        pin_history_step(cx);
        return;
    }

    if let Some(text) = clipboard_text {
        let text_changed = match (PIN_CLIPBOARD_TEXT.get(), clipboard_text_signature) {
            (Some(previous), Some(current)) => previous != current,
            (None, Some(_)) => true,
            _ => false,
        };
        if LAST_PIN_TRIGGER.with(|slot| slot.get()).is_none() || text_changed {
            open_pin_content(PinContent::Text(text), cx);
            return;
        }
        if pin_history_step(cx) {
            return;
        }
        open_pin_content(PinContent::Text(text), cx);
        return;
    }

    if LAST_PIN_TRIGGER.with(|slot| slot.get()).is_some() {
        pin_history_step(cx);
        return;
    }
    if let Some(frame) = crate::capture_history::latest() {
        pin_image(frame, cx);
    } else {
        log_warn!("pin", "no clipboard content or recent capture to pin");
    }
}

/// Pins the next older screenshot in history. Respects the flash-current
/// repeat preference while that pin is still on screen. Reaching the oldest
/// entry consumes the shortcut without reopening anything, so a walk never
/// shows an image twice.
fn pin_history_step(cx: &mut App) -> bool {
    let Some(current) = LAST_PIN_TRIGGER.with(|slot| slot.get()) else {
        return false;
    };
    // The pin may have just been dismissed with Esc. Reopen that same image
    // first; only the next press walks to an older history entry.
    let live = PINS.with(|pins| {
        pins.borrow()
            .values()
            .any(|state| state.content_hash == Some(current))
    });
    if !live && let Some(frame) = crate::capture_history::find(current) {
        open_pin_content_with_shadow(
            PinContent::from_frame_with_hash(frame, current),
            LAST_PIN_SHADOW.with(|slot| slot.get()),
            cx,
        );
        return true;
    }
    if settings_state::get().repeat_pin == settings_state::RepeatPinBehavior::FlashCurrent
        && flash_existing_pin(current, cx)
    {
        return true;
    }
    let Some(frame) = crate::capture_history::cycle(current) else {
        log_info!("pin", "screenshot history has no older entry");
        return true;
    };
    let hash = crate::capture_history::hash_frame(&frame);
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    if !flash_existing_pin(hash, cx) {
        open_pin_content(PinContent::from_frame_with_hash(frame, hash), cx);
    }
    true
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
            let history_hash = crate::capture_history::hash_frame(&history_frame);
            LAST_PIN_TRIGGER.with(|slot| slot.set(Some(history_hash)));
            if !flash_existing_pin(history_hash, cx) {
                open_pin_content(
                    PinContent::from_frame_with_hash(history_frame, history_hash),
                    cx,
                );
            }
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

pub fn open_frame_with_hash(
    frame: RgbaFrame,
    hash: u64,
    native_shadow: bool,
    cx: &mut App,
) -> Option<u64> {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    let content = PinContent::from_frame_with_hash(frame, hash);
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    let bounds = default_bounds(cx, width, height);
    open_pin_content_at(content, bounds, zoom, true, None, native_shadow, cx)
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
    monitor: Option<core_types::MonitorId>,
    native_shadow: bool,
    cx: &mut App,
) -> Option<u64> {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    let source = monitor.and_then(|id| {
        core_capture::platform_backend()
            .monitors()
            .ok()?
            .into_iter()
            .find(|monitor| monitor.id == id)
    });
    let bounds = source.as_ref().map_or(bounds, |monitor| {
        crate::display::window_bounds_on_monitor(monitor, bounds)
    });
    let display_id = source
        .as_ref()
        .map(|monitor| DisplayId::new(monitor.id.raw()));
    open_pin_content_at(
        PinContent::from_frame_with_hash(frame, hash),
        bounds,
        zoom,
        true,
        display_id,
        native_shadow,
        cx,
    )
}

fn open_image_pin(frame: RgbaFrame, hash: u64, cx: &mut App) {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    open_pin_content(PinContent::from_frame_with_hash(frame, hash), cx);
}

fn open_pin_content(content: PinContent, cx: &mut App) {
    open_pin_content_with_shadow(content, false, cx);
}

fn open_pin_content_with_shadow(content: PinContent, native_shadow: bool, cx: &mut App) {
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    log_info!("pin", "opening pin {}x{}", width.round(), height.round());
    let bounds = default_bounds(cx, width, height);
    open_pin_inner(
        content,
        None,
        bounds,
        zoom,
        1.0,
        false,
        None,
        true,
        false,
        native_shadow,
        None,
        cx,
    );
}

#[allow(clippy::too_many_arguments)]
fn open_pin_content_at(
    content: PinContent,
    bounds: Bounds<Pixels>,
    zoom: f32,
    record_history: bool,
    display_id: Option<DisplayId>,
    native_shadow: bool,
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
        native_shadow,
        display_id,
        cx,
    )
}

fn align_client_to_bounds(window: &Window, bounds: Bounds<Pixels>) {
    #[cfg(target_os = "windows")]
    {
        let scale = window.scale_factor().max(0.1);
        let physical = PhysicalRect::new(
            (f32::from(bounds.origin.x) * scale).round() as i32,
            (f32::from(bounds.origin.y) * scale).round() as i32,
            (f32::from(bounds.size.width) * scale).ceil().max(1.0) as u32,
            (f32::from(bounds.size.height) * scale).ceil().max(1.0) as u32,
        );
        let dpi = DpiScale::new(scale).unwrap_or_default();
        let _ = place_on_monitor_deferred(window, physical, dpi);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, bounds);
    }
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
    native_shadow: bool,
    display_id: Option<DisplayId>,
    cx: &mut App,
) -> Option<u64> {
    let id = id.unwrap_or_else(next_pin_id);
    LAST_PIN_SHADOW.with(|slot| slot.set(native_shadow));
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
    let display_id = display_id.or_else(|| display_for_bounds(cx, bounds));
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
        // Window captures bake their edge shadow into the image and must not
        // get a second system shadow. Region and long-capture pins have no
        // composed shadow, so the window supplies the floating edge.
        if native_shadow {
            enable_native_shadow(&*window);
        } else {
            disable_native_shadow(&*window);
        }
        // Hide the non-client border after the shadow policy: enabling
        // NC rendering can restore DWM's 1px outline, which reads as a white
        // frame around dark captures.
        hide_native_border(&*window);
        // GPUI centre-splits the popup's resize frame when it first places
        // the window. The frame sits on the left, right and bottom only, so
        // that split lifts the client area a few pixels above the requested
        // rect. Re-place the client area once the native frame is final.
        align_client_to_bounds(&*window, bounds);
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
        view.update(cx, |view, cx| {
            view.watch_close(window, cx);
        });
        // The DWM frame settles one turn after the shadow policy changes;
        // repeat the client alignment once the window has laid out.
        window.defer(cx, move |window, _| align_client_to_bounds(window, bounds));
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
                native_shadow,
                edit_state: None,
            };
            PINS.with(|pins| {
                pins.borrow_mut().insert(id, state);
            });
            PIN_CLIPBOARD.with(|slot| slot.set(clipboard_image_signature(cx)));
            PIN_CLIPBOARD_TEXT.with(|slot| slot.set(clipboard_text_signature(cx)));
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
            entry.shadow,
            None,
            cx,
        );
    }
    save_manifest();
}

/// Attaches the vector document to a pin that was just created from the
/// editor, without rewriting or restarting OCR for its flattened image.
pub(crate) fn set_edit_state(id: u64, edit_state: PinEditState) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.edit_state = Some(edit_state);
        }
    });
}

/// Updates the pin image and remembers the vector annotation document so the
/// next edit can select and change existing objects instead of starting from
/// a flattened bitmap.
pub(crate) fn replace_image_with_edit_state(
    id: u64,
    frame: RgbaFrame,
    edit_state: PinEditState,
    cx: &mut App,
) -> bool {
    replace_image_inner(id, frame, Some(edit_state), cx)
}

fn replace_image_inner(
    id: u64,
    frame: RgbaFrame,
    edit_state: Option<PinEditState>,
    cx: &mut App,
) -> bool {
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
            state.edit_state = edit_state;
        }
    });
    save_manifest();
    true
}

pub fn close_pin(id: u64, cx: &mut App) {
    let state = PINS.with(|pins| pins.borrow_mut().remove(&id));
    let Some(state) = state else {
        return;
    };
    state.entity.update(cx, |view, cx| view.release_image(cx));
    let _ = state
        .window
        .update(cx, |_, window, _| window.remove_window());
    if let Some(path) = state.image_path {
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
                    shadow: state.native_shadow,
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

/// Native window drag started from a pin. Screen coordinates are physical so
/// mixed-DPI desktops move the window at the same speed as the cursor.
struct PinDrag {
    cursor_start: (i32, i32),
    window_start: (i32, i32),
}

struct PinView {
    id: u64,
    content: PinContent,
    zoom: f32,
    /// Wheel zoom animates toward this value so a sequence of notches keeps
    /// moving in the same direction instead of restarting from the current
    /// window size.
    zoom_target: f32,
    zoom_serial: u64,
    /// Screen point that stays fixed across a zoom gesture. Captured once per
    /// gesture so frame-placement latency cannot make the pin creep.
    zoom_center: Option<(f32, f32)>,
    opacity: f32,
    click_through: bool,
    notice: Option<String>,
    notice_serial: u64,
    notice_leaving: bool,
    hud: Option<String>,
    hud_serial: u64,
    hud_leaving: bool,
    ocr: Option<platform_ocr::OcrOutput>,
    /// Restored pins start OCR from their saved PNG instead of keeping a
    /// full-frame Arc queued for every restored window.
    ocr_path: Option<PathBuf>,
    selection: Option<(usize, usize)>,
    selecting: bool,
    drag: Option<PinDrag>,
    hover_text: bool,
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
            zoom_target: zoom,
            zoom_serial: 0,
            zoom_center: None,
            opacity,
            click_through,
            ocr_path,
            notice: None,
            notice_serial: 0,
            notice_leaving: false,
            hud: None,
            hud_serial: 0,
            hud_leaving: false,
            ocr: None,
            selection: None,
            selecting: false,
            drag: None,
            hover_text: false,
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
        view.start_ocr(cx);
        view
    }

    /// Pulses a bright outline so a repeated pin shortcut makes the existing
    /// pin easy to find instead of stacking another copy on top.
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
        let block_right = lines
            .iter()
            .map(|line| line.rect.origin.x + line.rect.size.width as i32)
            .max()
            .unwrap_or(0);
        let mut text = String::new();
        let mut base = 0usize;
        let mut previous: Option<(&platform_ocr::OcrLine, bool)> = None;
        for line in lines {
            let length = line.chars.len();
            if end < base || start >= base + length {
                base += length;
                continue;
            }
            let from = start.saturating_sub(base).min(length);
            let to = (end.saturating_sub(base) + 1).min(length);
            let full_line = to == length;
            let separator = match previous {
                // Only a fully selected line can be reflowed into the next
                // one; partial selections keep an explicit line break.
                Some((previous_line, true)) if from == 0 => {
                    platform_ocr::line_join_separator(previous_line, line, block_right)
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
            previous = Some((line, full_line));
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

    /// Applies one animation step around the window centre. Resizing from the
    /// top-left made wheel zoom drift towards the bottom-right; moving the
    /// native frame keeps the image's centre anchored.
    fn apply_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.image_size().is_none() {
            return;
        }
        let zoom = zoom.clamp(0.15, 4.0);
        let (width, height) = self.content.size(zoom);
        let bounds = window.bounds();
        let (center_x, center_y) = self.zoom_center.unwrap_or_else(|| {
            (
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
                f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
            )
        });
        let origin = point(px(center_x - width / 2.0), px(center_y - height / 2.0));
        let scale = window.scale_factor().max(0.1);
        let dpi = DpiScale::new(scale).unwrap_or_default();
        let physical = PhysicalRect::new(
            (f32::from(origin.x) * scale).round() as i32,
            (f32::from(origin.y) * scale).round() as i32,
            (width * scale).ceil().max(1.0) as u32,
            (height * scale).ceil().max(1.0) as u32,
        );
        // Update the content scale before moving the native frame so a
        // repaint driven by the resize already draws the new image.
        self.zoom = zoom;
        if place_on_monitor_deferred(&*window, physical, dpi).is_err() {
            window.resize(size(px(width), px(height)));
        }
        update_bounds_state(
            self.id,
            Bounds {
                origin,
                size: size(px(width), px(height)),
            },
        );
        update_meta(self.id, self.zoom, self.opacity);
        cx.notify();
    }

    fn step_zoom(&mut self, serial: u64, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if serial != self.zoom_serial {
            return true;
        }
        let difference = self.zoom_target - self.zoom;
        if difference.abs() <= 0.001 {
            if (self.zoom_target - self.zoom).abs() > f32::EPSILON {
                self.apply_zoom(self.zoom_target, window, cx);
            }
            self.zoom_center = None;
            schedule_manifest_save(cx);
            return true;
        }
        let next = self.zoom + difference * 0.35;
        self.apply_zoom(next, window, cx);
        false
    }

    fn zoom_toward(&mut self, target: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.image_size().is_none() {
            return;
        }
        self.zoom_target = target.clamp(0.15, 4.0);
        let language = settings_state::get().language;
        let percent = (self.zoom_target * 100.0).round() as i32;
        // Animating a native window resize on Windows presents the stretched
        // old frame before the new image, which reads as the border moving
        // first. One resize per wheel notch keeps both together; the body is
        // also sized from the zoom target so it never waits for the native
        // frame to catch up.
        if cx.reduce_motion() || cfg!(target_os = "windows") {
            self.zoom_center = None;
            let target = self.zoom_target;
            self.apply_zoom(target, window, cx);
            schedule_manifest_save(cx);
            self.show_hud(format!("{} {percent}%", t(language, "pin.zoom_hud")), cx);
            return;
        }
        if self.zoom_center.is_none() {
            let bounds = window.bounds();
            self.zoom_center = Some((
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
                f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
            ));
        }
        self.zoom_serial = self.zoom_serial.wrapping_add(1);
        let serial = self.zoom_serial;
        let handle = window.window_handle();
        self.show_hud(format!("{} {percent}%", t(language, "pin.zoom_hud")), cx);
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
            // GPUI's Windows backend dispatches non-client presses back into
            // the view, so the native move loop never runs. Move the window
            // ourselves from the physical cursor delta instead; GPUI already
            // captured the mouse until button release.
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
        // Any replacement from outside the editor invalidates the vector
        // document; `replace_image_with_edit_state` attaches a fresh one after
        // this call.
        PINS.with(|pins| {
            if let Some(state) = pins.borrow_mut().get_mut(&self.id) {
                state.edit_state = None;
            }
        });
        let previous = std::mem::replace(
            &mut self.content,
            PinContent::from_frame_shared(frame, hash),
        );
        self.zoom_target = self.zoom;
        self.zoom_center = None;
        self.zoom_serial = self.zoom_serial.wrapping_add(1);
        if let PinContent::Image { image, .. } = previous {
            // Replacement runs inside an entity update, where `App::drop_image`
            // cannot see the pin window; defer until the update completes.
            cx.defer(move |cx| cx.drop_image(image, None));
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
        self.notice_leaving = false;
        self.notice_serial = self.notice_serial.wrapping_add(1);
        let serial = self.notice_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2200))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.notice_serial != serial {
                        return false;
                    }
                    this.notice_leaving = true;
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
                if this.notice_serial == serial {
                    this.notice = None;
                    this.notice_leaving = false;
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
        self.hud_leaving = false;
        self.hud_serial = self.hud_serial.wrapping_add(1);
        let serial = self.hud_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.hud_serial != serial {
                        return false;
                    }
                    this.hud_leaving = true;
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
                if this.hud_serial == serial {
                    this.hud = None;
                    this.hud_leaving = false;
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
        let edit_state = PINS.with(|pins| {
            pins.borrow()
                .get(&self.id)
                .and_then(|state| state.edit_state.clone())
        });
        let Some(frame) = edit_state
            .as_ref()
            .map(|state| state.base.clone())
            .or_else(|| self.frame_for_action())
        else {
            return;
        };
        let bounds = PINS.with(|pins| pins.borrow().get(&self.id).map(|state| state.bounds));
        let Some(bounds) = bounds else {
            return;
        };
        if !crate::editor::open_pin_editor_overlay(self.id, frame, bounds, edit_state, cx) {
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
            PinContent::Image { image, .. } => {
                // Size the image from the zoom target instead of the window:
                // the native frame can reach its new size one frame before
                // GPUI relayouts, which made the image visibly trail the
                // border during wheel zoom.
                let (width, height) = self.content.size(self.zoom.max(0.1));
                div()
                    .w(px(width))
                    .h(px(height))
                    .child(img(image.clone()).size_full().object_fit(ObjectFit::Fill))
            }
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
        let notice_serial = self.notice_serial;
        let notice_leaving = self.notice_leaving;
        let hud = self.hud.clone();
        let hud_serial = self.hud_serial;
        let hud_leaving = self.hud_leaving;
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
        let flash = self.flash_active.then(|| {
            let outline = div()
                .absolute()
                .inset_0()
                .border_4()
                .border_color(theme.accent);
            if cx.reduce_motion() {
                outline.opacity(self.flash_alpha).into_any_element()
            } else {
                outline
                    .with_animation(
                        SharedString::from(format!("pin-flash-{}", self.flash_generation)),
                        Animation::new(Duration::from_millis(520)).with_easing(|delta| {
                            let pulse = (delta * std::f32::consts::TAU * 2.0).sin().abs();
                            (pulse * (1.0 - delta) * 1.15).min(1.0)
                        }),
                        |outline, delta| outline.opacity(delta),
                    )
                    .into_any_element()
            }
        });
        let content = div()
            .absolute()
            .inset_0()
            .child(body)
            .children(text_layers)
            .children(flash);
        let cursor = match &self.content {
            PinContent::Image { .. } if self.drag.is_some() => crate::ui::closed_hand_cursor(),
            PinContent::Image { .. } if self.hover_text => CursorStyle::IBeam,
            PinContent::Image { .. } => crate::ui::open_hand_cursor(),
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
                    if this
                        .selection_range()
                        .is_some_and(|(start, end)| end > start)
                    {
                        this.copy_selected_text(cx);
                    } else if matches!(&this.content, PinContent::Image { .. }) {
                        this.copy_image(cx);
                    } else {
                        this.copy_content(cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                if command && key == "s" {
                    if matches!(&this.content, PinContent::Image { .. }) {
                        this.save_image(cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                if command && key == "a" && this.select_all_text(cx) {
                    cx.stop_propagation();
                    return;
                }
                if key == "escape" {
                    if this.click_through {
                        let id = this.id;
                        this.click_through = false;
                        let window_handle =
                            PINS.with(|pins| pins.borrow().get(&id).map(|state| state.window));
                        if let Some(handle) = window_handle {
                            let _ = handle.update(cx, |_, window, _| {
                                let _ = set_click_through(&*window, false);
                            });
                        }
                        set_state_click_through(id, false);
                        crate::tray::refresh_menu();
                        cx.notify();
                    } else {
                        let id = this.id;
                        cx.defer(move |cx| crate::pin::close_pin(id, cx));
                    }
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, _, _, cx| {
                    let id = this.id;
                    cx.defer(move |cx| crate::pin::close_pin(id, cx));
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    crate::pin::close_pin_menu(cx);
                    window.focus(&this.focus_handle, cx);
                    this.hover_text = this.text_index_at(event.position).is_some();
                    if event.click_count >= 2 {
                        this.select_word_at(event.position, cx);
                        cx.notify();
                        return;
                    }
                    let bounds = window.bounds().size;
                    let x = f32::from(event.position.x);
                    let y = f32::from(event.position.y);
                    let width = f32::from(bounds.width);
                    let height = f32::from(bounds.height);
                    let margin = 6.0;
                    let near_edge =
                        x < margin || y < margin || x > width - margin || y > height - margin;
                    let over_text = this.begin_text_selection(event.position);
                    if near_edge || event.modifiers.alt || !over_text {
                        // Dragging text selects it; Alt or an edge drag moves
                        // the pin. Both gestures stay available on text-heavy
                        // images.
                        this.selection = None;
                        this.selecting = false;
                        this.begin_window_drag(window);
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
                    let pin_id = this.id;
                    // Opening a GPUI window inside an input callback re-enters
                    // the App borrow and panics; create the menu after the
                    // event dispatch finishes.
                    cx.defer(move |cx| crate::pin::open_pin_menu(pin_id, position, items, cx));
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.drag.is_some() {
                    this.update_window_drag(window);
                    return;
                }
                if this.selecting {
                    this.update_text_selection(event.position);
                    cx.notify();
                    return;
                }
                let over_text = this.text_index_at(event.position).is_some();
                if over_text != this.hover_text {
                    this.hover_text = over_text;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if this.drag.take().is_some() {
                        update_bounds(this.id, window.bounds());
                        schedule_manifest_save(cx);
                        cx.notify();
                        return;
                    }
                    if this.selecting {
                        this.finish_text_selection(cx);
                    } else {
                        update_bounds(this.id, window.bounds());
                    }
                }),
            )
            .on_mouse_exit(cx.listener(|this, _, window, cx| {
                // A very fast drag can outrun the window; without this the
                // drag state would survive a mouse-up delivered outside and
                // the pin would keep following the cursor afterwards.
                if this.drag.take().is_some() {
                    update_bounds(this.id, window.bounds());
                    schedule_manifest_save(cx);
                    cx.notify();
                }
                if this.selecting {
                    this.finish_text_selection(cx);
                }
                if this.hover_text {
                    this.hover_text = false;
                    cx.notify();
                }
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let lines = matches!(event.delta, ScrollDelta::Lines(_));
                let (delta_x, delta_y) = match event.delta {
                    ScrollDelta::Lines(delta) => (delta.x, delta.y),
                    ScrollDelta::Pixels(delta) => {
                        (f32::from(delta.x) / 20.0, f32::from(delta.y) / 20.0)
                    }
                };
                // Windows translates Shift+wheel into a horizontal scroll with
                // a zero vertical delta, so opacity adjustments must read X.
                let primary = if delta_y.abs() > f32::EPSILON {
                    delta_y
                } else {
                    delta_x
                };
                if event.modifiers.shift || this.content.image_size().is_none() {
                    let step = if lines {
                        primary.signum() * 0.05
                    } else {
                        primary * 0.05
                    };
                    if step != 0.0 {
                        this.set_opacity(this.opacity - step, cx);
                    }
                } else {
                    // One wheel event is one notch. Windows reports the
                    // system's scroll-lines setting (three by default), which
                    // made a single notch jump the zoom by 30%.
                    let step = if lines {
                        delta_y.signum() * 0.10
                    } else {
                        delta_y * 0.10
                    };
                    if step != 0.0 {
                        this.zoom_toward(this.zoom_target + step, window, cx);
                    }
                }
            }))
            .child(content);

        root.children(hud.map(|message| {
            let duration = if hud_leaving {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(120)
            };
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
                .with_animation(
                    SharedString::from(format!("pin-hud-{hud_serial}-{hud_leaving}")),
                    Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |hud, delta| {
                        if hud_leaving {
                            hud.opacity(1.0 - delta)
                        } else {
                            hud.opacity(delta).top(px(6.0 - (1.0 - delta) * 3.0))
                        }
                    },
                )
        }))
        .children(notice.map(|notice| {
            let duration = if notice_leaving {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(120)
            };
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
                        .max_w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(notice),
                )
                .with_animation(
                    SharedString::from(format!("pin-notice-{notice_serial}-{notice_leaving}")),
                    Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |notice, delta| {
                        if notice_leaving {
                            notice.opacity(1.0 - delta)
                        } else {
                            notice.opacity(delta).bottom(px(8.0 - (1.0 - delta) * 4.0))
                        }
                    },
                )
        }))
    }
}

struct PinMenuView {
    pin_id: u64,
    items: Vec<(PinMenuAction, String)>,
    /// Keyboard highlight. `None` until an arrow key is pressed so opening
    /// the menu never paints a selected row.
    selected: Option<usize>,
    focus_handle: FocusHandle,
    _blur_subscription: Option<gpui_kit::Subscription>,
}

impl Render for PinMenuView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PinTheme::for_window(window);
        let pin_id = self.pin_id;
        let actions: Vec<PinMenuAction> = self.items.iter().map(|(action, _)| *action).collect();
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
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        forget_pin_menu();
                        window.remove_window();
                        cx.stop_propagation();
                    }
                    "up" => {
                        this.selected = Some(match this.selected {
                            Some(index) => index.saturating_sub(1),
                            None => actions.len().saturating_sub(1),
                        });
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.selected = Some(match this.selected {
                            Some(index) => (index + 1).min(actions.len().saturating_sub(1)),
                            None => 0,
                        });
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "enter" => {
                        let action = this.selected.and_then(|index| actions.get(index).copied());
                        if let Some(action) = action {
                            forget_pin_menu();
                            window.remove_window();
                            cx.defer(move |cx| perform_pin_action(pin_id, action, cx));
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }));
        for (index, (action, label)) in self.items.iter().enumerate() {
            let action = *action;
            let label = label.clone();
            let selected = self.selected == Some(index);
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
                    .when(selected, |item| item.bg(theme.menu_hover))
                    .hover(move |style| style.bg(theme.menu_hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(move |_, _, window, cx| {
                        forget_pin_menu();
                        window.remove_window();
                        // Edit opens a full-screen editor window; doing that
                        // inside this input callback would re-enter the App.
                        cx.defer(move |cx| perform_pin_action(pin_id, action, cx));
                    }))
                    .child(label),
            );
        }
        crate::ui::enter_fade(menu, "pin-menu-enter", Duration::from_millis(90))
    }
}

pub(crate) fn is_pin_menu_open() -> bool {
    PIN_MENU.with(|slot| slot.borrow().is_some())
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
            selected: None,
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
            close_pin(id, cx);
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
