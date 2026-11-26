use super::*;

impl OverlayView {
    pub(super) fn info_rect(&self) -> Option<Bounds<Pixels>> {
        let selection = self.selection_bounds()?;
        let window_width = f32::from(self.window_size.width);
        let window_height = f32::from(self.window_size.height);
        let width = INFO_BAR_WIDTH.min(window_width - 16.0);
        let left =
            (f32::from(selection.origin.x)).clamp(8.0, (window_width - width - 8.0).max(8.0));
        let toolbar = self.toolbar_rect();
        let selection_top = f32::from(selection.origin.y);
        let selection_bottom = selection_top + f32::from(selection.size.height);
        let toolbar_top = toolbar.map(|toolbar| f32::from(toolbar.origin.y));
        let toolbar_bottom =
            toolbar.map(|toolbar| f32::from(toolbar.origin.y) + f32::from(toolbar.size.height));
        let fits = |top: f32| top >= 8.0 && top + INFO_BAR_HEIGHT <= window_height - 8.0;
        let overlaps_toolbar = |top: f32| {
            toolbar_top.is_some_and(|toolbar_top| {
                top < toolbar_bottom.unwrap_or(0.0) && top + INFO_BAR_HEIGHT > toolbar_top
            })
        };
        let above = selection_top - INFO_BAR_HEIGHT - REGION_GAP;
        let below = selection_bottom + REGION_GAP;
        let inside_top = selection_top + REGION_GAP;
        let inside_bottom = selection_bottom - INFO_BAR_HEIGHT - REGION_GAP;
        let mut candidates = Vec::new();
        if fits(above) && !overlaps_toolbar(above) {
            candidates.push(above);
        }
        if let (Some(toolbar_top), Some(toolbar_bottom)) = (toolbar_top, toolbar_bottom) {
            if toolbar_top >= selection_bottom && fits(toolbar_bottom + REGION_GAP) {
                candidates.push(toolbar_bottom + REGION_GAP);
            }
            if toolbar_bottom <= selection_top && fits(toolbar_top - INFO_BAR_HEIGHT - REGION_GAP) {
                candidates.push(toolbar_top - INFO_BAR_HEIGHT - REGION_GAP);
            }
        }
        if fits(below) && !overlaps_toolbar(below) {
            candidates.push(below);
        }
        if fits(inside_top) {
            candidates.push(inside_top);
        }
        if fits(inside_bottom) {
            candidates.push(inside_bottom);
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

    pub(super) fn toolbar_rect(&self) -> Option<Bounds<Pixels>> {
        let selection = self.selection_bounds()?;
        let window_width = f32::from(self.window_size.width);
        let window_height = f32::from(self.window_size.height);
        let width = SELECTION_TOOLBAR_WIDTH.min(window_width - 16.0);
        let selection_left = f32::from(selection.origin.x);
        let selection_top = f32::from(selection.origin.y);
        let selection_right = selection_left + f32::from(selection.size.width);
        let selection_bottom = selection_top + f32::from(selection.size.height);
        let left = ((selection_left + selection_right) / 2.0 - width / 2.0)
            .clamp(8.0, (window_width - width - 8.0).max(8.0));
        let below = selection_bottom + REGION_GAP;
        let above = selection_top - SELECTION_TOOLBAR_HEIGHT - REGION_GAP;
        let top = if below + SELECTION_TOOLBAR_HEIGHT <= window_height - 8.0 {
            below
        } else if above >= 8.0 {
            above
        } else {
            (window_height - SELECTION_TOOLBAR_HEIGHT - 8.0).max(8.0)
        };
        Some(Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(SELECTION_TOOLBAR_HEIGHT)),
        })
    }

