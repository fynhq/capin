use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use core_types::RgbaFrame;
use serde::{Deserialize, Serialize};

use crate::logging::log_warn;
use crate::storage;

const HISTORY_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const HISTORY_LIMIT: usize = 50;
const HISTORY_MANIFEST: &str = "history.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HistoryEntry {
    file: String,
    created_at: u64,
    hash: u64,
    #[serde(default)]
    pin_shadow: Option<HistoryPinShadow>,
    #[serde(default)]
    scale: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryPinShadow {
    pub corner_radius: u32,
    pub scale: f32,
}

pub struct HistoryImage {
    pub frame: RgbaFrame,
    pub pin_shadow: Option<HistoryPinShadow>,
    pub scale: Option<f32>,
}

thread_local! {
    static ENTRIES: RefCell<Option<std::rc::Rc<Vec<HistoryEntry>>>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

pub fn generation() -> u64 {
    GENERATION.with(Cell::get)
}

pub fn record_png_bytes(
    bytes: &[u8],
    hash: u64,
    pin_shadow: Option<HistoryPinShadow>,
    scale: Option<f32>,
) -> bool {
    let recorded = record_png(hash, pin_shadow, scale, |path| {
        crate::persist::write_atomic(path, bytes).is_ok()
    });
    if recorded {
        GENERATION.with(|cell| cell.set(cell.get().wrapping_add(1)));
    }
    recorded
}

pub fn record_frame(
    frame: &RgbaFrame,
    hash: u64,
    pin_shadow: Option<HistoryPinShadow>,
    scale: Option<f32>,
) -> bool {
    let Ok(bytes) = crate::image_util::encode_png(frame) else {
        return false;
    };
    record_png_bytes(&bytes, hash, pin_shadow, scale)
}

pub fn record_png_file(
    source: &Path,
    hash: u64,
    pin_shadow: Option<HistoryPinShadow>,
    scale: Option<f32>,
) -> bool {
    let recorded = record_png(hash, pin_shadow, scale, |path| {
        crate::persist::copy_atomic(source, path).is_ok()
    });
    if recorded {
        GENERATION.with(|cell| cell.set(cell.get().wrapping_add(1)));
    }
    recorded
}

fn record_png(
    hash: u64,
    pin_shadow: Option<HistoryPinShadow>,
    scale: Option<f32>,
    write: impl FnOnce(&Path) -> bool,
) -> bool {
    let Some(directory) = history_dir() else {
        return false;
    };
    let now = now_millis();
    let mut entries = entries();
    let list = std::rc::Rc::make_mut(&mut entries);
    if let Some(first) = list.first_mut().filter(|entry| entry.hash == hash) {
        first.created_at = now;
        first.pin_shadow = pin_shadow;
        first.scale = scale;
        write_entries(&directory, list);
        ENTRIES.with(|slot| *slot.borrow_mut() = Some(entries));
        return true;
    }

    let path = unique_history_path(&directory, now);
    if !write(&path) {
        return false;
    }
    let Some(file) = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return false;
    };

    if let Some(index) = list.iter().position(|entry| entry.hash == hash) {
        let duplicate = list.remove(index);
        if let Some(name) = Path::new(&duplicate.file).file_name() {
            let _ = std::fs::remove_file(directory.join(name));
        }
    }
    list.insert(
        0,
        HistoryEntry {
            file,
            created_at: now,
            hash,
            pin_shadow,
            scale,
        },
    );
    prune(list, &directory, now);
    write_entries(&directory, list);
    ENTRIES.with(|slot| *slot.borrow_mut() = Some(entries));
    true
}

pub fn cycle(hash: u64) -> Option<HistoryImage> {
    let entries = entries();
    if entries.is_empty() {
        return None;
    }
    let directory = history_dir()?;
    let mut index = older_index(&entries, hash);
    while let Some(entry) = entries.get(index) {
        if entry.hash != hash
            && let Some(frame) = load_entry(&directory, entry)
        {
            return Some(HistoryImage {
                frame,
                pin_shadow: entry.pin_shadow,
                scale: entry.scale,
            });
        }
        index += 1;
    }
    None
}

fn older_index(entries: &[HistoryEntry], hash: u64) -> usize {
    match entries.iter().position(|entry| entry.hash == hash) {
        Some(index) => index + 1,
        None => 0,
    }
}

pub fn contains(hash: u64) -> bool {
    entries().iter().any(|entry| entry.hash == hash)
}

pub fn has_entries() -> bool {
    !entries().is_empty()
}

pub fn find(hash: u64) -> Option<HistoryImage> {
    let entries = entries();
    let directory = history_dir()?;
    entries
        .iter()
        .find(|entry| entry.hash == hash)
        .and_then(|entry| {
            Some(HistoryImage {
                frame: load_entry(&directory, entry)?,
                pin_shadow: entry.pin_shadow,
                scale: entry.scale,
            })
        })
}

pub fn latest() -> Option<HistoryImage> {
    let entries = entries();
    let directory = history_dir()?;
    entries.iter().find_map(|entry| {
        Some(HistoryImage {
            frame: load_entry(&directory, entry)?,
            pin_shadow: entry.pin_shadow,
            scale: entry.scale,
        })
    })
}

pub fn hash_frame(frame: &RgbaFrame) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let packed = frame.packed_pixels();
    for byte in packed.as_ref() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash ^= u64::from(frame.width()) << 32;
    hash ^= u64::from(frame.height());
    hash
}

