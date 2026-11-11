//! Platform text recognition and QR decoding for captured frames.

use std::sync::Arc;

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
    use std::sync::Arc;

    use core_types::RgbaFrame;

    use crate::OcrError;

    pub(super) fn recognize_text(_frame: Arc<RgbaFrame>) -> Result<Vec<crate::OcrLine>, OcrError> {
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
    recognize_shared(Arc::new(frame), options)
}

/// Recognizes text and/or QR codes from a shared frame.
///
/// Callers that already hold a pinned image can pass its `Arc` so queued OCR
/// work does not duplicate the full RGBA buffer.
pub fn recognize_shared(frame: Arc<RgbaFrame>, options: OcrOptions) -> OcrOutput {
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
    match platform::recognize_text(frame) {
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

/// Builds a line from word-level OCR hits. Windows returns every CJK glyph as
/// its own "word", so joining word text with spaces inserted a gap between
/// each Chinese character both on screen and when copied. Character boxes now
/// follow the real word rectangles instead of being spread evenly across the
/// whole line, which also makes text selection land on the right glyph.
#[allow(dead_code)] // Windows backend only
pub(crate) fn build_line_from_words(words: &[(String, PhysicalRect)]) -> Option<OcrLine> {
    let words: Vec<(&str, PhysicalRect)> = words
        .iter()
        .filter_map(|(text, rect)| {
            let text = text.trim();
            (!text.is_empty() && rect.size.width > 0).then_some((text, *rect))
        })
        .collect();
    if words.is_empty() {
        return None;
    }

    let mut text = String::new();
    let mut chars: Vec<OcrChar> = Vec::new();
    let mut line: Option<PhysicalRect> = None;
    let mut previous: Option<PhysicalRect> = None;
    let mut previous_char: Option<char> = None;
    for (word, rect) in &words {
        if let (Some(prev_char), Some(prev_rect), Some(first)) =
            (previous_char, previous, word.chars().next())
            && needs_space(prev_char, first)
        {
            let left = prev_rect.origin.x + prev_rect.size.width as i32;
            let width = (rect.origin.x - left).max(1) as u32;
            chars.push(OcrChar {
                ch: ' ',
                rect: PhysicalRect::new(left, rect.origin.y, width, rect.size.height),
            });
            text.push(' ');
        }
        let mut word_chars = layout_chars(word, *rect);
        text.push_str(word);
        chars.append(&mut word_chars);
        line = Some(match line {
            Some(current) => union_rect(current, *rect),
            None => *rect,
        });
        previous = Some(*rect);
        previous_char = word.chars().last();
    }

    Some(OcrLine {
        text: text.trim_end().to_owned(),
        rect: line?,
        chars,
    })
}

#[allow(dead_code)] // Windows backend only
fn needs_space(previous: char, next: char) -> bool {
    // Windows OCR can split a long number into word boxes. Inserting a space
    // between two digit runs turns one value into two, so keep digits joined.
    if previous.is_ascii_digit() && next.is_ascii_digit() {
        return false;
    }
    let latin = |ch: char| ch.is_ascii_alphanumeric() || ch.is_ascii_punctuation();
    if !latin(previous) || !latin(next) {
        return false;
    }
    if matches!(previous, '(' | '[' | '{' | '"' | '\'') {
        return false;
    }
    !matches!(
        next,
        ')' | ']' | '}' | ',' | '.' | ';' | ':' | '?' | '!' | '%' | '"' | '\''
    )
}

#[allow(dead_code)] // Windows backend only
fn union_rect(a: PhysicalRect, b: PhysicalRect) -> PhysicalRect {
    let left = a.origin.x.min(b.origin.x);
    let top = a.origin.y.min(b.origin.y);
    let right = (a.origin.x + a.size.width as i32).max(b.origin.x + b.size.width as i32);
    let bottom = (a.origin.y + a.size.height as i32).max(b.origin.y + b.size.height as i32);
    PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32)
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

    // rqrr scans the whole bitmap. A 4K screenshot is mostly wasted work for
    // QR detection; cap the long edge while keeping typical codes well above
    // the minimum module size.
    const QR_MAX_EDGE: u32 = 2048;
    let long_edge = gray.width().max(gray.height());
    let gray = if long_edge > QR_MAX_EDGE {
        let scale = QR_MAX_EDGE as f32 / long_edge as f32;
        image::imageops::resize(
            &gray,
            ((gray.width() as f32 * scale).round() as u32).max(1),
            ((gray.height() as f32 * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        gray
    };

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
    fn word_layout_keeps_cjk_words_tight() {
        let line = build_line_from_words(&[
            ("\u{4f60}".to_owned(), PhysicalRect::new(10, 0, 20, 20)),
            ("\u{597d}".to_owned(), PhysicalRect::new(32, 0, 20, 20)),
            (
                "\u{4e16}\u{754c}".to_owned(),
                PhysicalRect::new(54, 0, 40, 20),
            ),
        ])
        .unwrap();
        assert_eq!(line.text, "\u{4f60}\u{597d}\u{4e16}\u{754c}");
        assert!(!line.chars.iter().any(|ch| ch.ch == ' '));
        assert_eq!(line.chars[1].rect.origin.x, 32);
    }

    #[test]
    fn word_layout_keeps_spaces_between_latin_words() {
        let line = build_line_from_words(&[
            ("Hello,".to_owned(), PhysicalRect::new(0, 0, 50, 20)),
            ("world".to_owned(), PhysicalRect::new(58, 0, 50, 20)),
        ])
        .unwrap();
        assert_eq!(line.text, "Hello, world");
        assert_eq!(line.chars.iter().filter(|ch| ch.ch == ' ').count(), 1);
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
