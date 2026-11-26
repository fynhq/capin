use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use core_types::{DpiScale, MonitorId, PhysicalPoint, PhysicalRect, RgbaFrame, WindowId};
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
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationCondition,
    IUIAutomationElement, TreeScope_Children, TreeScope_Descendants, TreeScope_Element,
    UIA_BoundingRectanglePropertyId,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForWindow,
    MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetDesktopWindow,
    GetShellWindow, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    IsZoomed, MONITORINFOF_PRIMARY, WDA_EXCLUDEFROMCAPTURE, WS_CHILD, WS_EX_TOOLWINDOW,
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

use core_capture::{CaptureBackend, CaptureError, MonitorInfo, Result, WindowInfo};
use core_types::CaptureTarget;

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(2);

type CropRect = (u32, u32, u32, u32);

pub fn init_process_dpi_awareness() {
    // SAFETY: callers invoke this before creating UI windows; failure means awareness was already fixed.
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

const UIA_TIMEOUT_MS: u32 = 1_500;
const UIA_MAX_ELEMENTS: i32 = 16_384;
const CHILD_WINDOW_LIMIT: usize = 64;
const CHAIN_MAX_DEPTH: usize = 32;
const CHAIN_BUDGET: Duration = Duration::from_millis(300);

#[derive(Clone)]
struct Automation {
    automation: IUIAutomation2,
    cache: IUIAutomationCacheRequest,
}

thread_local! {
    static AUTOMATION: RefCell<Option<Automation>> = const { RefCell::new(None) };
}

fn automation() -> Option<Automation> {
    AUTOMATION.with(|slot| {
        if let Some(automation) = slot.borrow().as_ref() {
            return Some(automation.clone());
        }
        // SAFETY: COM is initialized on this thread and every created object is owned by the returned Automation.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let automation =
                CoCreateInstance::<_, IUIAutomation2>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                    .ok()?;
            let _ = automation.SetConnectionTimeout(UIA_TIMEOUT_MS);
            let _ = automation.SetTransactionTimeout(UIA_TIMEOUT_MS);
            let cache = automation.CreateCacheRequest().ok()?;
            let _ = cache.AddProperty(UIA_BoundingRectanglePropertyId);
            let _ = cache.SetTreeScope(TreeScope_Element);
            let automation = Automation { automation, cache };
            *slot.borrow_mut() = Some(automation.clone());
            Some(automation)
        }
    })
}

fn element_condition(automation: &IUIAutomation2) -> Option<IUIAutomationCondition> {
    // SAFETY: the automation object is initialized and owned by this thread.
    unsafe { automation.CreateTrueCondition().ok() }
}

fn belongs_to_current_process(hwnd: HWND) -> bool {
    let mut pid = 0;
    // SAFETY: hwnd is a live handle and pid is a writable local.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid != 0 && pid == GetCurrentProcessId()
    }
}

pub fn ui_elements(id: WindowId) -> Vec<PhysicalRect> {
    let hwnd = HWND(id.raw() as usize as *mut c_void);
    let mut result = if belongs_to_current_process(hwnd) {
        Vec::new()
    } else {
        uia_elements(hwnd)
    };
    if result.len() < 2 {
        result.extend(child_window_rects(hwnd));
        result.sort_by_key(|rect| {
            (
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            )
        });
        result.dedup();
    }
    result
}

pub fn ui_element_chain(id: WindowId, point: PhysicalPoint) -> Vec<PhysicalRect> {
    let hwnd = HWND(id.raw() as usize as *mut c_void);
    if belongs_to_current_process(hwnd) {
        return Vec::new();
    }
    uia_element_chain(hwnd, point)
}

fn uia_element_chain(hwnd: HWND, point: PhysicalPoint) -> Vec<PhysicalRect> {
    let Some(Automation { automation, cache }) = automation() else {
        return Vec::new();
    };
    let Some(condition) = element_condition(&automation) else {
        return Vec::new();
    };
    let started = Instant::now();
    let mut chain = Vec::new();
    // SAFETY: the calls use live COM objects owned here and indices bounded by the child count.
    unsafe {
        let Ok(mut current) = automation.ElementFromHandle(hwnd) else {
            return Vec::new();
        };
        for _ in 0..CHAIN_MAX_DEPTH {
            if started.elapsed() > CHAIN_BUDGET {
                break;
            }
            let Ok(children) = current.FindAllBuildCache(TreeScope_Children, &condition, &cache)
            else {
                break;
            };
            let length = children.Length().unwrap_or(0).clamp(0, UIA_MAX_ELEMENTS);
            let mut best: Option<(IUIAutomationElement, PhysicalRect)> = None;
            for index in 0..length {
                let Ok(child) = children.GetElement(index) else {
                    continue;
                };
                let Ok(rect) = child.CachedBoundingRectangle() else {
                    continue;
                };
                let bounds = physical_rect(rect);
                if bounds.size.width <= 1 || bounds.size.height <= 1 || !bounds.contains(point) {
                    continue;
                }
                let area = u64::from(bounds.size.width) * u64::from(bounds.size.height);
                let replace = best.as_ref().is_none_or(|(_, best)| {
                    u64::from(best.size.width) * u64::from(best.size.height) > area
                });
                if replace {
                    best = Some((child, bounds));
                }
            }
            let Some((child, bounds)) = best else {
                break;
            };
            chain.push(bounds);
            current = child;
        }
    }
    chain.sort_by_key(|rect| u64::from(rect.size.width) * u64::from(rect.size.height));
    chain.dedup();
    chain
}

