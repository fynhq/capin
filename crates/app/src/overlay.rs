use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_capture::{
    CaptureBackend, CaptureError, CaptureTarget, MonitorInfo, SelectionAction, SelectionOutcome,
    SelectionSession, WindowInfo,
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
        region_handle_cursor, resize_rect,
    },
    settings_state,
    ui::tooltip_view,
};

mod refresh;
mod registry;

const DIM: u32 = 0x00000066;
const FALLBACK_HOVER_ACCENT: u32 = 0x4c9aff;
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

thread_local! {
    static RETURN_APP_PID: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn has_open_overlays() -> bool {
    registry::has_live()
}

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
        view.update(cx, |this, cx| {
            cx.drop_image(this.image.clone(), None);
            if let Some(magnifier) = this.magnifier.take() {
                cx.drop_image(magnifier.image, None);
            }
            if let Some(editor) = this.editor.clone() {
                editor.update(cx, |editor, cx| editor.release_images(cx));
            }
        });
    }
}

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

    let accent_hex = system_accent_hex().unwrap_or(FALLBACK_HOVER_ACCENT);
    let accent = rgb(accent_hex);

    let mut captures: Vec<(MonitorInfo, Arc<RenderImage>, PhysicalSize)> =
        Vec::with_capacity(monitors.len());
    let capture_cursor = settings_state::get().capture_cursor;
    for monitor in monitors {
        if let Some((kept, _, _)) = captures
            .iter()
            .find(|(kept, _, _)| kept.bounds == monitor.bounds)
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
        let mut frame = match capture_monitor(&mut *backend, monitor.id, capture_cursor) {
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
        if !frame_matches_monitor(&frame, &monitor) {
            log_error!(
                "overlay",
                "monitor {} frame {}x{} does not match {:?}; retrying",
                monitor.id.raw(),
                frame.width(),
                frame.height(),
                monitor.bounds
            );
            frame = match capture_monitor(&mut *backend, monitor.id, capture_cursor) {
                Ok(frame) => frame,
                Err(error) => {
                    log_error!(
                        "overlay",
                        "retry failed for monitor {:?}: {error}",
                        monitor.id
                    );
                    continue;
                }
            };
        }
        if !frame_matches_monitor(&frame, &monitor) {
            log_error!(
                "overlay",
                "monitor {} kept returning {}x{} instead of {:?}; skipping",
                monitor.id.raw(),
                frame.width(),
                frame.height(),
                monitor.bounds
            );
            continue;
        }
        log_info!(
            "overlay",
            "captured monitor {}: {}x{}",
            monitor.id.raw(),
            frame.width(),
            frame.height()
        );
        let image_size = PhysicalSize::new(frame.width(), frame.height());
        captures.push((monitor, image_util::render_image(frame), image_size));
    }

    let mut opened = 0usize;
    for (monitor, image, image_size) in captures {
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

fn capture_monitor(
    backend: &mut dyn CaptureBackend,
    id: MonitorId,
    with_cursor: bool,
) -> Result<RgbaFrame, CaptureError> {
    let target = CaptureTarget::Monitor(id);
    if with_cursor {
        backend.capture_with_cursor(target)
    } else {
        backend.capture(target)
    }
}

fn frame_matches_monitor(frame: &RgbaFrame, monitor: &MonitorInfo) -> bool {
    frame.width().abs_diff(monitor.bounds.size.width) <= 1
        && frame.height().abs_diff(monitor.bounds.size.height) <= 1
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
    notice: Option<String>,
    notice_serial: u64,
    notice_leaving: bool,
    element_cache: HashMap<WindowId, Vec<PhysicalRect>>,
    element_cache_point: Option<PhysicalPoint>,
    element_retry: HashSet<WindowId>,
    element_window: Option<WindowId>,
    element_pending: Option<WindowId>,
    element_pending_point: Option<PhysicalPoint>,
    element_candidates: Vec<PhysicalRect>,
    element_level: usize,
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
            notice: None,
            notice_serial: 0,
            notice_leaving: false,
            element_cache: HashMap::new(),
            element_cache_point: None,
            element_retry: HashSet::new(),
            element_window: None,
            element_pending: None,
            element_pending_point: None,
            element_candidates: Vec::new(),
            element_level: 0,
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

    fn sample_color(&self, point: PhysicalPoint) -> Option<(u8, u8, u8)> {
        let monitor = self.session.monitor_bounds();
        let local_x = point.x - monitor.origin.x;
        let local_y = point.y - monitor.origin.y;
        if local_x < 0 || local_y < 0 {
            return None;
        }
        image_util::image_pixel(&self.image, local_x as u32, local_y as u32)
            .map(|[red, green, blue, _]| (red, green, blue))
    }

    fn refresh_auto_target(&mut self, point: PhysicalPoint, cx: &mut Context<Self>) {
        if self.session.is_dragging() || self.selected.is_some() {
            return;
        }
        let Some(window) = self.session.window_under_pointer().cloned() else {
            self.element_window = None;
            self.element_pending = None;
            self.element_pending_point = None;
            self.element_candidates.clear();
            self.element_level = 0;
            self.screen_targeting = false;
            self.session.set_screen_targeting(false);
            self.session.set_hovered_element(None);
            return;
        };
        if self.window_only {
            self.element_window = None;
            self.element_pending = None;
            self.element_pending_point = None;
            self.element_candidates.clear();
            self.element_level = 0;
            self.screen_targeting = false;
            self.session.set_screen_targeting(false);
            self.session.set_hovered_element(None);
            return;
        }
        if self.element_window != Some(window.id) {
            self.element_window = Some(window.id);
            self.element_cache_point = None;
            self.element_level = 0;
            self.element_candidates.clear();
            self.session.set_hovered_element(None);
        }
        let cached_for_point = {
            #[cfg(target_os = "macos")]
            {
                self.element_cache.contains_key(&window.id)
                    && self.element_cache_point == Some(point)
            }
            #[cfg(not(target_os = "macos"))]
            {
                self.element_cache.contains_key(&window.id)
            }
        };
        if !cached_for_point {
            if self.element_pending.is_none() {
                self.query_elements(window.id, point, cx);
            }
            self.session.set_hovered_element(None);
            return;
        }
        let Some(cached) = self.element_cache.get(&window.id) else {
            self.session.set_hovered_element(None);
            return;
        };

        let candidates = core_capture::element_candidates(cached, window.bounds, point);
        if self.element_level > candidates.len() {
            self.element_level = candidates.len();
        }
        self.element_candidates = candidates;
        self.session.set_screen_targeting(self.screen_targeting);
        let selected = (!self.screen_targeting)
            .then(|| self.element_candidates.get(self.element_level).copied())
            .flatten();
        self.session.set_hovered_element(selected);
    }

    fn query_elements(
        &mut self,
        window_id: WindowId,
        point: PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        self.element_pending = Some(window_id);
        self.element_pending_point = Some(point);
        cx.spawn(async move |this, cx| {
            let elements = cx
                .background_executor()
                .spawn(async move { platform_capture::ui_elements_at(window_id, point) })
                .await;
            let empty = elements.is_empty();
            let applied = this
                .update(cx, |this, cx| {
                    if this.element_pending != Some(window_id)
                        || this.element_pending_point != Some(point)
                    {
                        return false;
                    }
                    this.element_pending = None;
                    this.element_pending_point = None;
                    if empty && !platform_window::accessibility_trusted() {
                        return false;
                    }
                    this.element_cache.insert(window_id, elements);
                    #[cfg(target_os = "macos")]
                    {
                        this.element_cache_point = Some(point);
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
                .update(cx, |this, _| this.element_retry.insert(window_id))
                .unwrap_or(false);
            if !retry {
                return;
            }
            let elements = cx
                .background_executor()
                .spawn(async move { platform_capture::ui_elements_at(window_id, point) })
                .await;
            this.update(cx, |this, cx| {
                if this
                    .element_cache
                    .get(&window_id)
                    .is_some_and(Vec::is_empty)
                {
                    this.element_cache.insert(window_id, elements);
                    #[cfg(target_os = "macos")]
                    {
                        this.element_cache_point = Some(point);
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

    fn adjust_auto_target(&mut self, delta: f32) {
        if delta == 0.0 || self.session.has_drag() {
            return;
        }
        if !self.screen_targeting && self.session.window_under_pointer().is_none() {
            return;
        }
        let max = self.element_candidates.len();
        if self.screen_targeting {
            if delta < 0.0 {
                self.screen_targeting = false;
                self.element_level = max;
            }
        } else if delta > 0.0 {
            if self.element_level < max {
                self.element_level += 1;
            } else {
                self.screen_targeting = true;
            }
        } else {
            self.element_level = self.element_level.saturating_sub(1);
        }
        self.session.set_screen_targeting(self.screen_targeting);
        let selected = if self.screen_targeting {
            None
        } else {
            self.element_candidates.get(self.element_level).copied()
        };
        self.session.set_hovered_element(selected);
    }

    fn set_window_only(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let enabled = enabled && self.selected.is_none() && self.editor.is_none();
        if self.window_only == enabled {
            return;
        }
        self.window_only = enabled;
        self.element_window = None;
        self.element_pending = None;
        self.element_pending_point = None;
        self.element_candidates.clear();
        self.element_level = 0;
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

    fn update_magnifier(&mut self, window: &mut Window) {
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

    fn clear_magnifier(&mut self, window: &mut Window) {
        if let Some(magnifier) = self.magnifier.take() {
            let _ = window.drop_image(magnifier.image);
        }
    }

    fn refresh_geometry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn copy_color(&mut self, cx: &mut Context<Self>) {
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
        let text = self.color_text(red, green, blue);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.close_overlays(cx);
    }

    fn cycle_aspect(&mut self, cx: &mut Context<Self>) {
        self.aspect = self.aspect.next();
        self.session.set_aspect_ratio(self.aspect.ratio());
        cx.notify();
    }

    fn window_label(
        &self,
        window: &WindowInfo,
        rect: Bounds<Pixels>,
        element: Option<PhysicalRect>,
    ) -> Div {
        let language = settings_state::get().language;
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
                self.chrome.icon,
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
            .bg(self.chrome.surface)
            .border_1()
            .border_color(if element.is_some() {
                rgba((ELEMENT_HOVER << 8) | 0xCC)
            } else {
                self.chrome.border
            })
            .shadow(self.chrome.shadow())
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
                            .text_color(self.chrome.text)
                            .child(text),
                    ),
            )
    }

    fn screen_label(&self, language: Language) -> Div {
        let width = self.session.monitor_bounds().size.width;
        let height = self.session.monitor_bounds().size.height;
        div()
            .absolute()
            .left(px(16.0))
            .top(px(16.0))
            .px_2()
            .py_1()
            .bg(self.chrome.surface)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .rounded(px(6.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(icons::icon(icons::SCREEN, 12.0, self.chrome.icon))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(self.chrome.text)
                            .child(format!(
                                "{} · {}x{}",
                                t(language, "overlay.screen"),
                                width,
                                height
                            )),
                    ),
            )
    }

    fn magnifier_panel(
        &self,
        selection: Option<Bounds<Pixels>>,
        language: Language,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let magnifier = self.magnifier.as_ref()?;
        let point = self.cursor?;
        let monitor = self.session.monitor_bounds();
        let (scale_x, scale_y) = self.frame_ratio();
        let local_x = (point.x - monitor.origin.x) as f32 / scale_x;
        let local_y = (point.y - monitor.origin.y) as f32 / scale_y;
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
        let step = MAGNIFIER_SIZE as f32 / MAGNIFIER_SOURCE as f32;
        let mut lines: Vec<Div> = Vec::new();
        for index in 1..MAGNIFIER_SOURCE {
            let offset = step * index as f32;
            lines.push(
                div()
                    .absolute()
                    .left(px(offset))
                    .top(px(0.0))
                    .w(px(1.0))
                    .h(px(MAGNIFIER_SIZE as f32))
                    .bg(self.chrome.grid),
            );
            lines.push(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(offset))
                    .w(px(MAGNIFIER_SIZE as f32))
                    .h(px(1.0))
                    .bg(self.chrome.grid),
            );
        }
        let centre = step * (MAGNIFIER_SOURCE as f32 / 2.0).floor();
        lines.push(
            div()
                .absolute()
                .left(px(centre))
                .top(px(centre))
                .w(px(step))
                .h(px(step))
                .border_1()
                .border_color(self.chrome.grid_strong),
        );

        let color_row = color.map(|(red, green, blue)| {
            let text = match self.color_format {
                ColorFormat::Hex => format!("#{red:02x}{green:02x}{blue:02x}"),
                ColorFormat::Rgb => format!("{red}, {green}, {blue}"),
            };
            let format_label = match self.color_format {
                ColorFormat::Hex => "HEX",
                ColorFormat::Rgb => "RGB",
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .w(px(12.0))
                        .h(px(12.0))
                        .rounded(px(3.0))
                        .bg(rgb((u32::from(red) << 16)
                            | (u32::from(green) << 8)
                            | u32::from(blue)))
                        .border_1()
                        .border_color(self.chrome.border),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(self.chrome.text)
                        .child(text),
                )
                .child(
                    div()
                        .px(px(4.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(self.chrome.surface)
                        .border_1()
                        .border_color(self.chrome.border)
                        .text_size(px(9.0))
                        .text_color(self.chrome.muted)
                        .child(format_label),
                )
        });

        let size_row = selection.map(|selection| {
            let (scale_x, scale_y) = self.frame_ratio();
            let width = (f32::from(selection.size.width) * scale_x).round() as u32;
            let height = (f32::from(selection.size.height) * scale_y).round() as u32;
            div()
                .text_size(px(10.0))
                .text_color(self.chrome.muted)
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
                        .relative()
                        .w(px(MAGNIFIER_SIZE as f32))
                        .h(px(MAGNIFIER_SIZE as f32))
                        .rounded(px(6.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(self.chrome.border)
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
                        .text_color(self.chrome.muted)
                        .child(t(language, "overlay.magnifier_hint")),
                ),
        )
    }

    fn begin_selection(
        &mut self,
        outcome: SelectionOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        self.selected
            .as_ref()
            .and_then(|selected| self.local_rect(selected.rect))
    }

    fn selection_region(&self, selected: &Selected) -> CaptureRegion {
        CaptureRegion::new(self.session.monitor(), selected.rect)
    }

    fn capture_selected_raw(&self) -> Result<RgbaFrame, String> {
        let selected = self
            .selected
            .as_ref()
            .ok_or_else(|| "no selection".to_owned())?;
        let frame = match selected.target {
            CaptureTarget::Window(id) => {
                let frozen = {
                    #[cfg(target_os = "macos")]
                    {
                        !settings_state::get().capture_cursor && self.window_is_fully_visible(id)
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        false
                    }
                };
                log_info!("overlay", "window {} frozen capture: {frozen}", id.raw());
                if frozen {
                    crate::window_frame::compose_window_capture(
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
        Ok(frame)
    }

    fn crop_selected_frame(&self, selected: &Selected) -> Result<RgbaFrame, String> {
        image_util::cropped_rgba(&self.image, selected.rect)
            .ok_or_else(|| "empty selection".to_owned())
    }

    #[cfg(target_os = "macos")]
    fn window_is_fully_visible(&self, id: WindowId) -> bool {
        let Some(window) = self.session.window(id) else {
            return false;
        };
        let monitor = self.session.monitor_bounds();
        if window.bounds.intersection(monitor) != Some(window.bounds) {
            return false;
        }
        for candidate in self.session.windows() {
            if candidate.id == id {
                break;
            }
            if candidate
                .bounds
                .intersection(window.bounds)
                .is_some_and(|overlap| !overlap.is_empty())
            {
                return false;
            }
        }
        true
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

    fn open_editor_for_selected(
        &mut self,
        tool: Tool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        platform_window::set_cursor_hidden(false);
        self.window_only = false;
        self.commit_edit(cx);
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

    fn primary_pointer_up(
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

    fn copy_selected(&mut self, cx: &mut Context<Self>) {
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
                );
                image_util::copy_png_bytes(bytes, &output, cx);
                self.close_overlays(cx);
            }
            Err(error) => self.set_notice(error.to_string(), cx),
        }
    }

    fn save_selected(&mut self, cx: &mut Context<Self>) {
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
                );
                self.close_overlays(cx);
            }
            Err(error) => self.set_notice(error.to_string(), cx),
        }
    }

    fn pin_selected(&mut self, cx: &mut Context<Self>) {
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
                crate::capture_history::record_png_bytes(&bytes, hash, pin_shadow, Some(scale));
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

    fn scroll_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
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

    fn record_selected(&mut self, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
        self.close_overlays(cx);
        cx.defer(move |cx| crate::record::start(selected.target, cx));
    }

    fn set_notice(&mut self, message: String, cx: &mut Context<Self>) {
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

    fn info_rect(&self) -> Option<Bounds<Pixels>> {
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

    fn toolbar_rect(&self) -> Option<Bounds<Pixels>> {
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

    fn info_field_display(&self, selected: &Selected, field: InfoField) -> String {
        self.info_field_value(selected, field).to_string()
    }

    fn start_edit(&mut self, field: InfoField, cx: &mut Context<Self>) {
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

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.edit = None;
            cx.notify();
        }
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
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

    fn next_edit_field(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let Some(edit) = selected.edit.clone() else {
            return;
        };
        self.commit_edit(cx);
        self.start_edit(edit.field.next(), cx);
    }

    fn cycle_ratio(&mut self, cx: &mut Context<Self>) {
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

    fn nudge_selection(&mut self, dx: i32, dy: i32, resize: bool, cx: &mut Context<Self>) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        let monitor = self.session.monitor_bounds();
        let mut rect = selected.rect;
        if resize {
            let max_width = monitor
                .size
                .width
                .saturating_sub(rect.origin.x.max(0) as u32);
            let max_height = monitor
                .size
                .height
                .saturating_sub(rect.origin.y.max(0) as u32);
            let width = i64::from(rect.size.width) + i64::from(dx);
            let height = i64::from(rect.size.height) + i64::from(dy);
            rect.size.width = width.clamp(1, i64::from(max_width.max(1))) as u32;
            rect.size.height = height.clamp(1, i64::from(max_height.max(1))) as u32;
        } else {
            rect.origin.x += dx;
            rect.origin.y += dy;
        }
        rect = clamp_to_monitor(rect, monitor.size);
        selected.rect = rect;
        selected.target = CaptureTarget::Region(CaptureRegion::new(self.session.monitor(), rect));
        cx.notify();
    }

    fn update_region_drag(
        &mut self,
        point: PhysicalPoint,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
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
                let mut dx = local.x - (drag.start.x - monitor.origin.x);
                let mut dy = local.y - (drag.start.y - monitor.origin.y);
                if modifiers.shift {
                    if dx.abs() >= dy.abs() {
                        dy = 0;
                    } else {
                        dx = 0;
                    }
                }
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
                modifiers.shift,
                modifiers.alt,
                monitor.size,
            ),
        };
        selected.rect = rect;
        selected.target = CaptureTarget::Region(CaptureRegion::new(self.session.monitor(), rect));
        cx.notify();
    }

    fn finish_region_drag(&mut self, cx: &mut Context<Self>) {
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

    fn toggle_effects_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.effects_open = !selected.effects_open;
            selected.hints_open = false;
            selected.edit = None;
        }
        cx.notify();
    }

    fn set_region_radius(&mut self, radius: u32, cx: &mut Context<Self>) {
        self.effects.corner_radius = radius;
        settings_state::update(|settings| {
            settings.region_corner_radius = radius;
        });
        cx.notify();
    }

    fn toggle_region_shadow(&mut self, cx: &mut Context<Self>) {
        self.effects.shadow = !self.effects.shadow;
        let shadow = self.effects.shadow;
        settings_state::update(|settings| settings.region_shadow = shadow);
        cx.notify();
    }

    fn dismiss_effects(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.effects_open = false;
        }
        cx.notify();
    }

    fn dismiss_hints(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_mut() {
            selected.hints_open = false;
        }
        cx.notify();
    }

    fn info_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        let rect = self.info_rect()?;
        let language = settings_state::get().language;
        let mut bar = div()
            .id("overlay-info-bar")
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(7.0))
            .rounded(px(self.chrome.radius))
            .bg(self.chrome.surface_strong)
            .border_1()
            .border_color(self.chrome.border)
            .shadow(self.chrome.shadow())
            .cursor(CursorStyle::Arrow)
            .text_size(px(11.0));

        for field in [
            InfoField::X,
            InfoField::Y,
            InfoField::Width,
            InfoField::Height,
        ] {
            let label = match field {
                InfoField::X => "X",
                InfoField::Y => "Y",
                InfoField::Width => "W",
                InfoField::Height => "H",
            };
            let editing = selected
                .edit
                .as_ref()
                .is_some_and(|edit| edit.field == field);
            let value = if let Some(edit) = selected.edit.as_ref().filter(|_| editing) {
                edit.buffer.clone()
            } else {
                self.info_field_display(selected, field)
            };
            let mut value_row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(1.0))
                .child(div().text_color(self.chrome.text).child(value));
            if editing {
                value_row = value_row.child(
                    div()
                        .w(px(1.0))
                        .h(px(12.0))
                        .bg(self.chrome.accent)
                        .with_animation(
                            SharedString::from(format!("overlay-info-caret-{label}")),
                            crate::ui::caret_blink(),
                            |caret, delta| caret.opacity(delta),
                        ),
                );
            }
            bar = bar.child(
                div()
                    .id(format!("overlay-info-{label}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(3.0))
                    .px(px(4.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .when(editing, |chip| chip.bg(self.chrome.selected))
                    .when(!editing, |chip| {
                        chip.hover(|style| style.bg(self.chrome.hover))
                    })
                    .active(|style| style.opacity(0.85))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.start_edit(field, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(div().text_color(self.chrome.muted).child(label))
                    .child(value_row),
            );
        }

        let unit_label = selected.unit.label();
        let ratio_label = selected.ratio.label(language);
        bar = bar
            .child(
                div()
                    .id("overlay-info-unit")
                    .px(px(5.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .text_color(self.chrome.muted)
                    .hover(|style| style.bg(self.chrome.hover))
                    .active(|style| style.opacity(0.85))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(selected) = this.selected.as_mut() {
                            selected.unit = selected.unit.next();
                            selected.edit = None;
                        }
                        cx.notify();
                    }))
                    .child(unit_label),
            )
            .child(div().w(px(1.0)).h(px(14.0)).bg(self.chrome.separator))
            .child(
                div()
                    .id("overlay-info-ratio")
                    .px(px(6.0))
                    .py(px(3.0))
                    .rounded(px(5.0))
                    .text_color(self.chrome.text)
                    .hover(|style| style.bg(self.chrome.hover))
                    .active(|style| style.opacity(0.85))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_ratio(cx)))
                    .child(ratio_label),
            );

        if !matches!(selected.target, CaptureTarget::Window(_)) {
            bar = bar.child(crate::capture_panels::effects_chip(
                "overlay-info-effects",
                self.chrome,
                language,
                selected.effects_open,
                self.effects.active(),
                cx,
                Self::toggle_effects_panel,
            ));
        }

        bar = bar.child(crate::capture_panels::refresh_button(
            "overlay-info-refresh",
            self.chrome,
            language,
            cx,
            Self::refresh_frame,
        ));

        Some(
            crate::ui::enter_slide_fade(
                bar,
                "overlay-info-enter",
                f32::from(rect.origin.y),
                4.0,
                Duration::from_millis(90),
            )
            .into_any_element(),
        )
    }

    fn effects_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        if !selected.effects_open {
            return None;
        }
        let info = self.info_rect()?;
        let language = settings_state::get().language;
        let width = crate::capture_panels::effects_panel_width(language)
            .min((f32::from(self.window_size.width) - 16.0).max(120.0));
        let height = 40.0;
        let left = f32::from(info.origin.x).clamp(
            8.0,
            (f32::from(self.window_size.width) - width - 8.0).max(8.0),
        );
        let top = crate::ui::popover_top(
            info,
            height,
            f32::from(self.window_size.height),
            self.toolbar_rect(),
        );
        let rect = Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        };
        let offset = if top >= f32::from(info.origin.y) {
            5.0
        } else {
            -5.0
        };
        Some(crate::capture_panels::effects_panel(
            "overlay",
            self.chrome,
            language,
            rect,
            self.effects,
            offset,
            cx,
            Self::set_region_radius,
            Self::toggle_region_shadow,
            Self::dismiss_effects,
        ))
    }

    fn action_button(
        &self,
        id: &'static str,
        label: String,
        icon: &'static [u8],
        color: Rgba,
        cx: &mut Context<Self>,
        action: fn(&mut Self, &mut Context<Self>),
    ) -> AnyElement {
        div()
            .id(id)
            .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
            .w(px(30.0))
            .h(px(30.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(self.chrome.hover))
            .active(|style| style.opacity(0.85))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
            .child(icons::icon(icon, 17.0, color))
            .into_any_element()
    }

    fn selection_toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        if selected.space_down {
            return None;
        }
        let rect = self.toolbar_rect()?;
        let language = settings_state::get().language;
        let mut tools = div().flex().flex_row().items_center().gap(px(1.0));
        for tool in Tool::ALL {
            let label = t(language, tool.label_key()).to_owned();
            tools = tools.child(
                div()
                    .id(format!("overlay-tool-{}", tool.id()))
                    .tooltip(move |_, cx| tooltip_view(label.clone(), cx))
                    .w(px(30.0))
                    .h(px(30.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(self.chrome.hover))
                    .active(|style| style.opacity(0.85))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_editor_for_selected(tool, window, cx);
                    }))
                    .child(icons::icon(tool.icon(), 17.0, self.chrome.icon)),
            );
        }

        let mut actions = div().flex().flex_row().items_center().gap(px(1.0));
        actions = actions.child(self.action_button(
            "overlay-action-pin",
            t(language, "editor.pin").to_owned(),
            icons::PIN,
            self.chrome.icon,
            cx,
            Self::pin_selected,
        ));
        actions = actions.child(self.action_button(
            "overlay-action-copy",
            t(language, "editor.copy").to_owned(),
            icons::COPY,
            self.chrome.icon,
            cx,
            Self::copy_selected,
        ));
        actions = actions.child(self.action_button(
            "overlay-action-save",
            t(language, "editor.save").to_owned(),
            icons::SAVE,
            self.chrome.icon,
            cx,
            Self::save_selected,
        ));
        actions = actions.child(self.action_button(
            "overlay-action-scroll",
            t(language, "menu.scroll_capture").to_owned(),
            icons::SCROLL,
            self.chrome.icon,
            cx,
            Self::scroll_selected,
        ));
        actions = actions.child(self.action_button(
            "overlay-action-record",
            t(language, "menu.record").to_owned(),
            icons::RECORDING,
            self.chrome.icon,
            cx,
            Self::record_selected,
        ));
        actions = actions.child(self.action_button(
            "overlay-action-close",
            t(language, "editor.close").to_owned(),
            icons::CLOSE,
            self.chrome.muted,
            cx,
            Self::close_overlays,
        ));

        Some(
            crate::ui::enter_slide_fade(
                div()
                    .id("overlay-toolbar")
                    .absolute()
                    .left(rect.origin.x)
                    .top(rect.origin.y)
                    .w(rect.size.width)
                    .h(rect.size.height)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(8.0))
                    .rounded(px(self.chrome.radius))
                    .bg(self.chrome.surface)
                    .border_1()
                    .border_color(self.chrome.border)
                    .cursor(CursorStyle::Arrow)
                    .shadow(self.chrome.shadow())
                    .child(tools)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(1.0))
                            .child(actions),
                    ),
                "overlay-toolbar-enter",
                f32::from(rect.origin.y),
                4.0,
                Duration::from_millis(90),
            )
            .into_any_element(),
        )
    }

    fn hints_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        if !selected.hints_open {
            return None;
        }
        let language = settings_state::get().language;
        let width = crate::capture_panels::hints_panel_width()
            .min((f32::from(self.window_size.width) - 16.0).max(120.0));
        let pin_hotkey = crate::hotkey::display_hotkey(&settings_state::get().pin_hotkey);
        let k = |key: &str| t(language, key).to_owned();
        let rows = vec![
            (k("editor.hint_key_copy"), k("editor.hint_copy")),
            ("Cmd/Ctrl+S".to_owned(), k("editor.hint_save")),
            (pin_hotkey, k("editor.hint_pin")),
            ("Space".to_owned(), k("editor.hint_space")),
            ("R / F5".to_owned(), k("editor.hint_ratio_refresh")),
            ("Esc".to_owned(), k("editor.hint_cancel")),
        ];
        let height = crate::capture_panels::hints_panel_height(rows.len());
        let left = (f32::from(self.window_size.width) - width) / 2.0;
        let top = (f32::from(self.window_size.height) - height) / 2.0;
        let rect = Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        };
        Some(crate::capture_panels::hints_panel(
            "overlay",
            self.chrome,
            language,
            rect,
            rows,
            cx,
            Self::dismiss_hints,
        ))
    }

    fn selection_handles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(selection) = self.selection_bounds() else {
            return Vec::new();
        };
        let mut handles = Vec::new();
        let radius = self.selection_effects_radius();
        for (handle, position) in handle_positions(selection) {
            let (dx, dy) = corner_handle_offset(handle, radius);
            let cursor = region_handle_cursor(handle);
            handles.push(
                div()
                    .id(format!("overlay-handle-{:?}", handle))
                    .absolute()
                    .left(px(f32::from(position.x) + dx - HANDLE_SIZE / 2.0))
                    .top(px(f32::from(position.y) + dy - HANDLE_SIZE / 2.0))
                    .w(px(HANDLE_SIZE))
                    .h(px(HANDLE_SIZE))
                    .rounded(px(1.5))
                    .bg(self.chrome.handle_fill)
                    .border_1()
                    .border_color(self.chrome.handle_border)
                    .shadow(vec![
                        BoxShadow::new(px(0.0), px(1.0), hsla(0.0, 0.0, 0.0, 0.28))
                            .blur_radius(px(2.0)),
                    ])
                    .cursor(cursor)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let point = this.to_physical(event.position);
                            if let Some(selected) = this.selected.as_mut() {
                                selected.drag = Some(RegionDrag {
                                    kind: RegionDragKind::Resize(handle),
                                    start: point,
                                    original: selected.rect,
                                });
                                selected.edit = None;
                                selected.effects_open = false;
                                selected.hints_open = false;
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
        handles
    }

    fn selection_size_tag(&self, language: Language) -> Option<Div> {
        if self.selected.is_some() {
            return None;
        }
        let rect = self
            .session
            .selection_rect()
            .and_then(|rect| self.local_rect(rect))?;
        let (scale_x, scale_y) = self.frame_ratio();
        let width = (f32::from(rect.size.width) * scale_x).round() as u32;
        let height = (f32::from(rect.size.height) * scale_y).round() as u32;
        let ratio = self.aspect.ratio().map(|_| self.aspect.label(language));
        let text = match ratio {
            Some(ratio) => format!("{width} x {height}  {ratio}"),
            None => format!("{width} x {height}"),
        };
        let top = (f32::from(rect.origin.y) - 26.0).max(0.0);
        Some(
            div()
                .absolute()
                .left(rect.origin.x)
                .top(px(top))
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .bg(self.chrome.surface)
                .border_1()
                .border_color(self.chrome.border)
                .shadow(self.chrome.shadow())
                .text_size(px(11.0))
                .text_color(self.chrome.text)
                .child(text),
        )
    }

    fn close_overlays(&mut self, cx: &mut Context<Self>) {
        platform_window::set_cursor_hidden(false);
        let handles = cx.windows();
        cx.defer(move |cx| {
            crate::windows::close_capture_auxiliary(cx, handles);
        });
    }
}

impl Drop for OverlayView {
    fn drop(&mut self) {
        registry::clear_focus_if(self.entity_id);
    }
}

impl OverlayView {
    fn primary_pointer_down(
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
                    self.copy_selected(cx);
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

    fn render_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.chrome = ChromeTheme::for_window(window, Some(self.accent_hex));
        if self.window_only
            && self.cursor.is_some()
            && self.selected.is_none()
            && self.editor.is_none()
        {
            platform_window::set_cursor_hidden(true);
        }
        if let Some(editor) = self.editor.clone() {
            let rect = Some(editor.read(cx).image_view_rect(window));
            let layers = self.dim_layers(rect);
            return div()
                .relative()
                .size_full()
                .child(
                    img(self.image.clone())
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                )
                .children(layers)
                .child(editor)
                .into_any_element();
        }
        let language = settings_state::get().language;
        let selected_bounds = self.selection_bounds();
        let selected = self.selected.is_some();
        let drag_selection = self
            .session
            .selection_rect()
            .and_then(|rect| self.local_rect(rect));
        let hovered_window = self
            .session
            .hovered_window()
            .and_then(|window| self.local_bounds(window.bounds));
        let hovered_element = (!self.window_only)
            .then(|| {
                self.session
                    .hovered_element()
                    .and_then(|element| self.local_bounds(element))
            })
            .flatten();
        let hovered = hovered_element.or(hovered_window);
        let desktop_hover = !selected
            && !self.session.has_drag()
            && self.cursor.is_some()
            && hovered.is_none()
            && self.editing_rect.is_none();
        let screen_bounds = desktop_hover.then(|| Bounds {
            origin: point(px(0.0), px(0.0)),
            size: self.window_size,
        });
        let hole = selected_bounds
            .or(drag_selection)
            .or(hovered)
            .or(screen_bounds)
            .or(self.editing_rect);

        let magnifier = (!selected && !self.session.is_dragging())
            .then(|| self.magnifier_panel(drag_selection, language, cx))
            .flatten();
        let size_tag = self.selection_size_tag(language);
        let window_mode_hint = (!selected && self.window_only).then(|| {
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
                        .rounded(px(self.chrome.radius))
                        .bg(self.chrome.surface)
                        .border_1()
                        .border_color(rgba((ELEMENT_HOVER << 8) | 0xAA))
                        .shadow(self.chrome.shadow())
                        .text_size(px(12.5))
                        .text_color(self.chrome.text)
                        .child(t(language, "overlay.window_mode_hint")),
                )
        });
        let capture_hint = (!selected && self.cursor.is_none()).then(|| {
            div()
                .absolute()
                .bottom(px(16.0))
                .left(px(0.0))
                .right(px(0.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(11.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .bg(self.chrome.surface)
                        .border_1()
                        .border_color(self.chrome.border)
                        .shadow(self.chrome.shadow())
                        .text_size(px(10.5))
                        .text_color(self.chrome.muted)
                        .child(t(language, "overlay.capture_hint")),
                )
        });
        let notice_serial = self.notice_serial;
        let notice_leaving = self.notice_leaving;
        let notice = self.notice.clone().map(|message| {
            let duration = if notice_leaving {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(120)
            };
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
                    SharedString::from(format!("overlay-notice-{notice_serial}-{notice_leaving}")),
                    Animation::new(duration).with_easing(|delta| 1.0 - (1.0 - delta).powi(3)),
                    move |toast, delta| {
                        if notice_leaving {
                            toast.opacity(1.0 - delta)
                        } else {
                            toast.opacity(delta).bottom(px(46.0 - (1.0 - delta) * 4.0))
                        }
                    },
                )
        });

        let screen_color = rgba((SCREEN_SELECTION << 8) | 0xFF);
        let selection_radius = self.selection_effects_radius();
        let drag_radius = if self.selected.is_none() && drag_selection.is_some() {
            self.effects.corner_radius as f32
        } else {
            0.0
        };
        let radius = selection_radius.max(drag_radius);
        let mut layers = self.dim_layers_rounded(hole, radius);
        if let Some(rect) = selected_bounds {
            if radius <= 0.5 {
                layers.push(border(rect, self.accent, radius).into_any_element());
            }
        } else if let Some(rect) = drag_selection {
            if radius <= 0.5 {
                layers.push(border(rect, self.accent, 0.0).into_any_element());
            }
        } else if let Some(rect) = hovered_element {
            layers.push(element_border(rect, rgba((ELEMENT_HOVER << 8) | 0xFF)).into_any_element());
        } else if let Some(rect) = hovered_window {
            layers.push(border(rect, self.accent, 0.0).into_any_element());
        } else if let Some(rect) = screen_bounds {
            layers.push(screen_border(rect, screen_color).into_any_element());
        }

        if self.selected.is_none()
            && self.window_only
            && let Some(cursor) = self.cursor
        {
            let monitor = self.session.monitor_bounds();
            let (scale_x, scale_y) = self.frame_ratio();
            let x = (cursor.x - monitor.origin.x) as f32 / scale_x;
            let y = (cursor.y - monitor.origin.y) as f32 / scale_y;
            let width = f32::from(self.window_size.width);
            let height = f32::from(self.window_size.height);
            if x >= 0.0 && y >= 0.0 && x <= width && y <= height {
                layers.push(camera_cursor(x, y, self.chrome.dark).into_any_element());
            }
        }

        if !selected
            && let Some(window) = self.session.hovered_window()
            && let Some(rect) = hovered
        {
            layers.push(
                self.window_label(
                    window,
                    rect,
                    hovered_element.and(self.session.hovered_element()),
                )
                .into_any_element(),
            );
        }
        if !selected && desktop_hover {
            layers.push(self.screen_label(language).into_any_element());
        }

        let handles = if selected {
            self.selection_handles(cx)
        } else {
            Vec::new()
        };
        let info_bar = self.info_bar(cx);
        let toolbar = self.selection_toolbar(cx);
        let effects_panel = self.effects_panel(cx);
        let hints_panel = self.hints_panel(cx);

        div()
            .relative()
            .size_full()
            .cursor(self.cursor_style)
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(
                |this, event: &ModifiersChangedEvent, _window, cx| {
                    let shift = event.modifiers.shift;
                    if shift && !this.shift_held && this.selected.is_none() {
                        this.toggle_color_format(cx);
                    }
                    this.shift_held = shift;

                    this.set_window_only(control_down(&event.modifiers), cx);
                    cx.notify();
                },
            ))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let command =
                    event.keystroke.modifiers.platform || event.keystroke.modifiers.control;

                if key == "shift" && !this.shift_held && this.selected.is_none() {
                    this.toggle_color_format(cx);
                }
                if key == "shift" {
                    this.shift_held = true;
                }

                if let Some(selected) = this.selected.as_mut() {
                    if selected.edit.is_some() {
                        match key {
                            "escape" => {
                                this.cancel_edit(cx);
                                return;
                            }
                            "enter" => {
                                this.commit_edit(cx);
                                return;
                            }
                            "tab" => {
                                this.next_edit_field(cx);
                                return;
                            }
                            "backspace" | "delete" => {
                                if let Some(selected) = this.selected.as_mut()
                                    && let Some(edit) = selected.edit.as_mut()
                                {
                                    edit.fresh = false;
                                    edit.buffer.pop();
                                }
                                cx.notify();
                                return;
                            }
                            _ => {}
                        }
                        if let Some(ch) = event.keystroke.key.chars().next()
                            && (ch.is_ascii_digit() || ch == '-')
                        {
                            if let Some(selected) = this.selected.as_mut()
                                && let Some(edit) = selected.edit.as_mut()
                            {
                                if edit.fresh {
                                    edit.buffer.clear();
                                    edit.fresh = false;
                                }
                                edit.buffer.push(ch);
                            }
                            cx.notify();
                            return;
                        }
                        if !command {
                            return;
                        }
                    }
                    if key == "escape" {
                        if selected.hints_open || selected.effects_open {
                            selected.hints_open = false;
                            selected.effects_open = false;
                            cx.notify();
                            return;
                        }
                        this.close_overlays(cx);
                        return;
                    }
                    if command && matches!(key, "w" | "q") {
                        this.close_overlays(cx);
                        return;
                    }
                    if key == "enter" || (command && key == "c" && !event.keystroke.modifiers.shift)
                    {
                        this.copy_selected(cx);
                        return;
                    }
                    if command && key == "s" && !event.keystroke.modifiers.shift {
                        this.save_selected(cx);
                        return;
                    }
                    if key == "f5" {
                        if !event.is_held {
                            this.refresh_frame(window, cx);
                        }
                        return;
                    }
                    if key == "f1" {
                        if let Some(selected) = this.selected.as_mut() {
                            selected.hints_open = !selected.hints_open;
                            selected.effects_open = false;
                        }
                        cx.notify();
                        return;
                    }
                    if key.eq_ignore_ascii_case("r") {
                        this.cycle_ratio(cx);
                        return;
                    }
                    if key == "space" {
                        if let Some(selected) = this.selected.as_mut() {
                            selected.space_down = true;
                            selected.effects_open = false;
                            selected.hints_open = false;
                        }
                        cx.notify();
                        return;
                    }
                    if key == "delete" || key == "backspace" {
                        this.selected = None;
                        this.screen_targeting = false;
                        this.session.cancel();
                        cx.notify();
                        return;
                    }
                    if let Some((dx, dy)) = match key {
                        "left" => Some((-1, 0)),
                        "right" => Some((1, 0)),
                        "up" => Some((0, -1)),
                        "down" => Some((0, 1)),
                        _ => None,
                    } {
                        this.nudge_selection(dx, dy, event.keystroke.modifiers.shift, cx);
                        return;
                    }
                    return;
                }

                if key == "escape" || (command && matches!(key, "w" | "q")) {
                    this.screen_targeting = false;
                    this.session.cancel();
                    this.close_overlays(cx);
                    return;
                }
                if key.eq_ignore_ascii_case("r") {
                    this.cycle_aspect(cx);
                } else if key.eq_ignore_ascii_case("c") {
                    this.copy_color(cx);
                } else if key == "f5" && !event.is_held {
                    this.refresh_frame(window, cx);
                }
                let _ = window;
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                if event.keystroke.key == "shift" {
                    this.shift_held = false;
                    cx.notify();
                }
                if event.keystroke.key == "space"
                    && let Some(selected) = this.selected.as_mut()
                {
                    selected.space_down = false;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let point = this.to_physical(event.position);
                this.cursor = Some(point);
                this.set_window_only(control_down(&event.modifiers), cx);
                if this.window_only && this.selected.is_none() {
                    platform_window::set_cursor_hidden(true);
                }
                if this
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.drag.is_some())
                {
                    this.cursor_style = crate::ui::closed_hand_cursor();
                    if event.pressed_button.is_none() {
                        this.finish_region_drag(cx);
                    } else {
                        this.update_region_drag(point, event.modifiers, cx);
                    }
                    return;
                }
                if let Some(selected) = this.selected.as_ref() {
                    let monitor = this.session.monitor_bounds();
                    let local =
                        PhysicalPoint::new(point.x - monitor.origin.x, point.y - monitor.origin.y);
                    let edge_tolerance = 8.0 * this.effective_scale();
                    let inside = local.x >= selected.rect.origin.x
                        && local.y >= selected.rect.origin.y
                        && local.x <= selected.rect.origin.x + selected.rect.size.width as i32
                        && local.y <= selected.rect.origin.y + selected.rect.size.height as i32;
                    this.cursor_style = if let Some(handle) =
                        edge_resize_handle(selected.rect, local, edge_tolerance)
                    {
                        region_handle_cursor(handle)
                    } else if inside {
                        crate::ui::open_hand_cursor()
                    } else {
                        CursorStyle::Crosshair
                    };
                    cx.notify();
                    return;
                }
                this.cursor_style = CursorStyle::Crosshair;
                if this.session.has_drag() && event.pressed_button.is_none() {
                    this.screen_targeting = false;
                    this.session.cancel();
                } else {
                    this.session.pointer_moved(point);
                    if !this.session.has_drag() {
                        this.refresh_auto_target(point, cx);
                    }
                }
                this.update_magnifier(window);
                cx.notify();
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _window, cx| {
                if this.selected.is_some() {
                    return;
                }
                let delta = match event.delta {
                    ScrollDelta::Lines(delta) => delta.y,
                    ScrollDelta::Pixels(delta) => f32::from(delta.y),
                };
                if let Some(point) = this.cursor {
                    this.refresh_auto_target(point, cx);
                }
                this.adjust_auto_target(delta);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if cfg!(target_os = "macos")
                        && this.selected.is_none()
                        && this.editor.is_none()
                        && (this.window_only || control_down(&event.modifiers))
                    {
                        this.ctrl_click_primary = true;
                        this.primary_pointer_down(event, true, cx);
                        cx.stop_propagation();
                        return;
                    }
                    if this
                        .selected
                        .as_ref()
                        .is_some_and(|selected| selected.edit.is_some())
                    {
                        this.commit_edit(cx);
                        return;
                    }
                    if let Some(selected) = this.selected.as_mut() {
                        selected.hints_open = false;
                        selected.effects_open = false;
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.ctrl_click_primary {
                        this.ctrl_click_primary = false;
                        this.primary_pointer_up(event, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if this.ctrl_click_primary {
                        return;
                    }
                    this.primary_pointer_down(event, false, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if this.ctrl_click_primary {
                        this.ctrl_click_primary = false;
                        this.primary_pointer_up(event, window, cx);
                        return;
                    }
                    this.primary_pointer_up(event, window, cx);
                }),
            )
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, window, cx| {
                if this
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.drag.is_some())
                {
                    this.finish_region_drag(cx);
                }
                this.cursor = None;
                this.clear_magnifier(window);
                this.cursor_style = CursorStyle::Crosshair;
                this.ctrl_click_primary = false;
                this.element_window = None;
                this.element_pending = None;
                this.element_pending_point = None;
                this.element_candidates.clear();
                this.element_level = 0;
                this.screen_targeting = false;
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
            .children(handles)
            .children(info_bar)
            .children(toolbar)
            .children(magnifier)
            .children(size_tag)
            .children(window_mode_hint)
            .children(capture_hint)
            .children(notice)
            .children(effects_panel)
            .children(hints_panel)
            .into_any_element()
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
    use super::frame_matches_monitor;
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
        assert!(frame_matches_monitor(&frame(1920, 1080), &target));
        assert!(frame_matches_monitor(&frame(1921, 1079), &target));
        assert!(!frame_matches_monitor(&frame(1920, 1200), &target));
        assert!(!frame_matches_monitor(&frame(2560, 1440), &target));
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
