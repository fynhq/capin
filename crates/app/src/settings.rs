use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::input::{Input, InputEditorStyle, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{AutostartState, set_autostart, system_accent_hex};

use crate::i18n::{Language, t};
use crate::icons;
use crate::logging::{log_error, log_info};
use crate::settings_state::{
    self, AppSettings, RepeatPinBehavior, WINDOW_PADDINGS, WindowBackground,
};
use crate::storage;

const PROJECT_URL: &str = "https://github.com/fynhq/capin";
/// Short commit hash injected by dev packaging builds; formal releases leave
/// it unset and show the bare semver version.
const BUILD_ID: Option<&str> = option_env!("CAPIN_BUILD_ID");

thread_local! {
    /// Status handed to the next settings window when a tray or hotkey action
    /// fails before it can open its own surface (for example a denied screen
    /// recording permission), so the click never ends as a silent no-op.
    static PENDING_STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Open settings windows. Capture only freezes the screen, so its close
    /// path must leave these windows alive.
    static SETTINGS_WINDOWS: RefCell<Vec<AnyWindowHandle>> = const { RefCell::new(Vec::new()) };
}

pub fn is_settings_window(handle: &AnyWindowHandle) -> bool {
    SETTINGS_WINDOWS.with(|handles| {
        handles
            .borrow()
            .iter()
            .any(|open| open.window_id() == handle.window_id())
    })
}

pub fn has_open_settings(cx: &mut App) -> bool {
    let handles = SETTINGS_WINDOWS.with(|handles| handles.borrow().clone());
    let mut live = Vec::with_capacity(handles.len());
    for handle in handles {
        if handle.update(cx, |_, _, _| {}).is_ok() {
            live.push(handle);
        }
    }
    let alive = !live.is_empty();
    SETTINGS_WINDOWS.with(|handles| *handles.borrow_mut() = live);
    alive
}

#[cfg(target_os = "windows")]
const FALLBACK_ACCENT_LIGHT: u32 = 0x0067C0;
#[cfg(target_os = "windows")]
const FALLBACK_ACCENT_DARK: u32 = 0x4CC2FF;

#[cfg(target_os = "macos")]
const FALLBACK_ACCENT_LIGHT: u32 = 0x007AFF;
#[cfg(target_os = "macos")]
const FALLBACK_ACCENT_DARK: u32 = 0x0A84FF;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FALLBACK_ACCENT_LIGHT: u32 = 0x0067C0;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FALLBACK_ACCENT_DARK: u32 = 0x4CC2FF;

/// Native window metrics. Windows follows the system Settings layout, macOS
/// follows System Settings, so paddings, corner radii and type sizes match the host.
#[cfg(target_os = "windows")]
mod layout {
    pub const WINDOW_WIDTH: f32 = 940.0;
    pub const WINDOW_HEIGHT: f32 = 700.0;
    pub const MIN_WIDTH: f32 = 760.0;
    pub const MIN_HEIGHT: f32 = 560.0;
    pub const TITLEBAR_HEIGHT: f32 = 34.0;
    pub const CAPTION_BUTTON_WIDTH: f32 = 34.0;
    pub const CAPTION_BUTTON_HEIGHT: f32 = 34.0;
    pub const SIDEBAR_WIDTH: f32 = 180.0;
    pub const SIDEBAR_PAD: f32 = 12.0;
    pub const SIDEBAR_DIVIDER: bool = false;
    pub const NAV_ITEM_HEIGHT: f32 = 36.0;
    pub const NAV_ITEM_RADIUS: f32 = 4.0;
    pub const NAV_ITEM_PAD_X: f32 = 12.0;
    pub const NAV_GAP: f32 = 10.0;
    pub const NAV_ICON: f32 = 16.0;
    pub const NAV_FONT: f32 = 14.0;
    pub const CONTENT_MAX_WIDTH: f32 = 720.0;
    pub const CONTENT_PAD_X: f32 = 32.0;
    pub const CONTENT_PAD_TOP: f32 = 6.0;
    pub const CONTENT_PAD_BOTTOM: f32 = 32.0;
    pub const CONTENT_GAP: f32 = 24.0;
    pub const HEADER_GAP: f32 = 16.0;
    pub const GROUP_GAP: f32 = 28.0;
    pub const GROUP_HEADER_GAP: f32 = 8.0;
    pub const PAGE_TITLE: f32 = 28.0;
    pub const SECTION_TITLE: f32 = 16.0;
    pub const BODY: f32 = 14.0;
    pub const CAPTION: f32 = 12.0;
    pub const CARD_RADIUS: f32 = 4.0;
    pub const ROW_PAD_X: f32 = 16.0;
    pub const ROW_PAD_Y: f32 = 14.0;
    pub const BUTTON_HEIGHT: f32 = 32.0;
    pub const SEGMENT_HEIGHT: f32 = 24.0;
    pub const BUTTON_RADIUS: f32 = 4.0;
    pub const CONTROL_TEXT: f32 = 14.0;
}

#[cfg(target_os = "macos")]
mod layout {
    pub const WINDOW_WIDTH: f32 = 870.0;
    pub const WINDOW_HEIGHT: f32 = 640.0;
    pub const MIN_WIDTH: f32 = 710.0;
    pub const MIN_HEIGHT: f32 = 520.0;
    pub const TITLEBAR_HEIGHT: f32 = 34.0;
    pub const SIDEBAR_WIDTH: f32 = 150.0;
    pub const SIDEBAR_PAD: f32 = 10.0;
    pub const SIDEBAR_DIVIDER: bool = true;
    pub const NAV_ITEM_HEIGHT: f32 = 28.0;
    pub const NAV_ITEM_RADIUS: f32 = 6.0;
    pub const NAV_ITEM_PAD_X: f32 = 8.0;
    pub const NAV_GAP: f32 = 8.0;
    pub const NAV_ICON: f32 = 15.0;
    pub const NAV_FONT: f32 = 13.0;
    pub const CONTENT_MAX_WIDTH: f32 = 680.0;
    pub const CONTENT_PAD_X: f32 = 28.0;
    pub const CONTENT_PAD_TOP: f32 = 6.0;
    pub const CONTENT_PAD_BOTTOM: f32 = 24.0;
    pub const CONTENT_GAP: f32 = 20.0;
    pub const HEADER_GAP: f32 = 14.0;
    pub const GROUP_GAP: f32 = 22.0;
    pub const GROUP_HEADER_GAP: f32 = 6.0;
    pub const PAGE_TITLE: f32 = 22.0;
    pub const SECTION_TITLE: f32 = 15.0;
    pub const BODY: f32 = 13.0;
    pub const CAPTION: f32 = 11.0;
    pub const CARD_RADIUS: f32 = 10.0;
    pub const ROW_PAD_X: f32 = 14.0;
    pub const ROW_PAD_Y: f32 = 12.0;
    pub const BUTTON_HEIGHT: f32 = 28.0;
    pub const SEGMENT_HEIGHT: f32 = 24.0;
    pub const BUTTON_RADIUS: f32 = 6.0;
    pub const CONTROL_TEXT: f32 = 13.0;
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod layout {
    pub const WINDOW_WIDTH: f32 = 870.0;
    pub const WINDOW_HEIGHT: f32 = 640.0;
    pub const MIN_WIDTH: f32 = 710.0;
    pub const MIN_HEIGHT: f32 = 520.0;
    pub const TITLEBAR_HEIGHT: f32 = 0.0;
    pub const SIDEBAR_WIDTH: f32 = 150.0;
    pub const SIDEBAR_PAD: f32 = 10.0;
    pub const SIDEBAR_DIVIDER: bool = true;
    pub const NAV_ITEM_HEIGHT: f32 = 28.0;
    pub const NAV_ITEM_RADIUS: f32 = 6.0;
    pub const NAV_ITEM_PAD_X: f32 = 8.0;
    pub const NAV_GAP: f32 = 8.0;
    pub const NAV_ICON: f32 = 15.0;
    pub const NAV_FONT: f32 = 13.0;
    pub const CONTENT_MAX_WIDTH: f32 = 680.0;
    pub const CONTENT_PAD_X: f32 = 28.0;
    pub const CONTENT_PAD_TOP: f32 = 6.0;
    pub const CONTENT_PAD_BOTTOM: f32 = 24.0;
    pub const CONTENT_GAP: f32 = 20.0;
    pub const HEADER_GAP: f32 = 14.0;
    pub const GROUP_GAP: f32 = 22.0;
    pub const GROUP_HEADER_GAP: f32 = 6.0;
    pub const PAGE_TITLE: f32 = 22.0;
    pub const SECTION_TITLE: f32 = 15.0;
    pub const BODY: f32 = 13.0;
    pub const CAPTION: f32 = 11.0;
    pub const CARD_RADIUS: f32 = 10.0;
    pub const ROW_PAD_X: f32 = 14.0;
    pub const ROW_PAD_Y: f32 = 12.0;
    pub const BUTTON_HEIGHT: f32 = 28.0;
    pub const SEGMENT_HEIGHT: f32 = 24.0;
    pub const BUTTON_RADIUS: f32 = 6.0;
    pub const CONTROL_TEXT: f32 = 13.0;
}

fn custom_titlebar() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsSection {
    General,
    Capture,
    Recording,
    Shortcuts,
}

impl SettingsSection {
    const ALL: [Self; 4] = [
        Self::General,
        Self::Capture,
        Self::Recording,
        Self::Shortcuts,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::General => "settings-nav-general",
            Self::Capture => "settings-nav-capture",
            Self::Recording => "settings-nav-recording",
            Self::Shortcuts => "settings-nav-shortcuts",
        }
    }

    fn title(self, language: Language) -> &'static str {
        t(
            language,
            match self {
                Self::General => "nav.general",
                Self::Capture => "nav.capture",
                Self::Recording => "nav.recording",
                Self::Shortcuts => "nav.shortcuts",
            },
        )
    }

    fn icon(self) -> &'static [u8] {
        match self {
            Self::General => icons::GENERAL,
            Self::Capture => icons::CAPTURE,
            Self::Recording => icons::RECORDING,
            Self::Shortcuts => icons::SHORTCUTS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutSlot {
    Capture,
    Pin,
}

impl ShortcutSlot {
    fn id(self) -> &'static str {
        match self {
            Self::Capture => "settings-shortcut-capture",
            Self::Pin => "settings-shortcut-pin",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Self::Capture => "settings.capture_hotkey",
            Self::Pin => "settings.pin_hotkey",
        }
    }

    fn description_key(self) -> Option<&'static str> {
        match self {
            Self::Capture => None,
            Self::Pin => Some("settings.pin_hotkey_desc"),
        }
    }

    fn value(self, settings: &AppSettings) -> &str {
        match self {
            Self::Capture => &settings.capture_hotkey,
            Self::Pin => &settings.pin_hotkey,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveTarget {
    Screenshots,
    Recordings,
}

impl SaveTarget {
    fn title_key(self) -> &'static str {
        match self {
            Self::Screenshots => "settings.save_location",
            Self::Recordings => "settings.recording_location",
        }
    }

    fn open_id(self) -> &'static str {
        match self {
            Self::Screenshots => "settings-screenshot-folder-open",
            Self::Recordings => "settings-recording-folder-open",
        }
    }

    fn change_id(self) -> &'static str {
        match self {
            Self::Screenshots => "settings-screenshot-folder-change",
            Self::Recordings => "settings-recording-folder-change",
        }
    }

    fn default_id(self) -> &'static str {
        match self {
            Self::Screenshots => "settings-screenshot-folder-default",
            Self::Recordings => "settings-recording-folder-default",
        }
    }

    fn selected_directory(self, settings: &AppSettings) -> Option<&PathBuf> {
        match self {
            Self::Screenshots => settings.screenshot_dir.as_ref(),
            Self::Recordings => settings.recording_dir.as_ref(),
        }
    }

    fn set_directory(self, settings: &mut AppSettings, directory: Option<PathBuf>) {
        match self {
            Self::Screenshots => settings.screenshot_dir = directory,
            Self::Recordings => settings.recording_dir = directory,
        }
    }

    fn directory(self, settings: &AppSettings) -> PathBuf {
        match self {
            Self::Screenshots => settings
                .screenshot_dir
                .clone()
                .unwrap_or_else(storage::default_screenshot_dir),
            Self::Recordings => settings
                .recording_dir
                .clone()
                .unwrap_or_else(storage::default_recording_dir),
        }
    }
}

