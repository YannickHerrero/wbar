use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Sense, TextStyle, vec2};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

use super::Widget;
use crate::config::{SysinfoConfig, SysinfoMetric};
use crate::theme::Palette;

const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Fixed pixel width each sysinfo metric occupies. Picked to fit
/// "icon 100%" comfortably at 12pt monospace; long custom formats
/// (network throughput etc.) grow the slot via the max() below so they
/// don't get clipped. Keeping a fixed minimum means single-digit values
/// don't make the slot collapse and shift neighbours.
const SLOT_WIDTH: f32 = 60.0;

/// Pulls a metric from sysinfo on a configurable interval and renders it via
/// the user's format string (Python-style `{name}` / `{name:spec}` keys).
pub struct SysinfoWidget {
    cfg: SysinfoConfig,
    system: System,
    networks: Option<Networks>,
    last_sample: Option<Instant>,
    rendered: String,
    /// Most recent `value` extracted from the sample. Compared against
    /// `cfg.warn_above` to decide whether to apply the warn tint.
    current_value: Option<f64>,
    /// Most recent `charging` flag for the Battery metric. Used to pick
    /// between `cfg.icon` and `cfg.charging_icon`.
    current_charging: bool,
    /// Resolved warn colour: explicit `cfg.warn_color` if set, otherwise
    /// `palette.error`. Stored to avoid re-resolving on every frame.
    warn_color: Color32,
}

impl SysinfoWidget {
    pub fn new(cfg: SysinfoConfig, palette: &Palette) -> Self {
        let refresh = match cfg.metric {
            SysinfoMetric::Cpu => RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
            SysinfoMetric::Ram => {
                RefreshKind::nothing().with_memory(MemoryRefreshKind::everything())
            }
            SysinfoMetric::Network => RefreshKind::nothing(),
            SysinfoMetric::Battery => RefreshKind::nothing(),
        };
        let networks =
            matches!(cfg.metric, SysinfoMetric::Network).then(Networks::new_with_refreshed_list);
        let warn_color = cfg.warn_color.map(|c| c.0).unwrap_or(palette.error);
        Self {
            cfg,
            system: System::new_with_specifics(refresh),
            networks,
            last_sample: None,
            rendered: String::new(),
            current_value: None,
            current_charging: false,
            warn_color,
        }
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(self.cfg.interval_seconds.max(1))
    }

    fn refresh_if_due(&mut self) {
        let due = match self.last_sample {
            Some(t) => t.elapsed() >= self.interval(),
            None => true,
        };
        if !due {
            return;
        }
        let elapsed_secs = self
            .last_sample
            .map(|t| t.elapsed().as_secs_f64().max(0.001))
            .unwrap_or(self.cfg.interval_seconds.max(1) as f64);
        self.last_sample = Some(Instant::now());

        let vars = match self.cfg.metric {
            SysinfoMetric::Cpu => {
                self.system.refresh_cpu_usage();
                let avg = average_cpu(&self.system);
                let mut m = HashMap::new();
                m.insert("value".into(), avg as f64);
                Some(m)
            }
            SysinfoMetric::Ram => {
                self.system.refresh_memory();
                let used = self.system.used_memory() as f64;
                let total = self.system.total_memory() as f64;
                let mut m = HashMap::new();
                m.insert("used_gb".into(), used / BYTES_PER_GB);
                m.insert("total_gb".into(), total / BYTES_PER_GB);
                m.insert("free_gb".into(), (total - used).max(0.0) / BYTES_PER_GB);
                m.insert(
                    "value".into(),
                    if total > 0.0 {
                        used / total * 100.0
                    } else {
                        0.0
                    },
                );
                Some(m)
            }
            SysinfoMetric::Network => {
                let Some(networks) = self.networks.as_mut() else {
                    return;
                };
                networks.refresh(false);

                let mut rx: u64 = 0;
                let mut tx: u64 = 0;
                let match_all =
                    matches!(self.cfg.interface.as_deref(), None | Some("*") | Some(""));
                for (name, data) in networks.iter() {
                    let keep = match_all
                        || self
                            .cfg
                            .interface
                            .as_deref()
                            .map(|i| i == name)
                            .unwrap_or(false);
                    if !keep {
                        continue;
                    }
                    rx += data.received();
                    tx += data.transmitted();
                }

                let rx_bps = rx as f64 / elapsed_secs;
                let tx_bps = tx as f64 / elapsed_secs;
                let mut m = HashMap::new();
                m.insert("rx_bps".into(), rx_bps);
                m.insert("tx_bps".into(), tx_bps);
                m.insert("rx_kbps".into(), rx_bps / 1024.0);
                m.insert("tx_kbps".into(), tx_bps / 1024.0);
                m.insert("rx_mbps".into(), rx_bps / (1024.0 * 1024.0));
                m.insert("tx_mbps".into(), tx_bps / (1024.0 * 1024.0));
                Some(m)
            }
            SysinfoMetric::Battery => read_battery().map(|b| {
                self.current_charging = b.charging;
                let mut m = HashMap::new();
                m.insert("value".into(), b.percent);
                m.insert("charging".into(), if b.charging { 1.0 } else { 0.0 });
                m
            }),
        };

        if let Some(vars) = vars {
            self.current_value = vars.get("value").copied();
            self.rendered = format_with(&self.cfg.format, &vars);
        }
    }

