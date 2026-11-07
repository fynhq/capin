use core_capture::CaptureTarget;
use core_types::{PhysicalSize, RecordFormat};
use thiserror::Error;

use crate::FrameRate;

const WEBM_MAX_LONG_EDGE: u32 = 1920;
const WEBM_MAX_SHORT_EDGE: u32 = 1080;

/// Inputs for one recording session. Audio is intentionally absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordConfig {
    pub target: CaptureTarget,
    pub format: RecordFormat,
    pub frame_rate: FrameRate,
    pub include_cursor: bool,
}

impl RecordConfig {
    pub fn new(target: CaptureTarget, format: RecordFormat) -> Self {
        Self {
            target,
            format,
            frame_rate: FrameRate::Fps30,
            include_cursor: true,
        }
    }

    /// Validates the format-specific limits before a recorder starts.
    pub fn validate(&self, dimensions: PhysicalSize) -> Result<(), RecordError> {
        if dimensions.is_empty() {
            return Err(RecordError::EmptyDimensions);
        }
        if let CaptureTarget::Region(region) = self.target
            && region.is_empty()
        {
            return Err(RecordError::EmptyTarget);
        }

        if self.format == RecordFormat::WebM {
            if self.frame_rate != FrameRate::Fps30 {
                return Err(RecordError::WebmFrameRate(self.frame_rate));
            }
            let long_edge = dimensions.width.max(dimensions.height);
            let short_edge = dimensions.width.min(dimensions.height);
            if long_edge > WEBM_MAX_LONG_EDGE || short_edge > WEBM_MAX_SHORT_EDGE {
                return Err(RecordError::WebmResolution {
                    width: dimensions.width,
                    height: dimensions.height,
                });
            }
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
    #[error("WebM recording supports 30 fps, got {0}")]
    WebmFrameRate(FrameRate),
    #[error("WebM recording is limited to 1080p, got {width}x{height}")]
    WebmResolution { width: u32, height: u32 },
}

#[cfg(test)]
mod tests {
    use core_capture::CaptureTarget;
    use core_types::{CaptureRegion, MonitorId, PhysicalRect, PhysicalSize, RecordFormat};

    use super::*;

    fn monitor_target() -> CaptureTarget {
        CaptureTarget::Monitor(MonitorId::from_raw(1))
    }

    #[test]
    fn webm_accepts_1080p30() {
        let config = RecordConfig::new(monitor_target(), RecordFormat::WebM);
        assert_eq!(config.validate(PhysicalSize::new(1920, 1080)), Ok(()));
    }

    #[test]
    fn webm_accepts_portrait_1080p() {
        let config = RecordConfig::new(monitor_target(), RecordFormat::WebM);
        assert_eq!(config.validate(PhysicalSize::new(1080, 1920)), Ok(()));
    }

    #[test]
    fn webm_rejects_60_fps() {
        let mut config = RecordConfig::new(monitor_target(), RecordFormat::WebM);
        config.frame_rate = FrameRate::Fps60;
        assert_eq!(
            config.validate(PhysicalSize::new(1920, 1080)),
            Err(RecordError::WebmFrameRate(FrameRate::Fps60))
        );
    }

    #[test]
    fn webm_rejects_1440p() {
        let config = RecordConfig::new(monitor_target(), RecordFormat::WebM);
        assert_eq!(
            config.validate(PhysicalSize::new(2560, 1440)),
            Err(RecordError::WebmResolution {
                width: 2560,
                height: 1440
            })
        );
    }

    #[test]
    fn mp4_accepts_4k60() {
        let mut config = RecordConfig::new(monitor_target(), RecordFormat::Mp4);
        config.frame_rate = FrameRate::Fps60;
        assert_eq!(config.validate(PhysicalSize::new(3840, 2160)), Ok(()));
    }

    #[test]
    fn rejects_empty_dimensions() {
        let config = RecordConfig::new(monitor_target(), RecordFormat::Mp4);
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
        let config = RecordConfig::new(target, RecordFormat::Mp4);
        assert_eq!(
            config.validate(PhysicalSize::new(1920, 1080)),
            Err(RecordError::EmptyTarget)
        );
    }
}
