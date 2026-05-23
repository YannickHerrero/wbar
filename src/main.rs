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
mod ipc;
mod tray;
mod widgets;

use eframe::egui;
use tracing_subscriber::EnvFilter;

use std::sync::mpsc::Receiver;

use crate::appbar::{AppBar, Edge};
use crate::config::{BarPosition, Config};
use crate::glazewm::GlazewmClient;
use crate::hotreload::HotReload;
use crate::ipc::IpcCommand;
use crate::theme::Theme;
use std::path::PathBuf;

use crate::tray::{Tray, TrayEvent};
use crate::widgets::Widgets;

/// Horizontal padding between the bar contents and the screen edges,
/// applied as the CentralPanel frame's inner margin.
const BAR_EDGE_PAD: i8 = 16;
/// Extra cushion inside the right region so Nerd-Font glyphs with positive
/// right-side bearing don't paint past the slot edge.
const RIGHT_EDGE_CUSHION: f32 = 8.0;
/// Gap between consecutive widgets within the left and right regions.
const REGION_ITEM_SPACING: f32 = 14.0;

fn main() -> eframe::Result {
    // CLI client mode: any first argv that matches a known subcommand sends
    // the command to the already-running wbar over IPC and exits, instead of
    // booting a second bar instance.
    if let Some(code) = handle_cli() {
        std::process::exit(code);
    }

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

            let hot =
                config_path
                    .clone()
                    .and_then(|p| match hotreload::spawn(p, cc.egui_ctx.clone()) {
                        Ok(h) => Some(h),
                        Err(err) => {
                            tracing::warn!(error = ?err, "hot reload disabled");
                            None
                        }
                    });

            let glazewm = GlazewmClient::spawn(cc.egui_ctx.clone());

            let tray = match tray::build(cc.egui_ctx.clone()) {
                Ok(t) => Some(t),
                Err(err) => {
                    tracing::warn!(error = ?err, "tray icon disabled");
                    None
                }
            };

            let ipc_rx = match ipc::spawn(cc.egui_ctx.clone()) {
                Ok(rx) => Some(rx),
                Err(err) => {
                    tracing::warn!(error = ?err, "ipc control server disabled");
                    None
                }
            };

            Ok(Box::new(WbarApp::new(
                cfg,
                config_path,
                hot,
                glazewm,
                tray,
                ipc_rx,
            )))
        }),
    )
}

/// Inspect argv. Returns `Some(exit_code)` if a subcommand was handled (the
/// process should exit with that code); `None` if no subcommand was given
/// and the bar should run normally.
#[allow(clippy::print_stdout, clippy::print_stderr)]
fn handle_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return None;
    }
    let cmd = match args[1].as_str() {
        "--help" | "-h" | "help" => {
            print_usage(&args[0]);
            return Some(0);
        }
        "toggle" | "show" | "hide" | "quit" => args[1].clone(),
        "set-theme" => {
            let Some(name) = args.get(2) else {
                eprintln!("set-theme requires a theme name");
                eprintln!("usage: {} set-theme <Paper|Stone|Sage|Clay|Ink>", args[0]);
                return Some(2);
            };
            format!("set-theme {name}")
        }
        other => {
            eprintln!("unknown command: {other}");
            print_usage(&args[0]);
            return Some(2);
        }
    };

    match ipc::send(&cmd) {
        Ok(reply) => {
            if let Some(rest) = reply.strip_prefix("error:") {
                eprintln!("error:{rest}");
                Some(1)
            } else {
                Some(0)
            }
        }
        Err(err) => {
            eprintln!("ipc error: {err:#}");
            Some(1)
        }
    }
}

#[allow(clippy::print_stdout, clippy::print_stderr)]
fn print_usage(prog: &str) {
    eprintln!("wbar — minimalist status bar for Windows + GlazeWM");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  {prog}                     Run the bar (no arguments)");
    eprintln!("  {prog} toggle              Show/hide the bar");
    eprintln!("  {prog} show                Show the bar");
    eprintln!("  {prog} hide                Hide the bar (releases the AppBar reservation)");
    eprintln!("  {prog} quit                Exit the running bar");
    eprintln!("  {prog} set-theme <name>    Switch theme (Paper|Stone|Sage|Clay|Ink)");
    eprintln!("  {prog} --help              Show this message");
}

struct WbarApp {
    cfg: Config,
    /// Path where Config::save writes. Tray theme switcher and IPC
    /// set-theme persist through this. None when default_path() couldn't
    /// resolve a config dir (e.g. cargo-check on Linux without HOME).
    config_path: Option<PathBuf>,
    widgets: Widgets,
    glazewm: GlazewmClient,
    pinned: bool,
    visible: bool,
    hot: Option<HotReload>,
    appbar: Option<AppBar>,
    tray: Option<Tray>,
    ipc_rx: Option<Receiver<IpcCommand>>,
}

impl WbarApp {
    fn new(
        cfg: Config,
        config_path: Option<PathBuf>,
        hot: Option<HotReload>,
        glazewm: GlazewmClient,
        tray: Option<Tray>,
        ipc_rx: Option<Receiver<IpcCommand>>,
    ) -> Self {
        let palette = cfg.effective_palette();
        let radius = cfg.effective_tokens().radius_sm;
        let widgets = Widgets::from_config(&cfg, &palette, radius, &glazewm);
        Self {
            cfg,
            config_path,
            widgets,
            glazewm,
            pinned: false,
            visible: true,
            hot,
            appbar: None,
            tray,
            ipc_rx,
        }
    }

