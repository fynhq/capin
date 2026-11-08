//! System-native surfaces for the capture overlay and annotation chrome.
//!
//! The screenshot page floats over arbitrary content, so it uses an opaque-
//! enough neutral material with a hairline border and a soft shadow. Colors
//! follow the platform's light/dark appearance and use native flyout values on
//! Windows and popover values on macOS.

#[cfg(not(target_os = "macos"))]
use core_types::prefers_dark_foreground;
use gpui_kit::*;

#[cfg(target_os = "windows")]
const FALLBACK_ACCENT_LIGHT: u32 = 0x0067C0;
#[cfg(target_os = "windows")]
const FALLBACK_ACCENT_DARK: u32 = 0x4CC2FF;
#[cfg(target_os = "macos")]
const FALLBACK_ACCENT_LIGHT: u32 = 0x007AFF;
#[cfg(target_os = "macos")]
const FALLBACK_ACCENT_DARK: u32 = 0x0A84FF;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FALLBACK_ACCENT_LIGHT: u32 = 0x0067C0;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FALLBACK_ACCENT_DARK: u32 = 0x4CC2FF;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChromeTheme {
    pub surface: Rgba,
    pub surface_strong: Rgba,
    pub border: Rgba,
    pub separator: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub icon: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub accent: Rgba,
    pub accent_tint: Rgba,
    pub accent_text: Rgba,
    pub grid: Rgba,
    pub grid_strong: Rgba,
    pub shadow_alpha: f32,
    pub shadow_blur: f32,
    pub shadow_offset: f32,
    pub radius: f32,
    pub handle_fill: Rgba,
    pub handle_border: Rgba,
    pub canvas: Rgba,
}

