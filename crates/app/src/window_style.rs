use std::path::Path;

use core_compose::{Background, Shadow, WindowCanvas, compose_window};
use core_types::{RgbaFrame, WindowId};

use crate::settings_state::{self, AppSettings, WindowBackground};

const LIGHT_CANVAS: [u8; 4] = [0xF5, 0xF5, 0xF7, 0xFF];
const DARK_CANVAS: [u8; 4] = [0x1C, 0x1C, 0x1E, 0xFF];
const FALLBACK_ACCENT: u32 = 0x4C9AFF;
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
    };

    match compose_window(&frame, &canvas) {
        Ok(frame) => frame,
        Err(error) => {
            eprintln!("capin: failed to compose the window capture: {error}");
            frame
        }
    }
}

fn background(settings: &AppSettings) -> Background {
    match settings.window_background {
        WindowBackground::Transparent => Background::Transparent,
        WindowBackground::Light => Background::Solid(LIGHT_CANVAS),
        WindowBackground::Dark => Background::Solid(DARK_CANVAS),
        WindowBackground::Gradient => gradient(),
        WindowBackground::Wallpaper => {
            wallpaper(settings)
                .map(Background::Image)
                .unwrap_or_else(|| {
                    eprintln!("capin: window wallpaper is unavailable; using a transparent canvas");
                    Background::Transparent
                })
        }
    }
}

/// A custom image wins; otherwise the current system wallpaper is used, so
/// choosing "Wallpaper" works without asking the user to pick a file first.
fn wallpaper(settings: &AppSettings) -> Option<RgbaFrame> {
    if let Some(path) = settings.window_wallpaper.as_deref()
        && let Some(frame) = load_wallpaper(path)
    {
        return Some(frame);
    }
    platform_window::desktop_wallpaper_path()
        .as_deref()
        .and_then(load_wallpaper)
        .or_else(platform_window::desktop_wallpaper_frame)
}

fn gradient() -> Background {
    let accent = platform_window::system_accent_hex().unwrap_or(FALLBACK_ACCENT);
    let red = ((accent >> 16) & 0xFF) as f32;
    let green = ((accent >> 8) & 0xFF) as f32;
    let blue = (accent & 0xFF) as f32;
    let top = [
        red.round() as u8,
        green.round() as u8,
        blue.round() as u8,
        0xFF,
    ];
    let bottom = [
        (red * 0.35).round() as u8,
        (green * 0.35).round() as u8,
        (blue * 0.35).round() as u8,
        0xFF,
    ];
    Background::VerticalGradient(top, bottom)
}

fn load_wallpaper(path: &Path) -> Option<RgbaFrame> {
    let image = image::open(path).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    RgbaFrame::from_pixels(width, height, image.into_raw()).ok()
}
