mod backend;
mod error;
mod info;
mod selection;

pub use backend::CaptureBackend;
pub use core_types::CaptureTarget;
pub use error::{CaptureError, Result};
pub use info::{MonitorInfo, WindowInfo};
pub use selection::{
    DRAG_THRESHOLD_PX, SelectionAction, SelectionOutcome, SelectionSession, element_candidates,
    region_from_drag,
};
