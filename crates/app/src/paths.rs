use std::path::PathBuf;

const APP_FOLDER: &str = "Capin";

pub(crate) fn app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    });
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"));
    base.map(|base| base.join(APP_FOLDER))
}

pub(crate) fn config_path() -> Option<PathBuf> {
    app_data_dir().map(|directory| directory.join("settings.conf"))
}

pub(crate) fn pins_dir() -> Option<PathBuf> {
    let directory = app_data_dir()?.join("pins");
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

pub(crate) fn history_dir() -> Option<PathBuf> {
    let directory = app_data_dir()?.join("history");
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

pub(crate) fn logs_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(APP_FOLDER).join("logs")
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Logs")
            .join(APP_FOLDER)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".local")
            .join("state")
            .join(APP_FOLDER)
            .join("logs")
    }
}
