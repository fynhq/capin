use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub(super) const MAX_PERSISTED_PINS: usize = 50;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PersistedPin {
    pub(super) id: u64,
    pub(super) kind: String,
    pub(super) image: Option<String>,
    pub(super) text: Option<String>,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) opacity: f32,
    pub(super) zoom: f32,
    #[serde(default)]
    pub(super) click_through: bool,
    #[serde(default)]
    pub(super) shadow: bool,
    #[serde(default)]
    pub(super) export_padding: u32,
    #[serde(default)]
    pub(super) display: Option<u64>,
}

pub(super) fn load(directory: &Path) -> Vec<PersistedPin> {
    let path = directory.join("pins.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(mut entries) = serde_json::from_str::<Vec<PersistedPin>>(&text) else {
        crate::logging::log_warn!("pin", "pin manifest is corrupt; keeping a backup copy");
        let backup = directory.join("pins.json.corrupt");
        if let Err(error) = std::fs::rename(&path, &backup) {
            crate::logging::log_error!("pin", "failed to back up the corrupt manifest: {error}");
        }
        return Vec::new();
    };
    sort_restore_entries(&mut entries);
    entries
}

pub(super) fn save(directory: &Path, mut entries: Vec<PersistedPin>) {
    entries.sort_by_key(|entry| entry.id);
    entries.dedup_by_key(|entry| entry.id);
    let path = directory.join("pins.json");
    if entries.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&entries)
        && let Err(error) = crate::persist::write_atomic(&path, text.as_bytes())
    {
        crate::logging::log_error!("pin", "failed to save the pin manifest: {error}");
    }
}

pub(super) fn sort_restore_entries(entries: &mut [PersistedPin]) {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.id));
}

fn image_name(entry: &PersistedPin) -> Option<String> {
    entry
        .image
        .as_deref()
        .and_then(|name| Path::new(name).file_name())
        .map(|name| name.to_string_lossy().into_owned())
}

pub(super) fn enforce_limit(directory: &Path, entries: &mut Vec<PersistedPin>) {
    entries.sort_by_key(|entry| entry.id);
    entries.dedup_by_key(|entry| entry.id);
    let removed: Vec<PersistedPin> = if entries.len() > MAX_PERSISTED_PINS {
        let excess = entries.len() - MAX_PERSISTED_PINS;
        entries.drain(..excess).collect()
    } else {
        Vec::new()
    };
    let kept: HashSet<String> = entries.iter().filter_map(image_name).collect();
    for entry in &removed {
        if let Some(name) = image_name(entry)
            && !kept.contains(&name)
        {
            let _ = std::fs::remove_file(directory.join(name));
        }
    }
    let Ok(read) = std::fs::read_dir(directory) else {
        return;
    };
    for file in read.flatten() {
        let name = file.file_name().to_string_lossy().into_owned();
        if name.starts_with("pin-") && name.ends_with(".png") && !kept.contains(&name) {
            let _ = std::fs::remove_file(file.path());
        }
    }
}

