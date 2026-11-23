use super::*;

impl OverlayView {
    pub(super) fn sample_color(&self, point: PhysicalPoint) -> Option<(u8, u8, u8)> {
        let monitor = self.session.monitor_bounds();
        let local_x = point.x - monitor.origin.x;
        let local_y = point.y - monitor.origin.y;
        if local_x < 0 || local_y < 0 {
            return None;
        }
        image_util::image_pixel(&self.image, local_x as u32, local_y as u32)
            .map(|[red, green, blue, _]| (red, green, blue))
    }

    pub(super) fn refresh_auto_target(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) {
        if self.session.is_dragging() || self.selected.is_some() {
            return;
        }
        let Some(window) = self.session.window_under_pointer().cloned() else {
            self.element_probe.clear_hover();
            self.screen_targeting = false;
            self.session.set_screen_targeting(false);
            self.session.set_hovered_element(None);
            return;
        };
        if self.window_only {
            self.element_probe.clear_hover();
            self.screen_targeting = false;
            self.session.set_screen_targeting(false);
            self.session.set_hovered_element(None);
            return;
        }
        if self.element_probe.window != Some(window.id) {
            self.element_probe.window = Some(window.id);
            self.element_probe.cache_point = None;
            self.element_probe.level = 0;
            self.element_probe.candidates.clear();
            self.element_probe.scroll.reset();
            self.session.set_hovered_element(None);
        }
        let cached_for_point = {
            #[cfg(target_os = "macos")]
            {
                self.element_probe.cache.contains_key(&window.id)
                    && self.element_probe.cache_point == Some(point)
            }
            #[cfg(not(target_os = "macos"))]
            {
                self.element_probe.cache.contains_key(&window.id)
            }
        };
        if !cached_for_point {
            if self.element_probe.pending.is_none() {
                self.query_elements(window.id, point, cx);
            }
            self.session.set_hovered_element(None);
            return;
        }
        let Some(cached) = self.element_probe.cache.get(&window.id) else {
            self.session.set_hovered_element(None);
            return;
        };

        let candidates = core_capture::element_candidates(cached, window.bounds, point);
        if self.element_probe.level > candidates.len() {
            self.element_probe.level = candidates.len();
        }
        self.element_probe.candidates = candidates;
        self.session.set_screen_targeting(self.screen_targeting);
        let selected = (!self.screen_targeting)
            .then(|| {
                self.element_probe
                    .candidates
                    .get(self.element_probe.level)
                    .copied()
            })
            .flatten();
        self.session.set_hovered_element(selected);
    }

