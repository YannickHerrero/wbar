//! Windows AppBar registration. Tells the shell "this window owns a strip of
//! the screen edge"; once registered, maximised windows stop short of the bar
//! instead of going under it. No-op on non-Windows targets so the crate still
//! compiles on Linux for cargo-check during development.

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

#[cfg(windows)]
mod imp {
    use super::{Edge, Frame, Waker};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
        SHAppBarMessage,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetSystemMetrics, GetWindowLongPtrW, SM_CXSCREEN, SM_CYSCREEN,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
        WS_EX_TOOLWINDOW,
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

    /// Add `WS_EX_TOOLWINDOW` to the bar's extended window styles. This is
    /// the canonical "I'm a dock/utility window, not a regular app" hint
    /// that taskbars, Alt+Tab handlers, and tiling window managers all
    /// look at. egui's `with_taskbar(false)` only calls
    /// `ITaskbarList::DeleteTab` which removes the taskbar entry but
    /// leaves the window otherwise indistinguishable from a normal app.
    ///
    /// SAFETY: caller guarantees `hwnd` is a live, owned window.
    unsafe fn mark_as_toolwindow(hwnd: HWND) {
        let current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let new = current | (WS_EX_TOOLWINDOW.0 as isize);
        if current == new {
            return;
        }
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new) };
        // SetWindowLongPtrW takes effect on the next SetWindowPos with
        // SWP_FRAMECHANGED. Force it now so the change is visible to
        // the shell immediately (do_register's SetWindowPos that
        // follows doesn't carry SWP_FRAMECHANGED).
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        tracing::info!("marked bar HWND as WS_EX_TOOLWINDOW");
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

    unsafe fn do_register(hwnd: HWND, edge: Edge, height: i32) -> Option<AppBar> {
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
                abd.rc.bottom = height;
            }
            Edge::Bottom => {
                abd.rc.top = screen_h - height;
                abd.rc.bottom = screen_h;
            }
        }

        // ABM_QUERYPOS lets Windows adjust the rect (e.g. dodge other appbars).
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut abd) };

        // Re-pin the height after QUERYPOS may have widened the rect along the
        // perpendicular axis.
        match edge {
            Edge::Top => abd.rc.bottom = abd.rc.top + height,
            Edge::Bottom => abd.rc.top = abd.rc.bottom - height,
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
            height = h,
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
