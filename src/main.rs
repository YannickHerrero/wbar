// Several Palette fields and Tokens are consumed by widgets in later commits.
#[allow(dead_code)]
mod theme;
// Widget-specific config variants are consumed by widget modules in later commits.
#[allow(dead_code)]
mod config;

use eframe::egui;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

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
            theme::apply(&cc.egui_ctx, cfg.theme);
            Ok(Box::new(WbarApp::new(cfg)))
        }),
    )
}

struct WbarApp {
    // Used by layout, font, and widget commits that follow.
    #[allow(dead_code)]
    cfg: Config,
    pinned: bool,
}

impl WbarApp {
    fn new(cfg: Config) -> Self {
        Self { cfg, pinned: false }
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pin_to_top(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("wbar");
        });
    }
}
