use std::time::Instant;

use super::*;

use crate::logging::log_debug;

pub(crate) fn restore(cx: &mut App) {
    if PINS.with(|pins| !pins.borrow().is_empty()) {
        return;
    }
    let Some(directory) = crate::paths::pins_dir() else {
        return;
    };
    let entries = manifest::load(&directory);
    if entries.is_empty() {
        return;
    }

    let mut deferred = Vec::new();
    let mut restored = 0usize;
    for entry in entries {
        if restored >= MAX_RESTORED_PINS {
            if manifest::is_usable(&entry, &directory) {
                deferred.push(entry);
            }
            continue;
        }
        if !entry.x.is_finite()
            || !entry.y.is_finite()
            || !entry.width.is_finite()
            || !entry.height.is_finite()
            || entry.width < 1.0
            || entry.height < 1.0
        {
            continue;
        }
        let content = match entry.kind.as_str() {
            "image" => {
                let Some(name) = entry
                    .image
                    .as_deref()
                    .and_then(|name| Path::new(name).file_name().map(|name| name.to_owned()))
                else {
                    continue;
                };
                let path = directory.join(name);
                let Some(frame) = image_util::load_frame(&path) else {
                    continue;
                };
                PinContent::from_frame(frame)
            }
            "text" => {
                let Some(text) = entry.text.filter(|text| !text.is_empty()) else {
                    continue;
                };
                PinContent::Text(text)
            }
            _ => continue,
        };
        let zoom = entry.zoom.clamp(MIN_PIN_ZOOM, MAX_PIN_ZOOM);
        let (width, height) = content.size(zoom);
        let display_id = saved_display_id(cx, entry.display);
        let bounds = clamp_to_display(
            cx,
            Bounds {
                origin: point(px(entry.x), px(entry.y)),
                size: size(px(width), px(height)),
            },
            display_id,
        );
        let existing_image = match &content {
            PinContent::Image { .. } => entry
                .image
                .as_deref()
                .and_then(|name| Path::new(name).file_name())
                .map(|name| directory.join(name)),
            _ => None,
        };
        let id = open_pin_inner(
            content,
            Some(entry.id),
            bounds,
            zoom,
            entry.opacity.clamp(0.2, 1.0),
            entry.click_through,
            existing_image,
            false,
            entry.shadow,
            display_id,
            cx,
        );
        if let Some(id) = id {
            restored += 1;
            set_export_padding(id, entry.export_padding);
        }
    }
    DEFERRED_PINS.with(|pins| *pins.borrow_mut() = deferred);
    crate::pin::refresh_clipboard_signatures(cx);
    save_manifest();
}

pub(crate) fn set_edit_state(id: u64, edit_state: PinEditState) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.edit_state = Some(edit_state);
        }
    });
}

pub(crate) fn set_export_padding(id: u64, padding: u32) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.export_padding = padding;
        }
    });
}

pub(crate) fn replace_image_with_edit_state(
    id: u64,
    frame: RgbaFrame,
    edit_state: PinEditState,
    cx: &mut App,
) -> bool {
    replace_image_inner(id, frame, Some(edit_state), cx)
}

fn replace_image_inner(
    id: u64,
    frame: RgbaFrame,
    edit_state: Option<PinEditState>,
    cx: &mut App,
) -> bool {
    let hash = crate::capture_history::hash_frame(&frame);
    let state = PINS.with(|pins| {
        let pins = pins.borrow();
        let state = pins.get(&id)?;
        let zoom = state.values.borrow().zoom;
        Some((
            state.entity.clone(),
            state.window,
            state.image_path.clone(),
            zoom,
            state.bounds.origin,
        ))
    });
    let Some((entity, window_handle, image_path, zoom, origin)) = state else {
        return false;
    };
    let path = if let Some(path) = image_path {
        if !write_pin_image(&path, &frame) {
            return false;
        }
        Some(path)
    } else {
        save_pin_image(id, &frame)
    };
    let width = frame.width() as f32 * zoom;
    let height = frame.height() as f32 * zoom;
    entity.update(cx, |view, cx| {
        view.replace_image(Arc::new(frame), hash, cx);
    });
    let _ = window_handle.update(cx, |_, window, _| {
        window.resize(size(px(width), px(height)));
    });
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.kind = PinKind::Image;
            state.content_hash = Some(hash);
            state.image_path = path;
            state.bounds = Bounds {
                origin,
                size: size(px(width), px(height)),
            };
            state.edit_state = edit_state;
        }
    });
    save_manifest();
    true
}

