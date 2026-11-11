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
        Ok(mut lines) => {
            for line in &mut lines {
                normalize_latin_punctuation(line);
            }
            let text = join_ocr_lines(&lines);
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

/// Windows OCR often returns full-width punctuation for Latin text (a
/// U+FF0C comma instead of `,`, for example) because the active recognizer is
/// the CJK model. Lines without CJK content are mapped back to ASCII forms so
/// copied commands, code and URLs stay usable. Mixed or CJK lines keep their
/// original punctuation.
fn normalize_latin_punctuation(line: &mut OcrLine) {
    if line.text.chars().any(is_cjk) {
        return;
    }
    line.text = line.text.chars().map(normalize_latin_char).collect();
    for item in &mut line.chars {
        item.ch = normalize_latin_char(item.ch);
    }
}

fn normalize_latin_char(ch: char) -> char {
    let code = ch as u32;
    if (0xFF01..=0xFF5E).contains(&code) {
        // Full-width ASCII block maps one-to-one onto U+0021..U+007E.
        return char::from_u32(code - 0xFEE0).unwrap_or(ch);
    }
    match ch {
        '\u{3000}' => ' ',
        '\u{3002}' => '.',
        '\u{3001}' => ',',
        _ => ch,
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3040..=0x30FF   // kana
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xAC00..=0xD7AF // hangul syllables
            | 0xF900..=0xFAFF // CJK compatibility ideographs
    )
}

/// Joins recognized lines back into one text, treating lines that a paragraph
/// wrapped as a soft join instead of a newline. The decision uses sentence
/// punctuation, list markers, vertical gaps, paragraph indentation and short
/// paragraph-ending lines, which covers UI text and prose without merging
/// unrelated rows.
pub fn join_ocr_lines(lines: &[OcrLine]) -> String {
    let Some((first, rest)) = lines.split_first() else {
        return String::new();
    };
    let max_right = lines
        .iter()
        .map(|line| line.rect.origin.x + line.rect.size.width as i32)
        .max()
        .unwrap_or(0);
    let mut text = first.text.clone();
    let mut previous = first;
    for next in rest {
        let separator = line_join_separator(previous, next, max_right);
        if separator == " "
            && text.ends_with('-')
            && next
                .text
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_lowercase())
        {
            // A word split across a wrap keeps no space or hyphen.
            text.pop();
        } else {
            text.push_str(separator);
        }
        text.push_str(&next.text);
        previous = next;
    }
    text
}

/// Separator for two adjacent recognized lines. `block_right` is the right
/// edge of the widest line in the block, used to spot paragraph-ending short
/// lines. Returns `"\n"` for hard breaks, `" "` for wrapped Latin text and
/// `""` for wrapped CJK text.
pub fn line_join_separator(previous: &OcrLine, next: &OcrLine, block_right: i32) -> &'static str {
    let previous_text = previous.text.trim_end();
    let next_text = next.text.trim_start();
    if previous_text.is_empty() || next_text.is_empty() {
        return "\n";
    }
    let height = previous.rect.size.height.max(next.rect.size.height).max(1) as f32;
    let gap = next.rect.origin.y as f32
        - (previous.rect.origin.y + previous.rect.size.height as i32) as f32;
    let previous_right = previous.rect.origin.x + previous.rect.size.width as i32;
    let indent = next.rect.origin.x - previous.rect.origin.x;

    if gap > height * 0.6
        || ends_sentence(previous_text)
        || starts_list_item(next_text)
        || indent as f32 > height * 0.8
        || (previous_right as f32) < block_right as f32 - height * 2.0
    {
        return "\n";
    }

    let last = previous_text.chars().next_back();
    let first = next_text.chars().next();
    match (last, first) {
        (Some(last), Some(first)) if last.is_ascii() && first.is_ascii() => {
            if !needs_join_space(last, first) {
                ""
            } else {
                " "
            }
        }
        _ => "",
    }
}

