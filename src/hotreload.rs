use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::{Context as _, Result};
use eframe::egui;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{self, Config};
use crate::wake::Waker;

/// Watches a config file (via its parent directory, which is robust to atomic
/// rename-saves common in editors) and pushes a freshly parsed Config onto the
/// returned channel each time it changes. The egui context is awoken so the
/// reload is picked up on the very next frame, not after the next user input.
pub struct HotReload {
    pub rx: mpsc::Receiver<Config>,
    // Dropping the watcher tears down the OS-level subscription. Keep it alive.
    _watcher: RecommendedWatcher,
}

pub fn spawn(path: PathBuf, ctx: egui::Context, waker: Waker) -> Result<HotReload> {
    let (tx, rx) = mpsc::channel::<Config>();
    let watch_path = path.clone();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let event = match res {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = ?err, "config watcher error");
                return;
            }
        };

        if !matches!(
            event.kind,
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
        ) {
            return;
        }

        if !event.paths.iter().any(|p| p == &watch_path) {
            return;
        }

        match config::load(Some(&watch_path)) {
            Ok(cfg) => {
                tracing::info!(path = %watch_path.display(), "config reloaded");
                if tx.send(cfg).is_ok() {
                    ctx.request_repaint();
                    waker.wake();
                }
            }
            Err(err) => tracing::warn!(error = ?err, "config reload failed"),
        }
    })
    .context("creating config file watcher")?;

    // Watch the parent directory, not the file: editors atomic-save by writing
    // a temp file and renaming over the original, which detaches a per-file
    // watch from the new inode.
    let watch_dir = path.parent().unwrap_or(&path).to_path_buf();
    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", watch_dir.display()))?;

    tracing::info!(
        path = %path.display(),
        dir = %watch_dir.display(),
        "watching config for changes",
    );

    Ok(HotReload {
        rx,
        _watcher: watcher,
    })
}
