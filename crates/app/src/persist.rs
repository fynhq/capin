use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replaces `path` with `bytes` without ever exposing a half-written file.
///
/// The temporary file lives in the same directory so the final rename stays on
/// one filesystem. `std::fs::rename` replaces an existing destination on both
/// Windows (`MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`) and Unix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("path has no parent directory"));
    };
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_owned());

    let mut last_error = None;
    for _ in 0..32 {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{name}.tmp-{}-{stamp}-{counter}",
            std::process::id()
        ));
        let file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };

        let result = write_and_replace(file, &temp, path, bytes);
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        return result;
    }

    Err(last_error.unwrap_or_else(|| std::io::Error::other("could not create a temporary file")))
}

/// Copies `source` to `path` through a temporary file. Callers use this for
/// history images so a crash cannot leave a truncated PNG referenced by the
/// manifest.
pub fn copy_atomic(source: &Path, path: &Path) -> std::io::Result<()> {
    let bytes = fs::read(source)?;
    write_atomic(path, &bytes)
}

fn write_and_replace(
    mut file: fs::File,
    temp: &Path,
    path: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::{copy_atomic, write_atomic};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "capin-persist-{name}-{}-{}",
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
    fn writes_and_replaces_existing_contents() {
        let directory = temp_dir("replace");
        let path = directory.join("state.json");
        write_atomic(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn copy_atomic_writes_the_source_bytes() {
        let directory = temp_dir("copy");
        let source = directory.join("source.png");
        let target = directory.join("target.png");
        std::fs::write(&source, b"png-bytes").unwrap();
        copy_atomic(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"png-bytes");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_replace_keeps_the_existing_file() {
        let directory = temp_dir("failure");
        let path = directory.join("state.json");
        std::fs::write(&path, b"old").unwrap();
        // A directory cannot be replaced by a regular file.
        let blocker = directory.join("blocker");
        std::fs::create_dir_all(&blocker).unwrap();
        assert!(write_atomic(&blocker, b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(directory);
    }
}
