use core_capture::CaptureTarget;
#[cfg(target_os = "windows")]
use core_capture::window_is_fully_visible;
#[cfg(target_os = "macos")]
use core_compose::AlphaMask;
use core_compose::{Background, Border, Shadow, ShapeSource, WindowCanvas, compose_window_shaped};
#[cfg(target_os = "windows")]
use core_types::CaptureRegion;
#[cfg(target_os = "windows")]
use core_types::PhysicalRect;
use core_types::{RgbaFrame, WindowId};

use crate::logging::log_error;
#[cfg(target_os = "macos")]
use crate::logging::log_warn;
use crate::settings_state::{self, AppSettings, WindowBackground};

const WHITE_CANVAS: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const SHADOW_OFFSET: f32 = 6.0;
const SHADOW_BLUR: f32 = 16.0;
const SHADOW_OPACITY: f32 = 0.35;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct WindowShapeProbe {
    alpha: Vec<u8>,
    width: u32,
    height: u32,
}

#[cfg(target_os = "macos")]
fn probe_window_shape(id: WindowId) -> Option<WindowShapeProbe> {
    use core_capture::CaptureTarget;

    if platform_window::window_capture_includes_native_shadow() {
        return None;
    }
    let mut backend = platform_capture::platform_backend();
    let frame = match backend.capture(CaptureTarget::Window(id)) {
        Ok(frame) => frame,
        Err(error) => {
            log_warn!("window", "failed to probe the window shape: {error}");
            return None;
        }
    };
    let width = frame.width();
    let height = frame.height();
    let mut alpha = vec![0u8; width as usize * height as usize];
    for y in 0..height {
        let Some(row) = frame.row(y) else {
            continue;
        };
        let start = y as usize * width as usize;
        for (index, value) in alpha[start..start + width as usize].iter_mut().enumerate() {
            *value = row[index * 4 + 3];
        }
    }
    Some(WindowShapeProbe {
        alpha,
        width,
        height,
    })
}

#[cfg(target_os = "windows")]
pub fn capture_visible_window(id: WindowId) -> Option<RgbaFrame> {
    let mut backend = platform_capture::platform_backend();
    let monitors = backend.monitors().ok()?;
    let windows = backend.windows().ok()?;
    let window = windows.iter().find(|window| window.id == id)?;
    let monitor = window
        .monitor
        .and_then(|monitor| monitors.iter().find(|info| info.id == monitor))
        .or_else(|| {
            monitors
                .iter()
                .find(|info| info.bounds.contains(window.bounds.origin))
        })?;
    if !window_is_fully_visible(&windows, monitor.bounds, id) {
        return None;
    }
    let rect = PhysicalRect::new(
        window.bounds.origin.x - monitor.bounds.origin.x,
        window.bounds.origin.y - monitor.bounds.origin.y,
        window.bounds.size.width,
        window.bounds.size.height,
    );
    backend
        .capture(CaptureTarget::Region(CaptureRegion::new(monitor.id, rect)))
        .ok()
}

pub fn capture_refresh_frame(
    target: CaptureTarget,
    scale: f32,
    cursor: bool,
) -> Result<RgbaFrame, String> {
    let CaptureTarget::Window(id) = target else {
        let mut backend = platform_capture::platform_backend();
        return if cursor {
            backend.capture_with_cursor(target)
        } else {
            backend.capture(target)
        }
        .map_err(|error| error.to_string());
    };
    #[cfg(target_os = "windows")]
    if !cursor && let Some(frame) = capture_visible_window(id) {
        return Ok(compose_frozen_window_capture(id, frame, scale));
    }
    let mut backend = platform_capture::platform_backend();
    let frame = if cursor {
        backend.capture_with_cursor(target)
    } else {
        backend.capture(target)
    }
    .map_err(|error| error.to_string())?;
    Ok(compose_window_capture(id, frame, scale))
}

pub fn compose_frozen_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    #[cfg(target_os = "macos")]
    let probe = probe_window_shape(id);
    #[cfg(not(target_os = "macos"))]
    let probe: Option<WindowShapeProbe> = None;
    compose_window_capture_measured(id, frame, scale, probe, true, false, false)
}

pub fn compose_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    #[cfg(target_os = "macos")]
    let use_source_alpha = true;
    #[cfg(not(target_os = "macos"))]
    let use_source_alpha = false;
    compose_window_capture_measured(id, frame, scale, None, false, true, use_source_alpha)
}

