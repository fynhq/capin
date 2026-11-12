//! Windows backend built on Windows Graphics Capture, DWM, and Win32 enumeration.

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc;
use std::time::Duration;

use core_types::{DpiScale, MonitorId, PhysicalRect, RgbaFrame, WindowId};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUNDSMALL, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::{CUIAutomation, TreeScope_Descendants};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForWindow,
    MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetDesktopWindow, GetShellWindow,
    GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsIconic, IsWindowVisible, IsZoomed, MONITORINFOF_PRIMARY,
    WDA_EXCLUDEFROMCAPTURE, WS_CHILD, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::{CaptureBackend, CaptureError, CaptureTarget, MonitorInfo, Result, WindowInfo};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(2);

type CropRect = (u32, u32, u32, u32);

/// Enables Per-Monitor V2 DPI awareness. Call once before creating any window.
pub fn init_process_dpi_awareness() {
    // SAFETY: callers invoke this before creating UI windows. Failure usually
    // means awareness was already fixed by the manifest or an earlier call.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[derive(Default)]
pub struct WindowsCapture;

impl WindowsCapture {
    pub const fn new() -> Self {
        Self
    }
}

impl CaptureBackend for WindowsCapture {
    fn monitors(&mut self) -> Result<Vec<MonitorInfo>> {
        enumerate_monitors()
    }

    fn windows(&mut self) -> Result<Vec<WindowInfo>> {
        enumerate_windows()
    }

    fn capture(&mut self, target: CaptureTarget) -> Result<RgbaFrame> {
        capture_target(target, CursorCaptureSettings::WithoutCursor)
    }

    fn capture_with_cursor(&mut self, target: CaptureTarget) -> Result<RgbaFrame> {
        capture_target(target, CursorCaptureSettings::WithCursor)
    }
}

fn capture_target(target: CaptureTarget, cursor: CursorCaptureSettings) -> Result<RgbaFrame> {
    match target {
        CaptureTarget::Monitor(id) => {
            ensure_monitor(id)?;
            capture_item(
                Monitor::from_raw_hmonitor(id.raw() as usize as *mut c_void),
                None,
                cursor,
                "capture monitor",
            )
        }
        CaptureTarget::Window(id) => {
            let window = Window::from_raw_hwnd(id.raw() as usize as *mut c_void);
            if !window.is_valid() {
                return Err(CaptureError::WindowNotFound(id));
            }
            capture_item(window, None, cursor, "capture window")
        }
        CaptureTarget::Region(region) => {
            if region.is_empty() {
                return Err(CaptureError::EmptyRegion);
            }
            ensure_monitor(region.monitor)?;

            let rect = region.rect;
            let left = i64::from(rect.origin.x);
            let top = i64::from(rect.origin.y);
            let right = left + i64::from(rect.size.width);
            let bottom = top + i64::from(rect.size.height);
            if left < 0 || top < 0 || right > i64::from(u32::MAX) || bottom > i64::from(u32::MAX) {
                return Err(CaptureError::RegionOutOfBounds);
            }

            capture_item(
                Monitor::from_raw_hmonitor(region.monitor.raw() as usize as *mut c_void),
                Some((left as u32, top as u32, right as u32, bottom as u32)),
                cursor,
                "capture region",
            )
        }
    }
}

thread_local! {
    static AUTOMATION: RefCell<Option<windows::Win32::UI::Accessibility::IUIAutomation>> =
        const { RefCell::new(None) };
}

fn automation() -> Option<windows::Win32::UI::Accessibility::IUIAutomation> {
    AUTOMATION.with(|slot| {
        if let Some(automation) = slot.borrow().as_ref() {
            return Some(automation.clone());
        }
        // SAFETY: COM initialization is idempotent for the same apartment;
        // failure usually means the caller already initialized COM with another
        // model, which UI Automation can still use.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // SAFETY: CUIAutomation is a registered in-process COM class.
        let automation = unsafe {
            CoCreateInstance::<_, windows::Win32::UI::Accessibility::IUIAutomation>(
                &CUIAutomation,
                None,
                CLSCTX_INPROC_SERVER,
            )
        }
        .ok()?;
        *slot.borrow_mut() = Some(automation.clone());
        Some(automation)
    })
}

/// Returns best-effort UI Automation descendant bounds for a window.
///
/// The overlay is excluded by querying from the target window handle instead
/// of the screen point, so a normal topmost overlay does not mask the tree.
pub fn ui_elements(id: WindowId) -> Vec<PhysicalRect> {
    let hwnd = HWND(id.raw() as usize as *mut c_void);
    let Some(automation) = automation() else {
        return Vec::new();
    };
    // SAFETY: hwnd belongs to a live window that is being captured.
    let Ok(root) = (unsafe { automation.ElementFromHandle(hwnd) }) else {
        return Vec::new();
    };
    // SAFETY: the condition is a fresh COM object owned by this call.
    let Ok(condition) = (unsafe { automation.CreateTrueCondition() }) else {
        return Vec::new();
    };
    // SAFETY: FindAll returns a new COM array; the scope only affects traversal.
    let Ok(elements) = (unsafe { root.FindAll(TreeScope_Descendants, &condition) }) else {
        return Vec::new();
    };
    // SAFETY: the array and its elements are live COM objects for this scope.
    let length = unsafe { elements.Length() }.unwrap_or(0).clamp(0, 512);
    let mut result = Vec::with_capacity(length as usize);
    for index in 0..length {
        // SAFETY: index is within the reported array length.
        let Ok(element) = (unsafe { elements.GetElement(index) }) else {
            continue;
        };
        // SAFETY: the element is a live COM object.
        let Ok(rect) = (unsafe { element.CurrentBoundingRectangle() }) else {
            continue;
        };
        let bounds = physical_rect(rect);
        if bounds.size.width > 1 && bounds.size.height > 1 {
            result.push(bounds);
        }
    }
    result
}

/// Returns the window's effective corner radius in physical pixels.
///
/// The DWM corner preference was added in Windows 11. Windows 10 returns an
/// error for the attribute and keeps square corners.
pub fn window_corner_radius(id: WindowId, scale: f32) -> u32 {
    let hwnd = HWND(id.raw() as usize as *mut c_void);
    // Maximized windows use square corners even when DWM reports rounding.
    // SAFETY: hwnd belongs to the live window that is being captured.
    if unsafe { IsZoomed(hwnd) }.as_bool() {
        return 0;
    }
    let mut preference = DWM_WINDOW_CORNER_PREFERENCE::default();
    // SAFETY: the output buffer matches the requested attribute size, and hwnd
    // belongs to a live window that is about to be captured.
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw mut preference).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    };
    if result.is_err() {
        return 0;
    }
    let points = if preference == DWMWCP_DONOTROUND {
        0.0
    } else if preference == DWMWCP_ROUNDSMALL {
        4.0
    } else {
        8.0
    };
    (points * scale.max(0.0)).round() as u32
}

