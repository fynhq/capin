use core_capture::CaptureTarget;

use super::EditorView;
use crate::effects::RegionEffects;

impl EditorView {
    pub(super) fn region_effects_active(&self) -> bool {
        if !self.effects.active() {
            return false;
        }
        match self.inline.and_then(|inline| inline.capture) {
            Some(capture) => !matches!(capture.target, CaptureTarget::Window(_)),
            None => self.pin_target.is_some(),
        }
    }

    pub(super) fn stored_region_effects(&self) -> RegionEffects {
        if self.region_effects_active() {
            self.effects
        } else {
            RegionEffects::default()
        }
    }

    pub(super) fn pin_effects(&self) -> RegionEffects {
        let mut effects = self.stored_region_effects();
        if self.region_pin() {
            effects = effects.with_shadow();
        }
        effects
    }

    pub(super) fn region_pin(&self) -> bool {
        self.pin_shadow_always
            || self
                .inline
                .and_then(|inline| inline.capture)
                .is_some_and(|capture| matches!(capture.target, CaptureTarget::Region(_)))
    }

    pub(super) fn pin_native_shadow(&self) -> bool {
        self.inline.is_some()
            && self.inline.and_then(|inline| inline.capture).is_none()
            && self.pin_target.is_none()
    }

    pub(super) fn region_effects_scale(&self) -> f32 {
        self.inline
            .and_then(|inline| inline.capture)
            .map(|capture| capture.scale)
            .unwrap_or(self.pin_effects_scale)
            .max(0.1)
    }
}
