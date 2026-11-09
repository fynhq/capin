#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::str::FromStr;
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
        static REGISTRATION_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    fn fallback_modifiers() -> Modifiers {
        #[cfg(target_os = "windows")]
        {
            Modifiers::CONTROL | Modifiers::ALT
        }
        #[cfg(not(target_os = "windows"))]
        {
            Modifiers::META | Modifiers::SHIFT
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

    /// Registers each shortcut independently: a system-owned combination such
    /// as Win+Shift+S must not take the other shortcut down with it.
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

        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let mut actions = Vec::new();
                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state() == HotKeyState::Pressed {
                        actions.push(event.id());
                    }
                }
                if actions.is_empty() {
                    continue;
                }
                cx.update(|cx| {
                    let capture_id = CAPTURE_ID.with(|id| id.get());
                    let pin_id = PIN_ID.with(|id| id.get());
                    let capture_enabled = CAPTURE_ENABLED.with(|enabled| enabled.get());
                    let pin_enabled = PIN_ENABLED.with(|enabled| enabled.get());
                    for id in actions {
                        if capture_enabled && id == capture_id {
                            log_info!("hotkey", "capture shortcut pressed");
                            if crate::overlay::has_open_overlays() {
                                crate::overlay::close_open_overlays(cx);
                            } else {
                                crate::tray::replace_windows(cx, crate::overlay::open_overlays);
                            }
                        } else if pin_enabled && id == pin_id {
                            log_info!("hotkey", "pin shortcut pressed");
                            crate::pin::pin_last(cx);
                        }
                    }
                });
            }
        })
        .detach();
    }

    /// The latest registration failure, surfaced in the shortcuts page and
    /// logs. `None` means every configured shortcut is active.
    pub fn registration_error() -> Option<String> {
        REGISTRATION_ERROR.with(|slot| slot.borrow().clone())
    }

    /// Re-registers the global hotkeys after a settings change.
    pub fn refresh() -> Result<(), String> {
        let settings = settings_state::get();
        let capture = parse_or_default(&settings.capture_hotkey, default_capture());
        let pin = parse_or_default(&settings.pin_hotkey, default_pin());
        if capture.id() == pin.id() {
            return Err("shortcuts must differ".to_owned());
        }

        MANAGER.with(|slot| {
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
                // Roll back to the last working pair so a rejected combination
                // leaves the app usable and the settings page unchanged.
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
                // The failed combination is reverted, so the page warning must
                // describe the shortcuts that are actually active now.
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
        })
    }

    /// Converts a captured key plus modifier flags into the persisted form.
    /// Returns `None` when the key is unknown or only Shift is held.
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
pub use imp::{display_hotkey, install, refresh, registration_error, shortcut_from_key};

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
