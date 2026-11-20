use core_compose::{Background, Shadow, WindowCanvas, compose_window};
use core_types::RgbaFrame;

use crate::logging::log_error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RegionEffects {
    pub corner_radius: u32,
    pub shadow: bool,
}

impl RegionEffects {
    pub fn active(self) -> bool {
        self.corner_radius > 0 || self.shadow
    }

    pub fn with_shadow(mut self) -> Self {
        self.shadow = true;
        self
    }

    pub fn padding(self, scale: f32) -> u32 {
        if self.shadow {
            ((20.0 + 8.0 + 4.0) * scale.max(0.1)).round().max(1.0) as u32
        } else {
            0
        }
    }

    pub fn apply(self, frame: RgbaFrame, scale: f32) -> RgbaFrame {
        if !self.active() || frame.width() == 0 || frame.height() == 0 {
            return frame;
        }

        let scale = scale.max(0.1);
        let shadow = self.shadow.then_some(Shadow {
            offset_y: (8.0 * scale).round() as i32,
            blur_radius: (20.0 * scale).round().max(1.0) as u32,
            opacity: 0.35,
        });
        let padding = self.padding(scale);
        let canvas = WindowCanvas {
            padding,
            corner_radius: (self.corner_radius as f32 * scale).round() as u32,
            shadow,
            background: Background::Transparent,
            border: None,
            edge_bleed: false,
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
