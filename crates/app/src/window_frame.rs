use core_compose::{Background, Border, Shadow, WindowCanvas, compose_window};
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

#[cfg(target_os = "macos")]
pub fn measure_window_corner_radius(id: WindowId, scale: f32) -> Option<u32> {
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
    core_compose::estimate_corner_radius(&frame, (64.0 * scale.max(0.1)).round() as u32)
}

#[cfg(not(target_os = "macos"))]
pub fn measure_window_corner_radius(_id: WindowId, _scale: f32) -> Option<u32> {
    None
}

pub fn compose_frozen_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    compose_window_capture_measured(
        id,
        frame,
        scale,
        measure_window_corner_radius(id, scale),
        true,
    )
}

pub fn compose_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    compose_window_capture_measured(id, frame, scale, None, false)
}

fn compose_window_capture_measured(
    id: WindowId,
    frame: RgbaFrame,
    scale: f32,
    measured_radius: Option<u32>,
    edge_bleed: bool,
) -> RgbaFrame {
    let settings = settings_state::get();
    let scale = scale.max(0.1);
    #[cfg(target_os = "windows")]
    let frame = trim_capture_fringe(frame);
    let fallback_radius = platform_capture::window_corner_radius(id, scale);
    #[cfg(target_os = "macos")]
    let corner_radius = if platform_window::window_capture_includes_native_shadow() {
        fallback_radius
    } else {
        measured_radius
            .or_else(|| core_compose::estimate_corner_radius(&frame, (64.0 * scale).round() as u32))
            .unwrap_or(fallback_radius)
    };
    #[cfg(not(target_os = "macos"))]
    let corner_radius = {
        let _ = measured_radius;
        fallback_radius
    };
    let native_shadow = platform_window::window_capture_includes_native_shadow();
    let canvas = window_canvas(
        &settings,
        scale,
        corner_radius,
        native_shadow,
        edge_bleed && !native_shadow,
    );

    match compose_window(&frame, &canvas) {
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
    }
}

#[cfg(target_os = "macos")]
fn system_border(scale: f32) -> Option<Border> {
    let width = (scale.max(0.1)).round().max(1.0) as u32;
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
        let canvas = window_canvas(&AppSettings::default(), 2.0, 20, true, true);
        assert_eq!(canvas.padding, 48);
        assert_eq!(canvas.corner_radius, 0);
        assert!(canvas.shadow.is_none());
        assert!(canvas.border.is_none());
        assert!(!canvas.edge_bleed);
    }

    #[test]
    fn composed_window_shadow_adds_effects_when_native_is_absent() {
        let canvas = window_canvas(&AppSettings::default(), 2.0, 20, false, true);
        assert_eq!(canvas.padding, 48);
        assert_eq!(canvas.corner_radius, 20);
        assert!(canvas.shadow.is_some());
        assert!(canvas.edge_bleed);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(canvas.border.is_some());
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert!(canvas.border.is_none());
    }
}
