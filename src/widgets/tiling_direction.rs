use eframe::egui::{self, Color32, FontId, Sense, vec2};

use super::Widget;
use crate::config::TilingDirectionConfig;
use crate::glazewm::{GlazewmClient, TilingDirection};
use crate::theme::Palette;

/// Match the workspaces pill so the two widgets sit cleanly next to each
/// other; same font size, same height, same horizontal padding.
const PILL_FONT_SIZE: f32 = 10.0;
const PILL_HEIGHT: f32 = 18.0;
const PILL_PAD_X: f32 = 6.0;

/// Renders the focused container's current tiling direction as a single
/// glyph inside a pill (styled like an inactive workspaces pill). Reactive
/// — pulls from GlazewmClient::snapshot() which is kept in sync via the
/// IPC event stream (no polling).
pub struct TilingDirectionWidget {
    cfg: TilingDirectionConfig,
    client: GlazewmClient,
    bg: Color32,
    fg: Color32,
    radius: f32,
}

impl TilingDirectionWidget {
    pub fn new(
        cfg: TilingDirectionConfig,
        client: GlazewmClient,
        palette: &Palette,
        radius: f32,
    ) -> Self {
        Self {
            cfg,
            client,
            bg: palette.muted,
            fg: palette.ink,
            radius,
        }
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

        let font_id = FontId::monospace(PILL_FONT_SIZE);
        let galley = ui
            .painter()
            .layout_no_wrap(glyph.clone(), font_id, self.fg);
        let pill_w = galley.size().x + 2.0 * PILL_PAD_X;
        let pill_size = vec2(pill_w, PILL_HEIGHT);

        let (rect, _resp) = ui.allocate_exact_size(pill_size, Sense::hover());
        ui.painter().rect_filled(rect, self.radius, self.bg);
        let text_pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(text_pos, galley, self.fg);
    }
}
