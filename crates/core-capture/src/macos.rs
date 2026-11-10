use std::collections::VecDeque;
use std::ptr::NonNull;
use std::sync::OnceLock;

#[path = "macos_sck.rs"]
mod sck;

use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFArray as AxArray, CFRetained, CFString as AxString, CFType as AxType, CGPoint as AxPoint,
    CGRect as AxRect, CGSize as AxSize,
};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::access::ScreenCaptureAccess;
use core_graphics::base::{CGFloat, kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast};
use core_graphics::context::CGContext;
use core_graphics::display::CGDisplay;
use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::image::CGImage;
use core_graphics::window::{
    CGWindowID, create_image, kCGNullWindowID, kCGWindowImageBestResolution,
    kCGWindowImageBoundsIgnoreFraming, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionIncludingWindow, kCGWindowListOptionOnScreenOnly,
};
use core_types::{DpiScale, MonitorId, PhysicalRect, RgbaFrame, WindowId};

use crate::{CaptureBackend, CaptureError, CaptureTarget, MonitorInfo, Result, WindowInfo};

pub struct MacosCapture;

impl CaptureBackend for MacosCapture {
    fn monitors(&mut self) -> Result<Vec<MonitorInfo>> {
        Ok(displays()?
            .into_iter()
            .map(|display| MonitorInfo {
                id: MonitorId::from_raw(u64::from(display.id)),
                bounds: display.physical,
                scale: display.scale,
                is_primary: display.is_primary,
            })
            .collect())
    }

    fn windows(&mut self) -> Result<Vec<WindowInfo>> {
        let displays = displays()?;
        let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
        let raw =
            unsafe { core_graphics::window::CGWindowListCopyWindowInfo(option, kCGNullWindowID) };
        if raw.is_null() {
            return Ok(Vec::new());
        }
        let windows: CFArray<CFDictionary<CFString, CFType>> =
            unsafe { CFArray::wrap_under_create_rule(raw) };

        let mut result = Vec::new();
        for info in windows.iter() {
            if integer_field(&info, "kCGWindowIsOnscreen").unwrap_or(1) != 1 {
                continue;
            }
            // Windows with sharing disabled (capture overlays, capture bars
            // and pin menus) cannot appear in a screenshot, so they must not
            // be hover targets.
            if integer_field(&info, "kCGWindowSharingState").unwrap_or(1) == 0 {
                continue;
            }
            if integer_field(&info, "kCGWindowLayer").unwrap_or(0) != 0 {
                continue;
            }
            if number_field(&info, "kCGWindowAlpha").unwrap_or(1.0) <= 0.01 {
                continue;
            }
            let Some(number) = integer_field(&info, "kCGWindowNumber") else {
                continue;
            };
            let Some((x, y, width, height)) = bounds_field(&info) else {
                continue;
            };
            if width < 2.0 || height < 2.0 {
                continue;
            }
            let center_x = x + width / 2.0;
            let center_y = y + height / 2.0;
            let display = displays
                .iter()
                .find(|display| contains(display.point_bounds, center_x, center_y));
            let (monitor, scale) = display
                .map(|display| {
                    (
                        Some(MonitorId::from_raw(u64::from(display.id))),
                        display.scale.get(),
                    )
                })
                .unwrap_or((None, 1.0));
            let bounds = PhysicalRect::new(
                (x * f64::from(scale)).round() as i32,
                (y * f64::from(scale)).round() as i32,
                (width * f64::from(scale)).round().max(1.0) as u32,
                (height * f64::from(scale)).round().max(1.0) as u32,
            );
            let title = string_field(&info, "kCGWindowName")
                .filter(|title| !title.is_empty())
                .or_else(|| string_field(&info, "kCGWindowOwnerName"))
                .unwrap_or_default();
            result.push(WindowInfo {
                id: WindowId::from_raw(number as u64),
                bounds,
                title,
                monitor,
                scale: DpiScale::new(scale).unwrap_or(DpiScale::ONE),
            });
        }
        Ok(result)
    }

