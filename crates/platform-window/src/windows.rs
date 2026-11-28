use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::POINT as ScreenPoint;
use windows::Win32::Foundation::{COLORREF, GlobalFree, HANDLE, HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMNCRENDERINGPOLICY, DWMNCRP_DISABLED,
    DWMNCRP_ENABLED, DWMSBT_MAINWINDOW, DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR,
    DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetCursorPos, GetWindowLongPtrW, GetWindowRect,
    HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, LoadCursorW, SPI_GETCLIENTAREAANIMATION,
    SPI_GETHIGHCONTRAST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursor, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, SystemParametersInfoW,
    WDA_EXCLUDEFROMCAPTURE, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use windows::core::GUID;

use crate::{BackdropMaterial, PlatformWindowError, WindowStyle};

pub(super) fn trim_working_set() {
    // SAFETY: the pseudo-handle refers to this process; -1 sizes request a trim.
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

pub(super) fn reduce_motion() -> bool {
    let mut animations: i32 = 1;
    let queried = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut animations as *mut i32 as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
    };
    queried && animations == 0
}

pub(super) fn monitor_for_window(window: &impl HasWindowHandle) -> Option<u64> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    (!monitor.0.is_null()).then_some(monitor.0 as usize as u64)
}

pub(super) fn place_on_monitor(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
) -> Result<(), PlatformWindowError> {
    place_on_monitor_impl(window, bounds, true)
}

pub(super) fn place_on_monitor_deferred(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
) -> Result<(), PlatformWindowError> {
    place_on_monitor_impl(window, bounds, false)
}

fn place_on_monitor_impl(
    window: &impl HasWindowHandle,
    bounds: core_types::PhysicalRect,
    frame_changed: bool,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let (inset_left, inset_top, extra_width, extra_height) = client_frame_insets(hwnd);
    let width = i32::try_from(bounds.size.width)
        .ok()
        .and_then(|width| width.checked_add(extra_width))
        .ok_or_else(|| PlatformWindowError::Platform {
            operation: "SetWindowPos",
            message: "window width exceeds i32".to_owned(),
        })?;
    let height = i32::try_from(bounds.size.height)
        .ok()
        .and_then(|height| height.checked_add(extra_height))
        .ok_or_else(|| PlatformWindowError::Platform {
            operation: "SetWindowPos",
            message: "window height exceeds i32".to_owned(),
        })?;
    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
    if frame_changed {
        flags |= SWP_FRAMECHANGED;
    }
    // SAFETY: hwnd belongs to a live window created by the caller. The
    // rectangle is in physical pixels because the process is PerMonitorV2.
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            bounds.origin.x - inset_left,
            bounds.origin.y - inset_top,
            width,
            height,
            flags,
        )
    }
    .map_err(|error| platform_error("SetWindowPos", error))
}

fn client_frame_insets(hwnd: HWND) -> (i32, i32, i32, i32) {
    let mut window_rect = RECT::default();
    let mut client_rect = RECT::default();
    // SAFETY: both output buffers match the requested structures and hwnd
    // belongs to a live window created by the caller.
    if unsafe { GetWindowRect(hwnd, &mut window_rect) }.is_err()
        || unsafe { GetClientRect(hwnd, &mut client_rect) }.is_err()
    {
        return (0, 0, 0, 0);
    }
    let mut client_origin = ScreenPoint { x: 0, y: 0 };
    // SAFETY: hwnd is live and the point is a caller-owned output value.
    if !unsafe { ClientToScreen(hwnd, &mut client_origin) }.as_bool() {
        return (0, 0, 0, 0);
    }
    let window_width = window_rect.right - window_rect.left;
    let window_height = window_rect.bottom - window_rect.top;
    let client_width = client_rect.right - client_rect.left;
    let client_height = client_rect.bottom - client_rect.top;
    let left = client_origin.x - window_rect.left;
    let top = client_origin.y - window_rect.top;
    let right = window_width - left - client_width;
    let bottom = window_height - top - client_height;
    (left, top, left + right, top + bottom)
}

