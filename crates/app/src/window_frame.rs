use core_compose::{Background, WindowCanvas, compose_window};
use core_types::{RgbaFrame, WindowId};

use crate::logging::log_error;

/// Applies the platform window's own corner radius without adding padding,
/// background, or a synthetic shadow. Windows Graphics Capture returns square
/// corners; macOS ScreenCaptureKit already returns the native rounded frame
/// with its shadow, so the pixels are left untouched there.
pub fn round_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    if platform_window::window_capture_includes_native_shadow() {
        return frame;
    }
    let scale = scale.max(0.1);
    let corner_radius = core_capture::window_corner_radius(id, scale);
    if corner_radius == 0 {
        return frame;
    }
    let canvas = WindowCanvas {
        padding: 0,
        corner_radius,
        shadow: None,
        background: Background::Transparent,
        border: None,
    };
    match compose_window(&frame, &canvas) {
        Ok(frame) => frame,
        Err(error) => {
            log_error!("window", "failed to round the window capture: {error}");
            frame
        }
    }
}
