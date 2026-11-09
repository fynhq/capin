use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, ReleaseCapture, SendInput,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, HTCAPTION, HWND_TOPMOST, LWA_ALPHA, SPI_GETDESKWALLPAPER,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
    SystemParametersInfoW, WDA_EXCLUDEFROMCAPTURE, WM_NCLBUTTONDOWN, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WindowFromPoint,
};
use windows::core::GUID;

use crate::{PlatformWindowError, WindowStyle};

pub(super) fn apply(
    window: &impl HasWindowHandle,
    style: WindowStyle,
) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);

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
        // SAFETY: hwnd belongs to a live window created by the caller.
        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }
            .map_err(|error| platform_error("SetWindowDisplayAffinity", error))?;
    }

    Ok(())
}

pub(super) fn pictures_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Pictures).or_else(|| profile_dir("Pictures"))
}

pub(super) fn desktop_wallpaper_path() -> Option<PathBuf> {
    const MAX_PATH: usize = 32_768;
    let mut buffer = vec![0u16; MAX_PATH];
    // SAFETY: the buffer is writable for `buffer.len()` UTF-16 units and the
    // API only fills it with a null-terminated path.
    let size = buffer.len() as u32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            size,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Default::default(),
        )
    }
    .ok()?;
    let end = buffer.iter().position(|unit| *unit == 0)?;
    if end == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
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
        // Remove LAYERED again: leaving it behind can make later opacity or
        // composition changes behave differently from the original window.
        ex_style &= !(WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0);
    }
    // SAFETY: the extended style value came from the same window.
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
    }
    Ok(())
}

fn platform_error(operation: &'static str, error: impl std::fmt::Display) -> PlatformWindowError {
    PlatformWindowError::Platform {
        operation,
        message: error.to_string(),
    }
}

pub(super) fn begin_window_drag(window: &impl HasWindowHandle) -> Result<(), PlatformWindowError> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    // SAFETY: hwnd belongs to a live window created by the caller. The
    // message is processed synchronously and returns once the OS move loop
    // ends on left-button release.
    unsafe {
        let _ = ReleaseCapture();
        SendMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            Some(WPARAM(HTCAPTION as usize)),
            Some(LPARAM(0)),
        );
    }
    Ok(())
}

pub(super) fn native_window_id(window: &impl HasWindowHandle) -> Option<u64> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(win32.hwnd.get() as u64)
}

pub(super) fn scroll_wheel_at(monitor: u64, x: i32, y: i32, notches: i32) -> bool {
    if notches == 0 {
        return true;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: the monitor handle comes from the same process that enumerated
    // it; GetMonitorInfoW only writes into the provided structure.
    if !unsafe { GetMonitorInfoW(HMONITOR(monitor as usize as *mut c_void), &mut info) }.as_bool() {
        return false;
    }
    let screen_x = info.rcMonitor.left + x;
    let screen_y = info.rcMonitor.top + y;
    // SAFETY: the point is supplied by the caller and Query APIs do not retain it.
    let target = unsafe {
        WindowFromPoint(POINT {
            x: screen_x,
            y: screen_y,
        })
    };
    if target.0.is_null() {
        return false;
    }
    // SAFETY: target is a live window handle returned by WindowFromPoint.
    unsafe {
        let _ = SetForegroundWindow(target);
    }

    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: notches.saturating_mul(120) as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                ..Default::default()
            },
        },
    };
    // SAFETY: input is a fully initialized INPUT for a wheel event.
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) == 1 }
}