    /// Pick between `cfg.icon` and `cfg.charging_icon` based on the most
    /// recent battery sample. Non-Battery metrics always use `cfg.icon`.
    fn current_icon(&self) -> Option<&str> {
        if matches!(self.cfg.metric, SysinfoMetric::Battery) && self.current_charging {
            self.cfg
                .charging_icon
                .as_deref()
                .or(self.cfg.icon.as_deref())
        } else {
            self.cfg.icon.as_deref()
        }
    }

    fn should_warn(&self) -> bool {
        match (self.cfg.warn_above, self.current_value) {
            (Some(threshold), Some(value)) => value > threshold,
            _ => false,
        }
    }
}

impl Widget for SysinfoWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        self.refresh_if_due();

        // Compose "icon value" at natural width — no padding inside the
        // string. Width-stability comes from the fixed-pixel slot below,
        // not from char counting.
        let body = if let Some(icon) = self.current_icon() {
            format!("{icon} {}", self.rendered)
        } else {
            self.rendered.clone()
        };

        let color = if self.should_warn() {
            self.warn_color
        } else {
            ui.visuals().text_color()
        };

        // Lay out the text first so we know its natural size.
        let font_id = TextStyle::Body.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(body, font_id, color);

        // Allocate a fixed-width slot (or larger if the text overflows).
        // The parent region's cross_align=Center centres this rect
        // vertically in the bar; we centre the text inside the rect.
        // Result: value-width changes (e.g. "9%" → "100%") only shift
        // the text *within* this widget's slot — neighbouring widgets
        // never move.
        let slot_w = SLOT_WIDTH.max(galley.size().x);
        let slot_size = vec2(slot_w, galley.size().y);
        let (rect, _) = ui.allocate_exact_size(slot_size, Sense::hover());
        let text_pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(text_pos, galley, color);

        ui.ctx().request_repaint_after(self.interval());
    }
}

fn average_cpu(sys: &System) -> f32 {
    let cpus = sys.cpus();
    if cpus.is_empty() {
        return 0.0;
    }
    cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
}

/// Run `strfmt` and fall back to the raw template on error so a bad placeholder
/// doesn't blank the widget.
pub(super) fn format_with(template: &str, vars: &HashMap<String, f64>) -> String {
    match strfmt::strfmt(template, vars) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(template, error = ?err, "sysinfo format failed");
            template.to_string()
        }
    }
}

struct Battery {
    percent: f64,
    charging: bool,
}

/// Cross-platform battery snapshot via the starship-battery crate (which
/// wraps IOKit on macOS, GetSystemPowerStatus on Windows, and /sys
/// power_supply on Linux).
///
/// Returns None when no battery is present (desktops) or when the OS
/// can't report a state of charge — the widget hides itself in either
/// case, matching the previous Win32-only behaviour.
fn read_battery() -> Option<Battery> {
    let manager = starship_battery::Manager::new().ok()?;
    let battery = manager.batteries().ok()?.next()?.ok()?;
    // state_of_charge() returns a uom Ratio in 0.0..=1.0; multiply for %.
    let percent = f64::from(battery.state_of_charge().value) * 100.0;
    // Treat Full + Charging both as "on AC" so the charging_icon stays
    // shown while the laptop sits plugged in at 100%. Discharging /
    // Unknown / Empty fall through to "on battery".
    let charging = matches!(
        battery.state(),
        starship_battery::State::Charging | starship_battery::State::Full
    );
    Some(Battery { percent, charging })
}
