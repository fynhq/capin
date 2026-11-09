use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use core_types::RgbaFrame;

use crate::settings_state;

const APP_FOLDER: &str = "Capin";

/// Keeps the lock file open for the lifetime of the process. Two instances
/// would otherwise share `pins.json` and delete each other's pin images.
static INSTANCE_LOCK: OnceLock<Mutex<Option<File>>> = OnceLock::new();

fn instance_lock() -> &'static Mutex<Option<File>> {
    INSTANCE_LOCK.get_or_init(|| Mutex::new(None))
}

pub fn acquire_single_instance() -> bool {
    let Some(directory) = app_data_dir() else {
        return true;
    };
    if ensure_directory(&directory).is_err() {
        return true;
    }
    let path = directory.join("capin.lock");
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
    else {
        return true;
    };
    if file.try_lock().is_err() {
        return false;
    }
    let mut slot = instance_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if slot.is_none() {
        *slot = Some(file);
    }
    true
}

/// Drops the lock before a relaunch. The replacement process starts while
/// this one is still shutting down and would otherwise see the lock held and
/// exit immediately, which made the tray restart a plain quit.
pub fn release_single_instance() {
    if let Some(lock) = INSTANCE_LOCK.get() {
        let mut slot = lock.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(file) = slot.take() {
            drop(file);
        }
    }
}

pub fn screenshot_dir() -> PathBuf {
    settings_state::get()
        .screenshot_dir
        .unwrap_or_else(default_screenshot_dir)
}

pub fn recording_dir() -> PathBuf {
    settings_state::get()
        .recording_dir
        .unwrap_or_else(default_recording_dir)
}

pub fn default_screenshot_dir() -> PathBuf {
    platform_window::pictures_dir()
        .map(|directory| directory.join(APP_FOLDER))
        .unwrap_or_else(|| fallback_dir("Screenshots"))
}

pub fn default_recording_dir() -> PathBuf {
    platform_window::videos_dir()
        .map(|directory| directory.join(APP_FOLDER))
        .unwrap_or_else(|| fallback_dir("Recordings"))
}

pub fn ensure_directory(directory: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(directory)
}

pub fn app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    });
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"));
    base.map(|base| base.join(APP_FOLDER))
}

pub fn pins_dir() -> Option<PathBuf> {
    let directory = app_data_dir()?.join("pins");
    ensure_directory(&directory).ok()?;
    Some(directory)
}

pub fn save_png(frame: &RgbaFrame) -> Result<PathBuf, std::io::Error> {
    let directory = screenshot_dir();
    ensure_directory(&directory)?;
    let template = settings_state::get().filename_template;
    let stem = render_filename(&template, frame.width(), frame.height());
    let (path, mut file) = create_unique_file(&directory, &stem, "png")?;
    if let Err(error) = image::write_buffer_with_format(
        &mut file,
        frame.pixels(),
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    ) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(std::io::Error::other(error));
    }
    Ok(path)
}

/// Expands the user filename template. `{date}` is local `YYYYMMDD`,
/// `{time}` is local `HHMMSS`, and `{width}`/`{height}` are image pixels.
pub fn render_filename(template: &str, width: u32, height: u32) -> String {
    let template = if template.trim().is_empty() {
        crate::settings_state::DEFAULT_FILENAME_TEMPLATE
    } else {
        template
    };
    let now = chrono::Local::now();
    let rendered = template
        .replace("{date}", &now.format("%Y%m%d").to_string())
        .replace("{time}", &now.format("%H%M%S").to_string())
        .replace("{width}", &width.to_string())
        .replace("{height}", &height.to_string());
    sanitize_file_stem(&rendered)
}

fn sanitize_file_stem(stem: &str) -> String {
    let mut cleaned = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let cleaned = cleaned
        .trim()
        .trim_end_matches([' ', '.'])
        .chars()
        .take(120)
        .collect::<String>();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return "Capin".to_owned();
    }
    let upper = cleaned.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper
                .strip_prefix(prefix)
                .and_then(|suffix| suffix.parse::<u8>().ok())
                .is_some_and(|number| (1..=9).contains(&number))
        });
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

fn create_unique_file(
    directory: &Path,
    stem: &str,
    extension: &str,
) -> Result<(PathBuf, File), std::io::Error> {
    for suffix in 0..10_000u32 {
        let name = if suffix == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}_{suffix}.{extension}")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "filename template produced too many collisions",
    ))
}

pub fn recording_path(format: core_types::RecordFormat) -> Result<PathBuf, std::io::Error> {
    let directory = recording_dir();
    ensure_directory(&directory)?;
    Ok(unique_path(
        &directory,
        "capin-recording",
        format.extension(),
    ))
}

fn fallback_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(APP_FOLDER).join(name)
}

fn unique_path(directory: &Path, prefix: &str, extension: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let mut path = directory.join(format!("{prefix}-{millis}.{extension}"));
    let mut suffix = 1u32;
    while path.exists() {
        path = directory.join(format!("{prefix}-{millis}-{suffix}.{extension}"));
        suffix += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_template_expands_tokens_and_dimensions() {
        let rendered = render_filename("Capin_{date}_{time}_{width}x{height}", 1920, 1080);
        assert!(rendered.starts_with("Capin_"));
        assert!(rendered.ends_with("_1920x1080"));
        assert!(!rendered.contains('{'));
        assert!(!rendered.contains('}'));
    }

    #[test]
    fn empty_filename_template_falls_back_to_default() {
        let rendered = render_filename("", 1, 2);
        assert!(rendered.starts_with("Capin_"));
        assert!(!rendered.contains('{'));
    }

    #[test]
    fn filename_stem_is_sanitized_for_windows() {
        assert_eq!(sanitize_file_stem("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize_file_stem("CON"), "_CON");
        assert_eq!(sanitize_file_stem("   "), "Capin");
        assert_eq!(sanitize_file_stem("name. "), "name");
        assert_eq!(sanitize_file_stem(".."), "Capin");
    }
}
