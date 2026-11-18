use core_types::RgbaFrame;

use crate::{CaptureError, CaptureTarget, MonitorInfo, WindowInfo};

pub trait CaptureBackend {
    fn monitors(&mut self) -> Result<Vec<MonitorInfo>, CaptureError>;

    fn windows(&mut self) -> Result<Vec<WindowInfo>, CaptureError>;

    fn windows_excluding(
        &mut self,
        excluded: &[core_types::WindowId],
    ) -> Result<Vec<WindowInfo>, CaptureError> {
        let mut windows = self.windows()?;
        windows.retain(|window| !excluded.contains(&window.id));
        Ok(windows)
    }

    fn capture(&mut self, target: CaptureTarget) -> Result<RgbaFrame, CaptureError>;

    fn capture_with_cursor(&mut self, target: CaptureTarget) -> Result<RgbaFrame, CaptureError> {
        self.capture(target)
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
