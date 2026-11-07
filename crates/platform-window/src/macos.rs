use std::path::PathBuf;

use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use foreign_types_shared::ForeignType;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSScreenSaverWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowSharingType,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{PlatformWindowError, WindowStyle};

pub(super) fn pictures_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Pictures"))
}

pub(super) fn videos_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Movies"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(super) fn set_click_through(
    window: &impl HasWindowHandle,
    enabled: bool,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return Err(PlatformWindowError::NotAppKit);
    };
    // SAFETY: the handle points to the NSView owned by the live GPUI window.
    let view = unsafe { &*(appkit.ns_view.as_ptr() as *const NSView) };
    let Some(window) = view.window() else {
        return Err(PlatformWindowError::NotAppKit);
    };
    window.setIgnoresMouseEvents(enabled);
    Ok(())
}

pub(super) fn set_accessory_activation_policy() {
    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(marker);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub(super) fn apply(
    window: &impl HasWindowHandle,
    style: WindowStyle,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return Err(PlatformWindowError::NotAppKit);
    };

    // SAFETY: the handle points to the NSView owned by the live GPUI window.
    let view = unsafe { &*(appkit.ns_view.as_ptr() as *const NSView) };
    let Some(window) = view.window() else {
        return Err(PlatformWindowError::NotAppKit);
    };

    if style.always_on_top {
        window.setLevel(NSScreenSaverWindowLevel);
    }

    let mut behavior = window.collectionBehavior();
    behavior |= NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary;
    window.setCollectionBehavior(behavior);

    if style.exclude_from_capture {
        window.setSharingType(NSWindowSharingType::None);
    }

    Ok(())
}

pub(super) fn window_capture_includes_native_shadow() -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};

    !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 14,
        minorVersion: 0,
        patchVersion: 0,
    })
}

pub(super) fn native_window_id(window: &impl HasWindowHandle) -> Option<u64> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the handle points to the NSView owned by the live GPUI window.
    let view = unsafe { &*(appkit.ns_view.as_ptr() as *const NSView) };
    let window = view.window()?;
    Some(window.windowNumber() as u64)
}

pub(super) fn scroll_wheel_at(monitor: u64, x: i32, y: i32, notches: i32) -> bool {
    if notches == 0 {
        return true;
    }
    if !unsafe { AXIsProcessTrusted() } {
        return false;
    }
    let Some(target) = display_point(monitor, x, y) else {
        return false;
    };
    let source = match CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        Ok(source) => source,
        Err(()) => return false,
    };
    let original = cursor_point();

    if let Ok(event) =
        CGEvent::new_mouse_event(source, CGEventType::MouseMoved, target, CGMouseButton::Left)
    {
        event.post(CGEventTapLocation::HID);
    }

    let source = match CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        Ok(source) => source,
        Err(()) => return false,
    };
    let Ok(scroll) = CGEvent::new_scroll_event(source, ScrollEventUnit::LINE, 1, notches, 0, 0)
    else {
        return false;
    };
    unsafe {
        CGEventSetLocation(scroll.as_ptr(), target);
    }
    scroll.post(CGEventTapLocation::HID);

    std::thread::sleep(std::time::Duration::from_millis(30));
    if let Some(original) = original
        && let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        && let Ok(event) = CGEvent::new_mouse_event(
            source,
            CGEventType::MouseMoved,
            original,
            CGMouseButton::Left,
        )
    {
        event.post(CGEventTapLocation::HID);
    }
    true
}

fn cursor_point() -> Option<CGPoint> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    Some(event.location())
}

/// Converts a monitor-local physical pixel into an AppKit point.
///
/// Resolving the display by identifier keeps mixed-DPI desktops unambiguous:
/// the previous global inverse mapping could pick the wrong display when two
/// monitors used different scales.
fn display_point(monitor: u64, x: i32, y: i32) -> Option<CGPoint> {
    let display_id = u32::try_from(monitor).ok()?;
    if display_id == 0 {
        return None;
    }
    let ids = CGDisplay::active_displays().ok()?;
    if !ids.contains(&display_id) {
        return None;
    }
    let display = CGDisplay::new(display_id);
    let bounds = display.bounds();
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let scale = display.pixels_wide() as f64 / bounds.size.width;
    if scale <= 0.0 {
        return None;
    }
    Some(CGPoint::new(
        bounds.origin.x + f64::from(x) / scale,
        bounds.origin.y + f64::from(y) / scale,
    ))
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn CGEventSetLocation(event: core_graphics::sys::CGEventRef, location: CGPoint);
}
