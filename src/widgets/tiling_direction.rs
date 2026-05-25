use eframe::egui;

use super::Widget;
use crate::config::TilingDirectionConfig;
use crate::glazewm::{GlazewmClient, TilingDirection};

/// Renders a single glyph reflecting the focused container's current tiling
/// direction. Reactive — pulls from GlazewmClient::snapshot() which is kept
/// in sync via the IPC event stream (no polling).
pub struct TilingDirectionWidget {
    cfg: TilingDirectionConfig,
    client: GlazewmClient,
}

impl TilingDirectionWidget {
    pub fn new(cfg: TilingDirectionConfig, client: GlazewmClient) -> Self {
        Self { cfg, client }
    }
}

impl Widget for TilingDirectionWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        let state = self.client.snapshot();
        if !state.connected {
            return;
        }
        let glyph = match state.tiling_direction {
            Some(TilingDirection::Horizontal) => &self.cfg.horizontal,
            Some(TilingDirection::Vertical) => &self.cfg.vertical,
            None => return,
        };
        ui.label(glyph.as_str());
    }
}
