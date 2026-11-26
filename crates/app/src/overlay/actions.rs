use super::*;

use crate::logging::log_debug;

impl OverlayView {
    pub(super) fn begin_selection(
        &mut self,
        outcome: SelectionOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::hints_window::close(cx);
        registry::set_focus(self.entity_id);
        let monitor = self.session.monitor_bounds();
        let (target, rect, scale, ratio) = match outcome {
            SelectionOutcome::Region(region) => (
                CaptureTarget::Region(region),
                region.rect,
                self.effective_scale(),
                self.aspect,
            ),
            SelectionOutcome::Window(id) => {
                let Some(window) = self.session.window(id) else {
                    return;
                };
                let Some(clipped) = window.bounds.intersection(monitor) else {
                    return;
                };
                (
                    CaptureTarget::Window(id),
                    PhysicalRect::new(
                        clipped.origin.x - monitor.origin.x,
                        clipped.origin.y - monitor.origin.y,
                        clipped.size.width,
                        clipped.size.height,
                    ),
                    self.effective_scale(),
                    AspectPreset::Free,
                )
            }
            SelectionOutcome::Monitor(id) => (
                CaptureTarget::Monitor(id),
                PhysicalRect::new(0, 0, monitor.size.width, monitor.size.height),
                self.effective_scale(),
                AspectPreset::Free,
            ),
        };
        self.screen_targeting = false;
        self.session.cancel();
        log_info!(
            "overlay",
            "selection started: {}x{}",
            rect.size.width,
            rect.size.height
        );
        self.selected = Some(Selected {
            target,
            rect,
            scale,
            ratio,
            unit: SizeUnit::Px,
            drag: None,
            edit: None,
            effects_open: false,
            hints_open: false,
            space_down: false,
        });
        self.aspect = ratio;
        self.clear_magnifier(window);
        cx.notify();
    }

