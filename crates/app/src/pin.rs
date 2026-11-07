use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use core_types::{PhysicalRect, PhysicalSize, RgbaFrame};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style, set_click_through};
use serde::{Deserialize, Serialize};

use crate::i18n::{Language, t};
use crate::image_util;
use crate::settings_state;
use crate::storage;

const MAX_RESTORED_PINS: usize = 20;
const CARD_WIDTH: f32 = 380.0;
const PIN_MENU_WIDTH: f32 = 180.0;
const PIN_MENU_ITEM_HEIGHT: f32 = 28.0;
const PIN_MENU_PADDING: f32 = 8.0;

thread_local! {
    static PIN_CASCADE: Cell<u32> = const { Cell::new(0) };
    static LAST_CAPTURE: RefCell<Option<RgbaFrame>> = const { RefCell::new(None) };
    static PINS: RefCell<HashMap<u64, PinState>> = RefCell::new(HashMap::new());
    static PIN_MENU: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PinMenuAction {
    Rotate,
    Edit,
    Copy,
    Save,
    CopyQr,
    Ocr,
    Through,
    Open,
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
            Self::Ocr => "pin-menu-ocr",
            Self::Through => "pin-menu-through",
            Self::Open => "pin-menu-open",
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
            Self::Ocr => "pin.ocr",
            Self::Through => "pin.through",
            Self::Open => "pin.open",
            Self::Close => "pin.close",
        }
    }
}

pub fn remember(frame: &RgbaFrame) {
    LAST_CAPTURE.with(|slot| *slot.borrow_mut() = Some(frame.clone()));
}

#[derive(Clone)]
enum PinContent {
    Image {
        frame: RgbaFrame,
        image: Arc<RenderImage>,
        size: PhysicalSize,
    },
    Text(String),
    File(PathBuf),
    Color {
        red: u8,
        green: u8,
        blue: u8,
    },
}

impl PinContent {
    fn from_frame(frame: RgbaFrame) -> Self {
        let size = PhysicalSize::new(frame.width(), frame.height());
        let image = image_util::render_image(&frame);
        Self::Image { frame, image, size }
    }

    fn frame(&self) -> Option<&RgbaFrame> {
        match self {
            Self::Image { frame, .. } => Some(frame),
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
            Self::File(path) => Some(path.display().to_string()),
            Self::Color { red, green, blue } => {
                Some(format!("#{:02x}{:02x}{:02x}", *red, *green, *blue))
            }
        }
    }

    fn kind(&self) -> PinKind {
        match self {
            Self::Image { .. } => PinKind::Image,
            Self::Text(text) => PinKind::Text(text.clone()),
            Self::File(path) => PinKind::File(path.clone()),
            Self::Color { red, green, blue } => PinKind::Color(*red, *green, *blue),
        }
    }

    fn preferred_zoom(&self) -> f32 {
        match self {
            Self::Image { size, .. } => (900.0 / size.width as f32)
                .min(700.0 / size.height as f32)
                .min(1.0),
            _ => 1.0,
        }
    }

    fn size(&self, zoom: f32) -> (f32, f32) {
        match self {
            Self::Image { size, .. } => (size.width as f32 * zoom, size.height as f32 * zoom),
            Self::Text(text) => text_pin_size(text),
            Self::File(_) => (340.0, 92.0),
            Self::Color { .. } => (300.0, 110.0),
        }
    }
}

fn text_pin_size(text: &str) -> (f32, f32) {
    let chars_per_line = 24usize;
    let lines = text
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(chars_per_line))
        .sum::<usize>()
        .max(1);
    let height = (lines as f32 * 20.0 + 30.0).clamp(80.0, 600.0);
    (CARD_WIDTH, height)
}

#[derive(Clone)]
enum PinKind {
    Image,
    Text(String),
    File(PathBuf),
    Color(u8, u8, u8),
}

struct PinState {
    kind: PinKind,
    entity: Entity<PinView>,
    window: AnyWindowHandle,
    image_path: Option<PathBuf>,
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
    path: Option<String>,
    color: Option<String>,
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
                open_image_pin(frame, cx);
                return;
            }
        }
        for entry in item.entries() {
            let ClipboardEntry::ExternalPaths(paths) = entry else {
                continue;
            };
            if let Some(path) = paths.paths().first()
                && path.exists()
            {
                open_pin_content(PinContent::File(path.clone()), cx);
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
            if let Some((red, green, blue)) = parse_color(text) {
                open_pin_content(PinContent::Color { red, green, blue }, cx);
            } else {
                open_pin_content(PinContent::Text(text.to_owned()), cx);
            }
            return;
        }
    }

    if let Some(frame) = LAST_CAPTURE.with(|slot| slot.borrow().clone()) {
        open_image_pin(frame, cx);
    } else {
        eprintln!("capin: no clipboard content or recent capture to pin");
    }
}

