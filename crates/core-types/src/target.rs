use crate::{CaptureRegion, MonitorId, WindowId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureTarget {
    Monitor(MonitorId),
    Window(WindowId),
    Region(CaptureRegion),
}
