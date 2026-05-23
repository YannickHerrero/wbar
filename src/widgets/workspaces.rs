use eframe::egui::{self, Color32, FontId, Sense, vec2};

use super::Widget;
use crate::config::WorkspacesConfig;
use crate::glazewm::GlazewmClient;
use crate::theme::Palette;

/// Workspace pill text is intentionally smaller than the body font so the
/// pill itself doesn't push against the top and bottom of the bar. Matches
/// zebar's `.workspace { font-size: 10px }` styling.
const WORKSPACE_FONT_SIZE: f32 = 10.0;
/// Fixed pill height. Picked so a 28px bar leaves ~5px of breathing room
/// above and below — relying on Frame's auto-sizing was unreliable (egui's
/// label minimum height and content_ui sizing both bullied the pill back
/// up to the bar height regardless of font size).
const WORKSPACE_PILL_HEIGHT: f32 = 18.0;
/// Horizontal padding inside the pill, each side.
const WORKSPACE_PILL_PAD_X: f32 = 6.0;

pub struct WorkspacesWidget {
    cfg: WorkspacesConfig,
    client: GlazewmClient,
    focused_bg: Color32,
    focused_fg: Color32,
    inactive_bg: Color32,
    inactive_fg: Color32,
    radius: f32,
}

impl WorkspacesWidget {
    pub fn new(
        cfg: WorkspacesConfig,
        client: GlazewmClient,
        palette: &Palette,
        radius: f32,
    ) -> Self {
        Self {
            cfg,
            client,
            focused_bg: palette.accent,
            focused_fg: palette.paper,
            inactive_bg: palette.muted,
            inactive_fg: palette.ink,
            radius,
        }
    }
}

impl Widget for WorkspacesWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        let state = self.client.snapshot();
        // No glazewm, no widget — the bar should look indistinguishable from
        // a config without the workspaces entry so running wbar standalone
        // (no glazewm installed/running) just works.
        if !state.connected || state.workspaces.is_empty() {
            return;
        }

        ui.spacing_mut().item_spacing.x = 4.0;
        let font_id = FontId::monospace(WORKSPACE_FONT_SIZE);
        let radius = self.radius;
        for ws in &state.workspaces {
            // show_empty toggles a future "include workspaces with no
            // windows" refinement once the JSON exposes that signal; for
            // now every workspace is rendered.
            let (bg, fg) = if ws.focused {
                (self.focused_bg, self.focused_fg)
            } else {
                (self.inactive_bg, self.inactive_fg)
            };

            // Lay out the text first so we know the natural width.
            let galley = ui
                .painter()
                .layout_no_wrap(ws.display_name.clone(), font_id.clone(), fg);
            let pill_w = galley.size().x + 2.0 * WORKSPACE_PILL_PAD_X;
            let pill_size = vec2(pill_w, WORKSPACE_PILL_HEIGHT);

            // allocate_exact_size + cross_align=Center on the parent
            // gives us a fixed-size rect centred vertically inside the
            // bar. Direct painting (rect_filled + galley) sidesteps
            // egui's label minimum-height padding entirely.
            let (rect, _resp) = ui.allocate_exact_size(pill_size, Sense::hover());
            ui.painter().rect_filled(rect, radius, bg);
            let text_pos = rect.center() - galley.size() / 2.0;
            ui.painter().galley(text_pos, galley, fg);
        }
        // show_empty consumed so the field doesn't trip dead_code analysis
        // until we wire it to a richer JSON shape.
        let _ = self.cfg.show_empty;
    }
}
