use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use core_record::{
    CURSOR_HIGHLIGHT_ALPHA, CURSOR_HIGHLIGHT_RADIUS, CursorHighlight, FRAME_RATE, FrameDecision,
    FrameTimeline, RecordConfig, paint_cursor_highlight_bgra, video_bitrate,
};
use core_types::{CaptureTarget, PhysicalRect};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus,
    AVFileTypeMPEG4, AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey,
    AVVideoCodecTypeH264, AVVideoCompressionPropertiesKey, AVVideoHeightKey,
    AVVideoMaxKeyFrameIntervalKey, AVVideoWidthKey,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferPool, CVPixelBufferUnlockBaseAddress,
    kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey,
    kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString, NSURL};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use platform_capture::macos_sck_support as sck;

use crate::Flags;

const QUEUE_DEPTH: isize = 6;
const FRAME_QUEUE: usize = 4;

pub(crate) fn start(
    config: RecordConfig,
    output: PathBuf,
    temp: PathBuf,
    excluded_windows: Vec<u64>,
    flags: Arc<Flags>,
) -> Result<(), String> {
    ensure_permission()?;
    let content = sck::shareable_content()?;
    let plan = build_plan(&content, &config, &excluded_windows)?;
    let (tx, rx) = sync_channel::<EncodedFrame>(FRAME_QUEUE);
    let (recycle_tx, recycle_rx) = sync_channel::<Vec<u8>>(FRAME_QUEUE);
    let (width, height) = (plan.width, plan.height);
    let encoder_flags = Arc::clone(&flags);
    let encoder_temp = temp.clone();
    let encoder_output = output.clone();
    let encoder = std::thread::Builder::new()
        .name("capin-record-av".to_owned())
        .spawn(move || {
            encode_loop(
                &encoder_temp,
                &encoder_output,
                rx,
                width,
                height,
                encoder_flags,
                recycle_tx,
            )
        })
        .map_err(|error| error.to_string())?;

    let session_flags = Arc::clone(&flags);
    let result = capture_session(&config, &plan, session_flags, tx, recycle_rx);
    let encoded = match encoder.join() {
        Ok(result) => result,
        Err(_) => Err("the recording encoder thread panicked".to_owned()),
    };

    let outcome = match (result, encoded) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    };
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temp);
    } else {
        flags.finish();
    }
    outcome
}

struct SessionPlan {
    filter: Retained<SCContentFilter>,
    width: usize,
    height: usize,
    source_rect: Option<CGRect>,
    single_window: bool,
    cursor_space: CursorSpace,
    window: Option<Retained<SCWindow>>,
}

#[derive(Clone, Copy, Debug)]
enum CursorSpace {
    Screen {
        frame_x: f64,
        frame_y: f64,
        source_x: f64,
        source_y: f64,
        source_width: f64,
        source_height: f64,
        scale: f64,
    },
    Window {
        origin_x: f64,
        origin_y: f64,
        width: f64,
        height: f64,
        scale: f64,
    },
}

impl CursorSpace {
    fn dpi(self) -> f32 {
        let scale = match self {
            Self::Screen { scale, .. } | Self::Window { scale, .. } => scale,
        };
        scale.clamp(1.0, 4.0) as f32
    }

    fn cursor_in_frame(self, width: usize, height: usize) -> Option<(f32, f32)> {
        let (location_x, location_y) = sck::cursor_location()?;
        let (x, y) = match self {
            Self::Screen {
                frame_x,
                frame_y,
                source_x,
                source_y,
                source_width,
                source_height,
                ..
            } => (
                (location_x - frame_x - source_x) * (width as f64 / source_width.max(1.0)),
                (location_y - frame_y - source_y) * (height as f64 / source_height.max(1.0)),
            ),
            Self::Window {
                origin_x,
                origin_y,
                width: source_width,
                height: source_height,
                ..
            } => (
                (location_x - origin_x) * (width as f64 / source_width.max(1.0)),
                (location_y - origin_y) * (height as f64 / source_height.max(1.0)),
            ),
        };
        Some((x as f32, y as f32))
    }
}