fn compose_window_capture_measured(
    id: WindowId,
    frame: RgbaFrame,
    scale: f32,
    probe: Option<WindowShapeProbe>,
    edge_bleed: bool,
    trim_fringe: bool,
    use_source_alpha: bool,
) -> RgbaFrame {
    let settings = settings_state::get();
    let scale = scale.max(0.1);
    #[cfg(target_os = "windows")]
    let frame = if trim_fringe {
        trim_capture_fringe(frame)
    } else {
        frame
    };
    #[cfg(not(target_os = "windows"))]
    let _ = trim_fringe;
    let fallback_radius = platform_capture::window_corner_radius(id, scale);
    let native_shadow = platform_window::window_capture_includes_native_shadow();
    #[cfg(target_os = "macos")]
    let (corner_radius, shape, edge_feather) = if native_shadow {
        (fallback_radius, ShapeSource::Rounded, false)
    } else if use_source_alpha {
        (fallback_radius, ShapeSource::WindowAlpha, false)
    } else if let Some(probe) = probe.as_ref() {
        (
            fallback_radius,
            ShapeSource::Mask(AlphaMask {
                alpha: &probe.alpha,
                width: probe.width,
                height: probe.height,
            }),
            false,
        )
    } else {
        let radius = core_compose::estimate_corner_radius(&frame, (64.0 * scale).round() as u32)
            .unwrap_or(fallback_radius);
        (radius, ShapeSource::Rounded, true)
    };
    #[cfg(not(target_os = "macos"))]
    let (corner_radius, shape, edge_feather) = {
        let _ = (probe, use_source_alpha);
        (fallback_radius, ShapeSource::Rounded, false)
    };
    let canvas = window_canvas(
        &settings,
        scale,
        corner_radius,
        native_shadow,
        edge_bleed && !native_shadow,
        edge_feather,
    );

    match compose_window_shaped(&frame, &canvas, shape) {
        Ok(frame) => frame,
        Err(error) => {
            log_error!("window", "failed to compose the window capture: {error}");
            frame
        }
    }
}

fn window_canvas(
    settings: &AppSettings,
    scale: f32,
    corner_radius: u32,
    native_shadow: bool,
    edge_bleed: bool,
    edge_feather: bool,
) -> WindowCanvas {
    let padding = (settings.window_padding as f32 * scale)
        .round()
        .clamp(0.0, 2048.0) as u32;
    if native_shadow {
        return WindowCanvas {
            padding,
            corner_radius: 0,
            shadow: None,
            background: background(settings),
            border: None,
            edge_bleed: false,
            edge_feather: false,
        };
    }
    WindowCanvas {
        padding,
        corner_radius,
        shadow: Some(Shadow {
            offset_y: (SHADOW_OFFSET * scale).round() as i32,
            blur_radius: (SHADOW_BLUR * scale).round() as u32,
            opacity: SHADOW_OPACITY,
        }),
        background: background(settings),
        border: system_border(scale),
        edge_bleed,
        edge_feather,
    }
}

#[cfg(target_os = "macos")]
fn system_border(scale: f32) -> Option<Border> {
    let width = scale.max(0.1).round().max(1.0) as u32;
    let light = platform_window::system_uses_light_theme().unwrap_or(true);
    Some(Border {
        width,
        color: if light {
            [0x00, 0x00, 0x00, 0x28]
        } else {
            [0xFF, 0xFF, 0xFF, 0x1E]
        },
        outside: false,
    })
}

#[cfg(target_os = "windows")]
fn system_border(_scale: f32) -> Option<Border> {
    let light = platform_window::system_uses_light_theme().unwrap_or(true);
    Some(Border {
        width: 1,
        color: if light {
            [0xB6, 0xB6, 0xB6, 0xFF]
        } else {
            [0x4C, 0x4C, 0x4C, 0xFF]
        },
        outside: false,
    })
}

#[cfg(target_os = "windows")]
fn trim_capture_fringe(frame: RgbaFrame) -> RgbaFrame {
    let width = frame.width();
    let height = frame.height();
    if width <= 2 || height <= 2 {
        return frame;
    }
    frame
        .into_cropped(PhysicalRect::new(1, 1, width - 2, height - 2))
        .expect("trimmed capture keeps its interior")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_border(_scale: f32) -> Option<Border> {
    None
}

fn background(settings: &AppSettings) -> Background {
    match settings.window_background {
        WindowBackground::Transparent => Background::Transparent,
        WindowBackground::White => Background::Solid(WHITE_CANVAS),
    }
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, window_canvas};

    #[test]
    fn native_window_shadow_keeps_the_native_frame_intact() {
        let canvas = window_canvas(&AppSettings::default(), 2.0, 20, true, true, true);
        assert_eq!(canvas.padding, 48);
        assert_eq!(canvas.corner_radius, 0);
        assert!(canvas.shadow.is_none());
        assert!(canvas.border.is_none());
        assert!(!canvas.edge_bleed);
        assert!(!canvas.edge_feather);
    }

    #[test]
    fn composed_window_shadow_adds_effects_when_native_is_absent() {
        let canvas = window_canvas(&AppSettings::default(), 2.0, 20, false, true, true);
        assert_eq!(canvas.padding, 48);
        assert_eq!(canvas.corner_radius, 20);
        assert!(canvas.shadow.is_some());
        assert!(canvas.edge_bleed);
        assert!(canvas.edge_feather);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(canvas.border.is_some());
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert!(canvas.border.is_none());
    }
}