    fn capture(&mut self, target: CaptureTarget) -> Result<RgbaFrame> {
        ensure_permission()?;
        let sck_error = match sck::capture(target) {
            Ok(frame) => return Ok(frame),
            Err(error) => {
                eprintln!(
                    "capin: ScreenCaptureKit screenshot failed, falling back to CoreGraphics: {error}"
                );
                error.to_string()
            }
        };
        // Legacy CoreGraphics fallback, mainly for macOS 13 environments where
        // the ScreenCaptureKit path is unavailable.
        let fallback = match target {
            CaptureTarget::Monitor(id) => capture_display(id),
            CaptureTarget::Window(id) => capture_window(id),
            CaptureTarget::Region(region) => {
                capture_display(region.monitor).and_then(|frame| crop(&frame, region.rect))
            }
        };
        fallback.map_err(|error| CaptureError::Platform {
            operation: "ScreenCaptureKit+CoreGraphics",
            message: format!("ScreenCaptureKit: {sck_error}; fallback: {error}"),
        })
    }

    fn capture_with_cursor(&mut self, target: CaptureTarget) -> Result<RgbaFrame> {
        let frame = self.capture(target)?;
        let Some(cursor) = cursor_point() else {
            return Ok(frame);
        };
        let Some((x, y, scale)) = self.cursor_local(target, cursor) else {
            return Ok(frame);
        };
        Ok(draw_cursor_arrow(frame, x, y, scale))
    }
}

impl MacosCapture {
    fn cursor_local(&mut self, target: CaptureTarget, cursor: CGPoint) -> Option<(i32, i32, f32)> {
        let displays = displays().ok()?;
        match target {
            CaptureTarget::Monitor(id) => {
                let display = displays
                    .iter()
                    .find(|display| u64::from(display.id) == id.raw())?;
                if !contains(display.point_bounds, cursor.x, cursor.y) {
                    return None;
                }
                let scale = display.scale.get();
                Some((
                    ((cursor.x - display.point_bounds.origin.x) * f64::from(scale)).round() as i32,
                    ((cursor.y - display.point_bounds.origin.y) * f64::from(scale)).round() as i32,
                    scale,
                ))
            }
            CaptureTarget::Region(region) => {
                let monitor_id = u32::try_from(region.monitor.raw()).ok()?;
                let display = displays
                    .iter()
                    .find(|display| display.id == monitor_id)
                    .or_else(|| {
                        displays
                            .iter()
                            .find(|display| contains(display.point_bounds, cursor.x, cursor.y))
                    })?;
                if !contains(display.point_bounds, cursor.x, cursor.y) {
                    return None;
                }
                let scale = display.scale.get();
                let local_x =
                    ((cursor.x - display.point_bounds.origin.x) * f64::from(scale)).round() as i32;
                let local_y =
                    ((cursor.y - display.point_bounds.origin.y) * f64::from(scale)).round() as i32;
                let rect = region.rect;
                if local_x < rect.origin.x
                    || local_y < rect.origin.y
                    || local_x >= rect.origin.x + rect.size.width as i32
                    || local_y >= rect.origin.y + rect.size.height as i32
                {
                    return None;
                }
                Some((local_x - rect.origin.x, local_y - rect.origin.y, scale))
            }
            CaptureTarget::Window(id) => {
                let window = self
                    .windows()
                    .ok()?
                    .into_iter()
                    .find(|window| window.id == id)?;
                let scale = window.scale.get();
                let origin_x = f64::from(window.bounds.origin.x) / f64::from(scale);
                let origin_y = f64::from(window.bounds.origin.y) / f64::from(scale);
                let x = ((cursor.x - origin_x) * f64::from(scale)).round() as i32;
                let y = ((cursor.y - origin_y) * f64::from(scale)).round() as i32;
                if x < 0
                    || y < 0
                    || x >= window.bounds.size.width as i32
                    || y >= window.bounds.size.height as i32
                {
                    return None;
                }
                Some((x, y, scale))
            }
        }
    }
}

