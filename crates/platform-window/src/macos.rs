use std::path::PathBuf;

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use foreign_types_shared::ForeignType;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSColor, NSScreenSaverWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
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

/// Moves and resizes a window to a monitor's physical rectangle.
///
/// `NSWindow` frames use AppKit global points with a bottom-left origin, while
/// the caller's rectangle is in the monitor's physical pixel space with a
/// top-left origin. Dividing by the monitor scale and flipping around the main
/// display height keeps mixed-DPI desktops unambiguous.
pub(super) fn place_on_monitor(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    scale: core_types::DpiScale,
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

    let scale = f64::from(scale.get().max(0.1));
    let x = f64::from(bounds.origin.x) / scale;
    let top = f64::from(bounds.origin.y) / scale;
    let width = f64::from(bounds.size.width) / scale;
    let height = f64::from(bounds.size.height) / scale;
    let main_height = CGDisplay::main().bounds().size.height;
    let frame = NSRect::new(
        NSPoint::new(x, main_height - (top + height)),
        NSSize::new(width, height),
    );
    window.setFrame_display(frame, true);
    Ok(())
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
    // Points come from the window-server backing store (for example 2x the
    // reported menu-bar points). AppKit is the source of truth for the scale;
    // the display mode is only a fallback because its width can be reported in
    // either points or pixels depending on the OS and display.
    let scale = screen_backing_scale(display_id)
        .or_else(|| {
            display.display_mode().and_then(|mode| {
                let width = mode.width();
                (width > 0).then(|| mode.pixel_width() as f64 / width as f64)
            })
        })
        .unwrap_or_else(|| display.pixels_wide() as f64 / bounds.size.width);
    if scale <= 0.0 {
        return None;
    }
    Some(CGPoint::new(
        bounds.origin.x + f64::from(x) / scale,
        bounds.origin.y + f64::from(y) / scale,
    ))
}

/// Backing scale of the NSScreen that hosts a CGDirectDisplayID.
fn screen_backing_scale(id: u32) -> Option<f64> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let marker = MainThreadMarker::new()?;
    let main_height = CGDisplay::main().bounds().size.height;
    let point_bounds = CGDisplay::new(id).bounds();
    for screen in NSScreen::screens(marker).iter() {
        let frame = screen.frame();
        // NSScreen frames use a bottom-left origin; CGDisplay bounds are
        // top-left relative to the primary display.
        let top = main_height - (frame.origin.y + frame.size.height);
        if (frame.origin.x - point_bounds.origin.x).abs() <= 1.0
            && (top - point_bounds.origin.y).abs() <= 1.0
            && (frame.size.width - point_bounds.size.width).abs() <= 1.0
            && (frame.size.height - point_bounds.size.height).abs() <= 1.0
        {
            return Some(screen.backingScaleFactor());
        }
    }
    None
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef)
    -> bool;
    fn CGEventSetLocation(event: core_graphics::sys::CGEventRef, location: CGPoint);
}

// ---------------------------------------------------------------------------
// Native dropdown menus
// ---------------------------------------------------------------------------

thread_local! {
    static MENU_SELECTION: std::cell::Cell<Option<isize>> = const { std::cell::Cell::new(None) };
    static MENU_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Keeps a second popup from nesting inside an already tracking menu.
struct MenuGuard;

impl MenuGuard {
    fn acquire() -> Option<Self> {
        MENU_OPEN.with(|open| (!open.replace(true)).then_some(Self))
    }
}

impl Drop for MenuGuard {
    fn drop(&mut self) {
        MENU_OPEN.with(|open| open.set(false));
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CapinSettingsMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(selectItem:))]
        fn select_item(&self, sender: Option<&objc2_app_kit::NSMenuItem>) {
            if let Some(item) = sender {
                MENU_SELECTION.with(|slot| slot.set(Some(item.tag())));
            }
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        // SAFETY: MenuTarget does not override init; this dispatches to
        // NSObject's init.
        unsafe { msg_send![this, init] }
    }
}

/// Shows a native menu at the pointer and returns the chosen index. The
/// current item carries the system checkmark. `None` means the menu was
/// dismissed, another menu is already open, or no window was available.
pub(super) fn show_native_menu(items: &[String], selected: usize) -> Option<usize> {
    use objc2_app_kit::{
        NSAppearanceCustomization, NSControlStateValueOn, NSEvent, NSMenu, NSMenuItem,
    };

    if items.is_empty() {
        return None;
    }
    let _guard = MenuGuard::acquire()?;
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let window = app.keyWindow().or_else(|| app.mainWindow())?;
    let view = window.contentView()?;

    let menu = NSMenu::new(mtm);
    // Every item has an explicit target/action below; automatic enabling
    // would otherwise walk the responder chain and gray them out.
    menu.setAutoenablesItems(false);
    // A menu raised from a vibrant window can inherit the button's
    // appearance and render dark in light mode. Follow the app's effective
    // appearance, like the app's other native menus.
    let appearance = app.effectiveAppearance();
    menu.setAppearance(Some(&appearance));

    let target = MenuTarget::new(mtm);
    let target_ref: &MenuTarget = &target;
    for (index, label) in items.iter().enumerate() {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(label));
        // SAFETY: the selector is implemented by MenuTarget above.
        unsafe { item.setAction(Some(sel!(selectItem:))) };
        // SAFETY: target_ref stays alive through the nested tracking loop.
        unsafe { item.setTarget(Some(target_ref)) };
        item.setTag(index as isize);
        if index == selected {
            item.setState(NSControlStateValueOn);
        }
        menu.addItem(&item);
    }

    // `mouseLocation` is in screen coordinates; convert through the window
    // into its content view, the space `popUpMenuPositioningItem` expects.
    let screen_point = NSEvent::mouseLocation();
    let window_point = window.convertPointFromScreen(screen_point);
    let view_point = view.convertPoint_fromView(window_point, None);
    let view_ref: &NSView = &view;

    MENU_SELECTION.with(|slot| slot.set(None));
    let _ = menu.popUpMenuPositioningItem_atLocation_inView(None, view_point, Some(view_ref));
    MENU_SELECTION
        .with(|slot| slot.take())
        .and_then(|tag| usize::try_from(tag).ok())
}
