use crate::{CaptureRegion, MonitorId, WindowId};

/// A screen-capture target shared by screenshots, long capture, and recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureTarget {
    Monitor(MonitorId),
    Window(WindowId),
    /// A region in the monitor's local physical pixel coordinates.
    Region(CaptureRegion),
}