fn uia_elements(hwnd: HWND) -> Vec<PhysicalRect> {
    let Some(Automation { automation, cache }) = automation() else {
        return Vec::new();
    };
    let Some(condition) = element_condition(&automation) else {
        return Vec::new();
    };
    // SAFETY: the calls use live COM objects owned here and indices bounded by the reported length.
    unsafe {
        let Ok(root) = automation.ElementFromHandle(hwnd) else {
            return Vec::new();
        };
        let Ok(elements) = root.FindAllBuildCache(TreeScope_Descendants, &condition, &cache) else {
            return Vec::new();
        };
        let length = elements.Length().unwrap_or(0).clamp(0, UIA_MAX_ELEMENTS);
        let mut result = Vec::with_capacity(length as usize);
        for index in 0..length {
            let Ok(element) = elements.GetElement(index) else {
                continue;
            };
            let Ok(rect) = element.CachedBoundingRectangle() else {
                continue;
            };
            let bounds = physical_rect(rect);
            if bounds.size.width > 1 && bounds.size.height > 1 {
                result.push(bounds);
            }
        }
        result
    }
}

fn child_window_rects(hwnd: HWND) -> Vec<PhysicalRect> {
    let mut rects = Vec::new();
    // SAFETY: the callback only writes to the Vec borrowed for the duration of the call.
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(child_window_callback),
            LPARAM(&raw mut rects as isize),
        );
    }
    rects
}

unsafe extern "system" fn child_window_callback(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: data points at the Vec borrowed for the duration of EnumChildWindows.
    let rects = unsafe { &mut *(data.0 as *mut Vec<PhysicalRect>) };
    if rects.len() >= CHILD_WINDOW_LIMIT {
        return TRUE;
    }
    // SAFETY: hwnd is a live child window supplied by the enumeration.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return TRUE;
        }
        let bounds = physical_rect(rect);
        if bounds.size.width > 4 && bounds.size.height > 4 {
            rects.push(bounds);
        }
    }
    TRUE
}

pub fn window_corner_radius(id: WindowId, scale: f32) -> u32 {
    let hwnd = HWND(id.raw() as usize as *mut c_void);
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
    window_class(hwnd).is_some_and(|class| matches!(class.as_str(), "Progman" | "WorkerW"))
}

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

const WARM_STREAM_IDLE: Duration = Duration::from_secs(15);
const WARM_STREAM_CHECK: Duration = Duration::from_secs(5);

struct WarmStream {
    stream: MonitorStream,
    idle_since: Instant,
}

static WARM_STREAMS: OnceLock<Mutex<HashMap<MonitorId, WarmStream>>> = OnceLock::new();
static WARM_STREAM_WATCH: OnceLock<()> = OnceLock::new();
static WARM_STREAM_SEEDING: OnceLock<Mutex<HashSet<MonitorId>>> = OnceLock::new();

fn warm_streams() -> &'static Mutex<HashMap<MonitorId, WarmStream>> {
    WARM_STREAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn warm_stream_seeding() -> &'static Mutex<HashSet<MonitorId>> {
    WARM_STREAM_SEEDING.get_or_init(|| Mutex::new(HashSet::new()))
}

fn start_warm_stream_watch() {
    WARM_STREAM_WATCH.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("capin-capture-warm".to_owned())
            .spawn(|| {
                loop {
                    std::thread::sleep(WARM_STREAM_CHECK);
                    let expired = {
                        let Ok(mut streams) = warm_streams().lock() else {
                            continue;
                        };
                        let now = Instant::now();
                        let ids: Vec<MonitorId> = streams
                            .iter()
                            .filter(|(_, cached)| {
                                now.saturating_duration_since(cached.idle_since) >= WARM_STREAM_IDLE
                            })
                            .map(|(id, _)| *id)
                            .collect();
                        ids.into_iter()
                            .filter_map(|id| streams.remove(&id))
                            .collect::<Vec<_>>()
                    };
                    drop(expired);
                }
            });
    });
}

