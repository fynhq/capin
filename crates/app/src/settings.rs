use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use core_record::FrameRate;
use core_types::RecordFormat;
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{AutostartState, set_autostart, system_accent_hex};

use crate::i18n::{Language, t};
use crate::icons;
use crate::settings_state::{self, AppSettings, WindowBackground};
use crate::storage;

const PROJECT_URL: &str = "https://github.com/fynhq/capin";

thread_local! {
    /// Status handed to the next settings window when a tray or hotkey action
    /// fails before it can open its own surface (for example a denied screen
    /// recording permission), so the click never ends as a silent no-op.
    static PENDING_STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
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
    pub const WINDOW_WIDTH: f32 = 1000.0;
    pub const WINDOW_HEIGHT: f32 = 700.0;
    pub const MIN_WIDTH: f32 = 820.0;
    pub const MIN_HEIGHT: f32 = 560.0;
    pub const TITLEBAR_HEIGHT: f32 = 48.0;
    pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;
    pub const CAPTION_BUTTON_HEIGHT: f32 = 32.0;
    pub const SIDEBAR_WIDTH: f32 = 240.0;
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
    pub const CONTENT_PAD_TOP: f32 = 20.0;
    pub const CONTENT_PAD_BOTTOM: f32 = 32.0;
    pub const CONTENT_GAP: f32 = 24.0;
    pub const GROUP_GAP: f32 = 28.0;
    pub const GROUP_HEADER_GAP: f32 = 8.0;
    pub const PAGE_TITLE: f32 = 28.0;
    pub const SECTION_TITLE: f32 = 14.0;
    pub const BODY: f32 = 14.0;
    pub const CAPTION: f32 = 12.0;
    pub const CARD_RADIUS: f32 = 4.0;
    pub const ROW_PAD_X: f32 = 16.0;
    pub const ROW_PAD_Y: f32 = 14.0;
    pub const BUTTON_HEIGHT: f32 = 32.0;
    pub const BUTTON_RADIUS: f32 = 4.0;
    pub const CONTROL_TEXT: f32 = 14.0;
}

