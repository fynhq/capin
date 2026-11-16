#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowStyle {
    pub always_on_top: bool,
    pub skip_taskbar: bool,
    pub no_activate: bool,
    pub exclude_from_capture: bool,
    pub borderless: bool,
    pub opaque: bool,
    pub keeps_visible_when_inactive: bool,
}

impl WindowStyle {
    pub const EMPTY: Self = Self {
        always_on_top: false,
        skip_taskbar: false,
        no_activate: false,
        exclude_from_capture: false,
        borderless: false,
        opaque: false,
        keeps_visible_when_inactive: false,
    };

    pub const OVERLAY: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: true,
        borderless: true,
        opaque: true,
        keeps_visible_when_inactive: true,
    };

    pub const PIN: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: false,
        borderless: true,
        opaque: false,
        keeps_visible_when_inactive: true,
    };

    pub const PIN_MENU: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: true,
        borderless: false,
        opaque: false,
        keeps_visible_when_inactive: false,
    };

    pub const CAPTURE_BAR: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: true,
        exclude_from_capture: true,
        borderless: false,
        opaque: false,
        keeps_visible_when_inactive: true,
    };

    pub const CAPTURE_OUTLINE: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: true,
        exclude_from_capture: true,
        borderless: true,
        opaque: false,
        keeps_visible_when_inactive: true,
    };

    pub const RECORD_HIGHLIGHT: Self = Self::CAPTURE_OUTLINE;

    pub const INLINE_EDITOR: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: true,
        borderless: true,
        opaque: false,
        keeps_visible_when_inactive: true,
    };
}
