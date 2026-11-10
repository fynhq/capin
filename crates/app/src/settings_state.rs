use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use core_annotate::TextAlignment;
use platform_window::{AutostartState, set_autostart};

use crate::i18n::Language;
use crate::logging::{log_error, log_info, log_warn};

// Windows reserves Win+Shift+S for Snipping Tool, so the default has to move
// to a combination RegisterHotKey can actually claim.
#[cfg(target_os = "windows")]
pub const DEFAULT_CAPTURE_HOTKEY: &str = "ctrl+alt+KeyS";
#[cfg(not(target_os = "windows"))]
pub const DEFAULT_CAPTURE_HOTKEY: &str = "shift+meta+KeyS";
#[cfg(target_os = "windows")]
pub const DEFAULT_PIN_HOTKEY: &str = "ctrl+alt+KeyD";
#[cfg(not(target_os = "windows"))]
pub const DEFAULT_PIN_HOTKEY: &str = "shift+meta+KeyD";

#[cfg(target_os = "windows")]
const LEGACY_CAPTURE_HOTKEY: &str = "shift+meta+KeyS";
#[cfg(target_os = "windows")]
const LEGACY_PIN_HOTKEY: &str = "shift+meta+KeyD";
pub const DEFAULT_ANNOTATION_COLOR: u32 = 0xFF3B30FF;
pub const DEFAULT_ANNOTATION_FONT_SIZE: f32 = 20.0;
pub const DEFAULT_FILENAME_TEMPLATE: &str = "Capin_{date}_{time}";

/// Palette used by the annotation toolbar. The first item is the default
/// color; each entry carries its stable id and i18n key.
pub const ANNOTATION_COLORS: [(u32, &str, &str); 6] = [
    (0xFF3B30FF, "red", "color.red"),
    (0xFFCC00FF, "yellow", "color.yellow"),
    (0x34C759FF, "green", "color.green"),
    (0x0A84FFFF, "blue", "color.blue"),
    (0xFFFFFFFF, "white", "color.white"),
    (0x000000FF, "black", "color.black"),
];

pub const ANNOTATION_COLOR_TOOL_COUNT: usize = 6;

/// Color-bearing annotation tools. Shapes that do not use a color
/// (selection, blur, spotlight) are intentionally absent.
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

/// What the pin shortcut does when it is triggered again for an image that is
/// already on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatPinBehavior {
    /// Cycle to an older screenshot in the recent-capture history.
    #[default]
    RestoreHistory,
    /// Flash the existing pin instead of creating another copy.
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

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub language: Language,
    pub auto_start: bool,
    pub auto_start_explicit: bool,
    pub copy_to_clipboard: bool,
    pub record_cursor: bool,
    /// Draws a translucent colored disc under the cursor while recording.
    pub record_cursor_highlight: bool,
    /// Disc color as packed `0xRRGGBB`; opacity is fixed by the recorder.
    pub record_cursor_highlight_color: u32,
    pub capture_hotkey: String,
    pub pin_hotkey: String,
    /// Behavior when the pin shortcut repeats for an image already on screen.
    pub repeat_pin: RepeatPinBehavior,
    pub screenshot_dir: Option<PathBuf>,
    pub recording_dir: Option<PathBuf>,
    pub filename_template: String,
    pub capture_cursor: bool,
    /// Shared color used when `annotation_uniform_color` is enabled.
    pub annotation_color: u32,
    /// Per-tool colors used when `annotation_uniform_color` is disabled.
    pub annotation_colors: [u32; ANNOTATION_COLOR_TOOL_COUNT],
    pub annotation_uniform_color: bool,
    pub annotation_stroke_width: f32,
    pub annotation_font_size: f32,
    pub annotation_arrow_start: bool,
    pub annotation_arrow_end: bool,
    pub annotation_text_background: bool,
    pub annotation_text_outline: bool,
    pub annotation_text_alignment: TextAlignment,
    pub annotation_corner_radius: f32,
    pub annotation_blur_radius: u32,
    pub annotation_rect_filled: bool,
    pub annotation_ellipse_filled: bool,
    pub annotation_spotlight_intensity: u8,
    pub pin_text_ocr: bool,
    pub pin_qr: bool,
}

