//! Shared value types used across capture, annotation, recording, and UI code.

mod color;
mod color_space;
mod dpi;
mod frame;
mod geometry;
mod ids;
mod record;

pub use color::{abgr_to_rgb, prefers_dark_foreground, relative_luminance};
pub use color_space::{
    ColorSpace, convert_frame_to_srgb, convert_rgb_to_srgb, linear_p3_to_srgb, linear_to_srgb,
    srgb_to_linear, tone_map_reinhard,
};
pub use dpi::DpiScale;
pub use frame::{BYTES_PER_PIXEL, FrameError, RgbaFrame};
pub use geometry::{CaptureRegion, PhysicalPoint, PhysicalRect, PhysicalSize};
pub use ids::{MonitorId, WindowId};
pub use record::RecordFormat;
