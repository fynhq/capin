use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_capture::{
    CaptureTarget, MonitorInfo, SelectionAction, SelectionOutcome, SelectionSession, WindowInfo,
};
use core_types::{
    CaptureRegion, MonitorId, PhysicalPoint, PhysicalRect, PhysicalSize, RgbaFrame, WindowId,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style, system_accent_hex};

use crate::{
    capture_history::HistoryPinShadow,
    chrome_theme::ChromeTheme,
    editor::{EditorView, FullCanvasInit, InlineCapture, Tool},
    effects::RegionEffects,
    i18n::{Language, t},
    icons, image_util,
    logging::{log_error, log_info, log_warn},
    overlay_ui::{
        border, camera_cursor, clamp_to_monitor, element_border, fit_ratio_rect,
        fit_ratio_to_bounds, screen_border,
    },
    selection_geometry::{
        MIN_REGION_SIZE, ResizeHandle, corner_handle_offset, edge_resize_handle, handle_positions,
        region_handle_cursor, resize_edge, resize_rect,
    },
    settings_state::{self, DoubleClickAction},
    ui::tooltip_view,
};

mod actions;
mod capture;
mod edit;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod element_cache;
mod element_probe;
mod element_scroll;
mod hints_window;
mod panels;
mod query;
mod refresh;
mod registry;
mod render;

use element_probe::ElementProbe;

const DIM: u32 = 0x00000066;
const ELEMENT_HOVER: u32 = 0xffb020;
const SCREEN_SELECTION: u32 = 0x34c759;
const MAGNIFIER_SIZE: u32 = 144;
const MAGNIFIER_SOURCE: u32 = 9;
const MAGNIFIER_PANEL: f32 = 160.0;
const MAGNIFIER_PANEL_HEIGHT: f32 = 228.0;
const INFO_BAR_HEIGHT: f32 = 30.0;
const INFO_BAR_WIDTH: f32 = 440.0;
const SELECTION_TOOLBAR_HEIGHT: f32 = 40.0;
const SELECTION_TOOLBAR_WIDTH: f32 = 560.0;
const HANDLE_SIZE: f32 = 9.0;
const REGION_GAP: f32 = 8.0;

const OVERLAY_KEY_CONTEXT: &str = "Overlay";

gpui_kit::actions!([WidenTarget, NarrowTarget]);

thread_local! {
    static RETURN_APP_PID: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", WidenTarget, Some(OVERLAY_KEY_CONTEXT)),
        KeyBinding::new("shift-tab", NarrowTarget, Some(OVERLAY_KEY_CONTEXT)),
    ]);
}

pub fn has_open_overlays() -> bool {
    registry::has_live()
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn close_open_overlays(cx: &mut App) {
    let handles = cx.windows();
    log_info!("overlay", "closing overlays ({} windows)", handles.len());
    platform_window::set_cursor_hidden(false);
    crate::windows::close_capture_auxiliary(cx, handles);
    registry::clear();
}

pub(crate) fn remember_frontmost_application() {
    if let Some(pid) = platform_window::frontmost_other_application_pid() {
        RETURN_APP_PID.with(|slot| slot.set(Some(pid)));
    }
}

fn control_down(modifiers: &Modifiers) -> bool {
    modifiers.control || platform_window::control_modifier_down()
}

pub(crate) fn restore_previous_application(cx: &mut App) {
    let Some(pid) = RETURN_APP_PID.with(Cell::get) else {
        return;
    };
    if crate::windows::has_open_settings(cx) {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(60))
            .await;
        cx.update(|cx| {
            if has_open_overlays()
                || crate::windows::has_open_settings(cx)
                || crate::editor::has_open_editors()
            {
                return;
            }
            log_info!("overlay", "restoring focus to application {pid}");
            platform_window::activate_application_by_pid(pid);
        });
    })
    .detach();
}

fn live_overlay_window_ids(cx: &App) -> Vec<WindowId> {
    live_overlay_window_ids_excluding(cx, None)
}

