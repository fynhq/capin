use super::*;

fn stroke_id(width: f32) -> &'static str {
    match width as u32 {
        2 => "editor-width-2",
        4 => "editor-width-4",
        _ => "editor-width-8",
    }
}

impl EditorView {
    pub(super) fn toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>, Option<AnyElement>) {
        let language = settings_state::get().language;
        let chrome = self.chrome;
        let text = chrome.text;
        let muted = chrome.muted;
        let hover = chrome.hover;
        let border = chrome.border;

        let tools = self.tool_buttons(language, chrome, cx);
        let groups = self.toolbar_groups(language, chrome, cx);

        let mut config_children: Vec<AnyElement> = Vec::new();
        for (index, group) in groups.into_iter().enumerate() {
            if index > 0 {
                config_children.push(div().w(px(1.0)).h(px(18.0)).bg(border).into_any_element());
            }
            config_children.push(group);
        }

        let undo_label = t(language, "editor.undo").to_owned();
        let undo = self.toolbar_button(
            "editor-undo",
            undo_label,
            icons::UNDO,
            self.can_undo(),
            muted,
            cx,
            |this, cx| {
                this.undo_editor(cx);
                this.status = None;
                cx.notify();
            },
        );

        let redo_label = t(language, "editor.redo").to_owned();
        let redo = self.toolbar_button(
            "editor-redo",
            redo_label,
            icons::REDO,
            self.can_redo(),
            muted,
            cx,
            |this, cx| {
                this.redo_editor(cx);
                this.status = None;
                cx.notify();
            },
        );

        let delete_label = t(language, "editor.delete").to_owned();
        let delete = self.toolbar_button(
            "editor-delete",
            delete_label,
            icons::TRASH,
            self.document.selected().is_some(),
            text,
            cx,
            |this, cx| {
                this.commit_text(cx);
                this.delete_selected_annotation(cx);
            },
        );

        let pin_label = t(language, "editor.pin").to_owned();
        let pin = (self.pin_target.is_none()).then(|| {
            self.toolbar_button(
                "editor-pin",
                pin_label,
                icons::PIN,
                true,
                text,
                cx,
                |this, cx| this.pin(cx),
            )
        });

        let copy_label = t(language, "editor.copy").to_owned();
        let copy = self.toolbar_button(
            "editor-copy",
            copy_label,
            icons::COPY,
            true,
            text,
            cx,
            |this, cx| this.export(ExportAction::Copy, cx),
        );

        let save_label = t(language, "editor.save").to_owned();
        let save = self.toolbar_button(
            "editor-save",
            save_label,
            icons::SAVE,
            true,
            text,
            cx,
            |this, cx| this.export(ExportAction::Save, cx),
        );

        let capture = self.inline.and_then(|inline| inline.capture);
        let capture_actions_enabled = capture.is_some();
        let mut capture_actions: Vec<AnyElement> = Vec::new();
        if capture.is_some_and(|capture| crate::scroll::supports_target(capture.target)) {
            capture_actions.push(self.toolbar_button(
                "editor-scroll-capture",
                t(language, "menu.scroll_capture").to_owned(),
                icons::SCROLL,
                true,
                muted,
                cx,
                |this, cx| this.start_scroll_capture(cx),
            ));
        }
        if capture_actions_enabled {
            capture_actions.push(self.toolbar_button(
                "editor-record",
                t(language, "menu.record").to_owned(),
                icons::RECORDING,
                true,
                muted,
                cx,
                |this, cx| this.start_recording(cx),
            ));
        }

        let capture_page = capture_actions_enabled;
        let close_label = t(language, "editor.close").to_owned();
        let more_label = t(language, "editor.more").to_owned();
        let more = (!capture_page).then(|| {
            div()
                .id("editor-more")
                .tooltip(move |_, cx| tooltip_view(more_label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .when(self.toolbar.more_open, |button| button.bg(chrome.selected))
                .hover(move |style| style.bg(hover))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toolbar.more_open = !this.toolbar.more_open;
                    this.toolbar.effects_open = false;
                    this.toolbar.hints_visible = false;
                    cx.notify();
                }))
                .child(icons::icon(icons::MORE, 18.0, muted))
                .into_any_element()
        });

        let close = div()
            .id("editor-close")
            .tooltip(move |_, cx| tooltip_view(close_label.clone(), cx))
            .w(px(TOOLBAR_SIZE as f32))
            .h(px(TOOLBAR_SIZE as f32))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|this, _, window, cx| {
                this.release_images(cx);
                if this.inline.is_some() {
                    let handles = cx.windows();
                    cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
                } else {
                    window.remove_window();
                }
            }))
            .child(icons::icon(icons::CLOSE, 18.0, muted))
            .into_any_element();

        let bar_rect = self.toolbar_rect(window);
        let options_rect = self.options_rect(window, bar_rect);
        let options_panel = (!config_children.is_empty() && !self.toolbar.hidden).then(|| {
            let top = f32::from(options_rect.origin.y);
            let offset = if top >= f32::from(bar_rect.origin.y) + f32::from(bar_rect.size.height) {
                4.0
            } else {
                -4.0
            };
            crate::ui::enter_slide_fade(
                div()
                    .id("editor-options-panel")
                    .absolute()
                    .left(options_rect.origin.x)
                    .top(options_rect.origin.y)
                    .max_w(options_rect.size.width)
                    .h(options_rect.size.height)
                    .px(px(10.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(chrome.radius))
                    .bg(chrome.surface_strong)
                    .border_1()
                    .border_color(border)
                    .shadow(chrome.shadow())
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .children(config_children),
                "editor-options-enter",
                top,
                offset,
                Duration::from_millis(120),
            )
            .into_any_element()
        });

        let more_panel =
            (self.toolbar.more_open && !self.toolbar.hidden && !capture_page).then(|| {
                let more_rect = self.more_rect(window, bar_rect);
                let mut panel = div()
                    .id("editor-more-panel")
                    .absolute()
                    .left(more_rect.origin.x)
                    .top(more_rect.origin.y)
                    .w(more_rect.size.width)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .p(px(5.0))
                    .rounded(px(chrome.radius))
                    .bg(chrome.surface_strong)
                    .border_1()
                    .border_color(border)
                    .shadow(chrome.shadow())
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .child(self.more_row(
                        "editor-more-crop",
                        icons::CROP,
                        t(language, "editor.crop"),
                        cx,
                        |this, cx| this.toggle_crop(cx),
                    ));

                if self.pin_target.is_some() {
                    panel = panel
                        .child(self.more_row(
                            "editor-more-rotate",
                            icons::ROTATE,
                            t(language, "editor.rotate"),
                            cx,
                            |this, cx| this.rotate_canvas(cx),
                        ))
                        .child(self.more_row(
                            "editor-more-flip-horizontal",
                            icons::FLIP_H,
                            t(language, "editor.flip_horizontal"),
                            cx,
                            |this, cx| this.flip_canvas(true, cx),
                        ))
                        .child(self.more_row(
                            "editor-more-flip-vertical",
                            icons::FLIP_V,
                            t(language, "editor.flip_vertical"),
                            cx,
                            |this, cx| this.flip_canvas(false, cx),
                        ));
                }
                let top = f32::from(more_rect.origin.y);
                let offset =
                    if top >= f32::from(bar_rect.origin.y) + f32::from(bar_rect.size.height) {
                        4.0
                    } else {
                        -4.0
                    };
                crate::ui::enter_slide_fade(
                    panel.h(more_rect.size.height),
                    "editor-more-enter",
                    top,
                    offset,
                    Duration::from_millis(120),
                )
            });

        let more_panel = more_panel.map(|panel| {
            div()
                .id("editor-more-layer")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.toolbar.more_open = false;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element()
        });

        if self.toolbar.hidden {
            return (div().into_any_element(), None, more_panel);
        }

        let bar = div()
            .id("editor-toolbar")
            .absolute()
            .left(bar_rect.origin.x)
            .top(bar_rect.origin.y)
            .w(bar_rect.size.width)
            .h(bar_rect.size.height)
            .px(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .rounded(px(chrome.radius))
            .bg(chrome.surface)
            .border_1()
            .border_color(border)
            .shadow(chrome.shadow())
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(1.0))
                    .children(tools)
                    .child(div().w(px(5.0)))
                    .child(div().w(px(1.0)).h(px(18.0)).bg(border))
                    .child(div().w(px(5.0)))
                    .child(undo)
                    .child(redo)
                    .child(delete),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(1.0))
                    .children(pin)
                    .child(copy)
                    .child(save)
                    .children(capture_actions)
                    .children(more)
                    .child(close),
            );
        (bar.into_any_element(), options_panel, more_panel)
    }
}

