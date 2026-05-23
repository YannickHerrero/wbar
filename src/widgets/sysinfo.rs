use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

use super::Widget;
use crate::config::{SysinfoConfig, SysinfoMetric};
use crate::theme::Palette;

const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

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

/// Minimum visual width (chars) for the rendered value half. Pads with
/// leading spaces so consecutive samples like "9%" and "100%" occupy the
/// same horizontal real estate, keeping neighbouring widgets from
/// jittering as digit counts change. The user's format spec can produce
/// a longer string (e.g. "RAM 12.3G") — in that case no padding is
/// added.
const VALUE_MIN_WIDTH: usize = 4;

impl Widget for SysinfoWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        self.refresh_if_due();

        // Single label per widget — keeps each sysinfo widget atomic in
        // the parent region's flow. A previous attempt used ui.with_layout
        // to force icon-before-value ordering inside a right_to_left
        // parent, but with_layout reserves the parent's available_size
        // for the child, so each widget claimed the whole remaining
        // region width and consecutive widgets started overlapping.
        // Embedding the icon directly in the rendered string sidesteps
        // both the direction-inheritance issue and the rect-overclaim
        // issue.
        let padded = pad_left_to(&self.rendered, VALUE_MIN_WIDTH);
        let body = if let Some(icon) = self.current_icon() {
            // Single space between icon and value, always — for 3-digit
            // values like "100%" the padding alone wouldn't leave a gap
            // and the icon would touch the first digit. The total width
            // is still constant (icon + 1 + VALUE_MIN_WIDTH) so the
            // no-jitter property is preserved.
            format!("{icon} {padded}")
        } else {
            padded
        };
        if self.should_warn() {
            ui.colored_label(self.warn_color, body);
        } else {
            ui.label(body);
        }

        ui.ctx().request_repaint_after(self.interval());
    }
}

/// Right-align `s` within `min` characters by prefixing spaces. Strings
/// already `>= min` chars wide are returned unchanged. Uses `chars().count()`
/// so multi-byte glyphs in the value half (rare) still count correctly.
fn pad_left_to(s: &str, min: usize) -> String {
    let len = s.chars().count();
    if len >= min {
        s.to_string()
    } else {
        let pad = min - len;
        let mut out = String::with_capacity(pad + s.len());
        for _ in 0..pad {
            out.push(' ');
        }
        out.push_str(s);
        out
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

#[cfg(windows)]
fn read_battery() -> Option<Battery> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: GetSystemPowerStatus writes into the pointer we pass; it does
    // not retain it after returning.
    let ok = unsafe { GetSystemPowerStatus(&mut status) };
    if ok.is_err() {
        return None;
    }
    // BatteryLifePercent == 255 means "unknown"; BatteryFlag bit 7 (0x80)
    // means "no system battery present" — both should hide the widget.
    if status.BatteryLifePercent == 255 || status.BatteryFlag & 0x80 != 0 {
        return None;
    }
    Some(Battery {
        percent: f64::from(status.BatteryLifePercent),
        // ACLineStatus: 0 = offline, 1 = online, 255 = unknown.
        charging: status.ACLineStatus == 1,
    })
}

#[cfg(not(windows))]
fn read_battery() -> Option<Battery> {
    None
}
