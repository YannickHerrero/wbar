use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use super::Widget;
use crate::config::{SysinfoConfig, SysinfoMetric};

const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Pulls a metric from sysinfo on a configurable interval and renders it via
/// the user's format string (Python-style `{name}` / `{name:spec}` keys).
pub struct SysinfoWidget {
    cfg: SysinfoConfig,
    system: System,
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
            // Network is added in the next commit.
            SysinfoMetric::Network => RefreshKind::nothing(),
        };
        Self {
            cfg,
            system: System::new_with_specifics(refresh),
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
            SysinfoMetric::Network => None,
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