impl AppSettings {
    /// Resolves the color for a tool under the current mode.
    pub fn annotation_color_for(&self, tool: AnnotationColorTool) -> u32 {
        if self.annotation_uniform_color {
            self.annotation_color
        } else {
            self.annotation_colors[tool.index()]
        }
    }

    /// Applies a toolbar color choice. Unified mode updates every tool;
    /// independent mode updates only `tool`.
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
            copy_to_clipboard: true,
            record_cursor: true,
            record_cursor_highlight: false,
            record_cursor_highlight_color: 0xFFCC00,
            capture_hotkey: DEFAULT_CAPTURE_HOTKEY.to_owned(),
            pin_hotkey: DEFAULT_PIN_HOTKEY.to_owned(),
            repeat_pin: RepeatPinBehavior::RestoreHistory,
            screenshot_dir: None,
            recording_dir: None,
            filename_template: DEFAULT_FILENAME_TEMPLATE.to_owned(),
            capture_cursor: false,
            annotation_color: DEFAULT_ANNOTATION_COLOR,
            annotation_colors: [DEFAULT_ANNOTATION_COLOR; ANNOTATION_COLOR_TOOL_COUNT],
            annotation_uniform_color: true,
            annotation_stroke_width: 4.0,
            annotation_font_size: DEFAULT_ANNOTATION_FONT_SIZE,
            annotation_arrow_start: false,
            annotation_arrow_end: true,
            annotation_text_background: false,
            annotation_text_outline: false,
            annotation_text_alignment: TextAlignment::Left,
            annotation_corner_radius: 0.0,
            annotation_blur_radius: 16,
            annotation_rect_filled: false,
            annotation_ellipse_filled: false,
            annotation_spotlight_intensity: 60,
            pin_text_ocr: true,
            pin_qr: true,
        }
    }
}

static SETTINGS: OnceLock<Mutex<Arc<AppSettings>>> = OnceLock::new();

/// Returns a cheap snapshot of the current settings. Callers that only read
/// fields get an `Arc` clone instead of copying strings and paths on every
/// render.
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

/// Applies an explicit or previously enabled login-startup preference at
/// startup. A default `false` never removes an existing OS login item.
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
    let text = std::fs::read_to_string(config_path()?).ok()?;
    Some(parse(&text))
}

