use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    eframe::run_native(
        "wbar",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(WbarApp))),
    )
}

struct WbarApp;

impl eframe::App for WbarApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("wbar");
        });
    }
}