fn live_overlay_window_ids_excluding(cx: &App, exclude: Option<EntityId>) -> Vec<WindowId> {
    registry::live()
        .iter()
        .filter(|view| exclude != Some(view.entity_id()))
        .filter_map(WeakEntity::upgrade)
        .filter_map(|view| view.read(cx).native_id)
        .map(WindowId::from_raw)
        .collect()
}

pub(crate) fn release_textures(cx: &mut App) {
    release_texture_views(cx, None);
}

pub(crate) fn release_textures_except(cx: &mut App, keep: u64) {
    release_texture_views(cx, Some(keep));
}

fn release_texture_views(cx: &mut App, keep: Option<u64>) {
    let views: Vec<Entity<OverlayView>> = registry::live()
        .into_iter()
        .filter_map(|view| view.upgrade())
        .collect();
    for view in views {
        if keep == Some(view.entity_id().as_u64()) {
            continue;
        }
        view.update(cx, |this, cx| this.release_capture_resources(cx));
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn pin_current(cx: &mut App) -> bool {
    for weak in registry::focused() {
        let Some(view) = weak.upgrade() else {
            continue;
        };
        let handled = view.update(cx, |this, cx| {
            if let Some(editor) = this.editor.clone() {
                editor.update(cx, |editor, cx| editor.pin(cx));
                return true;
            }
            if this.selected.is_some() {
                this.pin_selected(cx);
                return true;
            }
            false
        });
        if handled {
            return true;
        }
    }
    false
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SizeUnit {
    Px,
    Dp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InfoField {
    X,
    Y,
    Width,
    Height,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegionDragKind {
    Move,
    Resize(ResizeHandle),
}

#[derive(Clone, Copy, Debug)]
struct RegionDrag {
    kind: RegionDragKind,
    start: PhysicalPoint,
    original: PhysicalRect,
}

#[derive(Clone, Debug)]
struct FieldEdit {
    field: InfoField,
    buffer: String,
    fresh: bool,
}

#[derive(Clone, Debug)]
struct Selected {
    target: CaptureTarget,
    rect: PhysicalRect,
    scale: f32,
    ratio: AspectPreset,
    unit: SizeUnit,
    drag: Option<RegionDrag>,
    edit: Option<FieldEdit>,
    effects_open: bool,
    hints_open: bool,
    space_down: bool,
}

impl ColorFormat {
    fn next(self) -> Self {
        match self {
            Self::Hex => Self::Rgb,
            Self::Rgb => Self::Hex,
        }
    }
}

impl SizeUnit {
    fn next(self) -> Self {
        match self {
            Self::Px => Self::Dp,
            Self::Dp => Self::Px,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Px => "px",
            Self::Dp => "dp",
        }
    }
}

impl InfoField {
    fn next(self) -> Self {
        match self {
            Self::X => Self::Y,
            Self::Y => Self::Width,
            Self::Width => Self::Height,
            Self::Height => Self::X,
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
}

pub fn open_overlays(cx: &mut App) {
    if has_open_overlays() {
        return;
    }
    remember_frontmost_application();
    if !crate::windows::has_open_settings(cx) {
        platform_window::set_macos_accessory_activation_policy();
    }
    #[cfg(target_os = "windows")]
    platform_capture::init_process_dpi_awareness();
    platform_window::set_cursor_hidden(false);

    let mut backend = platform_capture::platform_backend();
    let mut monitors = match backend.monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            log_error!("overlay", "failed to enumerate monitors: {error}");
            return;
        }
    };
    monitors.sort_by_key(|monitor| !monitor.is_primary);
    let excluded = live_overlay_window_ids(cx);
    let windows = match backend.windows_excluding(&excluded) {
        Ok(windows) => windows,
        Err(error) => {
            log_error!("overlay", "failed to enumerate windows: {error}");
            return;
        }
    };

    let accent_hex = system_accent_hex().unwrap_or_else(|| crate::theme::fallback_accent(false));
    let accent = rgb(accent_hex);

    let mut unique_monitors: Vec<MonitorInfo> = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        if let Some(kept) = unique_monitors
            .iter()
            .find(|kept| kept.bounds == monitor.bounds)
        {
            log_warn!(
                "overlay",
                "monitor {} shares bounds {:?} with {}; treating it as a clone",
                monitor.id.raw(),
                monitor.bounds,
                kept.id.raw()
            );
            continue;
        }
        log_info!(
            "overlay",
            "monitor {}: {:?} scale {:.2} primary {}",
            monitor.id.raw(),
            monitor.bounds,
            monitor.scale.get(),
            monitor.is_primary
        );
        unique_monitors.push(monitor);
    }

    let capture_cursor = settings_state::get().capture_cursor;
    let results = capture::capture_monitors(unique_monitors, capture_cursor);

    let mut captures: Vec<(
        MonitorInfo,
        Arc<RenderImage>,
        PhysicalSize,
        Option<capture::CaptureStream>,
    )> = Vec::with_capacity(results.len());
    for result in results {
        let monitor = result.monitor;
        let frame = match result.frame {
            Ok(frame) => frame,
            Err(error) => {
                log_error!(
                    "overlay",
                    "failed to capture monitor {:?}: {error}",
                    monitor.id
                );
                #[cfg(target_os = "macos")]
                if matches!(error, core_capture::CaptureError::PermissionDenied) {
                    crate::settings::report_capture_permission_denied(cx);
                    return;
                }
                continue;
            }
        };
        log_info!(
            "overlay",
            "captured monitor {}: {}x{}",
            monitor.id.raw(),
            frame.width(),
            frame.height()
        );
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        captures.push((
            monitor,
            image_util::render_image(frame),
            image_size,
            result.stream,
        ));
    }

    let mut opened = 0usize;
    for (monitor, image, image_size, monitor_stream) in captures {
        let window_bounds = logical_bounds(&monitor);
        let expected_size = window_bounds.size;
        let expected_scale = monitor.scale.get();
        let options = overlay_options(monitor.id, window_bounds);
        let placement = monitor.bounds;
        let placement_scale = monitor.scale;
        let session_monitor = monitor.id;
        let session_bounds = monitor.bounds;
        let session_windows = windows.clone();
        let monitor_id = monitor.id.raw();

        cx.activate(true);
        match gpui_kit::open_window(options, cx, move |window, cx| {
            if let Err(error) = apply_window_style(&*window, WindowStyle::OVERLAY) {
                log_error!("overlay", "failed to style overlay: {error}");
            }
            if let Err(error) =
                platform_window::place_on_monitor(&*window, placement, placement_scale)
            {
                log_error!(
                    "overlay",
                    "failed to place monitor {monitor_id} overlay: {error}"
                );
            }
            let scale = window.scale_factor();
            let actual_size = window.bounds().size;
            let window_size =
                if f32::from(actual_size.width) > 0.0 && f32::from(actual_size.height) > 0.0 {
                    actual_size
                } else {
                    expected_size
                };
            let actual_monitor = platform_window::monitor_for_window(&*window);
            log_info!(
                "overlay",
                "monitor {monitor_id} overlay window: display {:?} size {:?} scale {:.2} (expected {:?} @ {:.2})",
                actual_monitor,
                window_size,
                scale,
                expected_size,
                expected_scale
            );
            if let Some(actual) = actual_monitor
                && actual != monitor_id
            {
                log_error!(
                    "overlay",
                    "monitor {monitor_id} overlay landed on display {actual}"
                );
            }
            if (scale - expected_scale).abs() > 0.01
                || (f32::from(window_size.width) - f32::from(expected_size.width)).abs() > 1.0
                || (f32::from(window_size.height) - f32::from(expected_size.height)).abs() > 1.0
            {
                log_warn!(
                    "overlay",
                    "monitor {monitor_id} overlay geometry differs from the monitor; selections may be clipped"
                );
            }
            let session = SelectionSession::with_scale(
                session_monitor,
                session_bounds,
                session_windows,
                scale,
            );
            let native_id = platform_window::native_window_id(&*window);
            let view = cx.new(|cx| {
                OverlayView::new(
                    session,
                    native_id,
                    image,
                    image_size,
                    scale,
                    window_size,
                    accent,
                    accent_hex,
                    cx,
                )
            });
            #[cfg(target_os = "windows")]
            if let Some(stream) = monitor_stream {
                view.update(cx, |this, _| this.monitor_stream = Some(stream));
            }
            #[cfg(not(target_os = "windows"))]
            let _ = monitor_stream;
            let entity_id = view.entity_id().as_u64();
            view.update(cx, |this, _| this.entity_id = entity_id);
            let focus = view.read(cx).focus_handle.clone();
            view.update(cx, |this, cx| this.observe_appearance(window, cx));
            view.update(cx, |view, cx| view.watch_close(window, cx));
            registry::register(view.downgrade());
            let geometry = view.clone();
            window.defer(cx, move |window, cx| {
                if let Err(error) =
                    platform_window::place_on_monitor(&*window, placement, placement_scale)
                {
                    log_error!("overlay", "failed to re-place overlay: {error}");
                }
                if let Some(actual) = platform_window::monitor_for_window(&*window)
                    && actual != monitor_id
                {
                    log_error!(
                        "overlay",
                        "monitor {monitor_id} overlay still on display {actual} after re-placement"
                    );
                }
                geometry.update(cx, |this, cx| this.refresh_geometry(window, cx));
                window.focus(&focus, cx);
                window.activate_window();
                cx.activate(true);
            });
            view
        }) {
            Ok(_) => {
                opened += 1;
            }
            Err(error) => log_error!("overlay", "failed to open overlay: {error}"),
        }
    }

    if opened == 0 {
        log_error!("overlay", "no overlay windows could be opened");
    } else {
        log_info!("overlay", "opened {opened} overlay window(s)");
    }
}

fn overlay_options(monitor: MonitorId, bounds: Bounds<Pixels>) -> WindowOptions {
    crate::aux_window::popup_options(
        "capin-overlay",
        bounds,
        Some(DisplayId::new(monitor.raw())),
        true,
        overlay_window_kind(),
        WindowBackgroundAppearance::Opaque,
    )
}

fn overlay_window_kind() -> WindowKind {
    #[cfg(target_os = "macos")]
    {
        WindowKind::Normal
    }
    #[cfg(not(target_os = "macos"))]
    {
        WindowKind::PopUp
    }
}

fn logical_bounds(monitor: &MonitorInfo) -> Bounds<Pixels> {
    let scale = monitor.scale.get();
    let bounds = Bounds {
        origin: point(
            px(monitor.bounds.origin.x as f32 / scale),
            px(monitor.bounds.origin.y as f32 / scale),
        ),
        size: size(
            px(monitor.bounds.size.width as f32 / scale),
            px(monitor.bounds.size.height as f32 / scale),
        ),
    };
    crate::display::window_bounds_on_monitor(monitor, bounds)
}

struct Magnifier {
    point: PhysicalPoint,
    image: Arc<RenderImage>,
}

struct OverlayView {
    session: SelectionSession,
    entity_id: u64,
    native_id: Option<u64>,
    image: Arc<RenderImage>,
    image_size: PhysicalSize,
    scale: f32,
    window_size: Size<Pixels>,
    cursor: Option<PhysicalPoint>,
    accent: Rgba,
    accent_hex: u32,
    chrome: ChromeTheme,
    appearance_subscription: Option<Subscription>,
    aspect: AspectPreset,
    color_format: ColorFormat,
    shift_held: bool,
    window_only: bool,
    click_opens_editor: bool,
    ctrl_click_primary: bool,
    cursor_style: CursorStyle,
    magnifier: Option<Magnifier>,
    #[cfg(target_os = "windows")]
    monitor_stream: Option<platform_capture::MonitorStream>,
    last_refresh: Option<Instant>,
    refresh_in_flight: bool,
    notice: Option<String>,
    notice_serial: u64,
    notice_leaving: bool,
    element_probe: ElementProbe,
    screen_targeting: bool,
    focus_handle: FocusHandle,
    editor: Option<Entity<EditorView>>,
    editing_rect: Option<Bounds<Pixels>>,
    selected: Option<Selected>,
    effects: RegionEffects,
}

impl OverlayView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        mut session: SelectionSession,
        native_id: Option<u64>,
        image: Arc<RenderImage>,
        image_size: PhysicalSize,
        scale: f32,
        window_size: Size<Pixels>,
        accent: Rgba,
        accent_hex: u32,
        cx: &mut Context<Self>,
    ) -> Self {
        session.set_window_targeting(true);
        cx.on_release(|this, cx| {
            cx.drop_image(this.image.clone(), None);
            if let Some(magnifier) = this.magnifier.take() {
                cx.drop_image(magnifier.image, None);
            }
        })
        .detach();
        let settings = settings_state::get();
        Self {
            session,
            entity_id: 0,
            native_id,
            image,
            image_size,
            scale,
            window_size,
            cursor: None,
            accent,
            accent_hex,
            chrome: ChromeTheme::for_appearance(false, Some(accent_hex)),
            appearance_subscription: None,
            aspect: AspectPreset::Free,
            color_format: ColorFormat::Hex,
            shift_held: false,
            window_only: platform_window::control_modifier_down(),
            click_opens_editor: false,
            ctrl_click_primary: false,
            cursor_style: CursorStyle::Crosshair,
            magnifier: None,
            #[cfg(target_os = "windows")]
            monitor_stream: None,
            last_refresh: None,
            refresh_in_flight: false,
            notice: None,
            notice_serial: 0,
            notice_leaving: false,
            element_probe: ElementProbe::default(),
            screen_targeting: false,
            focus_handle: cx.focus_handle(),
            editor: None,
            editing_rect: None,
            selected: None,
            effects: RegionEffects {
                corner_radius: settings.region_corner_radius,
                shadow: settings.region_shadow,
            },
        }
    }

    fn observe_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance_subscription =
            Some(cx.observe_window_appearance(window, |this, window, cx| {
                this.chrome = ChromeTheme::for_window(window, Some(this.accent_hex));
                cx.notify();
            }));
    }

    fn frame_ratio(&self) -> (f32, f32) {
        let frame_width = self.image_size.width;
        let frame_height = self.image_size.height;
        let window_width = f32::from(self.window_size.width).max(1.0);
        let window_height = f32::from(self.window_size.height).max(1.0);
        (
            frame_width as f32 / window_width,
            frame_height as f32 / window_height,
        )
    }

    fn effective_scale(&self) -> f32 {
        self.frame_ratio().0.max(0.1)
    }

    fn to_physical(&self, position: Point<Pixels>) -> PhysicalPoint {
        let origin = self.session.monitor_bounds().origin;
        let (scale_x, scale_y) = self.frame_ratio();
        PhysicalPoint::new(
            origin.x + (f32::from(position.x) * scale_x).round() as i32,
            origin.y + (f32::from(position.y) * scale_y).round() as i32,
        )
    }

    fn local_rect(&self, rect: PhysicalRect) -> Option<Bounds<Pixels>> {
        let monitor = self.session.monitor_bounds();
        let local = PhysicalRect::new(0, 0, monitor.size.width, monitor.size.height);
        let clipped = rect.intersection(local)?;
        let (scale_x, scale_y) = self.frame_ratio();
        Some(Bounds {
            origin: point(
                px(clipped.origin.x as f32 / scale_x),
                px(clipped.origin.y as f32 / scale_y),
            ),
            size: size(
                px(clipped.size.width as f32 / scale_x),
                px(clipped.size.height as f32 / scale_y),
            ),
        })
    }

    fn local_bounds(&self, bounds: PhysicalRect) -> Option<Bounds<Pixels>> {
        let monitor = self.session.monitor_bounds();
        let clipped = bounds.intersection(monitor)?;
        let (scale_x, scale_y) = self.frame_ratio();
        Some(Bounds {
            origin: point(
                px((clipped.origin.x - monitor.origin.x) as f32 / scale_x),
                px((clipped.origin.y - monitor.origin.y) as f32 / scale_y),
            ),
            size: size(
                px(clipped.size.width as f32 / scale_x),
                px(clipped.size.height as f32 / scale_y),
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

    fn selection_effects_radius(&self) -> f32 {
        let Some(selected) = &self.selected else {
            return 0.0;
        };
        if matches!(selected.target, CaptureTarget::Window(_)) {
            return 0.0;
        }
        self.effects.corner_radius as f32
    }

    fn dim_layers_rounded(&self, hole: Option<Bounds<Pixels>>, radius: f32) -> Vec<AnyElement> {
        if radius <= 0.5 {
            return self
                .dim_layers(hole)
                .into_iter()
                .map(|layer| layer.into_any_element())
                .collect();
        }
        let Some(hole) = hole else {
            return self
                .dim_layers(None)
                .into_iter()
                .map(|layer| layer.into_any_element())
                .collect();
        };
        let accent = self.accent;
        let layer = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                crate::ui::paint_rounded_mask(
                    window,
                    bounds,
                    hole,
                    radius,
                    rgba(DIM),
                    Some((accent, 3.0)),
                );
            },
        )
        .absolute()
        .inset_0()
        .size_full();
        vec![layer.into_any_element()]
    }

    fn release_capture_resources(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "windows")]
        {
            if let Some(stream) = self.monitor_stream.take() {
                platform_capture::cache_monitor_stream(self.session.monitor(), stream);
            }
        }
        if let Some(magnifier) = self.magnifier.take() {
            cx.drop_image(magnifier.image, None);
        }
        cx.drop_image(self.image.clone(), None);
        self.image = crate::image_util::transparent_image();
        if let Some(editor) = self.editor.clone() {
            editor.update(cx, |editor, cx| editor.release_images(cx));
        }
    }

    fn close_overlays(&mut self, cx: &mut Context<Self>) {
        platform_window::set_cursor_hidden(false);
        hints_window::close(cx);
        self.release_capture_resources(cx);
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
        });
    }
}