#[cfg(target_os = "macos")]
mod layout {
    pub const WINDOW_WIDTH: f32 = 920.0;
    pub const WINDOW_HEIGHT: f32 = 640.0;
    pub const MIN_WIDTH: f32 = 760.0;
    pub const MIN_HEIGHT: f32 = 520.0;
    pub const TITLEBAR_HEIGHT: f32 = 52.0;
    pub const SIDEBAR_WIDTH: f32 = 200.0;
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
    pub const CONTENT_PAD_TOP: f32 = 24.0;
    pub const CONTENT_PAD_BOTTOM: f32 = 24.0;
    pub const CONTENT_GAP: f32 = 20.0;
    pub const GROUP_GAP: f32 = 22.0;
    pub const GROUP_HEADER_GAP: f32 = 6.0;
    pub const PAGE_TITLE: f32 = 22.0;
    pub const SECTION_TITLE: f32 = 13.0;
    pub const BODY: f32 = 13.0;
    pub const CAPTION: f32 = 11.0;
    pub const CARD_RADIUS: f32 = 10.0;
    pub const ROW_PAD_X: f32 = 14.0;
    pub const ROW_PAD_Y: f32 = 12.0;
    pub const BUTTON_HEIGHT: f32 = 28.0;
    pub const BUTTON_RADIUS: f32 = 6.0;
    pub const CONTROL_TEXT: f32 = 13.0;
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod layout {
    pub const WINDOW_WIDTH: f32 = 920.0;
    pub const WINDOW_HEIGHT: f32 = 640.0;
    pub const MIN_WIDTH: f32 = 760.0;
    pub const MIN_HEIGHT: f32 = 520.0;
    pub const TITLEBAR_HEIGHT: f32 = 0.0;
    pub const SIDEBAR_WIDTH: f32 = 200.0;
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
    pub const CONTENT_PAD_TOP: f32 = 16.0;
    pub const CONTENT_PAD_BOTTOM: f32 = 24.0;
    pub const CONTENT_GAP: f32 = 20.0;
    pub const GROUP_GAP: f32 = 22.0;
    pub const GROUP_HEADER_GAP: f32 = 6.0;
    pub const PAGE_TITLE: f32 = 22.0;
    pub const SECTION_TITLE: f32 = 13.0;
    pub const BODY: f32 = 13.0;
    pub const CAPTION: f32 = 11.0;
    pub const CARD_RADIUS: f32 = 10.0;
    pub const ROW_PAD_X: f32 = 14.0;
    pub const ROW_PAD_Y: f32 = 12.0;
    pub const BUTTON_HEIGHT: f32 = 28.0;
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

    fn description(self, language: Language) -> &'static str {
        t(
            language,
            match self {
                Self::General => "section.general.desc",
                Self::Capture => "section.capture.desc",
                Self::Recording => "section.recording.desc",
                Self::Shortcuts => "section.shortcuts.desc",
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

    fn description_key(self) -> &'static str {
        match self {
            Self::Capture => "settings.capture_hotkey_desc",
            Self::Pin => "settings.pin_hotkey_desc",
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

    fn description_key(self) -> &'static str {
        match self {
            Self::Screenshots => "settings.save_location_desc",
            Self::Recordings => "settings.recording_location_desc",
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
    title_key: &'static str,
    description_key: &'static str,
    value: bool,
    set: fn(&mut AppSettings, bool),
    refresh_pin: bool,
}

/// Popup menus replace segmented controls once a choice set grows past three
/// options. The anchor is the trigger's window-space bounds captured during
/// paint; the menu itself renders at the window root so it is not clipped by
/// the card or the scrolling page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectKind {
    WindowBackground,
    WindowPadding,
}

#[derive(Clone, Copy)]
struct SelectMenu {
    kind: SelectKind,
    anchor: Bounds<Pixels>,
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
    hover: Rgba,
    selected: Rgba,
    control: Rgba,
    control_hover: Rgba,
    toggle_off: Rgba,
    knob: Rgba,
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
            hover: palette.hover,
            selected: palette.selected,
            control: palette.control,
            control_hover: palette.control_hover,
            toggle_off: palette.toggle_off,
            knob: rgb(0xFFFFFF),
            #[cfg(target_os = "windows")]
            close_hover: rgb(0xC42B1C),
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

#[derive(Clone, Debug)]
struct TemplateEdit {
    content: String,
    cursor: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
}

impl TemplateEdit {
    fn new(content: String) -> Self {
        let end = content.len();
        Self {
            content,
            cursor: end,
            anchor: end,
            marked: None,
        }
    }

    fn cursor_utf8(&self) -> usize {
        self.cursor.min(self.content.len())
    }

    fn selection(&self) -> Range<usize> {
        let cursor = self.cursor_utf8();
        let anchor = self.anchor.min(self.content.len());
        if anchor <= cursor {
            anchor..cursor
        } else {
            cursor..anchor
        }
    }

    fn move_cursor(&mut self, cursor: usize, extend_selection: bool) {
        let cursor = cursor.min(self.content.len());
        let cursor = if self.content.is_char_boundary(cursor) {
            cursor
        } else {
            previous_char_boundary(&self.content, cursor)
        };
        self.cursor = cursor;
        if !extend_selection {
            self.anchor = cursor;
        }
    }

    fn previous_boundary(&self) -> usize {
        previous_char_boundary(&self.content, self.cursor_utf8())
    }

    fn next_boundary(&self) -> usize {
        next_char_boundary(&self.content, self.cursor_utf8())
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = offset.min(self.content.len());
        self.content[..offset]
            .chars()
            .map(|ch| ch.len_utf16())
            .sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_from_utf16(range.start);
        let end = self.offset_from_utf16(range.end).max(start);
        start..end
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_to_utf16(range.start);
        let end = self.offset_to_utf16(range.end).max(start);
        start..end
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str) {
        let selection = self.selection();
        let range = range
            .or_else(|| self.marked.take())
            .or_else(|| (!selection.is_empty()).then_some(selection))
            .unwrap_or(self.cursor_utf8()..self.cursor_utf8());
        let text: String = text
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect();
        self.content.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.marked = (!text.is_empty()).then_some(range.start..range.start + text.len());
    }
}

fn previous_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = cursor.min(content.len());
    content[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(content: &str, cursor: usize) -> usize {
    let cursor = cursor.min(content.len());
    content[cursor..]
        .chars()
        .next()
        .map(|ch| cursor + ch.len_utf8())
        .unwrap_or(cursor)
}

struct SettingsView {
    section: SettingsSection,
    settings: AppSettings,
    accent: Option<u32>,
    last_accent: Option<u32>,
    status: Option<String>,
    recording_shortcut: Option<ShortcutSlot>,
    filename_edit: Option<TemplateEdit>,
    filename_focus: FocusHandle,
    focus_handle: FocusHandle,
    #[cfg(target_os = "macos")]
    titlebar_drag: bool,
    select_menu: Option<SelectMenu>,
    logo: Option<Arc<RenderImage>>,
    #[cfg(target_os = "macos")]
    permission_screen: bool,
    #[cfg(target_os = "macos")]
    permission_accessibility: bool,
}

impl SettingsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let view = Self {
            section: SettingsSection::General,
            settings: settings_state::get(),
            accent: system_accent_hex(),
            last_accent: None,
            status: PENDING_STATUS.with(|slot| slot.borrow_mut().take()),
            recording_shortcut: None,
            filename_edit: None,
            filename_focus: cx.focus_handle(),
            focus_handle: cx.focus_handle(),
            #[cfg(target_os = "macos")]
            titlebar_drag: false,
            select_menu: None,
            logo: crate::image_util::render_png(include_bytes!(
                "../../../assets/icon/capin_icon_1024.png"
            )),
            #[cfg(target_os = "macos")]
            permission_screen: platform_window::screen_capture_trusted(),
            #[cfg(target_os = "macos")]
            permission_accessibility: platform_window::accessibility_trusted(),
        };
        view.start_accent_watch(cx);
        #[cfg(target_os = "macos")]
        view.start_permission_watch(cx);
        view
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
                .flex()
                .flex_row()
                .items_center()
                .gap(px(layout::NAV_GAP))
                .when(selected, |item| item.bg(selected_bg))
                .when(!selected, |item| item.hover(move |style| style.bg(hover)))
                .id(section.id())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    this.select_menu = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    section.icon(),
                    layout::NAV_ICON,
                    if selected { accent } else { muted },
                ))
                .child(
                    div()
                        .text_size(px(layout::NAV_FONT))
                        .text_color(text)
                        .when(selected, |label| label.font_weight(FontWeight::SEMIBOLD))
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
                .child(div().w_full().mt(px(18.0)).child(nav));
        }
        // Windows keeps the system caption buttons and drag strip, so the
        // navigation starts below the 48pt caption as before.
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
        let mut title = div()
            .w_full()
            .h(px(24.0))
            .px(px(layout::NAV_ITEM_PAD_X))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0));
        if let Some(logo) = self.logo.clone() {
            title = title.child(img(logo).w(px(18.0)).h(px(18.0)));
        }
        title.child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
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
        div()
            .mt_auto()
            .w_full()
            .pt(px(16.0))
            .pb(px(2.0))
            .flex()
            .justify_center()
            .child(
                div()
                    .id("settings-version-link")
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(accent))
                    .on_click(cx.listener(|this, _, _, cx| this.open_project(cx)))
                    .child(format!("Capin v{}", env!("CARGO_PKG_VERSION"))),
            )
    }

    /// The Windows caption is drawn inside the client area so it shares the
    /// window backdrop with the page while keeping the system caption hit
    /// testing (drag, snap, double-click). macOS uses the native title bar;
    /// its drag surface lives in the sidebar.
    #[cfg(target_os = "windows")]
    fn titlebar(&self, theme: &Theme, window: &mut Window, _cx: &mut Context<Self>) -> Div {
        let mut brand = div().flex().flex_row().items_center().gap(px(8.0));
        if let Some(logo) = self.logo.clone() {
            brand = brand.child(img(logo).w(px(18.0)).h(px(18.0)));
        }
        brand = brand.child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(t(self.settings.language, "settings.title")),
        );
        let leading = div()
            .id("settings-titlebar-drag")
            .flex_1()
            .h_full()
            .pl(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .window_control_area(WindowControlArea::Drag)
            .child(brand);

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(layout::TITLEBAR_HEIGHT))
            .flex()
            .flex_row()
            .items_start()
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

    fn content(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // macOS draws the settings title in the sidebar, so the page reaches
        // the top edge; Windows keeps the caption strip clear of scrolling
        // content so rows never slide under the caption buttons.
        #[cfg(target_os = "macos")]
        let top = 0.0;
        #[cfg(not(target_os = "macos"))]
        let top = layout::TITLEBAR_HEIGHT;
        div().flex_1().h_full().flex().flex_col().pt(px(top)).child(
            div()
                .flex_1()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .px(px(layout::CONTENT_PAD_X))
                .pt(px(layout::CONTENT_PAD_TOP))
                .pb(px(layout::CONTENT_PAD_BOTTOM))
                .id("settings-content")
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .max_w(px(layout::CONTENT_MAX_WIDTH))
                        .flex()
                        .flex_col()
                        .gap(px(layout::CONTENT_GAP))
                        .child(self.section_header(theme))
                        .child(match self.section {
                            SettingsSection::General => self.general(theme, cx),
                            SettingsSection::Capture => self.capture_settings(theme, cx),
                            SettingsSection::Recording => self.recording(theme, cx),
                            SettingsSection::Shortcuts => self.shortcuts(theme, cx),
                        }),
                ),
        )
    }

    fn section_header(&self, theme: &Theme) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(layout::PAGE_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(self.section.title(self.settings.language)),
            )
            .child(
                div()
                    .text_size(px(layout::BODY))
                    .text_color(theme.muted)
                    .child(self.section.description(self.settings.language)),
            )
    }

    fn general(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        #[cfg(target_os = "macos")]
        let language = self.settings.language;
        let content = card(
            theme,
            vec![
                div().child(self.language_row(theme, cx)),
                div().child(self.auto_start_row(theme, cx)),
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
            t(language, "settings.group.permissions_desc"),
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
            .child(row_label(theme, title, description))
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
            control = control.child(segment(theme, label, selected).id(code).on_click(
                cx.listener(move |this, _, window, cx| {
                    this.settings.language = option;
                    settings_state::update(|settings| settings.language = option);
                    crate::tray::refresh_menu();
                    window.set_window_title(t(option, "settings.title"));
                    this.status = None;
                    cx.notify();
                }),
            ));
        }

        row_shell()
            .child(row_label(
                theme,
                t(language, "settings.language"),
                t(language, "settings.language_desc"),
            ))
            .child(control)
    }

    fn auto_start_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.auto_start;

        row_shell()
            .child(row_label(
                theme,
                t(language, "settings.auto_start"),
                t(language, "settings.auto_start_desc"),
            ))
            .child(
                div()
                    .id("settings-auto-start")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let language = this.settings.language;
                        let desired = !this.settings.auto_start;
                        match set_autostart(desired) {
                            Ok(AutostartState::Unsupported) => {
                                this.status = Some(
                                    t(language, "settings.status_autostart_failed").to_owned(),
                                );
                            }
                            Ok(state) => {
                                this.settings.auto_start = desired;
                                this.settings.auto_start_explicit = true;
                                settings_state::update(|settings| {
                                    settings.auto_start = desired;
                                    settings.auto_start_explicit = true;
                                });
                                this.status = Some(
                                    match state {
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
                                    }
                                    .to_owned(),
                                );
                            }
                            Err(error) => {
                                this.status = Some(format!(
                                    "{}{}",
                                    t(language, "settings.status_autostart_failed"),
                                    error
                                ));
                            }
                        }
                        cx.notify();
                    }))
                    .child(toggle(theme, value)),
            )
    }

    fn capture_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut window_rows = vec![self.toggle_row(
            theme,
            cx,
            ToggleRow {
                id: "settings-window-composition",
                title_key: "settings.window_composition",
                description_key: "settings.window_composition_desc",
                value: self.settings.window_composition,
                set: |settings, value| settings.window_composition = value,
                refresh_pin: false,
            },
        )];
        if self.settings.window_composition {
            window_rows.push(self.window_background_row(theme, cx));
            window_rows.push(self.window_padding_row(theme, cx));
            window_rows.push(self.toggle_row(
                theme,
                cx,
                ToggleRow {
                    id: "settings-window-shadow",
                    title_key: "settings.window_shadow",
                    description_key: "settings.window_shadow_desc",
                    value: self.settings.window_shadow,
                    set: |settings, value| settings.window_shadow = value,
                    refresh_pin: false,
                },
            ));
            if self.settings.window_background == WindowBackground::Wallpaper {
                window_rows.push(self.window_wallpaper_row(theme, cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_GAP))
            .child(group(
                theme,
                t(language, "settings.group.selection"),
                t(language, "settings.group.selection_desc"),
                card(
                    theme,
                    vec![
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-copy-to-clipboard",
                                title_key: "settings.copy_clipboard",
                                description_key: "settings.copy_clipboard_desc",
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
                                title_key: "settings.capture_cursor",
                                description_key: "settings.capture_cursor_desc",
                                value: self.settings.capture_cursor,
                                set: |settings, value| settings.capture_cursor = value,
                                refresh_pin: false,
                            },
                        ),
                        info_row(
                            theme,
                            t(language, "settings.capture_mode"),
                            t(language, "settings.capture_mode_desc"),
                            t(language, "settings.capture_mode_value"),
                            Some(icons::CAPTURE),
                        ),
                        self.save_location_row(SaveTarget::Screenshots, theme, cx),
                        self.filename_template_row(theme, cx),
                    ],
                ),
            ))
            .child(group(
                theme,
                t(language, "settings.group.annotation"),
                t(language, "settings.group.annotation_desc"),
                card(
                    theme,
                    vec![self.toggle_row(
                        theme,
                        cx,
                        ToggleRow {
                            id: "settings-annotation-uniform-color",
                            title_key: "settings.annotation_uniform_color",
                            description_key: "settings.annotation_uniform_color_desc",
                            value: self.settings.annotation_uniform_color,
                            set: |settings, value| settings.annotation_uniform_color = value,
                            refresh_pin: false,
                        },
                    )],
                ),
            ))
            .child(group(
                theme,
                t(language, "settings.group.window"),
                t(language, "settings.group.window_desc"),
                card(theme, window_rows),
            ))
            .child(group(
                theme,
                t(language, "settings.group.pin"),
                t(language, "settings.group.pin_desc"),
                card(
                    theme,
                    vec![
                        self.toggle_row(
                            theme,
                            cx,
                            ToggleRow {
                                id: "settings-pin-text-ocr",
                                title_key: "settings.pin_text_ocr",
                                description_key: "settings.pin_text_ocr_desc",
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
                                title_key: "settings.pin_qr",
                                description_key: "settings.pin_qr_desc",
                                value: self.settings.pin_qr,
                                set: |settings, value| settings.pin_qr = value,
                                refresh_pin: true,
                            },
                        ),
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
                .child(row_label(
                    theme,
                    t(language, row.title_key),
                    t(language, row.description_key),
                ))
                .child(
                    div()
                        .id(row.id)
                        .cursor_pointer()
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
                        .child(toggle(theme, row.value)),
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
            .child(row_label(
                theme,
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
        let current = t(language, padding_key(self.settings.window_padding));
        row_shell()
            .child(row_label(
                theme,
                t(language, "settings.window_padding"),
                t(language, "settings.window_padding_desc"),
            ))
            .child(self.select_trigger(
                theme,
                current,
                "settings-window-padding",
                SelectKind::WindowPadding,
                cx,
            ))
    }

    /// A macOS-style popup button: the current value on the left, a chevron
    /// on the right. The trigger records its window-space bounds while it
    /// paints so the popup can be placed below it at the window root.
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
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(anchor) = anchor.get() {
                    this.select_menu = Some(SelectMenu { kind, anchor });
                }
                cx.notify();
            }))
            .child(label.to_owned())
            .child(icons::icon(icons::CHEVRON_DOWN, 10.0, chevron))
            .child(probe)
    }

    /// The popup layer for the two long option lists. Rendering it at the
    /// window root keeps it above the card and outside the scrolling page.
    fn select_popup(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let menu = self.select_menu?;
        let language = self.settings.language;
        let mut entries: Vec<(String, u32, bool)> = Vec::new();
        match menu.kind {
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
                for padding in [0u32, 24, 48, 96] {
                    entries.push((
                        t(language, padding_key(padding)).to_owned(),
                        padding,
                        self.settings.window_padding == padding,
                    ));
                }
            }
        }

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
            .bg(theme.control)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(1.0))
            .occlude();
        for (index, (label, value, selected)) in entries.into_iter().enumerate() {
            let accent = theme.accent;
            let accent_text = theme.accent_text;
            let text = theme.text;
            let hover = theme.hover;
            popup = popup.child(
                div()
                    .id(("settings-select-item", index))
                    .h(px(item_height))
                    .px(px(8.0))
                    .rounded(px(4.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(layout::CONTROL_TEXT))
                    .when(selected, |item| item.bg(accent).text_color(accent_text))
                    .when(!selected, |item| {
                        item.text_color(text).hover(move |style| style.bg(hover))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match menu.kind {
                            SelectKind::WindowBackground => {
                                let background = WindowBackground::ALL[value as usize];
                                this.settings.window_background = background;
                                settings_state::update(|settings| {
                                    settings.window_background = background
                                });
                            }
                            SelectKind::WindowPadding => {
                                this.settings.window_padding = value;
                                settings_state::update(|settings| settings.window_padding = value);
                            }
                        }
                        this.select_menu = None;
                        this.status = None;
                        cx.notify();
                    }))
                    .child(div().w(px(12.0)).flex_shrink_0().child(if selected {
                        "✓"
                    } else {
                        ""
                    }))
                    .child(label),
            );
        }
        Some(popup)
    }

    fn window_wallpaper_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let path = self
            .settings
            .window_wallpaper
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| t(language, "settings.system_wallpaper").to_owned());
        let choose = action_button(theme, t(language, "settings.choose_image").to_owned())
            .id("settings-wallpaper-choose")
            .on_click(cx.listener(|this, _, _, cx| this.choose_wallpaper(cx)));
        let clear = self.settings.window_wallpaper.as_ref().map(|_| {
            action_button(theme, t(language, "settings.clear_image").to_owned())
                .id("settings-wallpaper-clear")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.window_wallpaper = None;
                    settings_state::update(|settings| settings.window_wallpaper = None);
                    this.status = None;
                    cx.notify();
                }))
        });

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(layout::ROW_PAD_X))
            .py(px(layout::ROW_PAD_Y))
            .child(label_block(
                theme,
                t(language, "settings.window_wallpaper"),
                t(language, "settings.window_wallpaper_desc"),
            ))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(choose)
                    .children(clear),
            )
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .child(path),
            )
    }

    fn choose_wallpaper(&mut self, cx: &mut Context<Self>) {
        let language = self.settings.language;
        let prompt = t(language, "settings.choose_image").to_owned();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
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
                this.apply_wallpaper_choice(selected, error);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_wallpaper_choice(&mut self, selected: Option<PathBuf>, error: Option<String>) {
        let language = self.settings.language;
        if let Some(error) = error {
            self.status = Some(format!(
                "{} {}",
                t(language, "settings.status_picker_failed"),
                error
            ));
            return;
        }
        let Some(path) = selected else {
            return;
        };
        if image::open(&path).is_err() {
            self.status = Some(t(language, "settings.status_wallpaper_invalid").to_owned());
            return;
        }
        self.settings.window_wallpaper = Some(path.clone());
        settings_state::update(|settings| settings.window_wallpaper = Some(path));
        self.status = Some(t(language, "settings.status_wallpaper_updated").to_owned());
    }

    fn save_location_row(&self, target: SaveTarget, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let custom = target.selected_directory(&self.settings).is_some();
        let directory = target.directory(&self.settings).display().to_string();
        let open = action_button(theme, t(language, "settings.open_folder").to_owned())
            .id(target.open_id())
            .on_click(cx.listener(move |this, _, _, cx| this.open_directory(target, cx)));
        let change = action_button(theme, t(language, "settings.change_folder").to_owned())
            .id(target.change_id())
            .on_click(cx.listener(move |this, _, _, cx| this.choose_directory(target, cx)));
        let reset = custom.then(|| {
            action_button(theme, t(language, "settings.use_default_folder").to_owned())
                .id(target.default_id())
                .on_click(cx.listener(move |this, _, _, cx| this.reset_directory(target, cx)))
        });

        row_shell()
            .child(row_label(
                theme,
                t(language, target.title_key()),
                t(language, target.description_key()),
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

    fn filename_template_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let editing = self.filename_edit.is_some();
        let shown = self
            .filename_edit
            .as_ref()
            .map(|edit| {
                let cursor = edit.cursor_utf8();
                format!("{}|{}", &edit.content[..cursor], &edit.content[cursor..])
            })
            .unwrap_or_else(|| self.settings.filename_template.clone());
        let template = self
            .filename_edit
            .as_ref()
            .map(|edit| edit.content.as_str())
            .unwrap_or(self.settings.filename_template.as_str());
        let preview = crate::storage::render_filename(template, 1920, 1080);
        let action_label = t(
            language,
            if editing {
                "settings.done"
            } else {
                "settings.edit"
            },
        );
        let action = action_button(theme, action_label.to_owned())
            .id("settings-filename-template-action")
            .on_click(cx.listener(|this, _, window, cx| {
                if this.filename_edit.is_some() {
                    this.finish_filename_edit(window, cx);
                } else {
                    this.begin_filename_edit(window, cx);
                }
            }));

        let focus = self.filename_focus.clone();
        let entity = cx.entity();
        let input = editing.then(|| {
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    window.handle_input(
                        &focus,
                        ElementInputHandler::new(bounds, entity.clone()),
                        cx,
                    );
                },
            )
            .absolute()
            .inset_0()
            .into_any_element()
        });

        let field = div()
            .id("settings-filename-template-field")
            .track_focus(&self.filename_focus)
            .relative()
            .w(px(220.0))
            .h(px(layout::BUTTON_HEIGHT))
            .flex_shrink_0()
            .px(px(10.0))
            .rounded(px(layout::BUTTON_RADIUS))
            .bg(theme.control)
            .border_1()
            .border_color(if editing { theme.accent } else { theme.border })
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(layout::CONTROL_TEXT))
            .text_color(theme.text)
            .child(shown)
            .children(input)
            .on_click(cx.listener(|this, _, window, cx| {
                if this.filename_edit.is_some() {
                    window.focus(&this.filename_focus, cx);
                } else {
                    this.begin_filename_edit(window, cx);
                }
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
                    .child(row_label(
                        theme,
                        t(language, "settings.filename_template"),
                        t(language, "settings.filename_template_desc"),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(field)
                            .child(action),
                    ),
            )
            .child(
                div()
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .child(format!(
                        "{} {}",
                        t(language, "settings.filename_template_preview"),
                        preview
                    )),
            )
    }

    fn begin_filename_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filename_edit.is_none() {
            self.filename_edit = Some(TemplateEdit::new(self.settings.filename_template.clone()));
        }
        window.focus(&self.filename_focus, cx);
        self.status = None;
        cx.notify();
    }

    fn finish_filename_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.filename_edit.as_ref() else {
            return;
        };
        let template = edit.content.trim().to_owned();
        if template.is_empty() {
            self.status =
                Some(t(self.settings.language, "settings.filename_template_invalid").to_owned());
            window.focus(&self.filename_focus, cx);
            cx.notify();
            return;
        }
        self.settings.filename_template = template.clone();
        settings_state::update(|settings| settings.filename_template = template);
        self.filename_edit = None;
        self.status =
            Some(t(self.settings.language, "settings.filename_template_saved").to_owned());
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn cancel_filename_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filename_edit = None;
        window.focus(&self.focus_handle, cx);
        self.status = None;
        cx.notify();
    }

    fn persist_filename_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.filename_edit.as_ref() else {
            return;
        };
        let template = edit.content.trim();
        if template.is_empty() || template == self.settings.filename_template {
            return;
        }
        self.settings.filename_template = template.to_owned();
        settings_state::update(|settings| settings.filename_template = template.to_owned());
        cx.notify();
    }

    fn handle_filename_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.filename_edit.is_none() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        let extend = event.keystroke.modifiers.shift;
        match key {
            "escape" => {
                self.cancel_filename_edit(window, cx);
                return true;
            }
            "enter" => {
                self.finish_filename_edit(window, cx);
                return true;
            }
            "backspace" => {
                self.backspace_filename(cx);
                return true;
            }
            "delete" => {
                self.delete_filename(cx);
                return true;
            }
            "left" => {
                let position = self
                    .filename_edit
                    .as_ref()
                    .map(TemplateEdit::previous_boundary);
                if let Some(position) = position {
                    self.move_filename_cursor(position, extend, cx);
                }
                return true;
            }
            "right" => {
                let position = self.filename_edit.as_ref().map(TemplateEdit::next_boundary);
                if let Some(position) = position {
                    self.move_filename_cursor(position, extend, cx);
                }
                return true;
            }
            "home" => {
                self.move_filename_cursor(0, extend, cx);
                return true;
            }
            "end" => {
                let end = self.filename_edit.as_ref().map(|edit| edit.content.len());
                if let Some(end) = end {
                    self.move_filename_cursor(end, extend, cx);
                }
                return true;
            }
            _ => {}
        }
        if command && key == "a" {
            if let Some(edit) = &mut self.filename_edit {
                edit.anchor = 0;
                edit.cursor = edit.content.len();
                cx.notify();
            }
            return true;
        }
        if command && key == "c" {
            self.copy_filename_selection(cx);
            return true;
        }
        if command && key == "v" {
            self.paste_filename(cx);
            return true;
        }
        true
    }

    fn backspace_filename(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.filename_edit {
            let selection = edit.selection();
            let range = edit.marked.take().or_else(|| {
                if !selection.is_empty() {
                    Some(selection)
                } else {
                    let cursor = edit.cursor_utf8();
                    (cursor > 0).then(|| edit.previous_boundary()..cursor)
                }
            });
            if let Some(range) = range {
                edit.content.replace_range(range.clone(), "");
                edit.cursor = range.start;
                edit.anchor = range.start;
            }
        }
        self.persist_filename_edit(cx);
    }

    fn delete_filename(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.filename_edit {
            let selection = edit.selection();
            let range = if !selection.is_empty() {
                Some(selection)
            } else {
                let cursor = edit.cursor_utf8();
                (cursor < edit.content.len()).then(|| cursor..edit.next_boundary())
            };
            if let Some(range) = range {
                edit.content.replace_range(range.clone(), "");
                edit.cursor = range.start;
                edit.anchor = range.start;
            }
        }
        self.persist_filename_edit(cx);
    }

    fn move_filename_cursor(&mut self, cursor: usize, extend: bool, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.filename_edit {
            edit.move_cursor(cursor, extend);
            cx.notify();
        }
    }

    fn copy_filename_selection(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.filename_edit.as_ref() else {
            return;
        };
        let selection = edit.selection();
        if selection.is_empty() {
            return;
        }
        let text = edit.content[selection].to_owned();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn paste_filename(&mut self, cx: &mut Context<Self>) {
        let mut text = None;
        if let Some(item) = cx.read_from_clipboard() {
            for entry in item.entries() {
                if let ClipboardEntry::String(value) = entry {
                    text = Some(value.text.clone());
                    break;
                }
            }
        }
        if let Some(text) = text {
            if let Some(edit) = &mut self.filename_edit {
                edit.replace(None, &text);
            }
            self.persist_filename_edit(cx);
        }
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
                this.apply_directory_choice(target, selected, error);
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
    ) {
        let language = self.settings.language;
        if let Some(error) = error {
            self.status = Some(format!(
                "{} {}",
                t(language, "settings.status_picker_failed"),
                error
            ));
            return;
        }
        let Some(directory) = selected else {
            return;
        };
        if let Err(error) = storage::ensure_directory(&directory) {
            self.status = Some(format!(
                "{} {}",
                t(language, "settings.status_folder_failed"),
                error
            ));
            return;
        }
        let directory = Some(directory);
        target.set_directory(&mut self.settings, directory.clone());
        settings_state::update(|settings| target.set_directory(settings, directory));
        self.status = Some(t(language, "settings.status_folder_updated").to_owned());
    }

    fn reset_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings.language;
        target.set_directory(&mut self.settings, None);
        settings_state::update(|settings| target.set_directory(settings, None));
        self.status = Some(t(language, "settings.status_folder_default").to_owned());
        cx.notify();
    }

    fn open_directory(&mut self, target: SaveTarget, cx: &mut Context<Self>) {
        let language = self.settings.language;
        let directory = target.directory(&self.settings);
        if let Err(error) = storage::ensure_directory(&directory) {
            self.status = Some(format!(
                "{} {}",
                t(language, "settings.status_folder_failed"),
                error
            ));
        } else {
            cx.open_with_system(&directory);
        }
        cx.notify();
    }

    fn recording(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        div()
            .flex()
            .flex_col()
            .gap(px(layout::GROUP_HEADER_GAP))
            .child(note(theme, t(language, "settings.recording_note")))
            .child(card(
                theme,
                vec![
                    self.recording_format_row(theme, cx),
                    self.recording_frame_rate_row(theme, cx),
                    self.recording_quality_row(theme, cx),
                    self.toggle_row(
                        theme,
                        cx,
                        ToggleRow {
                            id: "settings-record-segments",
                            title_key: "settings.record_segments",
                            description_key: "settings.record_segments_desc",
                            value: self.settings.record_segments,
                            set: |settings, value| settings.record_segments = value,
                            refresh_pin: false,
                        },
                    ),
                    self.toggle_row(
                        theme,
                        cx,
                        ToggleRow {
                            id: "settings-record-cursor",
                            title_key: "settings.record_cursor",
                            description_key: "settings.record_cursor_desc",
                            value: self.settings.record_cursor,
                            set: |settings, value| settings.record_cursor = value,
                            refresh_pin: false,
                        },
                    ),
                    self.save_location_row(SaveTarget::Recordings, theme, cx),
                ],
            ))
    }

    fn recording_format_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut control = segmented();
        for (format, label) in [
            (RecordFormat::Mp4, t(language, "format.mp4")),
            (RecordFormat::WebP, t(language, "format.webp")),
        ] {
            let selected = self.settings.record_format == format;
            control = control.child(segment(theme, label, selected).id(label).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.settings.record_format = format;
                    let frame_rate = if format == RecordFormat::WebP {
                        FrameRate::Fps30
                    } else {
                        this.settings.record_frame_rate
                    };
                    this.settings.record_frame_rate = frame_rate;
                    settings_state::update(|settings| {
                        settings.record_format = format;
                        settings.record_frame_rate = frame_rate;
                    });
                    cx.notify();
                }),
            ));
        }

        row_shell()
            .child(row_label(
                theme,
                t(language, "settings.format"),
                t(language, "settings.format_desc"),
            ))
            .child(control)
    }

    fn recording_frame_rate_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut control = segmented();
        for (frame_rate, label) in [
            (FrameRate::Fps30, t(language, "fps.30")),
            (FrameRate::Fps60, t(language, "fps.60")),
        ] {
            let selected = self.settings.record_frame_rate == frame_rate;
            let disabled =
                self.settings.record_format == RecordFormat::WebP && frame_rate == FrameRate::Fps60;
            let button = if disabled {
                segment(theme, label, selected)
                    .opacity(0.4)
                    .into_any_element()
            } else {
                segment(theme, label, selected)
                    .id(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.record_frame_rate = frame_rate;
                        settings_state::update(|settings| {
                            settings.record_frame_rate = frame_rate;
                        });
                        cx.notify();
                    }))
                    .into_any_element()
            };
            control = control.child(button);
        }

        row_shell()
            .child(row_label(
                theme,
                t(language, "settings.frame_rate"),
                t(language, "settings.frame_rate_desc"),
            ))
            .child(control)
    }

    fn recording_quality_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut control = segmented();
        for quality in core_record::RecordQuality::ALL {
            let selected = self.settings.record_quality == quality;
            let label = t(
                language,
                match quality {
                    core_record::RecordQuality::Low => "quality.low",
                    core_record::RecordQuality::Standard => "quality.standard",
                    core_record::RecordQuality::High => "quality.high",
                },
            );
            control = control.child(
                segment(theme, label, selected)
                    .id(format!("record-quality-{}", quality.code()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.record_quality = quality;
                        settings_state::update(|settings| settings.record_quality = quality);
                        cx.notify();
                    })),
            );
        }

        row_shell()
            .child(row_label(
                theme,
                t(language, "settings.quality"),
                t(language, "settings.quality_desc"),
            ))
            .child(control)
    }

    fn shortcuts(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        div()
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
                            t(language, "shortcuts.capture_region_desc"),
                            t(language, "shortcuts.value.drag"),
                            None,
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.window_capture"),
                            t(language, "shortcuts.window_capture_desc"),
                            t(language, "shortcuts.value.auto"),
                            None,
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.adjust_target"),
                            t(language, "shortcuts.adjust_target_desc"),
                            t(language, "shortcuts.value.wheel"),
                            None,
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_ratio"),
                            t(language, "shortcuts.toggle_ratio_desc"),
                            "R",
                            None,
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.toggle_color_format"),
                            t(language, "shortcuts.toggle_color_format_desc"),
                            "Shift",
                            None,
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.copy_color"),
                            t(language, "shortcuts.copy_color_desc"),
                            "C",
                            None,
                        ),
                        info_row(
                            theme,
                            t(language, "shortcuts.cancel"),
                            t(language, "shortcuts.cancel_desc"),
                            "Esc",
                            None,
                        ),
                    ],
                ),
            ))
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
                .child(row_label(
                    theme,
                    t(language, slot.title_key()),
                    t(language, slot.description_key()),
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
            self.status = Some(t(language, "shortcuts.need_modifier").to_owned());
            cx.notify();
            return true;
        }
        let Some(stored) = crate::hotkey::shortcut_from_key(
            &event.keystroke.key,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
            modifiers.platform,
        ) else {
            self.status = Some(t(language, "shortcuts.invalid").to_owned());
            cx.notify();
            return true;
        };

        let other = match slot {
            ShortcutSlot::Capture => &self.settings.pin_hotkey,
            ShortcutSlot::Pin => &self.settings.capture_hotkey,
        };
        if other == &stored {
            self.status = Some(t(language, "shortcuts.duplicate").to_owned());
            cx.notify();
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
                self.status = Some(t(language, "shortcuts.updated").to_owned());
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
                self.status = Some(format!("{}{}", t(language, "shortcuts.failed"), error));
            }
        }
        cx.notify();
        true
    }

    fn open_project(&self, cx: &mut Context<Self>) {
        cx.open_url(PROJECT_URL);
    }
}

