use std::fmt;

use eframe::egui::{Color32, Context, Stroke, Visuals};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Deserialize)]
pub enum Theme {
    #[default]
    Paper,
    Stone,
    Sage,
    Clay,
    Ink,
}

pub struct Palette {
    pub paper: Color32,
    pub ink: Color32,
    pub accent: Color32,
    pub ink_soft: Color32,
    pub ink_faint: Color32,
    pub muted: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
}

/// Spacing, radius, and font-size tokens. Centralised so widgets stop
/// sprinkling bare `add_space(8.0)` calls and stay visually consistent
/// across themes.
#[derive(Clone, Copy)]
pub struct Tokens {
    pub space_xs: f32,
    pub space_sm: f32,
    pub space_md: f32,
    pub space_lg: f32,
    pub space_xl: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub font_body: f32,
    pub font_section_title: f32,
    pub font_page_title: f32,
    pub field_label_width: f32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Paper => Palette {
            paper: rgb(0xF4, 0xEB, 0xD9),
            ink: rgb(0x2B, 0x24, 0x1B),
            accent: rgb(0xB5, 0x59, 0x3A),
            ink_soft: rgb(0x6B, 0x5E, 0x4E),
            ink_faint: rgb(0xA4, 0x95, 0x80),
            muted: rgb(0xE5, 0xD8, 0xC0),
            success: rgb(0x4F, 0x7A, 0x3E),
            warning: rgb(0xC8, 0x8A, 0x2C),
            error: rgb(0xB3, 0x35, 0x25),
        },
        Theme::Stone => Palette {
            paper: rgb(0xE6, 0xE8, 0xEA),
            ink: rgb(0x2D, 0x33, 0x38),
            accent: rgb(0x4A, 0x6B, 0x8A),
            ink_soft: rgb(0x5F, 0x69, 0x72),
            ink_faint: rgb(0x9B, 0xA3, 0xAB),
            muted: rgb(0xD3, 0xD7, 0xDB),
            success: rgb(0x3F, 0x7A, 0x5C),
            warning: rgb(0xB8, 0x82, 0x35),
            error: rgb(0xB3, 0x3F, 0x3F),
        },
        Theme::Sage => Palette {
            paper: rgb(0xDD, 0xE4, 0xD2),
            ink: rgb(0x2C, 0x35, 0x26),
            accent: rgb(0x3F, 0x5C, 0x32),
            ink_soft: rgb(0x5E, 0x69, 0x54),
            ink_faint: rgb(0x97, 0xA2, 0x87),
            muted: rgb(0xCC, 0xD4, 0xBE),
            success: rgb(0x3F, 0x6E, 0x35),
            warning: rgb(0xB0, 0x82, 0x2C),
            error: rgb(0xA8, 0x3A, 0x2A),
        },
        Theme::Clay => Palette {
            paper: rgb(0xE8, 0xD4, 0xC2),
            ink: rgb(0x3A, 0x28, 0x20),
            accent: rgb(0x9E, 0x45, 0x21),
            ink_soft: rgb(0x6E, 0x55, 0x48),
            ink_faint: rgb(0xA8, 0x90, 0x7F),
            muted: rgb(0xD9, 0xC0, 0xA8),
            success: rgb(0x5C, 0x7A, 0x3E),
            warning: rgb(0xC0, 0x82, 0x2A),
            error: rgb(0xA8, 0x35, 0x20),
        },
        Theme::Ink => Palette {
            paper: rgb(0x00, 0x00, 0x00),
            ink: rgb(0xE4, 0xE1, 0xD8),
            accent: rgb(0xF5, 0xEF, 0xE0),
            ink_soft: rgb(0x9A, 0x96, 0x90),
            ink_faint: rgb(0x5A, 0x57, 0x52),
            muted: rgb(0x15, 0x15, 0x15),
            success: rgb(0x8F, 0xC0, 0x7A),
            warning: rgb(0xE6, 0xB8, 0x55),
            error: rgb(0xE6, 0x6A, 0x55),
        },
    }
}

pub fn tokens() -> Tokens {
    Tokens {
        space_xs: 4.0,
        space_sm: 8.0,
        space_md: 14.0,
        space_lg: 22.0,
        space_xl: 32.0,
        radius_sm: 4.0,
        radius_md: 8.0,
        font_body: 14.0,
        font_section_title: 13.0,
        font_page_title: 22.0,
        field_label_width: 160.0,
    }
}

/// Whether the supplied theme uses a dark base palette. Callers building a
/// custom palette with overrides still want the right egui base Visuals.
pub fn is_dark(theme: Theme) -> bool {
    matches!(theme, Theme::Ink)
}

pub fn apply(ctx: &Context, palette: &Palette, dark: bool) {
    let p = palette;
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.window_fill = p.paper;
    v.panel_fill = p.paper;
    v.extreme_bg_color = p.paper;
    v.override_text_color = Some(p.ink);

    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;

    let ink_stroke = Stroke::new(1.0, p.ink);
    v.widgets.noninteractive.fg_stroke = ink_stroke;
    v.widgets.inactive.fg_stroke = ink_stroke;
    v.widgets.active.fg_stroke = ink_stroke;
    v.widgets.hovered.fg_stroke = ink_stroke;

    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.ink_faint);

    v.widgets.inactive.bg_fill = p.muted;
    v.widgets.inactive.weak_bg_fill = p.muted;
    v.widgets.hovered.bg_fill = p.muted;
    v.widgets.hovered.weak_bg_fill = p.muted;

    ctx.set_visuals(v);
}

/// `#RRGGBB` (or `#RRGGBBAA`) hex string that deserialises into a `Color32`.
/// Failing parses surface a useful TOML error instead of silently falling
/// back to a default colour.
#[derive(Debug, Clone, Copy)]
pub struct HexColor(pub Color32);

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = HexColor;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a hex colour string like \"#RRGGBB\" or \"#RRGGBBAA\"")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<HexColor, E> {
                parse_hex(s).map(HexColor).map_err(de::Error::custom)
            }
        }
        d.deserialize_str(V)
    }
}

fn parse_hex(s: &str) -> Result<Color32, String> {
    let trimmed = s.strip_prefix('#').unwrap_or(s);
    let bytes = match trimmed.len() {
        6 | 8 => trimmed,
        _ => return Err(format!("expected 6 or 8 hex digits, got {:?}", s)),
    };
    let parse = |i: usize| -> Result<u8, String> {
        u8::from_str_radix(&bytes[i..i + 2], 16)
            .map_err(|e| format!("invalid hex byte at offset {}: {}", i, e))
    };
    let r = parse(0)?;
    let g = parse(2)?;
    let b = parse(4)?;
    let a = if bytes.len() == 8 { parse(6)? } else { 255 };
    Ok(Color32::from_rgba_unmultiplied(r, g, b, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rrggbb() {
        let c = parse_hex("#1e1e2e").expect("valid");
        assert_eq!(c.r(), 0x1e);
        assert_eq!(c.g(), 0x1e);
        assert_eq!(c.b(), 0x2e);
        assert_eq!(c.a(), 0xff);
    }

    #[test]
    fn parses_rrggbbaa() {
        let c = parse_hex("F4EBD980").expect("valid");
        assert_eq!(c.a(), 0x80);
    }

    #[test]
    fn rejects_short_string() {
        assert!(parse_hex("#abc").is_err());
    }
}