pub fn open_frame(frame: RgbaFrame, cx: &mut App) {
    open_image_pin(frame, cx);
}

fn open_image_pin(frame: RgbaFrame, cx: &mut App) {
    remember(&frame);
    open_pin_content(PinContent::from_frame(frame), cx);
}

fn open_pin_content(content: PinContent, cx: &mut App) {
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    let bounds = default_bounds(cx, width, height);
    let _ = open_pin_inner(content, None, bounds, zoom, 1.0, false, None, true, cx);
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
    cx: &mut App,
) -> Option<u64> {
    let id = id.unwrap_or_else(next_pin_id);
    let kind = content.kind();
    let had_existing_image = existing_image.is_some();
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
        kind: WindowKind::PopUp,
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
            eprintln!("capin: failed to style pin window: {error}");
        }
        if click_through {
            let _ = set_click_through(&*window, true);
        }
        cx.new(|cx| PinView::new(id, content_for_view, zoom, opacity, click_through, cx))
    }) {
        Ok((window, entity)) => {
            let state = PinState {
                kind,
                entity,
                window,
                image_path,
                bounds,
                opacity,
                zoom,
                click_through,
            };
            PINS.with(|pins| {
                pins.borrow_mut().insert(id, state);
            });
            if persist {
                save_manifest();
            }
            Some(id)
        }
        Err(error) => {
            eprintln!("capin: failed to open pin window: {error}");
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
            "file" => {
                let Some(path) = entry.path.map(PathBuf::from).filter(|path| path.exists()) else {
                    continue;
                };
                PinContent::File(path)
            }
            "color" => {
                let Some((red, green, blue)) = entry.color.as_deref().and_then(parse_color) else {
                    continue;
                };
                PinContent::Color { red, green, blue }
            }
            _ => continue,
        };
        let bounds = ensure_visible(
            cx,
            Bounds {
                origin: point(px(entry.x), px(entry.y)),
                size: size(px(entry.width), px(entry.height)),
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
            entry.zoom.clamp(0.1, 4.0),
            entry.opacity.clamp(0.2, 1.0),
            entry.click_through,
            existing_image,
            false,
            cx,
        );
    }
    save_manifest();
}

pub fn replace_image(id: u64, frame: &RgbaFrame, cx: &mut App) -> bool {
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
        save_pin_image(id, frame)
    };
    let width = frame.width() as f32 * zoom;
    let height = frame.height() as f32 * zoom;
    entity.update(cx, |view, cx| {
        view.replace_image(frame.clone(), cx);
    });
    let _ = window_handle.update(cx, |_, window, _| {
        window.resize(size(px(width), px(height)));
    });
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.kind = PinKind::Image;
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

