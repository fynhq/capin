//! Local file logging for field diagnosis.
//!
//! Logs never contain screen contents, window titles, clipboard text, or file
//! paths. The file lives outside the repository and rotates at 2 MiB.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, Once, OnceLock};

const LOG_FILE: &str = "capin.log";
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const KEEP_BACKUPS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
        }
    }

    fn from_env() -> Self {
        match std::env::var("CAPIN_LOG")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "debug" | "trace" => Self::Debug,
            "warn" | "warning" => Self::Warn,
            "error" | "off" => Self::Error,
            _ => Self::Info,
        }
    }
}

struct Logger {
    file: Option<File>,
    bytes: u64,
    level: Level,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
static INIT: Once = Once::new();

pub fn init() {
    INIT.call_once(|| {
        let level = Level::from_env();
        let (file, bytes) = open_log_file();
        let _ = LOGGER.set(Mutex::new(Logger { file, bytes, level }));

        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = if let Some(message) = info.payload().downcast_ref::<&str>() {
                (*message).to_owned()
            } else if let Some(message) = info.payload().downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic payload".to_owned()
            };
            let location = info
                .location()
                .map(|location| format!("{}:{}", location.file(), location.line()))
                .unwrap_or_else(|| "unknown location".to_owned());
            write(Level::Error, "panic", &format!("{payload} at {location}"));
            previous(info);
        }));
    });
}

pub fn write(level: Level, target: &str, message: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let Ok(mut logger) = logger.lock() else {
        return;
    };
    if level > logger.level {
        return;
    }

    let line = format!(
        "{} {:<5} {}: {}\n",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
        level.label(),
        target,
        message
    );
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }

    if logger.bytes >= MAX_BYTES {
        rotate(&mut logger);
    }
    if let Some(file) = logger.file.as_mut()
        && file.write_all(line.as_bytes()).is_ok()
    {
        let _ = file.flush();
        logger.bytes += line.len() as u64;
    }
}

/// Directory containing `capin.log`. Settings exposes it so field testers can
/// attach the file without hunting through AppData.
pub fn log_directory() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("Capin").join("logs")
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Logs")
            .join("Capin")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".local")
            .join("state")
            .join("Capin")
            .join("logs")
    }
}

fn open_log_file() -> (Option<File>, u64) {
    let directory = log_directory();
    if std::fs::create_dir_all(&directory).is_err() {
        return (None, 0);
    }
    let path = directory.join(LOG_FILE);
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= MAX_BYTES) {
        shift_backups(&directory);
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            let bytes = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            (Some(file), bytes)
        }
        Err(_) => (None, 0),
    }
}

fn rotate(logger: &mut Logger) {
    // Drop the handle first: Windows refuses to rename a file that is still
    // open without delete sharing.
    logger.file = None;
    let directory = log_directory();
    shift_backups(&directory);
    let path = directory.join(LOG_FILE);
    logger.file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    logger.bytes = logger
        .file
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .map(|metadata| metadata.len())
        .unwrap_or(0);
}

fn shift_backups(directory: &std::path::Path) {
    let backup = |index: usize| {
        directory.join(if index == 0 {
            LOG_FILE.to_owned()
        } else {
            format!("capin.{index}.log")
        })
    };
    let _ = std::fs::remove_file(backup(KEEP_BACKUPS));
    for index in (1..KEEP_BACKUPS).rev() {
        let _ = std::fs::rename(backup(index), backup(index + 1));
    }
    let _ = std::fs::rename(backup(0), backup(1));
}

macro_rules! log_error {
    ($target:expr, $($arg:tt)*) => {
        $crate::logging::write($crate::logging::Level::Error, $target, &format!($($arg)*))
    };
}

macro_rules! log_warn {
    ($target:expr, $($arg:tt)*) => {
        $crate::logging::write($crate::logging::Level::Warn, $target, &format!($($arg)*))
    };
}

macro_rules! log_info {
    ($target:expr, $($arg:tt)*) => {
        $crate::logging::write($crate::logging::Level::Info, $target, &format!($($arg)*))
    };
}

pub(crate) use {log_error, log_info, log_warn};