#[derive(Clone, Copy)]
struct ToggleRow {
    id: &'static str,
    icon: &'static [u8],
    title_key: &'static str,
    description_key: Option<&'static str>,
    value: bool,
    set: fn(&mut AppSettings, bool),
    refresh_pin: bool,
}

/// Popup menu for settings presented as a dropdown (repeat-pin behavior). The
/// anchor is the trigger's window-space bounds captured during paint; the menu
/// itself renders at the window root so it is not clipped by the card or the
/// scrolling page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectKind {
    WindowBackground,
    WindowPadding,
    RepeatPin,
}

#[derive(Clone, Copy)]
struct SelectMenu {
    kind: SelectKind,
    anchor: Bounds<Pixels>,
}

struct RecordColorPicker {
    anchor: Bounds<Pixels>,
    buffer: String,
    fresh: bool,
}

#[derive(Clone, Copy)]
struct Theme {
    text: Rgba,
    muted: Rgba,
    card: Rgba,
    card_border: Rgba,
    page: Option<Rgba>,
    border: Rgba,
    accent: Rgba,
    accent_text: Rgba,
    /// Subtle accent wash behind the selected dropdown row, matching the
    /// native component list highlight.
    selected_accent: Rgba,
    hover: Rgba,
    selected: Rgba,
    control: Rgba,
    control_hover: Rgba,
    toggle_off: Rgba,
    knob: Rgba,
    popover: Rgba,
    #[cfg(target_os = "windows")]
    close_hover: Rgba,
    hairline: f32,
}

struct Palette {
    text: Rgba,
    muted: Rgba,
    border: Rgba,
    control: Rgba,
    control_hover: Rgba,
    hover: Rgba,
    selected: Rgba,
    toggle_off: Rgba,
    popover: Rgba,
    page: Option<Rgba>,
}

impl Theme {
    fn for_window(window: &Window, accent: Option<u32>) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let accent_hex = accent.unwrap_or(if dark {
            FALLBACK_ACCENT_DARK
        } else {
            FALLBACK_ACCENT_LIGHT
        });
        Self::build(dark, accent_hex, window.scale_factor())
    }

    fn build(dark: bool, accent_hex: u32, scale_factor: f32) -> Self {
        let palette = native_palette(dark);
        Self {
            text: palette.text,
            muted: palette.muted,
            card: card_color(dark),
            card_border: card_border(dark),
            page: palette.page,
            border: palette.border,
            accent: rgb(accent_hex),
            accent_text: accent_text_color(accent_hex),
            selected_accent: rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x1A),
            hover: palette.hover,
            selected: palette.selected,
            control: palette.control,
            control_hover: palette.control_hover,
            toggle_off: palette.toggle_off,
            knob: rgb(0xFFFFFF),
            popover: palette.popover,
            #[cfg(target_os = "windows")]
            close_hover: rgb(0xEF4444),
            hairline: 1.0 / scale_factor.max(1.0),
        }
    }
}

