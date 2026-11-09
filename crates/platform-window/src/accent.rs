/// Returns the platform accent as packed `0xRRGGBB`, or `None` when the
/// platform has no usable accent source.
#[cfg(windows)]
pub fn system_accent_hex() -> Option<u32> {
    use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;

    const ACCENT_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent";

    if let Some(abgr) = hkcu_dword(ACCENT_KEY, "AccentColorMenu") {
        return Some(core_types::abgr_to_rgb(abgr));
    }

    let mut argb = 0u32;
    // DWM colorization is a fallback: it can lag and is tinted by the
    // transparency mix. Explorer's AccentColorMenu is what Settings uses.
    if unsafe { DwmGetColorizationColor(&mut argb, &mut Default::default()) }.is_err() {
        return None;
    }
    Some(argb & 0x00FF_FFFF)
}

#[cfg(windows)]
fn hkcu_dword(key: &str, value: &str) -> Option<u32> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
        RegQueryValueExW,
    };

    let utf16 =
        |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    let key_w = utf16(key);
    let value_w = utf16(value);
    let mut hkey = HKEY::default();

    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(key_w.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        )
    }
    .is_err()
    {
        return None;
    }

    let mut data = [0u8; 4];
    let mut size = 4u32;
    let mut kind = REG_VALUE_TYPE::default();
    let queried = unsafe {
        RegQueryValueExW(
            hkey,
            windows::core::PCWSTR(value_w.as_ptr()),
            None,
            Some(&mut kind),
            Some(data.as_mut_ptr()),
            Some(&mut size),
        )
    };
    let _ = unsafe { RegCloseKey(hkey) };

    if queried.is_ok() && size >= 4 && kind == REG_DWORD {
        Some(u32::from_le_bytes(data))
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
pub fn system_accent_hex() -> Option<u32> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSColor, NSColorSpace};

    let color = NSColor::controlAccentColor();
    let srgb = NSColorSpace::sRGBColorSpace();
    let converted: Retained<NSColor> = color.colorUsingColorSpace(&srgb)?;

    let (mut red, mut green, mut blue, mut alpha) = (0f64, 0f64, 0f64, 0f64);
    unsafe { converted.getRed_green_blue_alpha(&mut red, &mut green, &mut blue, &mut alpha) };

    let to_u8 = |component: f64| ((component.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    Some((to_u8(red) << 16) | (to_u8(green) << 8) | to_u8(blue))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn system_accent_hex() -> Option<u32> {
    None
}

/// Whether the Windows app theme is light, used to pick a tray glyph and the
/// "follow system" window background.
#[cfg(windows)]
pub fn system_uses_light_theme() -> Option<bool> {
    const PERSONALIZE_KEY: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
    hkcu_dword(PERSONALIZE_KEY, "AppsUseLightTheme").map(|value| value != 0)
}

/// Whether the app is running under the light system appearance. AppKit reads
/// the effective appearance of the shared application, so an explicit app
/// override would be reflected too.
#[cfg(target_os = "macos")]
pub fn system_uses_light_theme() -> Option<bool> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let marker = MainThreadMarker::new()?;
    let application = NSApplication::sharedApplication(marker);
    let appearance = application.effectiveAppearance();
    Some(!appearance.name().to_string().contains("Dark"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn system_uses_light_theme() -> Option<bool> {
    None
}

/// Whether the taskbar (not the app chrome) is light. The tray glyph sits on
/// the taskbar, so this follows `SystemUsesLightTheme` and falls back to the
/// app theme when the value is missing.
#[cfg(windows)]
pub fn system_taskbar_uses_light_theme() -> Option<bool> {
    const PERSONALIZE_KEY: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
    hkcu_dword(PERSONALIZE_KEY, "SystemUsesLightTheme")
        .or_else(|| hkcu_dword(PERSONALIZE_KEY, "AppsUseLightTheme"))
        .map(|value| value != 0)
}

#[cfg(not(windows))]
pub fn system_taskbar_uses_light_theme() -> Option<bool> {
    None
}
