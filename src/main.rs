// Release builds run as a Windows GUI app (no console window). Debug builds
// stay on the console subsystem so `cargo run` still shows tracing output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appbar;
// Several Palette fields and Tokens are consumed by widgets in later commits.
#[allow(dead_code)]
mod theme;
// Some widget-specific config fields are read by widgets in later commits.
#[allow(dead_code)]
mod config;
mod fonts;
mod glazewm;
mod hotreload;
mod widgets;

use eframe::egui;
use tracing_subscriber::EnvFilter;

use crate::appbar::{AppBar, Edge};
use crate::config::{BarPosition, Config};
use crate::glazewm::GlazewmClient;
use crate::hotreload::HotReload;
use crate::widgets::Widgets;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = config::default_path();
    let cfg = match config::load(config_path.as_deref()) {
        Ok(cfg) => {
            tracing::info!(?cfg, "loaded config");
            cfg
        }
        Err(err) => {
            tracing::warn!(error = ?err, "failed to load config, continuing with embedded default");
            Config::embedded_default()
        }
    };

    let viewport = egui::ViewportBuilder::default()
        .with_title("wbar")
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_inner_size([800.0, cfg.bar.height])
        .with_position([0.0, 0.0]);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "wbar",
        options,
        Box::new(move |cc| {
            fonts::install_nerd_font_fallback(&cc.egui_ctx);
            let palette = cfg.effective_palette();
            theme::apply(&cc.egui_ctx, &palette, theme::is_dark(cfg.theme));
            theme::apply_font_size(&cc.egui_ctx, cfg.font.size);

            let hot = config_path.and_then(|p| match hotreload::spawn(p, cc.egui_ctx.clone()) {
                Ok(h) => Some(h),
                Err(err) => {
                    tracing::warn!(error = ?err, "hot reload disabled");
                    None
                }
            });

            let glazewm = GlazewmClient::spawn(cc.egui_ctx.clone());

            Ok(Box::new(WbarApp::new(cfg, hot, glazewm)))
        }),
    )
}

struct WbarApp {
    cfg: Config,
    widgets: Widgets,
    glazewm: GlazewmClient,
    pinned: bool,
    hot: Option<HotReload>,
    appbar: Option<AppBar>,
}

impl WbarApp {
    fn new(cfg: Config, hot: Option<HotReload>, glazewm: GlazewmClient) -> Self {
        let palette = cfg.effective_palette();
        let radius = cfg.effective_tokens().radius_sm;
        let widgets = Widgets::from_config(&cfg, &palette, radius, &glazewm);
        Self {
            cfg,
            widgets,
            glazewm,
            pinned: false,
            hot,
            appbar: None,
        }
    }

    fn edge(&self) -> Edge {
        match self.cfg.bar.position {
            BarPosition::Top => Edge::Top,
            BarPosition::Bottom => Edge::Bottom,
        }
    }

    /// Register the bar with the Windows shell once the window has an HWND.
    /// SetWindowPos inside register() also moves the window to the rect the
    /// shell allocated, so this takes over from `pin_to_edge` once it succeeds.
    fn register_appbar(&mut self, frame: &eframe::Frame) {
        if self.appbar.is_some() {
            return;
        }
        let edge = self.edge();
        let height = self.cfg.bar.height as i32;
        self.appbar = appbar::register(frame, edge, height);
    }

    /// Drain any pending config reloads from the watcher and apply the latest.
    fn drain_reloads(&mut self, ctx: &egui::Context) {
        let Some(hot) = &self.hot else {
            return;
        };
        let mut latest = None;
        while let Ok(cfg) = hot.rx.try_recv() {
            latest = Some(cfg);
        }
        if let Some(cfg) = latest {
            let palette = cfg.effective_palette();
            let radius = cfg.effective_tokens().radius_sm;
            theme::apply(ctx, &palette, theme::is_dark(cfg.theme));
            theme::apply_font_size(ctx, cfg.font.size);
            self.widgets = Widgets::from_config(&cfg, &palette, radius, &self.glazewm);

            let position_changed = cfg.bar.position != self.cfg.bar.position
                || (cfg.bar.height - self.cfg.bar.height).abs() > f32::EPSILON;
            self.cfg = cfg;
            if position_changed {
                // Force re-pin and re-register; dropping the old AppBar issues
                // ABM_REMOVE before the next register's ABM_NEW.
                self.appbar = None;
                self.pinned = false;
            }
            tracing::info!("config reloaded — palette, font, widgets refreshed");
        }
    }

    /// On the first frame the OS reports a monitor size, stretch the window to
    /// full monitor width at the configured edge.
    fn pin_to_edge(&mut self, ctx: &egui::Context) {
        if self.pinned {
            return;
        }
        let monitor_size = ctx.input(|i| i.viewport().monitor_size);
        let Some(monitor_size) = monitor_size else {
            return;
        };
        let height = self.cfg.bar.height;
        let y = match self.cfg.bar.position {
            BarPosition::Top => 0.0,
            BarPosition::Bottom => monitor_size.y - height,
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            monitor_size.x,
            height,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(0.0, y)));
        tracing::info!(
            position = ?self.cfg.bar.position,
            width = monitor_size.x,
            height,
            "pinned bar"
        );
        self.pinned = true;
    }
}

impl eframe::App for WbarApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.drain_reloads(ctx);
        self.pin_to_edge(ctx);
        self.register_appbar(frame);
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_regions(ui);
        });
    }
}

impl WbarApp {
    fn bar_height(&self) -> f32 {
        self.cfg.bar.height
    }
}

impl WbarApp {
    fn draw_regions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let third = ui.available_width() / 3.0;

            ui.allocate_ui_with_layout(
                egui::vec2(third, self.bar_height()),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    for id in self.cfg.layout.left.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(third, self.bar_height()),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    for id in self.cfg.layout.center.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(third, self.bar_height()),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    for id in self.cfg.layout.right.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );
        });
    }
}
