use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use core_record::FrameRate;
use core_types::RecordFormat;
use platform_window::{AutostartState, set_autostart};

use crate::i18n::Language;

pub const DEFAULT_CAPTURE_HOTKEY: &str = "shift+meta+KeyS";
pub const DEFAULT_PIN_HOTKEY: &str = "shift+meta+KeyD";
pub const DEFAULT_ANNOTATION_COLOR: u32 = 0xFF3B30FF;

/// Palette shared by the settings page and the annotation toolbar. The first
/// item is the default color; each entry carries its stable id and i18n key.
pub const ANNOTATION_COLORS: [(u32, &str, &str); 5] = [
    (0xFF3B30FF, "red", "color.red"),
    (0xFFCC00FF, "yellow", "color.yellow"),
    (0x34C759FF, "green", "color.green"),
    (0x0A84FFFF, "blue", "color.blue"),
    (0xFFFFFFFF, "white", "color.white"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowBackground {
    #[default]
    Transparent,
    Light,
    Dark,
    Gradient,
    Wallpaper,
}

impl WindowBackground {
    pub const ALL: [Self; 5] = [
        Self::Transparent,
        Self::Light,
        Self::Dark,
        Self::Gradient,
        Self::Wallpaper,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Transparent => "transparent",
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Gradient => "gradient",
            Self::Wallpaper => "wallpaper",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "transparent" => Some(Self::Transparent),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "gradient" => Some(Self::Gradient),
            "wallpaper" => Some(Self::Wallpaper),
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
    pub record_format: RecordFormat,
    pub record_frame_rate: FrameRate,
    pub record_cursor: bool,
    pub capture_hotkey: String,
    pub pin_hotkey: String,
    pub screenshot_dir: Option<PathBuf>,
    pub recording_dir: Option<PathBuf>,
    pub window_composition: bool,
    pub window_background: WindowBackground,
    pub window_padding: u32,
    pub window_shadow: bool,
    pub window_wallpaper: Option<PathBuf>,
    pub capture_cursor: bool,
    pub annotation_color: u32,
    pub pin_text_ocr: bool,
    pub pin_qr: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: Language::Zh,
            auto_start: false,
            auto_start_explicit: false,
            copy_to_clipboard: true,
            record_format: RecordFormat::Mp4,
            record_frame_rate: FrameRate::Fps30,
            record_cursor: true,
            capture_hotkey: DEFAULT_CAPTURE_HOTKEY.to_owned(),
            pin_hotkey: DEFAULT_PIN_HOTKEY.to_owned(),
            screenshot_dir: None,
            recording_dir: None,
            window_composition: true,
            window_background: WindowBackground::Transparent,
            window_padding: 48,
            window_shadow: true,
            window_wallpaper: None,
            capture_cursor: false,
            annotation_color: DEFAULT_ANNOTATION_COLOR,
            pin_text_ocr: true,
            pin_qr: true,
        }
    }
}

static SETTINGS: OnceLock<Mutex<AppSettings>> = OnceLock::new();

pub fn get() -> AppSettings {
    SETTINGS
        .get_or_init(|| Mutex::new(load().unwrap_or_default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn update(update: impl FnOnce(&mut AppSettings)) {
    let mut settings = SETTINGS
        .get_or_init(|| Mutex::new(load().unwrap_or_default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    update(&mut settings);
    persist(&settings);
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
            eprintln!("capin: launch at login requires approval in system settings");
        }
        Ok(_) => {}
        Err(error) => eprintln!("capin: failed to apply launch at login: {error}"),
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
            "record_format" => match value {
                "webm" => settings.record_format = RecordFormat::WebM,
                "mp4" => settings.record_format = RecordFormat::Mp4,
                _ => {}
            },
            "record_frame_rate" => match value {
                "60" => settings.record_frame_rate = FrameRate::Fps60,
                "30" => settings.record_frame_rate = FrameRate::Fps30,
                _ => {}
            },
            "record_cursor" => settings.record_cursor = value != "false",
            "capture_hotkey" => settings.capture_hotkey = value.to_owned(),
            "pin_hotkey" => settings.pin_hotkey = value.to_owned(),
            "screenshot_dir" => settings.screenshot_dir = parse_directory(value),
            "recording_dir" => settings.recording_dir = parse_directory(value),
            "window_composition" => settings.window_composition = value != "false",
            "window_background" => {
                if let Some(background) = WindowBackground::from_code(value) {
                    settings.window_background = background;
                }
            }
            "window_padding" => {
                if let Ok(padding) = value.parse::<u32>() {
                    settings.window_padding = padding.min(256);
                }
            }
            "window_shadow" => settings.window_shadow = value != "false",
            "window_wallpaper" => settings.window_wallpaper = parse_directory(value),
            "capture_cursor" => settings.capture_cursor = value != "false",
            "annotation_color" => {
                if let Some(color) = parse_rgba(value) {
                    settings.annotation_color = color;
                }
            }
            "pin_text_ocr" => settings.pin_text_ocr = value != "false",
            "pin_qr" => settings.pin_qr = value != "false",
            _ => {}
        }
    }
    settings
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
        eprintln!("capin: failed to create settings directory: {error}");
        return;
    }
    let frame_rate = match settings.record_frame_rate {
        FrameRate::Fps30 => "30",
        FrameRate::Fps60 => "60",
    };
    let format = match settings.record_format {
        RecordFormat::Mp4 => "mp4",
        RecordFormat::WebM => "webm",
    };
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
    let window_wallpaper = settings
        .window_wallpaper
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let text = format!(
        "language={}\nauto_start={}\nauto_start_explicit={}\ncopy_to_clipboard={}\nrecord_format={}\nrecord_frame_rate={}\nrecord_cursor={}\ncapture_hotkey={}\npin_hotkey={}\nscreenshot_dir={}\nrecording_dir={}\nwindow_composition={}\nwindow_background={}\nwindow_padding={}\nwindow_shadow={}\nwindow_wallpaper={}\ncapture_cursor={}\nannotation_color={:08x}\npin_text_ocr={}\npin_qr={}\n",
        settings.language.code(),
        settings.auto_start,
        settings.auto_start_explicit,
        settings.copy_to_clipboard,
        format,
        frame_rate,
        settings.record_cursor,
        settings.capture_hotkey,
        settings.pin_hotkey,
        screenshot_dir,
        recording_dir,
        settings.window_composition,
        settings.window_background.code(),
        settings.window_padding,
        settings.window_shadow,
        window_wallpaper,
        settings.capture_cursor,
        settings.annotation_color,
        settings.pin_text_ocr,
        settings.pin_qr,
    );
    if let Err(error) = std::fs::write(&path, text) {
        eprintln!("capin: failed to save settings: {error}");
    }
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
    fn annotation_color_defaults_to_red() {
        assert_eq!(
            AppSettings::default().annotation_color,
            DEFAULT_ANNOTATION_COLOR
        );
        assert_eq!(DEFAULT_ANNOTATION_COLOR, 0xFF3B30FF);
    }
}
