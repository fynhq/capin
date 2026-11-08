//! Small, testable color-management primitives shared by capture and export.
//!
//! Platform capture code is responsible for reporting the source color space.
//! This module deliberately stays in encoded/linear RGB and does not depend on
//! an ICC profile parser.

use crate::RgbaFrame;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSpace {
    #[default]
    Srgb,
    DisplayP3,
    LinearSrgb,
}

/// Decodes an sRGB-encoded channel to linear light.
pub fn srgb_to_linear(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Encodes a linear-light channel with the sRGB transfer function.
pub fn linear_to_srgb(value: f32) -> f32 {
    let value = value.max(0.0);
    let encoded = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    encoded.clamp(0.0, 1.0)
}

/// Converts linear Display P3 RGB to linear sRGB RGB.
///
/// Values can fall outside `0..=1`; callers clamp after the conversion.
pub fn linear_p3_to_srgb(rgb: [f32; 3]) -> [f32; 3] {
    [
        1.224_940_1 * rgb[0] - 0.224_940_4 * rgb[1],
        -0.042_056_9 * rgb[0] + 1.042_057_1 * rgb[1],
        -0.019_637_6 * rgb[0] - 0.078_636_1 * rgb[1] + 1.098_273_5 * rgb[2],
    ]
}

/// Extended Reinhard tone mapping for linear-light values above SDR white.
pub fn tone_map_reinhard(rgb: [f32; 3], exposure: f32) -> [f32; 3] {
    let exposure = exposure.max(0.0);
    rgb.map(|channel| {
        let value = (channel * exposure).max(0.0);
        value / (1.0 + value)
    })
}

/// Converts one encoded/linear RGB triple to encoded sRGB.
pub fn convert_rgb_to_srgb(rgb: [f32; 3], source: ColorSpace, tone_map: bool) -> [f32; 3] {
    let linear = match source {
        ColorSpace::Srgb => rgb.map(srgb_to_linear),
        ColorSpace::DisplayP3 => linear_p3_to_srgb(rgb.map(srgb_to_linear)),
        ColorSpace::LinearSrgb => rgb,
    };
    let linear = if tone_map {
        tone_map_reinhard(linear, 1.0)
    } else {
        linear
    };
    linear.map(linear_to_srgb)
}

/// Converts an RGBA8 frame to encoded sRGB while preserving alpha.
///
/// This is intentionally lossy for HDR input: the output is SDR sRGB. It is
/// the common denominator for clipboard, PNG export and pin preview.
pub fn convert_frame_to_srgb(
    frame: &RgbaFrame,
    source: ColorSpace,
    tone_map: bool,
) -> Result<RgbaFrame, crate::FrameError> {
    if source == ColorSpace::Srgb && !tone_map {
        return Ok(frame.clone());
    }
    let mut pixels = frame.pixels().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let converted = convert_rgb_to_srgb(
            [
                f32::from(pixel[0]) / 255.0,
                f32::from(pixel[1]) / 255.0,
                f32::from(pixel[2]) / 255.0,
            ],
            source,
            tone_map,
        );
        pixel[0] = (converted[0] * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (converted[1] * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (converted[2] * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    RgbaFrame::from_pixels(frame.width(), frame.height(), pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pixel: [u8; 4]) -> RgbaFrame {
        RgbaFrame::from_pixels(1, 1, pixel.to_vec()).unwrap()
    }

    #[test]
    fn srgb_transfer_round_trips_midtones() {
        for value in [0.0, 0.05, 0.18, 0.5, 0.75, 1.0] {
            let encoded = linear_to_srgb(srgb_to_linear(value));
            assert!(
                (encoded - value).abs() < 0.001,
                "round trip drifted for {value}: {encoded}"
            );
        }
    }

    #[test]
    fn neutrals_survive_p3_conversion() {
        let converted = convert_rgb_to_srgb([0.5, 0.5, 0.5], ColorSpace::DisplayP3, false);
        for channel in converted {
            assert!(
                (channel - 0.5).abs() < 0.002,
                "neutral drifted: {converted:?}"
            );
        }
    }

    #[test]
    fn p3_red_is_mapped_toward_srgb_red() {
        let converted = convert_rgb_to_srgb([1.0, 0.0, 0.0], ColorSpace::DisplayP3, false);
        assert!(converted[0] > 0.99);
        assert!(converted[1] <= 0.001);
        assert!(converted[2] <= 0.001);
    }

    #[test]
    fn tone_mapping_compresses_highlights() {
        let converted = tone_map_reinhard([4.0, 2.0, 0.5], 1.0);
        assert!(converted[0] < 4.0 && converted[0] > 0.5);
        assert!(converted[1] < 2.0);
        assert!(converted[2] < 0.5);
    }

    #[test]
    fn frame_conversion_preserves_alpha() {
        let source = frame([255, 0, 255, 77]);
        let converted = convert_frame_to_srgb(&source, ColorSpace::DisplayP3, false).unwrap();
        assert_eq!(converted.width(), 1);
        assert_eq!(converted.height(), 1);
        assert_eq!(converted.pixel(0, 0).unwrap()[3], 77);
    }

    #[test]
    fn srgb_frame_without_tone_mapping_is_a_noop() {
        let source = frame([12, 34, 56, 78]);
        let converted = convert_frame_to_srgb(&source, ColorSpace::Srgb, false).unwrap();
        assert_eq!(converted.pixel(0, 0), Some([12, 34, 56, 78]));
    }
}