pub(super) fn is_usable(entry: &PersistedPin, directory: &Path) -> bool {
    if !entry.x.is_finite()
        || !entry.y.is_finite()
        || !entry.width.is_finite()
        || !entry.height.is_finite()
        || entry.width < 1.0
        || entry.height < 1.0
    {
        return false;
    }
    match entry.kind.as_str() {
        "image" => entry
            .image
            .as_deref()
            .and_then(|name| Path::new(name).file_name())
            .is_some_and(|name| directory.join(name).exists()),
        "text" => entry.text.as_deref().is_some_and(|text| !text.is_empty()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PERSISTED_PINS, PersistedPin, enforce_limit, is_usable, load, save,
        sort_restore_entries,
    };

    fn entry(id: u64) -> PersistedPin {
        PersistedPin {
            id,
            kind: "text".to_owned(),
            image: None,
            text: Some(format!("pin-{id}")),
            x: 10.0,
            y: 20.0,
            width: 120.0,
            height: 80.0,
            opacity: 1.0,
            zoom: 1.0,
            click_through: false,
            shadow: false,
            export_padding: 0,
            display: Some(7),
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "capin-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn restore_order_prefers_newest() {
        let mut entries = vec![entry(1), entry(30), entry(12), entry(25)];
        sort_restore_entries(&mut entries);
        let ids: Vec<u64> = entries.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, vec![30, 25, 12, 1]);
    }

    #[test]
    fn entries_round_trip_with_display_and_shadow() {
        let directory = temp_dir();
        let mut entry = entry(42);
        entry.shadow = true;
        save(&directory, vec![entry.clone()]);
        let loaded = load(&directory);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, 42);
        assert_eq!(loaded[0].display, Some(7));
        assert!(loaded[0].shadow);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn unusable_entries_are_rejected() {
        let directory = temp_dir();
        let mut image = entry(1);
        image.kind = "image".to_owned();
        image.image = Some("missing.png".to_owned());
        assert!(!is_usable(&image, &directory));
        let mut invalid = entry(2);
        invalid.width = 0.0;
        assert!(!is_usable(&invalid, &directory));
        assert!(is_usable(&entry(3), &directory));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn corrupt_manifest_is_backed_up_and_loads_empty() {
        let directory = temp_dir();
        let path = directory.join("pins.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load(&directory).is_empty());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(directory.join("pins.json.corrupt")).unwrap(),
            b"{not json"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    fn image_entry(id: u64) -> PersistedPin {
        let mut entry = entry(id);
        entry.kind = "image".to_owned();
        entry.text = None;
        entry.image = Some(format!("pin-{id}.png"));
        entry
    }

    #[test]
    fn limit_evicts_oldest_pins_and_their_files() {
        let directory = temp_dir();
        let total = MAX_PERSISTED_PINS + 5;
        let mut entries = Vec::new();
        for id in 1..=total as u64 {
            std::fs::write(directory.join(format!("pin-{id}.png")), b"png").unwrap();
            entries.push(image_entry(id));
        }
        enforce_limit(&directory, &mut entries);
        assert_eq!(entries.len(), MAX_PERSISTED_PINS);
        assert_eq!(entries[0].id, 6);
        for id in 1..=5 {
            assert!(!directory.join(format!("pin-{id}.png")).exists());
        }
        for id in 6..=total as u64 {
            assert!(directory.join(format!("pin-{id}.png")).exists());
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn orphan_sweep_keeps_referenced_and_unrelated_files() {
        let directory = temp_dir();
        std::fs::write(directory.join("pin-1.png"), b"png").unwrap();
        std::fs::write(directory.join("pin-999.png"), b"png").unwrap();
        std::fs::write(directory.join("notes.txt"), b"keep").unwrap();
        std::fs::write(directory.join("pins.json"), b"[]").unwrap();
        let mut entries = vec![image_entry(1)];
        enforce_limit(&directory, &mut entries);
        assert!(directory.join("pin-1.png").exists());
        assert!(!directory.join("pin-999.png").exists());
        assert!(directory.join("notes.txt").exists());
        assert!(directory.join("pins.json").exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn evicted_entry_sharing_a_file_keeps_the_file() {
        let directory = temp_dir();
        std::fs::write(directory.join("pin-shared.png"), b"png").unwrap();
        let mut oldest = image_entry(1);
        oldest.image = Some("pin-shared.png".to_owned());
        let mut newest = image_entry(2);
        newest.image = Some("pin-shared.png".to_owned());
        let mut entries = vec![oldest, newest];
        for id in 3..=(MAX_PERSISTED_PINS as u64 + 1) {
            entries.push(image_entry(id));
        }
        enforce_limit(&directory, &mut entries);
        assert_eq!(entries.len(), MAX_PERSISTED_PINS);
        assert!(directory.join("pin-shared.png").exists());
        let _ = std::fs::remove_dir_all(directory);
    }
}
