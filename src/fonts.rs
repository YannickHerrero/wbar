//! Discover Nerd-Font-patched files from the system font directories and
//! register them with egui:
//!   - a SemiBold / Medium / Bold variant (if found) becomes the *primary*
//!     entry in both Proportional and Monospace families, so all body text
//!     renders heavier.
//!   - a Regular variant (if found) is appended as a fallback so any glyph
//!     missing from the bold variant — typically nothing — still renders.
//!
//! We don't bundle any fonts: the assumption is the user already has at
//! least one Nerd Font installed. When nothing is found we log one info
//! line and continue with egui's bundled defaults.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// Filenames we recognise as regular-weight Nerd Font candidates, ordered by
/// preference. Mono variants lay out more predictably in a status bar.
const REGULAR_NERD_FONT_FILENAMES: &[&str] = &[
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

/// Heavier-weight variants to use for body text. SemiBold first (the
/// requested weight), Medium as a softer compromise, Bold as a last resort.
const BODY_NERD_FONT_FILENAMES: &[&str] = &[
    // SemiBold — exactly what the user asked for
    "JetBrainsMonoNerdFontMono-SemiBold.ttf",
    "JetBrainsMonoNLNerdFontMono-SemiBold.ttf",
    "JetBrainsMonoNerdFont-SemiBold.ttf",
    "JetBrainsMonoNFM-SemiBold.ttf",
    "0xProtoNerdFontMono-SemiBold.ttf",
    // Medium — nearly indistinguishable from SemiBold at 12pt
    "JetBrainsMonoNerdFontMono-Medium.ttf",
    "JetBrainsMonoNLNerdFontMono-Medium.ttf",
    "JetBrainsMonoNerdFont-Medium.ttf",
    "JetBrainsMonoNFM-Medium.ttf",
    "IosevkaNerdFontMono-Medium.ttf",
    "FiraCodeNerdFontMono-Medium.ttf",
    // Bold — fallback if no lighter heavy variant is installed
    "JetBrainsMonoNerdFontMono-Bold.ttf",
    "JetBrainsMonoNLNerdFontMono-Bold.ttf",
    "JetBrainsMonoNerdFont-Bold.ttf",
    "JetBrainsMonoNFM-Bold.ttf",
    "IosevkaNerdFontMono-Bold.ttf",
    "FiraCodeNerdFontMono-Bold.ttf",
    "HackNerdFontMono-Bold.ttf",
];

/// Install body + fallback fonts. Idempotent: always rebuilds from
/// FontDefinitions::default() so it's safe to re-run.
pub fn install_nerd_font_fallback(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // Body font: heavier variant promoted to primary in both families.
    // Without this everything stays at egui's bundled regular weight.
    if let Some((path, bytes)) = discover(BODY_NERD_FONT_FILENAMES) {
        tracing::info!(path = %path.display(), "loaded semi-bold body font");
        fonts
            .font_data
            .insert("body".to_owned(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.insert(0, "body".to_owned());
            }
        }
    } else {
        tracing::info!(
            "no SemiBold/Medium/Bold Nerd Font variant found — body text stays regular weight",
        );
    }

    // Glyph fallback: appended at the end so the body font wins for any
    // glyph it has; missing codepoints (rare for full Nerd Fonts) fall
    // through to this.
    if let Some((path, bytes)) = discover(REGULAR_NERD_FONT_FILENAMES) {
        tracing::info!(path = %path.display(), "loaded Nerd Font glyph fallback");
        fonts
            .font_data
            .insert("nerd".to_owned(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Monospace, FontFamily::Proportional] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("nerd".to_owned());
            }
        }
    } else {
        tracing::info!(
            "no Nerd Font found in system font directories — icon glyphs in widget format strings will not render",
        );
    }

    ctx.set_fonts(fonts);
}

fn discover(filenames: &[&str]) -> Option<(PathBuf, Vec<u8>)> {
    for dir in font_dirs() {
        for name in filenames {
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