/// Label color on the system accent. macOS keeps white-on-accent like the
/// system controls; Windows switches to a dark label on bright
/// accents (gold, lime) so the text stays readable.
fn accent_text_color(hex: u32) -> Rgba {
    #[cfg(target_os = "macos")]
    {
        let _ = hex;
        rgb(0xFFFFFF)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if prefers_dark_foreground(hex) {
            rgb(0x1A1A1A)
        } else {
            rgb(0xFFFFFF)
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn prefers_dark_foreground(hex: u32) -> bool {
    fn linearize(channel: u8) -> f64 {
        let value = f64::from(channel) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    let r = linearize(((hex >> 16) & 0xFF) as u8);
    let g = linearize(((hex >> 8) & 0xFF) as u8);
    let b = linearize((hex & 0xFF) as u8);
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let dark = (luminance + 0.05) / 0.05;
    let light = 1.05 / (luminance + 0.05);
    dark > light * 1.15
}

#[cfg(target_os = "windows")]
fn native_palette(dark: bool) -> Palette {
    if dark {
        Palette {
            text: rgb(0xF2F2F2),
            muted: rgb(0xA8A8A8),
            border: rgba(0xFFFFFF18),
            control: rgb(0x383838),
            control_hover: rgb(0x424242),
            hover: rgba(0xFFFFFF10),
            selected: rgba(0xFFFFFF14),
            toggle_off: rgba(0xFFFFFF1F),
            popover: rgb(0x2B2B2B),
            page: None,
        }
    } else {
        Palette {
            text: rgb(0x1B1B1B),
            muted: rgb(0x5D5D5D),
            border: rgba(0x0000000F),
            control: rgb(0xFCFCFC),
            control_hover: rgb(0xF2F2F2),
            hover: rgba(0x00000008),
            selected: rgba(0x0000000A),
            toggle_off: rgba(0x0000001A),
            popover: rgb(0xF9F9F9),
            page: None,
        }
    }
}

#[cfg(target_os = "macos")]
fn native_palette(dark: bool) -> Palette {
    if dark {
        Palette {
            text: rgb(0xF5F5F7),
            muted: rgb(0x98989D),
            border: rgba(0xFFFFFF1F),
            control: rgb(0x3A3A3C),
            control_hover: rgb(0x464648),
            hover: rgba(0xFFFFFF14),
            selected: rgba(0xFFFFFF1F),
            toggle_off: rgba(0xFFFFFF1F),
            popover: rgb(0x2C2C2E),
            // Native settings tint: 0.72 dark / 0.82 light over the live
            // window blur, using the same neutral-950 / white base palette as
            // the system popovers instead of a lighter custom grey.
            page: Some(rgba(0x0A0A0AB8)),
        }
    } else {
        Palette {
            text: rgb(0x1D1D1F),
            muted: rgb(0x6E6E73),
            border: rgba(0x00000014),
            control: rgb(0xFFFFFF),
            control_hover: rgb(0xF1F1F1),
            hover: rgba(0x0000000A),
            selected: rgba(0x0000000F),
            toggle_off: rgba(0x0000001A),
            popover: rgb(0xFFFFFF),
            page: Some(rgba(0xFFFFFFD1)),
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn native_palette(dark: bool) -> Palette {
    if dark {
        Palette {
            text: rgb(0xF5F5F7),
            muted: rgb(0x98989D),
            border: rgba(0xFFFFFF1F),
            control: rgb(0x3A3A3C),
            control_hover: rgb(0x464648),
            hover: rgba(0xFFFFFF14),
            selected: rgba(0xFFFFFF1F),
            toggle_off: rgba(0xFFFFFF4D),
            popover: rgb(0x2C2C2E),
            page: Some(rgb(0x1E1E1E)),
        }
    } else {
        Palette {
            text: rgb(0x1D1D1F),
            muted: rgb(0x6E6E73),
            border: rgba(0x00000014),
            control: rgb(0xFFFFFF),
            control_hover: rgb(0xF1F1F1),
            hover: rgba(0x0000000A),
            selected: rgba(0x0000000F),
            toggle_off: rgba(0x00000026),
            popover: rgb(0xFFFFFF),
            page: Some(rgb(0xF2F2F2)),
        }
    }
}

#[cfg(target_os = "windows")]
fn card_color(dark: bool) -> Rgba {
    // Opaque Windows cards: GPUI's Windows alpha blending accumulates
    // translucency across layers, so opaque cards are the stable surface.
    if dark { rgb(0x2B2B2B) } else { rgb(0xFAFAFA) }
}

#[cfg(target_os = "macos")]
fn card_color(dark: bool) -> Rgba {
    // macOS grouped cards: a translucent lift over the page tint.
    if dark {
        rgba(0xFFFFFF1A)
    } else {
        rgba(0xFFFFFFD1)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn card_color(dark: bool) -> Rgba {
    if dark { rgb(0x2C2C2E) } else { rgb(0xFFFFFF) }
}

#[cfg(target_os = "windows")]
fn card_border(dark: bool) -> Rgba {
    if dark {
        rgba(0x0000001A)
    } else {
        rgba(0x0000000F)
    }
}

#[cfg(not(target_os = "windows"))]
fn card_border(_dark: bool) -> Rgba {
    rgba(0x00000000)
}

struct SettingsView {
    section: SettingsSection,
    settings: AppSettings,
    accent: Option<u32>,
    last_accent: Option<u32>,
    status: Option<String>,
    status_serial: u64,
    recording_shortcut: Option<ShortcutSlot>,
    filename_input: Entity<InputState>,
    filename_subscription: Option<Subscription>,
    focus_handle: FocusHandle,
    #[cfg(target_os = "macos")]
    titlebar_drag: bool,
    select_menu: Option<SelectMenu>,
    record_color_picker: Option<RecordColorPicker>,
    #[cfg(target_os = "macos")]
    permission_screen: bool,
    #[cfg(target_os = "macos")]
    permission_accessibility: bool,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = settings_state::get().as_ref().clone();
        let theme = Theme::for_window(window, None);
        let filename_input =
            cx.new(|cx| InputState::new(window, cx).default_value(&settings.filename_template));
        filename_input.update(cx, |state, cx| {
            state.set_placeholder(settings_state::DEFAULT_FILENAME_TEMPLATE, window, cx);
            state.set_editor_style(input_editor_style(&theme));
        });
        let mut view = Self {
            section: SettingsSection::General,
            settings,
            accent: system_accent_hex(),
            last_accent: None,
            status: PENDING_STATUS.with(|slot| slot.borrow_mut().take()),
            status_serial: 0,
            recording_shortcut: None,
            filename_input,
            filename_subscription: None,
            focus_handle: cx.focus_handle(),
            #[cfg(target_os = "macos")]
            titlebar_drag: false,
            select_menu: None,
            record_color_picker: None,
            #[cfg(target_os = "macos")]
            permission_screen: platform_window::screen_capture_trusted(),
            #[cfg(target_os = "macos")]
            permission_accessibility: platform_window::accessibility_trusted(),
        };
        let input = view.filename_input.clone();
        view.filename_subscription = Some(cx.subscribe_in(
            &input,
            window,
            |this: &mut SettingsView, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change | InputEvent::Focus => cx.notify(),
                InputEvent::Blur => {
                    this.commit_filename_template(input, window, cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.commit_filename_template(input, window, cx);
                }
            },
        ));
        cx.on_release(|this, cx| {
            let template: String = this
                .filename_input
                .read(cx)
                .value()
                .trim()
                .chars()
                .take(120)
                .collect();
            if !template.is_empty() && template != settings_state::get().filename_template {
                settings_state::update(|settings| settings.filename_template = template);
            }
        })
        .detach();
        view.start_accent_watch(cx);
        #[cfg(target_os = "macos")]
        view.start_permission_watch(cx);
        if let Some(status) = view.status.clone() {
            view.show_status(status, cx);
        }
        view
    }

    /// Shows the bottom-right toast for a few seconds. Status used to stay
    /// forever after a filename or folder edit, so every user-visible message
    /// goes through this serial-guarded timer.
    fn show_status(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = Some(message);
        self.status_serial = self.status_serial.wrapping_add(1);
        let serial = self.status_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            this.update(cx, |this, cx| {
                if this.status_serial == serial {
                    this.status = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The platform appearance observer only fires on light/dark changes, so
    /// an accent-only change needs a poll. The sentinel `None` seed forces a
    /// first re-apply even if startup read the accent before the window was
    /// ready. The probe runs inside `update` because AppKit color reads are
    /// main-thread work.
    fn start_accent_watch(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let alive = this
                    .update(cx, |view: &mut SettingsView, cx| {
                        let current = system_accent_hex();
                        if current == view.last_accent {
                            return true;
                        }
                        view.last_accent = current;
                        view.accent = current;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(target_os = "macos")]
    fn start_permission_watch(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1000))
                    .await;
                let alive = this
                    .update(cx, |view: &mut SettingsView, cx| {
                        let screen = platform_window::screen_capture_trusted();
                        let accessibility = platform_window::accessibility_trusted();
                        if screen == view.permission_screen
                            && accessibility == view.permission_accessibility
                        {
                            return true;
                        }
                        view.permission_screen = screen;
                        view.permission_accessibility = accessibility;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let mut nav: Vec<AnyElement> = Vec::new();
        for section in SettingsSection::ALL {
            let selected = self.section == section;
            let hover = theme.hover;
            let accent = theme.accent;
            let muted = theme.muted;
            let text = theme.text;
            let selected_bg = theme.selected;
            let item = div()
                .w_full()
                .h(px(layout::NAV_ITEM_HEIGHT))
                .px(px(layout::NAV_ITEM_PAD_X))
                .rounded(px(layout::NAV_ITEM_RADIUS))
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(layout::NAV_GAP))
                .when(selected, |item| item.bg(selected_bg))
                .when(!selected, |item| item.hover(move |style| style.bg(hover)))
                .id(section.id())
                .active(move |style| style.bg(selected_bg))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    this.select_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px((layout::NAV_ITEM_HEIGHT - 16.0) / 2.0))
                        .w(px(3.0))
                        .h(px(16.0))
                        .rounded(px(1.5))
                        .when(selected, |bar| bar.bg(accent)),
                )
                .child(icons::icon(
                    section.icon(),
                    layout::NAV_ICON,
                    if selected { accent } else { muted },
                ))
                .child(
                    div()
                        .text_size(px(layout::NAV_FONT))
                        .text_color(text)
                        .child(section.title(self.settings.language)),
                );
            nav.push(item.into_any_element());
        }

        let nav = div().w_full().flex().flex_col().gap(px(2.0)).children(nav);
        let mut sidebar = div()
            .w(px(layout::SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(px(layout::SIDEBAR_PAD))
            .pb(px(layout::SIDEBAR_PAD));

        // macOS puts the traffic lights over the sidebar, so the title sits
        // below them at the top of the pane. The strip around the lights and
        // the title itself move the window; the content page starts at the
        // very top of the window to the right.
        #[cfg(target_os = "macos")]
        {
            sidebar = sidebar
                .child(self.macos_drag_area(
                    "settings-sidebar-drag-strip",
                    div().w_full().h(px(layout::TITLEBAR_HEIGHT)),
                    cx,
                ))
                .child(self.macos_drag_area(
                    "settings-sidebar-drag-title",
                    self.sidebar_title(theme),
                    cx,
                ))
                .child(div().w_full().mt(px(12.0)).child(nav));
        }
        // Windows keeps the system caption buttons and drag strip, so the
        // navigation starts below the 34pt caption.
        #[cfg(not(target_os = "macos"))]
        {
            sidebar = sidebar.pt(px(layout::TITLEBAR_HEIGHT + 8.0)).child(nav);
        }

        sidebar = sidebar.child(self.sidebar_footer(theme, cx));
        if layout::SIDEBAR_DIVIDER {
            sidebar.border_r_1().border_color(theme.border)
        } else {
            sidebar
        }
    }

    #[cfg(target_os = "macos")]
    fn sidebar_title(&self, theme: &Theme) -> Div {
        div()
            .w_full()
            .h(px(24.0))
            .px(px(layout::NAV_ITEM_PAD_X))
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.text)
                    .child(t(self.settings.language, "settings.title")),
            )
    }

    #[cfg(target_os = "macos")]
    fn macos_drag_area(
        &self,
        id: &'static str,
        area: Div,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        area.id(id)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _| {
                    if event.click_count == 2 {
                        this.titlebar_drag = false;
                        window.titlebar_double_click();
                    } else {
                        this.titlebar_drag = true;
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.titlebar_drag = false),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| this.titlebar_drag = false))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.titlebar_drag {
                    this.titlebar_drag = false;
                    window.start_window_move();
                }
            }))
    }

    fn sidebar_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let accent = theme.accent;
        let version = match BUILD_ID {
            Some(id) => format!("Capin v{}+{id}", env!("CARGO_PKG_VERSION")),
            None => format!("Capin v{}", env!("CARGO_PKG_VERSION")),
        };
        div()
            .mt_auto()
            .w_full()
            .pt(px(16.0))
            .pb(px(2.0))
            .pl(px(layout::NAV_ITEM_PAD_X))
            .flex()
            .justify_start()
            .child(
                div()
                    .id("settings-version-link")
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(accent))
                    .on_click(cx.listener(|this, _, _, cx| this.open_project(cx)))
                    .child(version),
            )
    }

    /// The Windows caption is drawn inside the client area so it shares the
    /// window backdrop with the page while keeping the system caption hit
    /// testing (drag, snap, double-click). macOS uses the native title bar;
    /// its drag surface lives in the sidebar.
    #[cfg(target_os = "windows")]
    fn titlebar(&self, theme: &Theme, window: &mut Window, _cx: &mut Context<Self>) -> Div {
        let leading = div()
            .id("settings-titlebar-drag")
            .flex_1()
            .h_full()
            .pl(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.text)
                    .child(t(self.settings.language, "settings.title")),
            );

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(layout::TITLEBAR_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .child(leading)
            .child(window_control_button(
                theme,
                "settings-window-minimize",
                icons::MINIMIZE,
                WindowControlArea::Min,
                false,
            ))
            .child(window_control_button(
                theme,
                "settings-window-maximize",
                if window.is_maximized() {
                    icons::RESTORE
                } else {
                    icons::MAXIMIZE
                },
                WindowControlArea::Max,
                false,
            ))
            .child(window_control_button(
                theme,
                "settings-window-close",
                icons::WINDOW_CLOSE,
                WindowControlArea::Close,
                true,
            ))
    }

    fn content(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // macOS draws the settings title in the sidebar, so the page reaches
        // the top edge; Windows keeps the caption strip clear of scrolling
        // content so rows never slide under the caption buttons.
        #[cfg(target_os = "macos")]
        let top = 0.0;
        #[cfg(not(target_os = "macos"))]
        let top = layout::TITLEBAR_HEIGHT;
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(top))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(px(layout::CONTENT_PAD_X))
                    .pt(px(layout::CONTENT_PAD_TOP))
                    .pb(px(layout::HEADER_GAP))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(layout::CONTENT_MAX_WIDTH))
                            .child(self.section_header(theme)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .px(px(layout::CONTENT_PAD_X))
                    .pb(px(layout::CONTENT_PAD_BOTTOM))
                    .id(format!("settings-content-{}", self.section.id()))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(layout::CONTENT_MAX_WIDTH))
                            .flex()
                            .flex_col()
                            .gap(px(layout::CONTENT_GAP))
                            .child(match self.section {
                                SettingsSection::General => self.general(theme, window, cx),
                                SettingsSection::Capture => self.capture_settings(theme, cx),
                                SettingsSection::Recording => self.recording(theme, cx),
                                SettingsSection::Shortcuts => self.shortcuts(theme, cx),
                            }),
                    ),
            )
    }

    fn section_header(&self, theme: &Theme) -> Div {
        div().flex().flex_col().child(
            div()
                .text_size(px(layout::PAGE_TITLE))
                .text_color(theme.text)
                .child(self.section.title(self.settings.language)),
        )
    }

    fn general(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        #[cfg(target_os = "macos")]
        let language = self.settings.language;
        let content = card(
            theme,
            vec![
                div().child(self.language_row(theme, cx)),
                div().child(self.auto_start_row(theme, cx)),
                div().child(self.logs_row(theme, cx)),
                div().child(self.filename_template_row(theme, window, cx)),
            ],
        );
        let page = div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(content);
        #[cfg(target_os = "macos")]
        let page = page.child(group(
            theme,
            t(language, "settings.group.permissions"),
            "",
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(self.permissions_card(theme, cx))
                .when(
                    !(self.permission_screen && self.permission_accessibility),
                    |body| body.child(self.permission_hint(theme, cx)),
                ),
        ));
        page
    }

    /// Rebuilds the permission guidance that accompanies the status rows.
    /// macOS binds TCC grants to the ad-hoc code signature, and screen
    /// recording only takes effect after a relaunch, so an existing System
    /// Settings entry can look enabled while this process is still denied.
    #[cfg(target_os = "macos")]
    fn permission_hint(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(note(theme, t(language, "settings.permission_hint"))),
            )
            .child(
                action_button(theme, t(language, "settings.permission_restart").to_owned())
                    .id("settings-permission-restart")
                    .active(|style| style.opacity(0.88))
                    .on_click(cx.listener(|_, _, _, cx| crate::tray::restart(cx))),
            )
    }

    #[cfg(target_os = "macos")]
    fn permissions_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        card(
            theme,
            vec![
                div().child(self.permission_row(
                    theme,
                    t(language, "settings.permission_screen"),
                    t(language, "settings.permission_screen_desc"),
                    self.permission_screen,
                    platform_window::PrivacyPane::ScreenCapture,
                    cx,
                )),
                div().child(self.permission_row(
                    theme,
                    t(language, "settings.permission_accessibility"),
                    t(language, "settings.permission_accessibility_desc"),
                    self.permission_accessibility,
                    platform_window::PrivacyPane::Accessibility,
                    cx,
                )),
            ],
        )
    }

    #[cfg(target_os = "macos")]
    fn permission_row(
        &self,
        theme: &Theme,
        title: &str,
        description: &str,
        granted: bool,
        pane: platform_window::PrivacyPane,
        cx: &mut Context<Self>,
    ) -> Div {
        let language = self.settings.language;
        let status = t(
            language,
            if granted {
                "settings.permission_granted"
            } else {
                "settings.permission_missing"
            },
        );
        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::SHIELD,
                title,
                description,
            ))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(layout::CAPTION))
                            .text_color(if granted { theme.muted } else { theme.accent })
                            .child(status),
                    )
                    .child(
                        action_button(theme, t(language, "settings.open_settings").to_owned())
                            .id(match pane {
                                platform_window::PrivacyPane::ScreenCapture => {
                                    "settings-open-screen-permission"
                                }
                                platform_window::PrivacyPane::Accessibility => {
                                    "settings-open-accessibility-permission"
                                }
                            })
                            .active(|style| style.opacity(0.88))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                platform_window::open_privacy_settings(pane);
                                this.permission_screen = platform_window::screen_capture_trusted();
                                this.permission_accessibility =
                                    platform_window::accessibility_trusted();
                                cx.notify();
                            })),
                    ),
            )
    }

    fn language_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let mut control = segmented();
        for option in Language::ALL {
            let selected = language == option;
            let label = option.label();
            let code = option.code();
            control = control.child(
                segment(theme, label, selected)
                    .id(code)
                    .active(|style| style.opacity(0.9))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.settings.language = option;
                        settings_state::update(|settings| settings.language = option);
                        crate::tray::refresh_menu();
                        window.set_window_title(t(option, "settings.title"));
                        this.status = None;
                        cx.notify();
                    })),
            );
        }

        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::LANGUAGE,
                t(language, "settings.language"),
                "",
            ))
            .child(control)
    }

    fn auto_start_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.auto_start;

        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::STARTUP,
                t(language, "settings.auto_start"),
                "",
            ))
            .child(
                div()
                    .id("settings-auto-start")
                    .cursor_pointer()
                    .active(move |style| style.opacity(0.85))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let language = this.settings.language;
                        let desired = !this.settings.auto_start;
                        match set_autostart(desired) {
                            Ok(AutostartState::Unsupported) => {
                                this.show_status(
                                    t(language, "settings.status_autostart_failed").to_owned(),
                                    cx,
                                );
                            }
                            Ok(state) => {
                                this.settings.auto_start = desired;
                                this.settings.auto_start_explicit = true;
                                settings_state::update(|settings| {
                                    settings.auto_start = desired;
                                    settings.auto_start_explicit = true;
                                });
                                let message = match state {
                                    AutostartState::Enabled => {
                                        t(language, "settings.status_autostart_enabled")
                                    }
                                    AutostartState::Disabled => {
                                        t(language, "settings.status_autostart_disabled")
                                    }
                                    AutostartState::RequiresApproval => {
                                        t(language, "settings.status_autostart_approval")
                                    }
                                    AutostartState::Unsupported => {
                                        t(language, "settings.status_autostart_failed")
                                    }
                                };
                                this.show_status(message.to_owned(), cx);
                            }
                            Err(error) => {
                                this.show_status(
                                    format!(
                                        "{}{}",
                                        t(language, "settings.status_autostart_failed"),
                                        error
                                    ),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    }))
                    .child(toggle(theme, value, "settings-auto-start-toggle")),
            )
    }

    fn logs_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let open = action_button(theme, t(language, "settings.open_logs").to_owned())
            .id("settings-open-logs")
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(|_, _, _, cx| {
                let directory = crate::logging::log_directory();
                let _ = std::fs::create_dir_all(&directory);
                log_info!("settings", "opening log directory");
                cx.open_with_system(&directory);
            }));
        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::LOGS,
                t(language, "settings.logs"),
                t(language, "settings.logs_desc"),
            ))
            .child(open)
    }

    fn capture_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;

        div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(group(
                theme,
                t(language, "settings.group.selection"),
                "",
                card(
                    theme,
                    vec![
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-copy-to-clipboard",
                                icon: icons::CLIPBOARD,
                                title_key: "settings.copy_clipboard",
                                description_key: None,
                                value: self.settings.copy_to_clipboard,
                                set: |settings, value| settings.copy_to_clipboard = value,
                                refresh_pin: false,
                            },
                        ),
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-capture-cursor",
                                icon: icons::CURSOR,
                                title_key: "settings.capture_cursor",
                                description_key: None,
                                value: self.settings.capture_cursor,
                                set: |settings, value| settings.capture_cursor = value,
                                refresh_pin: false,
                            },
                        ),
                        self.save_location_row(SaveTarget::Screenshots, theme, cx),
                    ],
                ),
            ))
            .child(group(
                theme,
                t(language, "settings.group.window"),
                "",
                card(
                    theme,
                    vec![
                        self.window_background_row(theme, cx),
                        self.window_padding_row(theme, cx),
                    ],
                ),
            ))
            .child(group(
                theme,
                t(language, "settings.group.annotation"),
                "",
                card(
                    theme,
                    vec![self.toggle_row(
                        theme,
                        cx,
                        ToggleRow {
                            id: "settings-annotation-uniform-color",
                            icon: icons::PALETTE,
                            title_key: "settings.annotation_uniform_color",
                            description_key: Some("settings.annotation_uniform_color_desc"),
                            value: self.settings.annotation_uniform_color,
                            set: |settings, value| settings.annotation_uniform_color = value,
                            refresh_pin: false,
                        },
                    )],
                ),
            ))
            .child(group(
                theme,
                t(language, "settings.group.pin"),
                "",
                card(
                    theme,
                    vec![
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-pin-text-ocr",
                                icon: icons::TEXT,
                                title_key: "settings.pin_text_ocr",
                                description_key: Some("settings.pin_text_ocr_desc"),
                                value: self.settings.pin_text_ocr,
                                set: |settings, value| settings.pin_text_ocr = value,
                                refresh_pin: true,
                            },
                        ),
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-pin-qr",
                                icon: icons::QR,
                                title_key: "settings.pin_qr",
                                description_key: None,
                                value: self.settings.pin_qr,
                                set: |settings, value| settings.pin_qr = value,
                                refresh_pin: true,
                            },
                        ),
                        self.repeat_pin_row(theme, cx),
                    ],
                ),
            ))
    }

    /// One on/off row. Only the switch is a hit target; the label and the row
    /// background stay inert like the native settings lists. `refresh_pin`
    /// reapplies pin recognition when a recognition toggle changes.
    fn toggle_row(&self, theme: &Theme, cx: &mut Context<Self>, row: ToggleRow) -> Div {
        let language = self.settings.language;
        div().child(
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    row.icon,
                    t(language, row.title_key),
                    row.description_key
                        .map(|key| t(language, key))
                        .unwrap_or_default(),
                ))
                .child(
                    div()
                        .id(row.id)
                        .cursor_pointer()
                        .active(move |style| style.opacity(0.85))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let value = !row.value;
                            (row.set)(&mut this.settings, value);
                            settings_state::update(|settings| (row.set)(settings, value));
                            if row.refresh_pin {
                                cx.defer(crate::pin::refresh_recognition);
                            }
                            this.status = None;
                            cx.notify();
                        }))
                        .child(toggle(theme, row.value, row.id)),
                ),
        )
    }

    fn window_background_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let current = t(
            language,
            background_label_key(self.settings.window_background),
        );
        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::IMAGE,
                t(language, "settings.window_background"),
                t(language, "settings.window_background_desc"),
            ))
            .child(self.select_trigger(
                theme,
                current,
                "settings-window-background",
                SelectKind::WindowBackground,
                cx,
            ))
    }

    fn window_padding_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let current = self.settings.window_padding.to_string();
        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::PADDING,
                t(language, "settings.window_padding"),
                "",
            ))
            .child(self.select_trigger(
                theme,
                &current,
                "settings-window-padding",
                SelectKind::WindowPadding,
                cx,
            ))
    }

    fn repeat_pin_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let current = t(language, repeat_pin_key(self.settings.repeat_pin));
        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::PIN,
                t(language, "settings.repeat_pin"),
                t(language, "settings.repeat_pin_desc"),
            ))
            .child(self.select_trigger(
                theme,
                current,
                "settings-repeat-pin",
                SelectKind::RepeatPin,
                cx,
            ))
    }

    fn select_entries(&self, kind: SelectKind) -> Vec<(String, u32, bool)> {
        let language = self.settings.language;
        let mut entries: Vec<(String, u32, bool)> = Vec::new();
        match kind {
            SelectKind::WindowBackground => {
                for (index, background) in WindowBackground::ALL.into_iter().enumerate() {
                    entries.push((
                        t(language, background_label_key(background)).to_owned(),
                        index as u32,
                        self.settings.window_background == background,
                    ));
                }
            }
            SelectKind::WindowPadding => {
                for padding in WINDOW_PADDINGS {
                    entries.push((
                        t(language, padding_key(padding)).to_owned(),
                        padding,
                        self.settings.window_padding == padding,
                    ));
                }
            }
            SelectKind::RepeatPin => {
                for (index, behavior) in RepeatPinBehavior::ALL.into_iter().enumerate() {
                    entries.push((
                        t(language, repeat_pin_key(behavior)).to_owned(),
                        index as u32,
                        self.settings.repeat_pin == behavior,
                    ));
                }
            }
        }
        entries
    }

    fn apply_select(&mut self, kind: SelectKind, value: u32, cx: &mut Context<Self>) {
        match kind {
            SelectKind::WindowBackground => {
                let background = WindowBackground::ALL[value as usize];
                self.settings.window_background = background;
                settings_state::update(|settings| settings.window_background = background);
            }
            SelectKind::WindowPadding => {
                if WINDOW_PADDINGS.contains(&value) {
                    self.settings.window_padding = value;
                    settings_state::update(|settings| settings.window_padding = value);
                }
            }
            SelectKind::RepeatPin => {
                let behavior = RepeatPinBehavior::ALL[value as usize];
                self.settings.repeat_pin = behavior;
                settings_state::update(|settings| settings.repeat_pin = behavior);
            }
        }
        self.status = None;
        cx.notify();
    }

    /// A macOS-style popup button: the current value on the left, a chevron
    /// on the right. Windows opens an in-window popover anchored to the
    /// trigger; macOS raises the native menu at the pointer.
    fn select_trigger(
        &self,
        theme: &Theme,
        label: &str,
        id: &'static str,
        kind: SelectKind,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let anchor: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let anchor_for_paint = anchor.clone();
        let probe = canvas(
            move |bounds, _, _| anchor_for_paint.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element();
        let hover = theme.control_hover;
        let chevron = theme.muted;
        div()
            .id(id)
            .relative()
            .w(px(160.0))
            .h(px(layout::BUTTON_HEIGHT))
            .flex_shrink_0()
            .px(px(10.0))
            .rounded(px(layout::BUTTON_RADIUS))
            .bg(theme.control)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .text_size(px(layout::CONTROL_TEXT))
            .text_color(theme.text)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .active(move |style| style.opacity(0.9))
            .on_click(cx.listener(move |this, _, _, cx| {
                #[cfg(target_os = "macos")]
                {
                    // The native menu runs its own tracking loop; showing it
                    // inside the click dispatch would hold the App borrow and
                    // abort on a re-entrant timer. Defer one main-queue turn.
                    let entries = this.select_entries(kind);
                    let selected = entries.iter().position(|(_, _, active)| *active);
                    let items: Vec<String> =
                        entries.iter().map(|(label, _, _)| label.clone()).collect();
                    let values: Vec<u32> = entries.iter().map(|(_, value, _)| *value).collect();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(1))
                            .await;
                        let Some(index) =
                            platform_window::show_native_menu(&items, selected.unwrap_or(0))
                        else {
                            return;
                        };
                        let Some(value) = values.get(index).copied() else {
                            return;
                        };
                        this.update(cx, |this, cx| this.apply_select(kind, value, cx))
                            .ok();
                    })
                    .detach();
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if let Some(anchor) = anchor.get() {
                        this.select_menu = Some(SelectMenu { kind, anchor });
                    }
                    cx.notify();
                }
            }))
            .child(label.to_owned())
            .child(icons::icon(icons::CHEVRON_DOWN, 10.0, chevron))
            .child(probe)
    }

    /// The popup layer for the long option lists (window background, padding,
    /// repeat-pin behavior). Rendering it at the window root keeps it above the
    /// card and outside the scrolling page.
    fn select_popup(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.select_menu?;
        let entries = self.select_entries(menu.kind);

        let width = 160.0;
        let item_height = 26.0;
        let padding = 4.0;
        let height = entries.len() as f32 * item_height + padding * 2.0;
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let anchor_x = f32::from(menu.anchor.origin.x);
        let anchor_y = f32::from(menu.anchor.origin.y);
        let anchor_width = f32::from(menu.anchor.size.width);
        let anchor_height = f32::from(menu.anchor.size.height);
        let mut x = anchor_x + anchor_width - width;
        let mut y = anchor_y + anchor_height + 4.0;
        if y + height > viewport_height - 8.0 {
            y = (anchor_y - height - 4.0).max(8.0);
        }
        x = x.clamp(8.0, (viewport_width - width - 8.0).max(8.0));

        let mut popup = div()
            .id("settings-select-popup")
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(width))
            .p(px(padding))
            .rounded(px(6.0))
            .bg(theme.popover)
            .shadow(vec![
                BoxShadow::new(px(0.0), px(0.0), Hsla::from(theme.border)).spread_radius(px(1.0)),
                BoxShadow::new(px(0.0), px(4.0), Hsla::from(rgba(0x0000001F)))
                    .blur_radius(px(3.0))
                    .spread_radius(px(-1.0)),
                BoxShadow::new(px(0.0), px(2.0), Hsla::from(rgba(0x00000014)))
                    .blur_radius(px(2.0))
                    .spread_radius(px(-2.0)),
            ])
            .flex()
            .flex_col()
            .gap(px(1.0))
            .occlude();
        for (index, (label, value, selected)) in entries.into_iter().enumerate() {
            let selected_bg = theme.selected_accent;
            let check = theme.muted;
            let text = theme.text;
            let hover = theme.hover;
            popup = popup.child(
                div()
                    .id(("settings-select-item", index))
                    .h(px(item_height))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(layout::CONTROL_TEXT))
                    .when(selected, |item| item.bg(selected_bg).text_color(text))
                    .when(!selected, |item| {
                        item.text_color(text).hover(move |style| style.bg(hover))
                    })
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.select_menu = None;
                        this.apply_select(menu.kind, value, cx);
                    }))
                    .child(div().flex_1().min_w(px(0.0)).child(label))
                    .child(div().w(px(12.0)).flex_shrink_0().when(selected, |slot| {
                        slot.child(icons::icon(icons::CHECK, 14.0, check))
                    })),
            );
        }
        let spring = SpringAnimation::new(SpringConfig::new(900.0, 52.0, 1.0))
            .to(1.0f32)
            .from(0.0);
        let base_top = y;
        Some(
            popup
                .with_spring(
                    SharedString::from(format!("settings-select-popup-{}", menu.kind as u8)),
                    spring,
                    move |popup, progress| {
                        popup
                            .top(px(base_top + (1.0 - progress.clamp(0.0, 1.0)) * 5.0))
                            .opacity(progress.clamp(0.0, 1.0))
                    },
                )
                .into_any_element(),
        )
    }
    fn save_location_row(&self, target: SaveTarget, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let custom = target.selected_directory(&self.settings).is_some();
        let directory = target.directory(&self.settings).display().to_string();
        let open = action_button(theme, t(language, "settings.open_folder").to_owned())
            .id(target.open_id())
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(move |this, _, _, cx| this.open_directory(target, cx)));
        let change = action_button(theme, t(language, "settings.change_folder").to_owned())
            .id(target.change_id())
            .active(|style| style.opacity(0.88))
            .on_click(cx.listener(move |this, _, _, cx| this.choose_directory(target, cx)));
        let reset = custom.then(|| {
            action_button(theme, t(language, "settings.use_default_folder").to_owned())
                .id(target.default_id())
                .active(|style| style.opacity(0.88))
                .on_click(cx.listener(move |this, _, _, cx| this.reset_directory(target, cx)))
        });

        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::FOLDER,
                t(language, target.title_key()),
                "",
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_shrink_1()
                    .min_w(px(0.0))
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_shrink_1()
                            .min_w(px(0.0))
                            .max_w(px(230.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_middle()
                            .text_size(px(layout::CAPTION))
                            .text_color(theme.muted)
                            .child(directory),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(open)
                            .child(change)
                            .children(reset),
                    ),
            )
    }

    fn filename_template_row(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        self.filename_input.update(cx, |state, _| {
            state.set_editor_style(input_editor_style(theme));
        });
        let template = self.filename_input.read(cx).value().to_string();
        let preview = crate::storage::render_filename(&template, 1920, 1080);
        let focused = self
            .filename_input
            .read(cx)
            .presentation()
            .focus_handle()
            .is_focused(window);
        let field = div()
            .id("settings-filename-field")
            .w(px(260.0))
            .h(px(layout::BUTTON_HEIGHT))
            .flex_shrink_0()
            .px(px(9.0))
            .rounded(px(layout::BUTTON_RADIUS))
            .bg(theme.control)
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border })
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(layout::CONTROL_TEXT))
            .cursor_text()
            .child(Input::new(&self.filename_input))
            .on_mouse_down_out(cx.listener(|_, _: &MouseDownEvent, window, cx| window.blur(cx)))
            .on_click(cx.listener(|this, _, window, cx| {
                let focus = this
                    .filename_input
                    .read(cx)
                    .presentation()
                    .focus_handle()
                    .clone();
                window.focus(&focus, cx);
            }));

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(layout::ROW_PAD_X))
            .py(px(layout::ROW_PAD_Y))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(24.0))
                    .child(row_label_with_icon(
                        theme,
                        icons::FILENAME,
                        t(language, "settings.filename_template"),
                        t(language, "settings.filename_template_desc"),
                    ))
                    .child(field),
            )
            .child(
                div()
                    .pl(px(28.0))
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .child(format!(
                        "{} {}",
                        t(language, "settings.filename_template_preview"),
                        preview
                    )),
            )
    }

    fn commit_filename_template(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = input.read(cx).value().to_string();
        let template: String = raw.trim().chars().take(120).collect();
        if template.is_empty() {
            let message =
                t(self.settings.language, "settings.filename_template_invalid").to_owned();
            self.show_status(message, cx);
            input.update(cx, |state, cx| {
                state.set_value(self.settings.filename_template.clone(), window, cx);
            });
            return;
        }
        if template != raw {
            input.update(cx, |state, cx| {
                state.set_value(template.clone(), window, cx);
            });
        }
        if template == self.settings.filename_template {
            return;
        }
        self.settings.filename_template = template.clone();
        settings_state::update(|settings| settings.filename_template = template);
        let message = t(self.settings.language, "settings.filename_template_saved").to_owned();
        self.show_status(message, cx);
    }
    fn choose_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings.language;
        let prompt = t(language, "settings.choose_folder").to_owned();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        cx.spawn(async move |this, cx| {
            let (selected, error) = match receiver.await {
                Ok(Ok(Some(mut paths))) => (paths.pop(), None),
                Ok(Ok(None)) => (None, None),
                Ok(Err(error)) => (None, Some(error.to_string())),
                Err(_) => (None, None),
            };
            this.update(cx, |this, cx| {
                this.apply_directory_choice(target, selected, error, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_directory_choice(
        &mut self,
        target: SaveTarget,
        selected: Option<PathBuf>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.language;
        if let Some(error) = error {
            self.show_status(
                format!("{} {}", t(language, "settings.status_picker_failed"), error),
                cx,
            );
            return;
        }
        let Some(directory) = selected else {
            return;
        };
        if let Err(error) = storage::ensure_directory(&directory) {
            self.show_status(
                format!("{} {}", t(language, "settings.status_folder_failed"), error),
                cx,
            );
            return;
        }
        let directory = Some(directory);
        target.set_directory(&mut self.settings, directory.clone());
        settings_state::update(|settings| target.set_directory(settings, directory));
        self.show_status(t(language, "settings.status_folder_updated").to_owned(), cx);
    }

    fn reset_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings.language;
        target.set_directory(&mut self.settings, None);
        settings_state::update(|settings| target.set_directory(settings, None));
        self.show_status(t(language, "settings.status_folder_default").to_owned(), cx);
    }

    fn open_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings.language;
        let directory = target.directory(&self.settings);
        if let Err(error) = storage::ensure_directory(&directory) {
            self.show_status(
                format!("{} {}", t(language, "settings.status_folder_failed"), error),
                cx,
            );
        } else {
            cx.open_with_system(&directory);
        }
    }

    fn recording(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut rows = vec![
            self.toggle_row(
                theme,
                cx,
                ToggleRow {
                    id: "settings-record-cursor",
                    icon: icons::CURSOR,
                    title_key: "settings.record_cursor",
                    description_key: None,
                    value: self.settings.record_cursor,
                    set: |settings, value| settings.record_cursor = value,
                    refresh_pin: false,
                },
            ),
            self.toggle_row(
                theme,
                cx,
                ToggleRow {
                    id: "settings-record-cursor-highlight",
                    icon: icons::CURSOR,
                    title_key: "settings.record_cursor_highlight",
                    description_key: Some("settings.record_cursor_highlight_desc"),
                    value: self.settings.record_cursor_highlight,
                    set: |settings, value| settings.record_cursor_highlight = value,
                    refresh_pin: false,
                },
            ),
        ];
        if self.settings.record_cursor_highlight {
            rows.push(self.record_highlight_color_row(theme, cx));
        }
        rows.push(self.save_location_row(SaveTarget::Recordings, theme, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_HEADER_GAP))
            .child(note(theme, t(language, "settings.recording_note")))
            .child(card(theme, rows))
    }

    /// Pastel swatches plus a custom hex picker for the recording cursor
    /// disc. The color is stored as RGB; the recorder applies a fixed
    /// translucent alpha.
    fn record_highlight_color_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let current = self.settings.record_cursor_highlight_color;
        let mut swatches = Vec::new();
        for (index, color) in settings_state::RECORD_HIGHLIGHT_COLORS
            .into_iter()
            .enumerate()
        {
            let selected = current == color;
            let swatch = div()
                .id(("settings-record-highlight-color", index))
                .w(px(28.0))
                .h(px(28.0))
                .flex_shrink_0()
                .rounded(px(14.0))
                .p(px(3.0))
                .cursor_pointer()
                .border_1()
                .when(selected, |item| item.border_2().border_color(theme.accent))
                .when(!selected, |item| item.border_color(theme.border))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.record_cursor_highlight_color = color;
                    this.record_color_picker = None;
                    settings_state::update(|settings| {
                        settings.record_cursor_highlight_color = color
                    });
                    this.status = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .size_full()
                        .rounded(px(11.0))
                        .bg(rgb(color))
                        .border_1()
                        .border_color(theme.card_border),
                );
            swatches.push(swatch.into_any_element());
        }

        let anchor: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let anchor_for_paint = anchor.clone();
        let probe = canvas(
            move |bounds, _, _| anchor_for_paint.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element();
        let custom_selected = !settings_state::RECORD_HIGHLIGHT_COLORS.contains(&current);
        let picker_open = self.record_color_picker.is_some();
        let custom_tooltip = t(language, "editor.custom_color").to_owned();
        let custom = div()
            .id("settings-record-highlight-custom")
            .tooltip(move |_, cx| crate::editor::tooltip_view(custom_tooltip.clone(), cx))
            .relative()
            .w(px(28.0))
            .h(px(28.0))
            .flex_shrink_0()
            .rounded(px(14.0))
            .p(px(3.0))
            .cursor_pointer()
            .border_1()
            .when(custom_selected || picker_open, |item| {
                item.border_2().border_color(theme.accent)
            })
            .when(!custom_selected && !picker_open, |item| {
                item.border_color(theme.border)
            })
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(anchor) = anchor.get() {
                    this.open_record_color_picker(anchor, window, cx);
                }
            }))
            .child(
                div()
                    .size_full()
                    .rounded(px(11.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(custom_selected, |item| item.bg(rgb(current)))
                    .when(!custom_selected, |item| {
                        item.bg(theme.control)
                            .child(icons::icon(icons::PALETTE, 14.0, theme.muted))
                    })
                    .border_1()
                    .border_color(theme.card_border),
            )
            .child(probe);
        swatches.push(custom.into_any_element());

        row_shell()
            .child(row_label_with_icon(
                theme,
                icons::PALETTE,
                t(language, "settings.record_cursor_highlight_color"),
                t(language, "settings.record_cursor_highlight_color_desc"),
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .children(swatches),
            )
    }

    fn open_record_color_picker(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record_color_picker = Some(RecordColorPicker {
            anchor,
            buffer: format!("{:06x}", self.settings.record_cursor_highlight_color),
            fresh: true,
        });
        self.select_menu = None;
        self.status = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn apply_record_color_buffer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.record_color_picker.as_mut() else {
            return false;
        };
        let hex = picker.buffer.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        let Ok(rgb) = u32::from_str_radix(hex, 16) else {
            return false;
        };
        let color = rgb & 0x00FF_FFFF;
        self.settings.record_cursor_highlight_color = color;
        settings_state::update(|settings| settings.record_cursor_highlight_color = color);
        self.status = None;
        cx.notify();
        true
    }

    fn handle_record_color_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.record_color_picker.as_mut() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.record_color_picker = None;
                cx.notify();
                return true;
            }
            "enter" => {
                self.apply_record_color_buffer(cx);
                return true;
            }
            "backspace" | "delete" => {
                if picker.fresh {
                    picker.buffer.clear();
                    picker.fresh = false;
                } else {
                    picker.buffer.pop();
                }
                cx.notify();
                return true;
            }
            _ => {}
        }
        let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        if !command
            && let Some(ch) = event.keystroke.key.chars().next()
            && ch.is_ascii_hexdigit()
        {
            if picker.fresh {
                picker.buffer.clear();
                picker.fresh = false;
            }
            if picker.buffer.len() < 6 {
                picker.buffer.push(ch.to_ascii_lowercase());
            }
            cx.notify();
            return true;
        }
        // Let the window shortcuts (Cmd/Ctrl+W, Cmd/Ctrl+Q) through.
        !command
    }

    fn record_color_popup(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.record_color_picker.as_ref()?;
        let language = self.settings.language;
        let width = 240.0;
        let height = 84.0;
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let anchor_x = f32::from(picker.anchor.origin.x);
        let anchor_y = f32::from(picker.anchor.origin.y);
        let anchor_width = f32::from(picker.anchor.size.width);
        let anchor_height = f32::from(picker.anchor.size.height);
        let mut x = anchor_x + anchor_width / 2.0 - width / 2.0;
        let mut y = anchor_y + anchor_height + 4.0;
        if y + height > viewport_height - 8.0 {
            y = (anchor_y - height - 4.0).max(8.0);
        }
        x = x.clamp(8.0, (viewport_width - width - 8.0).max(8.0));

        let preview_rgb = picker.buffer.trim().trim_start_matches('#');
        let preview = u32::from_str_radix(preview_rgb, 16)
            .ok()
            .filter(|_| preview_rgb.len() == 6)
            .map(|rgb| (rgb & 0x00FF_FFFF) << 8 | 0xFF)
            .unwrap_or((self.settings.record_cursor_highlight_color << 8) | 0xFF);
        let cursor = if picker.fresh { "" } else { "|" };
        let mut hue_row = div().flex().flex_row().items_center().gap(px(3.0));
        for index in 0..12u32 {
            let hue_color = crate::ui::hue_rgb(index as f32 * 30.0);
            hue_row = hue_row.child(
                div()
                    .id(("settings-record-hue", index))
                    .w(px(15.0))
                    .h(px(15.0))
                    .rounded(px(8.0))
                    .bg(rgb(hue_color))
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let color = hue_color;
                        this.settings.record_cursor_highlight_color = color;
                        settings_state::update(|settings| {
                            settings.record_cursor_highlight_color = color
                        });
                        if let Some(picker) = this.record_color_picker.as_mut() {
                            picker.buffer = format!("{color:06x}");
                            picker.fresh = false;
                        }
                        this.status = None;
                        cx.notify();
                    })),
            );
        }

        Some(
            div()
                .id("settings-record-color-popup")
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(width))
                .p(px(8.0))
                .rounded(px(6.0))
                .bg(theme.popover)
                .shadow(vec![
                    BoxShadow::new(px(0.0), px(0.0), Hsla::from(theme.border))
                        .spread_radius(px(1.0)),
                    BoxShadow::new(px(0.0), px(4.0), Hsla::from(rgba(0x0000001F)))
                        .blur_radius(px(3.0))
                        .spread_radius(px(-1.0)),
                ])
                .flex()
                .flex_col()
                .gap(px(6.0))
                .occlude()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .w(px(18.0))
                                .h(px(18.0))
                                .rounded(px(4.0))
                                .bg(rgba(preview))
                                .border_1()
                                .border_color(theme.border),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme.text)
                                .child(t(language, "editor.custom_color")),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(theme.control)
                                .text_size(px(11.0))
                                .text_color(theme.text)
                                .child(format!(
                                    "#{}{}",
                                    picker.buffer.to_ascii_uppercase(),
                                    cursor
                                )),
                        ),
                )
                .child(hue_row)
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(theme.muted)
                        .child(t(language, "editor.custom_color_hint")),
                )
                .into_any_element(),
        )
    }

    fn shortcuts(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let pin_hotkey = crate::hotkey::display_hotkey(&self.settings.pin_hotkey);
        let page = div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(layout::GROUP_HEADER_GAP))
                    .child(card(
                        theme,
                        vec![
                            self.shortcut_row(ShortcutSlot::Capture, theme, cx),
                            self.shortcut_row(ShortcutSlot::Pin, theme, cx),
                        ],
                    ))
                    .child(note(theme, t(language, "shortcuts.editor_note"))),
            )
            .child(group(
                theme,
                t(language, "settings.group.capture_keys"),
                t(language, "settings.group.capture_keys_desc"),
                card(
                    theme,
                    vec![
                        info_row(
                            theme,
                            t(language, "shortcuts.capture_region"),
                            "",
                            t(language, "shortcuts.value.drag"),
                            Some(icons::SELECT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.window_capture"),
                            "",
                            t(language, "shortcuts.value.auto"),
                            Some(icons::WINDOW),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.window_mode"),
                            t(language, "shortcuts.window_mode_desc"),
                            "Ctrl",
                            Some(icons::CAMERA),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.adjust_target"),
                            t(language, "shortcuts.adjust_target_desc"),
                            t(language, "shortcuts.value.wheel"),
                            Some(icons::SCROLL),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_target"),
                            "",
                            "Tab",
                            Some(icons::WINDOW),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.nudge_selection"),
                            t(language, "shortcuts.nudge_selection_desc"),
                            t(language, "shortcuts.value.arrow_keys"),
                            Some(icons::SELECT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_ratio"),
                            t(language, "shortcuts.toggle_ratio_desc"),
                            "R",
                            Some(icons::CROP),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_color_format"),
                            t(language, "shortcuts.toggle_color_format_desc"),
                            "Shift",
                            Some(icons::PALETTE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.copy_color"),
                            "",
                            "C",
                            Some(icons::CLIPBOARD),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.copy_selection"),
                            "",
                            t(language, "shortcuts.value.enter_or_double_click"),
                            Some(icons::COPY),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.save_capture"),
                            "",
                            t(language, "shortcuts.value.cmd_ctrl_s"),
                            Some(icons::SAVE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_current"),
                            "",
                            &pin_hotkey,
                            Some(icons::PIN),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_toolbar"),
                            "",
                            "Space",
                            Some(icons::MORE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.hints"),
                            "",
                            "F1",
                            Some(icons::SHORTCUTS),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.refresh"),
                            "",
                            "F5",
                            Some(icons::REFRESH),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.cancel"),
                            "",
                            "Esc",
                            Some(icons::CLOSE),
                        ),
                    ],
                ),
            ))
            .child(group(
                theme,
                t(language, "settings.group.pin_keys"),
                t(language, "settings.group.pin_keys_desc"),
                card(
                    theme,
                    vec![
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_move"),
                            t(language, "shortcuts.pin_move_desc"),
                            t(language, "shortcuts.value.drag"),
                            Some(icons::PIN),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_select_text"),
                            "",
                            t(language, "shortcuts.value.drag"),
                            Some(icons::TEXT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_word"),
                            "",
                            t(language, "shortcuts.value.double_click"),
                            Some(icons::TEXT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_copy_text"),
                            "",
                            t(language, "shortcuts.value.cmd_ctrl_c"),
                            Some(icons::COPY),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_select_all"),
                            "",
                            t(language, "shortcuts.value.cmd_ctrl_a"),
                            Some(icons::SELECT),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_zoom"),
                            "",
                            t(language, "shortcuts.value.wheel"),
                            Some(icons::IMAGE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_opacity"),
                            "",
                            t(language, "shortcuts.value.shift_wheel"),
                            Some(icons::PALETTE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_menu"),
                            "",
                            t(language, "shortcuts.value.right_click"),
                            Some(icons::MORE),
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.pin_close"),
                            "",
                            t(language, "shortcuts.value.esc_middle"),
                            Some(icons::CLOSE),
                        ),
                    ],
                ),
            ));
        page.when_some(crate::hotkey::registration_error(), |page, error| {
            page.child(note(
                theme,
                &format!("{} {error}", t(language, "shortcuts.unavailable")),
            ))
        })
    }

    fn shortcut_row(&self, slot: ShortcutSlot, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let recording = self.recording_shortcut == Some(slot);
        let display = if recording {
            t(language, "shortcuts.recording").to_owned()
        } else {
            crate::hotkey::display_hotkey(slot.value(&self.settings))
        };
        let accent = theme.accent;
        let selected = theme.selected;
        let control = theme.control;
        let control_hover = theme.control_hover;
        let border = theme.border;
        let text = theme.text;

        div().child(
            row_shell()
                .child(row_label_with_icon(
                    theme,
                    icons::SHORTCUTS,
                    t(language, slot.title_key()),
                    slot.description_key()
                        .map(|key| t(language, key))
                        .unwrap_or_default(),
                ))
                .child(
                    div()
                        .id(slot.id())
                        .h(px(layout::BUTTON_HEIGHT))
                        .px(px(12.0))
                        .rounded(px(layout::BUTTON_RADIUS))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .cursor_pointer()
                        .bg(if recording { selected } else { control })
                        .border_1()
                        .border_color(if recording { accent } else { border })
                        .text_size(px(layout::CONTROL_TEXT))
                        .text_color(if recording { accent } else { text })
                        .when(!recording, |pill| {
                            pill.hover(move |style| style.bg(control_hover))
                        })
                        .active(move |style| style.opacity(0.88))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.recording_shortcut = Some(slot);
                            this.status = None;
                            window.focus(&this.focus_handle, cx);
                            cx.notify();
                        }))
                        .child(display),
                ),
        )
    }

    fn handle_shortcut_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(slot) = self.recording_shortcut else {
            return false;
        };
        let language = self.settings.language;
        if event.keystroke.key == "escape" {
            self.recording_shortcut = None;
            self.status = None;
            cx.notify();
            return true;
        }

        let modifiers = &event.keystroke.modifiers;
        if !(modifiers.control || modifiers.alt || modifiers.platform) {
            let message = t(language, "shortcuts.need_modifier").to_owned();
            self.show_status(message, cx);
            return true;
        }
        let Some(stored) = crate::hotkey::shortcut_from_key(
            &event.keystroke.key,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
            modifiers.platform,
        ) else {
            let message = t(language, "shortcuts.invalid").to_owned();
            self.show_status(message, cx);
            return true;
        };

        let other = match slot {
            ShortcutSlot::Capture => &self.settings.pin_hotkey,
            ShortcutSlot::Pin => &self.settings.capture_hotkey,
        };
        if other == &stored {
            let message = t(language, "shortcuts.duplicate").to_owned();
            self.show_status(message, cx);
            return true;
        }

        let previous_capture = self.settings.capture_hotkey.clone();
        let previous_pin = self.settings.pin_hotkey.clone();
        match slot {
            ShortcutSlot::Capture => {
                self.settings.capture_hotkey = stored.clone();
                settings_state::update(|settings| settings.capture_hotkey = stored.clone());
            }
            ShortcutSlot::Pin => {
                self.settings.pin_hotkey = stored.clone();
                settings_state::update(|settings| settings.pin_hotkey = stored.clone());
            }
        }

        match crate::hotkey::refresh() {
            Ok(()) => {
                let message = t(language, "shortcuts.updated").to_owned();
                self.show_status(message, cx);
                self.recording_shortcut = None;
            }
            Err(error) => {
                self.settings.capture_hotkey = previous_capture.clone();
                self.settings.pin_hotkey = previous_pin.clone();
                settings_state::update(|settings| {
                    settings.capture_hotkey = previous_capture;
                    settings.pin_hotkey = previous_pin;
                });
                let _ = crate::hotkey::refresh();
                let message = format!("{}{}", t(language, "shortcuts.failed"), error);
                self.show_status(message, cx);
            }
        }
        cx.notify();
        true
    }

    fn open_project(&self, cx: &mut Context<Self>) {
        cx.open_url(PROJECT_URL);
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.settings.record_cursor_highlight {
            self.record_color_picker = None;
        }
        let theme = Theme::for_window(window, self.accent);
        let status = self.status.clone();
        #[cfg(target_os = "windows")]
        let titlebar = Some(self.titlebar(&theme, window, cx));
        #[cfg(not(target_os = "windows"))]
        let titlebar: Option<Div> = None;
        let color_backdrop = self.record_color_picker.as_ref().map(|_| {
            div()
                .id("settings-record-color-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.record_color_picker = None;
                        cx.notify();
                    }),
                )
        });
        let select_backdrop = self.select_menu.map(|_| {
            div()
                .id("settings-select-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.select_menu = None;
                        cx.notify();
                    }),
                )
        });
        let select_popup = self.select_popup(&theme, window, cx);
        let record_color_popup = self.record_color_popup(&theme, window, cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .when_some(theme.page, |root, page| root.bg(page))
            .text_color(theme.text)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let command =
                    event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
                if this.record_color_picker.is_some() && this.handle_record_color_key(event, cx) {
                    return;
                }
                if this.select_menu.is_some() && event.keystroke.key == "escape" {
                    this.select_menu = None;
                    cx.notify();
                    return;
                }
                // Cmd/Ctrl+W and Cmd/Ctrl+Q close only the settings window;
                // quitting the menu-bar app stays on the tray menu.
                if command && matches!(event.keystroke.key.as_str(), "w" | "q") {
                    platform_window::set_macos_accessory_activation_policy();
                    let input = this.filename_input.clone();
                    this.commit_filename_template(&input, window, cx);
                    window.remove_window();
                    return;
                }
                this.handle_shortcut_key(event, window, cx);
            }))
            .child(self.sidebar(&theme, cx))
            .child(self.content(&theme, window, cx))
            .children(titlebar)
            .children(color_backdrop)
            .children(select_backdrop)
            .children(select_popup)
            .children(record_color_popup)
            .children(status.map(|message| {
                div()
                    .absolute()
                    .bottom(px(16.0))
                    .right(px(16.0))
                    .max_w(px(480.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(layout::BUTTON_RADIUS))
                    .bg(rgba(0x000000CC))
                    .text_size(px(layout::CONTROL_TEXT))
                    .text_color(rgb(0xFFFFFF))
                    .child(message)
            }))
    }
}