impl ChromeTheme {
    pub fn for_window(window: &Window, accent_hex: Option<u32>) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        Self::for_appearance(dark, accent_hex)
    }

    pub fn for_appearance(dark: bool, accent_hex: Option<u32>) -> Self {
        let mut theme = Self::palette(dark);
        let accent_hex = accent_hex.unwrap_or(if dark {
            FALLBACK_ACCENT_DARK
        } else {
            FALLBACK_ACCENT_LIGHT
        });
        theme.accent = rgb(accent_hex);
        theme.accent_tint = rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x33);
        theme.accent_text = accent_text_color(accent_hex);
        theme.handle_border = theme.accent;
        theme
    }

    pub fn shadow(&self) -> Vec<BoxShadow> {
        vec![
            BoxShadow::new(
                px(0.0),
                px(self.shadow_offset),
                hsla(0.0, 0.0, 0.0, self.shadow_alpha),
            )
            .blur_radius(px(self.shadow_blur)),
        ]
    }

    #[cfg(target_os = "windows")]
    fn palette(dark: bool) -> Self {
        if dark {
            Self {
                surface: rgba(0x2B2B2BF2),
                surface_strong: rgba(0x2B2B2BF7),
                border: rgba(0xFFFFFF1A),
                separator: rgba(0xFFFFFF14),
                text: rgb(0xF2F2F2),
                muted: rgb(0xA8A8A8),
                icon: rgba(0xFFFFFFD9),
                hover: rgba(0xFFFFFF10),
                selected: rgba(0xFFFFFF1F),
                accent: rgb(FALLBACK_ACCENT_DARK),
                accent_tint: rgba(0x4CC2FF33),
                accent_text: rgb(0xFFFFFF),
                grid: rgba(0xFFFFFF33),
                grid_strong: rgba(0xFFFFFFB3),
                shadow_alpha: 0.35,
                shadow_blur: 20.0,
                shadow_offset: 8.0,
                radius: 8.0,
                handle_fill: rgb(0xFFFFFF),
                handle_border: rgb(FALLBACK_ACCENT_DARK),
                canvas: rgb(0x141414),
            }
        } else {
            Self {
                surface: rgba(0xF9F9F9F2),
                surface_strong: rgba(0xF9F9F9F7),
                border: rgba(0x00000014),
                separator: rgba(0x00000010),
                text: rgb(0x1B1B1B),
                muted: rgb(0x5D5D5D),
                icon: rgba(0x000000D9),
                hover: rgba(0x00000008),
                selected: rgba(0x0000000A),
                accent: rgb(FALLBACK_ACCENT_LIGHT),
                accent_tint: rgba(0x0067C033),
                accent_text: rgb(0xFFFFFF),
                grid: rgba(0x00000033),
                grid_strong: rgba(0x000000B3),
                shadow_alpha: 0.18,
                shadow_blur: 18.0,
                shadow_offset: 6.0,
                radius: 8.0,
                handle_fill: rgb(0xFFFFFF),
                handle_border: rgb(FALLBACK_ACCENT_LIGHT),
                canvas: rgb(0xF3F3F3),
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn palette(dark: bool) -> Self {
        if dark {
            Self {
                surface: rgba(0x2C2C2EE6),
                surface_strong: rgba(0x2C2C2EF2),
                border: rgba(0xFFFFFF1F),
                separator: rgba(0xFFFFFF14),
                text: rgb(0xF5F5F7),
                muted: rgb(0x98989D),
                icon: rgba(0xFFFFFFE6),
                hover: rgba(0xFFFFFF14),
                selected: rgba(0xFFFFFF1F),
                accent: rgb(FALLBACK_ACCENT_DARK),
                accent_tint: rgba(0x0A84FF33),
                accent_text: rgb(0xFFFFFF),
                grid: rgba(0xFFFFFF33),
                grid_strong: rgba(0xFFFFFFB3),
                shadow_alpha: 0.4,
                shadow_blur: 22.0,
                shadow_offset: 8.0,
                radius: 10.0,
                handle_fill: rgb(0xFFFFFF),
                handle_border: rgb(FALLBACK_ACCENT_DARK),
                canvas: rgb(0x1E1E1E),
            }
        } else {
            Self {
                surface: rgba(0xFFFFFFE6),
                surface_strong: rgba(0xFFFFFFF2),
                border: rgba(0x00000014),
                separator: rgba(0x0000000F),
                text: rgb(0x1D1D1F),
                muted: rgb(0x6E6E73),
                icon: rgba(0x000000D9),
                hover: rgba(0x0000000A),
                selected: rgba(0x0000000F),
                accent: rgb(FALLBACK_ACCENT_LIGHT),
                accent_tint: rgba(0x007AFF33),
                accent_text: rgb(0xFFFFFF),
                grid: rgba(0x00000033),
                grid_strong: rgba(0x000000B3),
                shadow_alpha: 0.2,
                shadow_blur: 22.0,
                shadow_offset: 8.0,
                radius: 10.0,
                handle_fill: rgb(0xFFFFFF),
                handle_border: rgb(FALLBACK_ACCENT_LIGHT),
                canvas: rgb(0xF5F5F7),
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn palette(dark: bool) -> Self {
        if dark {
            Self {
                surface: rgba(0x2C2C2EE6),
                surface_strong: rgba(0x2C2C2EF2),
                border: rgba(0xFFFFFF1F),
                separator: rgba(0xFFFFFF14),
                text: rgb(0xF5F5F7),
                muted: rgb(0x98989D),
                icon: rgba(0xFFFFFFE6),
                hover: rgba(0xFFFFFF14),
                selected: rgba(0xFFFFFF1F),
                accent: rgb(FALLBACK_ACCENT_DARK),
                accent_tint: rgba(0x4CC2FF33),
                accent_text: rgb(0xFFFFFF),
                grid: rgba(0xFFFFFF33),
                grid_strong: rgba(0xFFFFFFB3),
                shadow_alpha: 0.4,
                shadow_blur: 22.0,
                shadow_offset: 8.0,
                radius: 10.0,
                handle_fill: rgb(0xFFFFFF),
                handle_border: rgb(FALLBACK_ACCENT_DARK),
                canvas: rgb(0x1E1E1E),
            }
        } else {
            Self {
                surface: rgba(0xFFFFFFE6),
                surface_strong: rgba(0xFFFFFFF2),
                border: rgba(0x00000014),
                separator: rgba(0x0000000F),
                text: rgb(0x1D1D1F),
                muted: rgb(0x6E6E73),
                icon: rgba(0x000000D9),
                hover: rgba(0x0000000A),
                selected: rgba(0x0000000F),
                accent: rgb(FALLBACK_ACCENT_LIGHT),
                accent_tint: rgba(0x0067C033),
                accent_text: rgb(0xFFFFFF),
                grid: rgba(0x00000033),
                grid_strong: rgba(0x000000B3),
                shadow_alpha: 0.2,
                shadow_blur: 22.0,
                shadow_offset: 8.0,
                radius: 10.0,
                handle_fill: rgb(0xFFFFFF),
                handle_border: rgb(FALLBACK_ACCENT_LIGHT),
                canvas: rgb(0xF5F5F7),
            }
        }
    }
}

fn accent_text_color(hex: u32) -> Rgba {
    #[cfg(target_os = "macos")]
    {
        let _ = hex;
        rgb(0xFFFFFF)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if prefers_dark_foreground(hex) {
            rgb(0x1A1A1A)
        } else {
            rgb(0xFFFFFF)
        }
    }
}
