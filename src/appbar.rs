//! Windows AppBar registration. Tells the shell "this window owns a strip of
//! the screen edge"; once registered, maximised windows stop short of the bar
//! instead of going under it.
//!
//! No-op on non-Windows targets. macOS has no equivalent shell-level
//! reservation API — the closest is reading NSScreen.visibleFrame to
//! position the bar within the existing available area, which `main.rs`
//! does via `screen_insets_top_bottom`. macOS apps simply float over
//! each other; we don't get to claim a strip of pixels for ourselves.

use eframe::Frame;

use crate::wake::Waker;

#[derive(Clone, Copy, Debug)]
pub enum Edge {
    Top,
    // Wired up when bar.position becomes configurable (commit 20).
    #[allow(dead_code)]
    Bottom,
}

#[cfg(windows)]
pub use imp::AppBar;

#[cfg(not(windows))]
pub use stub::AppBar;

/// Try to register an AppBar reservation along the chosen edge. Returns None
/// if we can't extract an HWND from the eframe Frame yet (the first few
/// frames may not have one) or if any SHAppBarMessage call fails. On
/// success, also arms the Waker with the bar's HWND so background threads
/// can wake the eframe loop via InvalidateRect.
pub fn register(frame: &Frame, edge: Edge, height: i32, waker: &Waker) -> Option<AppBar> {
    AppBar::try_register(frame, edge, height, waker)
}

/// Re-assert the borderless tool-window styling on the bar's HWND. winit
/// re-applies its own window styles the first time a viewport is shown, which
/// clobbers the marking done during `register` (we create the window hidden
/// and mark it before revealing, to dodge the tiling-WM startup race). Calling
/// this for a few frames after reveal restores `WS_EX_TOOLWINDOW` so the bar
/// stays out of the taskbar, Alt+Tab, and a tiling WM's managed set. No-op
/// before the HWND exists and on non-Windows.
#[cfg(windows)]
pub fn reassert_toolwindow(frame: &Frame) {
    imp::reassert_toolwindow(frame);
}

#[cfg(not(windows))]
pub fn reassert_toolwindow(_frame: &Frame) {}

/// Physical-pixel description of one display, used to place and reserve a bar
/// on each monitor. Coordinates live in the virtual-desktop physical pixel
/// space that SHAppBarMessage and SetWindowPos expect.
#[derive(Clone, Debug)]
pub struct MonitorInfo {
    /// GDI device name, e.g. `\\.\DISPLAY1`. Doubles as the per-monitor child
    /// window title so the HWND can be recovered after eframe creates it.
    pub device_name: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    /// Effective DPI (96 = 100%).
    pub dpi: u32,
    pub is_primary: bool,
}

/// Enumerate every monitor with its physical bounds and DPI. Empty on
/// non-Windows.
#[cfg(windows)]
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    imp::enumerate_monitors()
}

#[cfg(not(windows))]
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

/// Register an AppBar reservation for a secondary monitor's bar. The child
/// viewport window is found by its unique title (eframe exposes no handle for
/// child viewports), marked as a tool window, and pinned to `monitor`'s edge.
/// Returns None until the window exists or if any shell call fails.
#[cfg(windows)]
pub fn register_on_monitor(
    title: &str,
    edge: Edge,
    height: i32,
    monitor: &MonitorInfo,
) -> Option<AppBar> {
    imp::register_on_monitor(title, edge, height, monitor)
}

#[cfg(not(windows))]
pub fn register_on_monitor(
    _title: &str,
    _edge: Edge,
    _height: i32,
    _monitor: &MonitorInfo,
) -> Option<AppBar> {
    None
}

/// The per-monitor analogue of `reassert_toolwindow`: re-apply the tool-window
/// styling on a child bar found by its title.
#[cfg(windows)]
pub fn reassert_toolwindow_by_title(title: &str) {
    imp::reassert_toolwindow_by_title(title);
}

#[cfg(not(windows))]
pub fn reassert_toolwindow_by_title(_title: &str) {}

/// Re-apply a child bar's physical rect on its monitor (window move only, no
/// shell re-registration). winit honours WM_DPICHANGED when the window first
/// lands on a different-DPI monitor and rescales it by the DPI ratio; calling
/// this once it has settled there pins it back to the intended physical size.
/// No-op before the window exists / on non-Windows.
#[cfg(windows)]
pub fn reposition_on_monitor(title: &str, edge: Edge, height: i32, monitor: &MonitorInfo) {
    imp::reposition_on_monitor(title, edge, height, monitor);
}

