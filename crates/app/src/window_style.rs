use core_compose::{Background, Shadow, WindowCanvas, compose_window};
use core_types::{RgbaFrame, WindowId};

use crate::logging::log_error;
use crate::settings_state::{self, AppSettings, WindowBackground};

const LIGHT_CANVAS: [u8; 4] = [0xF5, 0xF5, 0xF7, 0xFF];
const DARK_CANVAS: [u8; 4] = [0x1C, 0x1C, 0x1E, 0xFF];
const SHADOW_OFFSET: f32 = 12.0;
const SHADOW_BLUR: f32 = 32.0;
const SHADOW_OPACITY: f32 = 0.35;

pub fn compose_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    let settings = settings_state::get();
    if !settings.window_composition {
        return frame;
    }

    let scale = scale.max(0.1);
    let canvas = WindowCanvas {
        padding: (settings.window_padding as f32 * scale)
            .round()
            .clamp(0.0, 2048.0) as u32,
        corner_radius: core_capture::window_corner_radius(id, scale),
        shadow: (settings.window_shadow
            && !platform_window::window_capture_includes_native_shadow())
        .then_some(Shadow {
            offset_y: (SHADOW_OFFSET * scale).round() as i32,
            blur_radius: (SHADOW_BLUR * scale).round() as u32,
            opacity: SHADOW_OPACITY,
        }),
        background: background(&settings),
        border: None,
    };

    match compose_window(&frame, &canvas) {
        Ok(frame) => frame,
        Err(error) => {
            log_error!("window", "failed to compose the window capture: {error}");
            frame
        }
    }
}

fn background(settings: &AppSettings) -> Background {
    match settings.window_background {
        WindowBackground::Transparent => Background::Transparent,
        WindowBackground::System => {
            // "Follow system" is a plain white/black canvas, not the softer
            // light/dark presets, so it reads as a neutral backdrop in either
            // appearance.
            let light = platform_window::system_uses_light_theme().unwrap_or(true);
            Background::Solid(if light {
                [0xFF, 0xFF, 0xFF, 0xFF]
            } else {
                [0x00, 0x00, 0x00, 0xFF]
            })
        }
        WindowBackground::Light => Background::Solid(LIGHT_CANVAS),
        WindowBackground::Dark => Background::Solid(DARK_CANVAS),
    }
}
