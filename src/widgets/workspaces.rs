use eframe::egui::{self, Color32, Frame, RichText, Stroke};

use super::Widget;
use crate::config::WorkspacesConfig;
use crate::glazewm::GlazewmClient;
use crate::theme::Palette;

/// Workspace pill text is intentionally smaller than the body font so the
/// pill itself doesn't push against the top and bottom of the bar. Matches
/// zebar's `.workspace { font-size: 10px }` styling.
const WORKSPACE_FONT_SIZE: f32 = 10.0;

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
        for ws in &state.workspaces {
            // show_empty toggles a future "include workspaces with no windows"
            // refinement once the JSON exposes that signal; for now every
            // workspace is rendered.
            let (bg, fg) = if ws.focused {
                (self.focused_bg, self.focused_fg)
            } else {
                (self.inactive_bg, self.inactive_fg)
            };
            // Smaller pill font + a touch of vertical inner padding keeps
            // the pill comfortably inside the 28px bar with visible
            // breathing room above and below, without changing
            // bar.height.
            Frame::new()
                .fill(bg)
                .stroke(Stroke::NONE)
                .corner_radius(self.radius)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&ws.display_name)
                            .size(WORKSPACE_FONT_SIZE)
                            .color(fg),
                    );
                });
        }
        // show_empty consumed so the field doesn't trip dead_code analysis
        // until we wire it to a richer JSON shape.
        let _ = self.cfg.show_empty;
    }
}