#[cfg(not(windows))]
pub fn reposition_on_monitor(_title: &str, _edge: Edge, _height: i32, _monitor: &MonitorInfo) {}

#[cfg(windows)]
mod imp {
    use super::{Edge, Frame, MonitorInfo, Waker};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{FALSE, HWND, LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows::core::BOOL;

    /// `MONITORINFOF_PRIMARY` is not re-exported by the `windows` 0.61 Gdi
    /// module, so use its documented value.
    const MONITORINFOF_PRIMARY: u32 = 0x1;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
        SHAppBarMessage,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetSystemMetrics, GetWindowLongPtrW,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, SM_CXSCREEN, SM_CYSCREEN,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW,
        SetWindowPos, WS_CAPTION, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
        WS_SYSMENU, WS_THICKFRAME,
    };

    pub struct AppBar {
        hwnd: HWND,
    }

    impl AppBar {
        pub fn try_register(frame: &Frame, edge: Edge, height: i32, waker: &Waker) -> Option<Self> {
            let hwnd = hwnd_from_frame(frame)?;
            // Mark the bar as a Win32 tool window *before* AppBar
            // registration. This excludes it from the taskbar, from
            // Alt+Tab, and — crucially — from tiling window managers
            // like GlazeWM which key off the standard "is this a
            // managed app?" Win32 hints to decide what to lay out.
            // Without this, GlazeWM treats wbar as just another app
            // and tries to tile it into the centre of the screen.
            // SAFETY: hwnd is a live window we just received from the
            // eframe viewport in this process.
            unsafe { mark_as_toolwindow(hwnd) };
            // The root viewport always lives on the primary monitor; reserve
            // its strip using the primary metrics and the window's own DPI.
            // SAFETY: SHAppBarMessage and SetWindowPos take a valid HWND
            // owned by this process.
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            let appbar =
                unsafe { do_register_bounds(hwnd, edge, height, 0, 0, screen_w, screen_h, dpi) }?;
            // Arm the cross-thread waker now that we know the HWND.
            // Background threads (tray handler, IPC, glazewm reconnect)
            // call waker.wake() to InvalidateRect on this HWND, which
            // queues a WM_PAINT that winit turns into App::update.
            waker.set_hwnd(hwnd.0 as isize);
            Some(appbar)
        }
    }

