use core_types::{CaptureRegion, MonitorId, WindowId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureTarget {
    Monitor(MonitorId),
    Window(WindowId),
    /// A region in the monitor's local physical pixel coordinates.
    Region(CaptureRegion),
}