    fn query_elements(
        &mut self,
        window_id: WindowId,
        point: PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        self.element_probe.pending = Some(window_id);
        self.element_probe.pending_point = Some(point);
        cx.spawn(async move |this, cx| {
            let elements = cx
                .background_executor()
                .spawn(async move { platform_capture::ui_elements_at(window_id, point) })
                .await;
            let empty = elements.is_empty();
            let applied = this
                .update(cx, |this, cx| {
                    if this.element_probe.pending != Some(window_id)
                        || this.element_probe.pending_point != Some(point)
                    {
                        return false;
                    }
                    this.element_probe.pending = None;
                    this.element_probe.pending_point = None;
                    if empty && !platform_window::accessibility_trusted() {
                        return false;
                    }
                    this.element_probe.cache.insert(window_id, elements);
                    #[cfg(target_os = "macos")]
                    {
                        this.element_probe.cache_point = Some(point);
                    }
                    if let Some(point) = this.cursor {
                        this.refresh_auto_target(point, cx);
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !applied || !empty {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let retry = this
                .update(cx, |this, _| this.element_probe.retry.insert(window_id))
                .unwrap_or(false);
            if !retry {
                return;
            }
            let elements = cx
                .background_executor()
                .spawn(async move { platform_capture::ui_elements_at(window_id, point) })
                .await;
            this.update(cx, |this, cx| {
                let still_current = this.element_probe.pending.is_none()
                    && this.element_probe.cache_point.unwrap_or(point) == point;
                if still_current
                    && this
                        .element_probe
                        .cache
                        .get(&window_id)
                        .is_some_and(Vec::is_empty)
                {
                    this.element_probe.cache.insert(window_id, elements);
                    #[cfg(target_os = "macos")]
                    {
                        this.element_probe.cache_point = Some(point);
                    }
                    if let Some(point) = this.cursor {
                        this.refresh_auto_target(point, cx);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn set_window_only(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let enabled = enabled && self.selected.is_none() && self.editor.is_none();
        if self.window_only == enabled {
            return;
        }
        self.window_only = enabled;
        self.element_probe.clear_hover();
        self.screen_targeting = false;
        self.session.set_screen_targeting(false);
        self.session.set_hovered_element(None);
        if let Some(point) = self.cursor {
            self.session.pointer_moved(point);
            self.refresh_auto_target(point, cx);
        }
        platform_window::set_cursor_hidden(enabled && self.cursor.is_some());
        cx.notify();
    }

    pub(super) fn toggle_color_format(&mut self, cx: &mut Context<Self>) {
        self.color_format = self.color_format.next();
        cx.notify();
    }
}

fn color_text(format: ColorFormat, red: u8, green: u8, blue: u8) -> String {
    match format {
        ColorFormat::Hex => format!("#{red:02x}{green:02x}{blue:02x}"),
        ColorFormat::Rgb => format!("rgb({red}, {green}, {blue})"),
    }
}

impl OverlayView {
    pub(super) fn update_magnifier(&mut self, window: &mut Window) {
        if self.selected.is_some() {
            self.clear_magnifier(window);
            return;
        }
        if self.session.is_dragging() {
            self.clear_magnifier(window);
            return;
        }
        let Some(point) = self.cursor else {
            self.clear_magnifier(window);
            return;
        };
        if self
            .magnifier
            .as_ref()
            .is_some_and(|magnifier| magnifier.point == point)
        {
            return;
        }
        let monitor = self.session.monitor_bounds();
        let output_size = (MAGNIFIER_SIZE as f32 * window.scale_factor().max(0.1)).round() as u32;
        let image = image_util::magnifier_image(
            &self.image,
            point.x - monitor.origin.x,
            point.y - monitor.origin.y,
            MAGNIFIER_SOURCE,
            output_size.max(1),
        );
        if let Some(previous) = self.magnifier.replace(Magnifier { point, image }) {
            let _ = window.drop_image(previous.image);
        }
    }

    pub(super) fn clear_magnifier(&mut self, window: &mut Window) {
        if let Some(magnifier) = self.magnifier.take() {
            let _ = window.drop_image(magnifier.image);
        }
    }

    pub(super) fn refresh_geometry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scale = window.scale_factor();
        let size = window.bounds().size;
        if f32::from(size.width) <= 0.0 || f32::from(size.height) <= 0.0 {
            return;
        }
        if (self.scale - scale).abs() <= f32::EPSILON && self.window_size == size {
            return;
        }
        self.scale = scale;
        self.window_size = size;
        self.session.set_scale(scale);
        self.clear_magnifier(window);
        cx.notify();
    }

    pub(super) fn copy_color(&mut self, cx: &mut Context<Self>) {
        let Some(point) = self
            .magnifier
            .as_ref()
            .map(|magnifier| magnifier.point)
            .or(self.cursor)
        else {
            return;
        };
        let Some((red, green, blue)) = self.sample_color(point) else {
            return;
        };
        let text = color_text(self.color_format, red, green, blue);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.close_overlays(cx);
    }

    pub(super) fn cycle_aspect(&mut self, cx: &mut Context<Self>) {
        self.aspect = self.aspect.next();
        self.session.set_aspect_ratio(self.aspect.ratio());
        cx.notify();
    }
}

pub(super) fn window_label(
    chrome: &ChromeTheme,
    language: Language,
    window: &WindowInfo,
    rect: Bounds<Pixels>,
    element: Option<PhysicalRect>,
) -> Div {
    let title = if window.title.is_empty() {
        t(language, "overlay.untitled")
    } else {
        window.title.as_str()
    };
    let (kind, width, height, icon, icon_color) = match element {
        Some(element) => (
            t(language, "overlay.element"),
            element.size.width,
            element.size.height,
            icons::SELECT,
            rgb(ELEMENT_HOVER),
        ),
        None => (
            t(language, "overlay.window"),
            window.bounds.size.width,
            window.bounds.size.height,
            icons::WINDOW,
            chrome.icon,
        ),
    };
    let text = format!("{title} · {kind}  {width}x{height}");
    let top = (f32::from(rect.origin.y) - 26.0).max(0.0);

    div()
        .absolute()
        .left(rect.origin.x)
        .top(px(top))
        .px_2()
        .py_1()
        .bg(chrome.surface)
        .border_1()
        .border_color(if element.is_some() {
            rgba((ELEMENT_HOVER << 8) | 0xCC)
        } else {
            chrome.border
        })
        .shadow(chrome.shadow())
        .rounded(px(6.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(icons::icon(icon, 12.0, icon_color))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(chrome.text)
                        .child(text),
                ),
        )
}

pub(super) fn screen_label(chrome: &ChromeTheme, language: Language, size: PhysicalSize) -> Div {
    let width = size.width;
    let height = size.height;
    div()
        .absolute()
        .left(px(16.0))
        .top(px(16.0))
        .px_2()
        .py_1()
        .bg(chrome.surface)
        .border_1()
        .border_color(chrome.border)
        .shadow(chrome.shadow())
        .rounded(px(6.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(icons::icon(icons::SCREEN, 12.0, chrome.icon))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(chrome.text)
                        .child(format!(
                            "{} · {}x{}",
                            t(language, "overlay.screen"),
                            width,
                            height
                        )),
                ),
        )
}