fn parse(text: &str) -> AppSettings {
    let mut settings = AppSettings::default();
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
            "language" => {
                if let Some(language) = Language::from_code(value) {
                    settings.language = language;
                }
            }
            "auto_start" => settings.auto_start = value == "true",
            "auto_start_explicit" => settings.auto_start_explicit = value == "true",
            "copy_to_clipboard" => settings.copy_to_clipboard = value != "false",
            "record_cursor" => settings.record_cursor = value != "false",
            "record_cursor_highlight" => settings.record_cursor_highlight = value != "false",
            "record_cursor_highlight_color" => {
                if let Some(color) = parse_rgba(value) {
                    settings.record_cursor_highlight_color = color >> 8;
                }
            }
            "capture_hotkey" => settings.capture_hotkey = value.to_owned(),
            "pin_hotkey" => settings.pin_hotkey = value.to_owned(),
            "repeat_pin" => {
                if let Some(behavior) = RepeatPinBehavior::from_code(value) {
                    settings.repeat_pin = behavior;
                }
            }
            "screenshot_dir" => settings.screenshot_dir = parse_directory(value),
            "recording_dir" => settings.recording_dir = parse_directory(value),
            "filename_template" => {
                if !value.trim().is_empty() {
                    settings.filename_template = value.chars().take(120).collect();
                }
            }
            "capture_cursor" => settings.capture_cursor = value != "false",
            "annotation_color" => {
                if let Some(color) = parse_rgba(value) {
                    settings.annotation_color = color;
                    // Legacy files stored a single color for every tool.
                    settings.annotation_colors = [color; ANNOTATION_COLOR_TOOL_COUNT];
                }
            }
            "annotation_uniform_color" => {
                settings.annotation_uniform_color = value != "false";
            }
            "annotation_stroke_width" => {
                if let Ok(width) = value.parse::<f32>()
                    && width.is_finite()
                {
                    settings.annotation_stroke_width = width.clamp(1.0, 32.0);
                }
            }
            "annotation_font_size" => {
                if let Ok(size) = value.parse::<f32>()
                    && size.is_finite()
                {
                    settings.annotation_font_size = size.clamp(8.0, 96.0);
                }
            }
            "annotation_arrow_start" => settings.annotation_arrow_start = value != "false",
            "annotation_arrow_end" => settings.annotation_arrow_end = value != "false",
            "annotation_text_background" => settings.annotation_text_background = value != "false",
            "annotation_text_outline" => settings.annotation_text_outline = value != "false",
            "annotation_text_alignment" => {
                if let Some(alignment) = parse_text_alignment(value) {
                    settings.annotation_text_alignment = alignment;
                }
            }
            "annotation_corner_radius" => {
                if let Ok(radius) = value.parse::<f32>()
                    && radius.is_finite()
                {
                    settings.annotation_corner_radius = radius.clamp(0.0, 64.0);
                }
            }
            "annotation_blur_radius" => {
                if let Ok(radius) = value.parse::<u32>() {
                    settings.annotation_blur_radius = radius.clamp(1, 64);
                }
            }
            "annotation_rect_filled" => settings.annotation_rect_filled = value != "false",
            "annotation_ellipse_filled" => settings.annotation_ellipse_filled = value != "false",
            "annotation_spotlight_intensity" => {
                if let Ok(intensity) = value.parse::<u8>() {
                    settings.annotation_spotlight_intensity = intensity.min(100);
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
            "pin_text_ocr" => settings.pin_text_ocr = value != "false",
            "pin_qr" => settings.pin_qr = value != "false",
            _ => {}
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Older Windows builds persisted the Win+Shift combinations, which
        // RegisterHotKey can never claim while Snipping Tool owns them.
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

/// Parses `RRGGBB` or `RRGGBBAA`; six-digit values are opaque.
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
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        log_error!("settings", "failed to create settings directory: {error}");
        return;
    }
    let text = serialize(settings);
    if let Err(error) = std::fs::write(&path, text) {
        log_error!("settings", "failed to save settings: {error}");
    }
}

fn serialize(settings: &AppSettings) -> String {
    let screenshot_dir = settings
        .screenshot_dir
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let recording_dir = settings
        .recording_dir
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let mut text = format!(
        "language={}\nauto_start={}\nauto_start_explicit={}\ncopy_to_clipboard={}\nrecord_cursor={}\ncapture_hotkey={}\npin_hotkey={}\nscreenshot_dir={}\nrecording_dir={}\nfilename_template={}\ncapture_cursor={}\nannotation_color={:08x}\npin_text_ocr={}\npin_qr={}\n",
        settings.language.code(),
        settings.auto_start,
        settings.auto_start_explicit,
        settings.copy_to_clipboard,
        settings.record_cursor,
        settings.capture_hotkey,
        settings.pin_hotkey,
        screenshot_dir,
        recording_dir,
        settings.filename_template.replace(['\n', '\r'], " "),
        settings.capture_cursor,
        settings.annotation_color,
        settings.pin_text_ocr,
        settings.pin_qr,
    );
    text.push_str(&format!(
        "record_cursor_highlight={}\nrecord_cursor_highlight_color={:06x}\nrepeat_pin={}\n",
        settings.record_cursor_highlight,
        settings.record_cursor_highlight_color & 0x00FF_FFFF,
        settings.repeat_pin.code(),
    ));
    text.push_str(&format!(
        "annotation_uniform_color={}\nannotation_stroke_width={}\nannotation_font_size={}\nannotation_arrow_start={}\nannotation_arrow_end={}\nannotation_text_background={}\nannotation_text_outline={}\nannotation_text_alignment={}\nannotation_corner_radius={}\nannotation_blur_radius={}\nannotation_rect_filled={}\nannotation_ellipse_filled={}\nannotation_spotlight_intensity={}\n",
        settings.annotation_uniform_color,
        settings.annotation_stroke_width,
        settings.annotation_font_size,
        settings.annotation_arrow_start,
        settings.annotation_arrow_end,
        settings.annotation_text_background,
        settings.annotation_text_outline,
        text_alignment_code(settings.annotation_text_alignment),
        settings.annotation_corner_radius,
        settings.annotation_blur_radius,
        settings.annotation_rect_filled,
        settings.annotation_ellipse_filled,
        settings.annotation_spotlight_intensity,
    ));
    for tool in AnnotationColorTool::ALL {
        text.push_str(&format!(
            "annotation_color_{}={:08x}\n",
            tool.code(),
            settings.annotation_colors[tool.index()]
        ));
    }
    text
}

#[cfg(target_os = "windows")]
fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("Capin").join("settings.conf"))
}