pub fn open_settings(cx: &mut App) {
    let bounds = settings_bounds(cx);
    let display_id = cx.primary_display().map(|display| display.id());
    let language = settings_state::get().language;
    // The settings page is a real app window: show the Dock tile while it is
    // open so the user can find Capin in Mission Control and the app switcher.
    platform_window::set_macos_regular_activation_policy();
    let titlebar = custom_titlebar().then(|| TitlebarOptions {
        title: Some(t(language, "settings.title").into()),
        appears_transparent: true,
        traffic_light_position: Some(point(px(18.0), px(10.0))),
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar,
        app_owns_titlebar_drag: cfg!(target_os = "macos"),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        display_id,
        window_background: settings_background(),
        window_min_size: Some(size(px(layout::MIN_WIDTH), px(layout::MIN_HEIGHT))),
        app_id: Some("capin-settings".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, |window, cx| {
        let view = cx.new(|cx| SettingsView::new(window, cx));
        let close_view = view.clone();
        window.on_window_should_close(cx, move |window, cx| {
            close_view.update(cx, |this, cx| {
                let input = this.filename_input.clone();
                this.commit_filename_template(&input, window, cx);
            });
            let handle = window.window_handle();
            SETTINGS_WINDOWS.with(|handles| {
                handles
                    .borrow_mut()
                    .retain(|open| open.window_id() != handle.window_id());
            });
            // Closing the last real window returns Capin to its menu-bar
            // utility role (no Dock icon, no app-switcher entry).
            if !has_open_settings(cx) {
                platform_window::set_macos_accessory_activation_policy();
            }
            true
        });
        let focus = view.read(cx).focus_handle.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, _)) => {
            SETTINGS_WINDOWS.with(|handles| handles.borrow_mut().push(handle));
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
        }
        Err(error) => log_error!("settings", "failed to open settings: {error}"),
    }
}

/// A tray or hotkey capture attempt that dies on macOS TCC would otherwise
/// be a silent no-op. Surface the reason in the settings window instead of
/// leaving the user with no visible response.
#[cfg(target_os = "macos")]
pub fn report_capture_permission_denied(cx: &mut App) {
    let language = settings_state::get().language;
    let message = t(language, "settings.status_capture_permission").to_owned();
    PENDING_STATUS.with(|slot| *slot.borrow_mut() = Some(message));
    open_settings(cx);
}

fn settings_bounds(cx: &App) -> Bounds<Pixels> {
    crate::ui::centered_bounds(cx, layout::WINDOW_WIDTH, layout::WINDOW_HEIGHT)
}

fn settings_background() -> WindowBackgroundAppearance {
    #[cfg(target_os = "windows")]
    {
        WindowBackgroundAppearance::MicaBackdrop
    }
    #[cfg(target_os = "macos")]
    {
        WindowBackgroundAppearance::Blurred
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        WindowBackgroundAppearance::Transparent
    }
}

#[cfg(target_os = "windows")]
fn window_control_button(
    theme: &Theme,
    id: &'static str,
    icon: &'static [u8],
    area: WindowControlArea,
    close: bool,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(layout::CAPTION_BUTTON_WIDTH))
        .h(px(layout::CAPTION_BUTTON_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .when(close, |button| {
            button
                .group("settings-window-close")
                .hover(move |style| style.bg(theme.close_hover))
        })
        .when(!close, |button| {
            button.hover(move |style| style.bg(theme.selected))
        })
        .child(
            svg()
                .data(icon)
                .w(px(14.0))
                .h(px(14.0))
                .text_color(theme.text)
                .when(close, |glyph| {
                    glyph.group_hover("settings-window-close", |style| {
                        style.text_color(rgb(0xFFFFFF))
                    })
                }),
        )
}

fn card(theme: &Theme, rows: Vec<Div>) -> Div {
    let mut children: Vec<AnyElement> = Vec::with_capacity(rows.len() * 2);
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            children.push(card_divider(theme).into_any_element());
        }
        children.push(row.into_any_element());
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(layout::CARD_RADIUS))
        .bg(theme.card)
        .border_1()
        .border_color(theme.card_border)
        .overflow_hidden()
        .children(children)
}

