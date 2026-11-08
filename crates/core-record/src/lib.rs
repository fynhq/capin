//! Recording configuration and frame pacing shared by platform encoders.

mod config;
mod timing;
mod webp;

pub use config::{RecordConfig, RecordError, webp_output_size};
pub use timing::{FrameDecision, FrameRate, FrameTimeline};
pub use webp::WebpEncoder;
