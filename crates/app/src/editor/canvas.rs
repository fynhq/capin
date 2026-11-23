use super::*;

pub(super) fn push_preview_rounded_rect(
    builder: &mut PathBuilder,
    to_point: &impl Fn(f32, f32) -> Point<Pixels>,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) {
    if radius < 0.5 {
        builder.move_to(to_point(left, top));
        builder.line_to(to_point(right, top));
        builder.line_to(to_point(right, bottom));
        builder.line_to(to_point(left, bottom));
        return;
    }
    let handle = radius * core_types::ARC_HANDLE;
    builder.move_to(to_point(left + radius, top));
    builder.line_to(to_point(right - radius, top));
    builder.cubic_bezier_to(
        to_point(right, top + radius),
        to_point(right - radius + handle, top),
        to_point(right, top + radius - handle),
    );
    builder.line_to(to_point(right, bottom - radius));
    builder.cubic_bezier_to(
        to_point(right - radius, bottom),
        to_point(right, bottom - radius + handle),
        to_point(right - radius + handle, bottom),
    );
    builder.line_to(to_point(left + radius, bottom));
    builder.cubic_bezier_to(
        to_point(left, bottom - radius),
        to_point(left + radius - handle, bottom),
        to_point(left, bottom - radius + handle),
    );
    builder.line_to(to_point(left, top + radius));
    builder.cubic_bezier_to(
        to_point(left + radius, top),
        to_point(left, top + radius - handle),
        to_point(left + radius - handle, top),
    );
}

impl EditorView {
    pub(super) fn region_capture(&self) -> Option<(CaptureRegion, PhysicalRect)> {
        let inline = self.inline.as_ref()?;
        if inline.zoomable {
            return None;
        }
        let capture = inline.capture.as_ref()?;
        let CaptureTarget::Region(region) = capture.target else {
            return None;
        };
        Some((region, region.rect))
    }

    pub(super) fn can_adjust_region(&self) -> bool {
        let has_source = self.region_source.is_some()
            || self
                .full_canvas
                .is_some_and(|full_canvas| full_canvas.adjustable);
        self.tool == Tool::Select
            && has_source
            && self.region_capture().is_some()
            && self.document.selected().is_none()
            && self.draft.is_none()
            && self.move_draft.is_none()
            && self.resize_draft.is_none()
            && self.polyline_draft.is_none()
            && self.text_edit.is_none()
            && !self.crop_mode
            && self.region_adjust.draft.is_none()
    }

    pub(super) fn begin_region_drag(&mut self, handle: RegionHandle, cx: &mut Context<Self>) {
        let Some((_, rect)) = self.region_capture() else {
            return;
        };
        self.region_adjust.begin(handle, rect);
        self.status = None;
        cx.notify();
    }

