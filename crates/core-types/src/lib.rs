//! Shared value types used across capture, annotation, recording, and UI code.

mod color;
mod dpi;
mod frame;
mod geometry;
mod ids;
mod record;

pub use color::{abgr_to_rgb, prefers_dark_foreground, relative_luminance};
pub use dpi::DpiScale;
pub use frame::{BYTES_PER_PIXEL, FrameError, RgbaFrame};
pub use geometry::{CaptureRegion, PhysicalPoint, PhysicalRect, PhysicalSize};
pub use ids::{MonitorId, WindowId};
pub use record::RecordFormat;
