use core_capture::CaptureTarget;
use core_types::{PhysicalSize, RecordFormat};
use thiserror::Error;

use crate::FrameRate;

const WEBP_MAX_LONG_EDGE: u32 = 1920;
const WEBP_MAX_SHORT_EDGE: u32 = 1080;

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

        if self.format == RecordFormat::WebP && self.frame_rate != FrameRate::Fps30 {
            return Err(RecordError::WebpFrameRate(self.frame_rate));
        }

        Ok(())
    }
}

/// Output canvas for animated WebP recordings.
///
/// WebP is encoded in software, so large displays are scaled down to keep
/// real-time 30 fps capture viable. Aspect ratio is preserved.
pub fn webp_output_size(input: PhysicalSize) -> PhysicalSize {
    if input.is_empty() {
        return input;
    }
    let long_edge = input.width.max(input.height) as f32;
    let short_edge = input.width.min(input.height) as f32;
    let scale = (WEBP_MAX_LONG_EDGE as f32 / long_edge)
        .min(WEBP_MAX_SHORT_EDGE as f32 / short_edge)
        .min(1.0);
    if scale >= 1.0 {
        return input;
    }
    PhysicalSize::new(
        ((input.width as f32 * scale).round() as u32).max(2),
        ((input.height as f32 * scale).round() as u32).max(2),
    )
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RecordError {
    #[error("recording dimensions must be non-zero")]
    EmptyDimensions,
    #[error("recording target is empty")]
    EmptyTarget,
    #[error("WebP recording supports 30 fps, got {0}")]
    WebpFrameRate(FrameRate),
    #[error("WebP encoding failed: {0}")]
    Webp(String),
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
    fn webp_accepts_1080p30() {
        let config = RecordConfig::new(monitor_target(), RecordFormat::WebP);
        assert_eq!(config.validate(PhysicalSize::new(1920, 1080)), Ok(()));
    }

    #[test]
    fn webp_accepts_portrait_1080p() {
        let config = RecordConfig::new(monitor_target(), RecordFormat::WebP);
        assert_eq!(config.validate(PhysicalSize::new(1080, 1920)), Ok(()));
    }

    #[test]
    fn webp_rejects_60_fps() {
        let mut config = RecordConfig::new(monitor_target(), RecordFormat::WebP);
        config.frame_rate = FrameRate::Fps60;
        assert_eq!(
            config.validate(PhysicalSize::new(1920, 1080)),
            Err(RecordError::WebpFrameRate(FrameRate::Fps60))
        );
    }

    #[test]
    fn webp_scales_large_canvases_to_1080p() {
        assert_eq!(
            webp_output_size(PhysicalSize::new(2560, 1600)),
            PhysicalSize::new(1728, 1080)
        );
        assert_eq!(
            webp_output_size(PhysicalSize::new(3840, 2160)),
            PhysicalSize::new(1920, 1080)
        );
        assert_eq!(
            webp_output_size(PhysicalSize::new(1000, 2000)),
            PhysicalSize::new(960, 1920)
        );
    }

    #[test]
    fn webp_keeps_canvases_within_the_limit() {
        assert_eq!(
            webp_output_size(PhysicalSize::new(1920, 1080)),
            PhysicalSize::new(1920, 1080)
        );
        assert_eq!(
            webp_output_size(PhysicalSize::new(1080, 1920)),
            PhysicalSize::new(1080, 1920)
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
