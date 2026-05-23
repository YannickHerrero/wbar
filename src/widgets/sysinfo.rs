use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

use super::Widget;
use crate::config::{SysinfoConfig, SysinfoMetric};

const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Pulls a metric from sysinfo on a configurable interval and renders it via
/// the user's format string (Python-style `{name}` / `{name:spec}` keys).
pub struct SysinfoWidget {
    cfg: SysinfoConfig,
    system: System,
    networks: Option<Networks>,
    last_sample: Option<Instant>,
    rendered: String,
}

impl SysinfoWidget {
    pub fn new(cfg: SysinfoConfig) -> Self {
        let refresh = match cfg.metric {
            SysinfoMetric::Cpu => RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
            SysinfoMetric::Ram => {
                RefreshKind::nothing().with_memory(MemoryRefreshKind::everything())
            }
            SysinfoMetric::Network => RefreshKind::nothing(),
        };
        let networks =
            matches!(cfg.metric, SysinfoMetric::Network).then(Networks::new_with_refreshed_list);
        Self {
            cfg,
            system: System::new_with_specifics(refresh),
            networks,
            last_sample: None,
            rendered: String::new(),
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
        };

        if let Some(vars) = vars {
            self.rendered = format_with(&self.cfg.format, &vars);
        }
    }
}

impl Widget for SysinfoWidget {
    fn render(&mut self, ui: &mut egui::Ui) {
        self.refresh_if_due();
        ui.label(&self.rendered);
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
