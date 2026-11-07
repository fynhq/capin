use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformWindowError {
    #[error("window handle is unavailable")]
    Handle(#[from] raw_window_handle::HandleError),
    #[error("window styling is not supported on this platform")]
    UnsupportedPlatform,
    #[error("window is not a Win32 window")]
    NotWin32,
    #[error("window is not an AppKit window")]
    NotAppKit,
    #[error("{operation} failed: {message}")]
    Platform {
        operation: &'static str,
        message: String,
    },
}
