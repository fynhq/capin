use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use core_annotate::TextAlignment;
use platform_window::{AutostartState, set_autostart};

use crate::i18n::Language;
use crate::logging::{log_error, log_info, log_warn};

#[cfg(target_os = "windows")]
pub const DEFAULT_CAPTURE_HOTKEY: &str = "ctrl+alt+KeyS";
#[cfg(not(target_os = "windows"))]
pub const DEFAULT_CAPTURE_HOTKEY: &str = "ctrl+meta+KeyS";
#[cfg(target_os = "windows")]
pub const DEFAULT_PIN_HOTKEY: &str = "ctrl+alt+KeyD";
#[cfg(not(target_os = "windows"))]
pub const DEFAULT_PIN_HOTKEY: &str = "ctrl+meta+KeyD";

#[cfg(target_os = "windows")]
const LEGACY_CAPTURE_HOTKEY: &str = "shift+meta+KeyS";
#[cfg(target_os = "windows")]
const LEGACY_PIN_HOTKEY: &str = "shift+meta+KeyD";
#[cfg(target_os = "macos")]
const LEGACY_CAPTURE_HOTKEY: &str = "shift+meta+KeyS";
#[cfg(target_os = "macos")]
const LEGACY_PIN_HOTKEY: &str = "shift+meta+KeyD";
pub const DEFAULT_ANNOTATION_COLOR: u32 = 0xFF3B30FF;
pub const DEFAULT_ANNOTATION_FONT_SIZE: f32 = 20.0;
pub const DEFAULT_FILENAME_TEMPLATE: &str = "Capin_{date}_{time}";

pub const ANNOTATION_COLORS: [(u32, &str, &str); 6] = [
    (0xFF3B30FF, "red", "color.red"),
    (0xFFCC00FF, "yellow", "color.yellow"),
    (0x34C759FF, "green", "color.green"),
    (0x0A84FFFF, "blue", "color.blue"),
    (0xFFFFFFFF, "white", "color.white"),
    (0x000000FF, "black", "color.black"),
];

pub const ANNOTATION_COLOR_TOOL_COUNT: usize = 6;

pub const RECORD_HIGHLIGHT_COLORS: [u32; 4] = [0xFFE082, 0x90CAF9, 0xCE93D8, 0xA5D6A7];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationColorTool {
    Rect,
    Ellipse,
    Arrow,
    Highlighter,
    Text,
    Number,
}

impl AnnotationColorTool {
    pub const ALL: [Self; ANNOTATION_COLOR_TOOL_COUNT] = [
        Self::Rect,
        Self::Ellipse,
        Self::Arrow,
        Self::Highlighter,
        Self::Text,
        Self::Number,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Rect => 0,
            Self::Ellipse => 1,
            Self::Arrow => 2,
            Self::Highlighter => 3,
            Self::Text => 4,
            Self::Number => 5,
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
            Self::Arrow => "arrow",
            Self::Highlighter => "highlighter",
            Self::Text => "text",
            Self::Number => "number",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.code() == code)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowBackground {
    #[default]
    Transparent,
    White,
}

impl WindowBackground {
    pub const ALL: [Self; 2] = [Self::Transparent, Self::White];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Transparent => "transparent",
            Self::White => "white",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "transparent" => Some(Self::Transparent),
            "white" | "system" | "light" | "dark" => Some(Self::White),
            _ => None,
        }
    }
}

pub const WINDOW_PADDINGS: [u32; 3] = [24, 48, 96];
pub const DEFAULT_WINDOW_PADDING: u32 = 24;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatPinBehavior {
    #[default]
    RestoreHistory,
    FlashCurrent,
}

impl RepeatPinBehavior {
    pub const ALL: [Self; 2] = [Self::RestoreHistory, Self::FlashCurrent];

