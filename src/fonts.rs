//! Discover a Nerd-Font-patched file from the system font directories and
//! register it as a fallback in egui's monospace + proportional families.
//!
//! We don't bundle a font: the assumption is the user already has at least
//! one Nerd Font installed (anyone using zebar/glazewm style status bars
//! invariably does). When no NF file is found we log one info-level note and
//! widget format strings that contain icon glyphs render as tofu, which is
//! the correct signal that nothing is wired up.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// Filenames we recognise as Nerd Font candidates, ordered by preference.
/// Mono variants are smaller and lay out more predictably in a status bar.
const NERD_FONT_FILENAMES: &[&str] = &[
    "SymbolsNerdFontMono-Regular.ttf",
    "SymbolsNerdFont-Regular.ttf",
    "JetBrainsMonoNerdFontMono-Regular.ttf",
    "JetBrainsMonoNLNerdFontMono-Regular.ttf",
    "JetBrainsMonoNerdFont-Regular.ttf",
    "JetBrainsMonoNFM-Regular.ttf",
    "IosevkaNerdFontMono-Regular.ttf",
    "FiraCodeNerdFontMono-Regular.ttf",
    "HackNerdFontMono-Regular.ttf",
];

/// Install a Nerd Font (if one is discoverable) as a fallback in egui's
/// monospace and proportional families. Idempotent: re-running on hot reload
/// is fine because we always rebuild from FontDefinitions::default().
pub fn install_nerd_font_fallback(ctx: &egui::Context) {
    let Some((path, bytes)) = discover_nerd_font() else {
        tracing::info!(
            "no Nerd Font found in system font directories — icon glyphs in widget format strings will not render",
        );
        return;
    };
    tracing::info!(path = %path.display(), "loaded Nerd Font fallback");

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("nerd".to_owned(), Arc::new(FontData::from_owned(bytes)));
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("nerd".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

fn discover_nerd_font() -> Option<(PathBuf, Vec<u8>)> {
    for dir in font_dirs() {
        for name in NERD_FONT_FILENAMES {
            let candidate = dir.join(name);
            if let Ok(bytes) = std::fs::read(&candidate) {
                return Some((candidate, bytes));
            }
        }
    }
    None
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // Per-user font dir (where most modern Windows font installs land).
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        dirs.push(
            PathBuf::from(localappdata)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }
    // Machine-wide font dir.
    if let Ok(windir) = std::env::var("WINDIR") {
        dirs.push(PathBuf::from(windir).join("Fonts"));
    }
    dirs
}
