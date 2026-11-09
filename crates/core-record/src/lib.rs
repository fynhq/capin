//! Recording configuration and frame pacing shared by platform encoders.

mod config;
mod cursor;
mod timing;
mod webp;

pub use config::{CursorHighlight, RecordConfig, RecordError, RecordQuality, webp_output_size};
pub use cursor::{CURSOR_HIGHLIGHT_ALPHA, CURSOR_HIGHLIGHT_RADIUS, paint_cursor_highlight_bgra};
pub use timing::{FrameDecision, FrameRate, FrameTimeline};
pub use webp::WebpEncoder;