    pub const fn code(self) -> &'static str {
        match self {
            Self::RestoreHistory => "history",
            Self::FlashCurrent => "flash",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "history" => Some(Self::RestoreHistory),
            "flash" => Some(Self::FlashCurrent),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DoubleClickAction {
    #[default]
    NoAction,
    Copy,
    Pin,
}

impl DoubleClickAction {
    pub const ALL: [Self; 3] = [Self::NoAction, Self::Copy, Self::Pin];

    pub const fn code(self) -> &'static str {
        match self {
            Self::NoAction => "none",
            Self::Copy => "copy",
            Self::Pin => "pin",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "none" => Some(Self::NoAction),
            "copy" => Some(Self::Copy),
            "pin" => Some(Self::Pin),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub language: Language,
    pub auto_start: bool,
    pub auto_start_explicit: bool,
    pub record_cursor_highlight: bool,
    pub record_cursor_highlight_color: u32,
    pub capture_hotkey: String,
    pub pin_hotkey: String,
    pub repeat_pin: RepeatPinBehavior,
    pub screenshot_dir: Option<PathBuf>,
    pub recording_dir: Option<PathBuf>,
    pub filename_template: String,
    pub window_background: WindowBackground,
    pub window_padding: u32,
    pub capture_cursor: bool,
    pub capture_double_click: DoubleClickAction,
    pub region_corner_radius: u32,
    pub region_shadow: bool,
    pub annotation_color: u32,
    pub annotation_colors: [u32; ANNOTATION_COLOR_TOOL_COUNT],
    pub annotation_uniform_color: bool,
    pub annotation_rect_stroke_width: f32,
    pub annotation_arrow_stroke_width: f32,
    pub annotation_font_size: f32,
    pub annotation_arrow_start: bool,
    pub annotation_arrow_end: bool,
    pub annotation_arrow_polyline: bool,
    pub annotation_text_background: bool,
    pub annotation_text_outline: bool,
    pub annotation_text_alignment: TextAlignment,
    pub annotation_corner_radius: f32,
    pub annotation_highlighter_radius: f32,
    pub annotation_number_size: u32,
    pub annotation_blur_radius: u32,
    pub annotation_rect_filled: bool,
    pub annotation_ellipse_filled: bool,
    pub annotation_rect_ellipse: bool,
    pub annotation_spotlight_intensity: u8,
    pub annotation_spotlight_radius: f32,
    pub pin_text_ocr: bool,
    pub pin_qr: bool,
    pub hide_tray: bool,
}

impl AppSettings {
    pub fn annotation_color_for(&self, tool: AnnotationColorTool) -> u32 {
        if self.annotation_uniform_color {
            self.annotation_color
        } else {
            self.annotation_colors[tool.index()]
        }
    }

    pub fn apply_annotation_color(&mut self, tool: AnnotationColorTool, color: u32) {
        self.annotation_color = color;
        if self.annotation_uniform_color {
            self.annotation_colors = [color; ANNOTATION_COLOR_TOOL_COUNT];
        } else {
            self.annotation_colors[tool.index()] = color;
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: Language::Zh,
            auto_start: false,
            auto_start_explicit: false,
            record_cursor_highlight: false,
            record_cursor_highlight_color: RECORD_HIGHLIGHT_COLORS[0],
            capture_hotkey: DEFAULT_CAPTURE_HOTKEY.to_owned(),
            pin_hotkey: DEFAULT_PIN_HOTKEY.to_owned(),
            repeat_pin: RepeatPinBehavior::RestoreHistory,
            screenshot_dir: None,
            recording_dir: None,
            filename_template: DEFAULT_FILENAME_TEMPLATE.to_owned(),
            window_background: WindowBackground::Transparent,
            window_padding: DEFAULT_WINDOW_PADDING,
            capture_cursor: false,
            capture_double_click: DoubleClickAction::NoAction,
            region_corner_radius: 0,
            region_shadow: false,
            annotation_color: DEFAULT_ANNOTATION_COLOR,
            annotation_colors: [DEFAULT_ANNOTATION_COLOR; ANNOTATION_COLOR_TOOL_COUNT],
            annotation_uniform_color: true,
            annotation_rect_stroke_width: 4.0,
            annotation_arrow_stroke_width: 4.0,
            annotation_font_size: DEFAULT_ANNOTATION_FONT_SIZE,
            annotation_arrow_start: false,
            annotation_arrow_end: true,
            annotation_arrow_polyline: false,
            annotation_text_background: false,
            annotation_text_outline: false,
            annotation_text_alignment: TextAlignment::Left,
            annotation_corner_radius: 0.0,
            annotation_highlighter_radius: 0.0,
            annotation_number_size: 32,
            annotation_blur_radius: 16,
            annotation_rect_filled: false,
            annotation_ellipse_filled: false,
            annotation_rect_ellipse: false,
            annotation_spotlight_intensity: 40,
            annotation_spotlight_radius: 0.0,
            pin_text_ocr: true,
            pin_qr: true,
            hide_tray: false,
        }
    }
}

struct Field {
    key: &'static str,
    write: fn(&AppSettings, &mut String),
    read: fn(&mut AppSettings, &str),
}

static FIELDS: &[Field] = &[
    Field {
        key: "language",
        write: |settings, out| out.push_str(settings.language.code()),
        read: |settings, value| {
            if let Some(language) = Language::from_code(value) {
                settings.language = language;
            }
        },
    },
    Field {
        key: "auto_start",
        write: |settings, out| write_bool(out, settings.auto_start),
        read: |settings, value| settings.auto_start = read_bool_true(value),
    },
    Field {
        key: "auto_start_explicit",
        write: |settings, out| write_bool(out, settings.auto_start_explicit),
        read: |settings, value| settings.auto_start_explicit = read_bool_true(value),
    },
    Field {
        key: "capture_hotkey",
        write: |settings, out| out.push_str(&settings.capture_hotkey),
        read: |settings, value| settings.capture_hotkey = value.to_owned(),
    },
    Field {
        key: "pin_hotkey",
        write: |settings, out| out.push_str(&settings.pin_hotkey),
        read: |settings, value| settings.pin_hotkey = value.to_owned(),
    },
    Field {
        key: "screenshot_dir",
        write: |settings, out| {
            if let Some(path) = &settings.screenshot_dir {
                out.push_str(&path.display().to_string());
            }
        },
        read: |settings, value| settings.screenshot_dir = parse_directory(value),
    },
    Field {
        key: "recording_dir",
        write: |settings, out| {
            if let Some(path) = &settings.recording_dir {
                out.push_str(&path.display().to_string());
            }
        },
        read: |settings, value| settings.recording_dir = parse_directory(value),
    },
    Field {
        key: "filename_template",
        write: |settings, out| {
            out.push_str(&settings.filename_template.replace(['\n', '\r'], " "));
        },
        read: |settings, value| {
            if !value.trim().is_empty() {
                settings.filename_template = value.chars().take(120).collect();
            }
        },
    },
    Field {
        key: "window_background",
        write: |settings, out| out.push_str(settings.window_background.code()),
        read: |settings, value| {
            if let Some(background) = WindowBackground::from_code(value) {
                settings.window_background = background;
            }
        },
    },
    Field {
        key: "window_padding",
        write: |settings, out| write_u32(out, settings.window_padding),
        read: |settings, value| {
            if let Ok(padding) = value.parse::<u32>() {
                settings.window_padding = if WINDOW_PADDINGS.contains(&padding) {
                    padding
                } else {
                    DEFAULT_WINDOW_PADDING
                };
            }
        },
    },
    Field {
        key: "capture_cursor",
        write: |settings, out| write_bool(out, settings.capture_cursor),
        read: |settings, value| settings.capture_cursor = read_bool_not_false(value),
    },
    Field {
        key: "capture_double_click",
        write: |settings, out| out.push_str(settings.capture_double_click.code()),
        read: |settings, value| {
            if let Some(action) = DoubleClickAction::from_code(value) {
                settings.capture_double_click = action;
            }
        },
    },
    Field {
        key: "region_corner_radius",
        write: |settings, out| write_u32(out, settings.region_corner_radius),
        read: |settings, value| {
            if let Ok(radius) = value.parse::<u32>()
                && [0, 8, 16, 24].contains(&radius)
            {
                settings.region_corner_radius = radius;
            }
        },
    },
    Field {
        key: "region_shadow",
        write: |settings, out| write_bool(out, settings.region_shadow),
        read: |settings, value| settings.region_shadow = read_bool_true(value),
    },
    Field {
        key: "annotation_color",
        write: |settings, out| write_color8(out, settings.annotation_color),
        read: |settings, value| {
            if let Some(color) = parse_rgba(value) {
                settings.annotation_color = color;
                settings.annotation_colors = [color; ANNOTATION_COLOR_TOOL_COUNT];
            }
        },
    },
    Field {
        key: "pin_text_ocr",
        write: |settings, out| write_bool(out, settings.pin_text_ocr),
        read: |settings, value| settings.pin_text_ocr = read_bool_not_false(value),
    },
    Field {
        key: "pin_qr",
        write: |settings, out| write_bool(out, settings.pin_qr),
        read: |settings, value| settings.pin_qr = read_bool_not_false(value),
    },
    Field {
        key: "record_cursor_highlight",
        write: |settings, out| write_bool(out, settings.record_cursor_highlight),
        read: |settings, value| settings.record_cursor_highlight = read_bool_not_false(value),
    },
    Field {
        key: "record_cursor_highlight_color",
        write: |settings, out| write_color6(out, settings.record_cursor_highlight_color),
        read: |settings, value| {
            if let Some(color) = parse_rgba(value) {
                settings.record_cursor_highlight_color = color >> 8;
            }
        },
    },
    Field {
        key: "repeat_pin",
        write: |settings, out| out.push_str(settings.repeat_pin.code()),
        read: |settings, value| {
            if let Some(behavior) = RepeatPinBehavior::from_code(value) {
                settings.repeat_pin = behavior;
            }
        },
    },
    Field {
        key: "hide_tray",
        write: |settings, out| write_bool(out, settings.hide_tray),
        read: |settings, value| settings.hide_tray = read_bool_true(value),
    },
    Field {
        key: "annotation_uniform_color",
        write: |settings, out| write_bool(out, settings.annotation_uniform_color),
        read: |settings, value| settings.annotation_uniform_color = read_bool_not_false(value),
    },
    Field {
        key: "annotation_rect_stroke_width",
        write: |settings, out| write_f32(out, settings.annotation_rect_stroke_width),
        read: |settings, value| {
            if let Some(width) = parse_f32_range(value, 1.0, 32.0) {
                settings.annotation_rect_stroke_width = width;
            }
        },
    },
    Field {
        key: "annotation_arrow_stroke_width",
        write: |settings, out| write_f32(out, settings.annotation_arrow_stroke_width),
        read: |settings, value| {
            if let Some(width) = parse_f32_range(value, 1.0, 32.0) {
                settings.annotation_arrow_stroke_width = width;
            }
        },
    },
    Field {
        key: "annotation_font_size",
        write: |settings, out| write_f32(out, settings.annotation_font_size),
        read: |settings, value| {
            if let Some(size) = parse_f32_range(value, 8.0, 96.0) {
                settings.annotation_font_size = size;
            }
        },
    },
    Field {
        key: "annotation_arrow_start",
        write: |settings, out| write_bool(out, settings.annotation_arrow_start),
        read: |settings, value| settings.annotation_arrow_start = read_bool_not_false(value),
    },
    Field {
        key: "annotation_arrow_end",
        write: |settings, out| write_bool(out, settings.annotation_arrow_end),
        read: |settings, value| settings.annotation_arrow_end = read_bool_not_false(value),
    },
    Field {
        key: "annotation_arrow_polyline",
        write: |settings, out| write_bool(out, settings.annotation_arrow_polyline),
        read: |settings, value| settings.annotation_arrow_polyline = read_bool_not_false(value),
    },
    Field {
        key: "annotation_text_background",
        write: |settings, out| write_bool(out, settings.annotation_text_background),
        read: |settings, value| settings.annotation_text_background = read_bool_not_false(value),
    },
    Field {
        key: "annotation_text_outline",
        write: |settings, out| write_bool(out, settings.annotation_text_outline),
        read: |settings, value| settings.annotation_text_outline = read_bool_not_false(value),
    },
    Field {
        key: "annotation_text_alignment",
        write: |settings, out| {
            out.push_str(text_alignment_code(settings.annotation_text_alignment))
        },
        read: |settings, value| {
            if let Some(alignment) = parse_text_alignment(value) {
                settings.annotation_text_alignment = alignment;
            }
        },
    },
    Field {
        key: "annotation_corner_radius",
        write: |settings, out| write_f32(out, settings.annotation_corner_radius),
        read: |settings, value| {
            if let Some(radius) = parse_f32_range(value, 0.0, 64.0) {
                settings.annotation_corner_radius = radius;
            }
        },
    },
    Field {
        key: "annotation_highlighter_radius",
        write: |settings, out| write_f32(out, settings.annotation_highlighter_radius),
        read: |settings, value| {
            if let Some(radius) = parse_f32_range(value, 0.0, 64.0) {
                settings.annotation_highlighter_radius = radius;
            }
        },
    },
    Field {
        key: "annotation_number_size",
        write: |settings, out| write_u32(out, settings.annotation_number_size),
        read: |settings, value| {
            if let Some(size) = parse_u32_range(value, 8, 96) {
                settings.annotation_number_size = size;
            }
        },
    },
    Field {
        key: "annotation_blur_radius",
        write: |settings, out| write_u32(out, settings.annotation_blur_radius),
        read: |settings, value| {
            if let Some(radius) = parse_u32_range(value, 1, 64) {
                settings.annotation_blur_radius = radius;
            }
        },
    },
    Field {
        key: "annotation_rect_filled",
        write: |settings, out| write_bool(out, settings.annotation_rect_filled),
        read: |settings, value| settings.annotation_rect_filled = read_bool_not_false(value),
    },
    Field {
        key: "annotation_ellipse_filled",
        write: |settings, out| write_bool(out, settings.annotation_ellipse_filled),
        read: |settings, value| settings.annotation_ellipse_filled = read_bool_not_false(value),
    },
    Field {
        key: "annotation_rect_ellipse",
        write: |settings, out| write_bool(out, settings.annotation_rect_ellipse),
        read: |settings, value| settings.annotation_rect_ellipse = read_bool_not_false(value),
    },
    Field {
        key: "annotation_spotlight_intensity",
        write: |settings, out| write_u32(out, u32::from(settings.annotation_spotlight_intensity)),
        read: |settings, value| {
            if let Ok(intensity) = value.parse::<u8>() {
                settings.annotation_spotlight_intensity = intensity.min(100);
            }
        },
    },
    Field {
        key: "annotation_spotlight_radius",
        write: |settings, out| write_f32(out, settings.annotation_spotlight_radius),
        read: |settings, value| {
            if let Some(radius) = parse_f32_range(value, 0.0, 64.0) {
                settings.annotation_spotlight_radius = radius;
            }
        },
    },
];

fn write_bool(out: &mut String, value: bool) {
    out.push_str(if value { "true" } else { "false" });
}

fn write_u32(out: &mut String, value: u32) {
    let _ = write!(out, "{value}");
}

fn write_f32(out: &mut String, value: f32) {
    let _ = write!(out, "{value}");
}

fn write_color8(out: &mut String, value: u32) {
    let _ = write!(out, "{value:08x}");
}

fn write_color6(out: &mut String, value: u32) {
    let _ = write!(out, "{:06x}", value & 0x00FF_FFFF);
}

fn read_bool_true(value: &str) -> bool {
    value == "true"
}

fn read_bool_not_false(value: &str) -> bool {
    value != "false"
}

fn parse_f32_range(value: &str, min: f32, max: f32) -> Option<f32> {
    let value = value.parse::<f32>().ok()?;
    value.is_finite().then(|| value.clamp(min, max))
}

fn parse_u32_range(value: &str, min: u32, max: u32) -> Option<u32> {
    Some(value.parse::<u32>().ok()?.clamp(min, max))
}

static SETTINGS: OnceLock<Mutex<Arc<AppSettings>>> = OnceLock::new();
static SAVE_TX: OnceLock<Option<Sender<SaveRequest>>> = OnceLock::new();

enum SaveRequest {
    Text(String),
    Flush(Sender<bool>),
}

pub fn get() -> Arc<AppSettings> {
    SETTINGS
        .get_or_init(|| Mutex::new(Arc::new(load().unwrap_or_default())))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn update(update: impl FnOnce(&mut AppSettings)) {
    let mut slot = SETTINGS
        .get_or_init(|| Mutex::new(Arc::new(load().unwrap_or_default())))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let settings = Arc::make_mut(&mut *slot);
    update(settings);
    persist(settings);
}

pub fn apply_startup_autostart() {
    let settings = get();
    if !settings.auto_start_explicit && !settings.auto_start {
        return;
    }
    match set_autostart(settings.auto_start) {
        Ok(AutostartState::RequiresApproval) => {
            log_warn!("settings", "launch at login requires approval");
        }
        Ok(_) => log_info!("settings", "launch at login applied"),
        Err(error) => log_error!("settings", "failed to apply launch at login: {error}"),
    }
}

fn load() -> Option<AppSettings> {
    let text = std::fs::read_to_string(crate::paths::config_path()?).ok()?;
    Some(parse(&text))
}

fn parse(text: &str) -> AppSettings {
    let mut settings = AppSettings::default();
    let mut legacy_stroke_width = None;
    let mut rect_stroke_width = None;
    let mut arrow_stroke_width = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "annotation_stroke_width" => {
                if let Some(width) = parse_f32_range(value, 1.0, 32.0) {
                    legacy_stroke_width = Some(width);
                }
            }
            "annotation_rect_stroke_width" => {
                if let Some(width) = parse_f32_range(value, 1.0, 32.0) {
                    rect_stroke_width = Some(width);
                }
            }
            "annotation_arrow_stroke_width" => {
                if let Some(width) = parse_f32_range(value, 1.0, 32.0) {
                    arrow_stroke_width = Some(width);
                }
            }
            key if key.starts_with("annotation_color_") => {
                let code = &key["annotation_color_".len()..];
                if let (Some(tool), Some(color)) =
                    (AnnotationColorTool::from_code(code), parse_rgba(value))
                {
                    settings.annotation_colors[tool.index()] = color;
                }
            }
            key => {
                if let Some(field) = FIELDS.iter().find(|field| field.key == key) {
                    (field.read)(&mut settings, value);
                }
            }
        }
    }
    settings.annotation_rect_stroke_width =
        rect_stroke_width.or(legacy_stroke_width).unwrap_or(4.0);
    settings.annotation_arrow_stroke_width =
        arrow_stroke_width.or(legacy_stroke_width).unwrap_or(4.0);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        if settings.capture_hotkey == LEGACY_CAPTURE_HOTKEY {
            settings.capture_hotkey = DEFAULT_CAPTURE_HOTKEY.to_owned();
        }
        if settings.pin_hotkey == LEGACY_PIN_HOTKEY {
            settings.pin_hotkey = DEFAULT_PIN_HOTKEY.to_owned();
        }
    }
    settings
}

fn parse_text_alignment(value: &str) -> Option<TextAlignment> {
    match value {
        "left" => Some(TextAlignment::Left),
        "center" => Some(TextAlignment::Center),
        "right" => Some(TextAlignment::Right),
        _ => None,
    }
}

fn text_alignment_code(value: TextAlignment) -> &'static str {
    match value {
        TextAlignment::Left => "left",
        TextAlignment::Center => "center",
        TextAlignment::Right => "right",
    }
}

