#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use std::cell::RefCell;
    use std::time::Duration;

    use gpui_kit::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use crate::i18n::t;
    use crate::settings_state;

    const MENU_CAPTURE: &str = "capture";
    const MENU_SCROLL: &str = "scroll";
    const MENU_OCR: &str = "ocr";
    const MENU_FOLDER: &str = "folder";
    const MENU_RECORD: &str = "record";
    const MENU_SETTINGS: &str = "settings";
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
        let capture = MenuItem::with_id(MENU_CAPTURE, t(language, "menu.capture"), true, None);
        let record = MenuItem::with_id(MENU_RECORD, t(language, "menu.record"), true, None);
        let scroll = MenuItem::with_id(MENU_SCROLL, t(language, "menu.scroll_capture"), true, None);
        let ocr = MenuItem::with_id(MENU_OCR, t(language, "menu.ocr"), true, None);
        let folder = MenuItem::with_id(
            MENU_FOLDER,
            t(language, "menu.screenshot_folder"),
            true,
            None,
        );
        let settings = MenuItem::with_id(MENU_SETTINGS, t(language, "menu.settings"), true, None);
        let release_pins = MenuItem::with_id(
            MENU_RELEASE_PINS,
            t(language, "menu.release_pins"),
            true,
            None,
        );
        let separator = PredefinedMenuItem::separator();
        let restart = MenuItem::with_id(MENU_RESTART, t(language, "menu.restart"), true, None);
        let quit = MenuItem::with_id(MENU_QUIT, t(language, "menu.quit"), true, None);
        menu.append_items(&[
            &capture,
            &record,
            &scroll,
            &ocr,
            &folder,
            &settings,
            &release_pins,
            &separator,
            &restart,
            &quit,
        ])
        .ok()?;
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
            MENU_CAPTURE => replace_windows(cx, crate::overlay::open_overlays),
            MENU_RECORD => replace_windows(cx, crate::overlay::open_record_overlays),
            MENU_SCROLL => replace_windows(cx, crate::overlay::open_scroll_overlays),
            MENU_OCR => replace_windows(cx, crate::overlay::open_ocr_overlays),
            MENU_FOLDER => {
                let directory = crate::storage::screenshot_dir();
                if let Err(error) = crate::storage::ensure_directory(&directory) {
                    eprintln!("capin: failed to create the screenshot folder: {error}");
                } else {
                    cx.open_with_system(&directory);
                }
            }
            MENU_SETTINGS => replace_windows(cx, crate::settings::open_settings),
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
            crate::pin::close_windows_except_pins(cx, handles);
            // Let the closed window leave the compositor before a screenshot
            // freezes the desktop.
            std::thread::sleep(Duration::from_millis(50));
            open(cx);
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
