use core_compose::{Background, Border, Shadow, WindowCanvas, compose_window};
use core_types::{RgbaFrame, WindowId};

use crate::logging::log_error;
use crate::settings_state::{self, AppSettings, WindowBackground};

const WHITE_CANVAS: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const SHADOW_OFFSET: f32 = 6.0;
const SHADOW_BLUR: f32 = 16.0;
const SHADOW_OPACITY: f32 = 0.35;

/// Composes a window capture with the fixed window styling: the system corner
/// radius, a system-style drop shadow, the configured background and padding.
/// macOS captures already carry the native window shadow, so only the padding
/// and background are added there.
pub fn compose_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    let settings = settings_state::get();
    let scale = scale.max(0.1);
    let canvas = WindowCanvas {
        padding: (settings.window_padding as f32 * scale)
            .round()
            .clamp(0.0, 2048.0) as u32,
        corner_radius: core_capture::window_corner_radius(id, scale),
        shadow: (!platform_window::window_capture_includes_native_shadow()).then_some(Shadow {
            offset_y: (SHADOW_OFFSET * scale).round() as i32,
            blur_radius: (SHADOW_BLUR * scale).round() as u32,
            opacity: SHADOW_OPACITY,
        }),
        background: background(&settings),
        border: system_border(scale),
    };

    match compose_window(&frame, &canvas) {
        Ok(frame) => frame,
        Err(error) => {
            log_error!("window", "failed to compose the window capture: {error}");
            frame
        }
    }
}

/// macOS window screenshots carry a one-point hairline around the window.
/// ScreenCaptureKit's desktop-independent window image exposes only the
/// window's content, so the system outline has to be painted back on the
/// composed frame. Windows keeps its own DWM frame out of the capture and has
/// no equivalent hairline.
#[cfg(target_os = "macos")]
fn system_border(scale: f32) -> Option<Border> {
    let width = (scale.max(0.1)).round().max(1.0) as u32;
    let light = platform_window::system_uses_light_theme().unwrap_or(true);
    Some(Border {
        width,
        color: if light {
            [0x00, 0x00, 0x00, 0x28]
        } else {
            [0xFF, 0xFF, 0xFF, 0x34]
        },
    })
}

#[cfg(not(target_os = "macos"))]
fn system_border(_scale: f32) -> Option<Border> {
    None
}

fn background(settings: &AppSettings) -> Background {
    match settings.window_background {
        WindowBackground::Transparent => Background::Transparent,
        WindowBackground::White => Background::Solid(WHITE_CANVAS),
    }
}
