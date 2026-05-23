//! Windows AppBar registration. Tells the shell "this window owns a strip of
//! the screen edge"; once registered, maximised windows stop short of the bar
//! instead of going under it. No-op on non-Windows targets so the crate still
//! compiles on Linux for cargo-check during development.

use eframe::Frame;

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
/// frames may not have one) or if any SHAppBarMessage call fails.
pub fn register(frame: &Frame, edge: Edge, height: i32) -> Option<AppBar> {
    AppBar::try_register(frame, edge, height)
}

#[cfg(windows)]
mod imp {
    use super::{Edge, Frame};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
        SHAppBarMessage,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SetTimer,
        SetWindowPos,
    };

    /// Win32 timer id we use to keep winit's message pump waking up. Picked
    /// arbitrarily; only this module sets a timer on the bar's HWND.
    const WAKE_TIMER_ID: usize = 1;
    /// 200 ms = 5 wake-ups per second. Cheap (the message pump just
    /// processes a WM_TIMER no-op) and gives the tray a worst-case 200 ms
    /// latency between click and apply.
    const WAKE_TIMER_INTERVAL_MS: u32 = 200;

    pub struct AppBar {
        hwnd: HWND,
    }

    impl AppBar {
        pub fn try_register(frame: &Frame, edge: Edge, height: i32) -> Option<Self> {
            let hwnd = hwnd_from_frame(frame)?;
            // SAFETY: SHAppBarMessage and SetWindowPos take a valid HWND owned
            // by this process. We just got it from the live eframe viewport.
            unsafe { do_register(hwnd, edge, height) }
        }
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

        // Install a periodic WM_TIMER on our HWND so winit's message pump
        // wakes regularly. Without this, eframe 0.32 on Windows doesn't
        // reliably respond to ctx.request_repaint() from background
        // threads (tray handler, ipc handler, glazewm reconnect, ...), so
        // tray clicks could sit in the channel for tens of seconds before
        // update() finally ran. SetTimer with HWND owner posts WM_TIMER
        // straight into the thread's message queue. Re-calling SetTimer
        // with the same id replaces the existing timer, so it's safe to
        // call again on AppBar re-register after a Hide→Show.
        unsafe {
            SetTimer(Some(hwnd), WAKE_TIMER_ID, WAKE_TIMER_INTERVAL_MS, None);
        }

        tracing::info!(
            edge = ?edge,
            left = abd.rc.left,
            top = abd.rc.top,
            width = w,
            height = h,
            wake_timer_ms = WAKE_TIMER_INTERVAL_MS,
            "appbar registered",
        );

        Some(AppBar { hwnd })
    }
}

#[cfg(not(windows))]
mod stub {
    use super::{Edge, Frame};

    pub struct AppBar;

    impl AppBar {
        pub fn try_register(_frame: &Frame, _edge: Edge, _height: i32) -> Option<Self> {
            None
        }
    }
}
