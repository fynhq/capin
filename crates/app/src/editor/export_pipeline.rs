use std::time::Instant;

use core_capture::CaptureTarget;

use super::*;

use crate::logging::log_debug;

pub(super) fn region_effects_active(view: &EditorView) -> bool {
    if !view.effects.active() {
        return false;
    }
    match view.inline.and_then(|inline| inline.capture) {
        Some(capture) => !matches!(capture.target, CaptureTarget::Window(_)),
        None => view.pin_target.is_some(),
    }
}

pub(super) fn stored_region_effects(view: &EditorView) -> RegionEffects {
    if region_effects_active(view) {
        view.effects
    } else {
        RegionEffects::default()
    }
}

pub(super) fn pin_effects(view: &EditorView) -> RegionEffects {
    let mut effects = stored_region_effects(view);
    if region_pin(view) {
        effects = effects.with_shadow();
    }
    effects
}

pub(super) fn region_pin(view: &EditorView) -> bool {
    view.pin_shadow_always
        || view
            .inline
            .and_then(|inline| inline.capture)
            .is_some_and(|capture| matches!(capture.target, CaptureTarget::Region(_)))
}

pub(super) fn pin_native_shadow(view: &EditorView) -> bool {
    view.inline.is_some()
        && view.inline.and_then(|inline| inline.capture).is_none()
        && view.pin_target.is_none()
}

pub(super) fn region_effects_scale(view: &EditorView) -> f32 {
    view.inline
        .and_then(|inline| inline.capture)
        .map(|capture| capture.scale)
        .unwrap_or(view.pin_effects_scale)
        .max(0.1)
}

pub(super) fn render_export(
    view: &EditorView,
    with_pin_output: bool,
) -> Result<RenderedExport, core_annotate::RasterError> {
    let started = Instant::now();
    let document = view.action_document();
    let raw = rasterize(&document, &view.frame)?;
    let scale = region_effects_scale(view);
    let pin_effects = pin_effects(view);
    let pin_padding = pin_effects.padding(scale);
    let effects_active = region_effects_active(view);
    let (output, pin_output) = if with_pin_output {
        let copy = if effects_active {
            view.effects.apply(raw.clone(), scale)
        } else {
            raw.clone()
        };
        let pin = if pin_effects.active() {
            pin_effects.apply(raw, scale)
        } else {
            raw
        };
        (copy, Some(pin))
    } else {
        let copy = if effects_active {
            view.effects.apply(raw, scale)
        } else {
            raw
        };
        (copy, None)
    };
    let hash = crate::capture_history::hash_frame(&output);
    let record_history = view.pin_target.is_none();
    let pin_shadow = (record_history && region_pin(view) && !view.effects.shadow).then_some(
        crate::capture_history::HistoryPinShadow {
            corner_radius: view.effects.corner_radius,
            scale,
        },
    );
    log_debug!(
        "editor",
        "render_export with_pin={with_pin_output} duration_ms={} size={}x{}",
        started.elapsed().as_millis(),
        output.width(),
        output.height()
    );
    Ok(RenderedExport {
        document,
        output,
        pin_output,
        hash,
        record_history,
        pin_shadow,
        scale,
        pin_padding,
        stored_effects: stored_region_effects(view),
        pin_shadow_always: region_pin(view),
    })
}
