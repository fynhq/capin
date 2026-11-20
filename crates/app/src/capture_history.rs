use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use core_types::RgbaFrame;
use serde::{Deserialize, Serialize};

use crate::logging::{log_debug, log_warn};
use crate::paths::history_dir;

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
    #[serde(default)]
    native_shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryPinShadow {
    pub corner_radius: u32,
    pub scale: f32,
}

pub struct HistoryImage {
    pub frame: RgbaFrame,
    pub hash: u64,
    pub pin_shadow: Option<HistoryPinShadow>,
    pub scale: Option<f32>,
    pub native_shadow: bool,
}

thread_local! {
    static STORE: RefCell<Option<HistoryStore>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

struct HistoryStore {
    directory: PathBuf,
    entries: Vec<HistoryEntry>,
}

fn with_store<T>(action: impl FnOnce(&mut HistoryStore) -> T) -> Option<T> {
    let directory = history_dir()?;
    STORE.with(|slot| {
        let mut store = slot.borrow_mut();
        if store.is_none() {
            *store = Some(HistoryStore::load(directory));
        }
        store.as_mut().map(action)
    })
}

pub fn generation() -> u64 {
    GENERATION.with(Cell::get)
}

impl HistoryStore {
    fn load(directory: PathBuf) -> Self {
        let Ok(text) = std::fs::read_to_string(directory.join(HISTORY_MANIFEST)) else {
            return Self {
                directory,
                entries: Vec::new(),
            };
        };
        let Ok(mut entries) = serde_json::from_str::<Vec<HistoryEntry>>(&text) else {
            log_warn!(
                "history",
                "history manifest is corrupt; keeping recent images"
            );
            cleanup_orphans(&directory, now_millis(), &HashSet::new());
            return Self {
                directory,
                entries: Vec::new(),
            };
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
        Self { directory, entries }
    }

    fn record(
        &mut self,
        hash: u64,
        pin_shadow: Option<HistoryPinShadow>,
        scale: Option<f32>,
        native_shadow: bool,
        write: impl FnOnce(&Path) -> bool,
    ) -> bool {
        let now = now_millis();
        let updated_first =
            if let Some(first) = self.entries.first_mut().filter(|entry| entry.hash == hash) {
                first.created_at = now;
                first.pin_shadow = pin_shadow;
                first.scale = scale;
                first.native_shadow = native_shadow;
                true
            } else {
                false
            };
        if updated_first {
            write_entries(&self.directory, &self.entries);
            return true;
        }

        let path = unique_history_path(&self.directory, now);
        if !write(&path) {
            return false;
        }
        let Some(file) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return false;
        };
        if let Some(index) = self.entries.iter().position(|entry| entry.hash == hash) {
            let duplicate = self.entries.remove(index);
            if let Some(name) = Path::new(&duplicate.file).file_name() {
                let _ = std::fs::remove_file(self.directory.join(name));
            }
        }
        self.entries.insert(
            0,
            HistoryEntry {
                file,
                created_at: now,
                hash,
                pin_shadow,
                scale,
                native_shadow,
            },
        );
        prune(&mut self.entries, &self.directory, now);
        write_entries(&self.directory, &self.entries);
        true
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn cycle(&self, hash: u64) -> Option<HistoryImage> {
        if self.entries.is_empty() {
            return None;
        }
        let mut index = older_index(&self.entries, hash);
        while let Some(entry) = self.entries.get(index) {
            if entry.hash != hash
                && let Some(frame) = load_entry(&self.directory, entry)
            {
                return Some(HistoryImage {
                    frame,
                    hash: entry.hash,
                    pin_shadow: entry.pin_shadow,
                    scale: entry.scale,
                    native_shadow: entry.native_shadow,
                });
            }
            index += 1;
        }
        None
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn find(&self, hash: u64) -> Option<HistoryImage> {
        self.entries
            .iter()
            .find(|entry| entry.hash == hash)
            .and_then(|entry| {
                Some(HistoryImage {
                    frame: load_entry(&self.directory, entry)?,
                    hash: entry.hash,
                    pin_shadow: entry.pin_shadow,
                    scale: entry.scale,
                    native_shadow: entry.native_shadow,
                })
            })
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn latest(&self) -> Option<HistoryImage> {
        self.entries.iter().find_map(|entry| {
            Some(HistoryImage {
                frame: load_entry(&self.directory, entry)?,
                hash: entry.hash,
                pin_shadow: entry.pin_shadow,
                scale: entry.scale,
                native_shadow: entry.native_shadow,
            })
        })
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn contains(&self, hash: u64) -> bool {
        self.entries.iter().any(|entry| entry.hash == hash)
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }
}

pub fn record_png_bytes(
    bytes: &[u8],
    hash: u64,
    pin_shadow: Option<HistoryPinShadow>,
    scale: Option<f32>,
    native_shadow: bool,
) -> bool {
    let started = Instant::now();
    let recorded = with_store(|store| {
        store.record(hash, pin_shadow, scale, native_shadow, |path| {
            crate::persist::write_atomic(path, bytes).is_ok()
        })
    })
    .unwrap_or(false);
    log_debug!(
        "history",
        "record bytes={} duration_ms={} ok={recorded}",
        bytes.len(),
        started.elapsed().as_millis()
    );
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
    native_shadow: bool,
) -> bool {
    let started = Instant::now();
    let Ok(bytes) = crate::image_util::encode_png(frame) else {
        return false;
    };
    log_debug!(
        "history",
        "encode frame={}x{} bytes={} duration_ms={}",
        frame.width(),
        frame.height(),
        bytes.len(),
        started.elapsed().as_millis()
    );
    record_png_bytes(&bytes, hash, pin_shadow, scale, native_shadow)
}

pub fn record_png_file(
    source: &Path,
    hash: u64,
    pin_shadow: Option<HistoryPinShadow>,
    scale: Option<f32>,
    native_shadow: bool,
) -> bool {
    let started = Instant::now();
    let recorded = with_store(|store| {
        store.record(hash, pin_shadow, scale, native_shadow, |path| {
            crate::persist::copy_atomic(source, path).is_ok()
        })
    })
    .unwrap_or(false);
    log_debug!(
        "history",
        "record file duration_ms={} ok={recorded}",
        started.elapsed().as_millis()
    );
    if recorded {
        GENERATION.with(|cell| cell.set(cell.get().wrapping_add(1)));
    }
    recorded
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn cycle(hash: u64) -> Option<HistoryImage> {
    with_store(|store| store.cycle(hash)).flatten()
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn older_index(entries: &[HistoryEntry], hash: u64) -> usize {
    match entries.iter().position(|entry| entry.hash == hash) {
        Some(index) => index + 1,
        None => 0,
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn contains(hash: u64) -> bool {
    with_store(|store| store.contains(hash)).unwrap_or(false)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn has_entries() -> bool {
    with_store(|store| store.has_entries()).unwrap_or(false)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn find(hash: u64) -> Option<HistoryImage> {
    with_store(|store| store.find(hash)).flatten()
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn latest() -> Option<HistoryImage> {
    with_store(|store| store.latest()).flatten()
}

pub fn hash_frame(frame: &RgbaFrame) -> u64 {
    let mut hash = core_types::fnv1a64(frame.packed_pixels().as_ref());
    hash ^= u64::from(frame.width()) << 32;
    hash ^= u64::from(frame.height());
    hash
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

#[cfg(any(target_os = "windows", target_os = "macos"))]
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
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn store_records_reloads_and_returns_the_latest_image() {
        let directory = std::env::temp_dir().join(format!(
            "capin-history-store-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let source = frame(3);
        let bytes = crate::image_util::encode_png(&source).unwrap();
        let hash = hash_frame(&source);
        let mut store = HistoryStore::load(directory.clone());
        assert!(store.record(hash, None, Some(2.0), true, |path| {
            std::fs::write(path, &bytes).is_ok()
        }));
        assert!(store.contains(hash));
        assert!(store.has_entries());

        let latest = store.latest().expect("latest history image");
        assert_eq!(latest.hash, hash);
        assert_eq!(latest.scale, Some(2.0));
        assert!(latest.native_shadow);
        assert_eq!(latest.frame.pixels(), source.pixels());

        drop(store);
        let reloaded = HistoryStore::load(directory.clone());
        assert!(reloaded.contains(hash));
        let _ = std::fs::remove_dir_all(directory);
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
                native_shadow: false,
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
                native_shadow: false,
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
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn history_walk_advances_forward_and_stops_at_the_oldest() {
        let entries = vec![
            HistoryEntry {
                file: "new.png".to_owned(),
                created_at: 3,
                hash: 30,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
            },
            HistoryEntry {
                file: "mid.png".to_owned(),
                created_at: 2,
                hash: 20,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
            },
            HistoryEntry {
                file: "old.png".to_owned(),
                created_at: 1,
                hash: 10,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
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
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn next_walk_position_never_repeats_a_hash() {
        let entries = vec![
            HistoryEntry {
                file: "a.png".to_owned(),
                created_at: 3,
                hash: 1,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
            },
            HistoryEntry {
                file: "b.png".to_owned(),
                created_at: 2,
                hash: 2,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
            },
            HistoryEntry {
                file: "c.png".to_owned(),
                created_at: 1,
                hash: 3,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
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