fn build_plan(
    content: &SCShareableContent,
    config: &RecordConfig,
    extra_exclusions: &[u64],
) -> Result<SessionPlan, String> {
    match config.target {
        CaptureTarget::Monitor(id) => {
            let display = find_display(content, id.raw())?;
            let frame = unsafe { display.frame() };
            let excluded = sck::excluded_windows(content, extra_exclusions);
            let windows = NSArray::from_retained_slice(&excluded);
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &windows,
                )
            };
            let scale = sck::display_scale(&display);
            let output_width = (frame.size.width * scale).round().max(2.0) as usize;
            let output_height = (frame.size.height * scale).round().max(2.0) as usize;
            let (width, height) = fitted_output(output_width, output_height);
            let output_scale = width as f64 / frame.size.width.max(1.0);
            Ok(SessionPlan {
                filter,
                width,
                height,
                source_rect: None,
                single_window: false,
                window: None,
                cursor_space: CursorSpace::Screen {
                    frame_x: frame.origin.x,
                    frame_y: frame.origin.y,
                    source_x: 0.0,
                    source_y: 0.0,
                    source_width: frame.size.width,
                    source_height: frame.size.height,
                    scale: output_scale,
                },
            })
        }
        CaptureTarget::Region(region) => {
            let display = find_display(content, region.monitor.raw())?;
            let frame = unsafe { display.frame() };
            let (width, height) = region_size(region.rect)?;
            let scale = sck::display_scale(&display);
            let source_rect = CGRect::new(
                CGPoint::new(
                    region.rect.origin.x as f64 / scale,
                    region.rect.origin.y as f64 / scale,
                ),
                CGSize::new(
                    region.rect.size.width as f64 / scale,
                    region.rect.size.height as f64 / scale,
                ),
            );
            let excluded = sck::excluded_windows(content, extra_exclusions);
            let windows = NSArray::from_retained_slice(&excluded);
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &windows,
                )
            };
            let output_scale = width as f64 / source_rect.size.width.max(1.0);
            Ok(SessionPlan {
                filter,
                width,
                height,
                source_rect: Some(source_rect),
                single_window: false,
                window: None,
                cursor_space: CursorSpace::Screen {
                    frame_x: frame.origin.x,
                    frame_y: frame.origin.y,
                    source_x: source_rect.origin.x,
                    source_y: source_rect.origin.y,
                    source_width: source_rect.size.width,
                    source_height: source_rect.size.height,
                    scale: output_scale,
                },
            })
        }
        CaptureTarget::Window(id) => {
            let window = find_window(content, id.raw())?;
            let frame = unsafe { window.frame() };
            let (width, height) = window_size(&window, content)?;
            let filter = unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            };
            let output_scale = width as f64 / frame.size.width.max(1.0);
            Ok(SessionPlan {
                filter,
                width,
                height,
                source_rect: None,
                single_window: true,
                window: Some(window),
                cursor_space: CursorSpace::Window {
                    origin_x: frame.origin.x,
                    origin_y: frame.origin.y,
                    width: frame.size.width,
                    height: frame.size.height,
                    scale: output_scale,
                },
            })
        }
    }
}

fn find_display(content: &SCShareableContent, raw: u64) -> Result<Retained<SCDisplay>, String> {
    sck::find_display(content, raw)
        .ok_or_else(|| "the display to record no longer exists".to_owned())
}

fn region_size(rect: PhysicalRect) -> Result<(usize, usize), String> {
    if rect.size.width == 0 || rect.size.height == 0 {
        return Err("empty recording region".to_owned());
    }
    Ok(fitted_output(
        rect.size.width as usize,
        rect.size.height as usize,
    ))
}

fn fitted_output(width: usize, height: usize) -> (usize, usize) {
    const MAX_WIDTH: usize = 4096;
    const MAX_HEIGHT: usize = 2304;

    let width = width.max(2);
    let height = height.max(2);
    let ratio = (width as f64 / MAX_WIDTH as f64).max(height as f64 / MAX_HEIGHT as f64);
    if ratio > 1.0 {
        (
            even((width as f64 / ratio).floor() as usize),
            even((height as f64 / ratio).floor() as usize),
        )
    } else {
        (even(width), even(height))
    }
}

fn even(value: usize) -> usize {
    (value & !1).max(2)
}

fn window_size(window: &SCWindow, content: &SCShareableContent) -> Result<(usize, usize), String> {
    let frame = unsafe { window.frame() };
    if frame.size.width < 1.0 || frame.size.height < 1.0 {
        return Err("the window to record has no drawable area".to_owned());
    }
    let scale = sck::window_scale(window, content).unwrap_or(1.0);
    Ok(fitted_output(
        (frame.size.width * scale).round().max(1.0) as usize,
        (frame.size.height * scale).round().max(1.0) as usize,
    ))
}

