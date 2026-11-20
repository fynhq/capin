use super::*;

fn blur_image_index(radius: u32) -> usize {
    BLUR_RADII
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| candidate.abs_diff(radius))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

impl EditorView {
    pub(super) fn image_scale(&self, window: &Window) -> f32 {
        (f32::from(self.image_view_rect(window).size.width) / self.image_size.width.max(1) as f32)
            .max(0.01)
    }

    pub(super) fn update_selection_hover(&mut self, point: PhysicalPoint, window: &Window) -> bool {
        let scale = self.image_scale(window);
        let handle = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .and_then(|annotation| annotation_handle_at(&annotation.shape, point, scale));
        let hit = self.document.hit_test(point);
        let hover = if let Some(handle) = handle {
            SelectionHover::Handle(handle)
        } else if hit.is_some() {
            SelectionHover::Body
        } else if let Some(handle) = self.region_handle_at(point) {
            SelectionHover::Region(handle)
        } else {
            SelectionHover::None
        };
        if hover == self.selection_hover {
            return false;
        }
        self.selection_hover = hover;
        true
    }

    pub(super) fn fit_scale(&self, window: &Window) -> f32 {
        (self.image_scale(window) / self.view_zoom.max(0.01)).max(0.01)
    }

    pub(crate) fn inline_selection_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let inline = self.inline?;
        let capture = inline.capture?;
        let image = self.image_view_rect(window);
        if self.full_canvas.is_some() {
            let selection = self.live_selection_rect().unwrap_or(capture.region.rect);
            return full_selection_bounds(image, self.image_size, selection);
        }
        let scale = capture.scale.max(0.1);
        let width = capture.region.rect.size.width as f32 / scale;
        let height = capture.region.rect.size.height as f32 / scale;
        let center_x = f32::from(image.origin.x) + f32::from(image.size.width) / 2.0;
        let center_y = f32::from(image.origin.y) + f32::from(image.size.height) / 2.0;
        Some(Bounds {
            origin: point(px(center_x - width / 2.0), px(center_y - height / 2.0)),
            size: size(px(width), px(height)),
        })
    }

    pub(super) fn live_selection_rect(&self) -> Option<PhysicalRect> {
        self.region_drag
            .filter(|_| self.full_canvas.is_some())
            .map(|draft| draft.current)
            .or_else(|| self.full_selection())
    }

    pub(super) fn full_selection(&self) -> Option<PhysicalRect> {
        self.full_canvas.as_ref()?;
        Some(self.inline?.capture?.region.rect)
    }

    pub(super) fn action_document(&self) -> AnnotationDocument {
        let Some(selection) = self.full_selection() else {
            return self.document.clone();
        };
        cropped_document(&self.document, selection)
    }

    pub(super) fn full_selection_screen_bounds(&self) -> Option<Bounds<Pixels>> {
        let selection = self.full_selection()?;
        let inline = self.inline?;
        full_selection_bounds(inline.image_rect, self.image_size, selection)
    }

    pub(super) fn toolbar_anchor(&self, window: &Window) -> Bounds<Pixels> {
        self.inline_selection_rect(window)
            .unwrap_or_else(|| self.image_view_rect(window))
    }

    pub(super) fn toolbar_rect(&self, window: &Window) -> Bounds<Pixels> {
        let anchor = self.toolbar_anchor(window);
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = COMPACT_TOOLBAR_WIDTH.min((window_width - 16.0).max(120.0));
        let anchor_left = f32::from(anchor.origin.x);
        let anchor_top = f32::from(anchor.origin.y);
        let anchor_right = anchor_left + f32::from(anchor.size.width);
        let anchor_bottom = anchor_top + f32::from(anchor.size.height);
        let left = ((anchor_left + anchor_right) / 2.0 - width / 2.0)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let below = anchor_bottom + 8.0;
        let above = anchor_top - COMPACT_TOOLBAR_HEIGHT - 8.0;
        let top = if below + COMPACT_TOOLBAR_HEIGHT <= window_height - 8.0 {
            below
        } else if above >= 8.0 {
            above
        } else {
            (window_height - COMPACT_TOOLBAR_HEIGHT - 8.0).max(8.0)
        };
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(COMPACT_TOOLBAR_HEIGHT)),
        }
    }

    pub(super) fn options_rect(&self, window: &Window, toolbar: Bounds<Pixels>) -> Bounds<Pixels> {
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 560.0_f32.min((window_width - 16.0).max(120.0));
        let left = f32::from(toolbar.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let toolbar_top = f32::from(toolbar.origin.y);
        let toolbar_bottom = toolbar_top + f32::from(toolbar.size.height);
        let anchor = self.toolbar_anchor(window);
        let anchor_bottom = f32::from(anchor.origin.y) + f32::from(anchor.size.height);
        let toolbar_below = toolbar_top >= anchor_bottom;
        let below = toolbar_bottom + 6.0;
        let above = toolbar_top - OPTIONS_PANEL_HEIGHT - 6.0;
        let (preferred, fallback) = if toolbar_below {
            (below, above)
        } else {
            (above, below)
        };
        let fits = |top: f32| top >= 8.0 && top + OPTIONS_PANEL_HEIGHT <= window_height - 8.0;
        let top = if fits(preferred) {
            preferred
        } else if fits(fallback) {
            fallback
        } else {
            preferred.clamp(8.0, (window_height - OPTIONS_PANEL_HEIGHT - 8.0).max(8.0))
        };
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(OPTIONS_PANEL_HEIGHT)),
        }
    }

    pub(super) fn info_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let anchor = self.inline_selection_rect(window)?;
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = INFO_BAR_WIDTH.min((window_width - 16.0).max(120.0));
        let left = f32::from(anchor.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let toolbar = self.toolbar_rect(window);
        let anchor_top = f32::from(anchor.origin.y);
        let anchor_bottom = anchor_top + f32::from(anchor.size.height);
        let toolbar_top = f32::from(toolbar.origin.y);
        let toolbar_bottom = toolbar_top + f32::from(toolbar.size.height);
        let options_visible = self.config_tool().is_some() && !self.toolbar_hidden;
        let options = self.options_rect(window, toolbar);
        let options_top = f32::from(options.origin.y);
        let options_bottom = options_top + f32::from(options.size.height);
        let fits = |top: f32| top >= 8.0 && top + INFO_BAR_HEIGHT <= window_height - 8.0;
        let overlaps_toolbar = |top: f32| {
            !self.toolbar_hidden && top < toolbar_bottom && top + INFO_BAR_HEIGHT > toolbar_top
        };
        let above = anchor_top - INFO_BAR_HEIGHT - 8.0;
        let below = anchor_bottom + 8.0;
        let mut candidates = Vec::new();
        if fits(above) && !overlaps_toolbar(above) {
            candidates.push(above);
        }
        if toolbar_top >= anchor_bottom {
            let outside = if options_visible {
                options_bottom + 6.0
            } else {
                toolbar_bottom + 6.0
            };
            if fits(outside) {
                candidates.push(outside);
            }
        }
        if toolbar_bottom <= anchor_top {
            let outside = if options_visible {
                options_top - INFO_BAR_HEIGHT - 6.0
            } else {
                toolbar_top - INFO_BAR_HEIGHT - 6.0
            };
            if fits(outside) {
                candidates.push(outside);
            }
        }
        if fits(below) && !overlaps_toolbar(below) {
            candidates.push(below);
        }
        if fits(anchor_top + 8.0) {
            candidates.push(anchor_top + 8.0);
        }
        if fits(anchor_bottom - INFO_BAR_HEIGHT - 8.0) {
            candidates.push(anchor_bottom - INFO_BAR_HEIGHT - 8.0);
        }
        let top = candidates
            .first()
            .copied()
            .unwrap_or(8.0)
            .clamp(8.0, (window_height - INFO_BAR_HEIGHT - 8.0).max(8.0));
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(INFO_BAR_HEIGHT)),
        })
    }

    pub(super) fn more_rect(&self, window: &Window, toolbar: Bounds<Pixels>) -> Bounds<Pixels> {
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 196.0;
        let height = if self.pin_target.is_some() {
            136.0
        } else {
            42.0
        };
        let left = (f32::from(toolbar.origin.x) + f32::from(toolbar.size.width) - width)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let mut top = f32::from(toolbar.origin.y) + f32::from(toolbar.size.height) + 6.0;
        if top + height > window_height - 8.0 {
            top = (f32::from(toolbar.origin.y) - height - 6.0).max(8.0);
        }
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        }
    }

    pub(super) fn effects_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let info = self.info_rect(window)?;
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = crate::capture_panels::effects_panel_width(settings_state::get().language)
            .min((window_width - 16.0).max(120.0));
        let height = 40.0;
        let left = f32::from(info.origin.x).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let top =
            crate::ui::popover_top(info, height, window_height, Some(self.toolbar_rect(window)));
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        })
    }

    pub(super) fn custom_color_rect(&self, window: &Window) -> Option<Bounds<Pixels>> {
        if !self.custom_color_open {
            return None;
        }
        let toolbar = self.toolbar_rect(window);
        let options = self.options_rect(window, toolbar);
        let window_width = f32::from(window.viewport_size().width);
        let window_height = f32::from(window.viewport_size().height);
        let width = 236.0_f32.min((window_width - 16.0).max(120.0));
        let height = 116.0;
        let left = ((f32::from(options.origin.x) + f32::from(options.size.width) / 2.0)
            - width / 2.0)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let options_below_toolbar = f32::from(options.origin.y)
            >= f32::from(toolbar.origin.y) + f32::from(toolbar.size.height);
        let preferred = if options_below_toolbar {
            f32::from(options.origin.y) + f32::from(options.size.height) + 6.0
        } else {
            f32::from(options.origin.y) - height - 6.0
        };
        let fits = preferred >= 8.0 && preferred + height <= window_height - 8.0;
        let top = if fits {
            preferred
        } else {
            crate::ui::popover_top(options, height, window_height, Some(toolbar))
        };
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        })
    }

    pub(super) fn custom_color_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.custom_color_open || self.toolbar_hidden {
            return None;
        }
        let rect = self.custom_color_rect(window)?;
        let language = settings_state::get().language;
        let palette_color = self.palette_color();
        let preview_rgb = self
            .custom_color_buffer
            .trim()
            .trim_start_matches('#')
            .chars()
            .take(6)
            .collect::<String>();
        let preview = u32::from_str_radix(&preview_rgb, 16)
            .ok()
            .filter(|_| preview_rgb.len() == 6)
            .map(|rgb| (rgb & 0x00FF_FFFF) << 8 | 0xFF)
            .unwrap_or(palette_color);
        let mut hue_row = div().flex().flex_row().items_center().gap(px(3.0));
        for index in 0..12u32 {
            let hue_color = crate::ui::hue_rgb(index as f32 * 30.0);
            let label = format!("#{hue_color:06x}");
            hue_row = hue_row.child(
                div()
                    .id(format!("editor-hue-{index}"))
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded(px(7.0))
                    .bg(rgb(hue_color))
                    .border_1()
                    .border_color(self.chrome.border)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.apply_color((hue_color << 8) | 0xFF);
                        this.custom_color_buffer = format!("{hue_color:06x}");
                        this.custom_color_fresh = false;
                        this.status = None;
                        cx.notify();
                    })),
            );
        }

        let panel = div()
            .id("editor-custom-color-panel")
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
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
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded(px(4.0))
                            .bg(rgba(preview))
                            .border_1()
                            .border_color(self.chrome.border),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .child(t(language, "editor.custom_color")),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(self.chrome.selected)
                            .text_size(px(11.0))
                            .text_color(self.chrome.text)
                            .cursor_text()
                            .child({
                                let mut row =
                                    div().flex().flex_row().items_center().gap(px(1.0)).child(
                                        format!(
                                            "#{}",
                                            self.custom_color_buffer.to_ascii_uppercase()
                                        ),
                                    );
                                if !self.custom_color_fresh {
                                    row = row.child(
                                        div()
                                            .w(px(1.0))
                                            .h(px(11.0))
                                            .bg(self.chrome.accent)
                                            .with_animation(
                                                "editor-custom-color-caret",
                                                crate::ui::caret_blink(),
                                                |caret, delta| caret.opacity(delta),
                                            ),
                                    );
                                }
                                row
                            }),
                    ),
            )
            .child(hue_row)
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(self.chrome.muted)
                    .child(t(language, "editor.custom_color_hint")),
            );
        let panel = crate::ui::enter_slide_fade(
            panel,
            "editor-custom-color-enter",
            f32::from(rect.origin.y),
            4.0,
            Duration::from_millis(120),
        );

        Some(
            div()
                .id("editor-custom-color-layer")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_custom_color();
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    pub(super) fn zoom_at(
        &mut self,
        factor: f32,
        anchor: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.image_view_rect(window);
        let before_scale = f32::from(before.size.width) / self.image_size.width.max(1) as f32;
        if before_scale <= 0.0 {
            return;
        }
        let image_x = (f32::from(anchor.x) - f32::from(before.origin.x)) / before_scale;
        let image_y = (f32::from(anchor.y) - f32::from(before.origin.y)) / before_scale;
        let zoom = (self.view_zoom * factor).clamp(0.25, 32.0);
        if (zoom - self.view_zoom).abs() < f32::EPSILON {
            return;
        }
        self.view_zoom = zoom;
        let after = self.image_view_rect(window);
        let after_scale = f32::from(after.size.width) / self.image_size.width.max(1) as f32;
        let desired_x = f32::from(anchor.x) - image_x * after_scale;
        let desired_y = f32::from(anchor.y) - image_y * after_scale;
        self.view_pan.x += px(desired_x - f32::from(after.origin.x));
        self.view_pan.y += px(desired_y - f32::from(after.origin.y));
        cx.notify();
    }

    pub(super) fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.view_zoom = 1.0;
        self.view_pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    pub(super) fn refit_zoomable_view(&mut self) {
        let Some(mut inline) = self.inline else {
            return;
        };
        if !inline.zoomable {
            return;
        }
        let Some(area) = inline.fit_area else {
            return;
        };
        inline.image_rect = fit_frame_rect(self.image_size, area);
        let width = f32::from(inline.image_rect.size.width).max(1.0);
        self.ui_scale = (self.image_size.width as f32 / width).clamp(0.25, 32.0);
        self.inline = Some(inline);
    }

    pub(super) fn actual_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let fit = self.fit_scale(window);
        self.view_zoom = (1.0 / fit).clamp(0.25, 32.0);
        self.view_pan = point(px(0.0), px(0.0));
        cx.notify();
    }

    pub(super) fn push_canvas_snapshot(&mut self) {
        let snapshot = EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        };
        self.state.record_canvas_snapshot(snapshot);
    }

    pub(super) fn refresh_frame_images(&mut self, cx: &mut Context<Self>) {
        let image = image_util::render_image_ref(&self.frame);
        let old_image = std::mem::replace(&mut self.image, image);
        let old_blur_images =
            std::mem::replace(&mut self.blur_images, std::array::from_fn(|_| None));
        self.images_released = false;
        let mut dropped = Vec::with_capacity(1 + old_blur_images.len());
        dropped.push(old_image);
        dropped.extend(old_blur_images.into_iter().flatten());
        cx.defer(move |cx| {
            for image in dropped {
                cx.drop_image(image, None);
            }
        });
    }

    pub(super) fn blur_image(&mut self, radius: u32, _cx: &mut Context<Self>) -> Arc<RenderImage> {
        let index = blur_image_index(radius);
        if let Some(image) = &self.blur_images[index] {
            return image.clone();
        }
        let image = if self.full_canvas.is_some() {
            image_util::blurred_image_from_render(&self.image, radius)
        } else {
            image_util::blurred_image(&self.frame, radius)
        };
        self.blur_images[index] = Some(image.clone());
        image
    }

    pub(crate) fn release_images(&mut self, cx: &mut Context<Self>) {
        if self.images_released {
            return;
        }
        self.images_released = true;
        cx.drop_image(self.image.clone(), None);
        for image in self.blur_images.iter().flatten() {
            cx.drop_image(image.clone(), None);
        }
    }

    pub(crate) fn watch_close(&self, window: &Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = view.update(cx, |this, cx| this.release_images(cx));
            crate::windows::schedule_memory_trim();
            true
        });
    }

    pub(super) fn restore_canvas_snapshot(
        &mut self,
        snapshot: EditorSnapshot,
        cx: &mut Context<Self>,
    ) {
        self.frame = snapshot.frame;
        self.image_size = snapshot.image_size;
        self.document = snapshot.document;
        self.refresh_frame_images(cx);
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        self.pan_draft = None;
        self.crop_mode = false;
        self.crop_draft = None;
        self.refit_zoomable_view();
        self.reset_view(cx);
    }

    pub(super) fn editor_snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            frame: Arc::clone(&self.frame),
            image_size: self.image_size,
            document: self.document.clone(),
        }
    }

    pub(super) fn undo_editor(&mut self, cx: &mut Context<Self>) -> bool {
        let current = self.editor_snapshot();
        match self.state.take_undo(current) {
            Some(HistoryStep::Document) => true,
            Some(HistoryStep::Canvas(snapshot)) => {
                self.restore_canvas_snapshot(snapshot, cx);
                true
            }
            None => false,
        }
    }

    pub(super) fn redo_editor(&mut self, cx: &mut Context<Self>) -> bool {
        let current = self.editor_snapshot();
        match self.state.take_redo(current) {
            Some(HistoryStep::Document) => true,
            Some(HistoryStep::Canvas(snapshot)) => {
                self.restore_canvas_snapshot(snapshot, cx);
                true
            }
            None => false,
        }
    }

    pub(super) fn replace_canvas(&mut self, frame: RgbaFrame, cx: &mut Context<Self>) {
        self.frame = Arc::new(frame);
        self.image_size = PhysicalSize::new(self.frame.width(), self.frame.height());
        self.refresh_frame_images(cx);
        self.document = AnnotationDocument::new(self.image_size);
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        self.pan_draft = None;
        self.crop_mode = false;
        self.crop_draft = None;
        self.refit_zoomable_view();
        self.reset_view(cx);
    }
}
