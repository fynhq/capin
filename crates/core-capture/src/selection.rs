use core_types::{CaptureRegion, MonitorId, PhysicalPoint, PhysicalRect, WindowId};

use crate::WindowInfo;

/// Pointer travel, in logical pixels, before a press becomes a region drag.
pub const DRAG_THRESHOLD_PX: i32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionOutcome {
    Monitor(MonitorId),
    Window(WindowId),
    Region(CaptureRegion),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionAction {
    None,
    Capture(SelectionOutcome),
}

struct Drag {
    origin: PhysicalPoint,
    raw_current: PhysicalPoint,
    current: PhysicalPoint,
    start_window: Option<WindowId>,
    start_element: Option<PhysicalRect>,
    over_window: bool,
    active: bool,
}

/// Interaction state for one monitor's screenshot overlay.
///
/// Pointer coordinates are in the virtual desktop's physical pixel space.
/// Window bounds arrive in z-order from the backend, topmost first. Selection
/// geometry is exposed in monitor-local physical pixels for rendering.
///
/// Region selection is the default. The screenshot overlay enables
/// `window_targeting` for the whole session so hovering identifies a window or
/// element and clicking captures it; dragging still produces a free region.
pub struct SelectionSession {
    monitor: MonitorId,
    monitor_bounds: PhysicalRect,
    windows: Vec<WindowInfo>,
    hovered: Option<WindowId>,
    hovered_element: Option<PhysicalRect>,
    drag: Option<Drag>,
    aspect_ratio: Option<f32>,
    window_targeting: bool,
    drag_threshold: i32,
}

impl SelectionSession {
    pub fn new(monitor: MonitorId, monitor_bounds: PhysicalRect, windows: Vec<WindowInfo>) -> Self {
        Self::with_scale(monitor, monitor_bounds, windows, 1.0)
    }

    /// Builds a session for one monitor. Windows reported for other monitors
    /// are discarded so mixed-DPI macOS desktops cannot hover a window that
    /// visually belongs to a neighboring display.
    pub fn with_scale(
        monitor: MonitorId,
        monitor_bounds: PhysicalRect,
        windows: Vec<WindowInfo>,
        scale: f32,
    ) -> Self {
        let windows = windows
            .into_iter()
            .filter(|window| window.monitor.is_none() || window.monitor == Some(monitor))
            .collect();
        let drag_threshold = (DRAG_THRESHOLD_PX as f32 * scale.max(0.1)).round() as i32;
        Self {
            monitor,
            monitor_bounds,
            windows,
            hovered: None,
            hovered_element: None,
            drag: None,
            aspect_ratio: None,
            window_targeting: false,
            drag_threshold: drag_threshold.max(1),
        }
    }

    pub fn monitor(&self) -> MonitorId {
        self.monitor
    }

    pub fn monitor_bounds(&self) -> PhysicalRect {
        self.monitor_bounds
    }

    pub fn windows(&self) -> &[WindowInfo] {
        &self.windows
    }

    /// Enables automatic window picking. Disabling it clears any
    /// window/element highlight so the caller can fall back to pure region
    /// selection.
    pub fn set_window_targeting(&mut self, enabled: bool) {
        self.window_targeting = enabled;
        if !enabled {
            self.hovered = None;
            self.hovered_element = None;
        }
    }

    pub fn window_targeting(&self) -> bool {
        self.window_targeting
    }

    pub fn hovered_window(&self) -> Option<&WindowInfo> {
        let hovered = self.hovered?;
        self.window(hovered)
    }

    pub fn window(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.iter().find(|window| window.id == id)
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.as_ref().is_some_and(|drag| drag.active)
    }

    /// Returns whether a press is pending or a drag is active.
    pub fn has_drag(&self) -> bool {
        self.drag.is_some()
    }

    pub fn hovered_element(&self) -> Option<PhysicalRect> {
        self.hovered_element
    }

    /// Sets the UI element under the pointer. The overlay resolves platform
    /// accessibility bounds and keeps this session free of platform code.
    pub fn set_hovered_element(&mut self, element: Option<PhysicalRect>) {
        if self.drag.as_ref().is_some_and(|drag| drag.active) {
            self.hovered_element = None;
        } else {
            self.hovered_element = element;
        }
    }

    pub fn aspect_ratio(&self) -> Option<f32> {
        self.aspect_ratio
    }

    /// Locks drag geometry to `width / height`. `None` restores free selection.
    pub fn set_aspect_ratio(&mut self, ratio: Option<f32>) {
        self.aspect_ratio = ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        if let Some(drag) = &mut self.drag {
            drag.current = constrain_ratio(
                self.monitor_bounds,
                drag.origin,
                drag.raw_current,
                self.aspect_ratio,
            );
        }
    }

    /// Returns the active region in monitor-local physical pixels.
    pub fn selection_rect(&self) -> Option<PhysicalRect> {
        let drag = self.drag.as_ref()?;
        if !drag.active {
            return None;
        }
        region_from_drag(self.monitor, self.monitor_bounds, drag.origin, drag.current)
            .map(|region| region.rect)
    }

    pub fn pointer_moved(&mut self, point: PhysicalPoint) {
        let threshold = self.drag_threshold;
        if let Some(drag) = &mut self.drag {
            drag.raw_current = point;
            drag.current =
                constrain_ratio(self.monitor_bounds, drag.origin, point, self.aspect_ratio);
            if is_past_threshold(drag.origin, point, threshold) {
                drag.active = true;
                self.hovered = None;
                self.hovered_element = None;
            }
        } else {
            if self.window_targeting {
                self.hovered = self.hit_test(point).map(|window| window.id);
            } else {
                self.hovered = None;
            }
            if self.hovered.is_none() {
                self.hovered_element = None;
            }
        }
    }

    /// Clears hover when the pointer leaves the overlay, unless a drag is active.
    pub fn pointer_left(&mut self) {
        if self.drag.is_none() {
            self.hovered = None;
            self.hovered_element = None;
        }
    }

    pub fn pointer_pressed(&mut self, point: PhysicalPoint) {
        // Replace a stale press left behind by a mouse-up that happened outside
        // the overlay. Without this, the next click would capture a region from
        // the old origin.
        self.drag = None;

        let over_window = self.hit_test(point).is_some();
        let start_window = if self.window_targeting {
            self.hit_test(point).map(|window| window.id)
        } else {
            None
        };
        let start_element = if self.window_targeting {
            self.hovered_element
        } else {
            None
        };
        self.hovered = start_window;
        self.drag = Some(Drag {
            origin: point,
            raw_current: point,
            current: point,
            start_window,
            start_element,
            over_window,
            active: false,
        });
    }

    pub fn pointer_released(&mut self, point: PhysicalPoint) -> SelectionAction {
        let Some(mut drag) = self.drag.take() else {
            return SelectionAction::None;
        };
        drag.raw_current = point;
        drag.current = constrain_ratio(self.monitor_bounds, drag.origin, point, self.aspect_ratio);

        if drag.active || is_past_threshold(drag.origin, point, self.drag_threshold) {
            let Some(region) =
                region_from_drag(self.monitor, self.monitor_bounds, drag.origin, drag.current)
            else {
                self.hovered = None;
                return SelectionAction::None;
            };
            return SelectionAction::Capture(SelectionOutcome::Region(region));
        }

        if let Some(element) = drag.start_element
            && let Some(region) = region_from_rect(self.monitor, self.monitor_bounds, element)
        {
            return SelectionAction::Capture(SelectionOutcome::Region(region));
        }

        if let Some(window) = drag.start_window {
            SelectionAction::Capture(SelectionOutcome::Window(window))
        } else if drag.over_window {
            // Window targeting is disabled, so a click over a window does not
            // implicitly capture it.
            SelectionAction::None
        } else {
            SelectionAction::Capture(SelectionOutcome::Monitor(self.monitor))
        }
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.hovered = None;
        self.hovered_element = None;
    }

    fn hit_test(&self, point: PhysicalPoint) -> Option<&WindowInfo> {
        self.windows
            .iter()
            .find(|window| window.bounds.contains(point))
    }
}

/// Returns the accessible elements under `point`, smallest first. This is the
/// element hierarchy used by the overlay's scroll-wheel targeting.
pub fn element_candidates(
    elements: &[PhysicalRect],
    window: PhysicalRect,
    point: PhysicalPoint,
) -> Vec<PhysicalRect> {
    let mut candidates: Vec<PhysicalRect> = elements
        .iter()
        .copied()
        .filter(|rect| {
            *rect != window
                && rect.size.width > 4
                && rect.size.height > 4
                && rect.size.width <= window.size.width
                && rect.size.height <= window.size.height
                && rect.contains(point)
        })
        .collect();
    candidates.sort_by_key(|rect| u64::from(rect.size.width) * u64::from(rect.size.height));
    candidates
}

fn is_past_threshold(origin: PhysicalPoint, current: PhysicalPoint, threshold: i32) -> bool {
    (current.x - origin.x)
        .abs()
        .max((current.y - origin.y).abs())
        >= threshold
}

fn constrain_ratio(
    monitor_bounds: PhysicalRect,
    origin: PhysicalPoint,
    point: PhysicalPoint,
    ratio: Option<f32>,
) -> PhysicalPoint {
    let Some(ratio) = ratio else {
        return point;
    };
    let dx = i64::from(point.x) - i64::from(origin.x);
    let dy = i64::from(point.y) - i64::from(origin.y);
    if dx == 0 && dy == 0 {
        return point;
    }

    let sign_x = if dx < 0 { -1.0 } else { 1.0 };
    let sign_y = if dy < 0 { -1.0 } else { 1.0 };
    let width = dx.unsigned_abs() as f32;
    let height = dy.unsigned_abs() as f32;
    let mut target_width = width.max(height * ratio);
    let mut target_height = target_width / ratio;

    let available_width = if sign_x > 0.0 {
        (monitor_bounds.right() - i64::from(origin.x)).max(0) as f32
    } else {
        (i64::from(origin.x) - i64::from(monitor_bounds.origin.x)).max(0) as f32
    };
    let available_height = if sign_y > 0.0 {
        (monitor_bounds.bottom() - i64::from(origin.y)).max(0) as f32
    } else {
        (i64::from(origin.y) - i64::from(monitor_bounds.origin.y)).max(0) as f32
    };
    if target_width > available_width || target_height > available_height {
        let scale = (available_width / target_width.max(1.0))
            .min(available_height / target_height.max(1.0))
            .max(0.0);
        target_width *= scale;
        target_height *= scale;
    }

    PhysicalPoint::new(
        origin.x + (sign_x * target_width).round() as i32,
        origin.y + (sign_y * target_height).round() as i32,
    )
}

fn region_from_rect(
    monitor: MonitorId,
    monitor_bounds: PhysicalRect,
    rect: PhysicalRect,
) -> Option<CaptureRegion> {
    let rect = rect.intersection(monitor_bounds)?;
    Some(CaptureRegion::new(
        monitor,
        PhysicalRect::new(
            rect.origin.x - monitor_bounds.origin.x,
            rect.origin.y - monitor_bounds.origin.y,
            rect.size.width,
            rect.size.height,
        ),
    ))
}

/// Builds a monitor-local capture region from a drag in virtual desktop
/// coordinates. Returns `None` when the drag has no area after clamping.
pub fn region_from_drag(
    monitor: MonitorId,
    monitor_bounds: PhysicalRect,
    start: PhysicalPoint,
    end: PhysicalPoint,
) -> Option<CaptureRegion> {
    if monitor_bounds.is_empty() {
        return None;
    }

    let start = clamp_to_bounds(start, monitor_bounds);
    let end = clamp_to_bounds(end, monitor_bounds);
    let left = start.x.min(end.x);
    let top = start.y.min(end.y);
    let right = start.x.max(end.x);
    let bottom = start.y.max(end.y);

    if right <= left || bottom <= top {
        return None;
    }

    let rect = PhysicalRect::new(
        left - monitor_bounds.origin.x,
        top - monitor_bounds.origin.y,
        (right - left) as u32,
        (bottom - top) as u32,
    );
    Some(CaptureRegion::new(monitor, rect))
}

fn clamp_to_bounds(point: PhysicalPoint, bounds: PhysicalRect) -> PhysicalPoint {
    // Region edges are exclusive, so the far edge may equal right()/bottom().
    let max_x = bounds.right();
    let max_y = bounds.bottom();
    PhysicalPoint::new(
        i64::from(point.x).clamp(i64::from(bounds.origin.x), max_x) as i32,
        i64::from(point.y).clamp(i64::from(bounds.origin.y), max_y) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, x: i32, y: i32, width: u32, height: u32) -> WindowInfo {
        WindowInfo {
            id: WindowId::from_raw(id),
            bounds: PhysicalRect::new(x, y, width, height),
            title: String::new(),
            monitor: None,
            scale: core_types::DpiScale::ONE,
        }
    }

    fn session() -> SelectionSession {
        SelectionSession::new(
            MonitorId::from_raw(1),
            PhysicalRect::new(0, 0, 1000, 800),
            vec![
                window(10, 100, 100, 300, 200),
                window(20, 200, 150, 300, 200),
            ],
        )
    }

    #[test]
    fn hover_picks_the_topmost_window_when_targeting() {
        let mut session = session();
        session.set_window_targeting(true);
        session.pointer_moved(PhysicalPoint::new(250, 200));
        assert_eq!(
            session.hovered_window().map(|window| window.id),
            Some(WindowId::from_raw(10))
        );
    }

    #[test]
    fn hover_is_empty_without_window_targeting() {
        let mut session = session();
        session.pointer_moved(PhysicalPoint::new(250, 200));
        assert_eq!(session.hovered_window(), None);
    }

    #[test]
    fn targeted_click_on_a_window_captures_that_window() {
        let mut session = session();
        session.set_window_targeting(true);
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(122, 121)),
            SelectionAction::Capture(SelectionOutcome::Window(WindowId::from_raw(10)))
        );
    }

    #[test]
    fn plain_click_on_a_window_is_ignored() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(122, 121)),
            SelectionAction::None
        );
    }

    #[test]
    fn click_on_a_hovered_element_captures_its_region() {
        let mut session = session();
        session.set_window_targeting(true);
        let element = PhysicalRect::new(110, 110, 120, 40);
        session.set_hovered_element(Some(element));
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        let action = session.pointer_released(PhysicalPoint::new(120, 120));
        let SelectionAction::Capture(SelectionOutcome::Region(region)) = action else {
            panic!("expected an element region");
        };
        assert_eq!(region.rect, PhysicalRect::new(110, 110, 120, 40));
    }

    #[test]
    fn click_on_empty_desktop_captures_the_monitor() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(600, 600));
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(600, 600)),
            SelectionAction::Capture(SelectionOutcome::Monitor(MonitorId::from_raw(1)))
        );
    }

    #[test]
    fn square_ratio_locks_drag_geometry() {
        let mut session = session();
        session.set_aspect_ratio(Some(1.0));
        session.pointer_pressed(PhysicalPoint::new(100, 100));
        session.pointer_moved(PhysicalPoint::new(300, 150));

        let rect = session.selection_rect().expect("drag is active");
        assert_eq!(rect.size.width, rect.size.height);
        assert!(rect.size.width >= 200);

        let action = session.pointer_released(PhysicalPoint::new(300, 150));
        let SelectionAction::Capture(SelectionOutcome::Region(region)) = action else {
            panic!("expected a region");
        };
        assert_eq!(region.rect.size.width, region.rect.size.height);
    }

    #[test]
    fn ratio_is_clamped_to_the_monitor() {
        let mut session = session();
        session.set_aspect_ratio(Some(16.0 / 9.0));
        session.pointer_pressed(PhysicalPoint::new(900, 700));
        session.pointer_moved(PhysicalPoint::new(1200, 1200));

        let rect = session.selection_rect().expect("drag is active");
        assert!(rect.right() <= 1000);
        assert!(rect.bottom() <= 800);
        let expected = rect.size.width as f32 / rect.size.height as f32;
        assert!((expected - 16.0 / 9.0).abs() < 0.05);
    }

    #[test]
    fn a_short_drag_with_window_targeting_is_still_a_click() {
        let mut session = session();
        session.set_window_targeting(true);
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        session.pointer_moved(PhysicalPoint::new(123, 120));
        assert!(!session.is_dragging());
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(123, 120)),
            SelectionAction::Capture(SelectionOutcome::Window(WindowId::from_raw(10)))
        );
    }

    #[test]
    fn a_drag_past_the_threshold_becomes_a_region() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        session.pointer_moved(PhysicalPoint::new(400, 500));
        assert!(session.is_dragging());
        assert_eq!(session.hovered_window().map(|window| window.id), None);
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(400, 500)),
            SelectionAction::Capture(SelectionOutcome::Region(CaptureRegion::new(
                MonitorId::from_raw(1),
                PhysicalRect::new(120, 120, 280, 380)
            )))
        );
    }

    #[test]
    fn dragging_backwards_normalizes_the_region() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(400, 500));
        session.pointer_moved(PhysicalPoint::new(120, 120));
        assert_eq!(
            session.selection_rect(),
            Some(PhysicalRect::new(120, 120, 280, 380))
        );
    }

    #[test]
    fn region_is_clamped_to_the_monitor() {
        let mut session = SelectionSession::new(
            MonitorId::from_raw(2),
            PhysicalRect::new(-1920, 0, 1920, 1080),
            Vec::new(),
        );
        session.pointer_pressed(PhysicalPoint::new(-1900, 100));
        session.pointer_moved(PhysicalPoint::new(100, 2000));
        assert_eq!(
            session.selection_rect(),
            Some(PhysicalRect::new(20, 100, 1900, 980))
        );
    }

    #[test]
    fn region_uses_monitor_local_coordinates() {
        let mut session = SelectionSession::new(
            MonitorId::from_raw(3),
            PhysicalRect::new(-1920, 0, 1920, 1080),
            Vec::new(),
        );
        session.pointer_pressed(PhysicalPoint::new(-1800, 100));
        session.pointer_moved(PhysicalPoint::new(-1000, 600));
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(-1000, 600)),
            SelectionAction::Capture(SelectionOutcome::Region(CaptureRegion::new(
                MonitorId::from_raw(3),
                PhysicalRect::new(120, 100, 800, 500)
            )))
        );
    }

    #[test]
    fn zero_area_drag_is_ignored() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        session.pointer_moved(PhysicalPoint::new(200, 120));
        assert_eq!(
            session.pointer_released(PhysicalPoint::new(200, 120)),
            SelectionAction::None
        );
    }

    #[test]
    fn windows_from_other_monitors_are_filtered_out() {
        let monitor = MonitorId::from_raw(1);
        let mut other = window(30, 10, 10, 50, 50);
        other.monitor = Some(MonitorId::from_raw(2));
        let mut session = SelectionSession::with_scale(
            monitor,
            PhysicalRect::new(0, 0, 1000, 800),
            vec![window(10, 100, 100, 100, 100), other],
            2.0,
        );
        session.set_window_targeting(true);
        session.pointer_moved(PhysicalPoint::new(20, 20));
        assert_eq!(session.hovered_window(), None);
    }

    #[test]
    fn a_new_press_replaces_a_stale_drag() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        session.pointer_moved(PhysicalPoint::new(400, 500));
        assert!(session.has_drag());

        session.pointer_pressed(PhysicalPoint::new(600, 600));
        let action = session.pointer_released(PhysicalPoint::new(600, 600));
        assert_eq!(
            action,
            SelectionAction::Capture(SelectionOutcome::Monitor(MonitorId::from_raw(1)))
        );
    }

    #[test]
    fn drag_threshold_scales_with_the_monitor() {
        let mut session = SelectionSession::with_scale(
            MonitorId::from_raw(1),
            PhysicalRect::new(0, 0, 1000, 800),
            Vec::new(),
            2.0,
        );
        session.pointer_pressed(PhysicalPoint::new(100, 100));
        session.pointer_moved(PhysicalPoint::new(105, 100));
        assert!(!session.is_dragging());
        session.pointer_moved(PhysicalPoint::new(110, 100));
        assert!(session.is_dragging());
    }

    #[test]
    fn element_candidates_prefer_the_smallest_nested_rect() {
        let window = PhysicalRect::new(0, 0, 1000, 800);
        let large = PhysicalRect::new(100, 100, 500, 400);
        let small = PhysicalRect::new(150, 150, 200, 100);
        let outside = PhysicalRect::new(900, 700, 200, 100);
        let candidates = element_candidates(
            &[large, small, outside],
            window,
            PhysicalPoint::new(200, 180),
        );
        assert_eq!(candidates, vec![small, large]);
    }

    #[test]
    fn cancel_clears_hover_and_drag() {
        let mut session = session();
        session.pointer_pressed(PhysicalPoint::new(120, 120));
        session.pointer_moved(PhysicalPoint::new(400, 500));
        session.cancel();
        assert!(!session.is_dragging());
        assert_eq!(session.hovered_window().map(|window| window.id), None);
        assert_eq!(session.selection_rect(), None);
    }
}
