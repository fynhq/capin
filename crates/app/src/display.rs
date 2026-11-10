//! Converts virtual-desktop bounds into the coordinate space GPUI expects for
//! a window assigned to a specific monitor.
//!
//! Windows places windows in virtual-desktop logical coordinates, where each
//! monitor's origin is its physical origin divided by that monitor's scale.
//! macOS places them relative to the target display's top-left corner. The
//! same `WindowOptions` value therefore means different things on the two
//! platforms once a window is not on the primary display.

use core_capture::MonitorInfo;
use gpui_kit::{Bounds, Pixels, point, px, size};

/// Returns `bounds` in the space used by `WindowOptions.window_bounds` for a
/// window that belongs to `monitor`.
///
/// `bounds` must be in the virtual-desktop logical space built from physical
/// pixels divided by the source monitor's scale.
pub fn window_bounds_on_monitor(monitor: &MonitorInfo, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    #[cfg(target_os = "macos")]
    {
        let scale = monitor.scale.get().max(0.1);
        Bounds {
            origin: point(
                bounds.origin.x - px(monitor.bounds.origin.x as f32 / scale),
                bounds.origin.y - px(monitor.bounds.origin.y as f32 / scale),
            ),
            size: bounds.size,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = monitor;
        bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{DpiScale, MonitorId, PhysicalRect};

    fn monitor(x: i32, y: i32, width: u32, height: u32, scale: f32) -> MonitorInfo {
        MonitorInfo {
            id: MonitorId::from_raw(1),
            bounds: PhysicalRect::new(x, y, width, height),
            scale: DpiScale::new(scale).unwrap(),
            is_primary: false,
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn windows_keeps_virtual_desktop_bounds() {
        let monitor = monitor(1920, 0, 1920, 1080, 1.0);
        let bounds = Bounds {
            origin: point(px(1920.0), px(0.0)),
            size: size(px(1920.0), px(1080.0)),
        };
        assert_eq!(window_bounds_on_monitor(&monitor, bounds), bounds);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_moves_bounds_to_the_display_origin() {
        let monitor = monitor(2880, 0, 2880, 1800, 2.0);
        let bounds = Bounds {
            origin: point(px(1540.0), px(100.0)),
            size: size(px(160.0), px(80.0)),
        };
        let local = window_bounds_on_monitor(&monitor, bounds);
        assert_eq!(local.origin, point(px(100.0), px(100.0)));
        assert_eq!(local.size, bounds.size);
    }
}
