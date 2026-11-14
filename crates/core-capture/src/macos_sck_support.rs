//! Shared ScreenCaptureKit helpers used by screenshots and recording.
//!
//! These functions intentionally return `Option`/`String` rather than a
//! capture error so both `core-capture` and `platform-record` can map failures
//! into their own error types.

use std::sync::mpsc::channel;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{SCDisplay, SCShareableContent, SCWindow};

/// Queries shareable content once, excluding desktop windows and off-screen
/// windows. The completion handler only lends the object, so retain it before
/// the block returns and transfer ownership through the raw pointer.
pub fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = channel::<Result<usize, String>>();
    let block = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            if content.is_null() {
                let message =
                    unsafe { error_message(error) }.unwrap_or_else(|| "unknown error".to_owned());
                let _ = tx.send(Err(message));
                return;
            }
            let retained = unsafe { Retained::retain(content) };
            let pointer = retained.map_or(0, |content| Retained::into_raw(content) as usize);
            let _ = tx.send(Ok(pointer));
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            true,
            &block,
        );
    }
    let pointer = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "timed out while querying shareable content".to_owned())??;
    unsafe { Retained::from_raw(pointer as *mut SCShareableContent) }
        .ok_or_else(|| "the system returned empty shareable content".to_owned())
}

/// # Safety
/// `error` must be null or a valid `NSError` pointer for the duration of the call.
pub unsafe fn error_message(error: *mut NSError) -> Option<String> {
    if error.is_null() {
        return None;
    }
    unsafe { Retained::retain(error) }.map(|error| error.localizedDescription().to_string())
}

pub fn find_display(content: &SCShareableContent, raw: u64) -> Option<Retained<SCDisplay>> {
    let displays = unsafe { content.displays() };
    let target = raw as u32;
    displays
        .iter()
        .find(|display| unsafe { display.displayID() } == target)
}

pub fn find_window(content: &SCShareableContent, raw: u64) -> Option<Retained<SCWindow>> {
    let windows = unsafe { content.windows() };
    let target = raw as u32;
    windows
        .iter()
        .find(|window| unsafe { window.windowID() } == target)
}

pub fn display_width(display: &SCDisplay) -> usize {
    unsafe { display.width() }.max(1) as usize
}

pub fn display_height(display: &SCDisplay) -> usize {
    unsafe { display.height() }.max(1) as usize
}

/// Backing-store pixels per point. SCDisplay reports pixel dimensions that can
/// come from the panel rather than the window server's scaled backing store,
/// so prefer the display mode's pixel size for capture geometry.
pub fn display_scale(display: &SCDisplay) -> f64 {
    crate::display_backing_scale(u64::from(unsafe { display.displayID() }))
        .map(f64::from)
        .unwrap_or_else(|| {
            let frame = unsafe { display.frame() };
            if frame.size.width > 0.0 {
                display_width(display) as f64 / frame.size.width
            } else {
                1.0
            }
        })
}

pub fn window_scale(window: &SCWindow, content: &SCShareableContent) -> Option<f64> {
    let frame = unsafe { window.frame() };
    let center_x = frame.origin.x + frame.size.width / 2.0;
    let center_y = frame.origin.y + frame.size.height / 2.0;
    let displays = unsafe { content.displays() };
    displays
        .iter()
        .find(|display| {
            let bounds = unsafe { display.frame() };
            center_x >= bounds.origin.x
                && center_y >= bounds.origin.y
                && center_x <= bounds.origin.x + bounds.size.width
                && center_y <= bounds.origin.y + bounds.size.height
        })
        .map(|display| display_scale(&display))
}
