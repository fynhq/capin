use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use core_record::FrameRate;
use core_types::{RecordFormat, prefers_dark_foreground};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{AutostartState, set_autostart, system_accent_hex};

use crate::i18n::{Language, t};
use crate::icons;
use crate::settings_state::{self, AppSettings, WindowBackground};
use crate::storage;

const PROJECT_URL: &str = "https://github.com/fynhq/capin";

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsSection {
    General,
    Capture,
    Recording,
    Shortcuts,
    About,
}

impl SettingsSection {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Capture,
        Self::Recording,
        Self::Shortcuts,
        Self::About,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::General => "settings-nav-general",
            Self::Capture => "settings-nav-capture",
            Self::Recording => "settings-nav-recording",
            Self::Shortcuts => "settings-nav-shortcuts",
            Self::About => "settings-nav-about",
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
                Self::About => "nav.about",
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
                Self::About => "section.about.desc",
            },
        )
    }

    fn icon(self) -> &'static [u8] {
        match self {
            Self::General => icons::GENERAL,
            Self::Capture => icons::CAPTURE,
            Self::Recording => icons::RECORDING,
            Self::Shortcuts => icons::SHORTCUTS,
            Self::About => icons::ABOUT,
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
struct Theme {
    text: Rgba,
    muted: Rgba,
    sidebar: Rgba,
    card: Rgba,
    border: Rgba,
    accent: Rgba,
    hover: Rgba,
    selected: Rgba,
    control: Rgba,
    toggle_off: Rgba,
    knob: Rgba,
    accent_text: Rgba,
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
        if dark {
            Self::dark(accent_hex)
        } else {
            Self::light(accent_hex)
        }
    }

    fn light(accent_hex: u32) -> Self {
        Self {
            text: rgb(0x1A1A1A),
            muted: rgb(0x666666),
            sidebar: sidebar_color(false),
            card: card_color(false),
            border: rgba(0x0000001A),
            accent: rgb(accent_hex),
            hover: rgba(0x0000000D),
            selected: rgba((accent_hex << 8) | 0x33),
            control: rgba(0x0000000F),
            toggle_off: rgba(0x00000026),
            knob: rgb(0xFFFFFF),
            accent_text: accent_text_color(accent_hex),
        }
    }

    fn dark(accent_hex: u32) -> Self {
        Self {
            text: rgb(0xF2F2F2),
            muted: rgb(0xA0A0A0),
            sidebar: sidebar_color(true),
            card: card_color(true),
            border: rgba(0xFFFFFF1A),
            accent: rgb(accent_hex),
            hover: rgba(0xFFFFFF12),
            selected: rgba((accent_hex << 8) | 0x33),
            control: rgba(0xFFFFFF14),
            toggle_off: rgba(0xFFFFFF2E),
            knob: rgb(0xFFFFFF),
            accent_text: accent_text_color(accent_hex),
        }
    }
}

fn accent_text_color(accent_hex: u32) -> Rgba {
    if prefers_dark_foreground(accent_hex) {
        rgb(0x111111)
    } else {
        rgb(0xFFFFFF)
    }
}

#[cfg(target_os = "windows")]
fn sidebar_color(dark: bool) -> Rgba {
    // WinUI layer fill over Mica.
    if dark {
        rgba(0x3A3A3A4C)
    } else {
        rgba(0xFFFFFF80)
    }
}

#[cfg(target_os = "windows")]
fn card_color(dark: bool) -> Rgba {
    // Opaque WinUI cards: GPUI's Windows alpha blending accumulates
    // translucency across layers, so opaque cards are the stable surface.
    if dark { rgb(0x2B2B2B) } else { rgb(0xFAFAFA) }
}

#[cfg(target_os = "macos")]
fn sidebar_color(dark: bool) -> Rgba {
    if dark {
        rgba(0x1C1C1C80)
    } else {
        rgba(0xF5F5F5B3)
    }
}

