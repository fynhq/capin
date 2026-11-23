use std::time::Instant;

use super::*;

use crate::logging::log_debug;

impl EditorView {
    pub(super) fn refresh_full_canvas(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let monitor = capture.region.monitor;
        let region_target = matches!(capture.target, CaptureTarget::Region(_));
        let mut backend = platform_capture::platform_backend();
        let captured = if settings_state::get().capture_cursor {
            backend.capture_with_cursor(CaptureTarget::Monitor(monitor))
        } else {
            backend.capture(CaptureTarget::Monitor(monitor))
        };
        let frame = match captured {
            Ok(frame) => frame,
            Err(error) => {
                self.show_status(
                    format!(
                        "{} {}",
                        t(settings_state::get().language, "editor.refresh_failed"),
                        error
                    ),
                    cx,
                );
                return;
            }
        };
        let new_size = PhysicalSize::new(frame.width(), frame.height());
        let size_changed = new_size != self.image_size;
        let image = image_util::render_image(frame);
        let old_image = std::mem::replace(&mut self.image, image);
        self.blur_cache.drop_images(cx);
        self.images_released = false;
        cx.defer(move |cx| {
            cx.drop_image(old_image, None);
        });
        if size_changed {
            self.image_size = new_size;
            self.document = AnnotationDocument::new(new_size);
            self.state.clear_history();
            self.text_edit = None;
            self.draft = None;
            self.move_draft = None;
            self.resize_draft = None;
            self.crop_mode = false;
            self.crop_draft = None;
            self.polyline_draft = None;
            self.region_drag = None;
            self.selection_hover = SelectionHover::None;
            self.document.select(None);
            if let Some(inline) = self.inline.as_mut()
                && let Some(capture) = inline.capture.as_mut()
            {
                if region_target {
                    let width = capture.region.rect.size.width.min(new_size.width);
                    let height = capture.region.rect.size.height.min(new_size.height);
                    let x = capture
                        .region
                        .rect
                        .origin
                        .x
                        .clamp(0, (new_size.width - width) as i32);
                    let y = capture
                        .region
                        .rect
                        .origin
                        .y
                        .clamp(0, (new_size.height - height) as i32);
                    capture.region = CaptureRegion::new(
                        capture.region.monitor,
                        PhysicalRect::new(x, y, width, height),
                    );
                } else {
                    capture.region = CaptureRegion::new(
                        capture.region.monitor,
                        PhysicalRect::new(0, 0, new_size.width, new_size.height),
                    );
                }
                capture.target = if region_target {
                    CaptureTarget::Region(capture.region)
                } else {
                    CaptureTarget::Monitor(monitor)
                };
            }
            if let Some(inline) = &self.inline {
                self.ui_scale =
                    fit_ui_scale(new_size.width, f32::from(inline.image_rect.size.width));
            }
        }
        if let Some(selection) = self.full_selection()
            && let Some(cropped) = image_util::cropped_rgba(&self.image, selection)
        {
            self.frame = Arc::new(cropped);
        }
        self.show_status(
            t(settings_state::get().language, "editor.refreshed").to_owned(),
            cx,
        );
    }

    pub(super) fn refresh_capture(&mut self, cx: &mut Context<Self>) {
        if self.full_canvas.is_some() {
            self.refresh_full_canvas(cx);
            return;
        }
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        let mut backend = platform_capture::platform_backend();
        let captured = if settings_state::get().capture_cursor {
            backend.capture_with_cursor(capture.target)
        } else {
            backend.capture(capture.target)
        };
        let frame = match captured {
            Ok(frame) => match capture.target {
                CaptureTarget::Window(id) => {
                    crate::window_frame::compose_window_capture(id, frame, capture.scale)
                }
                _ => frame,
            },
            Err(error) => {
                self.show_status(
                    format!(
                        "{} {}",
                        t(settings_state::get().language, "editor.refresh_failed"),
                        error
                    ),
                    cx,
                );
                return;
            }
        };
        if matches!(capture.target, CaptureTarget::Region(_))
            && let Ok(source) = platform_capture::platform_backend()
                .capture(CaptureTarget::Monitor(capture.region.monitor))
        {
            self.region_source = Some(image_util::render_image(source));
        }
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        let size_changed = image_size != self.image_size;
        self.frame = Arc::new(frame);
        self.image_size = image_size;
        self.refresh_frame_images(cx);
        if size_changed {
            self.document = AnnotationDocument::new(image_size);
            self.state.clear_history();
            self.text_edit = None;
            self.draft = None;
            self.move_draft = None;
            self.resize_draft = None;
            self.crop_mode = false;
            self.crop_draft = None;
            self.polyline_draft = None;
        }
        let image_size = self.image_size;
        if let Some(inline) = self.inline.as_mut() {
            let scale = capture.scale.max(0.1);
            let center_x = (capture.region.rect.origin.x as f32
                + capture.region.rect.size.width as f32 / 2.0)
                / scale;
            let center_y = (capture.region.rect.origin.y as f32
                + capture.region.rect.size.height as f32 / 2.0)
                / scale;
            let image_width = image_size.width as f32 / scale;
            let image_height = image_size.height as f32 / scale;
            inline.image_rect = Bounds {
                origin: point(
                    px(center_x - image_width / 2.0),
                    px(center_y - image_height / 2.0),
                ),
                size: size(px(image_width), px(image_height)),
            };
        }
        self.show_status(
            t(settings_state::get().language, "editor.refreshed").to_owned(),
            cx,
        );
    }

