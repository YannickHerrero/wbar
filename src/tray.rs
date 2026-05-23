//! System tray icon + Quit menu. Without it the bar has no UI affordance to
//! exit (borderless window, no taskbar entry, no decorations) — the only
//! alternative would be killing the process from Task Manager.
//!
//! Windows-only by design: the bar is Windows-only anyway, and tray-icon's
//! Linux backend would drag in GTK as a build dep. Other targets get a stub
//! so the cargo-check workflow on WSL still compiles.

#[cfg(windows)]
pub use imp::{Tray, build, poll_quit};

#[cfg(not(windows))]
pub use stub::{Tray, build, poll_quit};

#[cfg(windows)]
mod imp {
    use anyhow::Result;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    pub struct Tray {
        // Dropping the TrayIcon removes it from the notification area; keep
        // it alive for the process lifetime.
        _inner: TrayIcon,
        quit_id: MenuId,
    }

    pub fn build() -> Result<Tray> {
        let menu = Menu::new();
        let quit = MenuItem::new("Quit wbar", true, None);
        let quit_id = quit.id().clone();
        menu.append(&quit)?;

        let icon = Icon::from_rgba(generate_icon_rgba(32), 32, 32)?;

        let inner = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("wbar")
            .with_icon(icon)
            .build()?;

        tracing::info!("tray icon ready (right-click for Quit menu)");
        Ok(Tray {
            _inner: inner,
            quit_id,
        })
    }

    /// Drain the global menu-event channel; return true if the user clicked
    /// Quit. Cheap to call every frame.
    pub fn poll_quit(tray: &Tray) -> bool {
        let mut quit = false;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == tray.quit_id {
                quit = true;
            }
        }
        quit
    }

    /// A small filled circle in the Paper-theme accent colour. Doesn't depend
    /// on the runtime palette so the tray icon stays recognisable across
    /// theme switches.
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

    pub struct Tray;

    pub fn build() -> Result<Tray> {
        anyhow::bail!("tray icon is only implemented on Windows")
    }

    pub fn poll_quit(_tray: &Tray) -> bool {
        false
    }
}
