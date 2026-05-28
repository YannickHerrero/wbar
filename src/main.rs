// Release builds run as a Windows GUI app (no console window). Debug builds
// stay on the console subsystem so `cargo run` still shows tracing output.
// The attribute is a no-op on non-Windows targets but gating it avoids a
// "unknown attribute" lint when cross-compiling.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

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
mod wake;
mod widgets;

use eframe::egui;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

use std::sync::mpsc::Receiver;

use crate::appbar::{AppBar, Edge};
use crate::config::{BarPosition, Config};
use crate::glazewm::GlazewmClient;
use crate::hotreload::HotReload;
use crate::ipc::IpcCommand;
use crate::theme::Theme;
use std::path::PathBuf;

use crate::tray::{Tray, TrayEvent};
use crate::wake::Waker;
use crate::widgets::Widgets;

/// Horizontal padding between the bar contents and the screen edges,
/// applied as the CentralPanel frame's inner margin.
const BAR_EDGE_PAD: i8 = 16;
/// Extra cushion inside the right region so Nerd-Font glyphs with positive
/// right-side bearing don't paint past the slot edge.
const RIGHT_EDGE_CUSHION: f32 = 8.0;
/// Gap between consecutive widgets within the left and right regions.
const REGION_ITEM_SPACING: f32 = 4.0;

fn main() -> eframe::Result {
    // CLI client mode: any first argv that matches a known subcommand sends
    // the command to the already-running wbar over IPC and exits, instead of
    // booting a second bar instance.
    if let Some(code) = handle_cli() {
        std::process::exit(code);
    }

    // macOS: declare the app as a menu-bar accessory before winit/eframe
    // initialises NSApplication so the brief "regular app" visual (Dock
    // tile, top main menu) never appears. No-op on other targets.
    set_macos_accessory_policy();

    let config_path = config::default_path();
    init_tracing(config_path.as_deref());
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

            // A status bar shouldn't expose drag-selection on its labels —
            // the cursor changes to a text caret on hover and click-drag
            // selects the value, which is noise nobody wants here.
            cc.egui_ctx.style_mut(|s| {
                s.interaction.selectable_labels = false;
            });

            // A shared cross-thread waker. Background subsystems (tray,
            // IPC, glazewm, hot-reload) call waker.wake() after notifying
            // egui so winit's pump actually runs and update() consumes
            // the events. The HWND inside it is armed by appbar::register
            // on the first frame; until then wake() is a no-op (no
            // background events fire that early).
            let waker = Waker::new();

            let hot = config_path.clone().and_then(|p| {
                match hotreload::spawn(p, cc.egui_ctx.clone(), waker.clone()) {
                    Ok(h) => Some(h),
                    Err(err) => {
                        tracing::warn!(error = ?err, "hot reload disabled");
                        None
                    }
                }
            });

            let glazewm = GlazewmClient::spawn(cc.egui_ctx.clone(), waker.clone());

            let tray = match tray::build(cc.egui_ctx.clone(), waker.clone()) {
                Ok(t) => Some(t),
                Err(err) => {
                    tracing::warn!(error = ?err, "tray icon disabled");
                    None
                }
            };

            let ipc_rx = match ipc::spawn(cc.egui_ctx.clone(), waker.clone()) {
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
                waker,
            )))
        }),
    )
}

/// Set the macOS application activation policy to `.accessory` so wbar
/// behaves like a menu-bar utility — no Dock tile, no "wbar" entry in
/// the system-wide top menu bar, doesn't steal focus on launch. Safe to
/// call before NSApplication is fully bootstrapped: `sharedApplication`
/// is idempotent and creates the singleton on the first call.
#[cfg(target_os = "macos")]
fn set_macos_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let mtm = objc2::MainThreadMarker::new()
        .expect("set_macos_accessory_policy must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    tracing::debug!("set macOS NSApplication activation policy to Accessory");
}

#[cfg(not(target_os = "macos"))]
fn set_macos_accessory_policy() {}

