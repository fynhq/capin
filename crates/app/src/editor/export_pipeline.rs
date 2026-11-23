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

pub(super) struct ExportInputs {
    pub(super) document: AnnotationDocument,
    pub(super) frame: Arc<RgbaFrame>,
    pub(super) effects: RegionEffects,
    pub(super) effects_active: bool,
    pub(super) pin_effects: RegionEffects,
    pub(super) scale: f32,
    pub(super) record_history: bool,
    pub(super) pin_shadow_always: bool,
    pub(super) region_pin: bool,
}

pub(super) fn export_inputs(view: &EditorView) -> ExportInputs {
    ExportInputs {
        document: view.action_document(),
        frame: view.frame.clone(),
        effects: view.effects,
        effects_active: region_effects_active(view),
        pin_effects: pin_effects(view),
        scale: region_effects_scale(view),
        record_history: view.pin_target.is_none(),
        pin_shadow_always: region_pin(view),
        region_pin: region_pin(view),
    }
}

pub(super) fn render_inputs(
    inputs: &ExportInputs,
    with_pin_output: bool,
) -> Result<RenderedExport, core_annotate::RasterError> {
    let started = Instant::now();
    let raw = rasterize(&inputs.document, &inputs.frame)?;
    let scale = inputs.scale;
    let pin_effects = inputs.pin_effects;
    let pin_padding = pin_effects.padding(scale);
    let effects_active = inputs.effects_active;
    let (output, pin_output) = if with_pin_output {
        let copy = if effects_active {
            inputs.effects.apply(raw.clone(), scale)
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
            inputs.effects.apply(raw, scale)
        } else {
            raw
        };
        (copy, None)
    };
    let hash = crate::capture_history::hash_frame(&output);
    let pin_shadow = (inputs.record_history && inputs.region_pin && !inputs.effects.shadow)
        .then_some(crate::capture_history::HistoryPinShadow {
            corner_radius: inputs.effects.corner_radius,
            scale,
        });
    log_debug!(
        "editor",
        "render_export with_pin={with_pin_output} duration_ms={} size={}x{}",
        started.elapsed().as_millis(),
        output.width(),
        output.height()
    );
    Ok(RenderedExport {
        document: inputs.document.clone(),
        output,
        pin_output,
        hash,
        record_history: inputs.record_history,
        pin_shadow,
        scale,
        pin_padding,
        stored_effects: if effects_active {
            inputs.effects
        } else {
            RegionEffects::default()
        },
        pin_shadow_always: inputs.pin_shadow_always,
    })
}

pub(super) fn render_export(
    view: &EditorView,
    with_pin_output: bool,
) -> Result<RenderedExport, core_annotate::RasterError> {
    render_inputs(&export_inputs(view), with_pin_output)
}