    pub(super) fn update_region_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let Some(draft) = self.region_adjust.draft.as_ref().copied() else {
            return;
        };
        let image_size = if self.full_canvas.is_some() {
            self.image_size
        } else {
            let Some(source) = self.region_source.clone() else {
                return;
            };
            image_util::image_size(&source)
        };
        let scale = capture.scale.max(0.1);
        let cursor_point = PhysicalPoint::new(
            (f32::from(position.x) * scale).round() as i32,
            (f32::from(position.y) * scale).round() as i32,
        );
        let rect = resize_rect(draft.original, draft.handle, cursor_point, None, image_size);
        self.region_adjust.update(rect);
        cx.notify();
    }

    pub(super) fn finish_region_drag(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.region_adjust.take() else {
            return;
        };
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        if self.full_canvas.is_some() {
            if draft.current == draft.original {
                return;
            }
            let Some(cropped) = image_util::cropped_rgba(&self.image, draft.current) else {
                return;
            };
            self.push_canvas_snapshot();
            self.frame = Arc::new(cropped);
            if let Some(inline) = self.inline.as_mut()
                && let Some(capture) = inline.capture.as_mut()
            {
                capture.region = CaptureRegion::new(capture.region.monitor, draft.current);
                capture.target = CaptureTarget::Region(capture.region);
            }
            if let Some(id) = self.document.selected()
                && self
                    .document
                    .object(id)
                    .is_none_or(|object| !object.shape.bounds().intersects(draft.current))
            {
                self.document.select(None);
                self.selection_hover = SelectionHover::None;
            }
            self.status = None;
            cx.notify();
            return;
        }
        let Some(source) = self.region_source.clone() else {
            return;
        };
        let scale = capture.scale.max(0.1);
        if draft.current == draft.original {
            self.restore_region_image_rect(scale, cx);
            return;
        }
        let Some(cropped) = image_util::cropped_rgba(&source, draft.current) else {
            self.restore_region_image_rect(scale, cx);
            return;
        };
        self.push_canvas_snapshot();
        let dx = draft.current.origin.x - draft.original.origin.x;
        let dy = draft.current.origin.y - draft.original.origin.y;
        let new_size = PhysicalSize::new(cropped.width(), cropped.height());
        let document = self.document.crop_to(PhysicalPoint::new(dx, dy), new_size);
        self.frame = Arc::new(cropped);
        self.image_size = new_size;
        self.refresh_frame_images(cx);
        self.document = document;
        self.text_edit = None;
        self.draft = None;
        self.move_draft = None;
        self.resize_draft = None;
        self.polyline_draft = None;
        self.selection_hover = SelectionHover::None;
        self.view_zoom = 1.0;
        self.view_pan = point(px(0.0), px(0.0));
        if let Some(inline) = self.inline.as_mut() {
            inline.image_rect = Bounds {
                origin: point(
                    px(draft.current.origin.x as f32 / scale),
                    px(draft.current.origin.y as f32 / scale),
                ),
                size: size(
                    px(draft.current.size.width as f32 / scale),
                    px(draft.current.size.height as f32 / scale),
                ),
            };
            if let Some(capture) = inline.capture.as_mut() {
                capture.region = CaptureRegion::new(capture.region.monitor, draft.current);
                capture.target = CaptureTarget::Region(capture.region);
            }
        }
        self.show_status(
            t(settings_state::get().language, "editor.cropped").to_owned(),
            cx,
        );
    }

    pub(super) fn restore_region_image_rect(&mut self, scale: f32, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            return;
        }
        let Some((_, rect)) = self.region_capture() else {
            return;
        };
        if let Some(inline) = self.inline.as_mut() {
            inline.image_rect = Bounds {
                origin: point(
                    px(rect.origin.x as f32 / scale),
                    px(rect.origin.y as f32 / scale),
                ),
                size: size(
                    px(rect.size.width as f32 / scale),
                    px(rect.size.height as f32 / scale),
                ),
            };
        }
        cx.notify();
    }

    pub(super) fn cancel_annotation_state(&mut self, cx: &mut Context<Self>) -> bool {
        if self.text_edit.is_some() {
            self.cancel_text(cx);
            return true;
        }
        if self.polyline_draft.is_some() {
            self.cancel_polyline(cx);
            return true;
        }
        if self.toolbar.close_panels() {
            cx.notify();
            return true;
        }
        if self.document.selected().is_some() || self.tool != Tool::Select {
            self.select_annotation(None);
            self.tool = Tool::Select;
            self.draft = None;
            self.cancel_move();
            self.cancel_resize();
            self.pan_draft = None;
            self.selection_hover = SelectionHover::None;
            self.status = None;
            cx.notify();
            return true;
        }
        false
    }

    pub(super) fn right_click(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.text_edit.is_some() {
            self.commit_text(cx);
            return;
        }
        if self.polyline_draft.is_some() {
            self.finish_polyline(cx);
            return;
        }
        if self.crop_mode {
            if self.crop_draft.is_some() {
                self.apply_crop(cx);
            } else {
                self.cancel_crop(cx);
            }
            return;
        }
        let _ = self.cancel_annotation_state(cx);
    }

    pub(super) fn region_handles(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.can_adjust_region() {
            return Vec::new();
        }
        let rect = self
            .inline_selection_rect(window)
            .unwrap_or_else(|| self.image_view_rect(window));
        let radius = if export_pipeline::region_effects_active(self) {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let mut handles = Vec::new();
        for (handle, position) in handle_positions(rect) {
            let (dx, dy) = corner_handle_offset(handle, radius);
            handles.push(
                div()
                    .id(format!("editor-region-handle-{:?}", handle))
                    .absolute()
                    .left(px(f32::from(position.x) + dx - HANDLE_HIT / 2.0))
                    .top(px(f32::from(position.y) + dy - HANDLE_HIT / 2.0))
                    .w(px(HANDLE_HIT))
                    .h(px(HANDLE_HIT))
                    .cursor(region_handle_cursor(handle))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            let focus = this.focus_handle.clone();
                            window.focus(&focus, cx);
                            this.begin_region_drag(handle, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px((HANDLE_HIT - REGION_HANDLE_SIZE) / 2.0))
                            .top(px((HANDLE_HIT - REGION_HANDLE_SIZE) / 2.0))
                            .w(px(REGION_HANDLE_SIZE))
                            .h(px(REGION_HANDLE_SIZE))
                            .rounded(px(1.5))
                            .bg(rgb(0xFFFFFF))
                            .border_1()
                            .border_color(rgba((self.accent_hex << 8) | 0xFF)),
                    )
                    .into_any_element(),
            );
        }
        handles
    }

    pub(super) fn region_edge_bands(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.can_adjust_region() {
            return Vec::new();
        }
        let Some(rect) = self.inline_selection_rect(window) else {
            return Vec::new();
        };
        let left = f32::from(rect.origin.x);
        let top = f32::from(rect.origin.y);
        let width = f32::from(rect.size.width);
        let height = f32::from(rect.size.height);
        let half = BAND_HALF;
        let mut bands = Vec::new();
        {
            let mut band = |handle: RegionHandle, x: f32, y: f32, w: f32, h: f32| {
                bands.push(
                    div()
                        .id(format!("editor-region-band-{:?}", handle))
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w.max(1.0)))
                        .h(px(h.max(1.0)))
                        .cursor(region_handle_cursor(handle))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                let focus = this.focus_handle.clone();
                                window.focus(&focus, cx);
                                this.begin_region_drag(handle, cx);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                );
            };
            band(RegionHandle::North, left, top - half, width, half * 2.0);
            band(
                RegionHandle::South,
                left,
                top + height - half,
                width,
                half * 2.0,
            );
            band(RegionHandle::West, left - half, top, half * 2.0, height);
            band(
                RegionHandle::East,
                left + width - half,
                top,
                half * 2.0,
                height,
            );
            let corner = half * 2.0;
            band(
                RegionHandle::NorthWest,
                left - half,
                top - half,
                corner,
                corner,
            );
            band(
                RegionHandle::NorthEast,
                left + width - half,
                top - half,
                corner,
                corner,
            );
            band(
                RegionHandle::SouthWest,
                left - half,
                top + height - half,
                corner,
                corner,
            );
            band(
                RegionHandle::SouthEast,
                left + width - half,
                top + height - half,
                corner,
                corner,
            );
        }
        bands
    }

    pub(super) fn selection_layer(&self, window: &Window) -> Option<AnyElement> {
        if self.text_edit.is_some() {
            return None;
        }
        let shape = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .map(|annotation| annotation.shape.clone())?;
        let image_rect = self.image_view_rect(window);
        let scale = f32::from(image_rect.size.width) / self.image_size.width.max(1) as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let accent = rgba((self.accent_hex << 8) | 0xFF);
        let rect = shape.bounds();
        let left = f32::from(image_rect.origin.x) + rect.origin.x as f32 * scale;
        let top = f32::from(image_rect.origin.y) + rect.origin.y as f32 * scale;
        let mut layer = div().absolute().inset_0();
        layer = layer.child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px((rect.size.width as f32 * scale).max(0.0)))
                .h(px((rect.size.height as f32 * scale).max(0.0)))
                .border_1()
                .border_color(accent),
        );
        for (_, handle) in handle_points(&shape) {
            let centre_x = f32::from(image_rect.origin.x) + handle.x as f32 * scale;
            let centre_y = f32::from(image_rect.origin.y) + handle.y as f32 * scale;
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(centre_x - SELECTION_HANDLE_HALF))
                    .top(px(centre_y - SELECTION_HANDLE_HALF))
                    .w(px(SELECTION_HANDLE_HALF * 2.0))
                    .h(px(SELECTION_HANDLE_HALF * 2.0))
                    .bg(accent)
                    .child(
                        div()
                            .absolute()
                            .left(px(SELECTION_HANDLE_HALF - 3.0))
                            .top(px(SELECTION_HANDLE_HALF - 3.0))
                            .w(px(6.0))
                            .h(px(6.0))
                            .bg(rgb(0xFFFFFF)),
                    ),
            );
        }
        Some(layer.into_any_element())
    }

    pub(super) fn rotate_canvas(&mut self, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            return;
        }
        self.commit_text(cx);
        let language = settings_state::get().language;
        let Ok(output) = rasterize(&self.document, &self.frame) else {
            self.show_status(t(language, "editor.export_failed").to_owned(), cx);
            return;
        };
        self.push_canvas_snapshot();
        let rotated = output.rotated_cw();
        self.replace_canvas(rotated, cx);
        self.show_status(t(language, "editor.rotated").to_owned(), cx);
    }

    pub(super) fn flip_canvas(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            return;
        }
        self.commit_text(cx);
        let language = settings_state::get().language;
        let Ok(output) = rasterize(&self.document, &self.frame) else {
            self.show_status(t(language, "editor.export_failed").to_owned(), cx);
            return;
        };
        self.push_canvas_snapshot();
        let flipped = output.flipped(horizontal);
        self.replace_canvas(flipped, cx);
        self.show_status(
            t(
                language,
                if horizontal {
                    "editor.flipped_horizontal"
                } else {
                    "editor.flipped_vertical"
                },
            )
            .to_owned(),
            cx,
        );
    }

    pub(super) fn toggle_crop(&mut self, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            return;
        }
        self.commit_text(cx);
        self.crop_mode = !self.crop_mode;
        self.crop_draft = None;
        self.text_edit = None;
        self.draft = None;
        self.cancel_move();
        self.cancel_resize();
        if self.crop_mode {
            self.document.select(None);
            self.selection_hover = SelectionHover::None;
        }
        self.status = self
            .crop_mode
            .then(|| t(settings_state::get().language, "editor.crop_hint").to_owned());
        cx.notify();
    }

    pub(super) fn apply_crop(&mut self, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            return;
        }
        let Some(draft) = self.crop_draft.take() else {
            return;
        };
        let rect = normalized_rect(draft.start, draft.current);
        if rect.size.width < 2 || rect.size.height < 2 {
            return;
        }
        let left = rect.origin.x.clamp(0, self.image_size.width as i32);
        let top = rect.origin.y.clamp(0, self.image_size.height as i32);
        let right =
            (rect.origin.x + rect.size.width as i32).clamp(left, self.image_size.width as i32);
        let bottom =
            (rect.origin.y + rect.size.height as i32).clamp(top, self.image_size.height as i32);
        let rect = PhysicalRect::new(left, top, (right - left) as u32, (bottom - top) as u32);
        if rect.size.width < 2 || rect.size.height < 2 {
            return;
        }
        let Some(cropped) = self.frame.as_ref().cropped(rect) else {
            return;
        };
        self.push_canvas_snapshot();
        let new_size = PhysicalSize::new(cropped.width(), cropped.height());
        let document = self.document.crop_to(rect.origin, new_size);
        self.frame = Arc::new(cropped);
        self.image_size = new_size;
        self.refresh_frame_images(cx);
        self.document = document;
        self.crop_mode = false;
        self.crop_draft = None;
        self.text_edit = None;
        self.refit_zoomable_view();
        self.reset_view(cx);
        self.show_status(
            t(settings_state::get().language, "editor.cropped").to_owned(),
            cx,
        );
    }

    pub(super) fn cancel_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = false;
        self.crop_draft = None;
        self.status = None;
        cx.notify();
    }

    pub(super) fn to_display_point(
        &self,
        position: Point<Pixels>,
        window: &Window,
    ) -> Option<PhysicalPoint> {
        let view = self.image_view_rect(window);
        let scale = f32::from(view.size.width) / self.image_size.width as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let x = ((f32::from(position.x) - f32::from(view.origin.x)) / scale).floor() as i32;
        let y = ((f32::from(position.y) - f32::from(view.origin.y)) / scale).floor() as i32;
        if x < 0 || y < 0 || x >= self.image_size.width as i32 || y >= self.image_size.height as i32
        {
            return None;
        }
        Some(PhysicalPoint::new(x, y))
    }

    pub(super) fn zoomable_pan_start(&self, position: Point<Pixels>, window: &Window) -> bool {
        let Some(inline) = self.inline else {
            return false;
        };
        if !inline.zoomable || self.tool != Tool::Select || self.text_edit.is_some() {
            return false;
        }
        let Some(point) = self.to_display_point(position, window) else {
            return false;
        };
        if self.document.hit_test(point).is_some() {
            return false;
        }
        let scale = self.image_scale(window);
        if let Some(annotation) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            && annotation_handle_at(&annotation.shape, point, scale).is_some()
        {
            return false;
        }
        true
    }

    pub(super) fn to_image_point(
        &self,
        position: Point<Pixels>,
        window: &Window,
    ) -> Option<PhysicalPoint> {
        let point = self.to_display_point(position, window)?;
        if self
            .full_selection()
            .is_some_and(|selection| !selection.contains(point))
        {
            return None;
        }
        Some(point)
    }

    pub(super) fn region_handle_at(&self, point: PhysicalPoint) -> Option<RegionHandle> {
        if !self.can_adjust_region() {
            return None;
        }
        let rect = self.full_selection().unwrap_or_else(|| {
            PhysicalRect::new(0, 0, self.image_size.width, self.image_size.height)
        });
        let scale = self
            .inline
            .map(|inline| {
                f32::from(inline.image_rect.size.width) / self.image_size.width.max(1) as f32
            })
            .unwrap_or(1.0)
            .max(0.01);
        edge_resize_handle(rect, point, (8.0 / scale).max(1.0))
    }

    pub(super) fn apply_custom_color(&mut self, cx: &mut Context<Self>) -> bool {
        let hex = self
            .toolbar
            .custom_color_buffer
            .trim()
            .trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        let Ok(rgb) = u32::from_str_radix(hex, 16) else {
            return false;
        };
        self.apply_color((rgb << 8) | 0xFF);
        self.close_custom_color();
        self.status = None;
        cx.notify();
        true
    }

    pub(super) fn finish_polyline(&mut self, cx: &mut Context<Self>) {
        if self.state.finish_polyline() {
            cx.notify();
        }
    }

    pub(super) fn cancel_polyline(&mut self, cx: &mut Context<Self>) {
        self.state.cancel_polyline();
        cx.notify();
    }

    pub(super) fn finish_resize(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) {
        if self.state.finish_resize(point) {
            cx.notify();
        }
    }

    pub(super) fn copy_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(annotation) = self
            .document
            .selected()
            .and_then(|id| self.document.object(id))
            .cloned()
        else {
            return false;
        };
        self.annotation_clipboard = Some(annotation);
        self.show_status(
            t(settings_state::get().language, "editor.annotation_copied").to_owned(),
            cx,
        );
        true
    }

    pub(super) fn paste_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(annotation) = self.annotation_clipboard.clone() else {
            return false;
        };
        let id = self.state.paste_annotation(&annotation, 16, 16);
        self.tool = Tool::Select;
        self.select_annotation(Some(id));
        self.show_status(
            t(settings_state::get().language, "editor.annotation_pasted").to_owned(),
            cx,
        );
        true
    }

    pub(super) fn delete_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.document.selected() else {
            log_info!("editor", "delete requested without a selected annotation");
            return false;
        };
        if !self.state.remove_annotation(id) {
            log_info!("editor", "delete failed for annotation {id:?}");
            return false;
        }
        log_info!("editor", "deleted annotation {id:?}");
        self.show_status(
            t(settings_state::get().language, "editor.annotation_deleted").to_owned(),
            cx,
        );
        true
    }
}
