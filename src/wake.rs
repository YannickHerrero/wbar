//! Wake eframe's main thread from a background thread.
//!
//! eframe 0.32 on Windows doesn't reliably wake winit's event loop in
//! response to `ctx.request_repaint()` from a non-main thread. The
//! pending-repaint flag is set on the egui Context, but the winit loop
//! is asleep in `MsgWaitForMultipleObjects` with no incoming message to
//! consume — so it never gets a chance to check the flag, never paints,
//! and `App::update` is never called.
//!
//! The correct Win32 primitive for "ask another thread's message pump to
//! wake up and paint" is `InvalidateRect`. It queues a `WM_PAINT` to the
//! window, which winit consumes as a `RedrawRequested` event, which
//! eframe routes to `App::update`. The egui pending-repaint flag is
//! still set first (by the caller's `ctx.request_repaint()`), so eframe
//! sees something to do once it's awake.
//!
//! A `Waker` is created up front and shared by clone with every
//! background subsystem (tray, IPC, glazewm, hot-reload). It starts
//! with no HWND (the bar's window doesn't exist until eframe runs);
//! `set_hwnd` is called from `appbar::register` once the HWND is known.
//! Until then, `wake()` is a no-op — that's fine because no background
//! events fire during the first ~100 ms of startup.

use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};

#[derive(Clone, Default)]
pub struct Waker(Arc<AtomicIsize>);

impl Waker {
    pub fn new() -> Self {
        Self(Arc::new(AtomicIsize::new(0)))
    }

    /// Called by `appbar::register` once the bar's HWND is extractable.
    /// The raw value is whatever `HWND(ptr).0 as isize` produces; we don't
    /// care about the type here since wake() reinterprets it.
    ///
    /// Unused on non-Windows targets where wake() is a no-op stub and
    /// appbar::register itself short-circuits — keep the API uniform so
    /// the cross-target build still compiles.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn set_hwnd(&self, hwnd_raw: isize) {
        self.0.store(hwnd_raw, Ordering::Release);
        tracing::debug!(hwnd_raw, "waker armed");
    }

    /// Ask the main thread's message pump to wake and paint. Safe to call
    /// from any thread. No-op until `set_hwnd` has been called at least
    /// once.
    #[cfg(windows)]
    pub fn wake(&self) {
        let raw = self.0.load(Ordering::Acquire);
        if raw == 0 {
            return;
        }
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::InvalidateRect;
        // SAFETY: InvalidateRect is documented as safe from any thread.
        // The HWND was set by appbar::register from a live window we
        // received via raw-window-handle; we never store a stale value
        // because the window outlives the process.
        let hwnd = HWND(raw as *mut std::ffi::c_void);
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    #[cfg(not(windows))]
    pub fn wake(&self) {}
}