impl EntityInputHandler for SettingsView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let edit = self.filename_edit.as_ref()?;
        let range = edit.range_from_utf16(&range_utf16);
        actual_range.replace(edit.range_to_utf16(&range));
        Some(edit.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let edit = self.filename_edit.as_ref()?;
        let selection = edit.selection();
        Some(UTF16Selection {
            range: edit.range_to_utf16(&selection),
            reversed: edit.anchor > edit.cursor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let edit = self.filename_edit.as_ref()?;
        edit.marked.as_ref().map(|range| edit.range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.filename_edit {
            edit.marked = None;
            window.notify_text_input_changed(&self.filename_focus);
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = &mut self.filename_edit {
            let range = range_utf16
                .as_ref()
                .map(|range| edit.range_from_utf16(range));
            edit.marked = None;
            edit.replace(range, new_text);
            window.notify_text_input_changed(&self.filename_focus);
        }
        self.persist_filename_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = &mut self.filename_edit {
            let range = range_utf16
                .as_ref()
                .map(|range| edit.range_from_utf16(range));
            edit.replace(range, new_text);
            window.notify_text_input_changed(&self.filename_focus);
        }
        self.persist_filename_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.filename_edit.as_ref()?;
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.filename_edit.as_ref()?;
        Some(edit.offset_to_utf16(edit.content.len()))
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.filename_edit.as_ref()?;
        Some(edit.offset_to_utf16(edit.content.len()))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_window(window, self.accent);
        let status = self.status.clone();
        #[cfg(target_os = "windows")]
        let titlebar = Some(self.titlebar(&theme, window, cx));
        #[cfg(not(target_os = "windows"))]
        let titlebar: Option<Div> = None;
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
                if this.select_menu.is_some() && event.keystroke.key == "escape" {
                    this.select_menu = None;
                    cx.notify();
                    return;
                }
                // Cmd/Ctrl+W and Cmd/Ctrl+Q close only the settings window;
                // quitting the menu-bar app stays on the tray menu.
                if command && matches!(event.keystroke.key.as_str(), "w" | "q") {
                    platform_window::set_macos_accessory_activation_policy();
                    window.remove_window();
                    return;
                }
                if this.handle_filename_key(event, window, cx) {
                    return;
                }
                this.handle_shortcut_key(event, window, cx);
            }))
            .child(self.sidebar(&theme, cx))
            .child(self.content(&theme, cx))
            .children(titlebar)
            .children(select_backdrop)
            .children(select_popup)
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
        traffic_light_position: Some(point(px(18.0), px(20.0))),
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
        window.on_window_should_close(cx, |_, _| {
            // Closing the last real window returns Capin to its menu-bar
            // utility role (no Dock icon, no app-switcher entry).
            platform_window::set_macos_accessory_activation_policy();
            true
        });
        let view = cx.new(SettingsView::new);
        let focus = view.read(cx).focus_handle.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, _)) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
        }
        Err(error) => eprintln!("capin: failed to open settings: {error}"),
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
            button.hover(move |style| style.bg(theme.hover))
        })
        .child(
            svg()
                .data(icon)
                .w(px(10.0))
                .h(px(10.0))
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
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(px(layout::CAPTION))
                .text_color(theme.muted)
                .child(description.to_owned()),
        )
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
        .child(
            div()
                .text_size(px(layout::CAPTION))
                .text_color(theme.muted)
                .child(subtitle.to_owned()),
        )
}

