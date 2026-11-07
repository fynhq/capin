use core_types::RgbaFrame;

use crate::{CaptureError, CaptureTarget, MonitorInfo, WindowInfo};

/// One-shot capture backend. Recording uses the same frame type and builds its
/// own streaming pipeline on top of the platform capture APIs.
pub trait CaptureBackend {
    fn monitors(&mut self) -> Result<Vec<MonitorInfo>, CaptureError>;

    /// Returns top-level windows in top-to-bottom z-order.
    ///
    /// Implementations must omit their own overlay/helper windows, invisible
    /// windows, DWM-cloaked windows, and platform shell surfaces (desktop,
    /// taskbar) so callers can use the returned order for hover hit-testing.
    fn windows(&mut self) -> Result<Vec<WindowInfo>, CaptureError>;

    fn capture(&mut self, target: CaptureTarget) -> Result<RgbaFrame, CaptureError>;

    /// Captures with the system cursor when the platform supports it. The
    /// default falls back to a cursor-free frame.
    fn capture_with_cursor(&mut self, target: CaptureTarget) -> Result<RgbaFrame, CaptureError> {
        self.capture(target)
    }
}

/// Returns the capture backend for the current platform.
///
/// Platforms without an implementation return [`CaptureError::UnsupportedPlatform`]
/// from every operation instead of failing at startup.
pub fn platform_backend() -> Box<dyn CaptureBackend> {
    #[cfg(target_os = "windows")]
    {
        Box::new(crate::WindowsCapture::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::MacosCapture)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Box::new(UnsupportedCapture)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
struct UnsupportedCapture;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl CaptureBackend for UnsupportedCapture {
    fn monitors(&mut self) -> Result<Vec<MonitorInfo>, CaptureError> {
        Err(CaptureError::UnsupportedPlatform)
    }

    fn windows(&mut self) -> Result<Vec<WindowInfo>, CaptureError> {
        Err(CaptureError::UnsupportedPlatform)
    }

    fn capture(&mut self, _target: CaptureTarget) -> Result<RgbaFrame, CaptureError> {
        Err(CaptureError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub;

    impl CaptureBackend for Stub {
        fn monitors(&mut self) -> Result<Vec<MonitorInfo>, CaptureError> {
            Ok(Vec::new())
        }

        fn windows(&mut self) -> Result<Vec<WindowInfo>, CaptureError> {
            Ok(Vec::new())
        }

        fn capture(&mut self, _target: CaptureTarget) -> Result<RgbaFrame, CaptureError> {
            Err(CaptureError::UnsupportedPlatform)
        }
    }

    #[test]
    fn backend_is_object_safe() {
        let mut backend = Stub;
        let erased: &mut dyn CaptureBackend = &mut backend;
        assert!(erased.monitors().unwrap().is_empty());
    }
}
