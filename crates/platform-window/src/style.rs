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
    /// Remove the native title bar and its traffic lights. Used by the
    /// full-screen capture overlay, which draws its own chrome.
    pub borderless: bool,
    /// Paint an opaque native surface. The capture overlay covers the whole
    /// screen with a frozen frame, so a transparent AppKit layer only invites
    /// compositor artifacts.
    pub opaque: bool,
}

impl WindowStyle {
    pub const EMPTY: Self = Self {
        always_on_top: false,
        skip_taskbar: false,
        no_activate: false,
        exclude_from_capture: false,
        borderless: false,
        opaque: false,
    };

    pub const OVERLAY: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: true,
        borderless: true,
        opaque: true,
    };

    pub const PIN: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        // Pins must accept keyboard focus so the system copy shortcut works on
        // a selected text run; a nonactivating window never receives it.
        no_activate: false,
        exclude_from_capture: true,
        borderless: false,
        opaque: false,
    };

    pub const CAPTURE_BAR: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: true,
        exclude_from_capture: true,
        borderless: false,
        opaque: false,
    };

    /// Full-screen inline annotation surface used for pin editing and long
    /// captures. It must stay focusable for keyboard input, be borderless and
    /// transparent so the image paints over the live desktop, and never
    /// appear in another capture.
    pub const INLINE_EDITOR: Self = Self {
        always_on_top: true,
        skip_taskbar: true,
        no_activate: false,
        exclude_from_capture: true,
        borderless: true,
        opaque: false,
    };
}
