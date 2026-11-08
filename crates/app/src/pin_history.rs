use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use core_types::RgbaFrame;
use serde::{Deserialize, Serialize};

use crate::storage;

const HISTORY_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const HISTORY_LIMIT: usize = 50;
const HISTORY_MANIFEST: &str = "history.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HistoryEntry {
    file: String,
    created_at: u64,
    hash: u64,
}

thread_local! {
    static ENTRIES: RefCell<Option<Vec<HistoryEntry>>> = const { RefCell::new(None) };
    static CURSOR: Cell<usize> = const { Cell::new(0) };
    static LAST_HASH: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Records a pinned image for the recent-pin history. Consecutive identical
/// images are deduplicated so holding the pin hotkey does not fill the list.
pub fn record(frame: &RgbaFrame) {
    let hash = hash_frame(frame);
    let Some(directory) = history_dir() else {
        return;
    };
    let now = now_millis();
    let mut entries = entries();
    if let Some(first) = entries.first_mut().filter(|entry| entry.hash == hash) {
        // Re-pinning the newest image refreshes its age instead of creating a
        // duplicate entry.
        first.created_at = now;
        write_entries(&directory, &entries);
        ENTRIES.with(|slot| *slot.borrow_mut() = Some(entries));
        reset_cursor(hash);
        return;
    }

    let Some(file) = save_image(&directory, frame, now) else {
        return;
    };
    entries.insert(
        0,
        HistoryEntry {
            file,
            created_at: now,
            hash,
        },
    );
    prune(&mut entries, &directory, now);
    write_entries(&directory, &entries);
    ENTRIES.with(|slot| *slot.borrow_mut() = Some(entries));
    reset_cursor(hash);
}

/// Returns the next older pinned image after the one matching `hash`.
pub fn cycle(hash: u64) -> Option<RgbaFrame> {
    let entries = entries();
    if entries.is_empty() {
        return None;
    }
    let directory = history_dir()?;
    let same_source = LAST_HASH.with(|last| last.get()) == Some(hash);
    let mut cursor = if same_source {
        CURSOR.with(|cursor| cursor.get())
    } else {
        LAST_HASH.with(|last| last.set(Some(hash)));
        0
    };

    if entries.len() == 1 {
        CURSOR.with(|slot| slot.set(0));
        return load_entry(&directory, &entries[0]);
    }

    for _ in 0..entries.len() {
        cursor = (cursor + 1) % entries.len();
        if entries[cursor].hash == hash {
            continue;
        }
        if let Some(frame) = load_entry(&directory, &entries[cursor]) {
            CURSOR.with(|slot| slot.set(cursor));
            return Some(frame);
        }
    }
    None
}

/// Stable in-process hash for matching the clipboard image against history.
pub fn hash_frame(frame: &RgbaFrame) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in frame.pixels() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash ^= u64::from(frame.width()) << 32;
    hash ^= u64::from(frame.height());
    hash
}

fn reset_cursor(hash: u64) {
    CURSOR.with(|cursor| cursor.set(0));
    LAST_HASH.with(|last| last.set(Some(hash)));
}

fn entries() -> Vec<HistoryEntry> {
    ENTRIES.with(|slot| {
        if slot.borrow().is_none() {
            let loaded = load_entries();
            *slot.borrow_mut() = Some(loaded);
        }
        slot.borrow().clone().unwrap_or_default()
    })
}

fn load_entries() -> Vec<HistoryEntry> {
    let Some(directory) = history_dir() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(directory.join(HISTORY_MANIFEST)) else {
        return Vec::new();
    };
    let Ok(mut entries) = serde_json::from_str::<Vec<HistoryEntry>>(&text) else {
        return Vec::new();
    };
    let before = entries.len();
    let now = now_millis();
    prune(&mut entries, &directory, now);
    if entries.len() != before {
        write_entries(&directory, &entries);
    }
    entries
}

fn prune(entries: &mut Vec<HistoryEntry>, directory: &Path, now: u64) {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    let mut kept = Vec::with_capacity(entries.len().min(HISTORY_LIMIT));
    for entry in entries.drain(..) {
        let Some(file) = Path::new(&entry.file).file_name() else {
            continue;
        };
        let path = directory.join(file);
        if kept.len() >= HISTORY_LIMIT
            || entry.created_at.saturating_add(HISTORY_TTL_MS) < now
            || !path.exists()
        {
            let _ = std::fs::remove_file(path);
            continue;
        }
        kept.push(entry);
    }
    *entries = kept;
}

fn write_entries(directory: &Path, entries: &[HistoryEntry]) {
    let path = directory.join(HISTORY_MANIFEST);
    if entries.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(entries) {
        let _ = std::fs::write(path, text);
    }
}

fn load_entry(directory: &Path, entry: &HistoryEntry) -> Option<RgbaFrame> {
    let file = Path::new(&entry.file).file_name()?;
    let image = image::open(directory.join(file)).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    RgbaFrame::from_pixels(width, height, image.into_raw()).ok()
}

fn save_image(directory: &Path, frame: &RgbaFrame, now: u64) -> Option<String> {
    let mut path = directory.join(format!("history-{now}.png"));
    let mut suffix = 1u32;
    while path.exists() {
        path = directory.join(format!("history-{now}-{suffix}.png"));
        suffix += 1;
    }
    image::save_buffer(
        &path,
        frame.pixels(),
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
    )
    .ok()?;
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn history_dir() -> Option<PathBuf> {
    let directory = storage::pins_dir()?.join("history");
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(red: u8) -> RgbaFrame {
        RgbaFrame::from_pixels(
            2,
            2,
            vec![
                red, 0, 0, 255, red, 0, 0, 255, red, 0, 0, 255, red, 0, 0, 255,
            ],
        )
        .unwrap()
    }

    #[test]
    fn hash_matches_identical_frames() {
        assert_eq!(hash_frame(&frame(1)), hash_frame(&frame(1)));
        assert_ne!(hash_frame(&frame(1)), hash_frame(&frame(2)));
    }

    #[test]
    fn prune_keeps_only_the_most_recent_entries() {
        let directory = std::env::temp_dir().join(format!(
            "capin-history-test-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut entries = Vec::new();
        for index in 0..(HISTORY_LIMIT + 3) {
            let file = format!("history-{index}.png");
            std::fs::write(directory.join(&file), b"x").unwrap();
            entries.push(HistoryEntry {
                file,
                created_at: index as u64 + 1,
                hash: index as u64,
            });
        }
        let now = HISTORY_LIMIT as u64 + 10;
        prune(&mut entries, &directory, now);
        assert_eq!(entries.len(), HISTORY_LIMIT);
        assert_eq!(
            entries.first().unwrap().created_at,
            HISTORY_LIMIT as u64 + 3
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