/// Return (top_inset, bottom_inset) in logical points for the main
/// screen. On macOS this is the system menu-bar height at the top and
/// the Dock height at the bottom (when the Dock is positioned at the
/// bottom — side Docks yield 0). On other platforms returns (0, 0).
///
/// Used by `pin_to_edge` so a top-positioned bar sits below the system
/// menu bar instead of underneath it, and a bottom-positioned bar
/// clears the Dock.
fn screen_insets_top_bottom() -> (f32, f32) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSScreen;
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return (0.0, 0.0);
        };
        let Some(screen) = NSScreen::mainScreen(mtm) else {
            return (0.0, 0.0);
        };
        let frame = screen.frame();
        let visible = screen.visibleFrame();

        // Top inset is forced to 0 on macOS: anyone running a custom
        // top-edge status bar wants the top edge, not "below the menu
        // bar". This intentionally overlaps a permanent menu bar (use
        // System Settings → Control Center → Menu Bar to auto-hide it)
        // and intentionally puts the bar's middle behind the notch on
        // notch Macs (NSScreen.visibleFrame still reports a notch-sized
        // inset there even when auto-hide is enabled, so honouring it
        // would leave the bar parked uselessly below the notch).
        //
        // Bottom inset still honours the Dock: a bottom-positioned bar
        // sitting underneath the Dock would be unreachable, with no
        // reasonable user workaround.
        let dock_bottom = (visible.origin.y - frame.origin.y).max(0.0) as f32;
        tracing::debug!(dock_bottom, "macOS screen insets (top forced to 0)");
        (0.0, dock_bottom)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (0.0, 0.0)
    }
}

/// Initialise tracing with two layers:
///   - stderr (visible in debug builds where the console subsystem is
///     attached; controlled by RUST_LOG, defaults to info).
///   - file at <config_dir>/wbar.log, truncated on every daemon start,
///     fixed at debug level so a release build always has a readable
///     diagnostic trail. Path is logged on the first frame so the user
///     can find it. Falls back to stderr-only if the log file can't be
///     created (e.g. directory missing on first run before save).
fn init_tracing(config_path: Option<&std::path::Path>) {
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(console_filter);

    let log_path = config_path
        .and_then(|p| p.parent())
        .map(|d| d.join("wbar.log"));
    let file_layer = log_path.as_ref().and_then(|path| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        let file = std::fs::File::create(path).ok()?;
        Some(
            tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .with_filter(EnvFilter::new("wbar=debug,info")),
        )
    });

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    if let Some(p) = &log_path {
        tracing::info!(log = %p.display(), "writing debug log to file");
    } else {
        tracing::info!("no config dir resolved, debug log file disabled");
    }
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
    eprintln!("wbar — minimalist status bar for Windows and macOS + GlazeWM");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  {prog}                     Run the bar (no arguments)");
    eprintln!("  {prog} toggle              Show/hide the bar");
    eprintln!("  {prog} show                Show the bar");
    eprintln!("  {prog} hide                Hide the bar (releases the AppBar reservation on Windows)");
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
    /// Cached monitor size from the first frame that reported one. Used by
    /// pin_to_edge so re-pinning works even when the window is currently
    /// parked off-screen and the viewport's reported monitor_size is None.
    monitor_size: Option<egui::Vec2>,
    hot: Option<HotReload>,
    appbar: Option<AppBar>,
    tray: Option<Tray>,
    ipc_rx: Option<Receiver<IpcCommand>>,
    /// Cross-thread waker passed to appbar::register so background
    /// subsystems can wake the eframe main loop via InvalidateRect.
    waker: Waker,
}

