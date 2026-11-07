//! Platform screen recorders for MP4 output.
//!
//! The recorder captures a monitor, a region, or a single window without audio
//! and reports progress through [`Recorder`]. Encoding stays on the platform:
//! Media Foundation on Windows and AVFoundation on macOS.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use core_capture::{CaptureTarget, platform_backend};
use core_record::RecordConfig;
use core_types::{PhysicalSize, RecordFormat};
use thiserror::Error;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Errors returned before a recording thread starts.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RecorderError {
    #[error(transparent)]
    Config(#[from] core_record::RecordError),
    #[error("the recording target no longer exists")]
    TargetMissing,
    #[error("the {} recording format is not available yet", .0.display_name())]
    UnsupportedFormat(RecordFormat),
    #[error("platform recording failed: {0}")]
    Platform(String),
    #[error("screen recording is not supported on this platform")]
    UnsupportedPlatform,
}

/// Shared control block between the UI and the platform recorder.
pub(crate) struct Flags {
    stop: AtomicBool,
    paused: AtomicBool,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
}

#[allow(dead_code)]
impl Flags {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub(crate) fn fail(&self, message: impl Into<String>) {
        if let Ok(mut slot) = self.error.lock()
            && slot.is_none()
        {
            *slot = Some(message.into());
        }
        self.finished.store(true, Ordering::Release);
    }

    pub(crate) fn finish(&self) {
        self.finished.store(true, Ordering::Release);
    }
}

/// A running recording session.
///
/// Dropping the recorder requests a stop; the platform thread still finishes
/// the file in the background.
pub struct Recorder {
    flags: Arc<Flags>,
}

impl Recorder {
    /// Validates the target and starts a platform recorder.
    ///
    /// The returned recorder reports setup and encoding failures through
    /// [`Recorder::error`] once the background thread stops.
    pub fn start(config: RecordConfig, output: PathBuf) -> Result<Self, RecorderError> {
        if config.format != RecordFormat::Mp4 {
            return Err(RecorderError::UnsupportedFormat(config.format));
        }
        let dimensions = resolve_dimensions(&config)?;
        config.validate(dimensions)?;

        let flags = Arc::new(Flags::new());
        let thread_flags = Arc::clone(&flags);
        std::thread::Builder::new()
            .name("capin-record".to_owned())
            .spawn(move || {
                if let Err(error) = platform_start(config, output, Arc::clone(&thread_flags)) {
                    thread_flags.fail(error);
                }
            })
            .map_err(|error| RecorderError::Platform(error.to_string()))?;

        Ok(Self { flags })
    }

    pub fn stop(&self) {
        self.flags.stop.store(true, Ordering::Release);
    }

    pub fn set_paused(&self, paused: bool) {
        self.flags.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.flags.finished.load(Ordering::Acquire)
    }

    pub fn error(&self) -> Option<String> {
        self.flags.error.lock().ok().and_then(|slot| slot.clone())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn resolve_dimensions(config: &RecordConfig) -> Result<PhysicalSize, RecorderError> {
    match config.target {
        CaptureTarget::Region(region) => Ok(region.rect.size),
        CaptureTarget::Monitor(id) => {
            let monitors = platform_backend()
                .monitors()
                .map_err(|error| RecorderError::Platform(error.to_string()))?;
            monitors
                .into_iter()
                .find(|monitor| monitor.id == id)
                .map(|monitor| monitor.bounds.size)
                .ok_or(RecorderError::TargetMissing)
        }
        CaptureTarget::Window(id) => {
            let windows = platform_backend()
                .windows()
                .map_err(|error| RecorderError::Platform(error.to_string()))?;
            windows
                .into_iter()
                .find(|window| window.id == id)
                .map(|window| window.bounds.size)
                .ok_or(RecorderError::TargetMissing)
        }
    }
}

#[cfg(target_os = "windows")]
fn platform_start(config: RecordConfig, output: PathBuf, flags: Arc<Flags>) -> Result<(), String> {
    windows::start(config, output, flags)
}

#[cfg(target_os = "macos")]
fn platform_start(config: RecordConfig, output: PathBuf, flags: Arc<Flags>) -> Result<(), String> {
    macos::start(config, output, flags)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_start(
    _config: RecordConfig,
    _output: PathBuf,
    _flags: Arc<Flags>,
) -> Result<(), String> {
    Err(RecorderError::UnsupportedPlatform.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{CaptureRegion, MonitorId, PhysicalRect};

    #[test]
    fn webm_is_rejected_until_the_encoder_lands() {
        let config = RecordConfig::new(
            CaptureTarget::Monitor(MonitorId::from_raw(1)),
            RecordFormat::WebM,
        );
        let output = std::env::temp_dir().join("capin-webm-test.mp4");
        assert_eq!(
            Recorder::start(config, output).err(),
            Some(RecorderError::UnsupportedFormat(RecordFormat::WebM))
        );
    }

    #[test]
    fn empty_regions_are_rejected_before_starting() {
        let config = RecordConfig::new(
            CaptureTarget::Region(CaptureRegion::new(
                MonitorId::from_raw(1),
                PhysicalRect::new(0, 0, 0, 0),
            )),
            RecordFormat::Mp4,
        );
        let output = std::env::temp_dir().join("capin-empty-test.mp4");
        assert_eq!(
            Recorder::start(config, output).err(),
            Some(RecorderError::Config(
                core_record::RecordError::EmptyDimensions
            ))
        );
    }
}
