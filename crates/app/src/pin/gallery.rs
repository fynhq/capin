use super::*;

#[cfg(any(target_os = "windows", target_os = "macos"))]
use super::pin_clipboard::{ClipboardContent, read_clipboard};

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn pin_last(cx: &mut App) {
    let generation = crate::capture_history::generation();
    if PIN_WALK_GENERATION.with(|slot| slot.get()) != Some(generation) {
        LAST_PIN_TRIGGER.with(|slot| slot.set(None));
        PIN_WALK_EXHAUSTED.with(|slot| slot.set(false));
    }
    if PIN_WALK_EXHAUSTED.with(|slot| slot.get()) {
        LAST_PIN_TRIGGER.with(|slot| slot.set(None));
        PIN_WALK_EXHAUSTED.with(|slot| slot.set(false));
    }
    let (content, signature) = read_clipboard(cx);
    match content {
        ClipboardContent::Image(frame) => {
            let previous_clipboard = PIN_CLIPBOARD.get();
            let clipboard_changed = match (previous_clipboard, signature) {
                (Some(previous), Some(current)) => previous != current,
                (None, Some(_)) => true,
                _ => false,
            };
            let walk_active = LAST_PIN_TRIGGER.with(|slot| slot.get()).is_some();
            if walk_active && !clipboard_changed {
                pin_history_step(cx);
                return;
            }
            let mut clipboard_image = crate::capture_history::HistoryImage {
                frame,
                hash: 0,
                pin_shadow: None,
                scale: None,
                native_shadow: false,
            };
            let start_with_clipboard = clipboard_changed
                || previous_clipboard.is_none()
                || !crate::capture_history::has_entries();
            if start_with_clipboard {
                let hash = crate::capture_history::hash_frame(&clipboard_image.frame);
                clipboard_image.hash = hash;
                crate::capture_history::record_frame(
                    &clipboard_image.frame,
                    hash,
                    None,
                    None,
                    false,
                );
                pin_image(clipboard_image, cx);
            } else if let Some(image) = crate::capture_history::latest() {
                pin_image(image, cx);
            }
            return;
        }
        ClipboardContent::Text(text) => {
            let text_changed = match (PIN_CLIPBOARD_TEXT.get(), signature) {
                (Some(previous), Some(current)) => previous != current,
                (None, Some(_)) => true,
                _ => false,
            };
            let walk_active = LAST_PIN_TRIGGER.with(|slot| slot.get()).is_some();
            let hash = text_signature(&text);
            let text_live = PINS.with(|pins| {
                pins.borrow()
                    .values()
                    .any(|state| state.content_hash == Some(hash))
            });
            if walk_active && !text_changed {
                if !text_live {
                    open_text_pin(text, cx);
                    return;
                }
                if pin_history_step(cx) {
                    return;
                }
                open_text_pin(text, cx);
                return;
            }
            let start_with_text = text_changed
                || PIN_CLIPBOARD_TEXT.get().is_none()
                || !crate::capture_history::has_entries();
            if start_with_text {
                if text_live && !text_changed && flash_existing_pin(hash, cx) {
                    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
                    mark_walk();
                    return;
                }
                open_text_pin(text, cx);
            } else if let Some(image) = crate::capture_history::latest() {
                pin_image(image, cx);
            } else {
                open_text_pin(text, cx);
            }
            return;
        }
        ClipboardContent::Empty => {}
    }

    if LAST_PIN_TRIGGER.with(|slot| slot.get()).is_some() {
        pin_history_step(cx);
        return;
    }
    if let Some(image) = crate::capture_history::latest() {
        pin_image(image, cx);
    } else {
        log_warn!("pin", "no clipboard content or recent capture to pin");
    }
}

pub(super) fn open_text_pin(text: String, cx: &mut App) {
    let hash = text_signature(&text);
    if open_pin_content(PinContent::Text(text), cx).is_some() {
        LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
        mark_walk();
    }
}