    pub(super) fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        self.selected
            .as_ref()
            .and_then(|selected| self.local_rect(selected.rect))
    }

    fn selection_region(&self, selected: &Selected) -> CaptureRegion {
        CaptureRegion::new(self.session.monitor(), selected.rect)
    }

    fn capture_selected_raw(&self) -> Result<RgbaFrame, String> {
        let started = Instant::now();
        let selected = self
            .selected
            .as_ref()
            .ok_or_else(|| "no selection".to_owned())?;
        let target_kind = match selected.target {
            CaptureTarget::Region(_) => "region",
            CaptureTarget::Window(_) => "window",
            CaptureTarget::Monitor(_) => "monitor",
        };
        let frame = match selected.target {
            CaptureTarget::Window(id) => {
                let frozen =
                    !settings_state::get().capture_cursor && self.window_is_fully_visible(id);
                log_info!("overlay", "window {} frozen capture: {frozen}", id.raw());
                if frozen {
                    crate::window_frame::compose_frozen_window_capture(
                        id,
                        self.crop_selected_frame(selected)?,
                        selected.scale,
                    )
                } else {
                    let mut backend = platform_capture::platform_backend();
                    let captured = if settings_state::get().capture_cursor {
                        backend.capture_with_cursor(CaptureTarget::Window(id))
                    } else {
                        backend.capture(CaptureTarget::Window(id))
                    };
                    match captured {
                        Ok(frame) => {
                            crate::window_frame::compose_window_capture(id, frame, selected.scale)
                        }
                        Err(error) => {
                            log_warn!(
                                "overlay",
                                "window capture failed ({error}); cropping the frozen frame"
                            );
                            self.crop_selected_frame(selected)?
                        }
                    }
                }
            }
            _ if !settings_state::get().capture_cursor => self.crop_selected_frame(selected)?,
            _ => {
                let mut backend = platform_capture::platform_backend();
                backend
                    .capture_with_cursor(selected.target)
                    .map_err(|error| error.to_string())?
            }
        };
        log_debug!(
            "overlay",
            "capture target={target_kind} duration_ms={} size={}x{}",
            started.elapsed().as_millis(),
            frame.width(),
            frame.height()
        );
        Ok(frame)
    }

    fn crop_selected_frame(&self, selected: &Selected) -> Result<RgbaFrame, String> {
        image_util::cropped_rgba(&self.image, selected.rect)
            .ok_or_else(|| "empty selection".to_owned())
    }

    fn window_is_fully_visible(&self, id: WindowId) -> bool {
        self.session.window_is_fully_visible(id)
    }

    fn selected_output(&self) -> Result<RgbaFrame, String> {
        let selected = self
            .selected
            .as_ref()
            .ok_or_else(|| "no selection".to_owned())?;
        let frame = self.capture_selected_raw()?;
        if matches!(selected.target, CaptureTarget::Window(_)) {
            Ok(frame)
        } else {
            Ok(self.effects.apply(frame, self.effective_scale()))
        }
    }

    pub(super) fn open_editor_for_selected(
        &mut self,
        tool: Tool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        platform_window::set_cursor_hidden(false);
        self.window_only = false;
        self.commit_edit(cx);
        if let Some(selected) = self.selected.as_mut() {
            selected.hints_open = false;
        }
        super::hints_window::close(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let region = self.selection_region(&selected);
        let scale = self.effective_scale();
        let full_monitor = matches!(
            selected.target,
            CaptureTarget::Region(_) | CaptureTarget::Monitor(_)
        );
        let mut full_canvas = None;
        let frame = if full_monitor {
            let Some(cache) = image_util::cropped_rgba(&self.image, selected.rect) else {
                self.set_notice("empty selection".to_owned(), cx);
                return;
            };
            full_canvas = Some(FullCanvasInit {
                image: self.image.clone(),
                image_size: self.image_size,
                adjustable: matches!(selected.target, CaptureTarget::Region(_)),
            });
            Arc::new(cache)
        } else {
            match self.capture_selected_raw() {
                Ok(frame) => Arc::new(frame),
                Err(error) => {
                    self.set_notice(error, cx);
                    return;
                }
            }
        };
        log_info!(
            "overlay",
            "opening inline editor for {}x{} region",
            frame.width(),
            frame.height()
        );
        let editor_rect = if full_monitor {
            Bounds {
                origin: point(px(0.0), px(0.0)),
                size: self.window_size,
            }
        } else {
            let center_x =
                (region.rect.origin.x as f32 + region.rect.size.width as f32 / 2.0) / scale;
            let center_y =
                (region.rect.origin.y as f32 + region.rect.size.height as f32 / 2.0) / scale;
            let image_width = frame.width() as f32 / scale;
            let image_height = frame.height() as f32 / scale;
            Bounds {
                origin: point(
                    px(center_x - image_width / 2.0),
                    px(center_y - image_height / 2.0),
                ),
                size: size(px(image_width), px(image_height)),
            }
        };
        let monitor_origin = self.session.monitor_bounds().origin;
        let screen_origin = point(
            px(monitor_origin.x as f32 / scale),
            px(monitor_origin.y as f32 / scale),
        );
        let inline_capture = InlineCapture {
            region,
            target: selected.target,
            scale,
            monitor_origin,
            effects: self.effects,
        };
        self.clear_magnifier(window);
        self.editing_rect = Some(editor_rect);
        self.selected = None;
        self.screen_targeting = false;
        self.session.cancel();
        let region_source = (!full_monitor && matches!(selected.target, CaptureTarget::Region(_)))
            .then(|| self.image.clone());

        let handles = cx.windows();
        let current = window.window_handle();
        let view = cx.entity();
        let keep = view.entity_id().as_u64();
        cx.defer(move |cx| {
            crate::overlay::release_textures_except(cx, keep);
            crate::windows::close_capture_auxiliary_except(cx, handles, current);
            let _ = current.update(cx, |_, window, cx| {
                let editor = view.update(cx, |this, cx| {
                    let editor = cx.new(|cx| {
                        EditorView::new_inline(
                            frame,
                            region_source,
                            editor_rect,
                            screen_origin,
                            tool,
                            Some(inline_capture),
                            None,
                            false,
                            None,
                            inline_capture.effects,
                            full_canvas,
                            cx,
                        )
                    });
                    editor.update(cx, |editor, _| {
                        editor
                            .set_display(Some(DisplayId::new(inline_capture.region.monitor.raw())));
                    });
                    if full_monitor {
                        this.image = image_util::transparent_image();
                    }
                    this.editor = Some(editor.clone());
                    this.editing_rect = Some(editor_rect);
                    cx.notify();
                    editor
                });
                let focus = editor.read(cx).focus_handle.clone();
                window.focus(&focus, cx);
                crate::editor::register_view(&editor, window);
                editor.update(cx, |this, cx| {
                    this.observe_appearance(window, cx);
                    this.watch_close(window, cx);
                });
            });
        });
    }

    pub(super) fn primary_pointer_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| selected.drag.is_some())
        {
            self.finish_region_drag(cx);
            return;
        }
        if self.selected.is_some() {
            return;
        }
        let point = self.to_physical(event.position);
        self.cursor = Some(point);
        let action = self.session.pointer_released(point);
        self.screen_targeting = false;
        self.session.set_screen_targeting(false);
        if let SelectionAction::Capture(outcome) = action {
            let click_opens_editor = self.click_opens_editor;
            self.click_opens_editor = false;
            self.begin_selection(outcome, window, cx);
            platform_window::set_cursor_hidden(false);
            if click_opens_editor {
                self.window_only = false;
                self.open_editor_for_selected(Tool::Select, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn copy_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let output = match self.selected_output() {
            Ok(output) => output,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let hash = crate::capture_history::hash_frame(&output);
        let pin_shadow = history_pin_shadow(self);
        match image_util::encode_png(&output) {
            Ok(bytes) => {
                log_info!("overlay", "selection copied to clipboard");
                crate::capture_history::record_png_bytes(
                    &bytes,
                    hash,
                    pin_shadow,
                    Some(self.effective_scale()),
                    false,
                );
                image_util::copy_png_bytes(bytes, &output, cx);
                self.close_overlays(cx);
            }
            Err(error) => self.set_notice(error.to_string(), cx),
        }
    }

    pub(super) fn save_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let output = match self.selected_output() {
            Ok(output) => output,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let hash = crate::capture_history::hash_frame(&output);
        let pin_shadow = history_pin_shadow(self);
        match crate::storage::save_png(&output) {
            Ok(path) => {
                log_info!("overlay", "selection saved");
                crate::capture_history::record_png_file(
                    &path,
                    hash,
                    pin_shadow,
                    Some(self.effective_scale()),
                    false,
                );
                self.close_overlays(cx);
            }
            Err(error) => self.set_notice(error.to_string(), cx),
        }
    }

    pub(super) fn pin_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let output = match self.capture_selected_raw() {
            Ok(frame) => frame,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let monitor = self.session.monitor_bounds();
        let scale = self.effective_scale();
        let region = matches!(selected.target, CaptureTarget::Region(_));
        let window = matches!(selected.target, CaptureTarget::Window(_));
        let raw = output;
        let display_effects = if region {
            self.effects.with_shadow()
        } else {
            self.effects
        };
        let export = if window {
            raw.clone()
        } else {
            self.effects.apply(raw.clone(), scale)
        };
        let display = if window {
            raw.clone()
        } else {
            display_effects.apply(raw.clone(), scale)
        };
        let center_x = (monitor.origin.x + selected.rect.origin.x) as f32 / scale
            + selected.rect.size.width as f32 / scale / 2.0;
        let center_y = (monitor.origin.y + selected.rect.origin.y) as f32 / scale
            + selected.rect.size.height as f32 / scale / 2.0;
        let width = display.width() as f32 / scale;
        let height = display.height() as f32 / scale;
        let bounds = Bounds {
            origin: point(px(center_x - width / 2.0), px(center_y - height / 2.0)),
            size: size(px(width), px(height)),
        };
        log_info!("overlay", "selection pinned");
        let hash = crate::capture_history::hash_frame(&export);
        let pin_shadow = (region && !self.effects.shadow).then_some(HistoryPinShadow {
            corner_radius: self.effects.corner_radius,
            scale,
        });
        let export_padding = if pin_shadow.is_some() {
            display_effects.padding(scale)
        } else {
            0
        };
        let monitor = self.session.monitor();
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
            if let Ok(bytes) = image_util::encode_png(&export) {
                crate::capture_history::record_png_bytes(
                    &bytes,
                    hash,
                    pin_shadow,
                    Some(scale),
                    false,
                );
            }
            if let Some(id) = crate::pin::open_frame_at_with_hash(
                display,
                bounds,
                1.0 / scale,
                hash,
                Some(monitor),
                false,
                cx,
            ) {
                crate::pin::set_export_padding(id, export_padding);
            }
        });
    }

    pub(super) fn scroll_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
        if !crate::scroll::supports_target(selected.target) {
            let language = settings_state::get().language;
            self.set_notice(t(language, "scroll.region_only").to_owned(), cx);
            return;
        }
        let frame = match self.capture_selected_raw() {
            Ok(frame) => frame,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let region = self.selection_region(&selected);
        self.close_overlays(cx);
        cx.defer(move |cx| crate::scroll::open_session(region, frame, cx));
    }

    pub(super) fn record_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
        self.close_overlays(cx);
        cx.defer(move |cx| crate::record::start(selected.target, cx));
    }

    pub(super) fn set_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = Some(message);
        self.notice_leaving = false;
        self.notice_serial = self.notice_serial.wrapping_add(1);
        let serial = self.notice_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1600))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.notice_serial != serial {
                        return false;
                    }
                    this.notice_leaving = true;
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
                if this.notice_serial == serial {
                    this.notice = None;
                    this.notice_leaving = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn primary_pointer_down(
        &mut self,
        event: &MouseDownEvent,
        ctrl_click: bool,
        cx: &mut Context<Self>,
    ) {
        let point = self.to_physical(event.position);
        self.cursor = Some(point);
        self.set_window_only(control_down(&event.modifiers) || ctrl_click, cx);
        self.click_opens_editor = self.window_only && self.selected.is_none();
        if let Some(selected) = self.selected.clone() {
            if selected.edit.is_some() {
                self.commit_edit(cx);
            }
            let monitor = self.session.monitor_bounds();
            let local = PhysicalPoint::new(point.x - monitor.origin.x, point.y - monitor.origin.y);
            let edge_tolerance = 8.0 * self.effective_scale();
            if let Some(handle) = edge_resize_handle(selected.rect, local, edge_tolerance) {
                if let Some(selected) = self.selected.as_mut() {
                    selected.drag = Some(RegionDrag {
                        kind: RegionDragKind::Resize(handle),
                        start: point,
                        original: selected.rect,
                    });
                    selected.edit = None;
                    selected.effects_open = false;
                    selected.hints_open = false;
                }
                super::hints_window::close(cx);
                self.cursor_style = region_handle_cursor(handle);
                cx.notify();
                return;
            }
            if local.x >= selected.rect.origin.x
                && local.y >= selected.rect.origin.y
                && local.x <= selected.rect.origin.x + selected.rect.size.width as i32
                && local.y <= selected.rect.origin.y + selected.rect.size.height as i32
            {
                if event.click_count >= 2 {
                    match settings_state::get().capture_double_click {
                        DoubleClickAction::NoAction => {}
                        DoubleClickAction::Copy => self.copy_selected(cx),
                        DoubleClickAction::Pin => self.pin_selected(cx),
                    }
                    return;
                }
                if let Some(selected) = self.selected.as_mut() {
                    selected.drag = Some(RegionDrag {
                        kind: RegionDragKind::Move,
                        start: point,
                        original: selected.rect,
                    });
                    selected.effects_open = false;
                    selected.hints_open = false;
                }
                super::hints_window::close(cx);
                self.cursor_style = crate::ui::closed_hand_cursor();
                cx.notify();
                return;
            }
            self.selected = None;
            self.cursor_style = CursorStyle::Crosshair;
        }
        self.session.pointer_moved(point);
        self.refresh_auto_target(point, cx);
        self.session.pointer_pressed(point);
        cx.notify();
    }
}
