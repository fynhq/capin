//! ScreenCaptureKit single-shot screenshots.
//!
//! macOS 14 and later use `SCScreenshotManager`. macOS 13 uses a one-frame
//! `SCStream`, because the single-shot manager was introduced in macOS 14.

use std::mem::ManuallyDrop;
use std::sync::mpsc::{SyncSender, channel, sync_channel};
use std::time::Duration;

use block2::RcBlock;
use core_types::RgbaFrame;
use dispatch2::DispatchQueue;
use foreign_types::ForeignType;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCScreenshotManager, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamOutput, SCStreamOutputType, SCWindow,
};

use crate::{CaptureError, CaptureTarget, Result};

use crate::macos_sck_support as support;

fn supports_screenshot_manager() -> bool {
    AnyClass::get(c"SCScreenshotManager").is_some()
}

pub(super) fn capture(target: CaptureTarget) -> Result<RgbaFrame> {
    let content = shareable_content()?;
    let filter = build_filter(&content, target)?;
    let config = build_config(&content, target)?;
    if supports_screenshot_manager() {
        capture_image(&filter, &config)
    } else {
        capture_stream(&filter, &config)
    }
}

fn capture_image(filter: &SCContentFilter, config: &SCStreamConfiguration) -> Result<RgbaFrame> {
    let (tx, rx) = channel::<std::result::Result<RgbaFrame, String>>();
    let block = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        if !error.is_null() {
            let message = unsafe { Retained::retain(error) }
                .map(|error| error.localizedDescription().to_string())
                .unwrap_or_else(|| "unknown error".to_owned());
            let _ = tx.send(Err(message));
            return;
        }
        if image.is_null() {
            let _ = tx.send(Err("the system returned an empty screenshot".to_owned()));
            return;
        }
        // The callback lends the image for its duration; convert it before
        // returning instead of retaining a CoreGraphics object across threads.
        let borrowed = ManuallyDrop::new(unsafe {
            core_graphics::image::CGImage::from_ptr(image as *mut core_graphics::sys::CGImage)
        });
        let result = super::image_to_frame(&borrowed).map_err(|error| error.to_string());
        let _ = tx.send(result);
    });

    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            filter,
            config,
            Some(&block),
        );
    }

    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| CaptureError::Platform {
            operation: "SCScreenshotManager",
            message: "the screenshot request timed out".to_owned(),
        })?
        .map_err(|message| CaptureError::Platform {
            operation: "SCScreenshotManager",
            message,
        })
}

fn capture_stream(filter: &SCContentFilter, config: &SCStreamConfiguration) -> Result<RgbaFrame> {
    let (tx, rx) = sync_channel::<RgbaFrame>(1);
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), filter, config, None)
    };
    let output = ScreenOutput::new(tx);
    let output_protocol: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*output);
    let queue = DispatchQueue::new("io.github.fynhq.capin.screenshot", None);
    unsafe {
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                output_protocol,
                SCStreamOutputType::Screen,
                Some(&queue),
            )
            .map_err(|error| CaptureError::Platform {
                operation: "SCStream.addStreamOutput",
                message: error.localizedDescription().to_string(),
            })?;
    }

    let (start_tx, start_rx) = channel::<Option<String>>();
    let start_block = RcBlock::new(move |error: *mut NSError| {
        let _ = start_tx.send(unsafe { support::error_message(error) });
    });
    unsafe { stream.startCaptureWithCompletionHandler(Some(&start_block)) };
    match start_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Some(error)) => {
            return Err(CaptureError::Platform {
                operation: "SCStream.startCapture",
                message: error,
            });
        }
        Ok(None) => {}
        Err(_) => {
            return Err(CaptureError::Platform {
                operation: "SCStream.startCapture",
                message: "the stream did not start in time".to_owned(),
            });
        }
    }

    let frame = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| CaptureError::Platform {
            operation: "SCStream",
            message: "no frame arrived in time".to_owned(),
        });

    let (stop_tx, stop_rx) = channel::<()>();
    let stop_block = RcBlock::new(move |_error: *mut NSError| {
        let _ = stop_tx.send(());
    });
    unsafe { stream.stopCaptureWithCompletionHandler(Some(&stop_block)) };
    let _ = stop_rx.recv_timeout(Duration::from_secs(5));
    let _ = unsafe {
        stream.removeStreamOutput_type_error(output_protocol, SCStreamOutputType::Screen)
    };
    frame
}

struct OutputIvars {
    tx: SyncSender<RgbaFrame>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "CapinScreenshotOutput"]
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
            let Some(buffer) = (unsafe { sample_buffer.image_buffer() }) else {
                return;
            };
            if let Some(frame) = copy_frame(&buffer) {
                let _ = self.ivars().tx.try_send(frame);
            }
        }
    }
);

impl ScreenOutput {
    fn new(tx: SyncSender<RgbaFrame>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars { tx });
        unsafe { msg_send![super(this), init] }
    }
}

