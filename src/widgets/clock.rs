use std::time::Duration;

use chrono::Local;
use eframe::egui;

use super::Widget;
use crate::config::ClockConfig;

pub struct ClockWidget {
    cfg: ClockConfig,
}

impl ClockWidget {
    pub fn new(cfg: ClockConfig) -> Self {
        Self { cfg }
    }
}

impl Widget for ClockWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        let now = Local::now();
        let text = now.format(&self.cfg.format).to_string();
        ui.label(text);
        // Ask egui to wake us up for the next tick. Immediate-mode would
        // otherwise idle indefinitely between input events.
        let tick = self.cfg.tick_seconds.max(1);
        ui.ctx().request_repaint_after(Duration::from_secs(tick));
    }
}
