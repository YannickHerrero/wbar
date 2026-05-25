use eframe::egui;

use super::Widget;
use crate::config::SpacerConfig;

/// Inserts blank horizontal space along the region's main axis.
/// Works for both left_to_right and right_to_left layouts — egui's
/// `add_space` flows in the direction of the parent layout.
pub struct SpacerWidget {
    width: f32,
}

impl SpacerWidget {
    pub fn new(cfg: SpacerConfig) -> Self {
        Self { width: cfg.width }
    }
}

impl Widget for SpacerWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        ui.add_space(self.width);
    }
}
