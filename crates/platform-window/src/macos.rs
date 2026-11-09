use std::mem::ManuallyDrop;
use std::path::PathBuf;

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_graphics::base::{CGFloat, kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast};
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use foreign_types_shared::ForeignType;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSColor, NSImage, NSScreen,
    NSScreenSaverWindowLevel, NSView, NSWindowCollectionBehavior, NSWindowSharingType,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{PlatformWindowError, PrivacyPane, WindowStyle};

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

pub(super) fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub(super) fn screen_capture_trusted() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}

pub(super) fn request_screen_capture_permission() -> bool {
    core_graphics::access::ScreenCaptureAccess.request()
}

pub(super) fn set_accessory_activation_policy() {
    set_activation_policy(NSApplicationActivationPolicy::Accessory);
}

pub(super) fn set_regular_activation_policy() {
    set_activation_policy(NSApplicationActivationPolicy::Regular);
}

fn set_activation_policy(policy: NSApplicationActivationPolicy) {
    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(marker);
    let _ = app.setActivationPolicy(policy);
    if policy == NSApplicationActivationPolicy::Regular {
        app.activate();
    }
}

pub(super) fn activate_application() {
    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(marker);
    app.activate();
}

pub(super) fn request_accessibility_permission() -> bool {
    let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}

pub(super) fn open_privacy_settings(pane: PrivacyPane) {
    // macOS Ventura moved Privacy & Security into the new Settings
    // extension; older releases still answer the legacy anchor. Try the
    // modern deep link first, then the fallback, and finally plain System
    // Settings so the click always does something visible.
    let modern = match pane {
        PrivacyPane::ScreenCapture => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        PrivacyPane::Accessibility => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility"
        }
    };
    let legacy = match pane {
        PrivacyPane::ScreenCapture => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        PrivacyPane::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
    };
    if open_url(modern) || open_url(legacy) || open_url("x-apple.systempreferences:") {
        return;
    }
    // AppKit can refuse the URL when the app is not frontmost; the `open`
    // helper asks LaunchServices directly and is the last-resort path.
    let _ = std::process::Command::new("open").arg(legacy).spawn();
}

fn open_url(url: &str) -> bool {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

pub(super) fn desktop_wallpaper_path() -> Option<PathBuf> {
    let marker = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(marker)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    Some(PathBuf::from(url.path()?.to_string()))
}

pub(super) fn desktop_wallpaper_frame() -> Option<core_types::RgbaFrame> {
    let marker = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(marker)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    let image = NSImage::initWithContentsOfURL(NSImage::alloc(), &url)?;
    let cg_image =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let borrowed = ManuallyDrop::new(unsafe {
        core_graphics::image::CGImage::from_ptr(
            objc2::rc::Retained::as_ptr(&cg_image) as *mut core_graphics::sys::CGImage
        )
    });
    cgimage_to_frame(&borrowed)
}

fn cgimage_to_frame(image: &core_graphics::image::CGImage) -> Option<core_types::RgbaFrame> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }
    let color_space = CGColorSpace::create_device_rgb();
    let mut context = CGContext::create_bitmap_context(
        None,
        width,
        height,
        8,
        width * 4,
        &color_space,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big,
    );
    context.draw_image(
        CGRect::new(
            &CGPoint::new(0.0, 0.0),
            &CGSize::new(width as CGFloat, height as CGFloat),
        ),
        image,
    );
    let mut pixels = context.data().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255) / alpha).min(255) as u8;
            }
        }
    }
    core_types::RgbaFrame::from_pixels(
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        pixels,
    )
    .ok()
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

    // A nonactivating panel never becomes the key window, which leaves a
    // full-screen capture overlay unable to receive Esc or letter shortcuts.
    // The overlay asks for a borderless normal window instead: GPUI's window
    // subclass already overrides `canBecomeKeyWindow`.
    if style.borderless {
        let content = window.contentRectForFrameRect(window.frame());
        window.setStyleMask(NSWindowStyleMask::Borderless);
        window.setFrame_display(content, true);
        window.setHasShadow(false);
    }
    if style.opaque {
        window.setOpaque(true);
        window.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.0, 0.0, 0.0, 1.0,
        )));
    }

    if style.always_on_top {
        window.setLevel(NSScreenSaverWindowLevel);
    }

    let mut behavior = window.collectionBehavior();
    behavior |= NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary;
    window.setCollectionBehavior(behavior);
    window.setAcceptsMouseMovedEvents(true);

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
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef)
    -> bool;
    fn CGEventSetLocation(event: core_graphics::sys::CGEventRef, location: CGPoint);
}
