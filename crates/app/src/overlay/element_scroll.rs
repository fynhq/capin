use std::time::{Duration, Instant};

use gpui_kit::{Context, ScrollDelta, TouchPhase, Window};

use super::{NarrowTarget, OverlayView, WidenTarget};

const ELEMENT_SCROLL_STEP: f32 = 64.0;
const ELEMENT_SCROLL_COOLDOWN: Duration = Duration::from_millis(120);
const ELEMENT_SCROLL_GESTURE_GAP: Duration = Duration::from_millis(350);

#[derive(Default)]
pub(super) struct ElementScroll {
    accumulator: f32,
    direction: f32,
    ended: bool,
    last_event: Option<Instant>,
    last_step: Option<Instant>,
}

impl OverlayView {
    pub(super) fn on_widen_target(
        &mut self,
        _: &WidenTarget,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_tab_target(1.0, cx);
    }

    pub(super) fn on_narrow_target(
        &mut self,
        _: &NarrowTarget,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_tab_target(-1.0, cx);
    }

    fn adjust_tab_target(&mut self, direction: f32, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.as_ref() {
            if selected.edit.is_some() {
                self.next_edit_field(cx);
            }
            return;
        }
        self.adjust_auto_target(direction);
        cx.notify();
    }

    pub(super) fn adjust_auto_target(&mut self, delta: f32) {
        if delta == 0.0 || self.session.has_drag() {
            return;
        }
        if !self.screen_targeting && self.session.window_under_pointer().is_none() {
            return;
        }
        let max = self.element_probe.candidates.len();
        if self.screen_targeting {
            if delta < 0.0 {
                self.screen_targeting = false;
                self.element_probe.level = max;
            }
        } else if delta > 0.0 {
            if self.element_probe.level < max {
                self.element_probe.level += 1;
            } else {
                self.screen_targeting = true;
            }
        } else {
            self.element_probe.level = self.element_probe.level.saturating_sub(1);
        }
        self.session.set_screen_targeting(self.screen_targeting);
        let selected = if self.screen_targeting {
            None
        } else {
            self.element_probe
                .candidates
                .get(self.element_probe.level)
                .copied()
        };
        self.session.set_hovered_element(selected);
    }
}

impl ElementScroll {
    pub(super) fn reset(&mut self) {
        self.accumulator = 0.0;
        self.direction = 0.0;
        self.ended = false;
    }

    pub(super) fn step(
        &mut self,
        now: Instant,
        delta: ScrollDelta,
        phase: TouchPhase,
    ) -> Option<f32> {
        let gap = self
            .last_event
            .is_some_and(|last| now.saturating_duration_since(last) >= ELEMENT_SCROLL_GESTURE_GAP);
        self.last_event = Some(now);
        match delta {
            ScrollDelta::Lines(delta) => {
                self.reset();
                (delta.y.abs() > f32::EPSILON).then_some(delta.y.signum())
            }
            ScrollDelta::Pixels(delta) => {
                match phase {
                    TouchPhase::Started => {
                        self.reset();
                        return None;
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.reset();
                        self.ended = true;
                        return None;
                    }
                    TouchPhase::Moved => {}
                }
                if self.ended && !gap {
                    return None;
                }
                self.ended = false;
                let delta_y = f32::from(delta.y);
                if delta_y.abs() < f32::EPSILON {
                    return None;
                }
                let direction = delta_y.signum();
                if self.direction != 0.0 && direction != self.direction {
                    self.accumulator = 0.0;
                    self.last_step = None;
                }
                self.direction = direction;
                self.accumulator += delta_y;
                if self.accumulator.abs() < ELEMENT_SCROLL_STEP {
                    return None;
                }
                if self.last_step.is_some_and(|last| {
                    now.saturating_duration_since(last) < ELEMENT_SCROLL_COOLDOWN
                }) {
                    self.accumulator = self
                        .accumulator
                        .clamp(-ELEMENT_SCROLL_STEP, ELEMENT_SCROLL_STEP);
                    return None;
                }
                let step_direction = self.accumulator.signum();
                self.accumulator = 0.0;
                self.last_step = Some(now);
                Some(step_direction)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gpui_kit::{ScrollDelta, TouchPhase, point, px};

    use super::ElementScroll;

    fn pixels(delta: f32) -> ScrollDelta {
        ScrollDelta::Pixels(point(px(0.0), px(delta)))
    }

    #[test]
    fn wheel_notches_step_once_each() {
        let mut scroll = ElementScroll::default();
        let now = Instant::now();
        assert_eq!(
            scroll.step(now, ScrollDelta::Lines(point(0.0, 1.0)), TouchPhase::Moved),
            Some(1.0)
        );
        assert_eq!(
            scroll.step(
                now + Duration::from_millis(10),
                ScrollDelta::Lines(point(0.0, -1.0)),
                TouchPhase::Moved
            ),
            Some(-1.0)
        );
    }

    #[test]
    fn trackpad_scroll_accumulates_before_stepping() {
        let mut scroll = ElementScroll::default();
        let start = Instant::now();
        assert_eq!(scroll.step(start, pixels(0.0), TouchPhase::Started), None);
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(16),
                pixels(20.0),
                TouchPhase::Moved
            ),
            None
        );
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(32),
                pixels(20.0),
                TouchPhase::Moved
            ),
            None
        );
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(48),
                pixels(20.0),
                TouchPhase::Moved
            ),
            None
        );
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(64),
                pixels(20.0),
                TouchPhase::Moved
            ),
            Some(1.0)
        );
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(80),
                pixels(-80.0),
                TouchPhase::Moved
            ),
            Some(-1.0)
        );
    }

    #[test]
    fn trackpad_momentum_after_the_gesture_is_ignored() {
        let mut scroll = ElementScroll::default();
        let start = Instant::now();
        assert_eq!(scroll.step(start, pixels(0.0), TouchPhase::Started), None);
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(16),
                pixels(80.0),
                TouchPhase::Moved
            ),
            Some(1.0)
        );
        assert_eq!(
            scroll.step(
                start + Duration::from_millis(32),
                pixels(0.0),
                TouchPhase::Ended
            ),
            None
        );
        for step in 1..6 {
            assert_eq!(
                scroll.step(
                    start + Duration::from_millis(32 + step * 16),
                    pixels(40.0),
                    TouchPhase::Moved
                ),
                None
            );
        }
    }
}