pub(crate) fn close_pin(id: u64, cx: &mut App) {
    close_pin_inner(id, cx, true);
}

pub(super) fn close_pin_inner(id: u64, cx: &mut App, remove_window: bool) {
    pin_help::close(cx);
    close_pin_menu(cx);
    let state = PINS.with(|pins| pins.borrow_mut().remove(&id));
    let Some(state) = state else {
        return;
    };
    state.entity.update(cx, |view, cx| view.release_image(cx));
    if remove_window {
        let _ = state
            .window
            .update(cx, |_, window, _| window.remove_window());
    }
    save_manifest();
    if let Some(path) = state.image_path {
        let _ = std::fs::remove_file(path);
    }
    crate::windows::schedule_memory_trim();
}

pub(super) fn persist_rotated_image(
    id: u64,
    frame: &RgbaFrame,
    hash: u64,
    bounds: Bounds<Pixels>,
) -> bool {
    let path = PINS.with(|pins| {
        pins.borrow()
            .get(&id)
            .and_then(|state| state.image_path.clone())
    });
    let saved = match path {
        Some(path) => {
            if !write_pin_image(&path, frame) {
                return false;
            }
            Some(path)
        }
        None => save_pin_image(id, frame),
    };
    if saved.is_none() {
        return false;
    }
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.kind = PinKind::Image;
            state.content_hash = Some(hash);
            state.image_path = saved;
            state.bounds = bounds;
        }
    });
    save_manifest();
    true
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn flush_manifest() {
    save_manifest();
}

pub(crate) fn is_pin_window(handle: &AnyWindowHandle) -> bool {
    PINS.with(|pins| {
        pins.borrow()
            .values()
            .any(|state| state.window.window_id() == handle.window_id())
    })
}

pub(crate) fn refresh_recognition(cx: &mut App) {
    let entities: Vec<Entity<PinView>> = PINS.with(|pins| {
        pins.borrow()
            .values()
            .map(|state| state.entity.clone())
            .collect()
    });
    for entity in entities {
        entity.update(cx, |view, cx| {
            view.ocr = None;
            view.selection = None;
            view.selecting = false;
            view.content_generation = view.content_generation.wrapping_add(1);
            view.start_ocr(cx);
            cx.notify();
        });
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn click_through_count() -> usize {
    PINS.with(|pins| {
        pins.borrow()
            .values()
            .filter(|state| state.values.borrow().click_through)
            .count()
    })
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn release_click_through(cx: &mut App) -> usize {
    let states: Vec<(u64, Entity<PinView>, AnyWindowHandle)> = PINS.with(|pins| {
        pins.borrow()
            .iter()
            .filter(|(_, state)| state.values.borrow().click_through)
            .map(|(id, state)| (*id, state.entity.clone(), state.window))
            .collect()
    });
    let count = states.len();
    for (id, entity, window) in states {
        let _ = window.update(cx, |_, window, _| {
            let _ = set_click_through(&*window, false);
        });
        entity.update(cx, |_, cx| cx.notify());
        PINS.with(|pins| {
            if let Some(state) = pins.borrow_mut().get_mut(&id) {
                state.values.borrow_mut().click_through = false;
            }
        });
    }
    save_manifest();
    crate::tray::refresh_menu();
    count
}

pub(super) fn update_bounds(id: u64, bounds: Bounds<Pixels>, display: Option<DisplayId>) -> bool {
    let changed = PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        let Some(state) = pins.get_mut(&id) else {
            return false;
        };
        let changed = state.bounds != bounds || (display.is_some() && state.display != display);
        if state.bounds != bounds {
            state.bounds = bounds;
        }
        if let Some(display) = display {
            state.display = Some(display);
        }
        changed
    });
    if changed {
        save_manifest();
    }
    changed
}

pub(super) fn update_bounds_without_save(
    id: u64,
    bounds: Bounds<Pixels>,
    display: Option<DisplayId>,
) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.bounds = bounds;
            if let Some(display) = display {
                state.display = Some(display);
            }
        }
    });
}

