//! Platform screen recorders for MP4 output.
//!
//! The recorder captures a monitor, a region, or a single window without audio
//! and reports progress through [`Recorder`]. MP4 uses the platform encoder
//! (Media Foundation on Windows, AVFoundation on macOS).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_capture::{CaptureTarget, platform_backend};
use core_record::RecordConfig;
use core_types::PhysicalSize;
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
    started: Mutex<Option<Instant>>,
    paused_since: Mutex<Option<Instant>>,
    paused_total: Mutex<Duration>,
}

#[allow(dead_code)] // platform backends only
impl Flags {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            started: Mutex::new(None),
            paused_since: Mutex::new(None),
            paused_total: Mutex::new(Duration::ZERO),
        }
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub(crate) fn mark_started(&self) {
        let now = Instant::now();
        if let Ok(mut started) = self.started.lock() {
            started.get_or_insert(now);
        }
    }

    /// Resets the active clock when the platform stream actually starts.
    /// Without this, setup time before the first frame appeared as a leading
    /// gap in the output file.
    pub(crate) fn restart_started(&self) {
        if let Ok(mut started) = self.started.lock() {
            *started = Some(Instant::now());
        }
        if let Ok(mut total) = self.paused_total.lock() {
            *total = Duration::ZERO;
        }
        if let Ok(mut since) = self.paused_since.lock() {
            *since = None;
        }
    }

    /// Active (non-paused) recording time, independent of whether frames
    /// arrived. This keeps the timer, pause math, and final file duration in
    /// sync on static screens.
    pub(crate) fn elapsed_active(&self) -> Duration {
        let started = self.started.lock().ok().and_then(|started| *started);
        let Some(started) = started else {
            return Duration::ZERO;
        };
        started.elapsed().saturating_sub(self.paused_duration())
    }

    pub(crate) fn paused_duration(&self) -> Duration {
        let mut total = self
            .paused_total
            .lock()
            .map(|total| *total)
            .unwrap_or_default();
        if let Ok(paused_since) = self.paused_since.lock()
            && let Some(since) = *paused_since
        {
            total += since.elapsed();
        }
        total
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        if let Ok(mut since) = self.paused_since.lock() {
            if paused {
                if since.is_none() {
                    *since = Some(Instant::now());
                }
            } else if let Some(started) = since.take()
                && let Ok(mut total) = self.paused_total.lock()
            {
                *total += started.elapsed();
            }
        }
        self.paused.store(paused, Ordering::Relaxed);
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

fn part_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "capin-recording".to_owned());
    let extension = output
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp4");
    output.with_file_name(format!(".{stem}.part.{extension}"))
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
        Self::start_excluding(config, output, &[])
    }

    /// Starts a recorder while excluding extra native window identifiers from
    /// display capture. macOS uses this for the recording bar; Windows already
    /// excludes it through `WDA_EXCLUDEFROMCAPTURE`.
    pub fn start_excluding(
        config: RecordConfig,
        output: PathBuf,
        excluded_windows: &[u64],
    ) -> Result<Self, RecorderError> {
        let dimensions = resolve_dimensions(&config)?;
        config.validate(dimensions)?;

        let flags = Arc::new(Flags::new());
        flags.mark_started();
        let thread_flags = Arc::clone(&flags);
        let temp = part_path(&output);
        let cleanup = temp.clone();
        let excluded = excluded_windows.to_vec();
        std::thread::Builder::new()
            .name("capin-record".to_owned())
            .spawn(move || {
                if let Err(error) =
                    platform_start(config, output, temp, excluded, Arc::clone(&thread_flags))
                {
                    let _ = std::fs::remove_file(&cleanup);
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
        self.flags.set_paused(paused);
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

/// Physical pixel dimensions a target will be recorded at. Callers that need
/// the size before starting (for filename templates) use this instead of
/// duplicating the target lookup.
pub fn target_dimensions(target: CaptureTarget) -> Result<PhysicalSize, RecorderError> {
    resolve_target_dimensions(target)
}

fn resolve_dimensions(config: &RecordConfig) -> Result<PhysicalSize, RecorderError> {
    resolve_target_dimensions(config.target)
}

fn resolve_target_dimensions(target: CaptureTarget) -> Result<PhysicalSize, RecorderError> {
    match target {
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
fn platform_start(
    config: RecordConfig,
    output: PathBuf,
    temp: PathBuf,
    _excluded_windows: Vec<u64>,
    flags: Arc<Flags>,
) -> Result<(), String> {
    windows::start(config, output, temp, flags)
}

#[cfg(target_os = "macos")]
fn platform_start(
    config: RecordConfig,
    output: PathBuf,
    temp: PathBuf,
    excluded_windows: Vec<u64>,
    flags: Arc<Flags>,
) -> Result<(), String> {
    macos::start(config, output, temp, excluded_windows, flags)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_start(
    _config: RecordConfig,
    _output: PathBuf,
    _temp: PathBuf,
    _excluded_windows: Vec<u64>,
    _flags: Arc<Flags>,
) -> Result<(), String> {
    Err(RecorderError::UnsupportedPlatform.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{CaptureRegion, MonitorId, PhysicalRect};

    #[test]
    fn part_files_keep_the_output_extension() {
        assert_eq!(
            part_path(Path::new("capin-recording.mp4")),
            Path::new(".capin-recording.part.mp4")
        );
    }

    #[test]
    fn empty_regions_are_rejected_before_starting() {
        let config = RecordConfig::new(CaptureTarget::Region(CaptureRegion::new(
            MonitorId::from_raw(1),
            PhysicalRect::new(0, 0, 0, 0),
        )));
        let output = std::env::temp_dir().join("capin-empty-test.mp4");
        assert_eq!(
            Recorder::start(config, output).err(),
            Some(RecorderError::Config(
                core_record::RecordError::EmptyDimensions
            ))
        );
    }
}
