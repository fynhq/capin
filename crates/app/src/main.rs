mod capture_history;
mod editor;
mod hotkey;
mod i18n;
mod icons;
mod image_util;
mod ocr;
mod overlay;
mod pin;
mod record;
mod record_session;
mod scroll;
mod settings;
mod settings_state;
mod storage;
mod tray;
mod ui;
mod window_style;
mod windows;

fn main() {
    if !storage::acquire_single_instance() {
        eprintln!("capin: another instance is already running");
        return;
    }
    gpui_kit::application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .run(|cx| {
            gpui_kit::init(cx);
            platform_window::set_macos_accessory_activation_policy();
            settings_state::apply_startup_autostart();

            if !tray::install(cx) {
                eprintln!("capin: tray icon is required for normal startup");
                cx.quit();
                return;
            }
            hotkey::install(cx);
            pin::restore(cx);

            let args: Vec<String> = std::env::args().skip(1).collect();
            if args.iter().any(|argument| argument == "--settings") {
                settings::open_settings(cx);
            } else if args.iter().any(|argument| argument == "--capture") {
                overlay::open_overlays(cx);
            }
        });
}