fn parse_directory(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn parse_rgba(value: &str) -> Option<u32> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some((value << 8) | 0xFF),
        8 => Some(value),
        _ => None,
    }
}

fn persist(settings: &AppSettings) {
    let text = serialize(settings);
    if let Some(worker) = save_worker()
        && worker.send(SaveRequest::Text(text.clone())).is_ok()
    {
        return;
    }
    if let Err(error) = write_settings(&text) {
        log_error!("settings", "failed to save settings synchronously: {error}");
    }
}

fn flush_pending(
    reply: Option<Sender<bool>>,
    pending: &mut Option<String>,
    written: &mut Option<String>,
    write: &mut impl FnMut(&str) -> std::io::Result<()>,
) {
    let Some(text) = pending.as_ref() else {
        if let Some(reply) = reply {
            let _ = reply.send(true);
        }
        return;
    };
    if written.as_deref() == Some(text.as_str()) {
        *pending = None;
        if let Some(reply) = reply {
            let _ = reply.send(true);
        }
        return;
    }
    match write(text) {
        Ok(()) => {
            *written = Some(text.clone());
            *pending = None;
            if let Some(reply) = reply {
                let _ = reply.send(true);
            }
        }
        Err(_) => {
            if let Some(reply) = reply {
                let _ = reply.send(false);
            }
        }
    }
}

