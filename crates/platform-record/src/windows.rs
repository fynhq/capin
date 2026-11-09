use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use core_capture::{CaptureTarget, platform_backend};
use core_record::{
    CURSOR_HIGHLIGHT_ALPHA, CURSOR_HIGHLIGHT_RADIUS, CursorHighlight, FrameDecision, FrameRate,
    FrameTimeline, RecordConfig, RecordQuality, WebpEncoder, paint_cursor_highlight_bgra,
    webp_output_size,
};
use core_types::{PhysicalRect, PhysicalSize, RecordFormat};
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
    format: RecordFormat,
    fps: u32,
    quality: RecordQuality,
    include_cursor: bool,
    cursor_highlight: Option<CursorHighlight>,
    cursor_space: CursorSpace,
    shared: Arc<Flags>,
}

/// Maps screen-space cursor positions into the captured frame.
#[derive(Clone, Copy, Debug)]
enum CursorSpace {
    /// The frame starts at a physical desktop origin and covers `width`x`height`.
    Screen {
        origin_x: i32,
        origin_y: i32,
        width: u32,
        height: u32,
        dpi: f32,
    },
    /// The frame follows a window, so its origin is queried per frame.
    Window { hwnd: isize, dpi: f32 },
}

impl CursorSpace {
    fn dpi(self) -> f32 {
        match self {
            Self::Screen { dpi, .. } | Self::Window { dpi, .. } => dpi.clamp(1.0, 4.0),
        }
    }

