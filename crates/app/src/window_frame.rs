use core_compose::{Background, Border, Shadow, WindowCanvas, compose_window};
#[cfg(target_os = "windows")]
use core_types::PhysicalRect;
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
    // WGC returns the window surface with a 1px fringe of non-client pixels.
    // Painting the old light border over it left a white-looking frame on
    // dark windows; trim the fringe instead and keep the visible edge clean.
    #[cfg(target_os = "windows")]
    let frame = trim_capture_fringe(frame);
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
/// composed frame.
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
        outside: false,
    })
}

/// Windows window screenshots keep a 1px system border. Paint it inside the
/// captured content so the composed image does not grow an extra pixel beyond
/// the window edge; the shadow still starts at that edge.
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

/// Removes the non-client fringe Windows Graphics Capture includes around a
/// window surface. The fringe reads as a light or dark hairline once the
/// capture is placed on a background, so every window screenshot would
/// otherwise get a visible 1px frame.
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
