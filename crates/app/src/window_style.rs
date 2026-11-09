use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use core_compose::{Background, Shadow, WindowCanvas, compose_window};
use core_types::{RgbaFrame, WindowId};

use crate::logging::{log_error, log_warn};
use crate::settings_state::{self, AppSettings, GradientPreset, WindowBackground};

const LIGHT_CANVAS: [u8; 4] = [0xF5, 0xF5, 0xF7, 0xFF];
const DARK_CANVAS: [u8; 4] = [0x1C, 0x1C, 0x1E, 0xFF];
const FALLBACK_ACCENT: u32 = 0x4C9AFF;
const SHADOW_OFFSET: f32 = 12.0;
const SHADOW_BLUR: f32 = 32.0;
const SHADOW_OPACITY: f32 = 0.35;

#[derive(Clone)]
struct WallpaperCacheEntry {
    path: PathBuf,
    modified: Option<SystemTime>,
    length: u64,
    frame: Arc<RgbaFrame>,
}

thread_local! {
    /// Decoding a 4K wallpaper for every window capture is pure CPU. Reuse
    /// the decoded frame until the file's size or mtime changes.
    static WALLPAPER_CACHE: RefCell<Option<WallpaperCacheEntry>> = const { RefCell::new(None) };
}

pub fn compose_window_capture(id: WindowId, frame: RgbaFrame, scale: f32) -> RgbaFrame {
    let settings = settings_state::get();
    if settings.window_background != WindowBackground::Wallpaper || !settings.window_composition {
        WALLPAPER_CACHE.with(|cache| *cache.borrow_mut() = None);
    }
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
        WindowBackground::Gradient => gradient(settings),
        WindowBackground::Wallpaper => {
            wallpaper(settings)
                .map(Background::Image)
                .unwrap_or_else(|| {
                    log_warn!(
                        "window",
                        "window wallpaper is unavailable; using a transparent canvas"
                    );
                    Background::Transparent
                })
        }
    }
}

/// A custom image wins; otherwise the current system wallpaper is used, so
/// choosing "Wallpaper" works without asking the user to pick a file first.
fn wallpaper(settings: &AppSettings) -> Option<Arc<RgbaFrame>> {
    if let Some(path) = settings.window_wallpaper.as_deref()
        && let Some(frame) = load_wallpaper(path)
    {
        return Some(frame);
    }
    platform_window::desktop_wallpaper_path()
        .as_deref()
        .and_then(load_wallpaper)
        .or_else(|| platform_window::desktop_wallpaper_frame().map(Arc::new))
}

fn gradient(settings: &AppSettings) -> Background {
    let (top, bottom) = gradient_colors(settings);
    Background::VerticalGradient(rgb_pair(top), rgb_pair(bottom))
}

/// Resolves the two gradient stops for the current preset. The settings page
/// reuses this so the hex fields always show the colors that will be painted.
pub fn gradient_colors(settings: &AppSettings) -> (u32, u32) {
    match settings.window_gradient {
        GradientPreset::Accent => accent_gradient(),
        GradientPreset::Custom => (
            settings.window_gradient_top & 0x00FF_FFFF,
            settings.window_gradient_bottom & 0x00FF_FFFF,
        ),
        preset => preset.colors().unwrap_or_else(accent_gradient),
    }
}

fn accent_gradient() -> (u32, u32) {
    let accent = platform_window::system_accent_hex().unwrap_or(FALLBACK_ACCENT);
    let dimmed = ((accent >> 16) & 0xFF) * 35 / 100;
    let dimmed_green = ((accent >> 8) & 0xFF) * 35 / 100;
    let dimmed_blue = (accent & 0xFF) * 35 / 100;
    (accent, (dimmed << 16) | (dimmed_green << 8) | dimmed_blue)
}

fn rgb_pair(color: u32) -> [u8; 4] {
    [
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
        0xFF,
    ]
}

fn load_wallpaper(path: &Path) -> Option<Arc<RgbaFrame>> {
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok());
    let length = metadata.as_ref().map_or(0, std::fs::Metadata::len);

    if let Some(cached) = WALLPAPER_CACHE.with(|cache| cache.borrow().clone())
        && cached.path == path
        && cached.modified == modified
        && cached.length == length
    {
        return Some(cached.frame);
    }

    let image = image::open(path).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    let frame = Arc::new(RgbaFrame::from_pixels(width, height, image.into_raw()).ok()?);
    WALLPAPER_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(WallpaperCacheEntry {
            path: path.to_path_buf(),
            modified,
            length,
            frame: frame.clone(),
        });
    });
    Some(frame)
}
