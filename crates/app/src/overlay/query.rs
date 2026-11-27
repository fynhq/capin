use std::collections::HashSet;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use super::element_cache;
use super::element_probe::ElementChain;
use super::*;

const SLOW_ELEMENT_QUERY: Duration = Duration::from_millis(500);
const EMPTY_ELEMENT_RETRY_DELAY: Duration = Duration::from_millis(150);
const FULL_QUERY_IDLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
enum FullSchedule {
    Run,
    Wait,
    Cancel,
}

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

    pub(super) fn seed_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cursor.is_some() {
            return;
        }
        let Some((x, y)) = platform_window::cursor_screen_position() else {
            return;
        };
        let point = PhysicalPoint::new(x, y);
        if !self.session.monitor_bounds().contains(point) {
            return;
        }
        self.cursor = Some(point);
        self.session.pointer_moved(point);
        if !self.session.has_drag() {
            self.refresh_auto_target(point, cx);
        }
        self.update_magnifier(window);
        cx.notify();
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
            self.element_probe.retain_cache_for(window.id);
            self.element_probe.switch_window(window.id);
            self.session.set_hovered_element(None);
        }
        #[cfg(target_os = "windows")]
        if !self.element_probe.cache.contains_key(&window.id)
            && let Some(elements) = element_cache::lookup(window.id, window.bounds)
        {
            log_element_query("cache", window.id, Duration::ZERO, elements.len());
            self.element_probe.chain = None;
            self.element_probe.cache.insert(window.id, elements);
        }
        self.start_element_queries(window.id, point, cx);

        let mut elements = Vec::new();
        if let Some(cached) = self.element_probe.cache.get(&window.id) {
            elements.extend_from_slice(cached);
        }
        if let Some(chain) = self
            .element_probe
            .chain
            .as_ref()
            .filter(|chain| chain.window == window.id)
        {
            elements.extend_from_slice(&chain.elements);
        }
        let mut candidates = core_capture::element_candidates(&elements, window.bounds, point);
        let mut seen = HashSet::new();
        candidates.retain(|rect| seen.insert(*rect));
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

    fn start_element_queries(
        &mut self,
        window: WindowId,
        point: PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        let has_full = self.element_probe.cache.contains_key(&window);
        let chain_matches = self
            .element_probe
            .chain
            .as_ref()
            .is_some_and(|chain| chain.window == window && chain.point == point);
        let need_chain =
            (!has_full || !platform_capture::ui_element_tree_is_complete()) && !chain_matches;
        if need_chain && self.element_probe.pending_chain.is_none() {
            self.query_element_chain(window, point, cx);
        }
        #[cfg(target_os = "windows")]
        let skip_full = element_cache::is_large(window);
        #[cfg(not(target_os = "windows"))]
        let skip_full = false;
        if !has_full && !skip_full && self.element_probe.pending_full.is_none() {
            self.query_full_elements(window, cx);
        }
    }

    fn query_element_chain(
        &mut self,
        window: WindowId,
        point: PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        self.element_probe.pending_chain = Some((window, point));
        cx.spawn(async move |this, cx| {
            let (elements, elapsed) = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    let elements = platform_capture::ui_element_chain(window, point);
                    (elements, started.elapsed())
                })
                .await;
            log_element_query("chain", window, elapsed, elements.len());
            this.update(cx, |this, cx| {
                if this.element_probe.pending_chain != Some((window, point)) {
                    return;
                }
                this.element_probe.pending_chain = None;
                let full_complete = platform_capture::ui_element_tree_is_complete()
                    && this.element_probe.cache.contains_key(&window);
                if this.element_probe.window == Some(window) && !full_complete {
                    this.element_probe.chain = Some(ElementChain {
                        window,
                        point,
                        elements,
                    });
                }
                if let Some(cursor) = this.cursor {
                    this.refresh_auto_target(cursor, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn query_full_elements(&mut self, window: WindowId, cx: &mut Context<Self>) {
        self.element_probe.pending_full = Some(window);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FULL_QUERY_IDLE).await;
                let schedule = this.update(cx, |this, cx| {
                    if this.element_probe.pending_full != Some(window) {
                        return FullSchedule::Cancel;
                    }
                    if this.element_probe.window != Some(window)
                        || this.element_probe.cache.contains_key(&window)
                    {
                        this.element_probe.pending_full = None;
                        if let Some(cursor) = this.cursor {
                            this.refresh_auto_target(cursor, cx);
                        }
                        return FullSchedule::Cancel;
                    }
                    if this.element_probe.pending_chain.is_some() {
                        FullSchedule::Wait
                    } else {
                        FullSchedule::Run
                    }
                });
                match schedule {
                    Ok(FullSchedule::Run) => break,
                    Ok(FullSchedule::Wait) => {}
                    _ => return,
                }
            }
            let (elements, elapsed) = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    let elements = platform_capture::ui_elements(window);
                    (elements, started.elapsed())
                })
                .await;
            log_element_query("full", window, elapsed, elements.len());
            let empty = elements.is_empty();
            let applied = this
                .update(cx, |this, cx| {
                    if this.element_probe.pending_full != Some(window) {
                        return false;
                    }
                    this.element_probe.pending_full = None;
                    if this.element_probe.window != Some(window) {
                        if let Some(cursor) = this.cursor {
                            this.refresh_auto_target(cursor, cx);
                        }
                        return false;
                    }
                    if empty && !platform_window::accessibility_trusted() {
                        return false;
                    }
                    #[cfg(target_os = "windows")]
                    if let Some(bounds) = this.session.window(window).map(|info| info.bounds) {
                        element_cache::store(window, bounds, &elements, elapsed);
                    }
                    this.element_probe.cache.insert(window, elements);
                    if platform_capture::ui_element_tree_is_complete() {
                        this.element_probe.chain = None;
                    }
                    if let Some(cursor) = this.cursor {
                        this.refresh_auto_target(cursor, cx);
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !applied || !empty {
                return;
            }
            cx.background_executor()
                .timer(EMPTY_ELEMENT_RETRY_DELAY)
                .await;
            let retry = this
                .update(cx, |this, _| this.element_probe.retry.insert(window))
                .unwrap_or(false);
            if !retry {
                return;
            }
            let (elements, elapsed) = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    let elements = platform_capture::ui_elements(window);
                    (elements, started.elapsed())
                })
                .await;
            log_element_query("full-retry", window, elapsed, elements.len());
            this.update(cx, |this, cx| {
                if this.element_probe.window != Some(window)
                    || !this
                        .element_probe
                        .cache
                        .get(&window)
                        .is_some_and(Vec::is_empty)
                {
                    return;
                }
                #[cfg(target_os = "windows")]
                if let Some(bounds) = this.session.window(window).map(|info| info.bounds) {
                    element_cache::store(window, bounds, &elements, elapsed);
                }
                this.element_probe.cache.insert(window, elements);
                if let Some(cursor) = this.cursor {
                    this.refresh_auto_target(cursor, cx);
                }
                cx.notify();
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

fn log_element_query(stage: &str, window: WindowId, elapsed: Duration, elements: usize) {
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    crate::logging::log_debug!(
        "element",
        "{stage} window={} elapsed_ms={elapsed_ms:.1} elements={elements}",
        window.raw()
    );
    if stage != "chain" && elapsed >= SLOW_ELEMENT_QUERY {
        crate::logging::log_info!(
            "element",
            "{stage} window={} elapsed_ms={elapsed_ms:.1} elements={elements}",
            window.raw()
        );
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