fn ensure_monitor(id: MonitorId) -> Result<()> {
    if enumerate_monitors()?.iter().any(|monitor| monitor.id == id) {
        Ok(())
    } else {
        Err(CaptureError::MonitorNotFound(id))
    }
}

fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    let mut monitors = Vec::new();
    // SAFETY: the callback only writes to the Vec borrowed for the duration of
    // the call, and EnumDisplayMonitors runs callbacks synchronously.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_callback),
            LPARAM(&raw mut monitors as isize),
        )
        .ok()
        .map_err(|error| platform_error("EnumDisplayMonitors", error))?;
    }

    if monitors.is_empty() {
        return Err(CaptureError::Platform {
            operation: "EnumDisplayMonitors",
            message: "no monitors were reported".to_owned(),
        });
    }

    Ok(monitors)
}

unsafe extern "system" fn enum_monitor_callback(
    monitor: HMONITOR,
    _device_context: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` points to the Vec owned by enumerate_monitors.
    let monitors = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
    // SAFETY: the HMONITOR is valid for the duration of the enumeration callback.
    if let Some(info) = unsafe { monitor_info(monitor) } {
        monitors.push(info);
    }
    TRUE
}

unsafe fn monitor_info(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        },
        szDevice: [0; 32],
    };

    // SAFETY: `info` is a valid MONITORINFOEXW with cbSize set.
    if !unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) }.as_bool() {
        return None;
    }

    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    // SAFETY: the monitor handle comes from EnumDisplayMonitors.
    let scale = if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .is_ok()
    {
        DpiScale::new(dpi_x as f32 / 96.0).unwrap_or(DpiScale::ONE)
    } else {
        DpiScale::ONE
    };

    Some(MonitorInfo {
        id: MonitorId::from_raw(monitor.0 as usize as u64),
        bounds: physical_rect(info.monitorInfo.rcMonitor),
        scale,
        is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

fn enumerate_windows() -> Result<Vec<WindowInfo>> {
    let mut handles: Vec<HWND> = Vec::new();
    // EnumWindows walks top-level windows in z-order, topmost first.
    // windows_capture::Window::enumerate is not used because it rejects every
    // WS_EX_TOOLWINDOW, which includes the taskbar and notification area.
    // SAFETY: the callback only writes to the Vec borrowed for the duration of
    // the call, and EnumWindows runs callbacks synchronously.
    unsafe {
        EnumWindows(
            Some(enum_window_callback),
            LPARAM(&raw mut handles as isize),
        )
    }
    .map_err(|error| platform_error("EnumWindows", error))?;

    let shell_window = unsafe { GetShellWindow() };
    let desktop_window = unsafe { GetDesktopWindow() };

    let mut result = Vec::with_capacity(handles.len());
    for hwnd in handles {
        if hwnd == shell_window || hwnd == desktop_window {
            continue;
        }
        // SAFETY: hwnd came from a live window enumeration.
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            continue;
        }
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        if style & WS_CHILD.0 != 0 {
            continue;
        }
        let taskbar = is_taskbar(hwnd);
        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 && !taskbar {
            continue;
        }
        if unsafe { IsIconic(hwnd) }.as_bool() {
            continue;
        }
        if is_cloaked(hwnd) || is_shell_surface(hwnd) || is_capture_excluded(hwnd) {
            continue;
        }

        let Some(bounds) = window_bounds(hwnd) else {
            continue;
        };
        if bounds.is_empty() {
            continue;
        }

        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let monitor_id =
            (!monitor.0.is_null()).then(|| MonitorId::from_raw(monitor.0 as usize as u64));
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = DpiScale::new(dpi as f32 / 96.0).unwrap_or(DpiScale::ONE);

        result.push(WindowInfo {
            id: WindowId::from_raw(hwnd.0 as usize as u64),
            bounds,
            title: window_title(hwnd),
            monitor: monitor_id,
            scale,
        });
    }

    Ok(result)
}

unsafe extern "system" fn enum_window_callback(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` points to the Vec owned by enumerate_windows.
    let handles = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    handles.push(hwnd);
    TRUE
}

