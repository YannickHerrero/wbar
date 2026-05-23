use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

use super::Widget;
use crate::config::CommandConfig;

#[cfg(windows)]
const SHELL: (&str, &str) = ("cmd", "/C");
#[cfg(not(windows))]
const SHELL: (&str, &str) = ("sh", "-c");

/// Runs an arbitrary shell command on an interval in a background thread and
/// displays the trimmed first line of stdout. Spawning in a thread keeps the
/// UI thread free of subprocess latency.
pub struct CommandWidget {
    cfg: CommandConfig,
    last_run: Option<Instant>,
    in_flight: Arc<AtomicBool>,
    tx: Sender<String>,
    rx: Receiver<String>,
    rendered: String,
}

impl CommandWidget {
    pub fn new(cfg: CommandConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cfg,
            last_run: None,
            in_flight: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            rendered: String::new(),
        }
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(self.cfg.interval_seconds.max(1))
    }

    fn run_if_due(&mut self, ctx: &egui::Context) {
        let due = match self.last_run {
            Some(t) => t.elapsed() >= self.interval(),
            None => true,
        };
        if !due || self.in_flight.load(Ordering::Acquire) {
            return;
        }
        self.last_run = Some(Instant::now());
        self.in_flight.store(true, Ordering::Release);

        let command = self.cfg.command.clone();
        let tx = self.tx.clone();
        let in_flight = self.in_flight.clone();
        let ctx = ctx.clone();

        std::thread::spawn(move || {
            let output = Command::new(SHELL.0).arg(SHELL.1).arg(&command).output();
            let text = match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    stdout.lines().next().unwrap_or("").trim().to_string()
                }
                Err(err) => {
                    tracing::warn!(?err, command, "command widget failed to spawn");
                    format!("{err}")
                }
            };
            let _ = tx.send(text);
            in_flight.store(false, Ordering::Release);
            ctx.request_repaint();
        });
    }

    fn drain(&mut self) {
        while let Ok(text) = self.rx.try_recv() {
            self.rendered = text;
        }
    }
}

impl Widget for CommandWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        self.drain();
        self.run_if_due(ui.ctx());
        ui.label(&self.rendered);
        ui.ctx().request_repaint_after(self.interval());
    }
}
