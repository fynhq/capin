/// Identifies a monitor within the current process session.
///
/// The value is derived from the platform handle and is not stable across
/// reboots or display topology changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonitorId(u64);

impl MonitorId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Identifies a window within the current process session.
///
/// The value is derived from the platform handle and may be reused by the
/// operating system after the window is destroyed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_raw_values() {
        assert_eq!(MonitorId::from_raw(7).raw(), 7);
        assert_eq!(WindowId::from_raw(42).raw(), 42);
    }
}