/// Grouped-list separator between rows. The hairline is one physical pixel;
/// it starts at the row text inset like native settings lists, and the last
/// row in a card has no trailing divider.
fn card_divider(theme: &Theme) -> Div {
    div()
        .w_full()
        .pl(px(layout::ROW_PAD_X))
        .child(div().w_full().h(px(theme.hairline)).bg(theme.border))
}

fn group_header(theme: &Theme, title: &str, description: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(layout::SECTION_TITLE))
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .when(!description.is_empty(), |header| {
            header.child(
                div()
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .child(description.to_owned()),
            )
        })
}

/// A group header and its card at the tight native label gap.
fn group(theme: &Theme, title: &str, description: &str, body: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(layout::GROUP_HEADER_GAP))
        .child(group_header(theme, title, description))
        .child(body)
}

fn note(theme: &Theme, text: &str) -> Div {
    div()
        .text_size(px(layout::CAPTION))
        .text_color(theme.muted)
        .child(text.to_owned())
}

fn row_shell() -> Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .px(px(layout::ROW_PAD_X))
        .py(px(layout::ROW_PAD_Y))
}

/// Label block for a horizontal settings row. It takes the remaining width so
/// long descriptions wrap instead of squeezing the control.
fn row_label(theme: &Theme, title: &str, subtitle: &str) -> Div {
    label_block(theme, title, subtitle).flex_1().min_w(px(0.0))
}

