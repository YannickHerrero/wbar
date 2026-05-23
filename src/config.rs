use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::theme::{self, HexColor, Palette, Theme, Tokens};

const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bar: BarConfig,
    pub font: FontConfig,
    pub theme: Theme,
    pub palette: PaletteOverride,
    pub tokens: TokensOverride,
    pub layout: LayoutConfig,
    pub widgets: BTreeMap<String, WidgetConfig>,
}

impl Config {
    /// Parse the embedded starter config. Used as a first-run fallback so the
    /// bar shows useful widgets out of the box. Distinct from `Default` because
    /// `Default` must not recurse through `toml::from_str` (serde calls it for
    /// missing-field fallback during deserialization).
    pub fn embedded_default() -> Self {
        toml::from_str::<Self>(DEFAULT_CONFIG).expect("embedded default config must parse")
    }

    /// Palette for the selected theme with `[palette]` overrides applied.
    pub fn effective_palette(&self) -> Palette {
        let mut p = theme::palette(self.theme);
        self.palette.apply_to(&mut p);
        p
    }

    /// Token set with `[tokens]` overrides applied.
    pub fn effective_tokens(&self) -> Tokens {
        let mut t = theme::tokens();
        self.tokens.apply_to(&mut t);
        t
    }
}

/// Per-field optional overrides of the selected theme's palette. Omit a field
/// and it falls back to the theme's value.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PaletteOverride {
    pub paper: Option<HexColor>,
    pub ink: Option<HexColor>,
    pub accent: Option<HexColor>,
    pub ink_soft: Option<HexColor>,
    pub ink_faint: Option<HexColor>,
    pub muted: Option<HexColor>,
    pub success: Option<HexColor>,
    pub warning: Option<HexColor>,
    pub error: Option<HexColor>,
}

impl PaletteOverride {
    fn apply_to(&self, p: &mut Palette) {
        if let Some(c) = self.paper {
            p.paper = c.0;
        }
        if let Some(c) = self.ink {
            p.ink = c.0;
        }
        if let Some(c) = self.accent {
            p.accent = c.0;
        }
        if let Some(c) = self.ink_soft {
            p.ink_soft = c.0;
        }
        if let Some(c) = self.ink_faint {
            p.ink_faint = c.0;
        }
        if let Some(c) = self.muted {
            p.muted = c.0;
        }
        if let Some(c) = self.success {
            p.success = c.0;
        }
        if let Some(c) = self.warning {
            p.warning = c.0;
        }
        if let Some(c) = self.error {
            p.error = c.0;
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TokensOverride {
    pub space_xs: Option<f32>,
    pub space_sm: Option<f32>,
    pub space_md: Option<f32>,
    pub space_lg: Option<f32>,
    pub space_xl: Option<f32>,
    pub radius_sm: Option<f32>,
    pub radius_md: Option<f32>,
    pub font_body: Option<f32>,
    pub font_section_title: Option<f32>,
    pub font_page_title: Option<f32>,
    pub field_label_width: Option<f32>,
}

impl TokensOverride {
    fn apply_to(&self, t: &mut Tokens) {
        if let Some(v) = self.space_xs {
            t.space_xs = v;
        }
        if let Some(v) = self.space_sm {
            t.space_sm = v;
        }
        if let Some(v) = self.space_md {
            t.space_md = v;
        }
        if let Some(v) = self.space_lg {
            t.space_lg = v;
        }
        if let Some(v) = self.space_xl {
            t.space_xl = v;
        }
        if let Some(v) = self.radius_sm {
            t.radius_sm = v;
        }
        if let Some(v) = self.radius_md {
            t.radius_md = v;
        }
        if let Some(v) = self.font_body {
            t.font_body = v;
        }
        if let Some(v) = self.font_section_title {
            t.font_section_title = v;
        }
        if let Some(v) = self.font_page_title {
            t.font_page_title = v;
        }
        if let Some(v) = self.field_label_width {
            t.field_label_width = v;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub position: BarPosition,
    pub height: f32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            position: BarPosition::default(),
            height: 32.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "Default".into(),
            size: 13.0,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum WidgetConfig {
    Glazewm(WorkspacesConfig),
    Clock(ClockConfig),
    Sysinfo(SysinfoConfig),
    Command(CommandConfig),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WorkspacesConfig {
    pub show_empty: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClockConfig {
    pub format: String,
    pub tick_seconds: u64,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            format: "%a %d %b  %H:%M".into(),
            tick_seconds: 1,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SysinfoConfig {
    pub metric: SysinfoMetric,
    pub format: String,
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: u64,
    #[serde(default)]
    pub interface: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SysinfoMetric {
    Cpu,
    Ram,
    Network,
}

fn default_interval_seconds() -> u64 {
    2
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandConfig {
    pub command: String,
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: u64,
}

/// Default config path: `%APPDATA%\wbar\config.toml` on Windows,
/// `$XDG_CONFIG_HOME/wbar/config.toml` elsewhere (useful for cross-target
/// development).
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("wbar").join("config.toml"))
}

/// Load the user's config, falling back to the embedded default if the file
/// does not exist. Returns an error only when the file is present but
/// unreadable or malformed.
pub fn load(path: Option<&std::path::Path>) -> Result<Config> {
    let Some(path) = path else {
        return Ok(Config::embedded_default());
    };
    if !path.exists() {
        tracing::info!(path = %path.display(), "config not found, using embedded default");
        return Ok(Config::embedded_default());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    let cfg = toml::from_str::<Config>(&raw)
        .with_context(|| format!("parsing config at {}", path.display()))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_default_config_parses() {
        let cfg = Config::embedded_default();
        assert_eq!(cfg.bar.position, BarPosition::Top);
        assert_eq!(cfg.theme, Theme::Paper);
        assert!(!cfg.widgets.is_empty());
    }

    #[test]
    fn literal_default_is_empty_but_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.theme, Theme::Paper);
        assert!(cfg.widgets.is_empty());
    }
}
