use std::fmt;

use core_types::{DpiScale, MonitorId, PhysicalRect, WindowId};

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub id: MonitorId,
    /// Physical bounds in the virtual desktop coordinate space.
    pub bounds: PhysicalRect,
    pub scale: DpiScale,
    pub is_primary: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: WindowId,
    /// Physical bounds including any DWM or AppKit shadow extents where
    /// the platform can provide them.
    pub bounds: PhysicalRect,
    pub title: String,
}

impl fmt::Debug for WindowInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowInfo")
            .field("id", &self.id)
            .field("bounds", &self.bounds)
            .field("title", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_debug_output_redacts_the_title() {
        let window = WindowInfo {
            id: WindowId::from_raw(3),
            bounds: PhysicalRect::new(0, 0, 100, 100),
            title: "private document.txt".to_owned(),
        };

        let text = format!("{window:?}");
        assert!(!text.contains("private document.txt"));
        assert!(text.contains("<redacted>"));
    }
}
