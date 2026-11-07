use core_types::RgbaFrame;
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::system_accent_hex;

use crate::i18n::{Language, t};
use crate::settings_state;

const BACKGROUND: u32 = 0x141414;
const TEXT: u32 = 0xF2F2F2;
const MUTED: u32 = 0x9A9A9A;
const BORDER: u32 = 0xFFFFFF1A;
const CARD: u32 = 0x1F1F1F;
const FALLBACK_ACCENT: u32 = 0x4c9aff;

pub fn start(frame: RgbaFrame, cx: &mut App) {
    let bounds = crate::ui::centered_bounds(cx, 620.0, 480.0);
    let display_id = cx.primary_display().map(|display| display.id());
    let language = settings_state::get().language;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(t(language, "ocr.title").into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        display_id,
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(420.0), px(320.0))),
        app_id: Some("capin-ocr".to_owned()),
        ..Default::default()
    };

    match gpui_kit::open_window(options, cx, move |window, cx| {
        let view = cx.new(|cx| OcrView::new(frame, cx));
        let focus = view.read(cx).focus_handle.clone();
        window.defer(cx, move |window, cx| {
            window.focus(&focus, cx);
            window.activate_window();
        });
        view
    }) {
        Ok((handle, _)) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
        }
        Err(error) => eprintln!("capin: failed to open OCR result: {error}"),
    }
}

struct OcrView {
    text: String,
    qr_codes: Vec<String>,
    error: Option<String>,
    loading: bool,
    copied: Option<String>,
    language: Language,
    accent: Rgba,
    focus_handle: FocusHandle,
}

impl OcrView {
    fn new(frame: RgbaFrame, cx: &mut Context<Self>) -> Self {
        let language = settings_state::get().language;
        cx.spawn(async move |this, cx| {
            let output = cx
                .background_executor()
                .spawn(async move { platform_ocr::recognize(frame) })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                this.text = output.text;
                this.qr_codes = output.qr_codes;
                this.error = output.error;
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            text: String::new(),
            qr_codes: Vec::new(),
            error: None,
            loading: true,
            copied: None,
            language,
            accent: rgb(system_accent_hex().unwrap_or(FALLBACK_ACCENT)),
            focus_handle: cx.focus_handle(),
        }
    }

    fn copy_text(&mut self, cx: &mut Context<Self>) {
        if self.text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.text.clone()));
        self.copied = Some(t(self.language, "ocr.copied").to_owned());
        cx.notify();
    }

    fn copy_qr(&mut self, cx: &mut Context<Self>) {
        if self.qr_codes.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.qr_codes.join("\n")));
        self.copied = Some(t(self.language, "ocr.copied").to_owned());
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let handles = cx.window_stack().unwrap_or_default();
        cx.defer(move |cx| {
            crate::windows::close_auxiliary(cx, handles);
        });
    }

    fn section_title(&self, title: String, muted: Rgba, copy: Option<impl IntoElement>) -> Div {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(div().text_size(px(12.0)).text_color(muted).child(title));
        if let Some(copy) = copy {
            row = row.child(copy);
        }
        row
    }
}

impl Render for OcrView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = rgba(MUTED);
        let accent = self.accent;
        let language = self.language;
        let copied = self.copied.clone();

        let copy_text = (!self.text.is_empty()).then(|| {
            crate::ui::flat_button(
                "ocr-copy-text",
                t(language, "ocr.copy_text").to_owned(),
                accent,
                cx,
                Self::copy_text,
            )
        });
        let copy_qr = (!self.qr_codes.is_empty()).then(|| {
            crate::ui::flat_button(
                "ocr-copy-qr",
                t(language, "ocr.copy_qr").to_owned(),
                accent,
                cx,
                Self::copy_qr,
            )
        });
        let close = crate::ui::flat_button(
            "ocr-close",
            t(language, "ocr.close").to_owned(),
            muted,
            cx,
            Self::close,
        );

        let body = if self.loading {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .text_color(accent)
                .child(t(language, "ocr.scanning"))
                .into_any_element()
        } else {
            let mut sections = Vec::new();
            if let Some(error) = &self.error {
                sections.push(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgba(0xFF7A7A))
                        .child(format!("{}{}", t(language, "ocr.failed"), error))
                        .into_any_element(),
                );
            }
            if !self.text.is_empty() {
                sections.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(self.section_title(
                            t(language, "ocr.text").to_owned(),
                            muted,
                            copy_text,
                        ))
                        .child(
                            div()
                                .w_full()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(rgba(BORDER))
                                .bg(rgb(CARD))
                                .p(px(10.0))
                                .text_size(px(13.0))
                                .line_height(px(20.0))
                                .text_color(rgb(TEXT))
                                .child(self.text.clone()),
                        )
                        .into_any_element(),
                );
            }
            if !self.qr_codes.is_empty() {
                sections.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(self.section_title(t(language, "ocr.qr").to_owned(), muted, copy_qr))
                        .child(
                            div()
                                .w_full()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(rgba(BORDER))
                                .bg(rgb(CARD))
                                .p(px(10.0))
                                .text_size(px(13.0))
                                .line_height(px(20.0))
                                .text_color(rgb(TEXT))
                                .child(self.qr_codes.join("\n\n")),
                        )
                        .into_any_element(),
                );
            }
            if sections.is_empty() {
                sections.push(
                    div()
                        .text_size(px(13.0))
                        .text_color(muted)
                        .child(t(language, "ocr.no_result"))
                        .into_any_element(),
                );
            }
            div()
                .id("ocr-results")
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(14.0))
                .children(sections)
                .into_any_element()
        };

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(16.0))
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let command =
                    event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
                if key == "escape" {
                    window.remove_window();
                } else if command && key == "c" {
                    this.copy_text(cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t(language, "ocr.title")),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(muted)
                                    .child(t(language, "ocr.subtitle")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .children(copied.map(|message| {
                                div().text_size(px(11.0)).text_color(accent).child(message)
                            }))
                            .child(close),
                    ),
            )
            .child(body)
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(0xFFFFFF66))
                    .child(t(language, "ocr.hint")),
            )
    }
}
