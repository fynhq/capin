#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::str::FromStr;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use gpui_kit::*;

    use crate::logging::{log_error, log_info, log_warn};
    use crate::settings_state;

    thread_local! {
        static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
        static REGISTERED: RefCell<Vec<HotKey>> = const { RefCell::new(Vec::new()) };
        static CAPTURE_ID: Cell<u32> = const { Cell::new(0) };
        static PIN_ID: Cell<u32> = const { Cell::new(0) };
        static CAPTURE_KEY: RefCell<Option<HotKey>> = const { RefCell::new(None) };
        static PIN_KEY: RefCell<Option<HotKey>> = const { RefCell::new(None) };
        static CAPTURE_ENABLED: Cell<bool> = const { Cell::new(false) };
        static PIN_ENABLED: Cell<bool> = const { Cell::new(false) };
        static RECORD_PAUSE_ID: Cell<u32> = const { Cell::new(0) };
        static RECORD_STOP_ID: Cell<u32> = const { Cell::new(0) };
        static RECORD_PAUSE_KEY: RefCell<Option<HotKey>> = const { RefCell::new(None) };
        static RECORD_STOP_KEY: RefCell<Option<HotKey>> = const { RefCell::new(None) };
        static REGISTRATION_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
        static THROUGH_ESCAPE_ID: Cell<u32> = const { Cell::new(0) };
        static THROUGH_ESCAPE_KEY: RefCell<Option<HotKey>> = const { RefCell::new(None) };
        static THROUGH_ESCAPE_FAILED: Cell<bool> = const { Cell::new(false) };
        static SETTINGS_RECORDING: Cell<bool> = const { Cell::new(false) };
        static SUSPENDED: Cell<bool> = const { Cell::new(false) };
    }

    fn sync_suspension() {
        let wanted = SETTINGS_RECORDING.with(Cell::get);
        if wanted == SUSPENDED.with(Cell::get) {
            return;
        }
        MANAGER.with(|slot| {
            let manager_borrow = slot.borrow();
            let Some(manager) = manager_borrow.as_ref() else {
                return;
            };
            if wanted {
                let registered = REGISTERED.with(|registered| registered.borrow().clone());
                for hotkey in registered {
                    let _ = manager.unregister(hotkey);
                }
                REGISTERED.with(|registered| registered.borrow_mut().clear());
                CAPTURE_ENABLED.with(|enabled| enabled.set(false));
                PIN_ENABLED.with(|enabled| enabled.set(false));
                SUSPENDED.with(|suspended| suspended.set(true));
                log_info!("hotkey", "global shortcuts suspended");
            } else {
                let capture = CAPTURE_KEY
                    .with(|slot| *slot.borrow())
                    .unwrap_or_else(default_capture);
                let pin = PIN_KEY
                    .with(|slot| *slot.borrow())
                    .unwrap_or_else(default_pin);
                let (registered, error) = register_pair(manager, capture, pin);
                CAPTURE_ENABLED.with(|enabled| enabled.set(registered.contains(&capture)));
                PIN_ENABLED.with(|enabled| enabled.set(registered.contains(&pin)));
                REGISTERED.with(|slot| *slot.borrow_mut() = registered);
                REGISTRATION_ERROR.with(|slot| *slot.borrow_mut() = error);
                SUSPENDED.with(|suspended| suspended.set(false));
                log_info!("hotkey", "global shortcuts resumed");
            }
        });
    }

    pub fn set_settings_recording(enabled: bool) {
        SETTINGS_RECORDING.with(|recording| recording.set(enabled));
        sync_suspension();
    }

    fn sync_through_escape(cx: &mut App) {
        let wanted = crate::pin::click_through_count() > 0
            && !crate::overlay::has_open_overlays()
            && !crate::editor::has_open_editors()
            && !crate::pin::is_pin_menu_open()
            && !crate::windows::has_open_settings(cx);
        let active = THROUGH_ESCAPE_ID.with(|id| id.get() != 0);
        if wanted == active {
            return;
        }
        MANAGER.with(|slot| {
            let manager = slot.borrow();
            let Some(manager) = manager.as_ref() else {
                return;
            };
            if wanted {
                if THROUGH_ESCAPE_FAILED.with(|failed| failed.get()) {
                    return;
                }
                let hotkey = HotKey::new(None, Code::Escape);
                match manager.register(hotkey) {
                    Ok(()) => {
                        THROUGH_ESCAPE_ID.with(|id| id.set(hotkey.id()));
                        THROUGH_ESCAPE_KEY.with(|key| *key.borrow_mut() = Some(hotkey));
                        log_info!("hotkey", "registered click-through escape");
                    }
                    Err(error) => {
                        THROUGH_ESCAPE_FAILED.with(|failed| failed.set(true));
                        log_warn!("hotkey", "failed to register click-through escape: {error}");
                    }
                }
            } else {
                if let Some(hotkey) = THROUGH_ESCAPE_KEY.with(|key| key.borrow_mut().take()) {
                    let _ = manager.unregister(hotkey);
                }
                THROUGH_ESCAPE_ID.with(|id| id.set(0));
                THROUGH_ESCAPE_FAILED.with(|failed| failed.set(false));
            }
        });
    }

    fn fallback_modifiers() -> Modifiers {
        #[cfg(target_os = "windows")]
        {
            Modifiers::CONTROL | Modifiers::ALT
        }
        #[cfg(not(target_os = "windows"))]
        {
            Modifiers::META | Modifiers::CONTROL
        }
    }

    fn default_capture() -> HotKey {
        settings_state::DEFAULT_CAPTURE_HOTKEY
            .parse()
            .unwrap_or_else(|_| HotKey::new(Some(fallback_modifiers()), Code::KeyS))
    }

    fn default_pin() -> HotKey {
        settings_state::DEFAULT_PIN_HOTKEY
            .parse()
            .unwrap_or_else(|_| HotKey::new(Some(fallback_modifiers()), Code::KeyD))
    }

    fn parse_or_default(value: &str, default: HotKey) -> HotKey {
        value.parse().unwrap_or(default)
    }

    fn register_pair(
        manager: &GlobalHotKeyManager,
        capture: HotKey,
        pin: HotKey,
    ) -> (Vec<HotKey>, Option<String>) {
        let mut registered = Vec::new();
        let mut errors = Vec::new();
        for (label, hotkey) in [("capture", capture), ("pin", pin)] {
            match manager.register(hotkey) {
                Ok(()) => {
                    log_info!("hotkey", "registered {label}: {}", hotkey.into_string());
                    registered.push(hotkey);
                }
                Err(error) => {
                    log_error!(
                        "hotkey",
                        "failed to register {label} {}: {error}",
                        hotkey.into_string()
                    );
                    errors.push(format!("{label}: {error}"));
                }
            }
        }
        let error = (!errors.is_empty()).then(|| errors.join("; "));
        (registered, error)
    }

    pub fn install(cx: &mut App) {
        if MANAGER.with(|slot| slot.borrow().is_some()) {
            return;
        }

        let settings = settings_state::get();
        let capture = parse_or_default(&settings.capture_hotkey, default_capture());
        let pin = parse_or_default(&settings.pin_hotkey, default_pin());
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(error) => {
                let message = error.to_string();
                REGISTRATION_ERROR.with(|slot| *slot.borrow_mut() = Some(message.clone()));
                log_error!(
                    "hotkey",
                    "failed to create global hotkey manager: {message}"
                );
                return;
            }
        };
        let (registered, error) = register_pair(&manager, capture, pin);
        REGISTRATION_ERROR.with(|slot| *slot.borrow_mut() = error);
        CAPTURE_KEY.with(|slot| *slot.borrow_mut() = Some(capture));
        PIN_KEY.with(|slot| *slot.borrow_mut() = Some(pin));
        CAPTURE_ENABLED.with(|enabled| enabled.set(registered.contains(&capture)));
        PIN_ENABLED.with(|enabled| enabled.set(registered.contains(&pin)));
        CAPTURE_ID.with(|id| id.set(capture.id()));
        PIN_ID.with(|id| id.set(pin.id()));
        MANAGER.with(|slot| *slot.borrow_mut() = Some(manager));
        REGISTERED.with(|slot| *slot.borrow_mut() = registered);
        sync_suspension();

        let (sender, receiver) = mpsc::channel::<u32>();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                let _ = sender.send(event.id());
            }
        }));
        let receiver = Arc::new(Mutex::new(receiver));
        cx.spawn(async move |cx| {
            loop {
                let receiver = Arc::clone(&receiver);
                let id = cx
                    .background_executor()
                    .spawn(async move {
                        receiver.lock().ok().and_then(|receiver| {
                            receiver.recv_timeout(Duration::from_millis(250)).ok()
                        })
                    })
                    .await;
                match id {
                    Some(id) => cx.update(|cx| handle_event(id, cx)),
                    None => cx.update(sync_through_escape),
                }
            }
        })
        .detach();
    }

    fn handle_event(id: u32, cx: &mut App) {
        let capture_id = CAPTURE_ID.with(|id| id.get());
        let pin_id = PIN_ID.with(|id| id.get());
        let record_pause_id = RECORD_PAUSE_ID.with(|id| id.get());
        let record_stop_id = RECORD_STOP_ID.with(|id| id.get());
        let through_escape_id = THROUGH_ESCAPE_ID.with(|id| id.get());
        let capture_enabled = CAPTURE_ENABLED.with(|enabled| enabled.get());
        let pin_enabled = PIN_ENABLED.with(|enabled| enabled.get());
        if capture_enabled && id == capture_id {
            log_info!("hotkey", "capture shortcut pressed");
            if crate::overlay::has_open_overlays() {
                crate::overlay::close_open_overlays(cx);
            } else {
                crate::overlay::open_overlays(cx);
            }
        } else if pin_enabled && id == pin_id {
            log_info!("hotkey", "pin shortcut pressed");
            if crate::overlay::has_open_overlays() {
                let _ = crate::overlay::pin_current(cx);
            } else if !crate::editor::pin_current(cx) {
                crate::pin::pin_last(cx);
            }
        } else if through_escape_id != 0 && id == through_escape_id {
            log_info!("hotkey", "click-through escape pressed");
            crate::pin::release_click_through(cx);
        } else if record_pause_id != 0 && id == record_pause_id {
            log_info!("hotkey", "recording pause shortcut pressed");
            crate::record::handle_shortcut(crate::record::RecordShortcut::Pause, cx);
        } else if record_stop_id != 0 && id == record_stop_id {
            log_info!("hotkey", "recording stop shortcut pressed");
            crate::record::handle_shortcut(crate::record::RecordShortcut::Stop, cx);
        }
    }

    pub fn registration_error() -> Option<String> {
        REGISTRATION_ERROR.with(|slot| slot.borrow().clone())
    }

    pub fn refresh() -> Result<(), String> {
        let settings = settings_state::get();
        let capture = parse_or_default(&settings.capture_hotkey, default_capture());
        let pin = parse_or_default(&settings.pin_hotkey, default_pin());
        if capture.id() == pin.id() {
            return Err("shortcuts must differ".to_owned());
        }

        if SUSPENDED.with(Cell::get) {
            CAPTURE_KEY.with(|slot| *slot.borrow_mut() = Some(capture));
            PIN_KEY.with(|slot| *slot.borrow_mut() = Some(pin));
            CAPTURE_ID.with(|id| id.set(capture.id()));
            PIN_ID.with(|id| id.set(pin.id()));
            REGISTRATION_ERROR.with(|slot| *slot.borrow_mut() = None);
            return Ok(());
        }

        let result = MANAGER.with(|slot| {
            let manager_borrow = slot.borrow();
            let Some(manager) = manager_borrow.as_ref() else {
                return Err("hotkey manager is unavailable".to_owned());
            };
            let previous = REGISTERED.with(|registered| registered.borrow().clone());
            let previous_capture = CAPTURE_KEY
                .with(|slot| *slot.borrow())
                .unwrap_or_else(default_capture);
            let previous_pin = PIN_KEY
                .with(|slot| *slot.borrow())
                .unwrap_or_else(default_pin);
            if let Err(error) = manager.unregister_all(&previous) {
                log_warn!("hotkey", "failed to unregister previous shortcuts: {error}");
            }
            let (registered, error) = register_pair(manager, capture, pin);
            if let Some(error) = error {
                for hotkey in &registered {
                    let _ = manager.unregister(*hotkey);
                }
                let (restored, restore_error) =
                    register_pair(manager, previous_capture, previous_pin);
                if let Some(restore_error) = restore_error.as_ref() {
                    log_error!(
                        "hotkey",
                        "failed to restore previous shortcuts: {restore_error}"
                    );
                }
                let restored_capture = restored.contains(&previous_capture);
                let restored_pin = restored.contains(&previous_pin);
                REGISTERED.with(|slot| *slot.borrow_mut() = restored);
                CAPTURE_ENABLED.with(|enabled| enabled.set(restored_capture));
                PIN_ENABLED.with(|enabled| enabled.set(restored_pin));
                CAPTURE_ID.with(|id| id.set(previous_capture.id()));
                PIN_ID.with(|id| id.set(previous_pin.id()));
                REGISTRATION_ERROR.with(|slot| *slot.borrow_mut() = restore_error);
                log_error!("hotkey", "failed to apply new shortcuts: {error}");
                return Err(error);
            }
            REGISTERED.with(|slot| *slot.borrow_mut() = registered);
            CAPTURE_KEY.with(|slot| *slot.borrow_mut() = Some(capture));
            PIN_KEY.with(|slot| *slot.borrow_mut() = Some(pin));
            CAPTURE_ENABLED.with(|enabled| enabled.set(true));
            PIN_ENABLED.with(|enabled| enabled.set(true));
            CAPTURE_ID.with(|id| id.set(capture.id()));
            PIN_ID.with(|id| id.set(pin.id()));
            REGISTRATION_ERROR.with(|slot| *slot.borrow_mut() = None);
            log_info!(
                "hotkey",
                "shortcuts updated: capture={} pin={}",
                capture.into_string(),
                pin.into_string()
            );
            Ok(())
        });
        if result.is_ok() {
            sync_suspension();
        }
        result
    }

    pub fn set_recording_shortcuts(enabled: bool) -> bool {
        MANAGER.with(|slot| {
            let manager_borrow = slot.borrow();
            let Some(manager) = manager_borrow.as_ref() else {
                return false;
            };
            if enabled {
                if RECORD_PAUSE_ID.with(|id| id.get()) != 0 {
                    return true;
                }
                let pause = HotKey::new(None, Code::F9);
                let stop = HotKey::new(None, Code::F10);
                if manager.register(pause).is_err() {
                    return false;
                }
                if manager.register(stop).is_err() {
                    let _ = manager.unregister(pause);
                    return false;
                }
                RECORD_PAUSE_ID.with(|id| id.set(pause.id()));
                RECORD_STOP_ID.with(|id| id.set(stop.id()));
                RECORD_PAUSE_KEY.with(|key| *key.borrow_mut() = Some(pause));
                RECORD_STOP_KEY.with(|key| *key.borrow_mut() = Some(stop));
                true
            } else {
                if let Some(key) = RECORD_PAUSE_KEY.with(|key| key.borrow_mut().take()) {
                    let _ = manager.unregister(key);
                }
                if let Some(key) = RECORD_STOP_KEY.with(|key| key.borrow_mut().take()) {
                    let _ = manager.unregister(key);
                }
                RECORD_PAUSE_ID.with(|id| id.set(0));
                RECORD_STOP_ID.with(|id| id.set(0));
                true
            }
        })
    }

    pub fn shortcut_from_key(
        key: &str,
        control: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> Option<String> {
        let mut mods = Modifiers::empty();
        if control {
            mods |= Modifiers::CONTROL;
        }
        if alt {
            mods |= Modifiers::ALT;
        }
        if shift {
            mods |= Modifiers::SHIFT;
        }
        if meta {
            mods |= Modifiers::META;
        }
        if !mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META) {
            return None;
        }
        let probe: HotKey = format!("control+{key}").parse().ok()?;
        Some(HotKey::new(Some(mods), probe.key).into_string())
    }

    pub fn display_hotkey(value: &str) -> String {
        let Ok(hotkey) = HotKey::from_str(value) else {
            return value.to_owned();
        };
        let mut parts = Vec::new();
        if hotkey.mods.contains(Modifiers::CONTROL) {
            parts.push("Ctrl".to_owned());
        }
        if hotkey.mods.contains(Modifiers::META) {
            parts.push(
                if cfg!(target_os = "macos") {
                    "Cmd"
                } else {
                    "Win"
                }
                .to_owned(),
            );
        }
        if hotkey.mods.contains(Modifiers::ALT) {
            parts.push(
                if cfg!(target_os = "macos") {
                    "Option"
                } else {
                    "Alt"
                }
                .to_owned(),
            );
        }
        if hotkey.mods.contains(Modifiers::SHIFT) {
            parts.push("Shift".to_owned());
        }
        parts.push(display_key(hotkey.key));
        parts.join("+")
    }

    fn display_key(code: Code) -> String {
        let text = format!("{code:?}");
        if let Some(rest) = text.strip_prefix("Key") {
            return rest.to_owned();
        }
        if let Some(rest) = text.strip_prefix("Digit") {
            return rest.to_owned();
        }
        match code {
            Code::Comma => ",".to_owned(),
            Code::Period => ".".to_owned(),
            Code::Slash => "/".to_owned(),
            Code::Semicolon => ";".to_owned(),
            Code::Quote => "'".to_owned(),
            Code::Minus => "-".to_owned(),
            Code::Equal => "=".to_owned(),
            _ => text,
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use imp::{
    display_hotkey, install, refresh, registration_error, set_recording_shortcuts,
    set_settings_recording, shortcut_from_key,
};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn install(_cx: &mut gpui_kit::App) {}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn refresh() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn registration_error() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn display_hotkey(value: &str) -> String {
    value.to_owned()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn shortcut_from_key(
    _key: &str,
    _control: bool,
    _alt: bool,
    _shift: bool,
    _meta: bool,
) -> Option<String> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn set_recording_shortcuts(_enabled: bool) -> bool {
    false
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn set_settings_recording(_enabled: bool) {}