fn save_worker() -> Option<&'static Sender<SaveRequest>> {
    SAVE_TX
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("capin-settings".to_owned())
                .spawn(move || {
                    let mut pending: Option<String> = None;
                    let mut written: Option<String> = None;
                    let mut write = write_settings;
                    loop {
                        match receiver.recv_timeout(Duration::from_millis(300)) {
                            Ok(SaveRequest::Text(text)) => {
                                if written.as_deref() == Some(text.as_str()) {
                                    pending = None;
                                } else if pending.as_deref() != Some(text.as_str()) {
                                    pending = Some(text);
                                }
                            }
                            Ok(SaveRequest::Flush(reply)) => {
                                flush_pending(Some(reply), &mut pending, &mut written, &mut write);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                flush_pending(None, &mut pending, &mut written, &mut write);
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                flush_pending(None, &mut pending, &mut written, &mut write);
                                break;
                            }
                        }
                    }
                });
            match spawned {
                Ok(_) => Some(sender),
                Err(error) => {
                    log_error!("settings", "failed to start the settings writer: {error}");
                    None
                }
            }
        })
        .as_ref()
}

fn write_settings(text: &str) -> std::io::Result<()> {
    let Some(path) = crate::paths::config_path() else {
        return Ok(());
    };
    if let Err(error) = crate::persist::write_atomic(&path, text.as_bytes()) {
        log_error!("settings", "failed to save settings: {error}");
        return Err(error);
    }
    Ok(())
}