#[cfg(target_os = "macos")]
fn config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|base| {
        PathBuf::from(base)
            .join("Library")
            .join("Application Support")
            .join("Capin")
            .join("settings.conf")
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|base| {
        PathBuf::from(base)
            .join(".config")
            .join("capin")
            .join("settings.conf")
    })
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
        assert_eq!(defaults.annotation_stroke_width, 4.0);
        assert_eq!(defaults.annotation_font_size, DEFAULT_ANNOTATION_FONT_SIZE);
        assert!(!defaults.annotation_arrow_start);
        assert!(defaults.annotation_arrow_end);
        assert!(!defaults.annotation_text_background);
        assert!(!defaults.annotation_text_outline);
        assert_eq!(defaults.annotation_text_alignment, TextAlignment::Left);
        assert_eq!(defaults.annotation_corner_radius, 0.0);
        assert_eq!(defaults.annotation_blur_radius, 16);
        assert!(!defaults.annotation_rect_filled);
        assert!(!defaults.annotation_ellipse_filled);
        assert_eq!(defaults.annotation_spotlight_intensity, 60);

        let settings = parse(
            "annotation_stroke_width=8\nannotation_font_size=32\nannotation_arrow_start=true\nannotation_arrow_end=false\nannotation_text_background=true\nannotation_text_outline=true\nannotation_text_alignment=center\nannotation_corner_radius=16\nannotation_blur_radius=32\nannotation_rect_filled=true\nannotation_ellipse_filled=true\nannotation_spotlight_intensity=80\n",
        );
        assert_eq!(settings.annotation_stroke_width, 8.0);
        assert_eq!(settings.annotation_font_size, 32.0);
        assert!(settings.annotation_arrow_start);
        assert!(!settings.annotation_arrow_end);
        assert!(settings.annotation_text_background);
        assert!(settings.annotation_text_outline);
        assert_eq!(settings.annotation_text_alignment, TextAlignment::Center);
        assert_eq!(settings.annotation_corner_radius, 16.0);
        assert_eq!(settings.annotation_blur_radius, 32);
        assert!(settings.annotation_rect_filled);
        assert!(settings.annotation_ellipse_filled);
        assert_eq!(settings.annotation_spotlight_intensity, 80);
    }

    #[test]
    fn serialized_annotation_options_round_trip() {
        let mut settings = AppSettings {
            annotation_uniform_color: false,
            annotation_stroke_width: 8.0,
            annotation_font_size: 32.0,
            annotation_arrow_start: true,
            annotation_arrow_end: false,
            annotation_text_background: true,
            annotation_text_outline: true,
            annotation_text_alignment: TextAlignment::Center,
            annotation_corner_radius: 16.0,
            annotation_blur_radius: 32,
            annotation_rect_filled: true,
            annotation_ellipse_filled: true,
            annotation_spotlight_intensity: 80,
            ..Default::default()
        };
        settings.apply_annotation_color(AnnotationColorTool::Rect, 0xFFCC00FF);
        settings.apply_annotation_color(AnnotationColorTool::Text, 0x0A84FFFF);

        assert_eq!(parse(&serialize(&settings)), settings);
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
        assert_eq!(defaults.record_cursor_highlight_color, 0xFFCC00);
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
}
