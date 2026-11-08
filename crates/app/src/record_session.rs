use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage;

const MANIFEST: &str = "session.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionManifest {
    format: String,
    started_at: u64,
    state: String,
    #[serde(default)]
    segments: Vec<String>,
}

/// A segmented recording session. Each segment is finalized by the platform
/// recorder, so a crash can lose at most the segment currently being written.
pub struct Session {
    directory: PathBuf,
    manifest: SessionManifest,
}

impl Session {
    pub fn create(format: core_types::RecordFormat) -> Result<Self, std::io::Error> {
        Self::create_in(&storage::recording_dir(), format)
    }

    fn create_in(
        directory: &Path,
        format: core_types::RecordFormat,
    ) -> Result<Self, std::io::Error> {
        storage::ensure_directory(directory)?;
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        for suffix in 0..1000u32 {
            let name = if suffix == 0 {
                format!("Capin_{stamp}")
            } else {
                format!("Capin_{stamp}_{suffix}")
            };
            let candidate = directory.join(name);
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    let session = Self {
                        directory: candidate,
                        manifest: SessionManifest {
                            format: format.extension().to_owned(),
                            started_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|duration| duration.as_millis() as u64)
                                .unwrap_or_default(),
                            state: "recording".to_owned(),
                            segments: Vec::new(),
                        },
                    };
                    session.write()?;
                    return Ok(session);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "too many recording session directories",
        ))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn segment_path(&self, index: u32, extension: &str) -> PathBuf {
        self.directory
            .join(format!("segment_{index:04}.{extension}"))
    }

    pub fn record_segment(&mut self, path: &Path) {
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return;
        };
        if !self.manifest.segments.contains(&name) {
            self.manifest.segments.push(name);
        }
        self.manifest.state = "recording".to_owned();
        let _ = self.write();
    }

    pub fn finish(&mut self) {
        self.manifest.state = "complete".to_owned();
        let _ = self.write();
    }

    pub fn segment_count(&self) -> usize {
        self.manifest.segments.len()
    }

    pub fn discard(self) {
        let _ = std::fs::remove_dir_all(self.directory);
    }

    fn write(&self) -> Result<(), std::io::Error> {
        let text = serde_json::to_string_pretty(&self.manifest).map_err(std::io::Error::other)?;
        std::fs::write(self.directory.join(MANIFEST), text)
    }
}

/// Returns recording session folders that were not marked complete.
pub fn pending_sessions() -> Vec<PathBuf> {
    let directory = storage::recording_dir();
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path.join(MANIFEST)) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<SessionManifest>(&text) else {
            continue;
        };
        if manifest.state == "recording" && !manifest.segments.is_empty() {
            sessions.push(path);
        }
    }
    sessions.sort();
    sessions
}

/// Marks a recovery folder as handled so the tray stops offering it.
pub fn mark_recovered(directory: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(directory.join(MANIFEST)) else {
        return false;
    };
    let Ok(mut manifest) = serde_json::from_str::<SessionManifest>(&text) else {
        return false;
    };
    manifest.state = "recovered".to_owned();
    let Ok(text) = serde_json::to_string_pretty(&manifest) else {
        return false;
    };
    std::fs::write(directory.join(MANIFEST), text).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::RecordFormat;

    #[test]
    fn session_records_segments_and_finishes() {
        let root = std::env::temp_dir().join(format!(
            "capin-session-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let session = Session::create_in(&root, RecordFormat::Mp4).unwrap();
        let first = session.segment_path(0, "mp4");
        let second = session.segment_path(1, "mp4");
        std::fs::write(&first, b"a").unwrap();
        std::fs::write(&second, b"b").unwrap();
        let directory = session.directory().to_path_buf();
        let mut session = session;
        session.record_segment(&first);
        session.record_segment(&second);
        assert_eq!(session.segment_count(), 2);
        session.finish();
        assert!(pending_sessions_for(&root).is_empty());
        let _ = std::fs::remove_dir_all(directory);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pending_sessions_excludes_completed_manifests() {
        let root = std::env::temp_dir().join(format!(
            "capin-session-pending-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let session = Session::create_in(&root, RecordFormat::WebP).unwrap();
        let path = session.segment_path(0, "webp");
        std::fs::write(&path, b"a").unwrap();
        let mut session = session;
        session.record_segment(&path);
        assert_eq!(pending_sessions_for(&root).len(), 1);
        session.finish();
        assert!(pending_sessions_for(&root).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    fn pending_sessions_for(directory: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let text = std::fs::read_to_string(path.join(MANIFEST)).ok()?;
                let manifest = serde_json::from_str::<SessionManifest>(&text).ok()?;
                (manifest.state == "recording" && !manifest.segments.is_empty()).then_some(path)
            })
            .collect()
    }
}
