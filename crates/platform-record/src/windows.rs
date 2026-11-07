use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use core_capture::CaptureTarget;
use core_record::RecordConfig;
use core_types::PhysicalRect;
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings as CaptureSettings,
};
use windows_capture::window::Window;

use crate::Flags;

/// What the WGC session should do with each captured frame.
#[derive(Clone, Copy, Debug)]
enum Source {
    Monitor,
    Region(PhysicalRect),
    Window,
}

#[derive(Clone)]
struct CaptureInfo {
    source: Source,
    output: PathBuf,
    fps: u32,
    include_cursor: bool,
    shared: Arc<Flags>,
}

pub(crate) fn start(
    config: RecordConfig,
    output: PathBuf,
    shared: Arc<Flags>,
) -> Result<(), String> {
    if let CaptureTarget::Region(region) = config.target
        && (region.rect.size.width == 0 || region.rect.size.height == 0)
    {
        return Err("empty recording region".to_owned());
    }

    let fps = config.frame_rate.as_u32();
    match config.target {
        CaptureTarget::Region(region) => {
            let monitor = find_monitor(region.monitor.raw())?;
            let info = CaptureInfo {
                source: Source::Region(region.rect),
                output,
                fps,
                include_cursor: config.include_cursor,
                shared,
            };
            spawn_monitor(monitor, info)
        }
        CaptureTarget::Monitor(id) => {
            let monitor = find_monitor(id.raw())?;
            let info = CaptureInfo {
                source: Source::Monitor,
                output,
                fps,
                include_cursor: config.include_cursor,
                shared,
            };
            spawn_monitor(monitor, info)
        }
        CaptureTarget::Window(id) => {
            let raw = id.raw() as usize as *mut std::ffi::c_void;
            let window = Window::from_raw_hwnd(raw);
            let info = CaptureInfo {
                source: Source::Window,
                output,
                fps,
                include_cursor: config.include_cursor,
                shared,
            };
            spawn_window(window, info)
        }
    }
}

fn find_monitor(raw: u64) -> Result<Monitor, String> {
    Monitor::enumerate()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|monitor| monitor.as_raw_hmonitor() as usize as u64 == raw)
        .or_else(|| Monitor::primary().ok())
        .ok_or_else(|| "monitor not found".to_owned())
}

fn spawn_monitor(monitor: Monitor, info: CaptureInfo) -> Result<(), String> {
    spawn(move || {
        let shared = Arc::clone(&info.shared);
        let cursor = cursor_settings(info.include_cursor);
        let settings = CaptureSettings::new(
            monitor,
            cursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            interval(info.fps),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            info,
        );
        if let Err(error) = Recorder::start(settings) {
            shared.fail(error.to_string());
            eprintln!("capin: recording stopped with an error: {error}");
        }
    })
}

fn spawn_window(window: Window, info: CaptureInfo) -> Result<(), String> {
    spawn(move || {
        let shared = Arc::clone(&info.shared);
        let cursor = cursor_settings(info.include_cursor);
        let settings = CaptureSettings::new(
            window,
            cursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            interval(info.fps),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            info,
        );
        if let Err(error) = Recorder::start(settings) {
            shared.fail(error.to_string());
            eprintln!("capin: recording stopped with an error: {error}");
        }
    })
}

fn spawn(run: impl FnOnce() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name("capin-record-wgc".to_owned())
        .spawn(run)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn interval(fps: u32) -> MinimumUpdateIntervalSettings {
    MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1000 / u64::from(fps.max(1))))
}

fn cursor_settings(include_cursor: bool) -> CursorCaptureSettings {
    if include_cursor {
        CursorCaptureSettings::WithCursor
    } else {
        CursorCaptureSettings::WithoutCursor
    }
}

struct Recorder {
    encoder: Option<VideoEncoder>,
    output_size: Option<(u32, u32)>,
    frame_bytes: Vec<u8>,
    flags: CaptureInfo,
    watching: bool,
    last_frame: Option<Instant>,
    pause_started: Option<Instant>,
    paused_total: Duration,
}