pub(super) fn apply(
    window: &impl HasWindowHandle,
    style: WindowStyle,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);

    if style.borderless && style.opaque {
        let mut window_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        window_style &=
            !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0);
        window_style |= WS_POPUP.0;
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_STYLE, window_style as isize);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    if style.skip_taskbar || style.no_activate {
        // SAFETY: hwnd belongs to a live window created by the caller.
        let mut ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if style.skip_taskbar {
            ex_style |= WS_EX_TOOLWINDOW.0;
        }
        if style.no_activate {
            ex_style |= WS_EX_NOACTIVATE.0;
        }
        // SAFETY: the extended style value came from the same window.
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
        }
    }

    if style.always_on_top {
        // SAFETY: hwnd belongs to a live window created by the caller.
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(|error| platform_error("SetWindowPos", error))?;
    }

    if style.exclude_from_capture {
        if !capture_exclusion_supported() {
            return Err(PlatformWindowError::Platform {
                operation: "SetWindowDisplayAffinity",
                message: "requires Windows 10 version 2004 or later".to_owned(),
            });
        }
        // SAFETY: hwnd belongs to a live window created by the caller.
        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }
            .map_err(|error| platform_error("SetWindowDisplayAffinity", error))?;
    }

    Ok(())
}

fn capture_exclusion_supported() -> bool {
    windows_build_number() >= 19_041
}

fn windows_build_number() -> u32 {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: version is a valid, writable version buffer.
    if unsafe { RtlGetVersion(&mut version) }.is_ok() {
        version.dwBuildNumber
    } else {
        0
    }
}

pub(super) fn transparency_allowed() -> bool {
    if high_contrast_enabled() {
        return false;
    }
    const PERSONALIZE_KEY: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
    super::accent::hkcu_dword(PERSONALIZE_KEY, "EnableTransparency")
        .map(|value| value != 0)
        .unwrap_or(true)
}

fn high_contrast_enabled() -> bool {
    let mut contrast = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: the struct is sized and writable for the duration of the call.
    let queried = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some(&mut contrast as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
    };
    queried && contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
}

pub(super) fn backdrop_supported(_material: BackdropMaterial) -> bool {
    windows_build_number() >= 22_621
}

pub(super) fn set_backdrop(
    window: &impl HasWindowHandle,
    material: BackdropMaterial,
) -> Result<(), PlatformWindowError> {
    if !backdrop_supported(material) {
        return Err(PlatformWindowError::Platform {
            operation: "SetWindowBackdrop",
            message: "requires Windows 11 22H2 or later".to_owned(),
        });
    }
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let backdrop = match material {
        BackdropMaterial::Mica => DWMSBT_MAINWINDOW,
        BackdropMaterial::Acrylic => DWMSBT_TRANSIENTWINDOW,
    };
    // SAFETY: hwnd is live and the DWM reads the value during this call.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const _ as *const c_void,
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
    .map_err(|error| platform_error("SetWindowBackdrop", error))
}

pub(super) fn pictures_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Pictures).or_else(|| profile_dir("Pictures"))
}

pub(super) fn videos_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Videos).or_else(|| profile_dir("Videos"))
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: the shell allocates a null-terminated UTF-16 string with CoTaskMemAlloc.
    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    // SAFETY: raw is a valid null-terminated UTF-16 string owned by this call.
    let path = unsafe { raw.to_string() }.ok();
    // SAFETY: the pointer came from SHGetKnownFolderPath and is freed exactly once.
    unsafe { CoTaskMemFree(Some(raw.0.cast::<c_void>())) };
    path.map(PathBuf::from)
}

fn profile_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|profile| PathBuf::from(profile).join(name))
}