    /// Returns the cursor position in frame pixels for a frame of `width`x`height`.
    fn cursor_in_frame(self, width: u32, height: u32) -> Option<(f32, f32)> {
        let (cursor_x, cursor_y) = cursor_position()?;
        match self {
            Self::Screen {
                origin_x,
                origin_y,
                width: source_width,
                height: source_height,
                ..
            } => {
                let scale_x = width as f32 / source_width.max(1) as f32;
                let scale_y = height as f32 / source_height.max(1) as f32;
                Some((
                    (cursor_x - origin_x) as f32 * scale_x,
                    (cursor_y - origin_y) as f32 * scale_y,
                ))
            }
            Self::Window { hwnd, .. } => {
                let (left, top, source_width, source_height) = window_rect(hwnd)?;
                let scale_x = width as f32 / source_width.max(1) as f32;
                let scale_y = height as f32 / source_height.max(1) as f32;
                Some((
                    (cursor_x - left) as f32 * scale_x,
                    (cursor_y - top) as f32 * scale_y,
                ))
            }
        }
    }
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
    // Cursor geometry is only needed for the demo highlight; skip the extra
    // monitor/window enumeration otherwise.
    let monitors = config
        .cursor_highlight
        .map(|_| platform_backend().monitors().unwrap_or_default())
        .unwrap_or_default();
    let windows =
        if config.cursor_highlight.is_some() && matches!(config.target, CaptureTarget::Window(_)) {
            platform_backend().windows().unwrap_or_default()
        } else {
            Vec::new()
        };
    match config.target {
        CaptureTarget::Region(region) => {
            let monitor = find_monitor(region.monitor.raw())?;
            let monitor_info = monitors.iter().find(|info| info.id == region.monitor);
            let cursor_space = CursorSpace::Screen {
                origin_x: monitor_info.map_or(region.rect.origin.x, |info| {
                    info.bounds.origin.x + region.rect.origin.x
                }),
                origin_y: monitor_info.map_or(region.rect.origin.y, |info| {
                    info.bounds.origin.y + region.rect.origin.y
                }),
                width: region.rect.size.width,
                height: region.rect.size.height,
                dpi: monitor_info.map_or(1.0, |info| info.scale.get()),
            };
            let info = CaptureInfo {
                source: Source::Region(region.rect),
                output: temp,
                final_output: output,
                format: config.format,
                fps,
                quality: config.quality,
                include_cursor: config.include_cursor,
                cursor_highlight: config.cursor_highlight,
                cursor_space,
                shared,
            };
            spawn_monitor(monitor, info)
        }
        CaptureTarget::Monitor(id) => {
            let monitor = find_monitor(id.raw())?;
            let monitor_info = monitors.iter().find(|info| info.id == id);
            let cursor_space = CursorSpace::Screen {
                origin_x: monitor_info.map_or(0, |info| info.bounds.origin.x),
                origin_y: monitor_info.map_or(0, |info| info.bounds.origin.y),
                width: monitor_info.map_or(1, |info| info.bounds.size.width),
                height: monitor_info.map_or(1, |info| info.bounds.size.height),
                dpi: monitor_info.map_or(1.0, |info| info.scale.get()),
            };
            let info = CaptureInfo {
                source: Source::Monitor,
                output: temp,
                final_output: output,
                format: config.format,
                fps,
                quality: config.quality,
                include_cursor: config.include_cursor,
                cursor_highlight: config.cursor_highlight,
                cursor_space,
                shared,
            };
            spawn_monitor(monitor, info)
        }
        CaptureTarget::Window(id) => {
            let raw = id.raw() as usize as *mut std::ffi::c_void;
            let window = Window::from_raw_hwnd(raw);
            let cursor_space = CursorSpace::Window {
                hwnd: id.raw() as isize,
                dpi: windows
                    .iter()
                    .find(|info| info.id == id)
                    .map_or(1.0, |info| info.scale.get()),
            };
            let info = CaptureInfo {
                source: Source::Window,
                output: temp,
                final_output: output,
                format: config.format,
                fps,
                quality: config.quality,
                include_cursor: config.include_cursor,
                cursor_highlight: config.cursor_highlight,
                cursor_space,
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
    let cursor = cursor_settings(info.include_cursor || info.cursor_highlight.is_some());
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
    let cursor = cursor_settings(info.include_cursor || info.cursor_highlight.is_some());
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

fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

/// Returns the window's physical outer rectangle as `(left, top, width, height)`.
fn window_rect(hwnd: isize) -> Option<(i32, i32, u32, u32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(HWND(hwnd as *mut std::ffi::c_void), &mut rect).ok()?;
    }
    Some((
        rect.left,
        rect.top,
        (rect.right - rect.left).max(1) as u32,
        (rect.bottom - rect.top).max(1) as u32,
    ))
}

struct Recorder {
    encoder: Option<VideoEncoder>,
    webp: Option<WebpEncoder>,
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
            webp: None,
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
        for (y, chunk) in frame_bytes.chunks_exact_mut(row).enumerate() {
            let source = (height as usize - 1 - y) * row;
            chunk.copy_from_slice(&data[source..source + row]);
        }

        if let Some(highlight) = self.flags.cursor_highlight
            && let Some((cursor_x, cursor_y)) =
                self.flags.cursor_space.cursor_in_frame(width, height)
        {
            paint_cursor_highlight_bgra(
                &mut frame_bytes,
                width,
                height,
                cursor_x,
                cursor_y,
                CURSOR_HIGHLIGHT_RADIUS * self.flags.cursor_space.dpi(),
                highlight.color,
                CURSOR_HIGHLIGHT_ALPHA,
            );
        }

        if self.flags.format == RecordFormat::WebP {
            if self.webp.is_none() {
                let input = PhysicalSize::new(width, height);
                let output = webp_output_size(input);
                match WebpEncoder::new(
                    input,
                    output,
                    self.flags.fps,
                    self.flags.quality.webp_quality(),
                ) {
                    Ok(encoder) => self.webp = Some(encoder),
                    Err(error) => {
                        self.flags.shared.fail(error.to_string());
                        capture_control.stop();
                        return Ok(());
                    }
                }
            }
            if let Some(encoder) = self.webp.as_mut() {
                let pts_millis =
                    (index.saturating_mul(1000) / u64::from(self.flags.fps.max(1))) as i64;
                if let Err(error) = encoder.push_bgra(&frame_bytes, pts_millis) {
                    self.flags.shared.fail(error.to_string());
                    capture_control.stop();
                }
            }
            return Ok(());
        }

        if self.encoder.is_none() {
            let bitrate = (u64::from(width) * u64::from(height) * u64::from(self.flags.fps)) / 4;
            let bitrate = (bitrate as f32 * self.flags.quality.bitrate_multiplier())
                .clamp(4_000_000.0, 60_000_000.0) as u32;
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
        if self
            .flags
            .shared
            .error
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_file(&self.flags.output);
            return;
        }

        if self.flags.format == RecordFormat::WebP {
            let Some(encoder) = self.webp.take() else {
                self.flags
                    .shared
                    .fail("no frames were captured before the recording stopped");
                let _ = std::fs::remove_file(&self.flags.output);
                return;
            };
            let end_ms = self.flags.shared.elapsed_active().as_millis() as i64;
            if let Err(error) = encoder.finish(end_ms, &self.flags.output) {
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
            return;
        }

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