fn ends_sentence(text: &str) -> bool {
    let mut chars = text.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    if is_sentence_end(last) {
        return true;
    }
    if matches!(
        last,
        '"' | '\'' | ')' | ']' | '}' | '\u{201D}' | '\u{2019}' | '\u{FF09}' | '\u{3011}'
    ) {
        return chars.next().is_some_and(is_sentence_end);
    }
    false
}

fn is_sentence_end(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | '\u{2026}' | '\u{3002}' | '\u{FF01}' | '\u{FF1F}'
    )
}

fn starts_list_item(text: &str) -> bool {
    let text = text.trim_start();
    let Some(first) = text.chars().next() else {
        return false;
    };
    if matches!(first, '•' | '·' | '●' | '○' | '▪' | '▫' | '*' | '–' | '—') {
        return true;
    }
    if first.is_ascii_digit() {
        let mut rest = text.chars().skip_while(|ch| ch.is_ascii_digit());
        return matches!(rest.next(), Some('.' | ')' | '\u{3001}' | '\u{FF09}'));
    }
    first == '-' && text.chars().nth(1).is_some_and(char::is_whitespace)
}

fn needs_join_space(previous: char, next: char) -> bool {
    if !previous.is_ascii() || !next.is_ascii() {
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
    fn latin_lines_normalize_full_width_punctuation() {
        let mut line = build_line(
            "Hello\u{FF0C}world\u{3002}".to_owned(),
            PhysicalRect::new(0, 0, 200, 20),
        );
        normalize_latin_punctuation(&mut line);
        assert_eq!(line.text, "Hello,world.");
        assert_eq!(
            line.chars.iter().map(|ch| ch.ch).collect::<String>(),
            "Hello,world."
        );

        let mut cjk = build_line(
            "\u{4F60}\u{597D}\u{FF0C}\u{4E16}\u{754C}".to_owned(),
            PhysicalRect::new(0, 0, 200, 20),
        );
        normalize_latin_punctuation(&mut cjk);
        assert_eq!(cjk.text, "\u{4F60}\u{597D}\u{FF0C}\u{4E16}\u{754C}");
    }

    #[test]
    fn wrapped_latin_lines_join_with_a_space() {
        let lines = vec![
            build_line(
                "The quick brown fox".to_owned(),
                PhysicalRect::new(0, 0, 190, 20),
            ),
            build_line(
                "jumps over the dog".to_owned(),
                PhysicalRect::new(0, 22, 160, 20),
            ),
        ];
        assert_eq!(
            join_ocr_lines(&lines),
            "The quick brown fox jumps over the dog"
        );
    }

    #[test]
    fn sentence_ends_and_list_markers_keep_hard_breaks() {
        let lines = vec![
            build_line(
                "First sentence.".to_owned(),
                PhysicalRect::new(0, 0, 190, 20),
            ),
            build_line(
                "Second sentence".to_owned(),
                PhysicalRect::new(0, 22, 140, 20),
            ),
            build_line(
                "\u{2022} bullet".to_owned(),
                PhysicalRect::new(0, 44, 80, 20),
            ),
        ];
        assert_eq!(
            join_ocr_lines(&lines),
            "First sentence.\nSecond sentence\n\u{2022} bullet"
        );
    }

    #[test]
    fn wrapped_cjk_lines_join_without_spaces() {
        let lines = vec![
            build_line(
                "\u{622A}\u{56FE}\u{5DE5}\u{5177}".to_owned(),
                PhysicalRect::new(0, 0, 160, 20),
            ),
            build_line(
                "\u{5E94}\u{7528}\u{7A0B}\u{5E8F}".to_owned(),
                PhysicalRect::new(0, 22, 160, 20),
            ),
        ];
        assert_eq!(
            join_ocr_lines(&lines),
            "\u{622A}\u{56FE}\u{5DE5}\u{5177}\u{5E94}\u{7528}\u{7A0B}\u{5E8F}"
        );
    }

    #[test]
    fn wrapped_hyphenation_drops_the_hyphen() {
        let lines = vec![
            build_line("inter-".to_owned(), PhysicalRect::new(0, 0, 190, 20)),
            build_line("national".to_owned(), PhysicalRect::new(0, 22, 190, 20)),
        ];
        assert_eq!(join_ocr_lines(&lines), "international");
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
