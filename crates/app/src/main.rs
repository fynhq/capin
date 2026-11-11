#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod capture_history;
mod chrome_theme;
mod display;
mod editor;
mod effects;
mod hotkey;
mod i18n;
mod icons;
mod image_util;
mod logging;
mod ocr_queue;
mod overlay;
mod pin;
mod record;
mod record_spotlight;
mod region_outline;
mod scroll;
mod settings;
mod settings_state;
mod storage;
mod tray;
mod ui;
mod window_frame;
mod windows;

use logging::{log_error, log_info, log_warn};

fn main() {
    logging::init();
    log_info!(
        "app",
        "capin {} starting on {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    if !storage::acquire_single_instance() {
        log_warn!("app", "another instance is already running");
        return;
    }
    gpui_kit::application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .run(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(platform_window::reduce_motion());
            platform_window::set_macos_accessory_activation_policy();
            settings_state::apply_startup_autostart();

            if !tray::install(cx) {
                log_error!("app", "tray icon is required for normal startup");
                cx.quit();
                return;
            }
            hotkey::install(cx);
            pin::restore(cx);
            tray::refresh_menu();

            let args: Vec<String> = std::env::args().skip(1).collect();
            log_info!("app", "startup arguments: {args:?}");
            if args.iter().any(|argument| argument == "--settings") {
                settings::open_settings(cx);
            } else if args.iter().any(|argument| argument == "--capture") {
                overlay::open_overlays(cx);
            }
        });
    log_info!("app", "event loop exited");
}