pub(super) fn schedule_manifest_save(cx: &mut Context<PinView>) {
    let revision = MANIFEST_REVISION.with(|revision| {
        let next = revision.get().wrapping_add(1);
        revision.set(next);
        next
    });
    cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        let _ = this.update(cx, |_, _| {
            if MANIFEST_REVISION.with(|current| current.get()) == revision {
                save_manifest();
            }
        });
    })
    .detach();
}

pub(super) fn set_state_click_through(id: u64, enabled: bool) {
    PINS.with(|pins| {
        if let Some(state) = pins.borrow_mut().get_mut(&id) {
            state.values.borrow_mut().click_through = enabled;
        }
    });
    save_manifest();
}

pub(super) fn next_pin_id() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let mut id = millis.max(1);
    PINS.with(|pins| {
        while pins.borrow().contains_key(&id) {
            id += 1;
        }
    });
    id
}

pub(super) fn save_pin_image(id: u64, frame: &RgbaFrame) -> Option<PathBuf> {
    let directory = crate::paths::pins_dir()?;
    let path = directory.join(format!("pin-{id}.png"));
    write_pin_image(&path, frame).then_some(path)
}

pub(super) fn write_pin_image(path: &Path, frame: &RgbaFrame) -> bool {
    let started = Instant::now();
    let Ok(bytes) = crate::image_util::encode_png(frame) else {
        return false;
    };
    let written = crate::persist::write_atomic(path, &bytes).is_ok();
    log_debug!(
        "pin",
        "pin image write bytes={} duration_ms={} ok={written}",
        bytes.len(),
        started.elapsed().as_millis()
    );
    written
}

pub(super) fn save_manifest() {
    let Some(directory) = crate::paths::pins_dir() else {
        return;
    };
    let started = Instant::now();
    let mut entries: Vec<PersistedPin> = PINS.with(|pins| {
        let mut entries: Vec<PersistedPin> = pins
            .borrow()
            .iter()
            .filter_map(|(id, state)| {
                let (kind, image, text) = match &state.kind {
                    PinKind::Image => {
                        let image = state
                            .image_path
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy().to_string())?;
                        ("image", Some(image), None)
                    }
                    PinKind::Text(text) => ("text", None, Some(text.clone())),
                };
                let values = state.values.borrow();
                Some(PersistedPin {
                    id: *id,
                    kind: kind.to_owned(),
                    image,
                    text,
                    x: f32::from(state.bounds.origin.x),
                    y: f32::from(state.bounds.origin.y),
                    width: f32::from(state.bounds.size.width),
                    height: f32::from(state.bounds.size.height),
                    opacity: values.opacity,
                    zoom: values.zoom,
                    click_through: values.click_through,
                    shadow: state.native_shadow,
                    export_padding: state.export_padding,
                    display: state.display.map(u64::from),
                })
            })
            .collect();
        entries.sort_by_key(|entry| entry.id);
        entries
    });
    entries.extend(DEFERRED_PINS.with(|pins| pins.borrow().clone()));
    manifest::enforce_limit(&directory, &mut entries);
    DEFERRED_PINS.with(|pins| {
        pins.borrow_mut()
            .retain(|entry| entries.iter().any(|kept| kept.id == entry.id));
    });
    let count = entries.len();
    manifest::save(&directory, entries);
    log_debug!(
        "pin",
        "manifest write entries={count} duration_ms={}",
        started.elapsed().as_millis()
    );
}
