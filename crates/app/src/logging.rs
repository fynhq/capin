//! Local file logging for field diagnosis.
//!
//! Logs never contain screen contents, window titles, clipboard text, or file
//! paths. One file is written per local calendar day, and files older than
//! seven days are pruned.

use chrono::{Duration, Local, NaiveDate};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock};

const LOG_PREFIX: &str = "capin-";
const LOG_SUFFIX: &str = ".log";
const KEEP_DAYS: i64 = 7;

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
    date: NaiveDate,
    level: Level,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
static INIT: Once = Once::new();

pub fn init() {
    INIT.call_once(|| {
        let level = Level::from_env();
        let date = Local::now().date_naive();
        let directory = log_directory();
        let _ = std::fs::create_dir_all(&directory);
        migrate_legacy_files(&directory, date);
        prune_old_files(&directory, date);
        let file = open_log_file(&directory, date);
        let _ = LOGGER.set(Mutex::new(Logger { file, date, level }));

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

    let now = Local::now();
    let date = now.date_naive();
    if date != logger.date {
        roll_over(&mut logger, date);
    }

    let line = format!(
        "{} {:<5} {}: {}\n",
        now.format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
        level.label(),
        target,
        message
    );
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }

    if let Some(file) = logger.file.as_mut()
        && file.write_all(line.as_bytes()).is_ok()
    {
        let _ = file.flush();
    }
}

/// Directory containing the daily log files. Settings exposes it so field
/// testers can attach the files without hunting through AppData.
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

fn log_file_name(date: NaiveDate) -> String {
    format!("{LOG_PREFIX}{}{LOG_SUFFIX}", date.format("%Y-%m-%d"))
}

fn log_file_date(name: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(
        name.strip_prefix(LOG_PREFIX)?.strip_suffix(LOG_SUFFIX)?,
        "%Y-%m-%d",
    )
    .ok()
}

fn open_log_file(directory: &Path, date: NaiveDate) -> Option<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(log_file_name(date)))
        .ok()
}

fn roll_over(logger: &mut Logger, date: NaiveDate) {
    let directory = log_directory();
    prune_old_files(&directory, date);
    logger.file = open_log_file(&directory, date);
    logger.date = date;
}

fn prune_old_files(directory: &Path, today: NaiveDate) {
    let oldest_kept = today - Duration::days(KEEP_DAYS - 1);
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(date) = name.to_str().and_then(log_file_date) else {
            continue;
        };
        if date < oldest_kept {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Drops the size-based `capin.log` rotation used before daily files.
fn migrate_legacy_files(directory: &Path, today: NaiveDate) {
    let legacy = directory.join("capin.log");
    if legacy.exists() {
        let current = directory.join(log_file_name(today));
        if current.exists() {
            let _ = std::fs::remove_file(&legacy);
        } else {
            let _ = std::fs::rename(&legacy, &current);
        }
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(index) = name
            .to_str()
            .and_then(|name| name.strip_prefix("capin."))
            .and_then(|name| name.strip_suffix(LOG_SUFFIX))
        else {
            continue;
        };
        if !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_directory(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("capin-logging-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn daily_file_names_round_trip() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let name = log_file_name(date);
        assert_eq!(name, "capin-2026-11-09.log");
        assert_eq!(log_file_date(&name), Some(date));
        assert_eq!(log_file_date("capin.log"), None);
        assert_eq!(log_file_date("capin-2026-13-09.log"), None);
    }

    #[test]
    fn prune_keeps_seven_calendar_days() {
        let directory = temp_directory("prune");
        let today = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        for offset in 0..=KEEP_DAYS {
            let date = today - Duration::days(offset);
            std::fs::write(directory.join(log_file_name(date)), b"x").unwrap();
        }
        std::fs::write(directory.join("notes.txt"), b"x").unwrap();

        prune_old_files(&directory, today);

        for offset in 0..KEEP_DAYS {
            let date = today - Duration::days(offset);
            assert!(directory.join(log_file_name(date)).exists());
        }
        let pruned = today - Duration::days(KEEP_DAYS);
        assert!(!directory.join(log_file_name(pruned)).exists());
        assert!(directory.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn legacy_rotation_is_folded_into_today() {
        let directory = temp_directory("legacy");
        let today = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        std::fs::write(directory.join("capin.log"), b"legacy").unwrap();
        for index in 1..=3 {
            std::fs::write(directory.join(format!("capin.{index}.log")), b"old").unwrap();
        }

        migrate_legacy_files(&directory, today);

        let current = directory.join(log_file_name(today));
        assert_eq!(std::fs::read(current).unwrap(), b"legacy");
        assert!(!directory.join("capin.log").exists());
        for index in 1..=3 {
            assert!(!directory.join(format!("capin.{index}.log")).exists());
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
