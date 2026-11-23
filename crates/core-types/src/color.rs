pub const fn abgr_to_rgb(abgr: u32) -> u32 {
    ((abgr & 0xFF) << 16) | (abgr & 0xFF00) | ((abgr >> 16) & 0xFF)
}

pub fn luma8(red: u8, green: u8, blue: u8) -> u8 {
    ((u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114) / 1000) as u8
}

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
    fn luma8_matches_the_perceptual_weights() {
        assert_eq!(luma8(255, 255, 255), 255);
        assert_eq!(luma8(0, 0, 0), 0);
        assert_eq!(luma8(255, 0, 0), 76);
        assert_eq!(luma8(0, 255, 0), 149);
        assert_eq!(luma8(0, 0, 255), 29);
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