impl EditorView {
    fn tool_buttons(
        &self,
        language: crate::i18n::Language,
        chrome: ChromeTheme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let muted = chrome.muted;
        let accent = chrome.accent;
        let accent_selected = chrome.accent_tint;
        let hover = chrome.hover;
        let mut tools = Vec::new();
        for tool in Tool::ALL {
            let selected = self.tool == tool;
            let label = t(language, tool.label_key()).to_owned();
            let button = div()
                .id(tool.id())
                .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                .w(px(TOOLBAR_SIZE as f32))
                .h(px(TOOLBAR_SIZE as f32))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |button| button.bg(accent_selected))
                .when(!selected, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.document.select(None);
                    this.close_custom_color();
                    this.crop_mode = false;
                    this.crop_draft = None;
                    this.polyline_draft = None;
                    this.tool = tool;
                    if let Some(color_tool) = tool.color_tool() {
                        this.active_color_tool = color_tool;
                        this.style.color = this.tool_color(color_tool);
                    }
                    match tool {
                        Tool::Rect => {
                            this.style.stroke_width = this.rect_stroke_width;
                            this.style.corner_radius = this.corner_radius * this.ui_scale;
                            this.style.filled = if this.shape_ellipse {
                                this.ellipse_filled
                            } else {
                                this.rect_filled
                            };
                        }
                        Tool::Arrow => this.style.stroke_width = this.arrow_stroke_width,
                        Tool::Highlighter => {
                            this.style.highlighter_radius = this.highlighter_radius * this.ui_scale;
                        }
                        Tool::Blur => this.style.blur_radius = this.blur_radius,
                        Tool::Spotlight => {
                            this.style.spotlight_intensity = this.spotlight_intensity;
                            this.style.spotlight_radius = this.spotlight_radius * this.ui_scale;
                        }
                        _ => {}
                    }
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    tool.icon(),
                    18.0,
                    if selected { accent } else { muted },
                ));
            tools.push(button.into_any_element());
        }

        tools
    }
}