    /// Make the bar's HWND a true borderless tool window:
    ///   - add `WS_EX_TOOLWINDOW` (excludes it from the taskbar, Alt+Tab,
    ///     and from tiling WMs like GlazeWM which key off the standard
    ///     "is this a managed app?" Win32 hints),
    ///   - strip `WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MIN/MAXBOX`
    ///     and add `WS_POPUP`. winit's `with_decorations(false)` only
    ///     overrides `WM_NCCALCSIZE` to collapse the visible non-client
    ///     area — it leaves the chrome style bits in place. That works
    ///     visually, but `AdjustWindowRectEx` is a pure function: it
    ///     computes chrome offsets from the style bits alone, ignoring
    ///     `WM_NCCALCSIZE`. So any code path inside eframe/winit that
    ///     translates an inner rect to an outer rect (e.g. deferred
    ///     application of `ViewportBuilder::with_inner_size`) will shift
    ///     the window by ~84 physical px at 150 % DPI. Stripping the
    ///     chrome bits makes `AdjustWindowRectEx` a no-op.
    ///
    /// SAFETY: caller guarantees `hwnd` is a live, owned window.
    unsafe fn mark_as_toolwindow(hwnd: HWND) {
        let ex_current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let ex_new = ex_current | (WS_EX_TOOLWINDOW.0 as isize);
        let chrome =
            (WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MAXIMIZEBOX.0 | WS_MINIMIZEBOX.0)
                as isize;
        let style_current = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
        let style_new = (style_current & !chrome) | (WS_POPUP.0 as isize);
        if ex_current == ex_new && style_current == style_new {
            return;
        }
        if ex_current != ex_new {
            unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_new) };
        }
        if style_current != style_new {
            unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, style_new) };
        }
        // SetWindowLongPtrW takes effect on the next SetWindowPos with
        // SWP_FRAMECHANGED. Force it now so the change is visible to
        // the shell immediately (do_register's SetWindowPos that
        // follows doesn't carry SWP_FRAMECHANGED). SWP_NOMOVE | SWP_NOSIZE
        // make this apply *only* the frame change: without them the 0,0,0,0
        // args would collapse the window to zero size, which is harmless when
        // do_register repositions right after but breaks a standalone
        // re-assert (reassert_toolwindow) that has no following SetWindowPos.
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        tracing::info!(
            ex_before = format!("0x{:X}", ex_current),
            ex_after = format!("0x{:X}", ex_new),
            style_before = format!("0x{:X}", style_current),
            style_after = format!("0x{:X}", style_new),
            "marked bar HWND as borderless tool window",
        );
    }

    impl Drop for AppBar {
        fn drop(&mut self) {
            let mut abd = empty_abd(self.hwnd);
            // SAFETY: ABM_REMOVE only reads cbSize and hWnd from the struct.
            unsafe { SHAppBarMessage(ABM_REMOVE, &mut abd) };
            tracing::info!("appbar removed");
        }
    }

    fn hwnd_from_frame(frame: &Frame) -> Option<HWND> {
        let handle = frame.window_handle().ok()?;
        if let RawWindowHandle::Win32(h) = handle.as_raw() {
            Some(HWND(h.hwnd.get() as *mut _))
        } else {
            None
        }
    }

    pub fn reassert_toolwindow(frame: &Frame) {
        if let Some(hwnd) = hwnd_from_frame(frame) {
            // SAFETY: hwnd is a live window owned by this process's eframe
            // viewport. mark_as_toolwindow self-checks and no-ops once the
            // styling already matches.
            unsafe { mark_as_toolwindow(hwnd) };
        }
    }

    fn empty_abd(hwnd: HWND) -> APPBARDATA {
        APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: 0,
            uEdge: 0,
            rc: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            lParam: LPARAM(0),
        }
    }

    /// Register and pin an AppBar to one monitor's edge. `left/top/right/bottom`
    /// are the monitor's physical bounds and `dpi` its effective DPI.
    ///
    /// SHAppBarMessage and SetWindowPos work in *physical* pixels (the process
    /// is per-monitor-DPI aware via eframe). bar.height in config is *logical*
    /// pixels; eframe renders the window at logical_height × dpi_scale physical
    /// pixels. Scaling here keeps the reservation matching the painted bar, so
    /// a hi-DPI display doesn't leave a thin strip where the bar paints but
    /// apps still tile (at 1.25x a 28-logical bar is 35 physical px).
    ///
    /// SAFETY: caller guarantees `hwnd` is a live, owned window.
    #[allow(clippy::too_many_arguments)]
    unsafe fn do_register_bounds(
        hwnd: HWND,
        edge: Edge,
        logical_height: i32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        dpi: u32,
    ) -> Option<AppBar> {
        let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
        let physical_height = (logical_height as f32 * scale).round() as i32;

        let mut abd = empty_abd(hwnd);
        abd.uEdge = match edge {
            Edge::Top => ABE_TOP,
            Edge::Bottom => ABE_BOTTOM,
        };

        // ABM_NEW just registers; rc is ignored.
        if unsafe { SHAppBarMessage(ABM_NEW, &mut abd) } == 0 {
            tracing::warn!("ABM_NEW failed");
            return None;
        }

        // Propose a rect spanning this monitor's full width, at the edge.
        abd.rc.left = left;
        abd.rc.right = right;
        match edge {
            Edge::Top => {
                abd.rc.top = top;
                abd.rc.bottom = top + physical_height;
            }
            Edge::Bottom => {
                abd.rc.top = bottom - physical_height;
                abd.rc.bottom = bottom;
            }
        }

        // ABM_QUERYPOS lets Windows adjust the rect (e.g. dodge other appbars).
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut abd) };

        // Re-pin the height after QUERYPOS may have widened the rect along the
        // perpendicular axis.
        match edge {
            Edge::Top => abd.rc.bottom = abd.rc.top + physical_height,
            Edge::Bottom => abd.rc.top = abd.rc.bottom - physical_height,
        }

        unsafe { SHAppBarMessage(ABM_SETPOS, &mut abd) };

        // SHAppBarMessage doesn't move our window — that's on us.
        let w = abd.rc.right - abd.rc.left;
        let h = abd.rc.bottom - abd.rc.top;
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                abd.rc.left,
                abd.rc.top,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };

        tracing::info!(
            edge = ?edge,
            left = abd.rc.left,
            top = abd.rc.top,
            width = w,
            physical_height = h,
            logical_height,
            dpi,
            "appbar registered",
        );

        Some(AppBar { hwnd })
    }

    pub fn enumerate_monitors() -> Vec<MonitorInfo> {
        let mut out: Vec<MonitorInfo> = Vec::new();
        // SAFETY: monitor_enum_proc runs only for the duration of this call and
        // dereferences the &mut Vec we pass through LPARAM.
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(monitor_enum_proc),
                LPARAM(&mut out as *mut Vec<MonitorInfo> as isize),
            );
        }
        out
    }

    unsafe extern "system" fn monitor_enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Vec<MonitorInfo>) };
        let mut mie = MONITORINFOEXW::default();
        mie.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let ok =
            unsafe { GetMonitorInfoW(hmon, &mut mie.monitorInfo as *mut MONITORINFO).as_bool() };
        if ok {
            let mut dx: u32 = 96;
            let mut dy: u32 = 96;
            // Best-effort; defaults to 96 (100%) if the call fails.
            let _ = unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
            let device = String::from_utf16_lossy(&mie.szDevice);
            let device = device.trim_end_matches('\0').to_string();
            let r = mie.monitorInfo.rcMonitor;
            out.push(MonitorInfo {
                device_name: device,
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
                dpi: dx,
                is_primary: (mie.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
            });
        }
        TRUE
    }

    pub fn register_on_monitor(
        title: &str,
        edge: Edge,
        height: i32,
        monitor: &MonitorInfo,
    ) -> Option<AppBar> {
        let hwnd = find_hwnd_by_title(title)?;
        // SAFETY: hwnd is a live window in this process, found by its unique
        // title. Child viewports don't arm the cross-thread waker (the root's
        // single HWND already drives the shared eframe loop, which renders
        // every immediate child viewport inline).
        unsafe { mark_as_toolwindow(hwnd) };
        unsafe {
            do_register_bounds(
                hwnd,
                edge,
                height,
                monitor.left,
                monitor.top,
                monitor.right,
                monitor.bottom,
                monitor.dpi,
            )
        }
    }

    pub fn reassert_toolwindow_by_title(title: &str) {
        if let Some(hwnd) = find_hwnd_by_title(title) {
            // SAFETY: hwnd is a live window in this process found by title.
            unsafe { mark_as_toolwindow(hwnd) };
        }
    }

    pub fn reposition_on_monitor(
        title: &str,
        edge: Edge,
        logical_height: i32,
        monitor: &MonitorInfo,
    ) {
        let Some(hwnd) = find_hwnd_by_title(title) else {
            return;
        };
        let scale = if monitor.dpi > 0 {
            monitor.dpi as f32 / 96.0
        } else {
            1.0
        };
        let physical_height = (logical_height as f32 * scale).round() as i32;
        let width = monitor.right - monitor.left;
        let (x, y) = match edge {
            Edge::Top => (monitor.left, monitor.top),
            Edge::Bottom => (monitor.left, monitor.bottom - physical_height),
        };
        // SAFETY: hwnd is a live window in this process found by title.
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                x,
                y,
                width,
                physical_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }

    struct WindowSearch {
        pid: u32,
        title: Vec<u16>,
        found: Option<HWND>,
    }

    fn find_hwnd_by_title(title: &str) -> Option<HWND> {
        let mut search = WindowSearch {
            pid: std::process::id(),
            title: title.encode_utf16().collect(),
            found: None,
        };
        // SAFETY: find_window_proc runs only for the duration of this call and
        // dereferences the &mut WindowSearch passed through LPARAM.
        unsafe {
            let _ = EnumWindows(
                Some(find_window_proc),
                LPARAM(&mut search as *mut WindowSearch as isize),
            );
        }
        search.found
    }

    unsafe extern "system" fn find_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut WindowSearch) };
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != search.pid {
            return TRUE; // not ours; keep enumerating
        }
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len > 0 {
            let mut buf = vec![0u16; (len + 1) as usize];
            let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
            buf.truncate(n as usize);
            if buf == search.title {
                search.found = Some(hwnd);
                return FALSE; // stop enumerating
            }
        }
        TRUE
    }
}

#[cfg(not(windows))]
mod stub {
    use super::{Edge, Frame, Waker};

    pub struct AppBar;

    impl AppBar {
        pub fn try_register(
            _frame: &Frame,
            _edge: Edge,
            _height: i32,
            _waker: &Waker,
        ) -> Option<Self> {
            None
        }
    }
}
