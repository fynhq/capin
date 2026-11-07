use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use core_capture::CaptureTarget;
use core_record::{FrameDecision, FrameRate, FrameTimeline, RecordConfig};
use core_types::PhysicalRect;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
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
    final_output: PathBuf,
    fps: u32,
    include_cursor: bool,
    shared: Arc<Flags>,
}

pub(crate) fn start(
    config: RecordConfig,
    output: PathBuf,
    temp: PathBuf,
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
                output: temp,
                final_output: output,
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
                output: temp,
                final_output: output,
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
                output: temp,
                final_output: output,
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
        .ok_or_else(|| "the monitor to record no longer exists".to_owned())
}

fn spawn_monitor(monitor: Monitor, info: CaptureInfo) -> Result<(), String> {
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
    let control = Recorder::start_free_threaded(settings).map_err(|error| error.to_string())?;
    supervise(control, shared);
    Ok(())
}

fn spawn_window(window: Window, info: CaptureInfo) -> Result<(), String> {
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
    let control = Recorder::start_free_threaded(settings).map_err(|error| error.to_string())?;
    supervise(control, shared);
    Ok(())
}

/// Watches for the stop request independently of frame arrival. The previous
/// implementation only created this watcher inside the first frame callback,
/// so a recording that never received a frame could not be stopped.
fn supervise(
    control: CaptureControl<Recorder, Box<dyn std::error::Error + Send + Sync>>,
    shared: Arc<Flags>,
) {
    std::thread::Builder::new()
        .name("capin-record-stop".to_owned())
        .spawn(move || {
            while !shared.stop_requested() {
                if shared.finished.load(Ordering::Acquire) {
                    let _ = control.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            if let Err(error) = control.stop() {
                shared.fail(error.to_string());
            }
        })
        .ok();
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
    last_buffer: Option<Vec<u8>>,
    last_pts: i64,
    frame_ticks: i64,
    timeline: FrameTimeline,
    flags: CaptureInfo,
}

impl GraphicsCaptureApiHandler for Recorder {
    type Flags = CaptureInfo;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let fps = ctx.flags.fps.max(1);
        ctx.flags.shared.restart_started();
        let frame_rate = FrameRate::from_u32(fps).unwrap_or(FrameRate::Fps30);
        Ok(Self {
            encoder: None,
            output_size: None,
            last_buffer: None,
            last_pts: 0,
            frame_ticks: 10_000_000 / i64::from(fps),
            timeline: FrameTimeline::new(frame_rate),
            flags: ctx.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.flags.shared.stop_requested() {
            return Ok(());
        }
        if self.flags.shared.is_paused() {
            return Ok(());
        }

        let paused_ticks = (self.flags.shared.paused_duration().as_nanos() / 100) as i64;
        let adjusted = frame
            .timestamp()?
            .Duration
            .saturating_sub(paused_ticks)
            .max(0);
        let adjusted = Duration::from_nanos((adjusted as u64).saturating_mul(100));
        let FrameDecision::Encode { index, .. } = self.timeline.submit(adjusted) else {
            return Ok(());
        };

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
            self.flags
                .shared
                .fail("the recording target changed size; stop and start again");
            capture_control.stop();
            return Ok(());
        }

        let row = width as usize * 4;
        let mut frame_bytes = vec![0u8; row * height as usize];

        let buffer = frame.buffer_crop(x0, y0, x1, y1)?;
        let mut packed = Vec::new();
        let data = buffer.as_nopadding_buffer(&mut packed);
        if data.len() < row * height as usize {
            return Ok(());
        }
        for y in 0..height as usize {
            let source = (height as usize - 1 - y) * row;
            frame_bytes[y * row..y * row + row].copy_from_slice(&data[source..source + row]);
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

        let pts = (index as i64).saturating_mul(self.frame_ticks);
        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame_buffer(&frame_bytes, pts)?;
        }
        self.last_pts = pts;
        self.last_buffer = Some(frame_bytes);
        Ok(())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let Some(mut encoder) = self.encoder.take() else {
            self.flags
                .shared
                .fail("no frames were captured before the recording stopped");
            let _ = std::fs::remove_file(&self.flags.output);
            return;
        };

        // Hold the last frame until the stop timestamp so static content does
        // not produce a file much shorter than the on-screen timer.
        if let Some(buffer) = self.last_buffer.take() {
            let elapsed_ticks = (self.flags.shared.elapsed_active().as_nanos() / 100) as i64;
            let final_pts = elapsed_ticks.max(self.last_pts.saturating_add(self.frame_ticks));
            if let Err(error) = encoder.send_frame_buffer(&buffer, final_pts) {
                self.flags.shared.fail(error.to_string());
                let _ = std::fs::remove_file(&self.flags.output);
                return;
            }
        }

        if let Err(error) = encoder.finish() {
            self.flags.shared.fail(error.to_string());
            let _ = std::fs::remove_file(&self.flags.output);
            return;
        }

        if let Err(error) = std::fs::rename(&self.flags.output, &self.flags.final_output) {
            self.flags
                .shared
                .fail(format!("failed to publish the recording: {error}"));
            let _ = std::fs::remove_file(&self.flags.output);
            return;
        }
        self.flags.shared.finish();
    }
}
