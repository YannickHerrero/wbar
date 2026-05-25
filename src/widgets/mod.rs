use std::collections::BTreeMap;

use eframe::egui;

use crate::config::{Config, WidgetConfig};
use crate::glazewm::GlazewmClient;
use crate::theme::Palette;

mod clock;
mod command;
mod spacer;
mod sysinfo;
mod tiling_direction;
mod workspaces;

pub trait Widget {
    fn render(&mut self, ui: &mut egui::Ui);
}

/// Registry of instantiated widgets keyed by their config id (the layout
/// arrays reference these ids by name).
pub struct Widgets {
    items: BTreeMap<String, Box<dyn Widget>>,
}

impl Widgets {
    pub fn from_config(
        cfg: &Config,
        palette: &Palette,
        radius: f32,
        glazewm: &GlazewmClient,
    ) -> Self {
        let items = cfg
            .widgets
            .iter()
            .map(|(id, wc)| (id.clone(), build(id, wc, palette, radius, glazewm)))
            .collect();
        Self { items }
    }

    /// Render the widget with the given id; falls back to a "?id" label if no
    /// widget with that id is registered (typical when layout references an
    /// undefined entry).
    pub fn render(&mut self, ui: &mut egui::Ui, id: &str) {
        match self.items.get_mut(id) {
            Some(w) => w.render(ui),
            None => {
                ui.label(format!("?{id}"));
            }
        }
    }
}

fn build(
    _id: &str,
    cfg: &WidgetConfig,
    palette: &Palette,
    radius: f32,
    glazewm: &GlazewmClient,
) -> Box<dyn Widget> {
    match cfg {
        WidgetConfig::Clock(c) => Box::new(clock::ClockWidget::new(c.clone())),
        WidgetConfig::Sysinfo(c) => Box::new(sysinfo::SysinfoWidget::new(c.clone(), palette)),
        WidgetConfig::Command(c) => Box::new(command::CommandWidget::new(c.clone())),
        WidgetConfig::Glazewm(c) => Box::new(workspaces::WorkspacesWidget::new(
            c.clone(),
            glazewm.clone(),
            palette,
            radius,
        )),
        WidgetConfig::TilingDirection(c) => Box::new(
            tiling_direction::TilingDirectionWidget::new(c.clone(), glazewm.clone(), palette, radius),
        ),
        WidgetConfig::Spacer(c) => Box::new(spacer::SpacerWidget::new(c.clone())),
    }
}