fn cursor_point() -> Option<CGPoint> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    Some(event.location())
}

fn draw_cursor_arrow(frame: RgbaFrame, x: i32, y: i32, scale: f32) -> RgbaFrame {
    let scale = scale.clamp(1.0, 3.0);
    let points = [
        (0.0, 0.0),
        (0.0, 18.0),
        (5.0, 14.0),
        (8.0, 21.0),
        (11.0, 19.0),
        (8.0, 13.0),
        (14.0, 13.0),
    ];
    let points: Vec<(f32, f32)> = points
        .iter()
        .map(|(px, py)| (x as f32 + px * scale, y as f32 + py * scale))
        .collect();
    let min_x = points
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::INFINITY, f32::min)
        .floor() as i32;
    let max_x = points
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i32;
    let min_y = points
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::INFINITY, f32::min)
        .floor() as i32;
    let max_y = points
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i32;
    let mut pixels = frame.pixels().to_vec();
    let width = frame.width() as i32;
    let height = frame.height() as i32;
    for py in min_y.max(0)..=max_y.min(height - 1) {
        for px in min_x.max(0)..=max_x.min(width - 1) {
            if !point_in_polygon(px as f32 + 0.5, py as f32 + 0.5, &points) {
                continue;
            }
            let border = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                .iter()
                .any(|(dx, dy)| {
                    !point_in_polygon(
                        px as f32 + 0.5 + dx * scale,
                        py as f32 + 0.5 + dy * scale,
                        &points,
                    )
                });
            let color = if border {
                [0u8, 0, 0, 255]
            } else {
                [255u8, 255, 255, 255]
            };
            let offset = (py as usize * width as usize + px as usize) * 4;
            if let Some(target) = pixels.get_mut(offset..offset + 4) {
                target.copy_from_slice(&color);
            }
        }
    }
    RgbaFrame::from_pixels(frame.width(), frame.height(), pixels).unwrap_or(frame)
}

