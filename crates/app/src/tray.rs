#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use std::cell::RefCell;
    use std::time::Duration;

    use gpui_kit::*;
    use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton as TrayMouseButton, MouseButtonState, TrayIcon, TrayIconBuilder,
        TrayIconEvent,
    };

    use crate::i18n::t;
    #[cfg(target_os = "windows")]
    use crate::logging::log_warn;
    use crate::logging::{log_error, log_info};
    use crate::settings_state;

    const MENU_START: &str = "start";
    const MENU_SETTINGS: &str = "settings";
    const MENU_RELEASE_PINS: &str = "release-pins";
    const MENU_RESTART: &str = "restart";
    const MENU_QUIT: &str = "quit";

    thread_local! {
        static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
        #[cfg(target_os = "windows")]
        static TRAY_LIGHT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    pub fn install(cx: &mut App) -> bool {
        if TRAY.with(|slot| slot.borrow().is_some()) {
            return true;
        }

        let Some(menu) = build_menu() else {
            log_error!("tray", "failed to build tray menu");
            return false;
        };

        let Some(icon) = load_icon() else {
            log_error!("tray", "failed to load tray icon");
            return false;
        };
        #[cfg(target_os = "windows")]
        TRAY_LIGHT.with(|slot| slot.set(Some(surface_is_light())));

        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .with_tooltip("Capin");
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon);

        let tray = match builder.build() {
            Ok(tray) => tray,
            Err(error) => {
                log_error!("tray", "failed to create tray icon: {error}");
                return false;
            }
        };

        TRAY.with(|slot| *slot.borrow_mut() = Some(tray));
        if settings_state::get().hide_tray {
            TRAY.with(|slot| {
                if let Some(tray) = slot.borrow().as_ref() {
                    let _ = tray.set_visible(false);
                }
            });
        }
        log_info!("tray", "tray icon installed");
        start_events(cx);
        true
    }

    pub fn set_visible(visible: bool) {
        TRAY.with(|slot| {
            if let Some(tray) = slot.borrow().as_ref() {
                let _ = tray.set_visible(visible);
            }
        });
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
        let bytes: &[u8] = include_bytes!("../../../assets/icon/tray/capin_tray_light_macos.png");
        #[cfg(not(target_os = "macos"))]
        let bytes: &[u8] = if surface_is_light() {
            include_bytes!("../../../assets/icon/tray/capin_tray_light.png")
        } else {
            include_bytes!("../../../assets/icon/tray/capin_tray_dark.png")
        };

        let image = image::load_from_memory(bytes).ok()?.into_rgba8();
        let (width, height) = image.dimensions();
        Icon::from_rgba(image.into_raw(), width, height).ok()
    }

    #[cfg(target_os = "windows")]
    fn surface_is_light() -> bool {
        platform_window::system_taskbar_uses_light_theme().unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    fn refresh_icon_for_theme() {}

    #[cfg(target_os = "windows")]
    fn refresh_icon_for_theme() {
        let light = surface_is_light();
        if TRAY_LIGHT.with(|slot| slot.get()) == Some(light) {
            return;
        }
        let Some(icon) = load_icon() else {
            return;
        };
        TRAY.with(|slot| {
            if let Some(tray) = slot.borrow().as_ref()
                && let Err(error) = tray.set_icon(Some(icon))
            {
                log_warn!("tray", "failed to swap tray icon: {error}");
                return;
            }
            TRAY_LIGHT.with(|slot| slot.set(Some(light)));
        });
    }

    fn start_events(cx: &mut App) {
        cx.spawn(async move |cx| {
            let mut ticks = 0u32;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;

                ticks = ticks.wrapping_add(1);
                if ticks.is_multiple_of(10) {
                    refresh_icon_for_theme();
                    crate::overlay::remember_frontmost_application();
                }

                let mut actions = Vec::new();
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    actions.push(event.id().as_ref().to_owned());
                }
                let mut start_capture = false;
                while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click {
                        button: TrayMouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        start_capture = true;
                    }
                }
                if actions.is_empty() && !start_capture {
                    continue;
                }

                cx.update(|cx| {
                    if start_capture && !crate::overlay::has_open_overlays() {
                        log_info!("tray", "tray click: starting capture");
                        crate::overlay::open_overlays(cx);
                    }
                    for action in actions {
                        log_info!("tray", "menu action: {action}");
                        handle_action(&action, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn handle_action(action: &str, cx: &mut App) {
        match action {
            MENU_START => {
                if crate::overlay::has_open_overlays() {
                    crate::overlay::close_open_overlays(cx);
                } else {
                    crate::overlay::open_overlays(cx);
                }
            }
            MENU_SETTINGS => replace_windows(cx, crate::settings::open_settings),
            MENU_RELEASE_PINS => {
                crate::pin::release_click_through(cx);
            }
            MENU_RESTART => restart(cx),
            MENU_QUIT => {
                if !crate::record::shutdown(cx, Duration::from_secs(10)) {
                    log_error!("record", "timed out while finalizing the recording");
                }
                crate::pin::flush_manifest();
                cx.quit();
            }
            _ => {}
        }
    }

    pub fn replace_windows(cx: &mut App, open: impl FnOnce(&mut App) + 'static) {
        let handles = cx.windows();
        if handles.is_empty() {
            open(cx);
            return;
        }

        cx.defer(move |cx| {
            crate::windows::close_auxiliary(cx, handles);
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                cx.update(|cx| open(cx));
            })
            .detach();
        });
    }

    pub fn restart(cx: &mut App) {
        if !crate::record::shutdown(cx, Duration::from_secs(10)) {
            log_error!("record", "timed out while finalizing the recording");
        }
        crate::pin::flush_manifest();
        crate::storage::release_single_instance();
        match relaunch() {
            Ok(()) => {
                log_info!("tray", "restarting");
                cx.quit();
            }
            Err(error) => {
                log_error!("tray", "failed to restart: {error}");
                crate::storage::acquire_single_instance();
            }
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

#[cfg(target_os = "macos")]
pub use imp::restart;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use imp::{install, refresh_menu, set_visible};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn install(_cx: &mut gpui_kit::App) -> bool {
    false
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn refresh_menu() {}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn set_visible(_visible: bool) {}
