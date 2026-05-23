use std::collections::BTreeMap;

use eframe::egui;

use crate::config::{Config, WidgetConfig};

mod clock;
mod command;
mod sysinfo;

pub trait Widget {
    fn render(&mut self, ui: &mut egui::Ui);
}

/// Registry of instantiated widgets keyed by their config id (the layout
/// arrays reference these ids by name).
pub struct Widgets {
    items: BTreeMap<String, Box<dyn Widget>>,
}

impl Widgets {
    pub fn from_config(cfg: &Config) -> Self {
        let items = cfg
            .widgets
            .iter()
            .map(|(id, wc)| (id.clone(), build(id, wc)))
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

fn build(id: &str, cfg: &WidgetConfig) -> Box<dyn Widget> {
    match cfg {
        WidgetConfig::Clock(c) => Box::new(clock::ClockWidget::new(c.clone())),
        WidgetConfig::Sysinfo(c) => Box::new(sysinfo::SysinfoWidget::new(c.clone())),
        WidgetConfig::Command(c) => Box::new(command::CommandWidget::new(c.clone())),
        // GlazeWM workspaces lands in commit 18.
        WidgetConfig::Glazewm(_) => Box::new(Placeholder(id.to_string())),
    }
}

struct Placeholder(String);

impl Widget for Placeholder {
    fn render(&mut self, ui: &mut egui::Ui) {
        ui.label(&self.0);
    }
}