impl Drop for OverlayView {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        if let Some(stream) = self.monitor_stream.take() {
            platform_capture::cache_monitor_stream(self.session.monitor(), stream);
        }
        registry::clear_focus_if(self.entity_id);
    }
}

impl Render for OverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_inner(window, cx)
    }
}

fn history_pin_shadow(view: &OverlayView) -> Option<HistoryPinShadow> {
    let selected = view.selected.as_ref()?;
    if !matches!(selected.target, CaptureTarget::Region(_)) || view.effects.shadow {
        return None;
    }
    Some(HistoryPinShadow {
        corner_radius: view.effects.corner_radius,
        scale: view.effective_scale(),
    })
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

#[cfg(test)]
mod tests {
    use super::capture::frame_matches_monitor;
    use crate::overlay_ui::fit_ratio_to_bounds;
    use core_capture::MonitorInfo;
    use core_types::{DpiScale, MonitorId, PhysicalRect, RgbaFrame};

    fn monitor(width: u32, height: u32) -> MonitorInfo {
        MonitorInfo {
            id: MonitorId::from_raw(1),
            bounds: PhysicalRect::new(0, 0, width, height),
            scale: DpiScale::new(1.0).unwrap(),
            is_primary: true,
        }
    }

    fn frame(width: u32, height: u32) -> RgbaFrame {
        RgbaFrame::from_pixels(width, height, vec![0; (width * height * 4) as usize]).unwrap()
    }

    #[test]
    fn frame_size_must_match_the_monitor() {
        let target = monitor(1920, 1080);
        assert!(frame_matches_monitor(&frame(1920, 1080), target.bounds));
        assert!(frame_matches_monitor(&frame(1921, 1079), target.bounds));
        assert!(!frame_matches_monitor(&frame(1920, 1200), target.bounds));
        assert!(!frame_matches_monitor(&frame(2560, 1440), target.bounds));
    }

    #[test]
    fn ratio_edit_stays_inside_the_monitor() {
        assert_eq!(
            fit_ratio_to_bounds(1000, 16.0 / 9.0, 1920, 1080),
            (1000, 563)
        );
        let (width, height) = fit_ratio_to_bounds(3000, 16.0 / 9.0, 1920, 1080);
        assert!(width <= 1920);
        assert!(height <= 1080);
        assert!((width as f32 / height as f32 - 16.0 / 9.0).abs() < 0.01);
    }
}