/// Same label block with a leading monochrome glyph, matching the icon column
/// used by the system settings lists.
fn row_label_with_icon(
    theme: &Theme,
    icon_data: &'static [u8],
    title: &str,
    subtitle: &str,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .flex_1()
        .min_w(px(0.0))
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(icons::icon(icon_data, 16.0, theme.muted)),
        )
        .child(label_block(theme, title, subtitle).flex_1().min_w(px(0.0)))
}

/// Label block for stacked layouts that manage their own width.
fn label_block(theme: &Theme, title: &str, subtitle: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(layout::BODY))
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .when(!subtitle.is_empty(), |block| {
            block.child(
                div()
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .child(subtitle.to_owned()),
            )
        })
}

fn info_row(
    theme: &Theme,
    title: &str,
    subtitle: &str,
    value: &str,
    icon: Option<&'static [u8]>,
) -> Div {
    let value_side = div()
        .flex()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(layout::CONTROL_TEXT))
                .text_color(theme.muted)
                .child(value.to_owned()),
        );

    let label = match icon {
        Some(icon) => row_label_with_icon(theme, icon, title, subtitle),
        None => row_label(theme, title, subtitle),
    };
    row_shell().child(label).child(value_side)
}

fn toggle(theme: &Theme, value: bool, id: &'static str) -> impl IntoElement {
    // Native-style 36x20 switch: a 16px white thumb travels 16px with a 2px
    // inset. The thumb and the accent wash are driven by one spring, so an
    // interrupted click reverses from where the thumb actually is.
    let accent = theme.accent;
    let off = theme.toggle_off;
    let knob = theme.knob;
    let spring = SpringAnimation::new(SpringConfig::new(700.0, 46.0, 1.0)).to(if value {
        1.0f32
    } else {
        0.0
    });
    div()
        .relative()
        .w(px(36.0))
        .h(px(20.0))
        .flex_shrink_0()
        .rounded(px(10.0))
        .bg(off)
        .with_spring(
            SharedString::from(format!("settings-toggle-{id}")),
            spring,
            move |track, progress| {
                track
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(9.0))
                            .bg(accent)
                            .opacity(progress.clamp(0.0, 1.0)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(2.0 + progress.clamp(0.0, 1.0) * 16.0))
                            .top(px(2.0))
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(px(8.0))
                            .bg(knob),
                    )
            },
        )
}