    /// Drain IPC + tray events and reconcile visibility / theme. Returns
    /// true if the app should exit on this frame (Quit was received).
    fn handle_controls(&mut self, ctx: &egui::Context) -> bool {
        let prev_visible = self.visible;
        let mut quit_requested = false;

        // Tray menu first — its events feed the same command set.
        if let Some(t) = &self.tray
            && let Some(event) = tray::poll(t)
        {
            match event {
                TrayEvent::Toggle => self.visible = !self.visible,
                TrayEvent::Quit => quit_requested = true,
                TrayEvent::SetTheme(theme) => self.apply_theme_persistent(ctx, theme),
            }
        }

        // Then any pending IPC commands. Drain into a Vec first so the
        // receiver borrow ends before we call &mut self methods like
        // apply_theme inside the dispatch loop.
        let pending: Vec<IpcCommand> = if let Some(rx) = &self.ipc_rx {
            std::iter::from_fn(|| rx.try_recv().ok()).collect()
        } else {
            Vec::new()
        };
        for cmd in pending {
            tracing::debug!(?cmd, "applying ipc command");
            match cmd {
                IpcCommand::Toggle => self.visible = !self.visible,
                IpcCommand::Show => self.visible = true,
                IpcCommand::Hide => self.visible = false,
                IpcCommand::Quit => quit_requested = true,
                IpcCommand::SetTheme(theme) => self.apply_theme_persistent(ctx, theme),
            }
        }

        if self.visible != prev_visible {
            self.apply_visibility(ctx);
        }
        quit_requested
    }

    fn apply_visibility(&mut self, ctx: &egui::Context) {
        if self.visible {
            tracing::info!("showing bar");
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            // The next register_appbar() call will re-claim AppBar space.
        } else {
            tracing::info!("hiding bar");
            // Drop the AppBar first so other windows reflow before the
            // hide takes effect, avoiding a one-frame visual glitch.
            self.appbar = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context, theme: Theme) {
        tracing::info!(?theme, "switching theme");
        self.cfg.theme = theme;
        let palette = self.cfg.effective_palette();
        let radius = self.cfg.effective_tokens().radius_sm;
        theme::apply(ctx, &palette, theme::is_dark(theme));
        self.widgets = Widgets::from_config(&self.cfg, &palette, radius, &self.glazewm);
    }

    /// Apply a theme and persist the choice to disk so it survives a
    /// restart. Used by the tray theme submenu (and the IPC set-theme
    /// handler in the next commit). Save errors are warn-logged but don't
    /// block the in-memory apply.
    fn apply_theme_persistent(&mut self, ctx: &egui::Context, theme: Theme) {
        self.apply_theme(ctx, theme);
        if let Some(path) = &self.config_path
            && let Err(err) = self.cfg.save(path)
        {
            tracing::warn!(error = ?err, "saving config after theme change");
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
        let quit = self.handle_controls(ctx);
        if quit {
            tracing::info!("quit requested (tray or ipc)");
            // Dropping WbarApp on close also drops the AppBar, which issues
            // ABM_REMOVE — the taskbar reflows immediately.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.visible {
            // Skip pin / register / render when hidden; the window is
            // already invisible and the AppBar reservation released. IPC
            // can still wake us up to flip back to visible.
            return;
        }
        self.pin_to_edge(ctx);
        self.register_appbar(frame);

        // CentralPanel defaults to a Frame with ~8px margins on every side,
        // which would eat most of a 28px bar and leave too little vertical
        // room for text to centre. Replace it with a zero-vertical-margin
        // frame so widgets get the full bar height; horizontal margin
        // (BAR_EDGE_PAD) is the breathing room between the bar contents and
        // the screen edges.
        let bg = ctx.style().visuals.panel_fill;
        let frame_style = egui::Frame::new()
            .fill(bg)
            .inner_margin(egui::Margin::symmetric(BAR_EDGE_PAD, 0));
        egui::CentralPanel::default()
            .frame(frame_style)
            .show(ctx, |ui| {
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
        // horizontal_centered (vs plain horizontal) makes the outer row fill
        // the panel's height and lays out children with cross-axis = Center,
        // so labels sit on the bar's vertical midline instead of the top.
        ui.horizontal_centered(|ui| {
            let bar_h = self.bar_height();
            let third = ui.available_width() / 3.0;
            let slot = egui::vec2(third, bar_h);

            ui.allocate_ui_with_layout(
                slot,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    // Force the slot to claim its full third even when no
                    // widget renders anything (e.g. workspaces hidden because
                    // glazewm isn't running). Without this, allocate_ui only
                    // advances the parent cursor by the actual contents size,
                    // so an empty left slot collapses to zero and the centre
                    // clock slides into the left third.
                    ui.set_min_size(slot);
                    ui.spacing_mut().item_spacing.x = REGION_ITEM_SPACING;
                    for id in self.cfg.layout.left.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                slot,
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.set_min_size(slot);
                    // No inner horizontal_centered: that wrapper builds a new
                    // left-to-right layout starting at the slot's left edge,
                    // pushing the clock to the left. centered_and_justified
                    // by itself centres a single label both axes.
                    for id in self.cfg.layout.center.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                slot,
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.set_min_size(slot);
                    ui.spacing_mut().item_spacing.x = REGION_ITEM_SPACING;
                    // In a right_to_left layout, add_space is consumed at the
                    // right edge first. This nudges the first widget inward
                    // so glyphs with positive right-side bearing (some Nerd
                    // Font battery icons) don't paint past the slot edge.
                    ui.add_space(RIGHT_EDGE_CUSHION);
                    for id in self.cfg.layout.right.clone() {
                        self.widgets.render(ui, &id);
                    }
                },
            );
        });
    }
}
