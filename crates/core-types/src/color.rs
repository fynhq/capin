/// Converts a Windows Explorer accent value stored as ABGR to packed RGB.
pub const fn abgr_to_rgb(abgr: u32) -> u32 {
    ((abgr & 0xFF) << 16) | (abgr & 0xFF00) | ((abgr >> 16) & 0xFF)
}

/// WCAG relative luminance of a packed `0xRRGGBB` color.
pub fn relative_luminance(hex: u32) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.040_45 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    let red = linearize(((hex >> 16) & 0xFF) as u8);
    let green = linearize(((hex >> 8) & 0xFF) as u8);
    let blue = linearize((hex & 0xFF) as u8);
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/// Whether a dark label reads better than a light one on `hex`.
///
/// The dark label must beat the light one by 15% rather than merely tie, so
/// mid-tone system accents keep the familiar white-on-blue look while bright
/// accents such as gold switch to dark text.
pub fn prefers_dark_foreground(hex: u32) -> bool {
    let luminance = relative_luminance(hex);
    let contrast_black = (luminance + 0.05) / 0.05;
    let contrast_white = 1.05 / (luminance + 0.05);
    contrast_black > contrast_white * 1.15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_menu_abgr_is_swizzled_to_rgb() {
        assert_eq!(abgr_to_rgb(0xFF15_7286), 0x86_7215);
        assert_eq!(abgr_to_rgb(0xFF00_0000), 0x00_0000);
    }

    #[test]
    fn bright_gold_prefers_dark_text() {
        assert!(prefers_dark_foreground(0xFFB900));
    }

    #[test]
    fn windows_blue_prefers_light_text() {
        assert!(!prefers_dark_foreground(0x0067C0));
    }

    #[test]
    fn extremes_follow_contrast() {
        assert!(prefers_dark_foreground(0xFFFFFF));
        assert!(!prefers_dark_foreground(0x000000));
    }
}