impl EditorView {
    fn toolbar_groups(
        &self,
        language: crate::i18n::Language,
        chrome: ChromeTheme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let text = chrome.text;
        let muted = chrome.muted;
        let accent = chrome.accent;
        let accent_selected = chrome.accent_tint;
        let hover = chrome.hover;
        let border = chrome.border;
        let config_tool = self.config_tool();
        let selected_style = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.style);
        let config_style = selected_style.unwrap_or(self.style);
        let config_ellipse = self.config_shape_ellipse();
        let config_filled = match config_tool {
            Some(Tool::Rect) => selected_style.map_or(
                if config_ellipse {
                    self.ellipse_filled
                } else {
                    self.rect_filled
                },
                |style| style.filled,
            ),
            _ => false,
        };
        let config_intensity =
            selected_style.map_or(self.spotlight_intensity, |style| style.spotlight_intensity);
        let show_color = config_tool.and_then(Tool::color_tool).is_some();
        let show_fill = config_tool == Some(Tool::Rect);
        let show_width = match config_tool {
            Some(Tool::Arrow) => true,
            Some(Tool::Rect) => !config_filled,
            _ => false,
        };
        let show_font = config_tool == Some(Tool::Text);
        let show_text_style = config_tool == Some(Tool::Text);
        let show_arrow_heads = config_tool == Some(Tool::Arrow);
        let show_corner = config_tool == Some(Tool::Rect) && !config_ellipse;
        let show_highlighter = config_tool == Some(Tool::Highlighter);
        let show_number = config_tool == Some(Tool::Number);
        let show_blur = config_tool == Some(Tool::Blur);
        let show_spotlight = config_tool == Some(Tool::Spotlight);
        let size_scale = self.ui_scale.max(0.1);

