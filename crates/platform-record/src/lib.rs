use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_record::RecordConfig;
use core_types::{CaptureTarget, PhysicalSize};
use platform_capture::platform_backend;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod macos;
mod mp4;
#[cfg(target_os = "windows")]
mod windows;

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

pub(crate) struct Flags {
    stop: AtomicBool,
    paused: AtomicBool,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
    started: Mutex<Option<Instant>>,
    paused_since: Mutex<Option<Instant>>,
    paused_total: Mutex<Duration>,
}

#[allow(dead_code)]
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

    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
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

pub struct Recorder {
    flags: Arc<Flags>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn active_recorder() -> &'static Mutex<Option<Arc<Flags>>> {
    static ACTIVE: std::sync::OnceLock<Mutex<Option<Arc<Flags>>>> = std::sync::OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(None))
}

fn clear_active(flags: &Arc<Flags>) {
    if let Ok(mut active) = active_recorder().lock()
        && active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, flags))
    {
        *active = None;
    }
}

pub fn stop_active() {
    if let Ok(active) = active_recorder().lock()
        && let Some(flags) = active.as_ref()
    {
        flags.stop.store(true, Ordering::Release);
    }
}

pub fn wait_active(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let target = {
            let Ok(active) = active_recorder().lock() else {
                return false;
            };
            match active.as_ref() {
                Some(flags) => {
                    if flags.is_finished() {
                        Some(Arc::clone(flags))
                    } else {
                        None
                    }
                }
                None => return true,
            }
        };
        if let Some(flags) = target {
            clear_active(&flags);
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Recorder {
    pub fn start(config: RecordConfig, output: PathBuf) -> Result<Self, RecorderError> {
        Self::start_excluding(config, output, &[])
    }

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
        if let Ok(mut active) = active_recorder().lock() {
            *active = Some(Arc::clone(&flags));
        }
        let thread = std::thread::Builder::new()
            .name("capin-record".to_owned())
            .spawn(move || {
                if let Err(error) =
                    platform_start(config, output, temp, excluded, Arc::clone(&thread_flags))
                {
                    let _ = std::fs::remove_file(&cleanup);
                    thread_flags.fail(error);
                }
                clear_active(&thread_flags);
            })
            .map_err(|error| {
                clear_active(&flags);
                RecorderError::Platform(error.to_string())
            })?;

        Ok(Self {
            flags,
            thread: Some(thread),
        })
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

    pub fn elapsed_active(&self) -> Duration {
        self.flags.elapsed_active()
    }

    pub fn is_paused(&self) -> bool {
        self.flags.is_paused()
    }

    pub fn wait_finished(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !self.is_finished() {
            if self
                .thread
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
            {
                break;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if self
            .thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
        self.is_finished()
    }

    pub fn stop_and_wait(&mut self, timeout: Duration) -> bool {
        self.stop();
        self.wait_finished(timeout)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

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
    use std::fs;

    #[test]
    fn part_files_keep_the_output_extension() {
        assert_eq!(
            part_path(Path::new("capin-recording.mp4")),
            Path::new(".capin-recording.part.mp4")
        );
    }

    #[test]
    fn stop_and_wait_joins_the_worker_and_removes_the_part_file() {
        let directory = std::env::temp_dir().join(format!(
            "capin-record-stop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&directory).unwrap();
        let output = directory.join("recording.mp4");
        let temp = part_path(&output);
        let flags = Arc::new(Flags::new());
        flags.mark_started();
        let worker_flags = Arc::clone(&flags);
        let worker_output = output.clone();
        let worker_temp = temp.clone();
        let thread = std::thread::Builder::new()
            .name("capin-record-test".to_owned())
            .spawn(move || {
                fs::write(&worker_temp, b"partial").unwrap();
                while !worker_flags.stop_requested() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                fs::rename(&worker_temp, &worker_output).unwrap();
                worker_flags.finish();
            })
            .unwrap();
        let mut recorder = Recorder {
            flags,
            thread: Some(thread),
        };
        assert!(recorder.stop_and_wait(Duration::from_secs(2)));
        assert!(output.is_file());
        assert!(!temp.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    #[ignore = "requires an interactive desktop and screen recording permission"]
    fn records_a_short_mp4_and_leaves_no_part_file() {
        let Some(monitor) = platform_backend()
            .monitors()
            .ok()
            .and_then(|monitors| monitors.into_iter().next())
        else {
            return;
        };
        let directory = std::env::temp_dir().join(format!(
            "capin-record-integration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&directory).unwrap();
        let output = directory.join("recording.mp4");
        let config = RecordConfig::new(CaptureTarget::Monitor(monitor.id));
        let mut recorder = Recorder::start(config, output.clone()).expect("start recording");
        std::thread::sleep(Duration::from_millis(800));
        assert!(
            recorder.stop_and_wait(Duration::from_secs(20)),
            "recorder did not finalize"
        );
        let metadata = fs::metadata(&output).expect("recording output");
        assert!(metadata.len() > 12);
        assert!(!part_path(&output).exists());
        let bytes = fs::read(&output).expect("read recording output");
        assert_eq!(&bytes[4..8], b"ftyp");
        let _ = fs::remove_dir_all(directory);
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