fn find_window(content: &SCShareableContent, raw: u64) -> Result<Retained<SCWindow>, String> {
    sck::find_window(content, raw).ok_or_else(|| "the window to record no longer exists".to_owned())
}

fn capture_session(
    config: &RecordConfig,
    plan: &SessionPlan,
    flags: Arc<Flags>,
    tx: SyncSender<EncodedFrame>,
    recycle_rx: std::sync::mpsc::Receiver<Vec<u8>>,
) -> Result<(), String> {
    let stream_config = build_stream_config(config, plan);

    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &plan.filter,
            &stream_config,
            None,
        )
    };

    let output = ScreenOutput::new(
        tx,
        Arc::clone(&flags),
        config.cursor_highlight,
        plan.cursor_space,
        plan.window.clone(),
        recycle_rx,
    );
    let output_protocol: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*output);
    let queue = DispatchQueue::new("io.github.fynhq.capin.record", None);
    unsafe {
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                output_protocol,
                SCStreamOutputType::Screen,
                Some(&queue),
            )
            .map_err(|error| format!("failed to attach the screen output: {}", ns_error(&error)))?;
    }

    let (start_tx, start_rx) = std::sync::mpsc::channel::<Option<String>>();
    let start_block = RcBlock::new(move |error: *mut NSError| {
        let _ = start_tx.send(unsafe { sck::error_message(error) });
    });
    unsafe { stream.startCaptureWithCompletionHandler(Some(&start_block)) };
    match start_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Some(error)) => return Err(format!("failed to start screen capture: {error}")),
        Ok(None) => {}
        Err(_) => return Err("screen capture did not start in time".to_owned()),
    }

    while !flags.stop_requested() && !flags.finished.load(std::sync::atomic::Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(50));
    }

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<Option<String>>();
    let stop_block = RcBlock::new(move |error: *mut NSError| {
        let _ = stop_tx.send(unsafe { sck::error_message(error) });
    });
    unsafe { stream.stopCaptureWithCompletionHandler(Some(&stop_block)) };
    let stop_error = stop_rx.recv_timeout(Duration::from_secs(5)).ok().flatten();

    let _ = unsafe {
        stream.removeStreamOutput_type_error(output_protocol, SCStreamOutputType::Screen)
    };
    drop(output);

    if let Some(error) = stop_error {
        return Err(format!("failed to stop screen capture: {error}"));
    }
    Ok(())
}

fn build_stream_config(
    config: &RecordConfig,
    plan: &SessionPlan,
) -> Retained<SCStreamConfiguration> {
    let stream_config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        stream_config.setWidth(plan.width);
        stream_config.setHeight(plan.height);
        stream_config.setPixelFormat(kCVPixelFormatType_32BGRA);
        stream_config.setShowsCursor(config.include_cursor || config.cursor_highlight.is_some());
        stream_config.setCapturesAudio(false);
        stream_config.setQueueDepth(QUEUE_DEPTH);
        stream_config.setScalesToFit(true);
        stream_config.setPreservesAspectRatio(true);
        stream_config.setShouldBeOpaque(true);
        stream_config.setMinimumFrameInterval(CMTime::new(1, FRAME_RATE as i32));
    }

    if let Some(rect) = plan.source_rect {
        unsafe { stream_config.setSourceRect(rect) };
    }
    if plan.single_window
        && stream_config.respondsToSelector(objc2::sel!(setIgnoreShadowsSingleWindow:))
    {
        unsafe { stream_config.setIgnoreShadowsSingleWindow(true) };
    }

    stream_config
}

fn ensure_permission() -> Result<(), String> {
    if CGPreflightScreenCaptureAccess() {
        return Ok(());
    }
    if CGRequestScreenCaptureAccess() {
        return Ok(());
    }
    Err("screen recording permission was not granted".to_owned())
}