fn point_in_polygon(x: f32, y: f32, points: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (x1, y1) = points[current];
        let (x2, y2) = points[previous];
        if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

struct DisplayGeometry {
    id: u32,
    point_bounds: CGRect,
    scale: DpiScale,
    physical: PhysicalRect,
    is_primary: bool,
}

fn displays() -> Result<Vec<DisplayGeometry>> {
    let ids = CGDisplay::active_displays().map_err(|error| CaptureError::Platform {
        operation: "CGDisplay::active_displays",
        message: format!("{error:?}"),
    })?;
    let main = CGDisplay::main().id;
    let mut displays = Vec::new();
    for id in ids {
        if id == 0 {
            continue;
        }
        let display = CGDisplay::new(id);
        let point_bounds = display.bounds();
        // GPUI renders every window at the screen's backing scale, and macOS
        // screenshots use the same backing-store pixels. Deriving the pixel
        // space from NSScreen keeps capture, the frozen overlay and window
        // placement in one coordinate system; CGDisplayPixelsWide only
        // describes the panel and can differ on scaled HiDPI modes.
        let scale = screen_backing_scale(id).unwrap_or_else(|| {
            let pixels_wide = u32::try_from(display.pixels_wide()).unwrap_or(0);
            if point_bounds.size.width > 0.0 && pixels_wide > 0 {
                f64::from(pixels_wide) / point_bounds.size.width
            } else {
                1.0
            }
        });
        let width = (point_bounds.size.width * scale).round().max(1.0) as u32;
        let height = (point_bounds.size.height * scale).round().max(1.0) as u32;
        let raw_scale = scale as f32;
        let scale = DpiScale::new(raw_scale).unwrap_or(DpiScale::ONE);
        let physical = PhysicalRect::new(
            (point_bounds.origin.x * f64::from(scale.get())).round() as i32,
            (point_bounds.origin.y * f64::from(scale.get())).round() as i32,
            width,
            height,
        );
        displays.push(DisplayGeometry {
            id,
            point_bounds,
            scale,
            physical,
            is_primary: id == main,
        });
    }
    Ok(displays)
}

/// Backing scale of the NSScreen that hosts a CGDirectDisplayID.
///
/// AppKit is the source of truth for the scale GPUI uses; the display-mode
/// width can be reported in either points or pixels depending on the OS and
/// display, so it is only used as a fallback elsewhere.
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

/// Returns the backing-store pixel scale of a display.
///
/// On HiDPI scaled modes this is the window server's render scale (usually
/// 2.0), not the panel's native pixel ratio. Capture targets must use it so
/// the frozen frame lines up one-to-one with GPUI's surface.
pub fn display_backing_scale(raw: u64) -> Option<f32> {
    let id = u32::try_from(raw).ok()?;
    screen_backing_scale(id).map(|scale| scale as f32)
}

/// Backing-store pixel size of a display, used as the capture output size.
pub(super) fn display_pixel_size(id: u32) -> Option<(usize, usize)> {
    let scale = screen_backing_scale(id)?;
    let point_bounds = CGDisplay::new(id).bounds();
    let width = (point_bounds.size.width * scale).round().max(1.0) as usize;
    let height = (point_bounds.size.height * scale).round().max(1.0) as usize;
    (width > 0 && height > 0).then_some((width, height))
}

fn capture_display(id: MonitorId) -> Result<RgbaFrame> {
    let display_id = u32::try_from(id.raw()).map_err(|_| CaptureError::MonitorNotFound(id))?;
    let available = CGDisplay::active_displays()
        .map_err(|error| CaptureError::Platform {
            operation: "CGDisplay::active_displays",
            message: format!("{error:?}"),
        })?
        .contains(&display_id);
    if !available {
        return Err(CaptureError::MonitorNotFound(id));
    }
    let image = CGDisplay::new(display_id)
        .image()
        .ok_or(CaptureError::Platform {
            operation: "CGDisplayCreateImage",
            message: "the system returned no image".to_owned(),
        })?;
    image_to_frame(&image)
}

/// AppKit uses a 10-point corner radius for standard windows.
pub fn window_corner_radius(_id: WindowId, scale: f32) -> u32 {
    (10.0 * scale.max(0.0)).round() as u32
}

const MAX_AX_ELEMENTS: usize = 512;
const MAX_AX_VISITS: usize = 1024;
const MAX_AX_DEPTH: usize = 32;

struct AxWindowInfo {
    pid: i32,
    bounds: AxRect,
    title: String,
    scale: f64,
}

/// Returns the accessibility element bounds inside a window, in the same
/// physical desktop coordinates as [`WindowInfo`].
///
/// The caller must grant Accessibility permission; without it macOS hides the
/// element tree and the overlay falls back to whole-window targeting.
pub fn ui_elements(id: WindowId) -> Vec<PhysicalRect> {
    if !unsafe { AXIsProcessTrusted() } {
        return Vec::new();
    }
    let Some(info) = ax_window_info(id) else {
        return Vec::new();
    };
    let application = unsafe { AXUIElement::new_application(info.pid) };
    unsafe { application.set_messaging_timeout(0.25) };
    let Some(window) = ax_window(&application, &info) else {
        return Vec::new();
    };
    unsafe { window.set_messaging_timeout(0.25) };

    let window_rect = ax_physical_rect(info.bounds, info.scale);
    let mut result = Vec::new();
    let mut queue: VecDeque<(CFRetained<AXUIElement>, usize)> = VecDeque::new();
    for child in ax_children(&window) {
        queue.push_back((child, 1));
    }

    let started = std::time::Instant::now();
    let mut visits = 0usize;
    while let Some((element, depth)) = queue.pop_front() {
        visits += 1;
        if visits > MAX_AX_VISITS
            || result.len() >= MAX_AX_ELEMENTS
            || started.elapsed() > std::time::Duration::from_millis(300)
        {
            break;
        }
        if let Some(frame) = ax_frame(&element) {
            let rect = ax_physical_rect(frame, info.scale);
            if rect.size.width > 1
                && rect.size.height > 1
                && rect.intersection(window_rect).is_some()
            {
                result.push(rect);
            }
        }
        if depth >= MAX_AX_DEPTH {
            continue;
        }
        for child in ax_children(&element) {
            if queue.len() + result.len() < MAX_AX_VISITS {
                queue.push_back((child, depth + 1));
            }
        }
    }
    result
}

fn ax_window_info(id: WindowId) -> Option<AxWindowInfo> {
    let raw = unsafe {
        core_graphics::window::CGWindowListCopyWindowInfo(
            kCGWindowListOptionIncludingWindow,
            u32::try_from(id.raw()).ok()? as CGWindowID,
        )
    };
    if raw.is_null() {
        return None;
    }
    let windows: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_create_rule(raw) };
    let info = windows.iter().next()?;
    let pid = integer_field(&info, "kCGWindowOwnerPID")?.try_into().ok()?;
    let (x, y, width, height) = bounds_field(&info)?;
    let title = string_field(&info, "kCGWindowName").unwrap_or_default();
    let (center_x, center_y) = (x + width / 2.0, y + height / 2.0);
    let scale = displays()
        .ok()?
        .into_iter()
        .find(|display| contains(display.point_bounds, center_x, center_y))
        .map_or(1.0, |display| f64::from(display.scale.get()));
    Some(AxWindowInfo {
        pid,
        bounds: AxRect::new(
            AxPoint::new(x, y),
            AxSize::new(width.max(1.0), height.max(1.0)),
        ),
        title,
        scale,
    })
}