fn persist_rotated_image(id: u64, frame: &RgbaFrame, bounds: Bounds<Pixels>) -> bool {
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

pub fn close_windows_except_pins(cx: &mut App, handles: Vec<AnyWindowHandle>) {
    for handle in handles {
        if !is_pin_window(&handle) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
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
                let (kind, image, text, file, color) = match &state.kind {
                    PinKind::Image => {
                        let image = state
                            .image_path
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy().to_string())?;
                        ("image", Some(image), None, None, None)
                    }
                    PinKind::Text(text) => ("text", None, Some(text.clone()), None, None),
                    PinKind::File(path) => {
                        ("file", None, None, Some(path.display().to_string()), None)
                    }
                    PinKind::Color(red, green, blue) => (
                        "color",
                        None,
                        None,
                        None,
                        Some(format!("#{:02x}{:02x}{:02x}", *red, *green, *blue)),
                    ),
                };
                Some(PersistedPin {
                    id: *id,
                    kind: kind.to_owned(),
                    image,
                    text,
                    path: file,
                    color,
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

fn parse_color(text: &str) -> Option<(u8, u8, u8)> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        return match hex.len() {
            3 => {
                let mut chars = hex.chars();
                let red = chars.next()?.to_digit(16)? as u8 * 17;
                let green = chars.next()?.to_digit(16)? as u8 * 17;
                let blue = chars.next()?.to_digit(16)? as u8 * 17;
                Some((red, green, blue))
            }
            6 | 8 => Some((
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            )),
            _ => None,
        };
    }
    let lower = text.to_ascii_lowercase();
    let inner = lower.strip_prefix("rgb(")?.strip_suffix(')')?;
    let mut channels = inner.split(',').map(str::trim);
    let red = channels.next()?.parse::<u8>().ok()?;
    let green = channels.next()?.parse::<u8>().ok()?;
    let blue = channels.next()?.parse::<u8>().ok()?;
    if channels.next().is_some() {
        return None;
    }
    Some((red, green, blue))
}

struct PinTheme {
    card: Rgba,
    text: Rgba,
    muted: Rgba,
    border: Rgba,
    control: Rgba,
}

impl PinTheme {
    fn for_window(window: &Window) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        if dark {
            Self {
                card: rgba(0x1E1E1EF2),
                text: rgb(0xF2F2F2),
                muted: rgba(0xFFFFFF99),
                border: rgba(0xFFFFFF24),
                control: rgba(0xFFFFFF1A),
            }
        } else {
            Self {
                card: rgba(0xFFFFFFF2),
                text: rgb(0x1A1A1A),
                muted: rgba(0x00000088),
                border: rgba(0x0000001A),
                control: rgba(0x0000000F),
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
    ocr: Option<platform_ocr::OcrOutput>,
    selection: Option<(usize, usize)>,
    selecting: bool,
    content_generation: u64,
}

impl PinView {
    fn new(
        id: u64,
        content: PinContent,
        zoom: f32,
        opacity: f32,
        click_through: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            id,
            content,
            zoom,
            opacity,
            click_through,
            notice: None,
            notice_serial: 0,
            ocr: None,
            selection: None,
            selecting: false,
            content_generation: 1,
        };
        view.start_ocr(cx);
        view
    }

    fn start_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.content.frame().cloned() else {
            self.ocr = None;
            return;
        };
        let settings = settings_state::get();
        if !settings.pin_text_ocr && !settings.pin_qr {
            self.ocr = None;
            return;
        }
        let options = platform_ocr::OcrOptions {
            text: settings.pin_text_ocr,
            qr: settings.pin_qr,
        };
        let generation = self.content_generation;
        cx.spawn(async move |this, cx| {
            let output = cx
                .background_executor()
                .spawn(async move { platform_ocr::recognize_with(frame, options) })
                .await;
            this.update(cx, |this, cx| {
                if this.content_generation == generation {
                    this.ocr = Some(output);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
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
            let vertical = if y < top {
                top - y
            } else if y > bottom {
                y - bottom
            } else {
                0.0
            };
            if vertical <= line.rect.size.height as f32 * 0.75 + 2.0 {
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
                    let distance = vertical + horizontal;
                    if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                        best = Some((distance, base + index));
                    }
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
        } else {
            self.copy_selected_text(cx);
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
        self.copy_selected_text(cx);
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
        update_meta(self.id, self.zoom, self.opacity);
        cx.notify();
    }

    fn set_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        self.opacity = opacity.clamp(0.2, 1.0);
        update_meta(self.id, self.zoom, self.opacity);
        cx.notify();
    }

    fn replace_image(&mut self, frame: RgbaFrame, cx: &mut Context<Self>) {
        self.content = PinContent::from_frame(frame);
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

    fn open_file(&mut self, cx: &mut Context<Self>) {
        if let PinContent::File(path) = &self.content {
            cx.open_with_system(path);
        }
    }

    fn copy_image(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.content.frame() else {
            return;
        };
        if image_util::copy_png(frame, cx).is_ok() {
            let language = settings_state::get().language;
            self.show_notice(t(language, "pin.copied").to_owned(), cx);
        }
    }

    fn save_image(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.content.frame() else {
            return;
        };
        let language = settings_state::get().language;
        match image_util::save_png(frame) {
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
        if let Some(frame) = self.content.frame() {
            crate::editor::open_pin_editor(frame.clone(), self.id, cx);
        }
    }

    fn run_ocr(&mut self, cx: &mut Context<Self>) {
        if let Some(frame) = self.content.frame() {
            crate::ocr::start(frame.clone(), cx);
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
                items.push(PinMenuAction::Ocr);
            }
            PinContent::File(_) => {
                items.push(PinMenuAction::Open);
                items.push(PinMenuAction::Copy);
            }
            PinContent::Text(_) | PinContent::Color { .. } => {
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

    fn body(&self, theme: &PinTheme, language: Language) -> Div {
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
            PinContent::File(path) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let parent = path
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default();
                let extension = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("FILE")
                    .to_uppercase();
                div()
                    .size_full()
                    .p(px(12.0))
                    .rounded(px(10.0))
                    .bg(theme.card)
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w(px(40.0))
                            .h(px(40.0))
                            .rounded(px(8.0))
                            .bg(theme.control)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.0))
                            .text_color(theme.muted)
                            .child(extension),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(13.0))
                                    .text_color(theme.text)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(10.0))
                                    .text_color(theme.muted)
                                    .child(parent),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme.muted)
                            .child(t(language, "pin.right_click")),
                    )
            }
            PinContent::Color { red, green, blue } => div()
                .size_full()
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(theme.card)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .w(px(48.0))
                        .h(px(48.0))
                        .rounded(px(8.0))
                        .bg(rgb((u32::from(*red) << 16)
                            | (u32::from(*green) << 8)
                            | u32::from(*blue))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(format!("#{:02x}{:02x}{:02x}", *red, *green, *blue)),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme.muted)
                                .child(format!("rgb({red}, {green}, {blue})")),
                        ),
                ),
        }
    }
}

