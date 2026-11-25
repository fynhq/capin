use core_capture::CaptureBackend;
use core_types::{PhysicalPoint, PhysicalRect, WindowId};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use core_capture::{CaptureError, MonitorInfo, WindowInfo};
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use core_types::CaptureTarget;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub mod macos_sck_support;
#[cfg(target_os = "windows")]
mod windows;

pub fn platform_backend() -> Box<dyn CaptureBackend> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsCapture::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosCapture)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Box::new(UnsupportedCapture)
    }
}

pub fn ui_elements(id: WindowId) -> Vec<PhysicalRect> {
    #[cfg(target_os = "windows")]
    {
        windows::ui_elements(id)
    }
    #[cfg(target_os = "macos")]
    {
        macos::ui_elements(id)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = id;
        Vec::new()
    }
}

pub fn ui_elements_at(id: WindowId, point: PhysicalPoint) -> Vec<PhysicalRect> {
    #[cfg(target_os = "windows")]
    {
        let _ = point;
        windows::ui_elements(id)
    }
    #[cfg(target_os = "macos")]
    {
        macos::ui_elements_at(id, point)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (id, point);
        Vec::new()
    }
}

pub fn window_corner_radius(id: WindowId, scale: f32) -> u32 {
    #[cfg(target_os = "windows")]
    {
        windows::window_corner_radius(id, scale)
    }
    #[cfg(target_os = "macos")]
    {
        macos::window_corner_radius(id, scale)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (id, scale);
        0
    }
}

pub fn capture_excluded_window_ids() -> Vec<u32> {
    #[cfg(target_os = "macos")]
    {
        macos::capture_excluded_window_ids()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
pub use macos::display_backing_scale;
#[cfg(target_os = "windows")]
pub use windows::{
    MonitorStream, cache_monitor_stream, init_process_dpi_awareness, seed_monitor_stream,
    take_monitor_stream,
};

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

    fn capture(&mut self, _target: CaptureTarget) -> Result<core_types::RgbaFrame, CaptureError> {
        Err(CaptureError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn unsupported_backend_reports_its_platform() {
        let mut backend = platform_backend();
        assert!(matches!(
            backend.monitors(),
            Err(CaptureError::UnsupportedPlatform)
        ));
    }
}
