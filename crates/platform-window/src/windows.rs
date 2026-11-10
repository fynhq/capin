use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::POINT as ScreenPoint;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetCursorPos, GetWindowLongPtrW, GetWindowRect,
    HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, LoadCursorW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetCursor, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
    WDA_EXCLUDEFROMCAPTURE, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    WindowFromPoint,
};
use windows::core::GUID;

use crate::{PlatformWindowError, WindowStyle};

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
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(PlatformWindowError::NotWin32);
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    // `bounds` describes the client area that must cover the monitor. A
    // window that still carries non-client frame styles would place its
    // client area a few pixels inside this rectangle, scaling the frozen
    // frame and clipping the right/bottom edges. Expand the native window by
    // the measured frame insets so the client matches the monitor.
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
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    }
    .map_err(|error| platform_error("SetWindowPos", error))
}

/// Native frame thickness around the client area, as
/// `(left, top, total_width, total_height)` in physical pixels.
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
        // GPUI's popup windows keep the resize frame on the left, right and
        // bottom even after the title bar is hidden. A capture overlay must
        // cover the monitor exactly, so drop the whole non-client frame
        // instead of living with a client area that is a few pixels smaller
        // than the display. Transparent inline editors keep GPUI's frame
        // compensation: they are placed before their style is applied.
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
        // SAFETY: hwnd belongs to a live window created by the caller.
        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }
            .map_err(|error| platform_error("SetWindowDisplayAffinity", error))?;
    }

    Ok(())
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
