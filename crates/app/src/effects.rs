use core_compose::{Background, Border, Shadow, WindowCanvas, compose_window};
use core_types::RgbaFrame;

use crate::logging::log_error;

/// Capture-level decoration applied to region and monitor captures.
///
/// Window captures already run through the window composition pipeline, so
/// these effects are skipped for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RegionEffects {
    /// Corner radius in logical pixels.
    pub corner_radius: u32,
    pub shadow: bool,
    pub border: bool,
}

impl RegionEffects {
    pub fn active(self) -> bool {
        self.corner_radius > 0 || self.shadow || self.border
    }

    pub fn apply(self, frame: &RgbaFrame, scale: f32) -> RgbaFrame {
        if !self.active() || frame.width() == 0 || frame.height() == 0 {
            return frame.clone();
        }

        let scale = scale.max(0.1);
        let shadow = self.shadow.then_some(Shadow {
            offset_y: (8.0 * scale).round() as i32,
            blur_radius: (20.0 * scale).round().max(1.0) as u32,
            opacity: 0.35,
        });
        let padding = if self.shadow {
            ((20.0 + 8.0 + 4.0) * scale).round().max(1.0) as u32
        } else {
            0
        };
        let canvas = WindowCanvas {
            padding,
            corner_radius: (self.corner_radius as f32 * scale).round() as u32,
            shadow,
            background: Background::Transparent,
            border: self.border.then_some(Border {
                width: (1.0 * scale).round().max(1.0) as u32,
                color: [0xFF, 0xFF, 0xFF, 0xCC],
            }),
        };

        match compose_window(frame, &canvas) {
            Ok(frame) => frame,
            Err(error) => {
                log_error!("effects", "failed to apply capture effects: {error}");
                frame.clone()
            }
        }
    }
}
