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

#[cfg(windows)]
mod imp {
    use super::{Edge, Frame, Waker};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
        SHAppBarMessage,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetSystemMetrics, GetWindowLongPtrW, SM_CXSCREEN, SM_CYSCREEN,
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
            // SAFETY: SHAppBarMessage and SetWindowPos take a valid HWND
            // owned by this process.
            let appbar = unsafe { do_register(hwnd, edge, height) }?;
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
        let chrome = (WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MAXIMIZEBOX.0
            | WS_MINIMIZEBOX.0) as isize;
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

    unsafe fn do_register(hwnd: HWND, edge: Edge, logical_height: i32) -> Option<AppBar> {
        // SHAppBarMessage and SetWindowPos work in *physical* pixels (the
        // process is per-monitor-DPI aware via eframe). bar.height in
        // config is in *logical* pixels — eframe renders the window at
        // logical_height × dpi_scale physical pixels. If we don't scale
        // before talking to the shell, the AppBar reservation
        // under-reports the bar height on hi-DPI displays and tiling
        // window managers can't compute the right work area. At 1.25x
        // DPI a 28-logical bar is 35 physical px but we'd reserve 28,
        // leaving a 7px strip where the bar paints but apps still tile.
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
        let physical_height = (logical_height as f32 * scale).round() as i32;

        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

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

        // Propose a rect spanning the full primary monitor width, at the
        // requested edge.
        abd.rc.left = 0;
        abd.rc.right = screen_w;
        match edge {
            Edge::Top => {
                abd.rc.top = 0;
                abd.rc.bottom = physical_height;
            }
            Edge::Bottom => {
                abd.rc.top = screen_h - physical_height;
                abd.rc.bottom = screen_h;
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
