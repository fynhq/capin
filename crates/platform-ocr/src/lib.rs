//! Platform text recognition and QR decoding for captured frames.

use core_types::{PhysicalRect, RgbaFrame};
use thiserror::Error;

/// One recognized character with its approximate image-space box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrChar {
    pub ch: char,
    pub rect: PhysicalRect,
}

/// One recognized text line and its character boxes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrLine {
    pub text: String,
    pub rect: PhysicalRect,
    pub chars: Vec<OcrChar>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OcrOutput {
    pub text: String,
    pub lines: Vec<OcrLine>,
    pub qr_codes: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcrOptions {
    pub text: bool,
    pub qr: bool,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            text: true,
            qr: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("text recognition is unavailable on this platform")]
    Unsupported,
    #[error("{0}")]
    Platform(String),
}

#[cfg(target_os = "macos")]
mod macos_impl;
#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod unsupported {
    use core_types::RgbaFrame;

    use crate::OcrError;

    pub(super) fn recognize_text(_frame: &RgbaFrame) -> Result<Vec<crate::OcrLine>, OcrError> {
        Err(OcrError::Unsupported)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use unsupported as platform;

#[cfg(target_os = "macos")]
use macos_impl as platform;
#[cfg(target_os = "windows")]
use windows_impl as platform;

pub fn recognize(frame: RgbaFrame) -> OcrOutput {
    recognize_with(frame, OcrOptions::default())
}

pub fn recognize_with(frame: RgbaFrame, options: OcrOptions) -> OcrOutput {
    let qr_codes = if options.qr {
        decode_qr(&frame)
    } else {
        Vec::new()
    };
    if !options.text {
        return OcrOutput {
            qr_codes,
            ..Default::default()
        };
    }
    match platform::recognize_text(&frame) {
        Ok(lines) => {
            let text = lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join(
                    "
",
                );
            OcrOutput {
                text,
                lines,
                qr_codes,
                error: None,
            }
        }
        Err(error) => OcrOutput {
            text: String::new(),
            lines: Vec::new(),
            qr_codes,
            error: Some(error.to_string()),
        },
    }
}

#[allow(dead_code)] // platform backends only
pub(crate) fn build_line(text: String, rect: PhysicalRect) -> OcrLine {
    let text = text.trim_end().to_owned();
    let chars = layout_chars(&text, rect);
    OcrLine { text, rect, chars }
}

#[allow(dead_code)]
fn layout_chars(text: &str, rect: PhysicalRect) -> Vec<OcrChar> {
    if text.is_empty() || rect.size.width == 0 {
        return Vec::new();
    }
    let weights: Vec<f32> = text.chars().map(char_weight).collect();
    let total: f32 = weights.iter().sum::<f32>().max(0.1);
    let mut x = rect.origin.x as f32;
    let mut chars = Vec::with_capacity(weights.len());
    for (index, (ch, weight)) in text.chars().zip(weights.iter()).enumerate() {
        let last = index + 1 == weights.len();
        let width = if last {
            (rect.origin.x as f32 + rect.size.width as f32 - x).max(1.0)
        } else {
            (rect.size.width as f32 * weight / total).max(1.0)
        };
        chars.push(OcrChar {
            ch,
            rect: PhysicalRect::new(
                x.round() as i32,
                rect.origin.y,
                width.round().max(1.0) as u32,
                rect.size.height,
            ),
        });
        x += width;
    }
    chars
}

#[allow(dead_code)]
fn char_weight(ch: char) -> f32 {
    if ch.is_whitespace() {
        0.5
    } else if ch.is_ascii_punctuation() {
        0.55
    } else if ch.is_ascii() {
        0.8
    } else {
        1.2
    }
}

fn decode_qr(frame: &RgbaFrame) -> Vec<String> {
    let mut gray = image::GrayImage::new(frame.width(), frame.height());
    for (target, source) in gray.pixels_mut().zip(frame.pixels().as_chunks::<4>().0) {
        let luminance =
            (u32::from(source[0]) * 299 + u32::from(source[1]) * 587 + u32::from(source[2]) * 114)
                / 1000;
        *target = image::Luma([luminance as u8]);
    }

    let mut prepared = rqrr::PreparedImage::prepare(gray);
    let mut codes = Vec::new();
    for grid in prepared.detect_grids() {
        if let Ok((_, content)) = grid.decode()
            && !content.is_empty()
            && !codes.contains(&content)
        {
            codes.push(content);
        }
    }
    codes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_frame() -> RgbaFrame {
        RgbaFrame::from_pixels(32, 32, vec![255; 32 * 32 * 4]).unwrap()
    }

    #[test]
    fn character_layout_preserves_text_order() {
        let line = build_line("AΩ b".to_owned(), PhysicalRect::new(10, 20, 120, 24));
        assert_eq!(line.text, "AΩ b");
        assert_eq!(line.chars.len(), 4);
        assert_eq!(line.chars[1].ch, 'Ω');
        assert_eq!(line.chars[2].ch, ' ');
        assert!(line.chars[0].rect.origin.x >= 10);
        let last = line.chars.last().unwrap().rect;
        assert!(last.right() <= 130);
        assert!(line.chars.iter().all(|ch| ch.rect.size.height == 24));
    }

    #[test]
    fn blank_frame_has_no_qr_codes() {
        assert!(decode_qr(&blank_frame()).is_empty());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn unsupported_platform_reports_an_error() {
        let output = recognize(blank_frame());
        assert!(output.text.is_empty());
        assert!(output.error.is_some());
    }
}
