use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_core_graphics::{CGEvent, CGEventSource, CGEventSourceStateID};
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{SCDisplay, SCShareableContent, SCWindow};

struct ContentSlot {
    pointer: Mutex<Option<usize>>,
    abandoned: AtomicBool,
}

fn release_pointer(pointer: usize) {
    unsafe {
        drop(Retained::from_raw(pointer as *mut SCShareableContent));
    }
}

pub fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = channel::<Result<(), String>>();
    let slot = Arc::new(ContentSlot {
        pointer: Mutex::new(None),
        abandoned: AtomicBool::new(false),
    });
    let block_slot = Arc::clone(&slot);
    let block = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            if content.is_null() {
                let message =
                    unsafe { error_message(error) }.unwrap_or_else(|| "unknown error".to_owned());
                let _ = tx.send(Err(message));
                return;
            }
            let Some(retained) = (unsafe { Retained::retain(content) }) else {
                let _ = tx.send(Err("could not retain shareable content".to_owned()));
                return;
            };
            let pointer = Retained::into_raw(retained) as usize;
            if block_slot.abandoned.load(Ordering::Acquire) {
                release_pointer(pointer);
                return;
            }
            let Ok(mut current) = block_slot.pointer.lock() else {
                release_pointer(pointer);
                let _ = tx.send(Err("shareable content slot is unavailable".to_owned()));
                return;
            };
            if block_slot.abandoned.load(Ordering::Acquire) {
                drop(current);
                release_pointer(pointer);
                return;
            }
            *current = Some(pointer);
            drop(current);
            let _ = tx.send(Ok(()));
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            true,
            &block,
        );
    }
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {
            let pointer = slot
                .pointer
                .lock()
                .ok()
                .and_then(|mut pointer| pointer.take());
            let Some(pointer) = pointer else {
                return Err("the system returned empty shareable content".to_owned());
            };
            unsafe { Retained::from_raw(pointer as *mut SCShareableContent) }
                .ok_or_else(|| "the system returned empty shareable content".to_owned())
        }
        Ok(Err(message)) => Err(message),
        Err(_) => {
            slot.abandoned.store(true, Ordering::Release);
            let pointer = slot
                .pointer
                .lock()
                .ok()
                .and_then(|mut pointer| pointer.take());
            if let Some(pointer) = pointer {
                release_pointer(pointer);
            }
            Err("timed out while querying shareable content".to_owned())
        }
    }
}

/// # Safety
/// `error` must be null or a valid `NSError` pointer for the duration of the call.
pub unsafe fn error_message(error: *mut NSError) -> Option<String> {
    if error.is_null() {
        return None;
    }
    unsafe { Retained::retain(error) }.map(|error| error.localizedDescription().to_string())
}

pub fn own_windows(content: &SCShareableContent, extra: &[u64]) -> Vec<Retained<SCWindow>> {
    let process = std::process::id() as i32;
    excluded_windows(content, extra)
        .into_iter()
        .filter(|window| {
            unsafe { window.owningApplication() }
                .is_some_and(|application| unsafe { application.processID() } == process)
        })
        .collect()
}

pub fn excluded_windows(content: &SCShareableContent, extra: &[u64]) -> Vec<Retained<SCWindow>> {
    let excluded = crate::capture_excluded_window_ids();
    unsafe { content.windows() }
        .iter()
        .filter(|window| {
            let id = unsafe { window.windowID() };
            excluded.contains(&id) || extra.contains(&(u64::from(id)))
        })
        .collect()
}

pub fn cursor_location() -> Option<(f64, f64)> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)?;
    let event = CGEvent::new(Some(&source))?;
    let location = CGEvent::location(Some(&event));
    Some((location.x, location.y))
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

pub fn display_scale(display: &SCDisplay) -> f64 {
    crate::display_backing_scale(u64::from(unsafe { display.displayID() }))
        .map(f64::from)
        .unwrap_or(1.0)
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