fn ns_error(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

struct OutputIvars {
    tx: SyncSender<EncodedFrame>,
    flags: Arc<Flags>,
    timeline: Mutex<FrameTimeline>,
    first_pts: Mutex<Option<i64>>,
    cursor_highlight: Option<CursorHighlight>,
    cursor_space: CursorSpace,
    window: Mutex<Option<Retained<SCWindow>>>,
    recycle: Mutex<std::sync::mpsc::Receiver<Vec<u8>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "CapinScreenRecorderOutput"]
    #[ivars = OutputIvars]
    struct ScreenOutput;

    unsafe impl NSObjectProtocol for ScreenOutput {}

    unsafe impl SCStreamOutput for ScreenOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output_sample_buffer(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            if output_type != SCStreamOutputType::Screen {
                return;
            }
            let ivars = self.ivars();
            if ivars.flags.is_paused() || ivars.flags.stop_requested() {
                return;
            }
            let Some(buffer) = (unsafe { sample_buffer.image_buffer() }) else {
                return;
            };
            let mut timeline = match ivars.timeline.lock() {
                Ok(timeline) => timeline,
                Err(poisoned) => poisoned.into_inner(),
            };
            if timeline.next_index() == 0 {
                ivars.flags.restart_started();
            }
            let pts = unsafe { sample_buffer.presentation_time_stamp() };
            let elapsed = if pts.flags.0 & 1 != 0 && pts.timescale > 0 {
                let nanos =
                    (i128::from(pts.value) * 1_000_000_000 / i128::from(pts.timescale)) as i64;
                let mut first = ivars
                    .first_pts
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let base = *first.get_or_insert(nanos);
                Duration::from_nanos((nanos - base).max(0) as u64)
                    .saturating_sub(ivars.flags.paused_duration())
            } else {
                ivars.flags.elapsed_active()
            };
            let FrameDecision::Encode { index, .. } = timeline.submit(elapsed) else {
                return;
            };
            drop(timeline);

            let reuse = ivars
                .recycle
                .lock()
                .ok()
                .and_then(|recycle| recycle.try_recv().ok())
                .unwrap_or_default();
            let (mut data, width, height) = copy_bgra(&buffer, reuse);
            if data.is_empty() {
                return;
            }
            let mut cursor_space = ivars.cursor_space;
            if let CursorSpace::Window {
                origin_x, origin_y, ..
            } = &mut cursor_space
                && let Ok(guard) = ivars.window.lock()
                && let Some(window) = guard.as_ref()
            {
                let frame = unsafe { window.frame() };
                *origin_x = frame.origin.x;
                *origin_y = frame.origin.y;
            }
            if let Some(highlight) = ivars.cursor_highlight
                && let Some((cursor_x, cursor_y)) = cursor_space.cursor_in_frame(width, height)
            {
                paint_cursor_highlight_bgra(
                    &mut data,
                    width as u32,
                    height as u32,
                    cursor_x,
                    cursor_y,
                    CURSOR_HIGHLIGHT_RADIUS * cursor_space.dpi(),
                    highlight.color,
                    CURSOR_HIGHLIGHT_ALPHA,
                );
            }
            let pts_millis = (index.saturating_mul(1000) / u64::from(FRAME_RATE)) as i64;
            let _ = ivars.tx.try_send(EncodedFrame {
                data,
                width,
                height,
                pts_millis,
            });
        }
    }
);