#[cfg(target_os = "macos")]
fn card_color(dark: bool) -> Rgba {
    if dark {
        rgba(0xFFFFFF0F)
    } else {
        rgba(0xFFFFFFCC)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn sidebar_color(dark: bool) -> Rgba {
    if dark {
        rgba(0x1C1C1C80)
    } else {
        rgba(0xF5F5F5B3)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn card_color(dark: bool) -> Rgba {
    if dark {
        rgba(0xFFFFFF0F)
    } else {
        rgba(0xFFFFFFCC)
    }
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
}

impl SettingsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let view = Self {
            section: SettingsSection::General,
            settings: settings_state::get(),
            accent: system_accent_hex(),
            last_accent: None,
            status: None,
            recording_shortcut: None,
            filename_edit: None,
            filename_focus: cx.focus_handle(),
            focus_handle: cx.focus_handle(),
        };
        view.start_accent_watch(cx);
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
                .h(px(38.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .when(selected, |item| item.bg(selected_bg))
                .hover(move |style| style.bg(hover))
                .id(section.id())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    cx.notify();
                }))
                .child(icons::icon(
                    section.icon(),
                    17.0,
                    if selected { accent } else { muted },
                ))
                .child(
                    div()
                        .text_size(px(13.5))
                        .text_color(text)
                        .child(section.title(self.settings.language)),
                );
            nav.push(item.into_any_element());
        }

        div()
            .w(px(220.0))
            .h_full()
            .flex()
            .flex_col()
            .p(px(14.0))
            .gap(px(3.0))
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(8.0))
                    .py(px(10.0))
                    .child(icons::icon(icons::APP, 26.0, theme.accent))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .text_size(px(14.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child("Capin"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme.muted)
                                    .child(t(self.settings.language, "settings.subtitle")),
                            ),
                    ),
            )
            .child(div().h(px(14.0)))
            .child(div().flex().flex_col().gap(px(3.0)).children(nav))
            .child(div().flex_1())
            .child(self.capture_button(theme, cx))
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(10.0))
                    .text_size(px(11.0))
                    .text_color(theme.muted)
                    .child(format!("Capin v{}", env!("CARGO_PKG_VERSION"))),
            )
    }

    fn capture_button(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = theme.accent;
        let accent_text = theme.accent_text;
        div()
            .h(px(38.0))
            .w_full()
            .rounded(px(8.0))
            .bg(accent)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .id("settings-capture")
            .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx)))
            .child(icons::icon(icons::CAPTURE, 16.0, accent_text))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent_text)
                    .child(t(self.settings.language, "button.capture")),
            )
    }

    fn content(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .p(px(32.0))
            .gap(px(24.0))
            .id("settings-content")
            .overflow_y_scroll()
            .child(self.section_header(theme))
            .child(match self.section {
                SettingsSection::General => self.general(theme, cx),
                SettingsSection::Capture => self.capture_settings(theme, cx),
                SettingsSection::Recording => self.recording(theme, cx),
                SettingsSection::Shortcuts => self.shortcuts(theme, cx),
                SettingsSection::About => self.about(theme, cx),
            })
    }

    fn section_header(&self, theme: &Theme) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(
                div()
                    .text_size(px(26.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(self.section.title(self.settings.language)),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.muted)
                    .child(self.section.description(self.settings.language)),
            )
    }

    fn general(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let content = card(
            theme,
            vec![
                div().child(self.language_row(theme, cx)),
                div().child(self.auto_start_row(theme, cx)),
                info_row(
                    theme,
                    t(language, "settings.version"),
                    t(language, "settings.version_desc"),
                    env!("CARGO_PKG_VERSION"),
                    None,
                ),
            ],
        );
        #[cfg(target_os = "macos")]
        {
            content
                .child(group_header(
                    theme,
                    t(language, "settings.group.permissions"),
                    t(language, "settings.group.permissions_desc"),
                ))
                .child(self.permissions_card(theme, cx))
        }
        #[cfg(not(target_os = "macos"))]
        {
            content
        }
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
                    platform_window::screen_capture_trusted(),
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                    cx,
                )),
                div().child(self.permission_row(
                    theme,
                    t(language, "settings.permission_accessibility"),
                    t(language, "settings.permission_accessibility_desc"),
                    platform_window::accessibility_trusted(),
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
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
        url: &'static str,
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
        row_shell(theme)
            .child(label_block(theme, title, description))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(if granted { theme.muted } else { theme.accent })
                            .child(status),
                    )
                    .child(
                        action_button(theme, t(language, "settings.open_settings").to_owned())
                            .id("settings-open-permission")
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(url))),
                    ),
            )
    }

    fn language_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let mut control = segmented(theme);
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

        row_shell(theme)
            .child(label_block(
                theme,
                t(language, "settings.language"),
                t(language, "settings.language_desc"),
            ))
            .child(control)
    }

    fn auto_start_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.auto_start;

        row_shell(theme)
            .id("settings-auto-start")
            .on_click(cx.listener(|this, _, _, cx| {
                let language = this.settings.language;
                let desired = !this.settings.auto_start;
                match set_autostart(desired) {
                    Ok(AutostartState::Unsupported) => {
                        this.status =
                            Some(t(language, "settings.status_autostart_failed").to_owned());
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
            .child(label_block(
                theme,
                t(language, "settings.auto_start"),
                t(language, "settings.auto_start_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn capture_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut window_rows = vec![div().child(self.window_composition_row(theme, cx))];
        if self.settings.window_composition {
            window_rows.push(div().child(self.window_background_row(theme, cx)));
            window_rows.push(div().child(self.window_padding_row(theme, cx)));
            window_rows.push(div().child(self.window_shadow_row(theme, cx)));
            if self.settings.window_background == WindowBackground::Wallpaper {
                window_rows.push(div().child(self.window_wallpaper_row(theme, cx)));
            }
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(group_header(
                theme,
                t(language, "settings.group.selection"),
                t(language, "settings.group.selection_desc"),
            ))
            .child(card(
                theme,
                vec![
                    div().child(self.copy_to_clipboard_row(theme, cx)),
                    div().child(self.capture_cursor_row(theme, cx)),
                    info_row(
                        theme,
                        t(language, "settings.capture_mode"),
                        t(language, "settings.capture_mode_desc"),
                        t(language, "settings.capture_mode_value"),
                        Some(icons::CAPTURE),
                    ),
                    div().child(self.save_location_row(SaveTarget::Screenshots, theme, cx)),
                    div().child(self.filename_template_row(theme, cx)),
                ],
            ))
            .child(group_header(
                theme,
                t(language, "settings.group.annotation"),
                t(language, "settings.group.annotation_desc"),
            ))
            .child(card(
                theme,
                vec![div().child(self.annotation_uniform_color_row(theme, cx))],
            ))
            .child(group_header(
                theme,
                t(language, "settings.group.window"),
                t(language, "settings.group.window_desc"),
            ))
            .child(card(theme, window_rows))
            .child(group_header(
                theme,
                t(language, "settings.group.pin"),
                t(language, "settings.group.pin_desc"),
            ))
            .child(card(
                theme,
                vec![
                    div().child(self.pin_text_ocr_row(theme, cx)),
                    div().child(self.pin_qr_row(theme, cx)),
                ],
            ))
    }

    fn pin_text_ocr_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.pin_text_ocr;

        row_shell(theme)
            .id("settings-pin-text-ocr")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.pin_text_ocr;
                this.settings.pin_text_ocr = value;
                settings_state::update(|settings| settings.pin_text_ocr = value);
                cx.defer(crate::pin::refresh_recognition);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.pin_text_ocr"),
                t(language, "settings.pin_text_ocr_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn pin_qr_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.pin_qr;

        row_shell(theme)
            .id("settings-pin-qr")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.pin_qr;
                this.settings.pin_qr = value;
                settings_state::update(|settings| settings.pin_qr = value);
                cx.defer(crate::pin::refresh_recognition);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.pin_qr"),
                t(language, "settings.pin_qr_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn annotation_uniform_color_row(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.annotation_uniform_color;

        row_shell(theme)
            .id("settings-annotation-uniform-color")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.annotation_uniform_color;
                this.settings.annotation_uniform_color = value;
                settings_state::update(|settings| settings.annotation_uniform_color = value);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.annotation_uniform_color"),
                t(language, "settings.annotation_uniform_color_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn capture_cursor_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.capture_cursor;

        row_shell(theme)
            .id("settings-capture-cursor")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.capture_cursor;
                this.settings.capture_cursor = value;
                settings_state::update(|settings| settings.capture_cursor = value);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.capture_cursor"),
                t(language, "settings.capture_cursor_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn window_composition_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.window_composition;

        row_shell(theme)
            .id("settings-window-composition")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.window_composition;
                this.settings.window_composition = value;
                settings_state::update(|settings| settings.window_composition = value);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.window_composition"),
                t(language, "settings.window_composition_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn window_background_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let mut control = segmented(theme);
        for background in WindowBackground::ALL {
            let selected = self.settings.window_background == background;
            control = control.child(
                segment(
                    theme,
                    t(language, background_label_key(background)),
                    selected,
                )
                .id(background_id(background))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.window_background = background;
                    settings_state::update(|settings| settings.window_background = background);
                    this.status = None;
                    cx.notify();
                })),
            );
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .px(px(16.0))
            .py(px(14.0))
            .border_b_1()
            .border_color(theme.border)
            .child(label_block(
                theme,
                t(language, "settings.window_background"),
                t(language, "settings.window_background_desc"),
            ))
            .child(control)
    }

    fn window_padding_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let mut control = segmented(theme);
        for (padding, label) in [
            (0u32, t(language, "padding.0")),
            (24, t(language, "padding.24")),
            (48, t(language, "padding.48")),
            (96, t(language, "padding.96")),
        ] {
            let selected = self.settings.window_padding == padding;
            control = control.child(
                segment(theme, label, selected)
                    .id(padding_id(padding))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.window_padding = padding;
                        settings_state::update(|settings| settings.window_padding = padding);
                        this.status = None;
                        cx.notify();
                    })),
            );
        }

        row_shell(theme)
            .child(label_block(
                theme,
                t(language, "settings.window_padding"),
                t(language, "settings.window_padding_desc"),
            ))
            .child(control)
    }

    fn window_shadow_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.window_shadow;

        row_shell(theme)
            .id("settings-window-shadow")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.window_shadow;
                this.settings.window_shadow = value;
                settings_state::update(|settings| settings.window_shadow = value);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.window_shadow"),
                t(language, "settings.window_shadow_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn window_wallpaper_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let path = self
            .settings
            .window_wallpaper
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| t(language, "settings.no_wallpaper").to_owned());
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
            .px(px(16.0))
            .py(px(14.0))
            .border_b_1()
            .border_color(theme.border)
            .child(label_block(
                theme,
                t(language, "settings.window_wallpaper"),
                t(language, "settings.window_wallpaper_desc"),
            ))
            .child(
                div()
                    .flex()
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
                    .text_size(px(12.0))
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

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(14.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(24.0))
                    .child(label_block(
                        theme,
                        t(language, target.title_key()),
                        t(language, target.description_key()),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(open)
                            .child(change)
                            .children(reset),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(directory),
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
            .h(px(30.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .bg(theme.control)
            .border_1()
            .border_color(if editing { theme.accent } else { theme.border })
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(12.0))
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
            .px(px(16.0))
            .py(px(14.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(24.0))
                    .child(label_block(
                        theme,
                        t(language, "settings.filename_template"),
                        t(language, "settings.filename_template_desc"),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(field)
                            .child(action),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
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

    fn copy_to_clipboard_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.copy_to_clipboard;

        row_shell(theme)
            .id("settings-copy-to-clipboard")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.copy_to_clipboard;
                this.settings.copy_to_clipboard = value;
                settings_state::update(|settings| settings.copy_to_clipboard = value);
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.copy_clipboard"),
                t(language, "settings.copy_clipboard_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn recording(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(t(language, "settings.recording_note")),
            )
            .child(card(
                theme,
                vec![
                    self.recording_format_row(theme, cx),
                    self.recording_frame_rate_row(theme, cx),
                    self.recording_quality_row(theme, cx),
                    div().child(self.recording_segments_row(theme, cx)),
                    div().child(self.recording_cursor_row(theme, cx)),
                    div().child(self.save_location_row(SaveTarget::Recordings, theme, cx)),
                ],
            ))
    }

    fn recording_format_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut control = segmented(theme);
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

        row_shell(theme)
            .child(label_block(
                theme,
                t(language, "settings.format"),
                t(language, "settings.format_desc"),
            ))
            .child(control)
    }

    fn recording_frame_rate_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut control = segmented(theme);
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

        row_shell(theme)
            .child(label_block(
                theme,
                t(language, "settings.frame_rate"),
                t(language, "settings.frame_rate_desc"),
            ))
            .child(control)
    }

    fn recording_quality_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let mut control = segmented(theme);
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

        row_shell(theme)
            .child(label_block(
                theme,
                t(language, "settings.quality"),
                t(language, "settings.quality_desc"),
            ))
            .child(control)
    }

    fn recording_segments_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.record_segments;

        row_shell(theme)
            .id("settings-record-segments")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.record_segments;
                this.settings.record_segments = value;
                settings_state::update(|settings| settings.record_segments = value);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.record_segments"),
                t(language, "settings.record_segments_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn recording_cursor_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.settings.language;
        let value = self.settings.record_cursor;

        row_shell(theme)
            .id("settings-record-cursor")
            .on_click(cx.listener(|this, _, _, cx| {
                let value = !this.settings.record_cursor;
                this.settings.record_cursor = value;
                settings_state::update(|settings| settings.record_cursor = value);
                this.status = None;
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, "settings.record_cursor"),
                t(language, "settings.record_cursor_desc"),
            ))
            .child(toggle(theme, value))
    }

    fn shortcuts(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(card(
                theme,
                vec![
                    div().child(self.shortcut_row(ShortcutSlot::Capture, theme, cx)),
                    div().child(self.shortcut_row(ShortcutSlot::Pin, theme, cx)),
                ],
            ))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(t(language, "shortcuts.editor_note")),
            )
            .child(group_header(
                theme,
                t(language, "settings.group.capture_keys"),
                t(language, "settings.group.capture_keys_desc"),
            ))
            .child(card(
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
            ))
    }

    fn shortcut_row(
        &self,
        slot: ShortcutSlot,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
        let border = theme.border;
        let text = theme.text;

        row_shell(theme)
            .id(slot.id())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.recording_shortcut = Some(slot);
                this.status = None;
                window.focus(&this.focus_handle, cx);
                cx.notify();
            }))
            .child(label_block(
                theme,
                t(language, slot.title_key()),
                t(language, slot.description_key()),
            ))
            .child(
                div()
                    .px(px(10.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(if recording { selected } else { control })
                    .border_1()
                    .border_color(if recording { accent } else { border })
                    .text_size(px(12.0))
                    .text_color(if recording { accent } else { text })
                    .child(display),
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

    fn about(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let language = self.settings.language;
        let hero = div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.0))
            .p(px(20.0))
            .rounded(px(12.0))
            .bg(theme.card)
            .border_1()
            .border_color(theme.border)
            .child(icons::icon(icons::APP, 52.0, theme.accent))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(px(21.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Capin"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.muted)
                            .child(format!(
                                "{} {}",
                                t(language, "settings.version"),
                                env!("CARGO_PKG_VERSION")
                            )),
                    ),
            );

        let details = card(
            theme,
            vec![
                info_row(
                    theme,
                    t(language, "about.license"),
                    t(language, "about.license_desc"),
                    t(language, "about.license_value"),
                    None,
                ),
                info_row(
                    theme,
                    t(language, "about.platforms"),
                    t(language, "about.platforms_desc"),
                    t(language, "about.platforms_value"),
                    None,
                ),
                info_row(
                    theme,
                    t(language, "about.privacy"),
                    t(language, "about.privacy_desc"),
                    t(language, "about.privacy_value"),
                    None,
                ),
                div().w_full().child(
                    row_shell(theme)
                        .id("settings-project-link")
                        .hover(move |style| style.bg(theme.hover))
                        .on_click(cx.listener(|_, _, _, cx| cx.open_url(PROJECT_URL)))
                        .child(label_block(
                            theme,
                            t(language, "about.project"),
                            t(language, "about.project_desc"),
                        ))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(theme.accent)
                                .child("github.com/fynhq/capin"),
                        ),
                ),
            ],
        );

        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(hero)
            .child(details)
    }

    fn start_capture(&self, cx: &mut Context<Self>) {
        let handles = cx.window_stack().unwrap_or_default();
        cx.defer(move |cx| {
            crate::windows::close_auxiliary(cx, handles);
            crate::overlay::open_overlays(cx);
        });
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
        div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .text_color(theme.text)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.handle_filename_key(event, window, cx) {
                    return;
                }
                this.handle_shortcut_key(event, window, cx);
            }))
            .child(self.sidebar(&theme, cx))
            .child(self.content(&theme, cx))
            .children(status.map(|message| {
                div()
                    .absolute()
                    .bottom(px(16.0))
                    .right(px(16.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(rgba(0x000000CC))
                    .text_size(px(12.0))
                    .text_color(rgb(0xFFFFFF))
                    .child(message)
            }))
    }
}

pub fn open_settings(cx: &mut App) {
    let bounds = settings_bounds(cx);
    let display_id = cx.primary_display().map(|display| display.id());
    let language = settings_state::get().language;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(t(language, "settings.title").into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        display_id,
        window_background: settings_background(),
        window_min_size: Some(size(px(760.0), px(520.0))),
        app_id: Some("capin-settings".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, |window, cx| {
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

fn settings_bounds(cx: &App) -> Bounds<Pixels> {
    crate::ui::centered_bounds(cx, 900.0, 620.0)
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

fn card(theme: &Theme, rows: Vec<Div>) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .bg(theme.card)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .children(rows)
}

fn group_header(theme: &Theme, title: &str, description: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(theme.muted)
                .child(description.to_owned()),
        )
}

fn row_shell(theme: &Theme) -> Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .px(px(16.0))
        .py(px(14.0))
        .border_b_1()
        .border_color(theme.border)
}

fn label_block(theme: &Theme, title: &str, subtitle: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(13.5))
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(px(12.0))
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
    let mut value_side = div().flex().flex_row().items_center().gap(px(6.0));
    if let Some(icon_data) = icon {
        value_side = value_side.child(icons::icon(icon_data, 15.0, theme.muted));
    }
    value_side = value_side.child(
        div()
            .text_size(px(12.5))
            .text_color(theme.muted)
            .child(value.to_owned()),
    );

    row_shell(theme)
        .child(label_block(theme, title, subtitle))
        .child(value_side)
}

fn toggle(theme: &Theme, value: bool) -> Div {
    let accent = theme.accent;
    let off = theme.toggle_off;
    let knob = theme.knob;
    div()
        .w(px(38.0))
        .h(px(22.0))
        .rounded(px(11.0))
        .bg(if value { accent } else { off })
        .flex()
        .items_center()
        .px(px(3.0))
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

fn background_id(background: WindowBackground) -> &'static str {
    match background {
        WindowBackground::Transparent => "window-background-transparent",
        WindowBackground::Light => "window-background-light",
        WindowBackground::Dark => "window-background-dark",
        WindowBackground::Gradient => "window-background-gradient",
        WindowBackground::Wallpaper => "window-background-wallpaper",
    }
}

fn padding_id(padding: u32) -> &'static str {
    match padding {
        0 => "window-padding-0",
        24 => "window-padding-24",
        48 => "window-padding-48",
        _ => "window-padding-96",
    }
}

fn action_button(theme: &Theme, label: String) -> Div {
    let hover = theme.hover;
    div()
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .bg(theme.control)
        .text_size(px(12.0))
        .text_color(theme.text)
        .hover(move |style| style.bg(hover))
        .child(label)
}

fn segmented(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .gap(px(2.0))
        .p(px(3.0))
        .rounded(px(9.0))
        .bg(theme.control)
}

fn segment(theme: &Theme, label: &str, selected: bool) -> Div {
    div()
        .px(px(12.0))
        .py(px(5.0))
        .rounded(px(7.0))
        .text_size(px(12.0))
        .text_color(if selected { theme.text } else { theme.muted })
        .when(selected, |button| button.bg(theme.selected))
        .child(label.to_owned())
}
