use std::sync::Arc;
use std::time::Duration;

use core_capture::{
    CaptureTarget, MonitorInfo, SelectionAction, SelectionOutcome, SelectionSession, WindowInfo,
    platform_backend,
};
use core_types::{MonitorId, PhysicalPoint, PhysicalRect, RgbaFrame};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style, system_accent_hex};

use crate::{
    i18n::{Language, t},
    icons, image_util, settings_state,
};

const DIM: u32 = 0x0000004d;
const FALLBACK_HOVER_ACCENT: u32 = 0x4c9aff;
const REGION_BORDER: u32 = 0xffffff;
const MAGNIFIER_SIZE: u32 = 144;
const MAGNIFIER_SOURCE: u32 = 24;
const MAGNIFIER_PANEL: f32 = 160.0;
const MAGNIFIER_PANEL_HEIGHT: f32 = 228.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AspectPreset {
    Free,
    Square,
    FourThree,
    SixteenNine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorFormat {
    Hex,
    Rgb,
}

impl ColorFormat {
    fn next(self) -> Self {
        match self {
            Self::Hex => Self::Rgb,
            Self::Rgb => Self::Hex,
        }
    }
}

impl AspectPreset {
    const ALL: [Self; 4] = [Self::Free, Self::Square, Self::FourThree, Self::SixteenNine];

    fn ratio(self) -> Option<f32> {
        match self {
            Self::Free => None,
            Self::Square => Some(1.0),
            Self::FourThree => Some(4.0 / 3.0),
            Self::SixteenNine => Some(16.0 / 9.0),
        }
    }

    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Free => t(language, "overlay.ratio_free"),
            Self::Square => "1:1",
            Self::FourThree => "4:3",
            Self::SixteenNine => "16:9",
        }
    }

    fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    fn id(self) -> &'static str {
        match self {
            Self::Free => "overlay-ratio-free",
            Self::Square => "overlay-ratio-square",
            Self::FourThree => "overlay-ratio-four-three",
            Self::SixteenNine => "overlay-ratio-sixteen-nine",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayMode {
    Capture,
    Scroll,
    Ocr,
    Record,
}

pub fn open_overlays(cx: &mut App) {
    open_overlays_mode(cx, OverlayMode::Capture);
}

pub fn open_scroll_overlays(cx: &mut App) {
    open_overlays_mode(cx, OverlayMode::Scroll);
}

pub fn open_ocr_overlays(cx: &mut App) {
    open_overlays_mode(cx, OverlayMode::Ocr);
}

pub fn open_record_overlays(cx: &mut App) {
    open_overlays_mode(cx, OverlayMode::Record);
}

fn open_overlays_mode(cx: &mut App, mode: OverlayMode) {
    #[cfg(target_os = "windows")]
    core_capture::init_process_dpi_awareness();

    let mut backend = platform_backend();
    let monitors = match backend.monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            eprintln!("capin: failed to enumerate monitors: {error}");
            return;
        }
    };
    let windows = match backend.windows() {
        Ok(windows) => windows,
        Err(error) => {
            eprintln!("capin: failed to enumerate windows: {error}");
            return;
        }
    };

    let accent = system_accent_hex()
        .map(rgb)
        .unwrap_or(rgb(FALLBACK_HOVER_ACCENT));
    let mut opened = 0usize;
    for monitor in monitors {
        let frame = match backend.capture(CaptureTarget::Monitor(monitor.id)) {
            Ok(frame) => frame,
            Err(error) => {
                eprintln!("capin: failed to capture monitor {:?}: {error}", monitor.id);
                continue;
            }
        };

        let window_bounds = logical_bounds(&monitor);
        let window_size = window_bounds.size;
        let frame = Arc::new(frame);
        let image = image_util::render_image(&frame);
        let scale = monitor.scale.get();
        let session = SelectionSession::new(monitor.id, monitor.bounds, windows.clone());
        let options = overlay_options(monitor.id, window_bounds);

        match gpui_kit::open_window(options, cx, move |window, cx| {
            if let Err(error) = apply_window_style(&*window, WindowStyle::OVERLAY) {
                eprintln!("capin: failed to style overlay: {error}");
            }
            let view = cx.new(|cx| {
                OverlayView::new(session, image, frame, scale, window_size, accent, mode, cx)
            });
            let focus = view.read(cx).focus_handle.clone();
            window.defer(cx, move |window, cx| {
                window.focus(&focus, cx);
                window.activate_window();
            });
            view
        }) {
            Ok(_) => opened += 1,
            Err(error) => eprintln!("capin: failed to open overlay: {error}"),
        }
    }

    if opened == 0 {
        eprintln!("capin: no overlay windows could be opened");
    }
}

