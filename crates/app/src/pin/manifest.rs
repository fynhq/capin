use std::path::Path;

use serde::{Deserialize, Serialize};

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
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(mut entries) = serde_json::from_str::<Vec<PersistedPin>>(&text) else {
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
    use super::{PersistedPin, is_usable, load, save, sort_restore_entries};

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
}
