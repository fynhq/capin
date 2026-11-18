use gpui_kit::*;
use platform_window::{WindowStyle, apply as apply_window_style};

pub(crate) fn open<T: Render + 'static>(
    id: &'static str,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App, bool) -> Entity<T> + 'static,
) -> Result<Entity<T>, String> {
    let options = crate::aux_window::popup_options(
        id,
        bounds,
        display_id,
        false,
        WindowKind::PopUp,
        WindowBackgroundAppearance::Transparent,
    );
    gpui_kit::open_window(options, cx, move |window, cx| {
        let styled = match apply_window_style(&*window, WindowStyle::CAPTURE_BAR) {
            Ok(()) => true,
            Err(error) => {
                crate::logging::log_error!("control-bar", "failed to style capture bar: {error}");
                false
            }
        };
        build(window, cx, styled)
    })
    .map(|(_, entity)| entity)
    .map_err(|error| error.to_string())
}