fn ax_window(application: &AXUIElement, info: &AxWindowInfo) -> Option<CFRetained<AXUIElement>> {
    let windows = ax_elements(application, "AXWindows");
    let single = windows.len() == 1;
    let mut fallback = None;
    for window in windows {
        let matches_frame =
            ax_frame(&window).is_some_and(|frame| ax_frames_overlap(frame, info.bounds));
        let matches_title = !info.title.is_empty()
            && ax_string(&window, "AXTitle").is_some_and(|title| title == info.title.as_str());
        if matches_frame || matches_title {
            return Some(window);
        }
        if single {
            fallback = Some(window);
        }
    }
    if let Some(focused) = ax_attribute(application, "AXFocusedWindow")
        .and_then(|value| value.downcast::<AXUIElement>().ok())
        && ax_frame(&focused).is_some_and(|frame| ax_frames_overlap(frame, info.bounds))
    {
        return Some(focused);
    }
    fallback
}

fn ax_children(element: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    let children = ax_elements(element, "AXChildren");
    if children.is_empty() {
        ax_elements(element, "AXContents")
    } else {
        children
    }
}

fn ax_elements(element: &AXUIElement, name: &str) -> Vec<CFRetained<AXUIElement>> {
    let Some(value) = ax_attribute(element, name) else {
        return Vec::new();
    };
    let Some(array) = value.downcast_ref::<AxArray>() else {
        return Vec::new();
    };
    // SAFETY: accessibility attribute arrays always contain CFType values.
    let array = unsafe { array.cast_unchecked::<AxType>() };
    array
        .iter()
        .filter_map(|item| item.downcast::<AXUIElement>().ok())
        .collect()
}

fn ax_attribute(element: &AXUIElement, name: &str) -> Option<CFRetained<AxType>> {
    let name = AxString::from_str(name);
    let mut value: *const AxType = std::ptr::null();
    let error = unsafe { element.copy_attribute_value(&name, NonNull::from(&mut value)) };
    if error != AXError::Success || value.is_null() {
        return None;
    }
    NonNull::new(value as *mut AxType).map(|pointer| unsafe { CFRetained::from_raw(pointer) })
}

fn ax_string(element: &AXUIElement, name: &str) -> Option<String> {
    ax_attribute(element, name)?
        .downcast_ref::<AxString>()
        .map(ToString::to_string)
}

