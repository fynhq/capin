use std::path::PathBuf;

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_graphics::display::CGDisplay;
use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSColor, NSEvent,
    NSEventModifierFlags, NSRunningApplication, NSScreenSaverWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{BackdropMaterial, PlatformWindowError, PrivacyPane, WindowStyle};

pub(super) fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

pub(super) fn transparency_allowed() -> bool {
    let workspace = NSWorkspace::sharedWorkspace();
    !workspace.accessibilityDisplayShouldReduceTransparency()
        && !workspace.accessibilityDisplayShouldIncreaseContrast()
}

pub(super) fn backdrop_supported(_material: BackdropMaterial) -> bool {
    true
}

pub(super) fn set_backdrop(
    _window: &impl HasWindowHandle,
    _material: BackdropMaterial,
) -> Result<(), PlatformWindowError> {
    Ok(())
}

pub(super) fn pictures_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Pictures"))
}

pub(super) fn videos_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Movies"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(super) fn place_on_monitor(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    scale: core_types::DpiScale,
) -> Result<(), PlatformWindowError> {
    place_on_monitor_impl(window, bounds, scale, true)
}

pub(super) fn place_on_monitor_deferred(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    scale: core_types::DpiScale,
) -> Result<(), PlatformWindowError> {
    place_on_monitor_impl(window, bounds, scale, false)
}

fn place_on_monitor_impl(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    scale: core_types::DpiScale,
    display_now: bool,
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
    window.setFrame_display(frame, display_now);
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

pub(super) fn control_modifier_down() -> bool {
    NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Control)
}

pub(super) fn frontmost_other_application_pid() -> Option<i32> {
    let frontmost = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = frontmost.processIdentifier();
    (pid != std::process::id() as i32).then_some(pid)
}

pub(super) fn activate_application_by_pid(pid: i32) {
    if let Some(application) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        application.activateWithOptions(NSApplicationActivationOptions(0));
    }
}

pub(super) fn request_accessibility_permission() -> bool {
    let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}

pub(super) fn open_privacy_settings(pane: PrivacyPane) {
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

    if style.borderless {
        let content = window.contentRectForFrameRect(window.frame());
        window.setStyleMask(NSWindowStyleMask::Borderless);
        window.setFrame_display(content, true);
        window.setHasShadow(false);
        let _ = window.makeFirstResponder(Some(view.as_super()));
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

    if style.keeps_visible_when_inactive {
        window.setHidesOnDeactivate(false);
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

pub(super) fn enable_native_shadow(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: the handle points to the NSView owned by the live GPUI window.
    let view = unsafe { &*(appkit.ns_view.as_ptr() as *const NSView) };
    if let Some(window) = view.window() {
        window.setHasShadow(true);
    }
}

pub(super) fn window_capture_includes_native_shadow() -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};

    !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 14,
        minorVersion: 0,
        patchVersion: 0,
    })
}

pub(super) fn cursor_screen_position() -> Option<(i32, i32)> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let location = event.location();
    let displays = CGDisplay::active_displays().ok()?;
    let display = displays.into_iter().find(|id| {
        let bounds = CGDisplay::new(*id).bounds();
        location.x >= bounds.origin.x
            && location.y >= bounds.origin.y
            && location.x < bounds.origin.x + bounds.size.width
            && location.y < bounds.origin.y + bounds.size.height
    })?;
    let bounds = CGDisplay::new(display).bounds();
    let scale = screen_backing_scale(display)
        .or_else(|| {
            (bounds.size.width > 0.0)
                .then(|| CGDisplay::new(display).pixels_wide() as f64 / bounds.size.width)
        })
        .unwrap_or(1.0)
        .max(0.1);
    Some((
        (location.x * scale).round() as i32,
        (location.y * scale).round() as i32,
    ))
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

fn screen_backing_scale(id: u32) -> Option<f64> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let marker = MainThreadMarker::new()?;
    let main_height = CGDisplay::main().bounds().size.height;
    let point_bounds = CGDisplay::new(id).bounds();
    for screen in NSScreen::screens(marker).iter() {
        let frame = screen.frame();
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
}

thread_local! {
    static MENU_SELECTION: std::cell::Cell<Option<isize>> = const { std::cell::Cell::new(None) };
    static MENU_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    menu.setAutoenablesItems(false);
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