impl WbarApp {
    fn new(
        cfg: Config,
        config_path: Option<PathBuf>,
        hot: Option<HotReload>,
        glazewm: GlazewmClient,
        tray: Option<Tray>,
        ipc_rx: Option<Receiver<IpcCommand>>,
        waker: Waker,
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
            monitor_size: None,
            hot,
            appbar: None,
            tray,
            ipc_rx,
            waker,
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
            tracing::info!(?event, prev_visible, "handle_controls applying tray event");
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
            // Force pin_to_edge to run again on the next update, which
            // re-positions the window back on-screen. register_appbar
            // reclaims the AppBar reservation right after.
            self.pinned = false;
        } else {
            tracing::info!("hiding bar");
            // Drop the AppBar first so other windows reflow before the
            // move takes effect, avoiding a one-frame visual glitch.
            self.appbar = None;
            // Park the window far off-screen instead of using
            // ViewportCommand::Visible(false). Hiding the root viewport
            // makes eframe stop scheduling paint cycles for it, and
            // request_repaint() from the tray handler no longer wakes
            // update() — so clicking Toggle a second time would do
            // nothing. Off-screen keeps the viewport "visible" to eframe
            // and the message pump while being invisible to the user.
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                -32000.0, -32000.0,
            )));
            self.pinned = false;
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
    /// register also arms the cross-thread Waker with the HWND on success.
    fn register_appbar(&mut self, frame: &eframe::Frame) {
        if self.appbar.is_some() {
            return;
        }
        let edge = self.edge();
        let height = self.cfg.bar.height as i32;
        self.appbar = appbar::register(frame, edge, height, &self.waker);
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

    /// On the first frame the OS reports a monitor size, stretch the window
    /// to full monitor width at the configured edge. The size is cached so
    /// re-pinning works while the window is parked off-screen during a hide
    /// (where viewport().monitor_size may not report sensibly).
    fn pin_to_edge(&mut self, ctx: &egui::Context) {
        if self.pinned {
            return;
        }
        if self.monitor_size.is_none()
            && let Some(s) = ctx.input(|i| i.viewport().monitor_size)
        {
            self.monitor_size = Some(s);
        }
        let Some(monitor_size) = self.monitor_size else {
            return;
        };
        let height = self.cfg.bar.height;
        // (0, 0) everywhere except macOS — there the bar sits below the
        // system menu bar (Top) or above the Dock (Bottom) instead of
        // overlapping them, since macOS has no AppBar-equivalent shell
        // reservation API for us to claim the strip.
        let (top_inset, bottom_inset) = screen_insets_top_bottom();
        let y = match self.cfg.bar.position {
            BarPosition::Top => top_inset,
            BarPosition::Bottom => monitor_size.y - height - bottom_inset,
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
            top_inset,
            bottom_inset,
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
        // Compute three equal rects explicitly. The previous
        // horizontal_centered + 3·allocate_ui_with_layout chain relied on
        // Layout::left_to_right's default main_align = Center, which
        // re-centred the whole row whenever item_spacing (default ≈8px)
        // pushed the three thirds past the panel width. That left the
        // right slot's right edge well short of the screen edge.
        let max_rect = ui.max_rect();
        let bar_h = self.bar_height();
        let total_w = max_rect.width();
        let third = total_w / 3.0;
        let top = max_rect.top();

        // Reserve the full panel area so the CentralPanel's min_rect
        // matches what we actually paint into.
        ui.allocate_rect(max_rect, egui::Sense::hover());

        let left_rect =
            egui::Rect::from_min_size(egui::pos2(max_rect.left(), top), egui::vec2(third, bar_h));
        let center_rect = egui::Rect::from_min_size(
            egui::pos2(max_rect.left() + third, top),
            egui::vec2(third, bar_h),
        );
        let right_rect = egui::Rect::from_min_size(
            egui::pos2(max_rect.left() + 2.0 * third, top),
            egui::vec2(third, bar_h),
        );

        let left = self.cfg.layout.left.clone();
        let center = self.cfg.layout.center.clone();
        let right = self.cfg.layout.right.clone();

        self.render_region(
            ui,
            left_rect,
            egui::Layout::left_to_right(egui::Align::Center),
            &left,
        );
        self.render_region(
            ui,
            center_rect,
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            &center,
        );
        self.render_region(
            ui,
            right_rect,
            egui::Layout::right_to_left(egui::Align::Center),
            &right,
        );
    }

    fn render_region(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        layout: egui::Layout,
        ids: &[String],
    ) {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
        child.spacing_mut().item_spacing.x = REGION_ITEM_SPACING;
        // For right_to_left, add_space is consumed at the right edge so
        // glyphs with positive right-side bearing don't paint past the
        // slot edge. left_to_right gets a leading cushion for symmetry.
        if matches!(layout.main_dir, egui::Direction::RightToLeft) {
            child.add_space(RIGHT_EDGE_CUSHION);
        }
        for id in ids {
            self.widgets.render(&mut child, id);
        }
    }
}