fn copy_frame(buffer: &CVPixelBuffer) -> Option<RgbaFrame> {
    unsafe {
        if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::empty()) != 0 {
            return None;
        }
        let width = CVPixelBufferGetWidth(buffer);
        let height = CVPixelBufferGetHeight(buffer);
        let row_bytes = CVPixelBufferGetBytesPerRow(buffer);
        let base = CVPixelBufferGetBaseAddress(buffer) as *const u8;
        if base.is_null() || width == 0 || height == 0 || row_bytes < width * 4 {
            CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
            return None;
        }
        let mut pixels = vec![0u8; width * height * 4];
        for y in 0..height {
            let row = base.add(y * row_bytes);
            for x in 0..width {
                let source = row.add(x * 4);
                let blue = *source;
                let green = *source.add(1);
                let red = *source.add(2);
                let alpha = *source.add(3);
                let target = &mut pixels[(y * width + x) * 4..(y * width + x) * 4 + 4];
                if alpha == 0 {
                    target.fill(0);
                } else if alpha < 255 {
                    let alpha = u32::from(alpha);
                    target[0] = ((u32::from(red) * 255) / alpha).min(255) as u8;
                    target[1] = ((u32::from(green) * 255) / alpha).min(255) as u8;
                    target[2] = ((u32::from(blue) * 255) / alpha).min(255) as u8;
                    target[3] = alpha as u8;
                } else {
                    target[0] = red;
                    target[1] = green;
                    target[2] = blue;
                    target[3] = 255;
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
        RgbaFrame::from_pixels(width as u32, height as u32, pixels).ok()
    }
}

fn build_filter(
    content: &SCShareableContent,
    target: CaptureTarget,
) -> Result<Retained<SCContentFilter>> {
    match target {
        CaptureTarget::Monitor(id) => {
            let display = find_display(content, id.raw())?;
            let excluded = own_windows(content);
            let windows = NSArray::from_retained_slice(&excluded);
            Ok(unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &windows,
                )
            })
        }
        CaptureTarget::Region(region) => {
            let display = find_display(content, region.monitor.raw())?;
            let excluded = own_windows(content);
            let windows = NSArray::from_retained_slice(&excluded);
            Ok(unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &windows,
                )
            })
        }
        CaptureTarget::Window(id) => {
            let window = find_window(content, id.raw())?;
            Ok(unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            })
        }
    }
}

fn build_config(
    content: &SCShareableContent,
    target: CaptureTarget,
) -> Result<Retained<SCStreamConfiguration>> {
    let config = unsafe { SCStreamConfiguration::new() };
    let (width, height, window) = match target {
        CaptureTarget::Monitor(id) => {
            let display = find_display(content, id.raw())?;
            let (width, height) = super::display_pixel_size(unsafe { display.displayID() })
                .unwrap_or_else(|| {
                    (
                        support::display_width(&display),
                        support::display_height(&display),
                    )
                });
            (width, height, false)
        }
        CaptureTarget::Region(region) => {
            let display = find_display(content, region.monitor.raw())?;
            let scale = support::display_scale(&display);
            let rect = CGRect::new(
                CGPoint::new(
                    region.rect.origin.x as f64 / scale,
                    region.rect.origin.y as f64 / scale,
                ),
                CGSize::new(
                    region.rect.size.width as f64 / scale,
                    region.rect.size.height as f64 / scale,
                ),
            );
            unsafe { config.setSourceRect(rect) };
            (
                region.rect.size.width as usize,
                region.rect.size.height as usize,
                false,
            )
        }
        CaptureTarget::Window(id) => {
            let window = find_window(content, id.raw())?;
            let frame = unsafe { window.frame() };
            if frame.size.width < 1.0 || frame.size.height < 1.0 {
                return Err(CaptureError::WindowNotFound(id));
            }
            let scale = support::window_scale(&window, content).unwrap_or(1.0);
            (
                (frame.size.width * scale).round().max(1.0) as usize,
                (frame.size.height * scale).round().max(1.0) as usize,
                true,
            )
        }
    };

    if width == 0 || height == 0 {
        return Err(CaptureError::EmptyRegion);
    }

    unsafe {
        config.setWidth(width);
        config.setHeight(height);
        config.setPixelFormat(kCVPixelFormatType_32BGRA);
        config.setShowsCursor(false);
        config.setCapturesAudio(false);
        config.setQueueDepth(3);
        config.setScalesToFit(true);
        config.setPreservesAspectRatio(true);
        // Screenshots of windows keep alpha so the window compositor can
        // apply its own rounded corners and shadow.
        config.setShouldBeOpaque(!window);
        if window && config.respondsToSelector(objc2::sel!(setIgnoreShadowsSingleWindow:)) {
            config.setIgnoreShadowsSingleWindow(true);
        }
    }
    Ok(config)
}

/// Returns Capin's own capture-helper windows so display captures never
/// include overlays, capture bars, inline editors, or pin menus. Pins and the
/// settings page stay capturable, exactly as the user sees them.
///
/// `NSWindow.sharingType` is ignored by ScreenCaptureKit on macOS 15, so the
/// exclusion must happen in the filter. The sharing flag is still readable
/// from the window server, which is what marks the helper windows.
fn own_windows(content: &SCShareableContent) -> Vec<Retained<SCWindow>> {
    let process = std::process::id() as i32;
    let excluded = super::capture_excluded_window_ids();
    unsafe { content.windows() }
        .iter()
        .filter(|window| {
            unsafe { window.owningApplication() }
                .is_some_and(|application| unsafe { application.processID() } == process)
        })
        .filter(|window| excluded.contains(&unsafe { window.windowID() }))
        .collect()
}

fn find_display(content: &SCShareableContent, raw: u64) -> Result<Retained<SCDisplay>> {
    support::find_display(content, raw)
        .ok_or_else(|| CaptureError::MonitorNotFound(core_types::MonitorId::from_raw(raw)))
}

fn find_window(content: &SCShareableContent, raw: u64) -> Result<Retained<SCWindow>> {
    support::find_window(content, raw)
        .ok_or_else(|| CaptureError::WindowNotFound(core_types::WindowId::from_raw(raw)))
}

fn shareable_content() -> Result<Retained<SCShareableContent>> {
    support::shareable_content().map_err(|message| CaptureError::Platform {
        operation: "SCShareableContent",
        message,
    })
}