impl Render for PinView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PinTheme::for_window(window);
        let language = settings_state::get().language;
        let notice = self.notice.clone();
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
                    .bg(rgba(0x0A84FF66))
            })
            .collect();
        let root = div()
            .size_full()
            .id("pin-window")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    crate::pin::close_pin_menu(cx);
                    if event.click_count >= 2 {
                        this.select_word_at(event.position, cx);
                        cx.notify();
                        return;
                    }
                    if !this.begin_text_selection(event.position) {
                        this.selection = None;
                        this.selecting = false;
                        window.start_window_move();
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
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting {
                    this.update_text_selection(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
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
            .children(text_layers);

        root.children(notice.map(|notice| {
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
}

impl Render for PinMenuView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(4.0))
            .rounded(px(9.0))
            .bg(rgba(0x000000F2))
            .border_1()
            .border_color(rgba(0xFFFFFF24))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    forget_pin_menu();
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .children(self.items.iter().map(|(action, label)| {
                let action = *action;
                let label = label.clone();
                let pin_id = self.pin_id;
                div()
                    .id(action.id())
                    .occlude()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(5.0))
                    .text_size(px(12.0))
                    .text_color(rgb(0xFFFFFF))
                    .hover(move |style| style.bg(rgba(0xFFFFFF1A)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(move |_, _, window, cx| {
                        forget_pin_menu();
                        perform_pin_action(pin_id, action, cx);
                        window.remove_window();
                    }))
                    .child(label)
            }))
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
    let height = PIN_MENU_PADDING * 2.0 + PIN_MENU_ITEM_HEIGHT * items.len() as f32;
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
        kind: WindowKind::PopUp,
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
            },
        );
        let view = cx.new(|cx| PinMenuView {
            pin_id,
            items,
            focus_handle: cx.focus_handle(),
        });
        let focus = view.read(cx).focus_handle.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, _)) => {
            PIN_MENU.with(|slot| *slot.borrow_mut() = Some(handle));
        }
        Err(error) => eprintln!("capin: failed to open pin menu: {error}"),
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
        PinMenuAction::Ocr => {
            entity.update(cx, |view, cx| view.run_ocr(cx));
        }
        PinMenuAction::Through => {
            enable_click_through(id, &entity, window_handle, cx);
        }
        PinMenuAction::Open => {
            entity.update(cx, |view, cx| view.open_file(cx));
        }
        PinMenuAction::Close => {
            close_pin(id);
            let _ = window_handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

fn rotate_pin(id: u64, entity: &Entity<PinView>, window_handle: AnyWindowHandle, cx: &mut App) {
    let (frame, zoom) = {
        let view = entity.read(cx);
        (view.content.frame().cloned(), view.zoom)
    };
    let Some(frame) = frame else {
        return;
    };
    let rotated = image_util::rotate_cw(&frame);
    let width = rotated.width() as f32 * zoom;
    let height = rotated.height() as f32 * zoom;
    let Ok(origin) = window_handle.update(cx, |_, window, _| window.bounds().origin) else {
        return;
    };
    let bounds = Bounds {
        origin,
        size: size(px(width), px(height)),
    };
    if !persist_rotated_image(id, &rotated, bounds) {
        entity.update(cx, |view, cx| {
            let language = settings_state::get().language;
            view.show_notice(t(language, "pin.rotate_failed").to_owned(), cx);
        });
        return;
    }
    let _ = window_handle.update(cx, |_, window, _| {
        window.resize(size(px(width), px(height)))
    });
    entity.update(cx, |view, cx| view.replace_image(rotated, cx));
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

fn centered_bounds(cx: &App, width: f32, height: f32) -> Bounds<Pixels> {
    match cx.primary_display() {
        Some(display) => {
            let center = display.bounds().center();
            Bounds {
                origin: point(center.x - px(width / 2.0), center.y - px(height / 2.0)),
                size: size(px(width), px(height)),
            }
        }
        None => Bounds {
            origin: point(px(100.0), px(100.0)),
            size: size(px(width), px(height)),
        },
    }
}
