//! System tray icon + menu. Without it the bar has no UI affordance to
//! exit (borderless window, no taskbar entry, no decorations) — the only
//! alternative would be killing the process from Task Manager.
//!
//! Windows-only by design: the bar is Windows-only anyway, and tray-icon's
//! Linux backend would drag in GTK as a build dep. Other targets get a stub
//! so the cargo-check workflow on WSL still compiles.

#[cfg(windows)]
pub use imp::{Tray, TrayEvent, build, poll};

#[cfg(not(windows))]
pub use stub::{Tray, TrayEvent, build, poll};

/// What the tray menu emits this frame. Mapped onto IPC commands by the
/// caller so tray clicks and `wbar toggle`-style invocations share a
/// single code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// On non-Windows the stub `poll` always returns None, so the lint can't see
// any constructor for the variants. Wired up on Windows in imp::poll.
#[allow(dead_code)]
pub enum TrayEventKind {
    Toggle,
    Show,
    Hide,
    Quit,
}

#[cfg(windows)]
mod imp {
    use anyhow::Result;
    use eframe::egui;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    pub use super::TrayEventKind as TrayEvent;

    pub struct Tray {
        // Dropping the TrayIcon removes it from the notification area; keep
        // it alive for the process lifetime.
        _inner: TrayIcon,
        toggle_id: MenuId,
        show_id: MenuId,
        hide_id: MenuId,
        quit_id: MenuId,
    }

    pub fn build(ctx: egui::Context) -> Result<Tray> {
        let menu = Menu::new();
        let toggle = MenuItem::new("Toggle bar", true, None);
        let show = MenuItem::new("Show bar", true, None);
        let hide = MenuItem::new("Hide bar", true, None);
        let quit = MenuItem::new("Quit wbar", true, None);

        let toggle_id = toggle.id().clone();
        let show_id = show.id().clone();
        let hide_id = hide.id().clone();
        let quit_id = quit.id().clone();

        menu.append(&toggle)?;
        menu.append(&show)?;
        menu.append(&hide)?;
        menu.append(&tray_icon::menu::PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let icon = Icon::from_rgba(generate_icon_rgba(32), 32, 32)?;

        let inner = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("wbar")
            .with_icon(icon)
            .build()?;

        // Without this handler, MenuEvents land in the global channel but
        // nothing wakes the eframe event loop while the bar window is
        // hidden — so Show / Quit clicks would do nothing once the bar is
        // hidden. request_repaint() forces an update tick which then drains
        // the channel via tray::poll.
        MenuEvent::set_event_handler(Some(move |_event| {
            ctx.request_repaint();
        }));

        tracing::info!("tray icon ready (right-click for menu)");
        Ok(Tray {
            _inner: inner,
            toggle_id,
            show_id,
            hide_id,
            quit_id,
        })
    }

    /// Drain the global menu-event channel and return the *latest* event
    /// since the last call (multiple clicks per frame collapse to one).
    pub fn poll(tray: &Tray) -> Option<TrayEvent> {
        let mut latest = None;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            latest = if event.id == tray.toggle_id {
                Some(TrayEvent::Toggle)
            } else if event.id == tray.show_id {
                Some(TrayEvent::Show)
            } else if event.id == tray.hide_id {
                Some(TrayEvent::Hide)
            } else if event.id == tray.quit_id {
                Some(TrayEvent::Quit)
            } else {
                latest
            };
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

#[cfg(not(windows))]
mod stub {
    use anyhow::Result;
    use eframe::egui;

    pub use super::TrayEventKind as TrayEvent;

    pub struct Tray;

    pub fn build(_ctx: egui::Context) -> Result<Tray> {
        anyhow::bail!("tray icon is only implemented on Windows")
    }

    pub fn poll(_tray: &Tray) -> Option<TrayEvent> {
        None
    }
}
