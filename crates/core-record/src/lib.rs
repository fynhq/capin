//! Recording configuration and frame pacing shared by platform encoders.

mod config;
mod timing;

pub use config::{RecordConfig, RecordError};
pub use timing::{FrameDecision, FrameRate, FrameTimeline};
