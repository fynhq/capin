use std::path::Path;
use std::time::UNIX_EPOCH;

use core_types::RgbaFrame;
use gpui_kit::*;

use crate::capture_history;
use crate::logging::log_warn;

pub(super) enum ClipboardContent {
    Image(RgbaFrame),
    Text(String),
    Empty,
}

pub(super) fn read_clipboard(cx: &mut App) -> (ClipboardContent, Option<u64>) {
    let Some(item) = cx.read_from_clipboard() else {
        return (ClipboardContent::Empty, None);
    };
    for entry in item.entries() {
        let ClipboardEntry::Image(image) = entry else {
            continue;
        };
        let signature = Some(clipboard_signature_of(&image.bytes));
        let Some(image) = crate::image_util::decode_untrusted_image(&image.bytes) else {
            continue;
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        if let Ok(frame) = RgbaFrame::from_pixels(width, height, rgba.into_raw()) {
            return (ClipboardContent::Image(frame), signature);
        }
    }
    for entry in item.entries() {
        let ClipboardEntry::ExternalPaths(paths) = entry else {
            continue;
        };
        let Some(path) = paths.0.iter().find(|path| is_image_path(path)) else {
            continue;
        };
        let signature = clipboard_file_signature(path);
        let Some(image) = crate::image_util::decode_untrusted_image_file(path) else {
            continue;
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        if let Ok(frame) = RgbaFrame::from_pixels(width, height, rgba.into_raw()) {
            return (ClipboardContent::Image(frame), signature);
        }
    }
    for entry in item.entries() {
        let ClipboardEntry::String(text) = entry else {
            continue;
        };
        let text = text.text.trim_end();
        if text.is_empty() {
            continue;
        }
        return (
            ClipboardContent::Text(text.to_owned()),
            Some(text_signature(text)),
        );
    }
    (ClipboardContent::Empty, None)
}

pub(super) fn paste_clipboard(cx: &mut App) {
    match read_clipboard(cx).0 {
        ClipboardContent::Image(frame) => {
            let hash = capture_history::hash_frame(&frame);
            capture_history::record_frame(&frame, hash, None, None);
            super::open_history_pin(
                capture_history::HistoryImage {
                    frame,
                    pin_shadow: None,
                    scale: None,
                },
                false,
                cx,
            );
        }
        ClipboardContent::Text(text) => super::open_text_pin(text, cx),
        ClipboardContent::Empty => log_warn!("pin", "clipboard has no image or text to paste"),
    }
}

fn clipboard_signature_of(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

pub(super) fn clipboard_image_signature(cx: &mut App) -> Option<u64> {
    let item = cx.read_from_clipboard()?;
    for entry in item.entries() {
        if let ClipboardEntry::Image(image) = entry {
            return Some(clipboard_signature_of(&image.bytes));
        }
    }
    for entry in item.entries() {
        let ClipboardEntry::ExternalPaths(paths) = entry else {
            continue;
        };
        if let Some(signature) = paths
            .0
            .iter()
            .find(|path| is_image_path(path))
            .and_then(|path| clipboard_file_signature(path))
        {
            return Some(signature);
        }
    }
    None
}

fn clipboard_file_signature(path: &Path) -> Option<u64> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let mut bytes = path.display().to_string().into_bytes();
    bytes.extend_from_slice(&metadata.len().to_le_bytes());
    bytes.extend_from_slice(&modified.to_le_bytes());
    Some(clipboard_signature_of(&bytes))
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp"
            )
        })
}

pub(super) fn clipboard_text_signature(cx: &mut App) -> Option<u64> {
    let item = cx.read_from_clipboard()?;
    for entry in item.entries() {
        let ClipboardEntry::String(text) = entry else {
            continue;
        };
        let text = text.text.trim_end();
        if text.is_empty() {
            continue;
        }
        return Some(text_signature(text));
    }
    None
}

pub(super) fn text_signature(text: &str) -> u64 {
    clipboard_signature_of(text.as_bytes())
}
