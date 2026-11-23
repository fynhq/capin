use super::*;

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.chrome = ChromeTheme::for_window(window, Some(self.accent_hex));
        let image_rect = self.image_view_rect(window);
        let scale = f32::from(image_rect.size.width) / self.image_size.width as f32;
        self.text_edit_screen = self.text_edit.as_ref().map(|edit| Bounds {
            origin: point(
                image_rect.origin.x + px(edit.bounds.origin.x as f32 * scale),
                image_rect.origin.y + px(edit.bounds.origin.y as f32 * scale),
            ),
            size: size(
                px((edit.bounds.size.width as f32 * scale).max(24.0)),
                px((edit.bounds.size.height as f32 * scale).max(12.0)),
            ),
        });
        let objects = self.document.objects().to_vec();
        let crop_selection = self
            .crop_draft
            .as_ref()
            .map(|draft| normalized_rect(draft.start, draft.current));
        let preview = self.preview_shape();
        let preview_blur = preview.as_ref().and_then(|(shape, style)| match shape {
            Shape::Blur(rect) if !rect.is_empty() => Some((*rect, style.blur_radius)),
            _ => None,
        });
        let status = self.status.clone();
        let status_serial = self.status_serial;
        let accent_hex = self.accent_hex;

        let object_layers = self.object_layers(&objects, scale, cx);

        let overlay_canvas = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if let Some((shape, style)) = &preview {
                    paint_shape(window, bounds, scale, shape, *style);
                }
                if let Some(rect) = crop_selection {
                    paint_crop_overlay(window, bounds, scale, rect, accent_hex);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let text_overlay = self.text_edit.as_ref().map(|edit| {
            let left = edit.origin.x as f32 * scale;
            let top = edit.origin.y as f32 * scale;
            let font_size = edit.style.font_size.max(1.0) * scale;
            let width = (edit.bounds.size.width as f32 * scale).max(24.0);
            let line_height = font_size * 1.3;
            let color = edit.style.color;
            let text_rgb = (color >> 8) & 0x00FF_FFFF;
            let entity = cx.entity();
            let focus = self.focus_handle.clone();
            let input = canvas(
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
            .inset_0();
            let layouts = layout_annotation_text(window, &edit.content, font_size, width);
            let boxes = layouts
                .as_ref()
                .map(|layouts| text_line_boxes(layouts, line_height))
                .unwrap_or_default();
            let caret = text_caret_position(layouts.as_deref(), &boxes, edit, width);
            let selection = edit.selection();
            let selection_boxes = layouts
                .as_ref()
                .map(|layouts| {
                    text_selection_boxes(
                        layouts,
                        &boxes,
                        &selection,
                        edit.style.text_alignment,
                        line_height,
                        width,
                    )
                })
                .unwrap_or_default();
            let selection_color = rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x66);
            let mut layer = div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .min_h(px(line_height))
                .text_size(px(font_size))
                .line_height(px(line_height))
                .text_color(if edit.content.is_empty() {
                    rgb(accent_hex)
                } else {
                    rgb(text_rgb)
                })
                .text_align(match edit.style.text_alignment {
                    TextAlignment::Left => TextAlign::Left,
                    TextAlignment::Center => TextAlign::Center,
                    TextAlignment::Right => TextAlign::Right,
                });
            if edit.style.text_background {
                layer =
                    layer
                        .rounded(px(font_size * 0.2))
                        .bg(if prefers_dark_foreground(text_rgb) {
                            rgba(0x000000A0)
                        } else {
                            rgba(0xFFFFFFBE)
                        });
            }
            for rect in selection_boxes {
                layer = layer.child(
                    div()
                        .absolute()
                        .left(rect.origin.x)
                        .top(rect.origin.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .bg(selection_color),
                );
            }
            if !edit.content.is_empty() {
                layer = layer.child(edit.content.clone());
            }
            layer = layer.child(
                div()
                    .absolute()
                    .left(caret.x)
                    .top(caret.y)
                    .w(px(1.5))
                    .h(px(line_height))
                    .bg(if edit.content.is_empty() {
                        rgb(accent_hex)
                    } else {
                        rgb(text_rgb)
                    })
                    .with_animation(
                        SharedString::from("editor-text-caret"),
                        crate::ui::caret_blink(),
                        |caret, delta| caret.opacity(delta),
                    ),
            );
            layer
                .child(input)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        let index = this.text_index_at(window, event.position).unwrap_or(0);
                        if event.click_count >= 2 {
                            this.select_text_word(index, cx);
                        } else if let Some(edit) = this.text_edit.as_mut() {
                            edit.move_cursor(index, false);
                            edit.selecting = true;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        if event.pressed_button.is_none()
                            && let Some(edit) = this.text_edit.as_mut()
                        {
                            edit.selecting = false;
                        }
                        return;
                    }
                    if !this.text_edit.as_ref().is_some_and(|edit| edit.selecting) {
                        return;
                    }
                    let index = this.text_index_at(window, event.position).unwrap_or(0);
                    if let Some(edit) = this.text_edit.as_mut() {
                        edit.move_cursor(index, true);
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        if let Some(edit) = this.text_edit.as_mut() {
                            edit.selecting = false;
                            if edit.selection().is_empty() {
                                edit.anchor = edit.cursor;
                            }
                        }
                        cx.notify();
                    }),
                )
                .into_any_element()
        });

        let preview_blur_layer = preview_blur.and_then(|(rect, radius)| {
            let (placed, blur_image) = self.draft_blur_image(rect, radius, cx)?;
            Some(
                div()
                    .absolute()
                    .left(px(placed.origin.x as f32 * scale))
                    .top(px(placed.origin.y as f32 * scale))
                    .w(px(placed.size.width as f32 * scale))
                    .h(px(placed.size.height as f32 * scale))
                    .border_1()
                    .border_color(rgba((accent_hex << 8) | 0xCC))
                    .child(
                        img(blur_image)
                            .w(px(placed.size.width as f32 * scale))
                            .h(px(placed.size.height as f32 * scale))
                            .object_fit(ObjectFit::Fill),
                    )
                    .into_any_element(),
            )
        });

        let effects_active = self.region_effects_active();
        let inline = self.inline.is_some();
        let image_cursor = if self.move_draft.is_some() {
            crate::ui::closed_hand_cursor()
        } else {
            match self.selection_hover {
                SelectionHover::Handle(handle) => handle_cursor(handle),
                SelectionHover::Region(handle) => region_handle_cursor(handle),
                SelectionHover::Body => crate::ui::open_hand_cursor(),
                SelectionHover::None
                    if self.tool == Tool::Select
                        && self.inline.is_some_and(|inline| inline.zoomable) =>
                {
                    crate::ui::open_hand_cursor()
                }
                SelectionHover::None if self.tool == Tool::Select => CursorStyle::Arrow,
                SelectionHover::None => CursorStyle::Crosshair,
            }
        };
        let outside_backdrop = inline.then(|| {
            div().absolute().inset_0().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    if this.image_view_rect(window).contains(&event.position) {
                        cx.stop_propagation();
                        return;
                    }
                    if this.document.selected().is_some()
                        || this.more_open
                        || this.effects_open
                        || this.hints_visible
                    {
                        this.select_annotation(None);
                        this.more_open = false;
                        this.effects_open = false;
                        this.hints_visible = false;
                        this.selection_hover = SelectionHover::None;
                        cx.notify();
                    }
                }),
            )
        });
        let full_frame = self.live_selection_rect();
        let show_full_frame = self
            .full_canvas
            .is_some_and(|full_canvas| full_canvas.adjustable);
        let selection_clip = show_full_frame
            .then_some(full_frame)
            .flatten()
            .map(|rect| Bounds {
                origin: point(
                    px(rect.origin.x as f32 * scale),
                    px(rect.origin.y as f32 * scale),
                ),
                size: size(
                    px(rect.size.width as f32 * scale),
                    px(rect.size.height as f32 * scale),
                ),
            });
        let image_radius = if effects_active && !show_full_frame {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let frame_radius = if effects_active {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let frame_overlay = selection_clip.map(|clip| {
            let origin = image_rect.origin;
            let hole = Bounds {
                origin: point(
                    px(f32::from(clip.origin.x) + f32::from(origin.x)),
                    px(f32::from(clip.origin.y) + f32::from(origin.y)),
                ),
                size: clip.size,
            };
            let accent = rgba((accent_hex << 8) | 0xFF);
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    crate::ui::paint_rounded_mask(
                        window,
                        bounds,
                        hole,
                        frame_radius,
                        rgba(0x00000066),
                        Some((accent, 3.0)),
                    );
                },
            )
            .absolute()
            .inset_0()
            .size_full()
        });
        let annotation_content = div()
            .absolute()
            .left(px(
                selection_clip.map_or(0.0, |clip| -f32::from(clip.origin.x))
            ))
            .top(px(
                selection_clip.map_or(0.0, |clip| -f32::from(clip.origin.y))
            ))
            .w(image_rect.size.width)
            .h(image_rect.size.height)
            .children(object_layers)
            .children(preview_blur_layer)
            .child(overlay_canvas)
            .children(text_overlay);
        let annotation_layer = if let Some(clip) = selection_clip {
            div()
                .absolute()
                .left(clip.origin.x)
                .top(clip.origin.y)
                .w(clip.size.width)
                .h(clip.size.height)
                .overflow_hidden()
                .child(annotation_content)
        } else {
            annotation_content
        };
        let image_layer = div()
            .absolute()
            .left(image_rect.origin.x)
            .top(image_rect.origin.y)
            .w(image_rect.size.width)
            .h(image_rect.size.height)
            .cursor(image_cursor)
            .overflow_hidden()
            .occlude()
            .child(
                img(self.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(px(image_radius))
                    .object_fit(ObjectFit::Fill),
            )
            .child(annotation_layer)
            .children(frame_overlay)
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let zoomable = this.inline.is_some_and(|inline| inline.zoomable);
                if this.inline.is_some() && !zoomable {
                    cx.stop_propagation();
                    return;
                }
                let (x, y) = match event.delta {
                    ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
                    ScrollDelta::Lines(delta) => (delta.x * 50.0, delta.y * 50.0),
                };
                if zoomable {
                    let delta = if y.abs() >= x.abs() { y } else { x };
                    let steps = (delta / 50.0).clamp(-4.0, 4.0);
                    if steps != 0.0 {
                        this.zoom_at(1.15f32.powf(steps), event.position, window, cx);
                    }
                } else if event.modifiers.control || event.modifiers.platform {
                    let steps = (y / 50.0).clamp(-4.0, 4.0);
                    this.zoom_at(1.15f32.powf(steps), event.position, window, cx);
                } else {
                    this.view_pan.x += px(x);
                    this.view_pan.y += px(y);
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.right_click(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::handle_canvas_mouse_down),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.region_drag.is_some() {
                    this.update_region_drag(event.position, cx);
                    return;
                }
                if let Some(pan_draft) = &this.pan_draft {
                    this.view_pan.x =
                        pan_draft.start_pan.x + (event.position.x - pan_draft.start.x);
                    this.view_pan.y =
                        pan_draft.start_pan.y + (event.position.y - pan_draft.start.y);
                    cx.notify();
                    return;
                }
                let Some(point) = this.to_image_point(event.position, window) else {
                    return;
                };
                if this.move_draft.is_some() || this.resize_draft.is_some() {
                    let changed = if this.resize_draft.is_some() {
                        this.update_resize(point)
                    } else {
                        this.update_move(point)
                    };
                    if changed {
                        cx.notify();
                    }
                    return;
                }
                if this.update_selection_hover(point, window) {
                    cx.notify();
                }
                if this.crop_mode {
                    if let Some(crop_draft) = &mut this.crop_draft {
                        crop_draft.current = point;
                        cx.notify();
                    }
                    return;
                }
                if let Some(polyline) = &mut this.polyline_draft {
                    polyline.current = point;
                    cx.notify();
                    return;
                }
                let Some(draft) = &mut this.draft else {
                    return;
                };
                draft.current = point;
                draft.shift = event.modifiers.shift;
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.region_drag.is_some() {
                        this.finish_region_drag(cx);
                        cx.stop_propagation();
                        return;
                    }
                    if this.crop_mode {
                        if let Some(point) = this.to_image_point(event.position, window)
                            && let Some(crop_draft) = &mut this.crop_draft
                        {
                            crop_draft.current = point;
                        }
                        cx.notify();
                        return;
                    }
                    if this.polyline_draft.is_some() {
                        cx.notify();
                        return;
                    }
                    if this.pan_draft.take().is_some() {
                        cx.notify();
                        return;
                    }
                    if finish_pointer_gestures(this, event, window, cx) {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_exit(cx.listener(|this, _, _, cx| {
                let hover_reset = this.selection_hover != SelectionHover::None;
                this.selection_hover = SelectionHover::None;
                let mut changed = hover_reset;
                if this.move_draft.is_some() {
                    this.cancel_move();
                    changed = true;
                }
                if this.resize_draft.is_some() {
                    this.cancel_resize();
                    changed = true;
                }
                if this.draft.take().is_some() || this.pan_draft.take().is_some() {
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    if this.region_drag.is_some() {
                        this.update_region_drag(event.position, cx);
                        this.finish_region_drag(cx);
                    }
                }),
            );

        let region_bands = self.region_edge_bands(window, cx);
        let region_handles = self.region_handles(window, cx);
        let selection_layer = self.selection_layer(window);
        let (toolbar, options_panel, more_panel) = self.toolbar(window, cx);
        let info_bar = if inline {
            self.info_bar(window, cx)
        } else {
            None
        };
        let effects_panel = if inline {
            self.effects_panel(window, cx)
        } else {
            None
        };
        let hints_panel = self.hints_panel(window, cx);
        let custom_color_panel = self.custom_color_panel(window, cx);
        let status_toast = status.map(|message| {
            div()
                .absolute()
                .bottom(px(46.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(self.chrome.surface_strong)
                        .border_1()
                        .border_color(self.chrome.border)
                        .shadow(self.chrome.shadow())
                        .text_size(px(11.0))
                        .text_color(self.chrome.text)
                        .child(message),
                )
                .with_animation(
                    SharedString::from(format!("editor-status-{status_serial}")),
                    Animation::new(Duration::from_millis(120))
                        .with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |toast, delta| toast.opacity(delta).bottom(px(46.0 - (1.0 - delta) * 4.0)),
                )
        });

        div()
            .relative()
            .size_full()
            .when(!inline, |root| root.bg(self.chrome.canvas))
            .text_color(self.chrome.text)
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                if let Some(draft) = &mut this.draft {
                    draft.shift = event.modifiers.shift;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if this.region_drag.is_some() {
                    this.update_region_drag(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if let Some(edit) = this.text_edit.as_mut() {
                        edit.selecting = false;
                    }
                    if finish_pointer_gestures(this, event, window, cx) {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, _window, cx| {
                if this.region_drag.is_some() {
                    this.finish_region_drag(cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.right_click(cx);
                }),
            )
            .on_key_down(cx.listener(Self::handle_editor_key))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if event.keystroke.key == "space" {
                    this.space_down = false;
                    cx.notify();
                }
            }))
            .children(outside_backdrop)
            .child(image_layer)
            .children(region_bands)
            .children(region_handles)
            .children(selection_layer)
            .children(info_bar)
            .child(toolbar)
            .children(options_panel)
            .children(more_panel)
            .children(effects_panel)
            .children(custom_color_panel)
            .children(hints_panel)
            .children(status_toast)
            .children((!inline).then(|| {
                div()
                    .absolute()
                    .left(px(12.0))
                    .bottom(px(8.0))
                    .text_size(px(10.5))
                    .text_color(self.chrome.muted)
                    .child(format!(
                        "{} x {} · {:.0}%",
                        self.image_size.width,
                        self.image_size.height,
                        self.image_scale(window) * 100.0
                    ))
            }))
            .children((inline && self.toolbar_hidden).then(|| {
                let language = settings_state::get().language;
                div()
                    .id("editor-toolbar-recall")
                    .absolute()
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(12.0))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("editor-toolbar-recall-chip")
                            .occlude()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(self.chrome.surface_strong)
                            .border_1()
                            .border_color(self.chrome.border)
                            .shadow(self.chrome.shadow())
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .cursor_pointer()
                            .hover(|style| style.bg(self.chrome.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toolbar_hidden = false;
                                cx.notify();
                            }))
                            .child(format!("{} · Space", t(language, "editor.show_toolbar"))),
                    )
            }))
    }
}
