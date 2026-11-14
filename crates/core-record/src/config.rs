use core_types::{CaptureTarget, PhysicalSize};
use thiserror::Error;

/// Fixed output frame rate for every recording.
pub const FRAME_RATE: u32 = 30;

/// Encoder bitrate for screen content, in bits per second.
///
/// Screen recordings spend bits on sharp text and flat UI rather than camera
/// motion, so one bit per eight pixels per frame keeps 1080p30 near 7.8 Mbps
/// instead of the 15.5 Mbps of a one-bit-per-four budget.
pub fn video_bitrate(width: u32, height: u32) -> u32 {
    let bits = u64::from(width) * u64::from(height) * u64::from(FRAME_RATE) / 8;
    bits.clamp(2_000_000, 24_000_000) as u32
}

/// The translucent disc painted under the system cursor during a recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorHighlight {
    /// Disc color as packed `0xRRGGBB`. Opacity is fixed by the recorder.
    pub color: u32,
}

/// Inputs for one recording session. Audio and format choice are intentionally
/// absent: recordings are always MP4 without sound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordConfig {
    pub target: CaptureTarget,
    pub include_cursor: bool,
    pub cursor_highlight: Option<CursorHighlight>,
}

impl RecordConfig {
    pub fn new(target: CaptureTarget) -> Self {
        Self {
            target,
            include_cursor: true,
            cursor_highlight: None,
        }
    }

    /// Validates the target before a recorder starts.
    pub fn validate(&self, dimensions: PhysicalSize) -> Result<(), RecordError> {
        if dimensions.is_empty() {
            return Err(RecordError::EmptyDimensions);
        }
        if let CaptureTarget::Region(region) = self.target
            && region.is_empty()
        {
            return Err(RecordError::EmptyTarget);
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RecordError {
    #[error("recording dimensions must be non-zero")]
    EmptyDimensions,
    #[error("recording target is empty")]
    EmptyTarget,
}

#[cfg(test)]
mod tests {
    use core_types::{CaptureRegion, MonitorId, PhysicalRect, PhysicalSize};

    use super::*;

    fn monitor_target() -> CaptureTarget {
        CaptureTarget::Monitor(MonitorId::from_raw(1))
    }

    #[test]
    fn accepts_a_monitor_canvas() {
        let config = RecordConfig::new(monitor_target());
        assert_eq!(config.validate(PhysicalSize::new(3840, 2160)), Ok(()));
    }

    #[test]
    fn rejects_empty_dimensions() {
        let config = RecordConfig::new(monitor_target());
        assert_eq!(
            config.validate(PhysicalSize::new(0, 1080)),
            Err(RecordError::EmptyDimensions)
        );
    }

    #[test]
    fn rejects_empty_region_target() {
        let target = CaptureTarget::Region(CaptureRegion::new(
            MonitorId::from_raw(1),
            PhysicalRect::new(0, 0, 0, 0),
        ));
        let config = RecordConfig::new(target);
        assert_eq!(
            config.validate(PhysicalSize::new(1920, 1080)),
            Err(RecordError::EmptyTarget)
        );
    }

    #[test]
    fn bitrate_halves_the_previous_screen_budget() {
        assert_eq!(super::video_bitrate(1920, 1080), 7_776_000);
        assert_eq!(super::video_bitrate(1280, 720), 3_456_000);
        assert_eq!(super::video_bitrate(3840, 2160), 24_000_000);
        assert_eq!(super::video_bitrate(160, 120), 2_000_000);
    }
}