fn mark_walk() {
    PIN_WALK_GENERATION.with(|slot| slot.set(Some(crate::capture_history::generation())));
    PIN_WALK_EXHAUSTED.with(|slot| slot.set(false));
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn pin_history_step(cx: &mut App) -> bool {
    let Some(current) = LAST_PIN_TRIGGER.with(|slot| slot.get()) else {
        return false;
    };
    let live = PINS.with(|pins| {
        pins.borrow()
            .values()
            .any(|state| state.content_hash == Some(current))
    });
    if !live && let Some(image) = crate::capture_history::find(current) {
        open_history_pin(image, cx);
        mark_walk();
        return true;
    }
    if settings_state::get().repeat_pin == settings_state::RepeatPinBehavior::FlashCurrent
        && flash_existing_pin(current, cx)
    {
        return true;
    }
    let Some(image) = crate::capture_history::cycle(current) else {
        log_info!("pin", "screenshot history has no older entry");
        PIN_WALK_EXHAUSTED.with(|slot| slot.set(true));
        return true;
    };
    let hash = image.hash;
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    if !flash_existing_pin(hash, cx) {
        open_history_pin(image, cx);
    }
    mark_walk();
    true
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn pin_image(image: crate::capture_history::HistoryImage, cx: &mut App) {
    let hash = image.hash;
    let repeated = LAST_PIN_TRIGGER.with(|slot| slot.get() == Some(hash))
        || PINS.with(|pins| {
            pins.borrow()
                .values()
                .any(|state| state.content_hash == Some(hash))
        });
    if repeated {
        let behavior = settings_state::get().repeat_pin;
        if behavior == settings_state::RepeatPinBehavior::RestoreHistory
            && crate::capture_history::contains(hash)
            && let Some(history_image) = crate::capture_history::cycle(hash)
        {
            let history_hash = history_image.hash;
            LAST_PIN_TRIGGER.with(|slot| slot.set(Some(history_hash)));
            if !flash_existing_pin(history_hash, cx) {
                open_history_pin(history_image, cx);
            } else {
                mark_walk();
            }
            return;
        }
        if flash_existing_pin(hash, cx) {
            return;
        }
    }
    open_history_pin(image, cx);
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn flash_existing_pin(hash: u64, cx: &mut App) -> bool {
    let target = PINS.with(|pins| {
        pins.borrow()
            .values()
            .find(|state| state.content_hash == Some(hash))
            .map(|state| (state.entity.clone(), state.window))
    });
    let Some((entity, window)) = target else {
        return false;
    };
    entity.update(cx, |view, cx| view.flash(cx));
    let _ = window.update(cx, |_, window, _| window.activate_window());
    true
}

pub(crate) fn open_frame_with_hash(
    frame: RgbaFrame,
    hash: u64,
    native_shadow: bool,
    cx: &mut App,
) -> Option<u64> {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    let content = PinContent::from_frame_with_hash(frame, hash);
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    let bounds = default_bounds(cx, width, height);
    let id = open_pin_content_at(content, bounds, zoom, None, native_shadow, cx);
    mark_walk();
    id
}

pub(crate) fn open_frame_at_with_hash(
    frame: RgbaFrame,
    bounds: Bounds<Pixels>,
    zoom: f32,
    hash: u64,
    monitor: Option<core_types::MonitorId>,
    native_shadow: bool,
    cx: &mut App,
) -> Option<u64> {
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    let source = monitor.and_then(|id| {
        platform_capture::platform_backend()
            .monitors()
            .ok()?
            .into_iter()
            .find(|monitor| monitor.id == id)
    });
    let bounds = source.as_ref().map_or(bounds, |monitor| {
        crate::display::window_bounds_on_monitor(monitor, bounds)
    });
    let display_id = source
        .as_ref()
        .map(|monitor| DisplayId::new(monitor.id.raw()));
    let id = open_pin_content_at(
        PinContent::from_frame_with_hash(frame, hash),
        bounds,
        zoom,
        display_id,
        native_shadow,
        cx,
    );
    mark_walk();
    id
}

pub(super) fn open_history_pin(image: crate::capture_history::HistoryImage, cx: &mut App) {
    let crate::capture_history::HistoryImage {
        frame,
        hash,
        pin_shadow,
        scale,
        native_shadow,
    } = image;
    LAST_PIN_TRIGGER.with(|slot| slot.set(Some(hash)));
    let zoom = scale.map(|scale| 1.0 / scale.max(0.1));
    let Some(shadow) = pin_shadow else {
        let content = PinContent::from_frame_with_hash(frame, hash);
        if let Some(zoom) = zoom {
            let (width, height) = content.size(zoom);
            let bounds = default_bounds(cx, width, height);
            if open_pin_content_at(content, bounds, zoom, None, native_shadow, cx).is_none() {
                crate::logging::log_warn!("pin", "failed to open a restored pin");
            }
        } else if open_pin_content_with_shadow(content, native_shadow, cx).is_none() {
            crate::logging::log_warn!("pin", "failed to open a restored pin");
        }
        mark_walk();
        return;
    };
    let display_effects = crate::effects::RegionEffects {
        corner_radius: shadow.corner_radius,
        shadow: true,
    };
    let display = display_effects.apply(frame, shadow.scale);
    let display_padding = display_effects.padding(shadow.scale);
    let content = PinContent::from_frame_with_hash(display, hash);
    let id = match zoom {
        Some(zoom) => {
            let (width, height) = content.size(zoom);
            let bounds = default_bounds(cx, width, height);
            open_pin_content_at(content, bounds, zoom, None, false, cx)
        }
        None => open_pin_content_with_shadow(content, false, cx),
    };
    if let Some(id) = id {
        set_export_padding(id, display_padding);
    }
    mark_walk();
}

fn open_pin_content(content: PinContent, cx: &mut App) -> Option<u64> {
    open_pin_content_with_shadow(content, false, cx)
}

fn open_pin_content_with_shadow(
    content: PinContent,
    native_shadow: bool,
    cx: &mut App,
) -> Option<u64> {
    let zoom = content.preferred_zoom();
    let (width, height) = content.size(zoom);
    log_info!("pin", "opening pin {}x{}", width.round(), height.round());
    let bounds = default_bounds(cx, width, height);
    open_pin_inner(
        content,
        None,
        bounds,
        zoom,
        1.0,
        false,
        None,
        true,
        native_shadow,
        None,
        false,
        cx,
    )
}

#[allow(clippy::too_many_arguments)]
fn open_pin_content_at(
    content: PinContent,
    bounds: Bounds<Pixels>,
    zoom: f32,
    display_id: Option<DisplayId>,
    native_shadow: bool,
    cx: &mut App,
) -> Option<u64> {
    let size = bounds.size;
    log_info!(
        "pin",
        "opening pin at {}x{}",
        f32::from(size.width).round(),
        f32::from(size.height).round()
    );
    open_pin_inner(
        content,
        None,
        bounds,
        zoom,
        1.0,
        false,
        None,
        true,
        native_shadow,
        display_id,
        false,
        cx,
    )
}

fn default_bounds(cx: &App, width: f32, height: f32) -> Bounds<Pixels> {
    let mut bounds = centered_bounds(cx, width, height);
    let cascade = PIN_CASCADE.with(|index| {
        let value = index.get();
        index.set((value + 1) % 8);
        value as f32 * 24.0
    });
    bounds.origin = point(bounds.origin.x + px(cascade), bounds.origin.y + px(cascade));
    bounds
}
