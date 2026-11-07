/// Native behavior applied to an existing window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowStyle {
    /// Keep the window above normal windows.
    pub always_on_top: bool,
    /// Keep the window out of the taskbar or window switcher.
    pub skip_taskbar: bool,
    /// Avoid activating the window when the user interacts with it.
    pub no_activate: bool,
    /// Exclude the window from screen capture where the OS supports it.
    pub exclude_from_capture: bool,
}

impl WindowStyle {
    pub const EMPTY: Self = Self {
        always_on_top: false,
        skip_taskbar: false,
        no_activate: false,
        exclude_from_capture: false,
    };

    pub const OVERLAY: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: false,
    };

    pub const PIN: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: true,
        exclude_from_capture: true,
    };

    pub const CAPTURE_BAR: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: true,
        exclude_from_capture: true,
    };
}
