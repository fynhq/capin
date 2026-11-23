use std::time::Duration;

pub(super) struct PinToast {
    pub(super) message: Option<String>,
    pub(super) leaving: bool,
    pub(super) serial: u64,
    pub(super) linger: Duration,
}

impl PinToast {
    pub(super) fn new(linger: Duration) -> Self {
        Self {
            message: None,
            leaving: false,
            serial: 0,
            linger,
        }
    }
}
