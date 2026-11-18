use core_capture::MonitorInfo;
use gpui_kit::{Bounds, Pixels};
#[cfg(target_os = "macos")]
use gpui_kit::{point, px};

pub(crate) struct LogicalMonitor {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub scale: f32,
}

pub(crate) fn logical_monitor(monitor: Option<&MonitorInfo>) -> LogicalMonitor {
    let Some(monitor) = monitor else {
        return LogicalMonitor {
            left: 0.0,
            top: 0.0,
            right: 1920.0,
            bottom: 1080.0,
            scale: 1.0,
        };
    };
    let scale = monitor.scale.get().max(0.1);
    LogicalMonitor {
        left: monitor.bounds.origin.x as f32 / scale,
        top: monitor.bounds.origin.y as f32 / scale,
        right: (monitor.bounds.origin.x + monitor.bounds.size.width as i32) as f32 / scale,
        bottom: (monitor.bounds.origin.y + monitor.bounds.size.height as i32) as f32 / scale,
        scale,
    }
}

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
    use gpui_kit::size;
    #[cfg(not(target_os = "macos"))]
    use gpui_kit::{point, px};

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

    #[test]
    fn logical_monitor_converts_physical_bounds() {
        let monitor = monitor(2880, 0, 2880, 1800, 2.0);
        let span = logical_monitor(Some(&monitor));
        assert_eq!(span.left, 1440.0);
        assert_eq!(span.right, 2880.0);
        assert_eq!(span.bottom, 900.0);
        assert_eq!(span.scale, 2.0);

        let fallback = logical_monitor(None);
        assert_eq!((fallback.left, fallback.top), (0.0, 0.0));
        assert_eq!((fallback.right, fallback.bottom), (1920.0, 1080.0));
    }
}