fn window_title(hwnd: HWND) -> String {
    // SAFETY: hwnd belongs to a live window; the buffer length is queried
    // first and the returned string is copied into the caller-owned buffer.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: the buffer is writable for its full length.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..copied as usize])
}

fn window_class(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 64];
    // SAFETY: the buffer is writable for its full length.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

fn is_taskbar(hwnd: HWND) -> bool {
    window_class(hwnd)
        .is_some_and(|class| matches!(class.as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"))
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: the output buffer matches the requested attribute size.
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        )
    };
    result.is_ok() && cloaked != 0
}

fn is_shell_surface(hwnd: HWND) -> bool {
    // The taskbar is a valid capture target. Only the desktop host windows
    // are excluded; their content is the wallpaper, not a window the user
    // can pick.
    window_class(hwnd).is_some_and(|class| matches!(class.as_str(), "Progman" | "WorkerW"))
}

/// Windows marked with `WDA_EXCLUDEFROMCAPTURE` cannot appear in a capture
/// (the overlay, capture bars and pin menus use this). They must not be
/// offered as hover targets, or a click would capture black content.
fn is_capture_excluded(hwnd: HWND) -> bool {
    let mut affinity = 0u32;
    // SAFETY: the affinity value is a caller-owned output buffer.
    unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.is_ok()
        && affinity == WDA_EXCLUDEFROMCAPTURE.0
}

