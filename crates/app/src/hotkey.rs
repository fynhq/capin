#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::str::FromStr;
    use std::time::Duration;

    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use gpui_kit::*;

    use crate::settings_state;

    thread_local! {
        static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
        static REGISTERED: RefCell<Vec<HotKey>> = const { RefCell::new(Vec::new()) };
        static CAPTURE_ID: Cell<u32> = const { Cell::new(0) };
        static PIN_ID: Cell<u32> = const { Cell::new(0) };
    }

    fn default_capture() -> HotKey {
        HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyS)
    }

    fn default_pin() -> HotKey {
        HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyD)
    }

    fn parse_or_default(value: &str, default: HotKey) -> HotKey {
        value.parse().unwrap_or(default)
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
                eprintln!("capin: failed to create global hotkey manager: {error}");
                return;
            }
        };
        if let Err(error) = manager.register_all(&[capture, pin]) {
            eprintln!("capin: failed to register global hotkeys: {error}");
        }
        MANAGER.with(|slot| *slot.borrow_mut() = Some(manager));
        REGISTERED.with(|registered| *registered.borrow_mut() = vec![capture, pin]);
        CAPTURE_ID.with(|id| id.set(capture.id()));
        PIN_ID.with(|id| id.set(pin.id()));

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
                    for id in actions {
                        if id == capture_id {
                            if crate::overlay::has_open_overlays() {
                                crate::overlay::close_open_overlays(cx);
                            } else {
                                crate::tray::replace_windows(cx, crate::overlay::open_overlays);
                            }
                        } else if id == pin_id {
                            crate::pin::pin_last(cx);
                        }
                    }
                });
            }
        })
        .detach();
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
            manager
                .unregister_all(&previous)
                .map_err(|error| error.to_string())?;
            manager
                .register_all(&[capture, pin])
                .map_err(|error| error.to_string())?;
            REGISTERED.with(|registered| *registered.borrow_mut() = vec![capture, pin]);
            CAPTURE_ID.with(|id| id.set(capture.id()));
            PIN_ID.with(|id| id.set(pin.id()));
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
pub use imp::{display_hotkey, install, refresh, shortcut_from_key};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn install(_cx: &mut gpui_kit::App) {}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn refresh() -> Result<(), String> {
    Ok(())
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