impl GraphicsCaptureApiHandler for Recorder {
    type Flags = CaptureInfo;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            encoder: None,
            output_size: None,
            frame_bytes: Vec::new(),
            flags: ctx.flags,
            watching: false,
            last_frame: None,
            pause_started: None,
            paused_total: Duration::ZERO,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.watch_stop(capture_control);

        if self.flags.shared.stop_requested() {
            return Ok(());
        }
        if self.flags.shared.is_paused() {
            if self.pause_started.is_none() {
                self.pause_started = Some(Instant::now());
            }
            return Ok(());
        }
        if let Some(started) = self.pause_started.take() {
            self.paused_total += started.elapsed();
        }

        let interval = Duration::from_millis(1000 / u64::from(self.flags.fps.max(1)));
        if self
            .last_frame
            .is_some_and(|last| last.elapsed() < interval.saturating_sub(Duration::from_millis(2)))
        {
            return Ok(());
        }
        self.last_frame = Some(Instant::now());

        let (x0, y0, x1, y1) = match self.flags.source {
            Source::Region(region) => {
                let x0 = region.origin.x.max(0) as u32;
                let y0 = region.origin.y.max(0) as u32;
                let x1 = (region.origin.x + region.size.width as i32).max(0) as u32;
                let y1 = (region.origin.y + region.size.height as i32).max(0) as u32;
                (x0, y0, x1, y1)
            }
            Source::Monitor | Source::Window => (0, 0, frame.width(), frame.height()),
        };
        if x0 >= x1 || y0 >= y1 || x1 > frame.width() || y1 > frame.height() {
            return Ok(());
        }
        // H.264 encoders reject odd frame dimensions, so drop at most one
        // pixel from the right and bottom edge.
        let x1 = x0 + ((x1 - x0) & !1);
        let y1 = y0 + ((y1 - y0) & !1);
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }

        let width = x1 - x0;
        let height = y1 - y0;
        if let Some(expected) = self.output_size
            && expected != (width, height)
        {
            return Ok(());
        }

        let row = width as usize * 4;
        if self.frame_bytes.len() < row * height as usize {
            self.frame_bytes.resize(row * height as usize, 0);
        }

        let paused_ticks = (self.paused_total.as_nanos() / 100) as i64;
        let timestamp = frame.timestamp()?.Duration.saturating_sub(paused_ticks);

        let buffer = frame.buffer_crop(x0, y0, x1, y1)?;
        let mut packed = Vec::new();
        let data = buffer.as_nopadding_buffer(&mut packed);
        if data.len() < row * height as usize {
            return Ok(());
        }
        for y in 0..height as usize {
            let source = (height as usize - 1 - y) * row;
            self.frame_bytes[y * row..y * row + row].copy_from_slice(&data[source..source + row]);
        }

        if self.encoder.is_none() {
            let bitrate = ((u64::from(width) * u64::from(height) * u64::from(self.flags.fps)) / 4)
                .clamp(4_000_000, 60_000_000) as u32;
            let encoder = match VideoEncoder::new(
                VideoSettingsBuilder::new(width, height)
                    .sub_type(VideoSettingsSubType::H264)
                    .frame_rate(self.flags.fps)
                    .bitrate(bitrate),
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                &self.flags.output,
            ) {
                Ok(encoder) => encoder,
                Err(error) => {
                    self.flags.shared.fail(error.to_string());
                    return Err(error.into());
                }
            };
            self.encoder = Some(encoder);
            self.output_size = Some((width, height));
        }

        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame_buffer(&self.frame_bytes[..row * height as usize], timestamp)?;
        }
        Ok(())
    }
}

impl Recorder {
    fn watch_stop(&mut self, capture_control: InternalCaptureControl) {
        if self.watching {
            return;
        }
        self.watching = true;
        let stop = Arc::clone(&self.flags.shared);
        std::thread::spawn(move || {
            while !stop.stop_requested() {
                if stop.finished.load(Ordering::Acquire) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            capture_control.stop();
        });
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(encoder) = self.encoder.take() {
            if let Err(error) = encoder.finish() {
                self.flags.shared.fail(error.to_string());
                return;
            }
        } else {
            self.flags
                .shared
                .fail("no frames were captured before the recording stopped");
            return;
        }
        self.flags.shared.finish();
    }
}
