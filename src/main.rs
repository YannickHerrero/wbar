mod appbar;
// Several Palette fields and Tokens are consumed by widgets in later commits.
#[allow(dead_code)]
mod theme;
// Some widget-specific config fields are read by widgets in later commits.
#[allow(dead_code)]
mod config;
mod hotreload;
mod widgets;

use eframe::egui;
use tracing_subscriber::EnvFilter;

use crate::appbar::{AppBar, Edge};
use crate::config::Config;
use crate::hotreload::HotReload;
use crate::widgets::Widgets;

const BAR_HEIGHT: f32 = 32.0;

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
        .with_inner_size([800.0, BAR_HEIGHT])
        .with_position([0.0, 0.0]);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "wbar",
        options,
        Box::new(move |cc| {
            let palette = cfg.effective_palette();
            theme::apply(&cc.egui_ctx, &palette, theme::is_dark(cfg.theme));

            let hot = config_path.and_then(|p| match hotreload::spawn(p, cc.egui_ctx.clone()) {
                Ok(h) => Some(h),
                Err(err) => {
                    tracing::warn!(error = ?err, "hot reload disabled");
                    None
                }
            });

            Ok(Box::new(WbarApp::new(cfg, hot)))
        }),
    )
}

struct WbarApp {
    cfg: Config,
    widgets: Widgets,
    pinned: bool,
    hot: Option<HotReload>,
    appbar: Option<AppBar>,
}

impl WbarApp {
    fn new(cfg: Config, hot: Option<HotReload>) -> Self {
        let widgets = Widgets::from_config(&cfg);
        Self {
            cfg,
            widgets,
            pinned: false,
            hot,
            appbar: None,
        }
    }

    /// Register the bar with the Windows shell once the window has an HWND.
    /// SetWindowPos inside register() also moves the window to the rect the
    /// shell allocated, so this takes over from `pin_to_top` once it succeeds.
    fn register_appbar(&mut self, frame: &eframe::Frame) {
        if self.appbar.is_some() {
            return;
        }
        let edge = Edge::Top;
        let height = BAR_HEIGHT as i32;
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
            theme::apply(ctx, &palette, theme::is_dark(cfg.theme));
            self.widgets = Widgets::from_config(&cfg);
            self.cfg = cfg;
        }
    }

    /// On the first frame the OS has reported the primary monitor size, so we
    /// stretch the window to the full monitor width and snap it to the top edge.
    fn pin_to_top(&mut self, ctx: &egui::Context) {
        if self.pinned {
            return;
        }
        let monitor_size = ctx.input(|i| i.viewport().monitor_size);
        let Some(monitor_size) = monitor_size else {
            return;
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            monitor_size.x,
            BAR_HEIGHT,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(0.0, 0.0)));
        tracing::info!(
            width = monitor_size.x,
            height = BAR_HEIGHT,
            "pinned bar to top"
        );
        self.pinned = true;
    }
}

impl eframe::App for WbarApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.drain_reloads(ctx);
        self.pin_to_top(ctx);
        self.register_appbar(frame);
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_regions(ui);
        });
    }
}

impl WbarApp {
    fn draw_regions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let third = ui.available_width() / 3.0;

            ui.allocate_ui_with_layout(
                egui::vec2(third, BAR_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    for id in self.cfg.layout.left.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(third, BAR_HEIGHT),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    for id in self.cfg.layout.center.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(third, BAR_HEIGHT),
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