pub fn take_monitor_stream(monitor: MonitorId) -> Option<MonitorStream> {
    warm_streams()
        .lock()
        .ok()?
        .remove(&monitor)
        .map(|cached| cached.stream)
}

pub fn cache_monitor_stream(monitor: MonitorId, stream: MonitorStream) {
    start_warm_stream_watch();
    let stale = warm_streams().lock().ok().and_then(|mut streams| {
        streams.insert(
            monitor,
            WarmStream {
                stream,
                idle_since: Instant::now(),
            },
        )
    });
    drop(stale);
}

pub fn seed_monitor_stream(monitor: MonitorId) {
    let cached = warm_streams()
        .lock()
        .map(|streams| streams.contains_key(&monitor))
        .unwrap_or(true);
    if cached {
        return;
    }
    let Ok(mut seeding) = warm_stream_seeding().lock() else {
        return;
    };
    if !seeding.insert(monitor) {
        return;
    }
    drop(seeding);
    let spawned = std::thread::Builder::new()
        .name("capin-capture-seed".to_owned())
        .spawn(move || {
            if let Ok(stream) = MonitorStream::start(monitor) {
                let cached = warm_streams()
                    .lock()
                    .map(|streams| streams.contains_key(&monitor))
                    .unwrap_or(true);
                if !cached {
                    cache_monitor_stream(monitor, stream);
                }
            }
            if let Ok(mut seeding) = warm_stream_seeding().lock() {
                seeding.remove(&monitor);
            }
        });
    if spawned.is_err()
        && let Ok(mut seeding) = warm_stream_seeding().lock()
    {
        seeding.remove(&monitor);
    }
}

pub struct MonitorStream {
    control: Option<CaptureControl<StreamHandler, String>>,
    shared: Arc<StreamShared>,
}

struct StreamShared {
    request: AtomicBool,
    frame: Mutex<Option<RawFrame>>,
    error: Mutex<Option<String>>,
}

struct StreamHandler {
    shared: Arc<StreamShared>,
}

impl GraphicsCaptureApiHandler for StreamHandler {
    type Flags = Arc<StreamShared>;
    type Error = String;

    fn new(context: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            shared: context.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let requested = self.shared.request.swap(false, Ordering::AcqRel);
        if !requested {
            return Ok(());
        }
        match read_frame(frame, None) {
            Ok(raw) => {
                if let Ok(mut slot) = self.shared.frame.lock() {
                    *slot = Some(raw);
                }
            }
            Err(error) => {
                if let Ok(mut slot) = self.shared.error.lock() {
                    *slot = Some(error);
                }
            }
        }
        Ok(())
    }
}

impl MonitorStream {
    pub fn start(monitor: MonitorId) -> Result<Self> {
        ensure_monitor(monitor)?;
        let shared = Arc::new(StreamShared {
            request: AtomicBool::new(false),
            frame: Mutex::new(None),
            error: Mutex::new(None),
        });
        let settings = Settings::new(
            Monitor::from_raw_hmonitor(monitor.raw() as usize as *mut c_void),
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Exclude,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            Arc::clone(&shared),
        );
        let control = StreamHandler::start_free_threaded(settings)
            .map_err(|error| platform_error("start monitor stream", error))?;
        Ok(Self {
            control: Some(control),
            shared,
        })
    }

    pub fn refresh(&mut self) -> Result<Option<RgbaFrame>> {
        if self.control.is_none() {
            return Err(CaptureError::Platform {
                operation: "monitor stream",
                message: "stream is closed".to_owned(),
            });
        }
        if let Ok(mut slot) = self.shared.frame.lock()
            && let Some(raw) = slot.take()
        {
            return RgbaFrame::from_pixels(raw.width, raw.height, raw.pixels)
                .map(Some)
                .map_err(CaptureError::from);
        }
        if let Ok(mut slot) = self.shared.error.lock() {
            *slot = None;
        }
        self.shared.request.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_millis(150);
        loop {
            if let Ok(mut slot) = self.shared.frame.lock()
                && let Some(raw) = slot.take()
            {
                return RgbaFrame::from_pixels(raw.width, raw.height, raw.pixels)
                    .map(Some)
                    .map_err(CaptureError::from);
            }
            if let Ok(mut slot) = self.shared.error.lock()
                && let Some(message) = slot.take()
            {
                return Err(CaptureError::Platform {
                    operation: "monitor stream",
                    message,
                });
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}

impl Drop for MonitorStream {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
    }
}

fn platform_error(operation: &'static str, error: impl std::fmt::Display) -> CaptureError {
    CaptureError::Platform {
        operation,
        message: error.to_string(),
    }
}