pub fn flush() {
    let Some(worker) = SAVE_TX.get().and_then(Option::as_ref) else {
        return;
    };
    let (reply, response) = mpsc::channel();
    if worker.send(SaveRequest::Flush(reply)).is_ok()
        && !matches!(response.recv_timeout(Duration::from_secs(2)), Ok(true))
    {
        log_error!("settings", "failed to flush settings before exit");
    }
}

fn serialize(settings: &AppSettings) -> String {
    let mut text = String::new();
    for field in FIELDS {
        text.push_str(field.key);
        text.push('=');
        (field.write)(settings, &mut text);
        text.push('\n');
    }
    for tool in AnnotationColorTool::ALL {
        let _ = writeln!(
            text,
            "annotation_color_{}={:08x}",
            tool.code(),
            settings.annotation_colors[tool.index()]
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opaque_and_alpha_annotation_colors() {
        assert_eq!(parse_rgba("ff3b30"), Some(0xFF3B30FF));
        assert_eq!(parse_rgba("#0a84ffcc"), Some(0x0A84FFCC));
        assert_eq!(parse_rgba("not-a-color"), None);
    }

    #[test]
    fn annotation_colors_default_to_red_and_unified() {
        let settings = AppSettings::default();
        assert_eq!(settings.annotation_color, DEFAULT_ANNOTATION_COLOR);
        assert!(settings.annotation_uniform_color);
        for tool in AnnotationColorTool::ALL {
            assert_eq!(
                settings.annotation_colors[tool.index()],
                DEFAULT_ANNOTATION_COLOR
            );
        }
        assert_eq!(DEFAULT_ANNOTATION_COLOR, 0xFF3B30FF);
    }

    #[test]
    fn annotation_color_scope_follows_the_uniform_mode() {
        let mut settings = AppSettings {
            annotation_uniform_color: false,
            ..Default::default()
        };
        settings.apply_annotation_color(AnnotationColorTool::Text, 0x0A84FFFF);

        assert_eq!(
            settings.annotation_color_for(AnnotationColorTool::Text),
            0x0A84FFFF
        );
        assert_eq!(
            settings.annotation_color_for(AnnotationColorTool::Rect),
            DEFAULT_ANNOTATION_COLOR
        );

        settings.annotation_uniform_color = true;
        settings.apply_annotation_color(AnnotationColorTool::Arrow, 0xFFCC00FF);
        for tool in AnnotationColorTool::ALL {
            assert_eq!(
                settings.annotation_color_for(tool),
                0xFFCC00FF,
                "unified mode should update {}",
                tool.code()
            );
        }
    }

    #[test]
    fn annotation_options_default_and_parse() {
        let defaults = AppSettings::default();
        assert_eq!(defaults.annotation_rect_stroke_width, 4.0);
        assert_eq!(defaults.annotation_arrow_stroke_width, 4.0);
        assert_eq!(defaults.annotation_font_size, DEFAULT_ANNOTATION_FONT_SIZE);
        assert!(!defaults.annotation_arrow_start);
        assert!(defaults.annotation_arrow_end);
        assert!(!defaults.annotation_arrow_polyline);
        assert!(!defaults.annotation_rect_ellipse);
        assert!(!defaults.annotation_text_background);
        assert!(!defaults.annotation_text_outline);
        assert_eq!(defaults.annotation_text_alignment, TextAlignment::Left);
        assert_eq!(defaults.annotation_corner_radius, 0.0);
        assert_eq!(defaults.annotation_highlighter_radius, 0.0);
        assert_eq!(defaults.annotation_blur_radius, 16);
        assert!(!defaults.annotation_rect_filled);
        assert!(!defaults.annotation_ellipse_filled);
        assert_eq!(defaults.annotation_spotlight_intensity, 40);

        let settings = parse(
            "annotation_stroke_width=8\nannotation_font_size=32\nannotation_arrow_start=true\nannotation_arrow_end=false\nannotation_arrow_polyline=true\nannotation_text_background=true\nannotation_text_outline=true\nannotation_text_alignment=center\nannotation_corner_radius=16\nannotation_highlighter_radius=12\nannotation_blur_radius=32\nannotation_blur_feather=24\nannotation_rect_filled=true\nannotation_ellipse_filled=true\nannotation_rect_ellipse=true\nannotation_spotlight_intensity=80\n",
        );
        assert_eq!(settings.annotation_rect_stroke_width, 8.0);
        assert_eq!(settings.annotation_arrow_stroke_width, 8.0);
        assert_eq!(settings.annotation_font_size, 32.0);
        assert!(settings.annotation_arrow_start);
        assert!(!settings.annotation_arrow_end);
        assert!(settings.annotation_arrow_polyline);
        assert!(settings.annotation_text_background);
        assert!(settings.annotation_text_outline);
        assert_eq!(settings.annotation_text_alignment, TextAlignment::Center);
        assert_eq!(settings.annotation_corner_radius, 16.0);
        assert_eq!(settings.annotation_highlighter_radius, 12.0);
        assert_eq!(settings.annotation_blur_radius, 32);
        assert!(settings.annotation_rect_filled);
        assert!(settings.annotation_ellipse_filled);
        assert!(settings.annotation_rect_ellipse);
        assert_eq!(settings.annotation_spotlight_intensity, 80);

        let per_tool = parse(
            "annotation_stroke_width=8\nannotation_rect_stroke_width=2\nannotation_arrow_stroke_width=12\n",
        );
        assert_eq!(per_tool.annotation_rect_stroke_width, 2.0);
        assert_eq!(per_tool.annotation_arrow_stroke_width, 12.0);
    }

    #[test]
    fn default_settings_round_trip_through_the_field_table() {
        let settings = AppSettings::default();
        assert_eq!(parse(&serialize(&settings)), settings);
    }

    #[test]
    fn serialized_annotation_options_round_trip() {
        let mut settings = AppSettings {
            annotation_uniform_color: false,
            annotation_rect_stroke_width: 2.0,
            annotation_arrow_stroke_width: 8.0,
            annotation_font_size: 32.0,
            annotation_arrow_start: true,
            annotation_arrow_end: false,
            annotation_arrow_polyline: true,
            annotation_text_background: true,
            annotation_text_outline: true,
            annotation_text_alignment: TextAlignment::Center,
            annotation_corner_radius: 16.0,
            annotation_highlighter_radius: 8.0,
            annotation_number_size: 40,
            annotation_blur_radius: 32,
            annotation_rect_filled: true,
            annotation_ellipse_filled: true,
            annotation_rect_ellipse: true,
            annotation_spotlight_intensity: 80,
            annotation_spotlight_radius: 16.0,
            ..Default::default()
        };
        settings.apply_annotation_color(AnnotationColorTool::Rect, 0xFFCC00FF);
        settings.apply_annotation_color(AnnotationColorTool::Text, 0x0A84FFFF);

        assert_eq!(parse(&serialize(&settings)), settings);
    }

    #[test]
    fn region_effects_default_and_round_trip() {
        let defaults = AppSettings::default();
        assert_eq!(defaults.region_corner_radius, 0);
        assert!(!defaults.region_shadow);

        let settings = parse("region_corner_radius=16\nregion_shadow=true\n");
        assert_eq!(settings.region_corner_radius, 16);
        assert!(settings.region_shadow);
        assert_eq!(parse(&serialize(&settings)), settings);

        let invalid = parse("region_corner_radius=10\n");
        assert_eq!(invalid.region_corner_radius, 0);
    }

    #[test]
    fn capture_double_click_defaults_and_round_trips() {
        assert_eq!(
            AppSettings::default().capture_double_click,
            DoubleClickAction::NoAction
        );

        assert_eq!(
            parse("capture_double_click=copy\n").capture_double_click,
            DoubleClickAction::Copy
        );
        assert_eq!(
            parse("capture_double_click=pin\n").capture_double_click,
            DoubleClickAction::Pin
        );
        assert_eq!(
            parse("capture_double_click=bogus\n").capture_double_click,
            DoubleClickAction::NoAction
        );
        assert_eq!(
            parse("capture_double_click=none\n").capture_double_click,
            DoubleClickAction::NoAction
        );

        let settings = AppSettings {
            capture_double_click: DoubleClickAction::Pin,
            ..Default::default()
        };
        assert_eq!(parse(&serialize(&settings)), settings);
    }

    #[test]
    fn window_capture_defaults_and_padding_floor() {
        let defaults = AppSettings::default();
        assert_eq!(defaults.window_background, WindowBackground::Transparent);
        assert_eq!(defaults.window_padding, DEFAULT_WINDOW_PADDING);

        assert_eq!(
            parse("window_background=system\n").window_background,
            WindowBackground::White,
            "legacy non-transparent choices stay non-transparent"
        );
        assert_eq!(
            parse("window_background=white\n").window_background,
            WindowBackground::White
        );
        assert_eq!(
            parse("window_padding=48\n").window_padding,
            48,
            "offered values are kept"
        );
        assert_eq!(
            parse("window_padding=0\n").window_padding,
            DEFAULT_WINDOW_PADDING,
            "zero padding is no longer offered"
        );
    }

    #[test]
    fn serialized_recording_options_round_trip() {
        let settings = AppSettings {
            record_cursor_highlight: true,
            record_cursor_highlight_color: 0x34C759,
            repeat_pin: RepeatPinBehavior::FlashCurrent,
            ..Default::default()
        };
        assert_eq!(parse(&serialize(&settings)), settings);
    }

    #[test]
    fn default_recording_options() {
        let defaults = AppSettings::default();
        assert_eq!(
            defaults.record_cursor_highlight_color,
            RECORD_HIGHLIGHT_COLORS[0]
        );
        assert_eq!(defaults.repeat_pin, RepeatPinBehavior::RestoreHistory);
    }

    #[test]
    fn filename_template_defaults_and_round_trips() {
        let defaults = AppSettings::default();
        assert_eq!(defaults.filename_template, DEFAULT_FILENAME_TEMPLATE);

        let settings = parse("filename_template=Shot_{date}_{width}x{height}\n");
        assert_eq!(settings.filename_template, "Shot_{date}_{width}x{height}");
        assert!(serialize(&settings).contains("filename_template=Shot_{date}_{width}x{height}"));

        let empty = parse("filename_template=\n");
        assert_eq!(empty.filename_template, DEFAULT_FILENAME_TEMPLATE);
    }

    #[test]
    fn parses_legacy_and_per_tool_annotation_colors() {
        let settings = parse("annotation_color=ffcc00ff\nannotation_uniform_color=false\n");
        assert!(!settings.annotation_uniform_color);
        for tool in AnnotationColorTool::ALL {
            assert_eq!(settings.annotation_colors[tool.index()], 0xFFCC00FF);
        }

        let settings = parse(
            "annotation_color=ff3b30ff\nannotation_color_text=0a84ffff\nannotation_color_number=34c759ff\n",
        );
        assert_eq!(
            settings.annotation_colors[AnnotationColorTool::Rect.index()],
            0xFF3B30FF
        );
        assert_eq!(
            settings.annotation_colors[AnnotationColorTool::Text.index()],
            0x0A84FFFF
        );
        assert_eq!(
            settings.annotation_colors[AnnotationColorTool::Number.index()],
            0x34C759FF
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn migrates_the_legacy_windows_hotkeys_independently() {
        let untouched = parse("capture_hotkey=shift+meta+KeyS\npin_hotkey=shift+meta+KeyD\n");
        assert_eq!(untouched.capture_hotkey, DEFAULT_CAPTURE_HOTKEY);
        assert_eq!(untouched.pin_hotkey, DEFAULT_PIN_HOTKEY);

        let mixed = parse("capture_hotkey=control+alt+KeyQ\npin_hotkey=shift+meta+KeyD\n");
        assert_eq!(mixed.capture_hotkey, "control+alt+KeyQ");
        assert_eq!(mixed.pin_hotkey, DEFAULT_PIN_HOTKEY);
    }

    #[test]
    fn failed_flush_keeps_pending_and_succeeds_on_retry() {
        let mut pending = Some("capture_cursor=true\n".to_owned());
        let mut written = None;
        let mut fail = |_: &str| Err(std::io::Error::other("disk full"));
        let (reply, response) = mpsc::channel();
        flush_pending(Some(reply), &mut pending, &mut written, &mut fail);
        assert_eq!(response.recv_timeout(Duration::from_secs(1)), Ok(false));
        assert_eq!(pending.as_deref(), Some("capture_cursor=true\n"));
        assert!(written.is_none());

        let mut success = |_: &str| Ok(());
        let (reply, response) = mpsc::channel();
        flush_pending(Some(reply), &mut pending, &mut written, &mut success);
        assert_eq!(response.recv_timeout(Duration::from_secs(1)), Ok(true));
        assert!(pending.is_none());
        assert_eq!(written.as_deref(), Some("capture_cursor=true\n"));
    }

    #[test]
    fn flushing_an_already_written_value_skips_the_writer() {
        let mut pending = Some("capture_cursor=true\n".to_owned());
        let mut written = pending.clone();
        let mut writes = 0;
        let mut write = |_: &str| {
            writes += 1;
            Ok(())
        };
        let (reply, response) = mpsc::channel();
        flush_pending(Some(reply), &mut pending, &mut written, &mut write);
        assert_eq!(response.recv_timeout(Duration::from_secs(1)), Ok(true));
        assert_eq!(writes, 0);
        assert!(pending.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn migrates_the_legacy_macos_hotkeys_independently() {
        let untouched = parse("capture_hotkey=shift+meta+KeyS\npin_hotkey=shift+meta+KeyD\n");
        assert_eq!(untouched.capture_hotkey, DEFAULT_CAPTURE_HOTKEY);
        assert_eq!(untouched.pin_hotkey, DEFAULT_PIN_HOTKEY);

        let mixed = parse("capture_hotkey=control+meta+KeyQ\npin_hotkey=shift+meta+KeyD\n");
        assert_eq!(mixed.capture_hotkey, "control+meta+KeyQ");
        assert_eq!(mixed.pin_hotkey, DEFAULT_PIN_HOTKEY);
    }
}
