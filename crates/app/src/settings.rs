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
use crate::logging::{log_error, log_info, log_warn};
use crate::settings_state::{
    self, AppSettings, DoubleClickAction, RepeatPinBehavior, WINDOW_PADDINGS, WindowBackground,
};
use crate::storage;
use crate::theme::{accent_text_color, fallback_accent, settings_palette};

const PROJECT_URL: &str = "https://github.com/fynhq/capin";

mod capture;
mod general;
mod recording;
mod settings_scroll;
mod shortcuts;
mod storage_rows;

thread_local! {
    static PENDING_STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn focus_settings_window(cx: &mut App) -> bool {
    let Some((handle, view)) = crate::windows::live_settings_windows(cx).into_iter().next() else {
        return false;
    };
    let pending = PENDING_STATUS.with(|slot| slot.borrow_mut().take());
    if let Some(pending) = pending
        && let Some(view) = view.upgrade()
    {
        view.update(cx, |this, cx| this.show_status(pending, cx));
    }
    let _ = handle.update(cx, |_, window, _| window.activate_window());
    cx.activate(true);
    true
}

#[cfg(target_os = "windows")]
mod layout {
    pub const WINDOW_WIDTH: f32 = 870.0;
    pub const WINDOW_HEIGHT: f32 = 640.0;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectKind {
    WindowBackground,
    WindowPadding,
    RepeatPin,
    DoubleClick,
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
    sidebar: Rgba,
    border: Rgba,
    accent: Rgba,
    accent_text: Rgba,
    option_selected: Rgba,
    option_hover: Rgba,
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

impl Theme {
    fn for_window(window: &Window, accent: Option<u32>) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let accent_hex = accent.unwrap_or_else(|| fallback_accent(dark));
        Self::build(dark, accent_hex, window.scale_factor())
    }

    fn build(dark: bool, accent_hex: u32, scale_factor: f32) -> Self {
        let palette = settings_palette(dark);
        Self {
            text: palette.text,
            muted: palette.muted,
            card: palette.card,
            card_border: palette.card_border,
            page: palette.page,
            sidebar: palette.sidebar,
            border: palette.border,
            accent: rgb(accent_hex),
            accent_text: accent_text_color(accent_hex),
            option_selected: rgb(if dark { 0x262626 } else { 0xF5F5F5 }),
            option_hover: rgb(if dark { 0x282828 } else { 0xF6F6F6 }),
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

pub(crate) struct SettingsView {
    section: SettingsSection,
    scroll_handle: ScrollHandle,
    accent: Option<u32>,
    last_accent: Option<u32>,
    status: Option<String>,
    status_serial: u64,
    status_leaving: bool,
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
    fn settings(&self) -> std::sync::Arc<AppSettings> {
        settings_state::get()
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = settings_state::get();
        let theme = Theme::for_window(window, None);
        let filename_input =
            cx.new(|cx| InputState::new(window, cx).default_value(&settings.filename_template));
        filename_input.update(cx, |state, cx| {
            state.set_placeholder(settings_state::DEFAULT_FILENAME_TEMPLATE, window, cx);
            state.set_editor_style(input_editor_style(&theme));
        });
        let mut view = Self {
            section: SettingsSection::General,
            scroll_handle: ScrollHandle::new(),
            accent: system_accent_hex(),
            last_accent: None,
            status: PENDING_STATUS.with(|slot| slot.borrow_mut().take()),
            status_serial: 0,
            status_leaving: false,
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

    fn show_status(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = Some(message);
        self.status_leaving = false;
        self.status_serial = self.status_serial.wrapping_add(1);
        let serial = self.status_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.status_serial != serial {
                        return false;
                    }
                    this.status_leaving = true;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !alive {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            this.update(cx, |this, cx| {
                if this.status_serial == serial {
                    this.status = None;
                    this.status_leaving = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

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

    fn sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
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
                .role(Role::Button)
                .aria_label(section.title(self.settings().language))
                .aria_selected(selected)
                .active(move |style| style.bg(selected_bg))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    this.scroll_handle.set_offset(point(px(0.0), px(0.0)));
                    this.select_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id(format!("settings-nav-icon-{}", section.id()))
                        .role(Role::Image)
                        .aria_label(section.title(self.settings().language))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icons::icon(
                            section.icon(),
                            layout::NAV_ICON,
                            if selected { accent } else { muted },
                        )),
                )
                .child(
                    div()
                        .text_size(px(layout::NAV_FONT))
                        .text_color(text)
                        .child(section.title(self.settings().language)),
                );
            nav.push(item.into_any_element());
        }

        let selected_index = SettingsSection::ALL
            .iter()
            .position(|candidate| *candidate == self.section)
            .unwrap_or(0) as f32;
        let indicator_top = selected_index * (layout::NAV_ITEM_HEIGHT + 2.0)
            + (layout::NAV_ITEM_HEIGHT - 16.0) / 2.0;
        let spring =
            SpringAnimation::new(SpringConfig::new(700.0, 46.0, 1.0)).to(px(indicator_top));
        let indicator = div()
            .absolute()
            .left(px(0.0))
            .top(px(indicator_top))
            .w(px(3.0))
            .h(px(16.0))
            .rounded(px(1.5))
            .bg(theme.accent)
            .with_spring("settings-nav-indicator", spring, |bar, top| bar.top(top));
        let nav = div()
            .id("settings-nav")
            .role(Role::Navigation)
            .aria_label(t(self.settings().language, "settings.a11y_sidebar"))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(nav)
            .child(indicator);
        let mut sidebar = div()
            .id("settings-sidebar")
            .role(Role::Complementary)
            .aria_label(t(self.settings().language, "settings.a11y_sidebar"))
            .w(px(layout::SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(px(layout::SIDEBAR_PAD))
            .pb(px(layout::SIDEBAR_PAD));

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
        #[cfg(not(target_os = "macos"))]
        {
            sidebar = sidebar.pt(px(layout::TITLEBAR_HEIGHT + 8.0)).child(nav);
        }

        sidebar = sidebar
            .child(self.sidebar_footer(theme, cx))
            .bg(theme.sidebar);
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
                    .child(t(self.settings().language, "settings.title")),
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
        let version = crate::build_info::version_label();
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
                    .role(Role::Button)
                    .aria_label(version.clone())
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(accent))
                    .on_click(cx.listener(|this, _, _, cx| this.open_project(cx)))
                    .child(version),
            )
    }

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
                    .child(t(self.settings().language, "settings.title")),
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
        #[cfg(target_os = "macos")]
        let top = 0.0;
        #[cfg(not(target_os = "macos"))]
        let top = layout::TITLEBAR_HEIGHT;
        let content_width = (f32::from(window.bounds().size.width)
            - layout::SIDEBAR_WIDTH
            - layout::CONTENT_PAD_X * 2.0)
            .clamp(240.0, layout::CONTENT_MAX_WIDTH);
        let header = div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(layout::CONTENT_PAD_X))
            .pt(px(layout::CONTENT_PAD_TOP))
            .pb(px(layout::HEADER_GAP))
            .child(div().w(px(content_width)).child(self.section_header(theme)));
        #[cfg(target_os = "macos")]
        let top_block = self
            .macos_drag_area(
                "settings-content-drag-strip",
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(div().w_full().h(px(layout::TITLEBAR_HEIGHT)))
                    .child(header),
                cx,
            )
            .into_any_element();
        #[cfg(not(target_os = "macos"))]
        let top_block = header.into_any_element();
        div()
            .id("settings-content")
            .role(Role::Main)
            .aria_label(t(self.settings().language, "settings.a11y_content"))
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(top))
            .when_some(theme.page, |content, page| content.bg(page))
            .child(top_block)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .px(px(layout::CONTENT_PAD_X))
                            .pb(px(layout::CONTENT_PAD_BOTTOM))
                            .id(format!("settings-content-{}", self.section.id()))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                            .child(
                                div()
                                    .w(px(content_width))
                                    .flex()
                                    .flex_col()
                                    .gap(px(layout::CONTENT_GAP))
                                    .child(match self.section {
                                        SettingsSection::General => self.general(theme, window, cx),
                                        SettingsSection::Capture => {
                                            self.capture_settings(theme, cx)
                                        }
                                        SettingsSection::Recording => self.recording(theme, cx),
                                        SettingsSection::Shortcuts => self.shortcuts(theme, cx),
                                    })
                                    .with_animation(
                                        SharedString::from(format!(
                                            "settings-page-{}",
                                            self.section.id()
                                        )),
                                        Animation::new(Duration::from_millis(120))
                                            .with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                                        |page, delta| page.opacity(delta),
                                    ),
                            ),
                    )
                    .child(settings_scroll::scrollbar(theme, &self.scroll_handle)),
            )
    }

    fn section_header(&self, theme: &Theme) -> Div {
        let title = self.section.title(self.settings().language);
        div().flex().flex_col().child(
            div()
                .id("settings-section-heading")
                .role(Role::Heading)
                .aria_label(title)
                .aria_level(1)
                .text_size(px(layout::PAGE_TITLE))
                .text_color(theme.text)
                .child(title),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.settings().record_cursor_highlight {
            self.record_color_picker = None;
        }
        let theme = Theme::for_window(window, self.accent);
        let status = self.status.clone();
        let status_serial = self.status_serial;
        let status_leaving = self.status_leaving;
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
                let duration = if status_leaving {
                    Duration::from_millis(150)
                } else {
                    Duration::from_millis(120)
                };
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
                    .with_animation(
                        SharedString::from(format!(
                            "settings-status-{status_serial}-{status_leaving}"
                        )),
                        Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                        move |toast, delta| {
                            if status_leaving {
                                toast.opacity(1.0 - delta)
                            } else {
                                toast.opacity(delta).bottom(px(16.0 - (1.0 - delta) * 4.0))
                            }
                        },
                    )
            }))
    }
}

pub fn open_settings(cx: &mut App) {
    if focus_settings_window(cx) {
        return;
    }
    let bounds = general::settings_bounds(cx);
    let display_id = cx.primary_display().map(|display| display.id());
    let language = settings_state::get().language;
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
            crate::windows::unregister_settings_window(&handle);
            if !crate::windows::has_open_settings(cx) {
                platform_window::set_macos_accessory_activation_policy();
            }
            true
        });
        let focus = view.read(cx).focus_handle.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, view)) => {
            crate::windows::register_settings_window(handle, view.downgrade());
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
        }
        Err(error) => log_error!("settings", "failed to open settings: {error}"),
    }
}

