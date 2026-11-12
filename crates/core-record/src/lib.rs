//! Recording configuration and frame pacing shared by platform encoders.

mod config;
mod cursor;
mod timing;

pub use config::{CursorHighlight, FRAME_RATE, RecordConfig, RecordError, video_bitrate};
pub use cursor::{CURSOR_HIGHLIGHT_ALPHA, CURSOR_HIGHLIGHT_RADIUS, paint_cursor_highlight_bgra};
pub use timing::{FrameDecision, FrameTimeline};