fn overlay_options(monitor: MonitorId, bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(DisplayId::new(monitor.raw())),
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("capin-overlay".to_owned()),
        ..Default::default()
    }
}

fn logical_bounds(monitor: &MonitorInfo) -> Bounds<Pixels> {
    let scale = monitor.scale.get();
    Bounds {
        origin: point(
            px(monitor.bounds.origin.x as f32 / scale),
            px(monitor.bounds.origin.y as f32 / scale),
        ),
        size: size(
            px(monitor.bounds.size.width as f32 / scale),
            px(monitor.bounds.size.height as f32 / scale),
        ),
    }
}

struct Magnifier {
    point: PhysicalPoint,
    image: Arc<RenderImage>,
}

struct OverlayView {
    session: SelectionSession,
    image: Arc<RenderImage>,
    frame: Arc<RgbaFrame>,
    scale: f32,
    window_size: Size<Pixels>,
    cursor: Option<PhysicalPoint>,
    suppress_capture: bool,
    accent: Rgba,
    mode: OverlayMode,
    aspect: AspectPreset,
    color_format: ColorFormat,
    window_targeting: bool,
    shift_held: bool,
    magnifier: Option<Magnifier>,
    copy_serial: u64,
    copied: bool,
    focus_handle: FocusHandle,
}