#[cfg(target_os = "macos")]
pub fn report_capture_permission_denied(cx: &mut App) {
    let language = settings_state::get().language;
    let message = t(language, "settings.status_capture_permission").to_owned();
    PENDING_STATUS.with(|slot| *slot.borrow_mut() = Some(message));
    open_settings(cx);
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
        .role(Role::Button)
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

fn card(theme: &Theme, rows: Vec<AnyElement>) -> Div {
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
                .id("settings-group-heading")
                .role(Role::Heading)
                .aria_label(title.to_owned())
                .aria_level(2)
                .text_size(px(layout::SECTION_TITLE))
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .when(!description.is_empty(), |header| {
            header.child(
                div()
                    .id("settings-group-description")
                    .role(Role::Label)
                    .aria_label(description.to_owned())
                    .text_size(px(layout::CAPTION))
                    .text_color(theme.muted)
                    .child(description.to_owned()),
            )
        })
}

fn group(id: &'static str, theme: &Theme, title: &str, description: &str, body: Div) -> AnyElement {
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(title.to_owned())
        .flex()
        .flex_col()
        .gap(px(layout::GROUP_HEADER_GAP))
        .child(group_header(theme, title, description))
        .child(body)
        .into_any_element()
}

fn a11y_row(id: impl Into<ElementId>, label: impl Into<SharedString>, row: Div) -> AnyElement {
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(label)
        .w_full()
        .child(row)
        .into_any_element()
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

fn row_label(theme: &Theme, title: &str, subtitle: &str) -> impl IntoElement {
    label_block(theme, title, subtitle).flex_1().min_w(px(0.0))
}

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
                .id("settings-row-icon")
                .role(Role::Image)
                .aria_label(title.to_owned())
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

fn label_block(theme: &Theme, title: &str, subtitle: &str) -> Stateful<Div> {
    let label = if subtitle.is_empty() {
        title.to_owned()
    } else {
        format!("{title} {subtitle}")
    };
    div()
        .id("settings-row-text")
        .role(Role::Label)
        .aria_label(label)
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
) -> AnyElement {
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
        Some(icon) => row_label_with_icon(theme, icon, title, subtitle).into_any_element(),
        None => row_label(theme, title, subtitle).into_any_element(),
    };
    a11y_row(
        ElementId::Name(SharedString::from(format!("settings-info-{title}"))),
        title.to_owned(),
        row_shell().child(label).child(value_side),
    )
}

fn toggle(theme: &Theme, value: bool, id: &'static str) -> impl IntoElement {
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
        WindowBackground::White => "window_background.white",
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

fn double_click_key(action: DoubleClickAction) -> &'static str {
    match action {
        DoubleClickAction::NoAction => "double_click.none",
        DoubleClickAction::Copy => "double_click.copy",
        DoubleClickAction::Pin => "double_click.pin",
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
