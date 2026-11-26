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
    self, AppSettings, DoubleClickAction, PinDoubleClickAction, RepeatPinBehavior, WINDOW_PADDINGS,
    WindowBackground,
};
use crate::storage;
use crate::theme::{accent_text_color, fallback_accent, settings_palette};

const PROJECT_URL: &str = "https://github.com/fynhq/capin";

mod capture;
mod general;
mod layout;
mod nav;
mod recording;
mod render;
mod settings_scroll;
mod shortcuts;
mod storage_rows;
mod toast;
mod window;

use toast::SettingsToast;

thread_local! {
    static PENDING_STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn watch_settings_activation(handle: AnyWindowHandle, cx: &mut App) {
    let window_id = handle.window_id().as_u64();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(200))
            .await;
        let active = handle
            .update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false);
        if active {
            log_info!("settings", "settings window={window_id} is active");
        } else {
            log_warn!("settings", "settings window={window_id} is not active");
        }
    })
    .detach();
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
    let focused = handle
        .update(cx, |_, window, _| window.activate_window())
        .is_ok();
    if focused {
        log_info!(
            "settings",
            "focused existing settings window={}",
            handle.window_id().as_u64()
        );
    } else {
        log_warn!(
            "settings",
            "existing settings window={} could not be focused",
            handle.window_id().as_u64()
        );
    }
    cx.activate(true);
    watch_settings_activation(handle, cx);
    true
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
    PinDoubleClick,
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
    status: SettingsToast,
    recording_shortcut: Option<ShortcutSlot>,
    filename_input: Entity<InputState>,
    filename_subscription: Option<Subscription>,
    activation_subscription: Option<Subscription>,
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
            status: SettingsToast::from_message(
                PENDING_STATUS.with(|slot| slot.borrow_mut().take()),
            ),
            recording_shortcut: None,
            filename_input,
            filename_subscription: None,
            activation_subscription: None,
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
        view.activation_subscription =
            Some(cx.observe_window_activation(window, |this, window, cx| {
                log_info!(
                    "settings",
                    "settings window={} active={}",
                    window.window_handle().window_id().as_u64(),
                    window.is_window_active()
                );
                if !window.is_window_active() && this.recording_shortcut.take().is_some() {
                    crate::hotkey::set_settings_recording(false);
                    cx.notify();
                }
            }));
        cx.on_release(|this, cx| {
            crate::hotkey::set_settings_recording(false);
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
        start_accent_watch(cx);
        #[cfg(target_os = "macos")]
        start_permission_watch(cx);
        if let Some(status) = view.status.message().cloned() {
            view.show_status(status, cx);
        }
        view
    }

    fn show_status(&mut self, message: String, cx: &mut Context<Self>) {
        self.status.show(message, cx);
    }
}

fn start_accent_watch(cx: &mut Context<SettingsView>) {
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
fn start_permission_watch(cx: &mut Context<SettingsView>) {
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
        window_background: crate::aux_window::panel_background(),
        window_min_size: Some(size(px(layout::MIN_WIDTH), px(layout::MIN_HEIGHT))),
        app_id: Some("capin-settings".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, |window, cx| {
        let view = cx.new(|cx| SettingsView::new(window, cx));
        let close_view = view.clone();
        window.on_window_should_close(cx, move |window, cx| {
            log_info!(
                "settings",
                "settings window={} closing",
                window.window_handle().window_id().as_u64()
            );
            close_view.update(cx, |this, cx| {
                crate::hotkey::set_settings_recording(false);
                commit_settings_inputs(this, window, cx);
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
            log_info!(
                "settings",
                "opened settings window={} at={:.0},{:.0} size={:.0}x{:.0}",
                handle.window_id().as_u64(),
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height)
            );
            crate::windows::register_settings_window(handle, view.downgrade());
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
            watch_settings_activation(handle, cx);
        }
        Err(error) => log_error!("settings", "failed to open settings window: {error}"),
    }
}

#[cfg(target_os = "macos")]
pub fn report_capture_permission_denied(cx: &mut App) {
    let language = settings_state::get().language;
    let message = t(language, "settings.status_capture_permission").to_owned();
    PENDING_STATUS.with(|slot| *slot.borrow_mut() = Some(message));
    open_settings(cx);
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

fn pin_double_click_key(action: PinDoubleClickAction) -> &'static str {
    match action {
        PinDoubleClickAction::NoAction => "pin_double_click.none",
        PinDoubleClickAction::Close => "pin_double_click.close",
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

fn commit_settings_inputs(
    view: &mut SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    let filename = view.filename_input.clone();
    view.commit_filename_template(&filename, window, cx);
}