impl OverlayView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        session: SelectionSession,
        image: Arc<RenderImage>,
        frame: Arc<RgbaFrame>,
        scale: f32,
        window_size: Size<Pixels>,
        accent: Rgba,
        mode: OverlayMode,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            session,
            image,
            frame,
            scale,
            window_size,
            cursor: None,
            suppress_capture: false,
            accent,
            mode,
            aspect: AspectPreset::Free,
            color_format: ColorFormat::Hex,
            window_targeting: false,
            shift_held: false,
            magnifier: None,
            copy_serial: 0,
            copied: false,
            focus_handle: cx.focus_handle(),
        }
    }

    fn to_physical(&self, position: Point<Pixels>) -> PhysicalPoint {
        let origin = self.session.monitor_bounds().origin;
        PhysicalPoint::new(
            origin.x + (f32::from(position.x) * self.scale).round() as i32,
            origin.y + (f32::from(position.y) * self.scale).round() as i32,
        )
    }

    fn local_bounds(&self, bounds: PhysicalRect) -> Option<Bounds<Pixels>> {
        let monitor = self.session.monitor_bounds();
        let clipped = bounds.intersection(monitor)?;
        Some(Bounds {
            origin: point(
                px((clipped.origin.x - monitor.origin.x) as f32 / self.scale),
                px((clipped.origin.y - monitor.origin.y) as f32 / self.scale),
            ),
            size: size(
                px(clipped.size.width as f32 / self.scale),
                px(clipped.size.height as f32 / self.scale),
            ),
        })
    }

    fn dim_layers(&self, hole: Option<Bounds<Pixels>>) -> Vec<Div> {
        let width = f32::from(self.window_size.width);
        let height = f32::from(self.window_size.height);
        let Some(hole) = hole else {
            return vec![
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .w(px(width))
                    .h(px(height))
                    .bg(rgba(DIM)),
            ];
        };

        let left = f32::from(hole.origin.x);
        let top = f32::from(hole.origin.y);
        let right = left + f32::from(hole.size.width);
        let bottom = top + f32::from(hole.size.height);
        let mut layers = Vec::new();

        if top > 0.0 {
            layers.push(dim_rect(0.0, 0.0, width, top));
        }
        if bottom < height {
            layers.push(dim_rect(0.0, bottom, width, height - bottom));
        }
        if left > 0.0 {
            layers.push(dim_rect(0.0, top, left, bottom - top));
        }
        if right < width {
            layers.push(dim_rect(right, top, width - right, bottom - top));
        }

        layers
    }

    fn sample_color(&self, point: PhysicalPoint) -> Option<(u8, u8, u8)> {
        let monitor = self.session.monitor_bounds();
        let local_x = point.x - monitor.origin.x;
        let local_y = point.y - monitor.origin.y;
        if local_x < 0 || local_y < 0 {
            return None;
        }
        self.frame
            .pixel(local_x as u32, local_y as u32)
            .map(|[red, green, blue, _]| (red, green, blue))
    }

    fn set_window_targeting(&mut self, enabled: bool) {
        if self.window_targeting == enabled {
            return;
        }
        self.window_targeting = enabled;
        self.session.set_window_targeting(enabled);
        if let Some(point) = self.cursor {
            self.session.pointer_moved(point);
        }
    }

    fn toggle_color_format(&mut self, cx: &mut Context<Self>) {
        self.color_format = self.color_format.next();
        cx.notify();
    }

    fn color_text(&self, red: u8, green: u8, blue: u8) -> String {
        match self.color_format {
            ColorFormat::Hex => format!("#{red:02x}{green:02x}{blue:02x}"),
            ColorFormat::Rgb => format!("rgb({red}, {green}, {blue})"),
        }
    }

    fn update_magnifier(&mut self) {
        if self.mode != OverlayMode::Capture {
            self.magnifier = None;
            return;
        }
        let Some(point) = self.cursor else {
            self.magnifier = None;
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
        let image = image_util::magnifier_image(
            &self.frame,
            point.x - monitor.origin.x,
            point.y - monitor.origin.y,
            (MAGNIFIER_SOURCE as f32 * self.scale).round().max(1.0) as u32,
            MAGNIFIER_SIZE,
        );
        self.magnifier = Some(Magnifier { point, image });
    }

    fn copy_color(&mut self, cx: &mut Context<Self>) {
        let Some(point) = self.cursor else {
            return;
        };
        let Some((red, green, blue)) = self.sample_color(point) else {
            return;
        };
        let text = self.color_text(red, green, blue);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = true;
        self.copy_serial = self.copy_serial.wrapping_add(1);
        let serial = self.copy_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            this.update(cx, |this, cx| {
                if this.copy_serial == serial {
                    this.copied = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn cycle_aspect(&mut self, cx: &mut Context<Self>) {
        self.aspect = self.aspect.next();
        self.session.set_aspect_ratio(self.aspect.ratio());
        cx.notify();
    }

    fn window_label(&self, window: &WindowInfo, rect: Bounds<Pixels>) -> Div {
        let title = if window.title.is_empty() {
            t(settings_state::get().language, "overlay.window")
        } else {
            window.title.as_str()
        };
        let text = format!(
            "{title}  {}x{}",
            window.bounds.size.width, window.bounds.size.height
        );
        let top = (f32::from(rect.origin.y) - 26.0).max(0.0);

        div()
            .absolute()
            .left(rect.origin.x)
            .top(px(top))
            .px_2()
            .py_1()
            .bg(rgba(0x000000cc))
            .rounded_lg()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(icons::icon(icons::WINDOW, 12.0, rgb(0xffffff)))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xffffff))
                            .child(text),
                    ),
            )
    }

    fn ratio_bar(&self, language: Language, cx: &mut Context<Self>) -> Div {
        let mut control = div()
            .id("overlay-ratios")
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .flex()
            .flex_row()
            .gap(px(2.0))
            .p(px(3.0))
            .rounded(px(9.0))
            .bg(rgba(0x000000CC));
        for preset in AspectPreset::ALL {
            let selected = self.aspect == preset;
            control = control.child(
                div()
                    .id(preset.id())
                    .px(px(10.0))
                    .py(px(5.0))
                    .rounded(px(7.0))
                    .when(selected, |button| button.bg(self.accent))
                    .text_size(px(11.5))
                    .text_color(if selected {
                        rgb(0xFFFFFF)
                    } else {
                        rgba(0xFFFFFFAA)
                    })
                    .hover(move |style| style.bg(rgba(0xFFFFFF1A)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.aspect = preset;
                        this.session.set_aspect_ratio(preset.ratio());
                        cx.notify();
                    }))
                    .child(preset.label(language)),
            );
        }
        div()
            .absolute()
            .top(px(16.0))
            .left(px(0.0))
            .right(px(0.0))
            .flex()
            .justify_center()
            .child(control)
    }

    fn magnifier_panel(
        &self,
        selection: Option<Bounds<Pixels>>,
        language: Language,
    ) -> Option<Div> {
        let magnifier = self.magnifier.as_ref()?;
        let point = self.cursor?;
        let monitor = self.session.monitor_bounds();
        let local_x = (point.x - monitor.origin.x) as f32 / self.scale;
        let local_y = (point.y - monitor.origin.y) as f32 / self.scale;
        let window_width = f32::from(self.window_size.width);
        let window_height = f32::from(self.window_size.height);
        let mut left = local_x + 18.0;
        if left + MAGNIFIER_PANEL > window_width {
            left = local_x - MAGNIFIER_PANEL - 18.0;
        }
        let mut top = local_y + 18.0;
        if top + MAGNIFIER_PANEL_HEIGHT > window_height {
            top = local_y - MAGNIFIER_PANEL_HEIGHT - 18.0;
        }
        left = left.clamp(0.0, (window_width - MAGNIFIER_PANEL).max(0.0));
        top = top.clamp(0.0, (window_height - MAGNIFIER_PANEL_HEIGHT).max(0.0));

        let color = self.sample_color(point);
        let mut lines: Vec<Div> = Vec::new();
        for step in 1..3 {
            let offset = MAGNIFIER_SIZE as f32 * step as f32 / 3.0;
            lines.push(
                div()
                    .absolute()
                    .left(px(offset))
                    .top(px(0.0))
                    .w(px(1.0))
                    .h(px(MAGNIFIER_SIZE as f32))
                    .bg(rgba(0xFFFFFF30)),
            );
            lines.push(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(offset))
                    .w(px(MAGNIFIER_SIZE as f32))
                    .h(px(1.0))
                    .bg(rgba(0xFFFFFF30)),
            );
        }
        lines.push(
            div()
                .absolute()
                .left(px(MAGNIFIER_SIZE as f32 / 2.0))
                .top(px(0.0))
                .w(px(1.0))
                .h(px(MAGNIFIER_SIZE as f32))
                .bg(rgba(0xFFFFFFAA)),
        );
        lines.push(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(MAGNIFIER_SIZE as f32 / 2.0))
                .w(px(MAGNIFIER_SIZE as f32))
                .h(px(1.0))
                .bg(rgba(0xFFFFFFAA)),
        );

        let color_row = color.map(|(red, green, blue)| {
            let hex = format!("#{red:02x}{green:02x}{blue:02x}");
            let rgb_text = format!("{red}, {green}, {blue}");
            let hex_active = self.color_format == ColorFormat::Hex;
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .w(px(10.0))
                        .h(px(10.0))
                        .rounded(px(3.0))
                        .bg(rgb((u32::from(red) << 16)
                            | (u32::from(green) << 8)
                            | u32::from(blue))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(if hex_active {
                                    rgb(0xFFFFFF)
                                } else {
                                    rgba(0xFFFFFF88)
                                })
                                .child(hex),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(if hex_active {
                                    rgba(0xFFFFFF88)
                                } else {
                                    rgb(0xFFFFFF)
                                })
                                .child(format!("rgb({rgb_text})")),
                        ),
                )
        });

        let size_row = selection.map(|selection| {
            let width = (f32::from(selection.size.width) * self.scale).round() as u32;
            let height = (f32::from(selection.size.height) * self.scale).round() as u32;
            div()
                .text_size(px(10.0))
                .text_color(rgba(0xFFFFFF99))
                .child(format!("{width} x {height}"))
        });

        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(MAGNIFIER_PANEL))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .bg(rgba(0x000000E6))
                .border_1()
                .border_color(rgba(0xFFFFFF33))
                .child(
                    div()
                        .relative()
                        .w(px(MAGNIFIER_SIZE as f32))
                        .h(px(MAGNIFIER_SIZE as f32))
                        .rounded(px(6.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgba(0xFFFFFF44))
                        .child(
                            img(magnifier.image.clone())
                                .absolute()
                                .inset_0()
                                .size_full()
                                .object_fit(ObjectFit::Fill),
                        )
                        .children(lines),
                )
                .children(color_row)
                .children(size_row)
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(rgba(0xFFFFFF77))
                        .child(t(language, "overlay.magnifier_hint")),
                ),
        )
    }

    fn finish(&self, outcome: SelectionOutcome, cx: &mut Context<Self>) {
        let region = match &outcome {
            SelectionOutcome::Region(region) => Some(*region),
            _ => None,
        };
        if matches!(self.mode, OverlayMode::Scroll | OverlayMode::Ocr) && region.is_none() {
            return;
        }
        let handles = cx.window_stack().unwrap_or_default();
        let target = match outcome {
            SelectionOutcome::Monitor(id) => CaptureTarget::Monitor(id),
            SelectionOutcome::Window(id) => CaptureTarget::Window(id),
            SelectionOutcome::Region(region) => CaptureTarget::Region(region),
        };
        let mode = self.mode;
        let scale = self.scale;

        cx.defer(move |cx| {
            crate::pin::close_windows_except_pins(cx, handles);
            // Give the compositor a moment to remove the overlays before
            // capturing the monitor or region underneath them.
            std::thread::sleep(Duration::from_millis(50));

            let mut backend = platform_backend();
            if mode == OverlayMode::Record {
                crate::record::start(target, cx);
                return;
            }
            if mode == OverlayMode::Scroll || mode == OverlayMode::Ocr {
                let Some(region) = region else {
                    return;
                };
                match backend.capture(CaptureTarget::Region(region)) {
                    Ok(frame) if mode == OverlayMode::Scroll => {
                        crate::scroll::open_session(region, frame, cx);
                    }
                    Ok(frame) => crate::ocr::start(frame, cx),
                    Err(error) => eprintln!("capin: capture failed: {error}"),
                }
                return;
            }
            let captured = if settings_state::get().capture_cursor {
                backend.capture_with_cursor(target)
            } else {
                backend.capture(target)
            };
            match captured {
                Ok(frame) => {
                    let frame = match target {
                        CaptureTarget::Window(id) => {
                            crate::window_style::compose_window_capture(id, frame, scale)
                        }
                        _ => frame,
                    };
                    crate::editor::open_editor(frame, cx);
                }
                Err(error) => eprintln!("capin: capture failed: {error}"),
            }
        });
    }

    fn close_overlays(&self, cx: &mut Context<Self>) {
        let handles = cx.window_stack().unwrap_or_default();
        cx.defer(move |cx| {
            crate::pin::close_windows_except_pins(cx, handles);
        });
    }

    fn open_settings(&self, cx: &mut Context<Self>) {
        let handles = cx.window_stack().unwrap_or_default();
        cx.defer(move |cx| {
            crate::pin::close_windows_except_pins(cx, handles);
            crate::settings::open_settings(cx);
        });
    }
}

