use std::cell::RefCell;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style, system_accent_hex};

use crate::capture_panels::{hints_card, hints_panel_height, hints_panel_width};
use crate::chrome_theme::ChromeTheme;
use crate::i18n::{Language, t};
use crate::logging::log_error;
use crate::settings_state;

thread_local! {
    static PIN_HELP: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
}

pub(super) fn toggle(
    anchor: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    image: bool,
    cx: &mut App,
) {
    if close(cx) {
        return;
    }
    open(anchor, display_id, image, cx);
}

fn open(anchor: Bounds<Pixels>, display_id: Option<DisplayId>, image: bool, cx: &mut App) {
    let language = settings_state::get().language;
    let rows = rows(image, language);
    let display = display_id
        .and_then(|id| cx.displays().into_iter().find(|display| display.id() == id))
        .or_else(|| cx.primary_display());
    let Some(display) = display else {
        return;
    };
    let area = display.bounds();
    let area_left = f32::from(area.origin.x);
    let area_top = f32::from(area.origin.y);
    let area_width = f32::from(area.size.width);
    let area_height = f32::from(area.size.height);
    let width = hints_panel_width().min((area_width - 32.0).max(160.0));
    let height = hints_panel_height(rows.len());
    let anchor_left = f32::from(anchor.origin.x);
    let anchor_top = f32::from(anchor.origin.y);
    let right = anchor_left + f32::from(anchor.size.width) + 8.0;
    let left = if right + width <= area_left + area_width - 8.0 {
        right
    } else {
        anchor_left - width - 8.0
    };
    let left = left.clamp(
        area_left + 8.0,
        (area_left + area_width - width - 8.0).max(area_left + 8.0),
    );
    let top = anchor_top + (f32::from(anchor.size.height) - height) / 2.0;
    let top = top.clamp(
        area_top + 8.0,
        (area_top + area_height - height - 8.0).max(area_top + 8.0),
    );
    let bounds = Bounds {
        origin: point(px(left), px(top)),
        size: size(px(width), px(height)),
    };
    let options = crate::aux_window::popup_options(
        "capin-pin-help",
        bounds,
        Some(display.id()),
        true,
        super::pin_window_kind(),
        WindowBackgroundAppearance::Transparent,
    );
    match gpui_kit::open_window(options, cx, move |window, cx| {
        let _ = apply_window_style(&*window, WindowStyle::PIN_MENU);
        let view = cx.new(|cx| PinHelpView {
            rows,
            width,
            accent_hex: system_accent_hex().unwrap_or_else(|| crate::theme::fallback_accent(false)),
            focus_handle: cx.focus_handle(),
            _blur_subscription: None,
        });
        let focus = view.read(cx).focus_handle.clone();
        view.update(cx, |view, cx| {
            view._blur_subscription = Some(cx.on_blur(&focus, window, |_, window, _| {
                forget_help();
                window.remove_window();
            }));
        });
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        view
    }) {
        Ok((handle, _)) => {
            PIN_HELP.with(|slot| *slot.borrow_mut() = Some(handle));
        }
        Err(error) => log_error!("pin", "failed to open pin help: {error}"),
    }
}

pub(super) fn close(cx: &mut App) -> bool {
    let handle = PIN_HELP.with(|slot| slot.borrow_mut().take());
    let Some(handle) = handle else {
        return false;
    };
    handle
        .update(cx, |_, window, _| window.remove_window())
        .is_ok()
}

fn forget_help() {
    PIN_HELP.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn rows(image: bool, language: Language) -> Vec<(String, String)> {
    let mut rows = vec![
        (
            "Cmd/Ctrl+C".to_owned(),
            t(
                language,
                if image {
                    "pin.hint_copy_image"
                } else {
                    "pin.hint_copy_text"
                },
            )
            .to_owned(),
        ),
        (
            "Cmd/Ctrl+V".to_owned(),
            t(language, "pin.hint_paste").to_owned(),
        ),
    ];
    if image {
        rows.push((
            "Cmd/Ctrl+S".to_owned(),
            t(language, "pin.hint_save").to_owned(),
        ));
        rows.push(("Scroll".to_owned(), t(language, "pin.hint_zoom").to_owned()));
        rows.push((
            "Shift+Scroll".to_owned(),
            t(language, "pin.hint_opacity").to_owned(),
        ));
        rows.push((
            "Alt+Drag".to_owned(),
            t(language, "pin.hint_drag_any").to_owned(),
        ));
    } else {
        rows.push(("Drag".to_owned(), t(language, "pin.hint_drag").to_owned()));
    }
    rows.push(("F1".to_owned(), t(language, "pin.hint_help").to_owned()));
    rows.push(("Esc".to_owned(), t(language, "pin.hint_close").to_owned()));
    rows
}

struct PinHelpView {
    rows: Vec<(String, String)>,
    width: f32,
    accent_hex: u32,
    focus_handle: FocusHandle,
    _blur_subscription: Option<gpui_kit::Subscription>,
}

impl Render for PinHelpView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ChromeTheme::for_window(window, Some(self.accent_hex));
        let language = settings_state::get().language;
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" || event.keystroke.key == "f1" {
                    forget_help();
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .child(crate::ui::enter_fade(
                hints_card(theme, language, self.width, &self.rows, false, false),
                "pin-help-enter",
                Duration::from_millis(100),
            ))
    }
}
