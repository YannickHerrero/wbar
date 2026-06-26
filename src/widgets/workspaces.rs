use eframe::egui::{self, Color32, FontId, Sense, Stroke, StrokeKind, vec2};

use super::Widget;
use crate::config::WorkspacesConfig;
use crate::glazewm::{GlazewmClient, MonitorTarget};
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
    /// Outline colour for the workspace displayed on a non-focused monitor.
    accent: Color32,
    radius: f32,
    /// Which monitor's workspaces this instance renders. Set per-bar before
    /// each viewport draws (see Widgets::set_monitor_target).
    target: MonitorTarget,
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
            accent: palette.accent,
            radius,
            target: MonitorTarget::Focused,
        }
    }
}

impl Widget for WorkspacesWidget {
    fn set_monitor_target(&mut self, target: &MonitorTarget) {
        self.target = target.clone();
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        let state = self.client.snapshot();
        // No glazewm, no widget — the bar should look indistinguishable from
        // a config without the workspaces entry so running wbar standalone
        // (no glazewm installed/running) just works.
        if !state.connected {
            return;
        }
        // Only this bar's monitor; skip if glazewm doesn't report it yet.
        let Some(monitor) = state.monitor_for(&self.target) else {
            return;
        };
        if monitor.workspaces.is_empty() {
            return;
        }

        ui.spacing_mut().item_spacing.x = 4.0;
        let font_id = FontId::monospace(WORKSPACE_FONT_SIZE);
        let radius = self.radius;
        for ws in &monitor.workspaces {
            // Three states: the globally-focused workspace is a solid accent
            // pill; the workspace displayed on a *non-focused* monitor gets a
            // muted fill with an accent outline (it's active on its screen but
            // doesn't hold focus); everything else is inactive.
            let (bg, fg, outlined) = if ws.focused {
                (self.focused_bg, self.focused_fg, false)
            } else if ws.is_displayed {
                (self.inactive_bg, self.inactive_fg, true)
            } else {
                (self.inactive_bg, self.inactive_fg, false)
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
            if outlined {
                ui.painter().rect_stroke(
                    rect,
                    radius,
                    Stroke::new(1.5, self.accent),
                    StrokeKind::Inside,
                );
            }
            let text_pos = rect.center() - galley.size() / 2.0;
            ui.painter().galley(text_pos, galley, fg);
        }
        // show_empty consumed so the field doesn't trip dead_code analysis
        // until we wire it to a richer JSON shape.
        let _ = self.cfg.show_empty;
    }
}
