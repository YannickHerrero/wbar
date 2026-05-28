//! System tray / menu-bar icon + menu. Without it the bar has no UI
//! affordance to exit (borderless window, no taskbar entry / Dock tile,
//! no decorations) — the only alternative would be killing the process
//! from Task Manager / Activity Monitor.
//!
//! Available on Windows and macOS: tray-icon routes to Shell_NotifyIcon
//! on Windows and NSStatusBar/NSStatusItem on macOS. The Linux backend
//! would drag in GTK as a build dep so we don't compile it for other
//! targets — they get a stub that lets cargo-check still pass.

#[cfg(any(windows, target_os = "macos"))]
pub use imp::{Tray, TrayEvent, build, poll};

#[cfg(not(any(windows, target_os = "macos")))]
pub use stub::{Tray, TrayEvent, build, poll};

use crate::theme::Theme;

/// What the tray menu emits this frame. Mapped onto IPC commands by the
/// caller so tray clicks and `wbar toggle`-style invocations share a
/// single code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// On non-supported platforms the stub `poll` always returns None, so the
// lint can't see any constructor for the variants. Wired up on Windows /
// macOS inside imp::poll.
#[allow(dead_code)]
pub enum TrayEventKind {
    Toggle,
    Quit,
    SetTheme(Theme),
}

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use std::collections::BTreeMap;
    use std::sync::mpsc::{self, Receiver};

    use anyhow::Result;
    use eframe::egui;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use crate::theme::Theme;
    use crate::wake::Waker;

    pub use super::TrayEventKind as TrayEvent;

    /// Themes shown in the tray submenu, in display order.
    const THEMES: &[(Theme, &str)] = &[
        (Theme::Paper, "Paper"),
        (Theme::Stone, "Stone"),
        (Theme::Sage, "Sage"),
        (Theme::Clay, "Clay"),
        (Theme::Ink, "Ink"),
    ];

    pub struct Tray {
        // Dropping the TrayIcon removes it from the notification area; keep
        // it alive for the process lifetime.
        _inner: TrayIcon,
        toggle_id: MenuId,
        quit_id: MenuId,
        theme_ids: BTreeMap<MenuId, Theme>,
        /// MenuEvents forwarded by the muda handler. We can't use
        /// `MenuEvent::receiver()` because muda's send() routes to *either*
        /// the handler or the channel — never both — and we need the
        /// handler to wake the egui loop via ctx.request_repaint().
        rx: Receiver<MenuEvent>,
    }

    pub fn build(ctx: egui::Context, waker: Waker) -> Result<Tray> {
        let menu = Menu::new();
        let toggle = MenuItem::new("Toggle bar", true, None);
        let theme_submenu = Submenu::new("Theme", true);
        let quit = MenuItem::new("Quit wbar", true, None);

        let toggle_id = toggle.id().clone();
        let quit_id = quit.id().clone();
        let mut theme_ids = BTreeMap::new();
        for (theme, label) in THEMES {
            let item = MenuItem::new(*label, true, None);
            theme_ids.insert(item.id().clone(), *theme);
            theme_submenu.append(&item)?;
        }

        menu.append(&toggle)?;
        menu.append(&theme_submenu)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let icon = Icon::from_rgba(generate_icon_rgba(32), 32, 32)?;

        let inner = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("wbar")
            .with_icon(icon)
            .build()?;

        // muda's MenuEvent::send is either/or: setting a handler suppresses
        // delivery to MenuEvent::receiver(). So we forward events into our
        // own channel from inside the handler. ctx.request_repaint() *and*
        // waker.wake() — the former sets egui's pending-repaint flag, the
        // latter calls InvalidateRect on the bar HWND so winit's pump
        // actually wakes up to see the flag. Without the wake(), eframe
        // 0.32 on Windows leaves the loop asleep when the only signal is
        // a background-thread request_repaint. waker.wake() is a no-op on
        // macOS — request_repaint reliably wakes the Cocoa run loop there.
        let (tx, rx) = mpsc::channel::<MenuEvent>();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            tracing::info!(id = ?event.id, "tray handler fired");
            let _ = tx.send(event);
            ctx.request_repaint();
            waker.wake();
        }));

        tracing::info!("tray icon ready (right-click for menu)");
        Ok(Tray {
            _inner: inner,
            toggle_id,
            quit_id,
            theme_ids,
            rx,
        })
    }

    /// Drain the local forwarded channel and return the *latest* event
    /// since the last call (multiple clicks per frame collapse to one).
    pub fn poll(tray: &Tray) -> Option<TrayEvent> {
        let mut latest = None;
        while let Ok(event) = tray.rx.try_recv() {
            tracing::debug!(id = ?event.id, "tray::poll drained event");
            let mapped = if event.id == tray.toggle_id {
                Some(TrayEvent::Toggle)
            } else if event.id == tray.quit_id {
                Some(TrayEvent::Quit)
            } else {
                tray.theme_ids
                    .get(&event.id)
                    .map(|theme| TrayEvent::SetTheme(*theme))
            };
            if let Some(m) = mapped {
                tracing::info!(event = ?m, "tray::poll matched event");
                latest = Some(m);
            } else {
                tracing::warn!(id = ?event.id, "tray::poll unmatched event id");
            }
        }
        latest
    }

    /// A small filled circle in the Paper-theme accent colour. Doesn't
    /// depend on the runtime palette so the tray icon stays recognisable
    /// across theme switches.
    fn generate_icon_rgba(size: u32) -> Vec<u8> {
        const ACCENT: [u8; 4] = [0xB5, 0x59, 0x3A, 0xFF];
        const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

        let mut rgba = vec![0u8; (size * size * 4) as usize];
        let s = size as f32;
        let cx = (s - 1.0) / 2.0;
        let cy = (s - 1.0) / 2.0;
        let r = s * 0.45;

        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let pixel = if d <= r { ACCENT } else { TRANSPARENT };
                let i = ((y * size + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&pixel);
            }
        }
        rgba
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod stub {
    use anyhow::Result;
    use eframe::egui;

    use crate::wake::Waker;

    pub use super::TrayEventKind as TrayEvent;

    pub struct Tray;

    pub fn build(_ctx: egui::Context, _waker: Waker) -> Result<Tray> {
        anyhow::bail!("tray icon is only implemented on Windows and macOS")
    }

    pub fn poll(_tray: &Tray) -> Option<TrayEvent> {
        None
    }
}