pub(super) fn set_click_through(
    window: &impl HasWindowHandle,
    enabled: bool,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    // SAFETY: hwnd belongs to a live window created by the caller.
    let mut ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if enabled {
        ex_style |= WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0;
        // SAFETY: hwnd is a live window; failure is safe to ignore when the
        // window already uses a composition surface.
        let _ = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) };
    } else {
        ex_style &= !(WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0);
    }
    // SAFETY: the extended style value came from the same window.
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
    }
    Ok(())
}

const CF_DIB_FORMAT: u32 = 8;
const CF_DIBV5_FORMAT: u32 = 17;
const BI_BITFIELDS: u32 = 3;
const LCS_SRGB: u32 = 0x7352_4742;
const LCS_GM_GRAPHICS: u32 = 2;

pub(super) fn set_image_clipboard(
    width: u32,
    height: u32,
    rgba: &[u8],
    png: Option<&[u8]>,
) -> Result<(), PlatformWindowError> {
    let pixel_bytes = width as usize * height as usize * 4;
    if width == 0 || height == 0 || rgba.len() < pixel_bytes {
        return Err(PlatformWindowError::Platform {
            operation: "SetClipboardData",
            message: "invalid image buffer".to_owned(),
        });
    }
    let dibv5 = dibv5_from_rgba(width, height, rgba);
    let dib = dib24_from_rgba(width, height, rgba);

    let mut opened = false;
    for _ in 0..5 {
        // SAFETY: no clipboard data is read before this thread owns it.
        if unsafe { OpenClipboard(None) }.is_ok() {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if !opened {
        return Err(platform_error(
            "OpenClipboard",
            std::io::Error::last_os_error(),
        ));
    }
    let result = (|| -> windows::core::Result<()> {
        // SAFETY: this thread owns the open clipboard until CloseClipboard.
        unsafe {
            EmptyClipboard()?;
            if let Some(png) = png {
                let format = RegisterClipboardFormatW(windows::core::w!("PNG"));
                if format != 0 {
                    set_clipboard_bytes(format, png)?;
                }
            }
            set_clipboard_bytes(CF_DIBV5_FORMAT, &dibv5)?;
            set_clipboard_bytes(CF_DIB_FORMAT, &dib)?;
        }
        Ok(())
    })();
    // SAFETY: pairs with the successful OpenClipboard above.
    unsafe {
        let _ = CloseClipboard();
    }
    result.map_err(|error| platform_error("SetClipboardData", error))
}

fn dibv5_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let pixel_bytes = width as usize * height as usize * 4;
    let mut dib = Vec::with_capacity(124 + pixel_bytes);
    let mut header = [0u8; 124];
    header[0..4].copy_from_slice(&124u32.to_le_bytes());
    header[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    header[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    header[12..14].copy_from_slice(&1u16.to_le_bytes());
    header[14..16].copy_from_slice(&32u16.to_le_bytes());
    header[16..20].copy_from_slice(&BI_BITFIELDS.to_le_bytes());
    header[20..24].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
    header[24..28].copy_from_slice(&2835i32.to_le_bytes());
    header[28..32].copy_from_slice(&2835i32.to_le_bytes());
    header[40..44].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
    header[44..48].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
    header[48..52].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
    header[52..56].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
    header[56..60].copy_from_slice(&LCS_SRGB.to_le_bytes());
    header[108..112].copy_from_slice(&LCS_GM_GRAPHICS.to_le_bytes());
    dib.extend_from_slice(&header);
    for row in (0..height as usize).rev() {
        for column in 0..width as usize {
            let index = (row * width as usize + column) * 4;
            dib.push(rgba[index + 2]);
            dib.push(rgba[index + 1]);
            dib.push(rgba[index]);
            dib.push(rgba[index + 3]);
        }
    }
    dib
}

fn dib24_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row_bytes = ((width as usize * 3 + 3) & !3).max(4);
    let pixel_bytes = row_bytes * height as usize;
    let mut dib = Vec::with_capacity(40 + pixel_bytes);
    let mut header = [0u8; 40];
    header[0..4].copy_from_slice(&40u32.to_le_bytes());
    header[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    header[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    header[12..14].copy_from_slice(&1u16.to_le_bytes());
    header[14..16].copy_from_slice(&24u16.to_le_bytes());
    header[20..24].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
    header[24..28].copy_from_slice(&2835i32.to_le_bytes());
    header[28..32].copy_from_slice(&2835i32.to_le_bytes());
    dib.extend_from_slice(&header);
    dib.resize(40 + pixel_bytes, 0);
    for row in 0..height as usize {
        let target_row = 40 + (height as usize - 1 - row) * row_bytes;
        for column in 0..width as usize {
            let index = (row * width as usize + column) * 4;
            let alpha = u32::from(rgba[index + 3]);
            for (channel, source) in [(0usize, 2usize), (1, 1), (2, 0)] {
                let value = u32::from(rgba[index + source]);
                dib[target_row + column * 3 + channel] =
                    ((value * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            }
        }
    }
    dib
}

fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> windows::core::Result<()> {
    // SAFETY: the clipboard is open on this thread; the allocation is handed
    // to the system on success and freed on every failure path.
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            let _ = GlobalFree(Some(handle));
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len());
        let _ = GlobalUnlock(handle);
        if let Err(error) = SetClipboardData(format, Some(HANDLE(handle.0))) {
            let _ = GlobalFree(Some(handle));
            return Err(error);
        }
    }
    Ok(())
}

fn platform_error(operation: &'static str, error: impl std::fmt::Display) -> PlatformWindowError {
    PlatformWindowError::Platform {
        operation,
        message: error.to_string(),
    }
}

pub(super) fn native_window_id(window: &impl HasWindowHandle) -> Option<u64> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(win32.hwnd.get() as u64)
}

pub(super) fn cursor_screen_position() -> Option<(i32, i32)> {
    let mut point = ScreenPoint { x: 0, y: 0 };
    // SAFETY: the point is a caller-owned output buffer.
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

pub(super) fn window_screen_origin(window: &impl HasWindowHandle) -> Option<(i32, i32)> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let mut rect = RECT::default();
    // SAFETY: the rectangle is a caller-owned output buffer and hwnd is live.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some((rect.left, rect.top))
}

pub(super) fn move_window_to(
    window: &impl HasWindowHandle,
    x: i32,
    y: i32,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    // SAFETY: hwnd is live; only the origin changes, size and z-order stay.
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .map_err(|error| platform_error("SetWindowPos", error))
}

pub(super) fn set_cursor_hidden(hidden: bool) {
    let cursor = if hidden {
        None
    } else {
        // SAFETY: IDC_ARROW is a system cursor resource.
        unsafe { LoadCursorW(None, IDC_ARROW) }.ok()
    };
    // SAFETY: the cursor belongs to this thread's window; None hides it.
    unsafe {
        SetCursor(cursor);
    }
}

pub(super) fn hide_native_border(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let border = DWMWA_COLOR_NONE;
    let corners = DWMWCP_DONOTROUND;
    // SAFETY: hwnd is live and the attribute pointers outlive each call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const _ as *const c_void,
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corners as *const _ as *const c_void,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

pub(super) fn disable_native_shadow(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let policy = DWMNCRP_DISABLED;
    // SAFETY: hwnd is live and the policy pointer outlives the call. DWM keeps
    // the client surface; only non-client frame and shadow rendering stop.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as *const c_void,
            size_of::<DWMNCRENDERINGPOLICY>() as u32,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub(super) fn enable_native_shadow(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let policy = DWMNCRP_ENABLED;
    // SAFETY: hwnd is live and the policy pointer outlives the call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as *const c_void,
            size_of::<DWMNCRENDERINGPOLICY>() as u32,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
