#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use std::cell::RefCell;
    use std::time::Duration;

    use gpui_kit::*;
    use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use crate::i18n::t;
    use crate::settings_state;

    const MENU_START: &str = "start";
    const MENU_SETTINGS: &str = "settings";
    const MENU_RECOVER: &str = "recover-recording";
    const MENU_RELEASE_PINS: &str = "release-pins";
    const MENU_RESTART: &str = "restart";
    const MENU_QUIT: &str = "quit";

    thread_local! {
        static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    }

    pub fn install(cx: &mut App) -> bool {
        if TRAY.with(|slot| slot.borrow().is_some()) {
            return true;
        }

        let Some(menu) = build_menu() else {
            eprintln!("capin: failed to build tray menu");
            return false;
        };

        let Some(icon) = load_icon() else {
            eprintln!("capin: failed to load tray icon");
            return false;
        };

        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Capin");
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon);
        #[cfg(target_os = "windows")]
        let builder = builder.with_icon(icon).with_menu_on_left_click(false);

        let tray = match builder.build() {
            Ok(tray) => tray,
            Err(error) => {
                eprintln!("capin: failed to create tray icon: {error}");
                return false;
            }
        };

        TRAY.with(|slot| *slot.borrow_mut() = Some(tray));
        start_events(cx);
        true
    }

    fn build_menu() -> Option<Menu> {
        let language = settings_state::get().language;
        let menu = Menu::new();
        let start = MenuItem::with_id(MENU_START, t(language, "menu.start"), true, None);
        let settings = MenuItem::with_id(MENU_SETTINGS, t(language, "menu.settings"), true, None);
        let separator = PredefinedMenuItem::separator();
        let restart = MenuItem::with_id(MENU_RESTART, t(language, "menu.restart"), true, None);
        let quit = MenuItem::with_id(MENU_QUIT, t(language, "menu.quit"), true, None);
        let mut items: Vec<&dyn IsMenuItem> = vec![&start, &settings];

        // Recovery entries only appear when they can do something, so the
        // normal menu stays at Start / Settings / Restart / Quit while a
        // click-through pin or an interrupted recording is still reachable.
        let recover = (!crate::record_session::pending_sessions().is_empty()).then(|| {
            MenuItem::with_id(
                MENU_RECOVER,
                t(language, "menu.recover_recording"),
                true,
                None,
            )
        });
        if let Some(item) = recover.as_ref() {
            items.push(item);
        }
        let release_pins = (crate::pin::click_through_count() > 0).then(|| {
            MenuItem::with_id(
                MENU_RELEASE_PINS,
                t(language, "menu.release_pins"),
                true,
                None,
            )
        });
        if let Some(item) = release_pins.as_ref() {
            items.push(item);
        }
        items.push(&separator);
        items.push(&restart);
        items.push(&quit);
        menu.append_items(&items).ok()?;
        Some(menu)
    }

    /// Rebuilds the tray menu after a language change.
    pub fn refresh_menu() {
        TRAY.with(|slot| {
            if let Some(tray) = slot.borrow().as_ref()
                && let Some(menu) = build_menu()
            {
                tray.set_menu(Some(Box::new(menu)));
            }
        });
    }

    fn load_icon() -> Option<Icon> {
        #[cfg(target_os = "macos")]
        let bytes: &[u8] = include_bytes!("../../../assets/icon/tray/capin_tray_template.png");
        #[cfg(target_os = "windows")]
        let bytes: &[u8] = {
            if platform_window::system_uses_light_theme() == Some(true) {
                include_bytes!("../../../assets/icon/tray/capin_tray_dark.png")
            } else {
                include_bytes!("../../../assets/icon/tray/capin_tray_light.png")
            }
        };

        let image = image::load_from_memory(bytes).ok()?.into_rgba8();
        let (width, height) = image.dimensions();
        Icon::from_rgba(image.into_raw(), width, height).ok()
    }

    fn start_events(cx: &mut App) {
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;

                let mut actions = Vec::new();
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    actions.push(event.id().as_ref().to_owned());
                }
                if actions.is_empty() {
                    continue;
                }

                cx.update(|cx| {
                    for action in actions {
                        handle_action(&action, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn handle_action(action: &str, cx: &mut App) {
        match action {
            MENU_START => replace_windows(cx, crate::overlay::open_overlays),
            MENU_SETTINGS => replace_windows(cx, crate::settings::open_settings),
            MENU_RECOVER => {
                if let Some(directory) = crate::record_session::pending_sessions().last() {
                    cx.open_with_system(directory);
                    crate::record_session::mark_recovered(directory);
                    refresh_menu();
                }
            }
            MENU_RELEASE_PINS => {
                crate::pin::release_click_through(cx);
            }
            MENU_RESTART => restart(cx),
            MENU_QUIT => cx.quit(),
            _ => {}
        }
    }

    pub fn replace_windows(cx: &mut App, open: impl FnOnce(&mut App) + 'static) {
        let handles = cx.window_stack().unwrap_or_default();
        if handles.is_empty() {
            open(cx);
            return;
        }

        cx.defer(move |cx| {
            crate::windows::close_auxiliary(cx, handles);
            // Let the closed window leave the compositor without blocking the
            // UI thread. Capture itself excludes Capin windows, so this delay
            // is only a visual cleanup.
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                cx.update(|cx| open(cx));
            })
            .detach();
        });
    }

    fn restart(cx: &mut App) {
        match relaunch() {
            Ok(()) => cx.quit(),
            Err(error) => eprintln!("capin: failed to restart: {error}"),
        }
    }

    fn relaunch() -> std::io::Result<()> {
        let executable = std::env::current_exe()?;

        #[cfg(target_os = "macos")]
        {
            if let Some(bundle) = executable
                .ancestors()
                .find(|path| path.extension() == Some(std::ffi::OsStr::new("app")))
            {
                std::process::Command::new("open")
                    .arg("-n")
                    .arg(bundle)
                    .spawn()?;
                return Ok(());
            }
        }

        std::process::Command::new(executable).spawn()?;
        Ok(())
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use imp::{install, refresh_menu, replace_windows};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn install(_cx: &mut gpui_kit::App) -> bool {
    false
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn refresh_menu() {}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn replace_windows(cx: &mut gpui_kit::App, open: impl FnOnce(&mut gpui_kit::App) + 'static) {
    open(cx);
}
