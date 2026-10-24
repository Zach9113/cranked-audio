//! Window themes: every colour role in the `Palette` global, per theme. "system" reads the KDE colour scheme.

use slint::Color;

/// Theme names in the order the Settings page lists them.
pub const NAMES: [(&str, &str); 5] = [
    ("cranked", "Cranked (dark)"),
    ("midnight", "Midnight (blue-black)"),
    ("graphite", "Graphite (grey)"),
    ("light", "Light"),
    ("system", "Match my desktop colours"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub card: Color,
    pub centre: Color,
    pub centre_hover: Color,
    pub track: Color,
    pub button: Color,
    pub button_hover: Color,
    pub edge: Color,
    pub text: Color,
    pub text_strong: Color,
    pub text_dim: Color,
    pub subtle: Color,
    pub faint: Color,
    pub hint: Color,
    pub list: Color,
    pub selected: Color,
    pub selected_hover: Color,
    pub check: Color,
    pub ok: Color,
}

fn hex(value: u32) -> Color {
    Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// `amount` of `a` over `b`.
pub fn mix(a: Color, b: Color, amount: f32) -> Color {
    let m = |x: u8, y: u8| (f32::from(x) * amount + f32::from(y) * (1.0 - amount)).round() as u8;
    Color::from_rgb_u8(m(a.red(), b.red()), m(a.green(), b.green()), m(a.blue(), b.blue()))
}

/// A whole theme from a few base colours: background, card, text and accent; the rest is mixed from them.
pub fn from_base(background: Color, card: Color, text: Color, accent: Color) -> Theme {
    let dark = u32::from(background.red()) + u32::from(background.green()) + u32::from(background.blue()) < 384;
    let lift = |c: Color, by: f32| if dark { mix(text, c, by) } else { mix(Color::from_rgb_u8(0, 0, 0), c, by) };
    Theme {
        background,
        card,
        centre: lift(card, 0.06),
        centre_hover: lift(card, 0.09),
        track: lift(card, 0.12),
        button: lift(card, 0.04),
        button_hover: lift(card, 0.08),
        edge: lift(card, 0.14),
        text,
        text_strong: if dark { Color::from_rgb_u8(255, 255, 255) } else { Color::from_rgb_u8(0, 0, 0) },
        text_dim: mix(text, card, 0.8),
        subtle: mix(text, card, 0.66),
        faint: mix(text, card, 0.52),
        hint: mix(text, card, 0.4),
        list: if dark { mix(Color::from_rgb_u8(0, 0, 0), background, 0.15) } else { lift(background, 0.02) },
        selected: mix(accent, card, 0.25),
        selected_hover: mix(accent, card, 0.33),
        check: accent,
        ok: accent,
    }
}

pub fn named(name: &str) -> Theme {
    match name {
        "midnight" => from_base(hex(0x0b0f1a), hex(0x131a2b), hex(0xe6ecf8), hex(0x5aa9ff)),
        "graphite" => from_base(hex(0x1e1f22), hex(0x2a2b2f), hex(0xececee), hex(0x9aa4b2)),
        "light" => from_base(hex(0xeef0f3), hex(0xffffff), hex(0x1d1f24), hex(0x1f9d55)),
        "system" => system().unwrap_or_else(|| named("cranked")),
        // "cranked": the original look, exactly
        _ => Theme {
            background: hex(0x121216),
            card: hex(0x1b1b21),
            centre: hex(0x2a2a32),
            centre_hover: hex(0x2f2f38),
            track: hex(0x34343e),
            button: hex(0x26262e),
            button_hover: hex(0x30303a),
            edge: hex(0x3a3a44),
            text: hex(0xf2f2f5),
            text_strong: hex(0xffffff),
            text_dim: hex(0xc4c4ce),
            subtle: hex(0xa8a8b4),
            faint: hex(0x8e8e9a),
            hint: hex(0x6e6e7a),
            list: hex(0x18181d),
            selected: hex(0x264a32),
            selected_hover: hex(0x2f5a3c),
            check: hex(0x6ee08a),
            ok: hex(0x5cc878),
        },
    }
}

/// `r,g,b` from a kdeglobals line.
fn rgb(text: &str) -> Option<Color> {
    let mut parts = text.trim().split(',').map(|p| p.trim().parse::<u8>().ok());
    Some(Color::from_rgb_u8(parts.next()??, parts.next()??, parts.next()??))
}

/// A value from one `[group]` of kdeglobals.
pub fn kde_value(file: &str, group: &str, key: &str) -> Option<String> {
    let mut in_group = false;
    for line in file.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == format!("[{group}]");
        } else if in_group {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// The desktop's colour scheme (KDE's kdeglobals): window background, view background, text and accent.
fn system() -> Option<Theme> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    let file = std::fs::read_to_string(home.join("kdeglobals")).ok()?;
    from_kdeglobals(&file)
}

pub fn from_kdeglobals(file: &str) -> Option<Theme> {
    let get = |group: &str, key: &str| kde_value(file, group, key).and_then(|v| rgb(&v));
    let background = get("Colors:Window", "BackgroundNormal")?;
    let card = get("Colors:View", "BackgroundNormal").map(|v| mix(v, background, 0.5)).unwrap_or(background);
    let text = get("Colors:Window", "ForegroundNormal")?;
    let accent = get("General", "AccentColor")
        .or_else(|| get("Colors:Selection", "BackgroundNormal"))
        .or_else(|| get("Colors:View", "DecorationFocus"))
        .unwrap_or(Color::from_rgb_u8(0x3d, 0xae, 0xe9));
    Some(from_base(background, card, text, accent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_kde_colour_scheme() {
        let file = "[General]\nAccentColor=0,255,127\n\n[Colors:Window]\nBackgroundNormal=28,28,33\nForegroundNormal=211,218,227\n\n[Colors:View]\nBackgroundNormal=16,16,19\n";
        let theme = from_kdeglobals(file).unwrap();
        assert_eq!(theme.background, Color::from_rgb_u8(28, 28, 33));
        assert_eq!(theme.text, Color::from_rgb_u8(211, 218, 227));
        assert_eq!(theme.check, Color::from_rgb_u8(0, 255, 127));
        assert!(from_kdeglobals("[Colors:Window]\n").is_none());
    }

    #[test]
    fn every_theme_has_readable_text() {
        let brightness = |c: Color| u32::from(c.red()) + u32::from(c.green()) + u32::from(c.blue());
        for (name, _) in NAMES {
            let theme = named(name);
            let gap = brightness(theme.text).abs_diff(brightness(theme.card));
            assert!(gap > 300, "{name}: text and cards too close ({gap})");
        }
    }
}
