use core_types::RgbaFrame;

use crate::{CaptureError, CaptureTarget, MonitorInfo, WindowInfo};

/// One-shot capture backend. Recording uses the same frame type and builds its
/// own streaming pipeline on top of the platform capture APIs.
pub trait CaptureBackend {
    fn monitors(&mut self) -> Result<Vec<MonitorInfo>, CaptureError>;

    /// Returns top-level windows in top-to-bottom z-order.
    ///
    /// Implementations include other windows owned by this process (pins and
    /// settings are valid capture targets) and the taskbar, but must omit
    /// invisible windows, cloaked windows, and the desktop shell so callers
    /// can use the returned order for hover hit-testing. Callers that show
    /// their own full-screen helper windows pass them to
    /// [`Self::windows_excluding`].
    fn windows(&mut self) -> Result<Vec<WindowInfo>, CaptureError>;

    /// Same as [`Self::windows`], without the given windows. Capture overlays
    /// use this to hide themselves without hiding pins or the settings page.
    fn windows_excluding(
        &mut self,
        excluded: &[core_types::WindowId],
    ) -> Result<Vec<WindowInfo>, CaptureError> {
        let mut windows = self.windows()?;
        windows.retain(|window| !excluded.contains(&window.id));
        Ok(windows)
    }

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