impl Render for OverlayView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_magnifier();
        let language = settings_state::get().language;
        let selection = self
            .session
            .selection_rect()
            .and_then(|rect| self.local_bounds(rect));
        let hovered = self
            .session
            .hovered_window()
            .and_then(|window| self.local_bounds(window.bounds));
        let hole = selection.or(hovered);

        let hint_key = match self.mode {
            OverlayMode::Capture => None,
            OverlayMode::Scroll => Some("overlay.scroll_hint"),
            OverlayMode::Ocr => Some("overlay.ocr_hint"),
            OverlayMode::Record => Some("overlay.record_hint"),
        };
        let mode_hint = hint_key.map(|key| {
            div()
                .absolute()
                .top(px(16.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(7.0))
                        .rounded(px(8.0))
                        .bg(rgba(0x000000CC))
                        .text_size(px(12.5))
                        .text_color(rgb(0xFFFFFF))
                        .child(t(language, key)),
                )
        });
        let ratio_bar = (self.mode == OverlayMode::Capture).then(|| self.ratio_bar(language, cx));
        let magnifier = self.magnifier_panel(selection, language);
        let capture_hint = (self.mode == OverlayMode::Capture).then(|| {
            div()
                .absolute()
                .bottom(px(16.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(rgba(0x000000B8))
                        .text_size(px(11.0))
                        .text_color(rgba(0xFFFFFFCC))
                        .child(t(language, "overlay.capture_hint")),
                )
        });
        let copied = self.copied.then(|| {
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
                        .rounded(px(8.0))
                        .bg(rgba(0x000000CC))
                        .text_size(px(11.0))
                        .text_color(rgb(0xFFFFFF))
                        .child(t(language, "overlay.color_copied")),
                )
        });

        let mut layers = self.dim_layers(hole);
        if let Some(rect) = selection {
            layers.push(border(rect, rgb(REGION_BORDER)));
        } else if let Some(rect) = hovered {
            layers.push(border(rect, self.accent));
        }

        if let Some(window) = self.session.hovered_window()
            && let Some(rect) = self.local_bounds(window.bounds)
        {
            layers.push(self.window_label(window, rect));
        }

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(
                |this, event: &ModifiersChangedEvent, _window, cx| {
                    this.set_window_targeting(event.modifiers.alt);
                    let shift = event.modifiers.shift;
                    if shift
                        && !this.shift_held
                        && this.cursor.is_some()
                        && this.mode == OverlayMode::Capture
                    {
                        this.toggle_color_format(cx);
                    }
                    this.shift_held = shift;
                    cx.notify();
                },
            ))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key == "escape" {
                    this.session.cancel();
                    this.close_overlays(cx);
                    return;
                }
                if this.mode == OverlayMode::Capture {
                    if event.keystroke.key.eq_ignore_ascii_case("r") {
                        this.cycle_aspect(cx);
                    } else if event.keystroke.key.eq_ignore_ascii_case("c") {
                        this.copy_color(cx);
                    }
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                this.set_window_targeting(event.modifiers.alt);
                let point = this.to_physical(event.position);
                this.cursor = Some(point);
                this.session.pointer_moved(point);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.set_window_targeting(event.modifiers.alt);
                    let point = this.to_physical(event.position);
                    this.cursor = Some(point);
                    this.session.pointer_pressed(point);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    if this.suppress_capture {
                        this.suppress_capture = false;
                        this.session.cancel();
                        cx.notify();
                        return;
                    }
                    let point = this.to_physical(event.position);
                    this.cursor = Some(point);
                    let action = this.session.pointer_released(point);
                    if let SelectionAction::Capture(outcome) = action {
                        this.finish(outcome, cx);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, _window, cx| {
                this.cursor = None;
                this.session.pointer_left();
                cx.notify();
            }))
            .child(
                img(self.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .children(layers)
            .children(ratio_bar)
            .children(magnifier)
            .children(mode_hint)
            .children(capture_hint)
            .children(copied)
            .child(
                div()
                    .absolute()
                    .top(px(16.0))
                    .right(px(16.0))
                    .w(px(36.0))
                    .h(px(36.0))
                    .rounded(px(10.0))
                    .bg(rgba(0x00000099))
                    .flex()
                    .items_center()
                    .justify_center()
                    .id("overlay-settings")
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.suppress_capture = true;
                            cx.stop_propagation();
                        }),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx)))
                    .child(icons::icon(icons::SETTINGS, 18.0, rgb(0xffffff))),
            )
    }
}

fn dim_rect(left: f32, top: f32, width: f32, height: f32) -> Div {
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width.max(0.0)))
        .h(px(height.max(0.0)))
        .bg(rgba(DIM))
}

fn border(rect: Bounds<Pixels>, color: Rgba) -> Div {
    div()
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .w(rect.size.width)
        .h(rect.size.height)
        .border_2()
        .border_color(color)
}