fn info_row(
    theme: &Theme,
    title: &str,
    subtitle: &str,
    value: &str,
    icon: Option<&'static [u8]>,
) -> Div {
    let mut value_side = div()
        .flex()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(6.0));
    if let Some(icon_data) = icon {
        value_side = value_side.child(icons::icon(icon_data, 15.0, theme.muted));
    }
    value_side = value_side.child(
        div()
            .text_size(px(layout::CONTROL_TEXT))
            .text_color(theme.muted)
            .child(value.to_owned()),
    );

    row_shell()
        .child(row_label(theme, title, subtitle))
        .child(value_side)
}

fn toggle(theme: &Theme, value: bool) -> Div {
    // Native-style 36x20 switch used on both platforms: a 16px white thumb,
    // 1px transparent border and 2px travel. The unchecked track is a neutral
    // translucent fill.
    let accent = theme.accent;
    let off = theme.toggle_off;
    let knob = theme.knob;
    div()
        .w(px(36.0))
        .h(px(20.0))
        .flex_shrink_0()
        .rounded(px(10.0))
        .border_1()
        .border_color(rgba(0x00000000))
        .p(px(1.0))
        .bg(if value { accent } else { off })
        .flex()
        .items_center()
        .when(value, |toggle| toggle.justify_end())
        .when(!value, |toggle| toggle.justify_start())
        .child(div().w(px(16.0)).h(px(16.0)).rounded(px(8.0)).bg(knob))
}

fn background_label_key(background: WindowBackground) -> &'static str {
    match background {
        WindowBackground::Transparent => "window_background.transparent",
        WindowBackground::Light => "window_background.light",
        WindowBackground::Dark => "window_background.dark",
        WindowBackground::Gradient => "window_background.gradient",
        WindowBackground::Wallpaper => "window_background.wallpaper",
    }
}

fn padding_key(padding: u32) -> &'static str {
    match padding {
        0 => "padding.0",
        24 => "padding.24",
        48 => "padding.48",
        _ => "padding.96",
    }
}

fn action_button(theme: &Theme, label: String) -> Div {
    let hover = theme.control_hover;
    div()
        .h(px(layout::BUTTON_HEIGHT))
        .px(px(12.0))
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
        .h(px(layout::BUTTON_HEIGHT))
        .px(px(12.0))
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
