use gpui_kit::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChromeTheme {
    pub dark: bool,
    pub surface: Rgba,
    pub surface_strong: Rgba,
    pub border: Rgba,
    pub separator: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub icon: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub accent: Rgba,
    pub accent_tint: Rgba,
    pub accent_text: Rgba,
    pub grid: Rgba,
    pub grid_strong: Rgba,
    pub shadow_alpha: f32,
    pub shadow_blur: f32,
    pub shadow_offset: f32,
    pub radius: f32,
    pub handle_fill: Rgba,
    pub handle_border: Rgba,
    pub canvas: Rgba,
}

impl ChromeTheme {
    pub fn for_window(window: &Window, accent_hex: Option<u32>) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        Self::for_appearance(dark, accent_hex)
    }

    pub fn for_appearance(dark: bool, accent_hex: Option<u32>) -> Self {
        let palette = crate::theme::chrome_palette(dark);
        let accent_hex = accent_hex.unwrap_or_else(|| crate::theme::fallback_accent(dark));
        let accent = rgb(accent_hex);
        Self {
            dark,
            surface: palette.surface,
            surface_strong: palette.surface_strong,
            border: palette.border,
            separator: palette.separator,
            text: palette.text,
            muted: palette.muted,
            icon: palette.icon,
            hover: palette.hover,
            selected: palette.selected,
            accent,
            accent_tint: rgba(((accent_hex & 0x00FF_FFFF) << 8) | 0x33),
            accent_text: crate::theme::accent_text_color(accent_hex),
            grid: palette.grid,
            grid_strong: palette.grid_strong,
            shadow_alpha: palette.shadow_alpha,
            shadow_blur: palette.shadow_blur,
            shadow_offset: palette.shadow_offset,
            radius: palette.radius,
            handle_fill: palette.handle_fill,
            handle_border: accent,
            canvas: palette.canvas,
        }
    }

    pub fn shadow(&self) -> Vec<BoxShadow> {
        vec![
            BoxShadow::new(
                px(0.0),
                px(self.shadow_offset),
                hsla(0.0, 0.0, 0.0, self.shadow_alpha),
            )
            .blur_radius(px(self.shadow_blur)),
        ]
    }
}