fn ax_frame(element: &AXUIElement) -> Option<AxRect> {
    if let Some(frame) = ax_attribute(element, "AXFrame")
        && let Some(value) = frame.downcast_ref::<AXValue>()
    {
        let mut rect = std::mem::MaybeUninit::<AxRect>::uninit();
        let pointer = NonNull::new(rect.as_mut_ptr().cast::<std::ffi::c_void>())?;
        if unsafe { value.value(AXValueType::CGRect, pointer) } {
            return Some(unsafe { rect.assume_init() });
        }
    }

    let position = ax_attribute(element, "AXPosition")
        .and_then(|value| value.downcast_ref::<AXValue>().and_then(ax_value_point))?;
    let size = ax_attribute(element, "AXSize")
        .and_then(|value| value.downcast_ref::<AXValue>().and_then(ax_value_size))?;
    Some(AxRect::new(position, size))
}

fn ax_value_point(value: &AXValue) -> Option<AxPoint> {
    let mut point = std::mem::MaybeUninit::<AxPoint>::uninit();
    let pointer = NonNull::new(point.as_mut_ptr().cast::<std::ffi::c_void>())?;
    if unsafe { value.value(AXValueType::CGPoint, pointer) } {
        Some(unsafe { point.assume_init() })
    } else {
        None
    }
}

fn ax_value_size(value: &AXValue) -> Option<AxSize> {
    let mut size = std::mem::MaybeUninit::<AxSize>::uninit();
    let pointer = NonNull::new(size.as_mut_ptr().cast::<std::ffi::c_void>())?;
    if unsafe { value.value(AXValueType::CGSize, pointer) } {
        Some(unsafe { size.assume_init() })
    } else {
        None
    }
}

fn ax_physical_rect(rect: AxRect, scale: f64) -> PhysicalRect {
    let width = (rect.size.width * scale).round().max(1.0) as u32;
    let height = (rect.size.height * scale).round().max(1.0) as u32;
    PhysicalRect::new(
        (rect.origin.x * scale).round() as i32,
        (rect.origin.y * scale).round() as i32,
        width,
        height,
    )
}

fn ax_frames_overlap(a: AxRect, b: AxRect) -> bool {
    let left = a.origin.x.max(b.origin.x);
    let top = a.origin.y.max(b.origin.y);
    let right = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let bottom = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    let intersection = (right - left).max(0.0) * (bottom - top).max(0.0);
    let smaller = (a.size.width * a.size.height).min(b.size.width * b.size.height);
    smaller > 0.0 && intersection / smaller >= 0.85
}

fn capture_window(id: WindowId) -> Result<RgbaFrame> {
    let window_id = u32::try_from(id.raw()).map_err(|_| CaptureError::WindowNotFound(id))?;
    let image = create_image(
        CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(0.0, 0.0)),
        kCGWindowListOptionOnScreenOnly | kCGWindowListOptionIncludingWindow,
        window_id as CGWindowID,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution,
    )
    .ok_or(CaptureError::WindowNotFound(id))?;
    image_to_frame(&image)
}

fn image_to_frame(image: &CGImage) -> Result<RgbaFrame> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err(CaptureError::Platform {
            operation: "CGImage",
            message: "captured image is empty".to_owned(),
        });
    }
    // Convert into explicit sRGB instead of the device-dependent RGB space.
    // ScreenCaptureKit can hand back Display P3 frames; device RGB resolves to
    // the display profile, and leaving the pixels in a wide-gamut space makes
    // the pinned preview and exported PNG look washed out next to the source.
    let color_space = core_graphics::color_space::CGColorSpace::create_with_name(unsafe {
        core_graphics::color_space::kCGColorSpaceSRGB
    })
    .unwrap_or_else(core_graphics::color_space::CGColorSpace::create_device_rgb);
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
    unpremultiply(&mut pixels);
    Ok(RgbaFrame::from_pixels(
        u32::try_from(width).unwrap_or(0),
        u32::try_from(height).unwrap_or(0),
        pixels,
    )?)
}

