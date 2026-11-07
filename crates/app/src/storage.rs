use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use core_types::RgbaFrame;

use crate::settings_state;

const APP_FOLDER: &str = "Capin";

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
    let path = unique_path(&directory, "capin", "png");
    image::save_buffer(
        &path,
        frame.pixels(),
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
    )
    .map_err(std::io::Error::other)?;
    Ok(path)
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
