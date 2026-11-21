#[cfg(not(target_os = "macos"))]
use core_types::prefers_dark_foreground;
use gpui_kit::{Rgba, rgb, rgba};

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

pub(crate) fn fallback_accent(dark: bool) -> u32 {
    if dark {
        FALLBACK_ACCENT_DARK
    } else {
        FALLBACK_ACCENT_LIGHT
    }
}

pub(crate) fn accent_text_color(hex: u32) -> Rgba {
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

pub(crate) struct ChromePalette {
    pub surface: Rgba,
    pub surface_strong: Rgba,
    pub border: Rgba,
    pub separator: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub icon: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub grid: Rgba,
    pub grid_strong: Rgba,
    pub shadow_alpha: f32,
    pub shadow_blur: f32,
    pub shadow_offset: f32,
    pub radius: f32,
    pub handle_fill: Rgba,
    pub canvas: Rgba,
}

pub(crate) struct SettingsPalette {
    pub text: Rgba,
    pub muted: Rgba,
    pub border: Rgba,
    pub control: Rgba,
    pub control_hover: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub toggle_off: Rgba,
    pub popover: Rgba,
    pub page: Option<Rgba>,
    pub sidebar: Rgba,
    pub card: Rgba,
    pub card_border: Rgba,
}

pub(crate) struct PinPalette {
    pub card: Rgba,
    pub text: Rgba,
    pub border: Rgba,
    pub menu: Rgba,
    pub menu_hover: Rgba,
    pub menu_border: Rgba,
}

#[cfg(target_os = "windows")]
pub(crate) fn chrome_palette(dark: bool) -> ChromePalette {
    if dark {
        ChromePalette {
            surface: rgba(0x2B2B2BF2),
            surface_strong: rgba(0x2B2B2BF7),
            border: rgba(0xFFFFFF1A),
            separator: rgba(0xFFFFFF14),
            text: rgb(0xF2F2F2),
            muted: rgb(0xA8A8A8),
            icon: rgba(0xFFFFFFD9),
            hover: rgba(0xFFFFFF10),
            selected: rgba(0xFFFFFF1F),
            grid: rgba(0xFFFFFF33),
            grid_strong: rgba(0xFFFFFFB3),
            shadow_alpha: 0.35,
            shadow_blur: 20.0,
            shadow_offset: 8.0,
            radius: 8.0,
            handle_fill: rgb(0xFFFFFF),
            canvas: rgb(0x141414),
        }
    } else {
        ChromePalette {
            surface: rgba(0xF9F9F9F2),
            surface_strong: rgba(0xF9F9F9F7),
            border: rgba(0x00000014),
            separator: rgba(0x00000010),
            text: rgb(0x1B1B1B),
            muted: rgb(0x5D5D5D),
            icon: rgba(0x000000D9),
            hover: rgba(0x00000008),
            selected: rgba(0x0000000A),
            grid: rgba(0x00000033),
            grid_strong: rgba(0x000000B3),
            shadow_alpha: 0.18,
            shadow_blur: 18.0,
            shadow_offset: 6.0,
            radius: 8.0,
            handle_fill: rgb(0xFFFFFF),
            canvas: rgb(0xF3F3F3),
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn chrome_palette(dark: bool) -> ChromePalette {
    if dark {
        ChromePalette {
            surface: rgba(0x2C2C2EE6),
            surface_strong: rgba(0x2C2C2EF2),
            border: rgba(0xFFFFFF1F),
            separator: rgba(0xFFFFFF14),
            text: rgb(0xF5F5F7),
            muted: rgb(0x98989D),
            icon: rgba(0xFFFFFFE6),
            hover: rgba(0xFFFFFF14),
            selected: rgba(0xFFFFFF1F),
            grid: rgba(0xFFFFFF33),
            grid_strong: rgba(0xFFFFFFB3),
            shadow_alpha: 0.4,
            shadow_blur: 22.0,
            shadow_offset: 8.0,
            radius: 10.0,
            handle_fill: rgb(0xFFFFFF),
            canvas: rgb(0x1E1E1E),
        }
    } else {
        ChromePalette {
            surface: rgba(0xFFFFFFE6),
            surface_strong: rgba(0xFFFFFFF2),
            border: rgba(0x00000014),
            separator: rgba(0x0000000F),
            text: rgb(0x1D1D1F),
            muted: rgb(0x6E6E73),
            icon: rgba(0x000000D9),
            hover: rgba(0x0000000A),
            selected: rgba(0x0000000F),
            grid: rgba(0x00000033),
            grid_strong: rgba(0x000000B3),
            shadow_alpha: 0.2,
            shadow_blur: 22.0,
            shadow_offset: 8.0,
            radius: 10.0,
            handle_fill: rgb(0xFFFFFF),
            canvas: rgb(0xF5F5F7),
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) fn chrome_palette(dark: bool) -> ChromePalette {
    let mut palette = chrome_palette_macos(dark);
    palette.canvas = if dark { rgb(0x1E1E1E) } else { rgb(0xF5F5F7) };
    palette
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn chrome_palette_macos(dark: bool) -> ChromePalette {
    if dark {
        ChromePalette {
            surface: rgba(0x2C2C2EE6),
            surface_strong: rgba(0x2C2C2EF2),
            border: rgba(0xFFFFFF1F),
            separator: rgba(0xFFFFFF14),
            text: rgb(0xF5F5F7),
            muted: rgb(0x98989D),
            icon: rgba(0xFFFFFFE6),
            hover: rgba(0xFFFFFF14),
            selected: rgba(0xFFFFFF1F),
            grid: rgba(0xFFFFFF33),
            grid_strong: rgba(0xFFFFFFB3),
            shadow_alpha: 0.4,
            shadow_blur: 22.0,
            shadow_offset: 8.0,
            radius: 10.0,
            handle_fill: rgb(0xFFFFFF),
            canvas: rgb(0x1E1E1E),
        }
    } else {
        ChromePalette {
            surface: rgba(0xFFFFFFE6),
            surface_strong: rgba(0xFFFFFFF2),
            border: rgba(0x00000014),
            separator: rgba(0x0000000F),
            text: rgb(0x1D1D1F),
            muted: rgb(0x6E6E73),
            icon: rgba(0x000000D9),
            hover: rgba(0x0000000A),
            selected: rgba(0x0000000F),
            grid: rgba(0x00000033),
            grid_strong: rgba(0x000000B3),
            shadow_alpha: 0.2,
            shadow_blur: 22.0,
            shadow_offset: 8.0,
            radius: 10.0,
            handle_fill: rgb(0xFFFFFF),
            canvas: rgb(0xF5F5F7),
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn settings_palette(dark: bool) -> SettingsPalette {
    if dark {
        SettingsPalette {
            text: rgb(0xF2F2F2),
            muted: rgb(0xA8A8A8),
            border: rgba(0xFFFFFF18),
            control: rgb(0x383838),
            control_hover: rgb(0x424242),
            hover: rgba(0xFFFFFF10),
            selected: rgba(0xFFFFFF14),
            toggle_off: rgba(0xFFFFFF1F),
            popover: rgb(0x2B2B2B),
            page: None,
            sidebar: rgba(0x00000000),
            card: rgb(0x2B2B2B),
            card_border: rgba(0x0000001A),
        }
    } else {
        SettingsPalette {
            text: rgb(0x1B1B1B),
            muted: rgb(0x5D5D5D),
            border: rgba(0x0000000F),
            control: rgb(0xFCFCFC),
            control_hover: rgb(0xF2F2F2),
            hover: rgba(0x00000008),
            selected: rgba(0x0000000A),
            toggle_off: rgba(0x0000001A),
            popover: rgb(0xF9F9F9),
            page: None,
            sidebar: rgba(0x00000000),
            card: rgb(0xFAFAFA),
            card_border: rgba(0x0000000F),
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn settings_palette(dark: bool) -> SettingsPalette {
    if dark {
        SettingsPalette {
            text: rgb(0xF5F5F7),
            muted: rgb(0x98989D),
            border: rgba(0xFFFFFF1F),
            control: rgb(0x3A3A3C),
            control_hover: rgb(0x464648),
            hover: rgba(0xFFFFFF14),
            selected: rgba(0xFFFFFF1F),
            toggle_off: rgba(0xFFFFFF1F),
            popover: rgb(0x2C2C2E),
            page: Some(rgb(0x1C1C1E)),
            sidebar: rgba(0x202022E6),
            card: rgb(0x2C2C2E),
            card_border: rgba(0x00000000),
        }
    } else {
        SettingsPalette {
            text: rgb(0x1D1D1F),
            muted: rgb(0x6E6E73),
            border: rgba(0x00000014),
            control: rgb(0xFFFFFF),
            control_hover: rgb(0xF1F1F1),
            hover: rgba(0x0000000A),
            selected: rgba(0x0000000F),
            toggle_off: rgba(0x0000001A),
            popover: rgb(0xFFFFFF),
            page: Some(rgb(0xFFFFFF)),
            sidebar: rgba(0xECECECE6),
            card: rgb(0xF7F7F7),
            card_border: rgba(0x00000000),
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) fn settings_palette(dark: bool) -> SettingsPalette {
    if dark {
        SettingsPalette {
            text: rgb(0xF5F5F7),
            muted: rgb(0x98989D),
            border: rgba(0xFFFFFF1F),
            control: rgb(0x3A3A3C),
            control_hover: rgb(0x464648),
            hover: rgba(0xFFFFFF14),
            selected: rgba(0xFFFFFF1F),
            toggle_off: rgba(0xFFFFFF4D),
            popover: rgb(0x2C2C2E),
            page: Some(rgb(0x1E1E1E)),
            sidebar: rgba(0x00000000),
            card: rgb(0x2C2C2E),
            card_border: rgba(0x00000000),
        }
    } else {
        SettingsPalette {
            text: rgb(0x1D1D1F),
            muted: rgb(0x6E6E73),
            border: rgba(0x00000014),
            control: rgb(0xFFFFFF),
            control_hover: rgb(0xF1F1F1),
            hover: rgba(0x0000000A),
            selected: rgba(0x0000000F),
            toggle_off: rgba(0x00000026),
            popover: rgb(0xFFFFFF),
            page: Some(rgb(0xF2F2F2)),
            sidebar: rgba(0x00000000),
            card: rgb(0xFFFFFF),
            card_border: rgba(0x00000000),
        }
    }
}

pub(crate) fn pin_palette(dark: bool) -> PinPalette {
    if dark {
        PinPalette {
            card: rgba(0x1E1E1EF2),
            text: rgb(0xF2F2F2),
            border: rgba(0xFFFFFF24),
            menu: rgba(0x2B2B2BFA),
            menu_hover: rgba(0xFFFFFF16),
            menu_border: rgba(0xFFFFFF22),
        }
    } else {
        PinPalette {
            card: rgba(0xFFFFFFF2),
            text: rgb(0x1A1A1A),
            border: rgba(0x0000001A),
            menu: rgba(0xF9F9F9FA),
            menu_hover: rgba(0x00000010),
            menu_border: rgba(0x0000001A),
        }
    }
}
