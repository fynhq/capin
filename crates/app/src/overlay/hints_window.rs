use std::cell::RefCell;

use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style};

use crate::capture_panels::{hints_card, hints_panel_height, hints_panel_width};
use crate::chrome_theme::ChromeTheme;
use crate::logging::{log_error, log_warn};
use crate::settings_state;

use super::OverlayView;

thread_local! {
    static OPEN_HINTS: RefCell<Option<OpenHints>> = const { RefCell::new(None) };
}

struct OpenHints {
    handle: AnyWindowHandle,
    owner_handle: AnyWindowHandle,
    owner_focus: FocusHandle,
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
struct HintsDrag {
    cursor: (i32, i32),
    origin: (i32, i32),
}

fn window_kind() -> WindowKind {
    #[cfg(target_os = "macos")]
    {
        WindowKind::Floating
    }
    #[cfg(not(target_os = "macos"))]
    {
        WindowKind::PopUp
    }
}

fn rows() -> Vec<(String, String)> {
    let language = settings_state::get().language;
    let pin_hotkey = crate::hotkey::display_hotkey(&settings_state::get().pin_hotkey);
    let k = |key: &str| crate::i18n::t(language, key).to_owned();
    vec![
        (k("editor.hint_key_copy"), k("editor.hint_copy")),
        ("Cmd/Ctrl+S".to_owned(), k("editor.hint_save")),
        (pin_hotkey, k("editor.hint_pin")),
        ("Space".to_owned(), k("editor.hint_space")),
        ("R / F5".to_owned(), k("editor.hint_ratio_refresh")),
        ("Esc".to_owned(), k("editor.hint_cancel")),
    ]
}

pub(super) fn show(
    owner: WeakEntity<OverlayView>,
    owner_handle: AnyWindowHandle,
    owner_focus: FocusHandle,
    display: Option<DisplayId>,
    theme: ChromeTheme,
    overlay_bounds: Bounds<Pixels>,
    cx: &mut App,
) {
    let Some(view) = owner.upgrade() else {
        return;
    };
    let open = view
        .read(cx)
        .selected
        .as_ref()
        .is_some_and(|selected| selected.hints_open);
    if !open {
        return;
    }
    close(cx);
    let rows = rows();
    let width = hints_panel_width().min((f32::from(overlay_bounds.size.width) - 16.0).max(120.0));
    let height = hints_panel_height(rows.len());
    let left =
        f32::from(overlay_bounds.origin.x) + (f32::from(overlay_bounds.size.width) - width) / 2.0;
    let top =
        f32::from(overlay_bounds.origin.y) + (f32::from(overlay_bounds.size.height) - height) / 2.0;
    let bounds = Bounds {
        origin: point(px(left), px(top)),
        size: size(px(width), px(height)),
    };
    let options = crate::aux_window::popup_options(
        "capin-overlay-hints",
        bounds,
        display,
        false,
        window_kind(),
        crate::aux_window::panel_background(),
    );
    let view_focus = owner_focus.clone();
    match gpui_kit::open_window(options, cx, move |window, cx| {
        if let Err(error) = apply_window_style(&*window, WindowStyle::HINTS) {
            log_warn!("overlay", "failed to style hints window: {error}");
        }
        cx.new(|cx| HintsWindowView {
            owner: owner.clone(),
            owner_handle,
            owner_focus: view_focus,
            focus_handle: cx.focus_handle(),
            theme,
            rows,
            width,
            #[cfg(target_os = "windows")]
            drag: None,
        })
    }) {
        Ok((handle, _)) => {
            OPEN_HINTS.with(|slot| {
                *slot.borrow_mut() = Some(OpenHints {
                    handle,
                    owner_handle,
                    owner_focus,
                });
            });
        }
        Err(error) => log_error!("overlay", "failed to open hints window: {error}"),
    }
}

pub(super) fn close(cx: &mut App) {
    let Some(state) = OPEN_HINTS.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    let OpenHints {
        handle,
        owner_handle,
        owner_focus,
    } = state;
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    let _ = owner_handle.update(cx, |_, window, cx| {
        window.activate_window();
        window.focus(&owner_focus, cx);
    });
}

fn forget() {
    OPEN_HINTS.with(|slot| {
        slot.borrow_mut().take();
    });
}

struct HintsWindowView {
    owner: WeakEntity<OverlayView>,
    owner_handle: AnyWindowHandle,
    owner_focus: FocusHandle,
    focus_handle: FocusHandle,
    theme: ChromeTheme,
    rows: Vec<(String, String)>,
    width: f32,
    #[cfg(target_os = "windows")]
    drag: Option<HintsDrag>,
}

impl HintsWindowView {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = self.owner.clone();
        let owner_handle = self.owner_handle;
        let owner_focus = self.owner_focus.clone();
        forget();
        owner
            .update(cx, |view, cx| view.hints_window_closed(cx))
            .ok();
        let _ = owner_handle.update(cx, |_, window, cx| {
            window.activate_window();
            window.focus(&owner_focus, cx);
        });
        window.remove_window();
    }

    fn begin_drag(&mut self, window: &mut Window) {
        #[cfg(target_os = "windows")]
        {
            let cursor = platform_window::cursor_screen_position();
            let origin = platform_window::window_screen_origin(&*window);
            if let (Some(cursor), Some(origin)) = (cursor, origin) {
                self.drag = Some(HintsDrag { cursor, origin });
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            window.start_window_move();
        }
    }

    #[cfg(target_os = "windows")]
    fn update_drag(&mut self, window: &mut Window) {
        if let Some(drag) = self.drag
            && let Some((cursor_x, cursor_y)) = platform_window::cursor_screen_position()
        {
            let x = drag.origin.0 + cursor_x - drag.cursor.0;
            let y = drag.origin.1 + cursor_y - drag.cursor.1;
            let _ = platform_window::move_window_to(&*window, x, y);
        }
    }
}

impl Render for HintsWindowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = settings_state::get().language;
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" || event.keystroke.key == "f1" {
                    this.close(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.begin_drag(window);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|_this, _event: &MouseMoveEvent, _window, _cx| {
                #[cfg(target_os = "windows")]
                _this.update_drag(_window);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|_this, _event: &MouseUpEvent, _window, _cx| {
                    #[cfg(target_os = "windows")]
                    {
                        _this.drag = None;
                    }
                }),
            )
            .child(hints_card(
                self.theme, language, self.width, &self.rows, false, true,
            ))
    }
}
