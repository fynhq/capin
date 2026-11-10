use core_compose::{Background, WindowCanvas, compose_window};
use core_types::RgbaFrame;

use crate::logging::log_error;

/// Capture-level decoration applied to region and monitor captures.
///
/// Only the corner radius survives: padding, synthetic shadow, and borders
/// added a visible canvas around the captured pixels, while the editor and
/// pin should show the original image at its original size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RegionEffects {
    /// Corner radius in logical pixels.
    pub corner_radius: u32,
}

impl RegionEffects {
    pub fn active(self) -> bool {
        self.corner_radius > 0
    }

    /// Applies the effects, returning the original frame unchanged when they
    /// are disabled. Taking the frame by value avoids a full-image clone on
    /// the common path where no region effect is configured.
    pub fn apply(self, frame: RgbaFrame, scale: f32) -> RgbaFrame {
        if !self.active() || frame.width() == 0 || frame.height() == 0 {
            return frame;
        }

        let scale = scale.max(0.1);
        let canvas = WindowCanvas {
            padding: 0,
            corner_radius: (self.corner_radius as f32 * scale).round() as u32,
            shadow: None,
            background: Background::Transparent,
            border: None,
        };

        match compose_window(&frame, &canvas) {
            Ok(frame) => frame,
            Err(error) => {
                log_error!("effects", "failed to apply capture effects: {error}");
                frame
            }
        }
    }
}