    fn info_field_value(&self, selected: &Selected, field: InfoField) -> i64 {
        let monitor = self.session.monitor_bounds().origin;
        let scale = selected.scale;
        let value = match field {
            InfoField::X => monitor.x + selected.rect.origin.x,
            InfoField::Y => monitor.y + selected.rect.origin.y,
            InfoField::Width => selected.rect.size.width as i32,
            InfoField::Height => selected.rect.size.height as i32,
        };
        match selected.unit {
            SizeUnit::Px => i64::from(value),
            SizeUnit::Dp => (value as f32 / scale).round() as i64,
        }
    }

    pub(super) fn info_field_display(&self, selected: &Selected, field: InfoField) -> String {
        self.info_field_value(selected, field).to_string()
    }

    pub(super) fn start_edit(&mut self, field: InfoField, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.as_ref() else {
            return;
        };
        let value = self.info_field_display(selected, field);
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        selected.edit = Some(FieldEdit {
            field,
            buffer: value,
            fresh: true,
        });
        selected.effects_open = false;
        cx.notify();
    }

    pub(super) fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.edit = None;
            cx.notify();
        }
    }

    pub(super) fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let Some(edit) = selected.edit.clone() else {
            return;
        };
        let Ok(value) = edit.buffer.trim().parse::<f32>() else {
            self.cancel_edit(cx);
            return;
        };
        let scale = selected.scale.max(0.1);
        let physical = match selected.unit {
            SizeUnit::Px => value.round() as i32,
            SizeUnit::Dp => (value * scale).round() as i32,
        };
        let monitor = self.session.monitor_bounds();
        let mut rect = selected.rect;
        let ratio = selected.ratio.ratio();
        match edit.field {
            InfoField::X => {
                rect.origin.x = (physical - monitor.origin.x).clamp(
                    0,
                    (monitor.size.width as i32 - rect.size.width as i32).max(0),
                );
            }
            InfoField::Y => {
                rect.origin.y = (physical - monitor.origin.y).clamp(
                    0,
                    (monitor.size.height as i32 - rect.size.height as i32).max(0),
                );
            }
            InfoField::Width => {
                let width = physical.max(MIN_REGION_SIZE);
                let max_width = (monitor.size.width as i32 - rect.origin.x).max(MIN_REGION_SIZE);
                let max_height = (monitor.size.height as i32 - rect.origin.y).max(MIN_REGION_SIZE);
                let (width, height) = if let Some(ratio) = ratio {
                    fit_ratio_to_bounds(width, ratio, max_width, max_height)
                } else {
                    (width.min(max_width), rect.size.height as i32)
                };
                rect.size.width = width as u32;
                rect.size.height = height.max(MIN_REGION_SIZE) as u32;
            }
            InfoField::Height => {
                let height = physical.max(MIN_REGION_SIZE);
                let max_width = (monitor.size.width as i32 - rect.origin.x).max(MIN_REGION_SIZE);
                let max_height = (monitor.size.height as i32 - rect.origin.y).max(MIN_REGION_SIZE);
                let (width, height) = if let Some(ratio) = ratio {
                    fit_ratio_to_bounds(
                        (height as f32 * ratio).round() as i32,
                        ratio,
                        max_width,
                        max_height,
                    )
                } else {
                    (rect.size.width as i32, height.min(max_height))
                };
                rect.size.width = width.max(MIN_REGION_SIZE) as u32;
                rect.size.height = height as u32;
            }
        }
        if let Some(selected) = self.selected.as_mut() {
            selected.rect = rect;
            selected.edit = None;
            selected.target =
                CaptureTarget::Region(CaptureRegion::new(self.session.monitor(), rect));
        }
        cx.notify();
    }

    pub(super) fn next_edit_field(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let Some(edit) = selected.edit.clone() else {
            return;
        };
        self.commit_edit(cx);
        self.start_edit(edit.field.next(), cx);
    }

    pub(super) fn cycle_ratio(&mut self, cx: &mut Context<Self>) {
        let next = self.aspect.next();
        self.aspect = next;
        if let Some(selected) = self.selected.as_mut() {
            selected.ratio = next;
            selected.edit = None;
            if let Some(ratio) = next.ratio() {
                let monitor = self.session.monitor_bounds();
                selected.rect = fit_ratio_rect(selected.rect, ratio, monitor.size);
                selected.target = CaptureTarget::Region(CaptureRegion::new(
                    self.session.monitor(),
                    selected.rect,
                ));
            }
        }
        self.session.set_aspect_ratio(next.ratio());
        cx.notify();
    }

    pub(super) fn nudge_selection(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        let monitor = self.session.monitor_bounds();
        let mut rect = selected.rect;
        rect.origin.x += dx;
        rect.origin.y += dy;
        rect = clamp_to_monitor(rect, monitor.size);
        selected.rect = rect;
        selected.target = CaptureTarget::Region(CaptureRegion::new(self.session.monitor(), rect));
        cx.notify();
    }

    pub(super) fn update_region_drag(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        let Some(drag) = selected.drag else {
            return;
        };
        let monitor = self.session.monitor_bounds();
        let local = PhysicalPoint::new(point.x - monitor.origin.x, point.y - monitor.origin.y);
        let rect = match drag.kind {
            RegionDragKind::Move => {
                let dx = local.x - (drag.start.x - monitor.origin.x);
                let dy = local.y - (drag.start.y - monitor.origin.y);
                let mut rect = drag.original;
                rect.origin.x += dx;
                rect.origin.y += dy;
                clamp_to_monitor(rect, monitor.size)
            }
            RegionDragKind::Resize(handle) => resize_rect(
                drag.original,
                handle,
                local,
                selected.ratio.ratio(),
                monitor.size,
            ),
        };
        selected.rect = rect;
        selected.target = CaptureTarget::Region(CaptureRegion::new(self.session.monitor(), rect));
        cx.notify();
    }

    pub(super) fn finish_region_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            let changed = selected
                .drag
                .is_some_and(|drag| drag.original != selected.rect);
            selected.drag = None;
            if changed {
                selected.target = CaptureTarget::Region(CaptureRegion::new(
                    self.session.monitor(),
                    selected.rect,
                ));
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_effects_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.effects_open = !selected.effects_open;
            selected.hints_open = false;
            selected.edit = None;
        }
        super::hints_window::close(cx);
        cx.notify();
    }

    pub(super) fn toggle_hints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        selected.hints_open = !selected.hints_open;
        selected.effects_open = false;
        let open = selected.hints_open;
        if open {
            let owner = cx.entity().downgrade();
            let owner_handle = window.window_handle();
            let owner_focus = self.focus_handle.clone();
            let display = window.display(cx).map(|display| display.id());
            let bounds = window.bounds();
            let theme = self.chrome;
            cx.defer(move |cx| {
                super::hints_window::show(
                    owner,
                    owner_handle,
                    owner_focus,
                    display,
                    theme,
                    bounds,
                    cx,
                );
            });
        } else {
            super::hints_window::close(cx);
        }
        cx.notify();
    }

    pub(super) fn hints_window_closed(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.hints_open = false;
        }
        cx.notify();
    }

    pub(super) fn set_region_radius(&mut self, radius: u32, cx: &mut Context<Self>) {
        self.effects.corner_radius = radius;
        settings_state::update(|settings| {
            settings.region_corner_radius = radius;
        });
        cx.notify();
    }

    pub(super) fn toggle_region_shadow(&mut self, cx: &mut Context<Self>) {
        self.effects.shadow = !self.effects.shadow;
        let shadow = self.effects.shadow;
        settings_state::update(|settings| settings.region_shadow = shadow);
        cx.notify();
    }

    pub(super) fn dismiss_effects(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.effects_open = false;
        }
        cx.notify();
    }

    pub(super) fn dismiss_hints(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.hints_open = false;
        }
        super::hints_window::close(cx);
        cx.notify();
    }
}