fn crop(frame: &RgbaFrame, region: PhysicalRect) -> Result<RgbaFrame> {
    if region.is_empty() {
        return Err(CaptureError::EmptyRegion);
    }
    let x0 = i64::from(region.origin.x);
    let y0 = i64::from(region.origin.y);
    let x1 = region.right();
    let y1 = region.bottom();
    if x0 < 0 || y0 < 0 || x1 > i64::from(frame.width()) || y1 > i64::from(frame.height()) {
        return Err(CaptureError::RegionOutOfBounds);
    }
    let row_bytes = region.size.width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * region.size.height as usize);
    for row in y0..y1 {
        let start = (row as usize * frame.width() as usize + x0 as usize) * 4;
        pixels.extend_from_slice(&frame.pixels()[start..start + row_bytes]);
    }
    Ok(RgbaFrame::from_pixels(
        region.size.width,
        region.size.height,
        pixels,
    )?)
}

fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel.fill(0);
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255) / alpha).min(255) as u8;
            }
        }
    }
}

fn ensure_permission() -> Result<()> {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    let access = ScreenCaptureAccess;
    if access.preflight() {
        return Ok(());
    }
    if REQUESTED.get().is_none() {
        let granted = access.request();
        let _ = REQUESTED.set(granted);
        if granted {
            return Ok(());
        }
    }
    Err(CaptureError::PermissionDenied)
}

/// Window ids whose owner disabled sharing (`NSWindowSharingType::None`).
///
/// The capture helpers (overlay, capture bars, inline editor and pin menus)
/// are the only Capin windows that set it. ScreenCaptureKit excludes those
/// windows so they never appear in a frozen frame, while pins and the
/// settings page must stay capturable and visible.
pub(super) fn capture_excluded_window_ids() -> Vec<u32> {
    let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let raw = unsafe { core_graphics::window::CGWindowListCopyWindowInfo(option, kCGNullWindowID) };
    if raw.is_null() {
        return Vec::new();
    }
    let windows: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_create_rule(raw) };
    let mut ids = Vec::new();
    for info in windows.iter() {
        if integer_field(&info, "kCGWindowSharingState").unwrap_or(1) != 0 {
            continue;
        }
        if let Some(id) = integer_field(&info, "kCGWindowNumber")
            && let Ok(id) = u32::try_from(id)
        {
            ids.push(id);
        }
    }
    ids
}

fn integer_field(info: &CFDictionary<CFString, CFType>, key: &str) -> Option<i64> {
    let key = CFString::new(key);
    info.find(&key)?.downcast::<CFNumber>()?.to_i64()
}

fn number_field(info: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
    let key = CFString::new(key);
    info.find(&key)?.downcast::<CFNumber>()?.to_f64()
}

fn string_field(info: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
    let key = CFString::new(key);
    Some(info.find(&key)?.downcast::<CFString>()?.to_string())
}

fn bounds_field(info: &CFDictionary<CFString, CFType>) -> Option<(f64, f64, f64, f64)> {
    let key = CFString::new("kCGWindowBounds");
    let untyped = info.find(&key)?.downcast::<CFDictionary>()?;
    // The bounds value is a nested dictionary; re-wrap it with typed keys so
    // the numeric fields can be read through the safe dictionary API.
    let bounds: CFDictionary<CFString, CFType> =
        unsafe { TCFType::wrap_under_get_rule(untyped.as_concrete_TypeRef()) };
    Some((
        number_field(&bounds, "X")?,
        number_field(&bounds, "Y")?,
        number_field(&bounds, "Width")?,
        number_field(&bounds, "Height")?,
    ))
}

fn contains(bounds: CGRect, x: f64, y: f64) -> bool {
    x >= bounds.origin.x
        && y >= bounds.origin.y
        && x < bounds.origin.x + bounds.size.width
        && y < bounds.origin.y + bounds.size.height
}
