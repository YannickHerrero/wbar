use eframe::egui::{self, Color32, Frame, Stroke};

use super::Widget;
use crate::config::WorkspacesConfig;
use crate::glazewm::GlazewmClient;
use crate::theme::Palette;

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
            Frame::new()
                .fill(bg)
                .stroke(Stroke::NONE)
                .corner_radius(self.radius)
                .inner_margin(egui::Margin::symmetric(8, 2))
                .show(ui, |ui| {
                    ui.colored_label(fg, &ws.display_name);
                });
        }
        // show_empty consumed so the field doesn't trip dead_code analysis
        // until we wire it to a richer JSON shape.
        let _ = self.cfg.show_empty;
    }
}