        let mut groups: Vec<AnyElement> = Vec::new();
        if show_color {
            let palette_color = self.palette_color();
            let highlighter_palette = self.palette_color_tool() == AnnotationColorTool::Highlighter;
            let mut swatches = Vec::new();
            for (color, name, label_key) in settings_state::ANNOTATION_COLORS {
                let selected = (palette_color & 0xFFFFFF00) == (color & 0xFFFFFF00);
                let label = t(language, label_key).to_owned();
                let preview = if highlighter_palette {
                    core_annotate::highlighter_color(color)
                } else {
                    color
                };
                let swatch = div()
                    .id(name)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded(px(9.0))
                    .bg(rgb((preview >> 8) & 0x00FF_FFFF))
                    .border_2()
                    .border_color(if selected { text } else { rgba(0x00000000) })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_color(color);
                        this.status = None;
                        cx.notify();
                    }));
                swatches.push(swatch.into_any_element());
            }
            let custom_label = t(language, "editor.custom_color").to_owned();
            let custom_active = self.toolbar.custom_color_open
                || !settings_state::ANNOTATION_COLORS
                    .iter()
                    .any(|(color, _, _)| (palette_color & 0xFFFFFF00) == (color & 0xFFFFFF00));
            swatches.push(
                div()
                    .id("editor-color-custom")
                    .tooltip(move |_, cx| tooltip_view(custom_label.clone(), cx))
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded(px(9.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if custom_active {
                        accent_selected
                    } else {
                        chrome.surface
                    })
                    .border_2()
                    .border_color(if custom_active { text } else { border })
                    .when(!custom_active, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.commit_text(cx);
                        if this.toolbar.custom_color_open {
                            this.close_custom_color();
                        } else {
                            this.open_custom_color();
                            this.toolbar.more_open = false;
                            this.toolbar.effects_open = false;
                            this.toolbar.hints_visible = false;
                        }
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icons::PALETTE,
                        12.0,
                        if custom_active { accent } else { muted },
                    ))
                    .into_any_element(),
            );
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .children(swatches)
                    .into_any_element(),
            );
        }

        if config_tool == Some(Tool::Rect) {
            let mut options = Vec::new();
            for (ellipse, id, label_key, icon) in [
                (false, "editor-shape-rect", "editor.tool.rect", icons::RECT),
                (
                    true,
                    "editor-shape-ellipse",
                    "editor.tool.ellipse",
                    icons::ELLIPSE,
                ),
            ] {
                let selected = config_ellipse == ellipse;
                let label = t(language, label_key).to_owned();
                let button = div()
                    .id(id)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.document.select(None);
                        this.apply_shape_ellipse(ellipse);
                        this.style.filled = if ellipse {
                            this.ellipse_filled
                        } else {
                            this.rect_filled
                        };
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icon,
                        16.0,
                        if selected { accent } else { muted },
                    ));
                options.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(options)
                    .into_any_element(),
            );
        }

        if show_fill {
            let mut options = Vec::new();
            for (filled, id, label_key, icon) in [
                (
                    false,
                    "editor-fill-outline",
                    "editor.outline",
                    icons::OUTLINE,
                ),
                (true, "editor-fill-solid", "editor.fill", icons::FILL),
            ] {
                let selected = config_filled == filled;
                let label = t(language, label_key).to_owned();
                let button = div()
                    .id(id)
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_fill(filled);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icon,
                        16.0,
                        if selected { accent } else { muted },
                    ));
                options.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(options)
                    .into_any_element(),
            );
        }

        if show_width {
            let mut widths = Vec::new();
            for width in STROKE_WIDTHS {
                let selected = (config_style.stroke_width - width).abs() < 0.1;
                let label = format!("{width:.0}");
                let button = div()
                    .id(stroke_id(width))
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |button| button.bg(accent_selected))
                    .when(!selected, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_text(cx);
                        this.apply_stroke_width(width);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(width))
                            .rounded(px(width / 2.0))
                            .bg(if selected { accent } else { muted }),
                    );
                widths.push(button.into_any_element());
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(widths)
                    .into_any_element(),
            );
        }

        if show_font {
            let mut sizes = Vec::new();
            for size in FONT_SIZES {
                let selected = (config_style.font_size - size).abs() < 0.1;
                let label = format!("{size:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.font_size"));
                sizes.push(self.label_option_chip(
                    format!("editor-font-{label}"),
                    label,
                    tooltip,
                    selected,
                    24.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_font_size(size);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::TEXT, 14.0, muted))
                    .children(sizes)
                    .into_any_element(),
            );
        }

        if show_text_style {
            let background_enabled = config_style.text_background;
            let outline_enabled = config_style.text_outline;
            let alignment = config_style.text_alignment;
            let alignment_label = match alignment {
                TextAlignment::Left => t(language, "editor.align_left"),
                TextAlignment::Center => t(language, "editor.align_center"),
                TextAlignment::Right => t(language, "editor.align_right"),
            };
            let background_tooltip = t(language, "editor.text_background").to_owned();
            let outline_tooltip = t(language, "editor.text_outline").to_owned();
            let alignment_tooltip = t(language, "editor.text_alignment").to_owned();
            let background = div()
                .id("editor-text-background")
                .tooltip(move |_, cx| tooltip_view(background_tooltip.clone(), cx))
                .w(px(24.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .when(background_enabled, |button| button.bg(accent_selected))
                .when(!background_enabled, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.commit_text(cx);
                    let enabled = !this.text_background;
                    this.apply_text_background(enabled);
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    icons::FILL,
                    15.0,
                    if background_enabled { accent } else { muted },
                ))
                .into_any_element();
            let outline = div()
                .id("editor-text-outline")
                .tooltip(move |_, cx| tooltip_view(outline_tooltip.clone(), cx))
                .w(px(24.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .when(outline_enabled, |button| button.bg(accent_selected))
                .when(!outline_enabled, |button| {
                    button.hover(move |style| style.bg(hover))
                })
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.commit_text(cx);
                    let enabled = !this.text_outline;
                    this.apply_text_outline(enabled);
                    this.status = None;
                    cx.notify();
                }))
                .child(icons::icon(
                    icons::OUTLINE,
                    15.0,
                    if outline_enabled { accent } else { muted },
                ))
                .into_any_element();
            let next_alignment = match alignment {
                TextAlignment::Left => TextAlignment::Center,
                TextAlignment::Center => TextAlignment::Right,
                TextAlignment::Right => TextAlignment::Left,
            };
            let alignment_button = div()
                .id("editor-text-alignment")
                .tooltip(move |_, cx| tooltip_view(alignment_tooltip.clone(), cx))
                .w(px(28.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(cx);
                    this.apply_text_alignment(next_alignment);
                    this.status = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(accent)
                        .child(alignment_label),
                )
                .into_any_element();
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(background)
                    .child(outline)
                    .child(alignment_button)
                    .into_any_element(),
            );
        }

        if show_arrow_heads {
            let mut buttons = Vec::new();
            for (is_start, enabled, label, tooltip_key, id) in [
                (
                    true,
                    config_style.arrow_start,
                    "←",
                    "editor.arrow_start",
                    "editor-arrow-start",
                ),
                (
                    false,
                    config_style.arrow_end,
                    "→",
                    "editor.arrow_end",
                    "editor-arrow-end",
                ),
            ] {
                let tooltip = t(language, tooltip_key).to_owned();
                let button = div()
                    .id(id)
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(enabled, |button| button.bg(accent_selected))
                    .when(!enabled, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let start = if is_start {
                            !enabled
                        } else {
                            config_style.arrow_start
                        };
                        let end = if is_start {
                            config_style.arrow_end
                        } else {
                            !enabled
                        };
                        this.apply_arrow_heads(start, end);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(if enabled { accent } else { muted })
                            .child(label),
                    );
                buttons.push(button.into_any_element());
            }
            let polyline = self.config_arrow_polyline();
            let tooltip = t(language, "editor.tool.polyline").to_owned();
            buttons.push(
                div()
                    .id("editor-arrow-polyline")
                    .tooltip(move |_, cx| tooltip_view(tooltip.clone(), cx))
                    .w(px(24.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(polyline, |button| button.bg(accent_selected))
                    .when(!polyline, |button| {
                        button.hover(move |style| style.bg(hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.document.select(None);
                        this.apply_arrow_polyline(!polyline);
                        this.status = None;
                        cx.notify();
                    }))
                    .child(icons::icon(
                        icons::POLYLINE,
                        14.0,
                        if polyline { accent } else { muted },
                    ))
                    .into_any_element(),
            );
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .children(buttons)
                    .into_any_element(),
            );
        }

        if show_corner {
            let mut radii = Vec::new();
            for radius in CORNER_RADII {
                let applied = (radius * size_scale).round();
                let selected = (config_style.corner_radius - applied).abs() < 0.1;
                let label = format!("{radius:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.corner_radius"));
                radii.push(self.label_option_chip(
                    format!("editor-corner-{label}"),
                    label,
                    tooltip,
                    selected,
                    22.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_corner_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::RECT, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }

        if show_highlighter {
            let mut radii = Vec::new();
            for radius in CORNER_RADII {
                let applied = (radius * size_scale).round();
                let selected = (config_style.highlighter_radius - applied).abs() < 0.1;
                let label = format!("{radius:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.highlighter_radius"));
                radii.push(self.label_option_chip(
                    format!("editor-highlighter-radius-{label}"),
                    label,
                    tooltip,
                    selected,
                    22.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_highlighter_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::HIGHLIGHTER, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }

        if show_number {
            let current = self
                .document
                .selected()
                .and_then(|id| self.document.object(id))
                .and_then(|annotation| match annotation.shape {
                    Shape::Number { bounds, .. } => Some(bounds.size.height),
                    _ => None,
                })
                .unwrap_or(((self.number_size as f32 * size_scale).round() as u32).max(8));
            let mut sizes = Vec::new();
            for size in NUMBER_SIZES {
                let applied = ((size as f32 * size_scale).round() as u32).max(8);
                let selected = current.abs_diff(applied) <= 1;
                let label = format!("{size}");
                let tooltip = format!("{} {label}", t(language, "editor.number_size"));
                sizes.push(self.label_option_chip(
                    format!("editor-number-{label}"),
                    label,
                    tooltip,
                    selected,
                    24.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_number_size(size);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::NUMBER, 14.0, muted))
                    .children(sizes)
                    .into_any_element(),
            );
        }

        if show_blur {
            let mut radii = Vec::new();
            for radius in BLUR_RADII {
                let selected = config_style.blur_radius == radius;
                let label = format!("{radius}");
                let tooltip = format!("{} {label}", t(language, "editor.blur_radius"));
                radii.push(self.label_option_chip(
                    format!("editor-blur-{label}"),
                    label,
                    tooltip,
                    selected,
                    24.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_blur_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::BLUR, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }

        if show_spotlight {
            let mut levels = Vec::new();
            for intensity in SPOTLIGHT_INTENSITIES {
                let selected = config_intensity == intensity;
                let label = format!("{intensity}%");
                let tooltip = format!("{} {label}", t(language, "editor.intensity"));
                levels.push(self.label_option_chip(
                    format!("editor-spotlight-{intensity}"),
                    label,
                    tooltip,
                    selected,
                    32.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_spotlight_intensity(intensity);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::SPOTLIGHT, 14.0, muted))
                    .children(levels)
                    .into_any_element(),
            );

            let mut radii = Vec::new();
            for radius in CORNER_RADII {
                let applied = (radius * size_scale).round();
                let selected = (config_style.spotlight_radius - applied).abs() < 0.1;
                let label = format!("{radius:.0}");
                let tooltip = format!("{} {label}", t(language, "editor.spotlight_radius"));
                radii.push(self.label_option_chip(
                    format!("editor-spotlight-radius-{label}"),
                    label,
                    tooltip,
                    selected,
                    22.0,
                    cx,
                    move |this, cx| {
                        this.commit_text(cx);
                        this.apply_spotlight_radius(radius);
                        this.status = None;
                        cx.notify();
                    },
                ));
            }
            groups.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(icons::icon(icons::RECT, 14.0, muted))
                    .children(radii)
                    .into_any_element(),
            );
        }
        groups
    }
}