    pub(super) fn start_scroll_capture(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        if !crate::scroll::supports_target(capture.target) {
            let language = settings_state::get().language;
            self.show_status(t(language, "scroll.region_only").to_owned(), cx);
            return;
        }
        self.commit_text(cx);
        let document = self.action_document();
        let first_frame = match rasterize(&document, &self.frame) {
            Ok(frame) => frame,
            Err(error) => {
                self.show_status(
                    format!(
                        "{}{}",
                        t(settings_state::get().language, "editor.export_failed"),
                        error
                    ),
                    cx,
                );
                return;
            }
        };
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
            crate::scroll::open_session(capture.region, first_frame, cx);
        });
    }

    pub(super) fn start_recording(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.inline.and_then(|inline| inline.capture) else {
            return;
        };
        self.commit_text(cx);
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
            crate::record::start(capture.target, cx);
        });
    }

    pub(super) fn render_export(
        &self,
        with_pin_output: bool,
    ) -> Result<RenderedExport, core_annotate::RasterError> {
        let started = Instant::now();
        let document = self.action_document();
        let raw = rasterize(&document, &self.frame)?;
        let scale = self.region_effects_scale();
        let pin_effects = self.pin_effects();
        let pin_padding = pin_effects.padding(scale);
        let effects_active = self.region_effects_active();
        let (output, pin_output) = if with_pin_output {
            let copy = if effects_active {
                self.effects.apply(raw.clone(), scale)
            } else {
                raw.clone()
            };
            let pin = if pin_effects.active() {
                pin_effects.apply(raw, scale)
            } else {
                raw
            };
            (copy, Some(pin))
        } else {
            let copy = if effects_active {
                self.effects.apply(raw, scale)
            } else {
                raw
            };
            (copy, None)
        };
        let hash = crate::capture_history::hash_frame(&output);
        let record_history = self.pin_target.is_none();
        let pin_shadow = (record_history && self.region_pin() && !self.effects.shadow).then_some(
            crate::capture_history::HistoryPinShadow {
                corner_radius: self.effects.corner_radius,
                scale,
            },
        );
        log_debug!(
            "editor",
            "render_export with_pin={with_pin_output} duration_ms={} size={}x{}",
            started.elapsed().as_millis(),
            output.width(),
            output.height()
        );
        Ok(RenderedExport {
            document,
            output,
            pin_output,
            hash,
            record_history,
            pin_shadow,
            scale,
            pin_padding,
            stored_effects: self.stored_region_effects(),
            pin_shadow_always: self.region_pin(),
        })
    }

    pub(crate) fn pin(&mut self, cx: &mut Context<Self>) {
        self.commit_text(cx);
        let inline_host = self.inline;
        let selection_bounds = self.full_selection_screen_bounds();
        let native_shadow = self.pin_native_shadow();
        let rendered = match self.render_export(true) {
            Ok(rendered) => rendered,
            Err(error) => {
                let language = settings_state::get().language;
                self.show_status(
                    format!("{}{}", t(language, "editor.export_failed"), error),
                    cx,
                );
                cx.notify();
                return;
            }
        };
        let RenderedExport {
            document,
            output: export,
            pin_output,
            hash,
            record_history,
            pin_shadow,
            scale,
            pin_padding,
            stored_effects,
            pin_shadow_always,
        } = rendered;
        let Some(output) = pin_output else {
            return;
        };
        let export_padding = if self.effects.shadow { 0 } else { pin_padding };
        let placement = pin_placement(
            inline_host,
            selection_bounds,
            self.image_size,
            output.width(),
            output.height(),
        );
        let monitor = inline_host
            .and_then(|inline| inline.capture)
            .map(|capture| capture.region.monitor)
            .or_else(|| {
                inline_host
                    .and_then(|inline| inline.display)
                    .map(|display| MonitorId::from_raw(u64::from(display)))
            });
        if inline_host.is_some() {
            self.release_images(cx);
        }
        let handles = cx.windows();
        let edit_state = crate::pin::PinEditState::new(
            self.frame.clone(),
            document,
            stored_effects,
            pin_shadow_always,
            pin_padding,
            scale,
        );
        cx.defer(move |cx| {
            if inline_host.is_some() {
                crate::windows::close_capture_auxiliary(cx, handles);
            }
            if record_history && let Ok(bytes) = image_util::encode_png(&export) {
                crate::capture_history::record_png_bytes(
                    &bytes,
                    hash,
                    pin_shadow,
                    Some(scale),
                    native_shadow,
                );
            }
            let id = if let Some((bounds, zoom)) = placement {
                crate::pin::open_frame_at_with_hash(
                    output,
                    bounds,
                    zoom,
                    hash,
                    monitor,
                    native_shadow,
                    cx,
                )
            } else {
                crate::pin::open_frame_with_hash(output, hash, native_shadow, cx)
            };
            if let Some(id) = id {
                crate::pin::set_edit_state(id, edit_state);
                crate::pin::set_export_padding(id, export_padding);
            }
        });
        log_info!("editor", "pin requested");
        cx.notify();
    }

    pub(super) fn export(&mut self, action: ExportAction, cx: &mut Context<Self>) {
        let started = Instant::now();
        self.commit_text(cx);
        let language = settings_state::get().language;
        let native_shadow = self.pin_native_shadow();
        let copy = matches!(action, ExportAction::Copy);
        let pin_update = (!copy).then_some(self.pin_target).flatten();
        let mut messages = Vec::new();
        let mut copy_ok = false;
        let mut save_ok = false;
        let mut pin_ok = false;
        match self.render_export(pin_update.is_some()) {
            Ok(rendered) => {
                let RenderedExport {
                    document,
                    output,
                    pin_output,
                    hash,
                    record_history,
                    pin_shadow,
                    scale,
                    pin_padding,
                    stored_effects,
                    pin_shadow_always,
                } = rendered;
                if copy {
                    match image_util::encode_png(&output) {
                        Ok(bytes) => {
                            copy_ok = true;
                            if record_history {
                                crate::capture_history::record_png_bytes(
                                    &bytes,
                                    hash,
                                    pin_shadow,
                                    Some(scale),
                                    native_shadow,
                                );
                            }
                            image_util::copy_png_bytes(bytes, &output, cx);
                            messages.push(t(language, "editor.copied").to_owned());
                        }
                        Err(error) => {
                            messages.push(format!("{}{}", t(language, "editor.copy_failed"), error))
                        }
                    }
                } else if self.pin_target.is_none() {
                    match crate::storage::save_png(&output) {
                        Ok(path) => {
                            save_ok = true;
                            if record_history {
                                crate::capture_history::record_png_file(
                                    &path,
                                    hash,
                                    pin_shadow,
                                    Some(scale),
                                    native_shadow,
                                );
                            }
                            messages.push(format!(
                                "{} {}",
                                t(language, "editor.saved"),
                                path.display()
                            ));
                        }
                        Err(error) => messages.push(format!(
                            "{} {}",
                            t(language, "editor.save_failed"),
                            error
                        )),
                    }
                } else if let Some(pin_id) = self.pin_target {
                    let edit_state = crate::pin::PinEditState::new(
                        self.frame.clone(),
                        document,
                        stored_effects,
                        pin_shadow_always,
                        pin_padding,
                        scale,
                    );
                    let pin_image = pin_output.unwrap_or(output);
                    if crate::pin::replace_image_with_edit_state(pin_id, pin_image, edit_state, cx)
                    {
                        let export_padding = if self.effects.shadow { 0 } else { pin_padding };
                        crate::pin::set_export_padding(pin_id, export_padding);
                        pin_ok = true;
                        messages.push(t(language, "editor.pin_updated").to_owned());
                    } else {
                        messages.push(t(language, "editor.pin_missing").to_owned());
                    }
                }
            }
            Err(error) => {
                messages.push(format!("{}{}", t(language, "editor.export_failed"), error))
            }
        }
        self.show_status(messages.join(" · "), cx);
        let close = if copy {
            copy_ok
        } else if self.pin_target.is_some() {
            pin_ok
        } else {
            save_ok
        };
        log_info!(
            "editor",
            "export finished: action={action:?} duration_ms={} copy_ok={copy_ok} save_ok={save_ok} close={close}",
            started.elapsed().as_millis()
        );
        if close && self.inline.is_some() {
            self.release_images(cx);
            let handles = cx.windows();
            cx.defer(move |cx| crate::windows::close_capture_auxiliary(cx, handles));
        } else {
            cx.notify();
        }
    }
}
