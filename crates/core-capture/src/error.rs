use core_types::{FrameError, MonitorId, WindowId};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, CaptureError>;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("screen capture is not supported on this platform")]
    UnsupportedPlatform,
    #[error("screen recording permission was denied")]
    PermissionDenied,
    #[error("monitor {0:?} is no longer available")]
    MonitorNotFound(MonitorId),
    #[error("window {0:?} is no longer available or cannot be captured")]
    WindowNotFound(WindowId),
    #[error("capture region is empty")]
    EmptyRegion,
    #[error("capture region is outside the monitor bounds")]
    RegionOutOfBounds,
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("{operation} failed: {message}")]
    Platform {
        operation: &'static str,
        message: String,
    },
}