impl ScreenOutput {
    fn new(
        tx: SyncSender<EncodedFrame>,
        flags: Arc<Flags>,
        cursor_highlight: Option<CursorHighlight>,
        cursor_space: CursorSpace,
        window: Option<Retained<SCWindow>>,
        recycle: std::sync::mpsc::Receiver<Vec<u8>>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars {
            tx,
            flags,
            timeline: Mutex::new(FrameTimeline::new()),
            first_pts: Mutex::new(None),
            cursor_highlight,
            cursor_space,
            window: Mutex::new(window),
            recycle: Mutex::new(recycle),
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct EncodedFrame {
    data: Vec<u8>,
    width: usize,
    height: usize,
    pts_millis: i64,
}

fn copy_bgra(buffer: &CVPixelBuffer, mut reuse: Vec<u8>) -> (Vec<u8>, usize, usize) {
    unsafe {
        if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::empty()) != 0 {
            return (Vec::new(), 0, 0);
        }
        let width = CVPixelBufferGetWidth(buffer);
        let height = CVPixelBufferGetHeight(buffer);
        let row_bytes = CVPixelBufferGetBytesPerRow(buffer);
        let base = CVPixelBufferGetBaseAddress(buffer) as *const u8;
        if base.is_null() || width == 0 || height == 0 || row_bytes < width * 4 {
            CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
            return (Vec::new(), 0, 0);
        }
        reuse.clear();
        reuse.resize(width * height * 4, 0);
        for y in 0..height {
            std::ptr::copy_nonoverlapping(
                base.add(y * row_bytes),
                reuse.as_mut_ptr().add(y * width * 4),
                width * 4,
            );
        }
        CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
        (reuse, width, height)
    }
}

const NO_FRAMES_ERROR: &str = "no frames were captured before the recording stopped";

fn encode_loop(
    temp: &Path,
    final_output: &Path,
    rx: std::sync::mpsc::Receiver<EncodedFrame>,
    width: usize,
    height: usize,
    flags: Arc<Flags>,
    recycle_tx: SyncSender<Vec<u8>>,
) -> Result<(), String> {
    let result = (|| {
        let mut writer: Option<Writer> = None;
        while let Ok(frame) = rx.recv() {
            if writer.is_none() {
                writer = Some(Writer::new(temp, width, height)?);
            }
            if let Some(writer) = writer.as_mut() {
                writer.append(&frame)?;
            }
            let _ = recycle_tx.try_send(frame.data);
        }
        let final_pts = flags.elapsed_active().as_millis() as i64;
        let Some(writer) = writer else {
            return Err(NO_FRAMES_ERROR.to_owned());
        };
        writer.finish(final_pts)?;
        if let Err(error) = crate::mp4::inject_metadata(temp) {
            flags.warn(format!("recording metadata skipped: {error}"));
        }
        std::fs::rename(temp, final_output)
            .map_err(|error| format!("failed to publish the recording: {error}"))
    })();
    if let Err(error) = &result {
        let _ = std::fs::remove_file(temp);
        if error != NO_FRAMES_ERROR {
            flags.fail(error.clone());
        }
    }
    result
}

struct Writer {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    adaptor: Retained<AVAssetWriterInputPixelBufferAdaptor>,
    width: usize,
    height: usize,
    last_pts_millis: i64,
}

impl Writer {
    fn new(path: &Path, width: usize, height: usize) -> Result<Self, String> {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        let path_string = NSString::from_str(&path.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path_string);
        let file_type =
            unsafe { AVFileTypeMPEG4 }.ok_or_else(|| "MP4 is unavailable".to_owned())?;
        let writer = unsafe {
            AVAssetWriter::initWithURL_fileType_error(AVAssetWriter::alloc(), &url, file_type)
        }
        .map_err(|error| format!("failed to create the movie writer: {}", ns_error(&error)))?;

        let settings = video_settings(width, height)?;
        let media_type = unsafe { AVMediaTypeVideo }
            .ok_or_else(|| "video media type is unavailable".to_owned())?;
        let input = unsafe {
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                media_type,
                Some(&settings),
            )
        };
        unsafe {
            input.setExpectsMediaDataInRealTime(true);
        }
        if !unsafe { writer.canAddInput(&input) } {
            return Err("the movie writer rejected the video input".to_owned());
        }
        unsafe { writer.addInput(&input) };
        let pixel_attributes = pixel_buffer_attributes(width, height);
        let adaptor = unsafe {
            AVAssetWriterInputPixelBufferAdaptor::initWithAssetWriterInput_sourcePixelBufferAttributes(
                AVAssetWriterInputPixelBufferAdaptor::alloc(),
                &input,
                Some(&pixel_attributes),
            )
        };
        if !unsafe { writer.startWriting() } {
            let message = unsafe { writer.error() }
                .map(|error| ns_error(&error))
                .unwrap_or_else(|| "unknown error".to_owned());
            return Err(format!("failed to start the movie writer: {message}"));
        }
        unsafe { writer.startSessionAtSourceTime(CMTime::new(0, 1000)) };

        Ok(Self {
            writer,
            input,
            adaptor,
            width,
            height,
            last_pts_millis: 0,
        })
    }

    fn append(&mut self, frame: &EncodedFrame) -> Result<(), String> {
        if frame.width != self.width || frame.height != self.height {
            return Err("the recording target changed size; stop and start again".to_owned());
        }
        if !unsafe { self.input.isReadyForMoreMediaData() } {
            return Ok(());
        }
        let buffer = self.pixel_buffer()?;
        unsafe {
            if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()) != 0 {
                return Err("failed to lock the encoder pixel buffer".to_owned());
            }
            let base = CVPixelBufferGetBaseAddress(&buffer) as *mut u8;
            let row_bytes = CVPixelBufferGetBytesPerRow(&buffer);
            if base.is_null() || row_bytes < self.width * 4 {
                CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
                return Err("the encoder pixel buffer is not writable".to_owned());
            }
            for y in 0..self.height {
                std::ptr::copy_nonoverlapping(
                    frame.data.as_ptr().add(y * self.width * 4),
                    base.add(y * row_bytes),
                    self.width * 4,
                );
            }
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
        }
        let appended = unsafe {
            self.adaptor.appendPixelBuffer_withPresentationTime(
                &buffer,
                CMTime::new(frame.pts_millis.max(0), 1000),
            )
        };
        if !appended {
            let message = unsafe { self.writer.error() }
                .map(|error| ns_error(&error))
                .unwrap_or_else(|| "unknown error".to_owned());
            return Err(format!("failed to append a video frame: {message}"));
        }
        self.last_pts_millis = self.last_pts_millis.max(frame.pts_millis);
        Ok(())
    }