fn window_bounds(hwnd: HWND) -> Option<PhysicalRect> {
    let mut rect = RECT::default();
    // SAFETY: the output buffer matches the requested attribute size.
    let dwm_result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            size_of::<RECT>() as u32,
        )
    };

    if dwm_result.is_err() {
        // SAFETY: hwnd came from a live window enumeration.
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            return None;
        }
    }

    Some(physical_rect(rect))
}

fn physical_rect(rect: RECT) -> PhysicalRect {
    let width = (i64::from(rect.right) - i64::from(rect.left)).clamp(0, i64::from(u32::MAX)) as u32;
    let height =
        (i64::from(rect.bottom) - i64::from(rect.top)).clamp(0, i64::from(u32::MAX)) as u32;
    PhysicalRect::new(rect.left, rect.top, width, height)
}

struct CaptureFlags {
    sender: mpsc::Sender<std::result::Result<RawFrame, String>>,
    crop: Option<CropRect>,
}

struct OneShotHandler {
    sender: Option<mpsc::Sender<std::result::Result<RawFrame, String>>>,
    crop: Option<CropRect>,
}

struct RawFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl GraphicsCaptureApiHandler for OneShotHandler {
    type Flags = CaptureFlags;
    type Error = String;

    fn new(context: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            sender: Some(context.flags.sender),
            crop: context.flags.crop,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let result = read_frame(frame, self.crop);
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(result);
        }
        capture_control.stop();
        Ok(())
    }
}

fn read_frame(frame: &mut Frame, crop: Option<CropRect>) -> std::result::Result<RawFrame, String> {
    let mut scratch = Vec::new();
    let (width, height, pixels) = match crop {
        Some((left, top, right, bottom)) => {
            if right > frame.width() || bottom > frame.height() {
                return Err("capture region exceeds the captured frame".to_owned());
            }
            let buffer = frame
                .buffer_crop(left, top, right, bottom)
                .map_err(|error| error.to_string())?;
            let width = buffer.width();
            let height = buffer.height();
            let pixels = buffer.as_nopadding_buffer(&mut scratch).to_vec();
            (width, height, pixels)
        }
        None => {
            let buffer = frame.buffer().map_err(|error| error.to_string())?;
            let width = buffer.width();
            let height = buffer.height();
            let pixels = buffer.as_nopadding_buffer(&mut scratch).to_vec();
            (width, height, pixels)
        }
    };

    Ok(RawFrame {
        width,
        height,
        pixels,
    })
}

fn capture_item<T>(
    item: T,
    crop: Option<CropRect>,
    cursor: CursorCaptureSettings,
    operation: &'static str,
) -> Result<RgbaFrame>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let settings = Settings::new(
        item,
        cursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Exclude,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        CaptureFlags { sender, crop },
    );

    let control: CaptureControl<OneShotHandler, String> =
        OneShotHandler::start_free_threaded(settings)
            .map_err(|error| platform_error(operation, error))?;

    match receiver.recv_timeout(FIRST_FRAME_TIMEOUT) {
        Ok(Ok(frame)) => {
            let _ = control.stop();
            RgbaFrame::from_pixels(frame.width, frame.height, frame.pixels)
                .map_err(CaptureError::from)
        }
        Ok(Err(message)) => {
            let _ = control.stop();
            Err(CaptureError::Platform { operation, message })
        }
        Err(_) => {
            let _ = control.stop();
            Err(CaptureError::Platform {
                operation,
                message: "timed out waiting for the first frame".to_owned(),
            })
        }
    }
}

fn platform_error(operation: &'static str, error: impl std::fmt::Display) -> CaptureError {
    CaptureError::Platform {
        operation,
        message: error.to_string(),
    }
}