fn entries() -> std::rc::Rc<Vec<HistoryEntry>> {
    ENTRIES.with(|slot| {
        if slot.borrow().is_none() {
            let loaded = load_entries();
            *slot.borrow_mut() = Some(std::rc::Rc::new(loaded));
        }
        slot.borrow().as_ref().cloned().unwrap_or_default()
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
        log_warn!(
            "history",
            "history manifest is corrupt; keeping recent images"
        );
        cleanup_orphans(&directory, now_millis(), &HashSet::new());
        return Vec::new();
    };
    let before = entries.len();
    let now = now_millis();
    prune(&mut entries, &directory, now);
    if entries.len() != before {
        write_entries(&directory, &entries);
    }
    let referenced = entries
        .iter()
        .filter_map(|entry| {
            Path::new(&entry.file)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect::<HashSet<_>>();
    cleanup_orphans(&directory, now, &referenced);
    entries
}

fn prune(entries: &mut Vec<HistoryEntry>, directory: &Path, now: u64) {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    let mut kept = Vec::with_capacity(entries.len().min(HISTORY_LIMIT));
    let mut seen = HashSet::with_capacity(entries.len());
    for entry in entries.drain(..) {
        let Some(file) = Path::new(&entry.file).file_name() else {
            continue;
        };
        let path = directory.join(file);
        if kept.len() >= HISTORY_LIMIT
            || entry.created_at.saturating_add(HISTORY_TTL_MS) < now
            || !path.exists()
            || !seen.insert(entry.hash)
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
    if let Ok(text) = serde_json::to_string_pretty(entries)
        && let Err(error) = crate::persist::write_atomic(&path, text.as_bytes())
    {
        log_warn!("history", "failed to save the history manifest: {error}");
    }
}

fn cleanup_orphans(directory: &Path, now: u64, referenced: &HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("history-") || !name.ends_with(".png") {
            continue;
        }
        if referenced.contains(name.as_ref()) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        if modified.saturating_add(HISTORY_TTL_MS) < now {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn load_entry(directory: &Path, entry: &HistoryEntry) -> Option<RgbaFrame> {
    let file = Path::new(&entry.file).file_name()?;
    crate::image_util::load_frame(&directory.join(file))
}

fn unique_history_path(directory: &Path, now: u64) -> PathBuf {
    let mut path = directory.join(format!("history-{now}.png"));
    let mut suffix = 1u32;
    while path.exists() {
        path = directory.join(format!("history-{now}-{suffix}.png"));
        suffix += 1;
    }
    path
}

fn history_dir() -> Option<PathBuf> {
    let directory = storage::app_data_dir()?.join("history");
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
                pin_shadow: None,
                scale: None,
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

    #[test]
    fn prune_drops_older_duplicate_hashes() {
        let directory = std::env::temp_dir().join(format!(
            "capin-history-dedupe-test-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut entries = Vec::new();
        for (index, hash) in [(1u64, 7u64), (2, 8), (3, 7), (4, 9)] {
            let file = format!("history-{index}.png");
            std::fs::write(directory.join(&file), b"x").unwrap();
            entries.push(HistoryEntry {
                file,
                created_at: index,
                hash,
                pin_shadow: None,
                scale: None,
            });
        }
        prune(&mut entries, &directory, 100);
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries.iter().map(|entry| entry.hash).collect::<Vec<_>>(),
            vec![9, 7, 8],
            "the newer copy of hash 7 stays ahead of hash 8"
        );
        assert!(!directory.join("history-1.png").exists());
        assert!(directory.join("history-3.png").exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn history_walk_advances_forward_and_stops_at_the_oldest() {
        let entries = vec![
            HistoryEntry {
                file: "new.png".to_owned(),
                created_at: 3,
                hash: 30,
                pin_shadow: None,
                scale: None,
            },
            HistoryEntry {
                file: "mid.png".to_owned(),
                created_at: 2,
                hash: 20,
                pin_shadow: None,
                scale: None,
            },
            HistoryEntry {
                file: "old.png".to_owned(),
                created_at: 1,
                hash: 10,
                pin_shadow: None,
                scale: None,
            },
        ];
        assert_eq!(older_index(&entries, 30), 1);
        assert_eq!(older_index(&entries, 20), 2);
        assert_eq!(older_index(&entries, 10), 3);
        assert_eq!(
            older_index(&entries, 99),
            0,
            "an unknown source starts at the newest screenshot"
        );
    }

    #[test]
    fn next_walk_position_never_repeats_a_hash() {
        let entries = vec![
            HistoryEntry {
                file: "a.png".to_owned(),
                created_at: 3,
                hash: 1,
                pin_shadow: None,
                scale: None,
            },
            HistoryEntry {
                file: "b.png".to_owned(),
                created_at: 2,
                hash: 2,
                pin_shadow: None,
                scale: None,
            },
            HistoryEntry {
                file: "c.png".to_owned(),
                created_at: 1,
                hash: 3,
                pin_shadow: None,
                scale: None,
            },
        ];
        let mut index = older_index(&entries, 1);
        assert_eq!(index, 1);
        index = older_index(&entries, entries[index].hash);
        assert_eq!(index, 2);
        index = older_index(&entries, entries[index].hash);
        assert_eq!(index, entries.len());
    }
}