fn input_editor_style(theme: &Theme) -> InputEditorStyle {
    InputEditorStyle {
        foreground: theme.text.into(),
        muted_foreground: theme.muted.into(),
        selection: Hsla::from(theme.accent).opacity(0.35),
        caret: theme.accent.into(),
        ..Default::default()
    }
}

fn background_label_key(background: WindowBackground) -> &'static str {
    match background {
        WindowBackground::Transparent => "window_background.transparent",
        WindowBackground::System => "window_background.system",
        WindowBackground::Light => "window_background.light",
        WindowBackground::Dark => "window_background.dark",
    }
}

fn padding_key(padding: u32) -> &'static str {
    match padding {
        24 => "padding.24",
        48 => "padding.48",
        _ => "padding.96",
    }
}

fn repeat_pin_key(behavior: RepeatPinBehavior) -> &'static str {
    match behavior {
        RepeatPinBehavior::RestoreHistory => "settings.repeat_pin_history",
        RepeatPinBehavior::FlashCurrent => "settings.repeat_pin_flash",
    }
}

fn action_button(theme: &Theme, label: String) -> Div {
    let hover = theme.control_hover;
    div()
        .h(px(layout::SEGMENT_HEIGHT))
        .px(px(8.0))
        .rounded(px(layout::BUTTON_RADIUS))
        .flex_shrink_0()
        .bg(theme.control)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .text_size(px(layout::CONTROL_TEXT))
        .text_color(theme.text)
        .hover(move |style| style.bg(hover))
        .child(label)
}

/// Flattened option buttons following the system Settings style: each choice is its own
/// control, and the selected tile carries the system accent instead of a
/// nested segmented track.
fn segmented() -> Div {
    div().flex().flex_row().flex_shrink_0().gap(px(4.0))
}

fn segment(theme: &Theme, label: &str, selected: bool) -> Div {
    let hover = theme.control_hover;
    div()
        .h(px(layout::SEGMENT_HEIGHT))
        .px(px(8.0))
        .rounded(px(layout::BUTTON_RADIUS))
        .flex()
        .flex_shrink_0()
        .items_center()
        .text_size(px(layout::CONTROL_TEXT))
        .text_color(if selected {
            theme.accent_text
        } else {
            theme.text
        })
        .bg(if selected {
            theme.accent
        } else {
            theme.control
        })
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .when(!selected, |button| {
            button.hover(move |style| style.bg(hover))
        })
        .child(label.to_owned())
}
