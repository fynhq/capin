use std::sync::OnceLock;

#[path = "macos_sck.rs"]
mod sck;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::access::ScreenCaptureAccess;
use core_graphics::base::{CGFloat, kCGBitmapByteOrder32Host, kCGImageAlphaPremultipliedLast};
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

        let process = i64::from(std::process::id());
        let mut result = Vec::new();
        for info in windows.iter() {
            let owner_pid = integer_field(&info, "kCGWindowOwnerPID").unwrap_or(0);
            if owner_pid == process {
                continue;
            }
            if integer_field(&info, "kCGWindowIsOnscreen").unwrap_or(1) != 1 {
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
            let scale = displays
                .iter()
                .find(|display| contains(display.point_bounds, center_x, center_y))
                .map(|display| display.scale.get())
                .unwrap_or(1.0);
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
            });
        }
        Ok(result)
    }

    fn capture(&mut self, target: CaptureTarget) -> Result<RgbaFrame> {
        ensure_permission()?;
        match sck::capture(target) {
            Ok(frame) => return Ok(frame),
            Err(error) => {
                eprintln!(
                    "capin: ScreenCaptureKit screenshot failed, falling back to CoreGraphics: {error}"
                );
            }
        }
        // Legacy CoreGraphics fallback, mainly for macOS 13 environments where
        // the ScreenCaptureKit path is unavailable.
        match target {
            CaptureTarget::Monitor(id) => capture_display(id),
            CaptureTarget::Window(id) => capture_window(id),
            CaptureTarget::Region(region) => {
                let frame = capture_display(region.monitor)?;
                crop(&frame, region.rect)
            }
        }
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
                let display = displays
                    .iter()
                    .find(|display| contains(display.point_bounds, cursor.x, cursor.y))?;
                let scale = display.scale.get();
                let window = self
                    .windows()
                    .ok()?
                    .into_iter()
                    .find(|window| window.id == id)?;
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
        let pixels_wide = u32::try_from(display.pixels_wide()).unwrap_or(0);
        let pixels_high = u32::try_from(display.pixels_high()).unwrap_or(0);
        let width = if pixels_wide > 0 {
            pixels_wide
        } else {
            point_bounds.size.width.max(1.0).round() as u32
        };
        let height = if pixels_high > 0 {
            pixels_high
        } else {
            point_bounds.size.height.max(1.0).round() as u32
        };
        let raw_scale = if point_bounds.size.width > 0.0 {
            width as f32 / point_bounds.size.width as f32
        } else {
            1.0
        };
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

/// Accessibility element enumeration is not wired up yet; callers fall back
/// to whole-window selection.
pub fn ui_elements(_id: WindowId) -> Vec<PhysicalRect> {
    Vec::new()
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
    let color_space = core_graphics::color_space::CGColorSpace::create_device_rgb();
    let mut context = CGContext::create_bitmap_context(
        None,
        width,
        height,
        8,
        width * 4,
        &color_space,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Host,
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
        && x <= bounds.origin.x + bounds.size.width
        && y <= bounds.origin.y + bounds.size.height
}