    fn pixel_buffer(&self) -> Result<CFRetained<CVPixelBuffer>, String> {
        let mut raw: *mut CVPixelBuffer = std::ptr::null_mut();
        let status = if let Some(pool) = unsafe { self.adaptor.pixelBufferPool() } {
            unsafe { CVPixelBufferPool::create_pixel_buffer(None, &pool, NonNull::from(&mut raw)) }
        } else {
            unsafe {
                objc2_core_video::CVPixelBufferCreate(
                    None,
                    self.width,
                    self.height,
                    kCVPixelFormatType_32BGRA,
                    None,
                    NonNull::from(&mut raw),
                )
            }
        };
        if status != 0 || raw.is_null() {
            return Err(format!("failed to allocate a video frame (error {status})"));
        }
        let pointer = NonNull::new(raw).ok_or_else(|| "empty video frame".to_owned())?;
        Ok(unsafe { CFRetained::from_raw(pointer) })
    }

    fn finish(self, final_pts_millis: i64) -> Result<(), String> {
        if final_pts_millis > self.last_pts_millis {
            unsafe {
                self.writer
                    .endSessionAtSourceTime(CMTime::new(final_pts_millis, 1000));
            }
        }
        unsafe { self.input.markAsFinished() };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let block = RcBlock::new(move || {
            let _ = tx.send(());
        });
        unsafe { self.writer.finishWritingWithCompletionHandler(&block) };
        if rx.recv_timeout(Duration::from_secs(10)).is_err() {
            return Err("the movie writer did not finish in time".to_owned());
        }
        if unsafe { self.writer.status() } != AVAssetWriterStatus::Completed {
            let message = unsafe { self.writer.error() }
                .map(|error| ns_error(&error))
                .unwrap_or_else(|| "unknown error".to_owned());
            return Err(format!("the movie writer did not complete: {message}"));
        }
        Ok(())
    }
}

fn pixel_buffer_attributes(
    width: usize,
    height: usize,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    let format_key = string_key(unsafe { kCVPixelBufferPixelFormatTypeKey });
    let width_key = string_key(unsafe { kCVPixelBufferWidthKey });
    let height_key = string_key(unsafe { kCVPixelBufferHeightKey });
    let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
    let width = NSNumber::new_usize(width);
    let height = NSNumber::new_usize(height);
    NSDictionary::from_slices(
        &[&*format_key, &*width_key, &*height_key],
        &[any_ref(&*format), any_ref(&*width), any_ref(&*height)],
    )
}

fn string_key(key: &objc2_core_foundation::CFString) -> Retained<NSString> {
    NSString::from_str(&key.to_string())
}

fn video_settings(
    width: usize,
    height: usize,
) -> Result<Retained<NSDictionary<NSString, AnyObject>>, String> {
    let codec = unsafe { AVVideoCodecTypeH264 }.ok_or_else(|| "H.264 is unavailable".to_owned())?;
    let bitrate = NSNumber::new_u32(video_bitrate(width as u32, height as u32));
    let interval = NSNumber::new_u32(FRAME_RATE);
    let compression = NSDictionary::from_slices(
        &[
            unsafe { AVVideoAverageBitRateKey }.unwrap(),
            unsafe { AVVideoMaxKeyFrameIntervalKey }.unwrap(),
        ],
        &[any_ref(&*bitrate), any_ref(&*interval)],
    );
    let width = NSNumber::new_usize(width);
    let height = NSNumber::new_usize(height);

    Ok(NSDictionary::from_slices(
        &[
            unsafe { AVVideoCodecKey }.unwrap(),
            unsafe { AVVideoWidthKey }.unwrap(),
            unsafe { AVVideoHeightKey }.unwrap(),
            unsafe { AVVideoCompressionPropertiesKey }.unwrap(),
        ],
        &[
            any_ref(codec),
            any_ref(&*width),
            any_ref(&*height),
            any_ref(&*compression),
        ],
    ))
}

fn any_ref(value: &impl Message) -> &AnyObject {
    // SAFETY: every Objective-C object shares the `AnyObject` representation;
    // the reference only lives for the duration of the dictionary construction.
    unsafe { &*(value as *const _ as *const AnyObject) }
}
