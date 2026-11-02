use std::cell::Cell;

use ui::Color;

pub const UI_SCALE: f32 = 1.22;
pub const FONT: &str = "Adwaita Sans";
pub const ICONS: &str = "JetBrainsMono Nerd Font Mono";
pub const ICON_FALLBACK: &str = "JetBrainsMonoNL Nerd Font";
pub const FAST: f64 = 140.0;
pub const NORMAL: f64 = 220.0;
pub const SLOW: f64 = 340.0;

pub const FONT_FAMILIES: &[&str] = &[
    FONT,
    ICONS,
    "JetBrainsMonoNL Nerd Font:weight=80:slant=0",
    "Noto Sans",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans CJK JP",
    "Noto Color Emoji",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub dim: Color,
    pub surface: Color,
    pub elevated: Color,
    pub line: Color,
    pub text: Color,
    pub muted: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub blue: Color,
}

pub const GRUVBOX: Palette = Palette {
    bg: Color::rgb(0x282828),
    dim: Color::rgb(0x1d2021),
    surface: Color::rgb(0x32302f),
    elevated: Color::rgb(0x3c3836),
    line: Color::rgb(0x504945),
    text: Color::rgb(0xd4be98),
    muted: Color::rgb(0x928374),
    green: Color::rgb(0xa9b665),
    yellow: Color::rgb(0xd8a657),
    red: Color::rgb(0xea6962),
    blue: Color::rgb(0x7daea3),
};

thread_local! {
    static CURRENT: Cell<Palette> = const { Cell::new(GRUVBOX) };
}

pub fn palette() -> Palette {
    CURRENT.with(Cell::get)
}

pub fn set_palette(palette: Palette) -> bool {
    CURRENT.with(|current| current.replace(palette) != palette)
}

fn parse_hex(value: &str) -> Option<Color> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(Color::rgb)
}

impl Palette {
    pub fn from_json(text: &str) -> Option<Palette> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let color = |key: &str| value.get(key)?.as_str().and_then(parse_hex);
        Some(Palette {
            bg: color("bg")?,
            dim: color("dim")?,
            surface: color("surface")?,
            elevated: color("elevated")?,
            line: color("line")?,
            text: color("text")?,
            muted: color("muted")?,
            green: color("green")?,
            yellow: color("yellow")?,
            red: color("red")?,
            blue: color("blue")?,
        })
    }
}

pub fn bg() -> Color {
    palette().bg
}

pub fn dim() -> Color {
    palette().dim
}

pub fn surface() -> Color {
    palette().surface
}

pub fn elevated() -> Color {
    palette().elevated
}

pub fn line() -> Color {
    palette().line
}

pub fn text() -> Color {
    palette().text
}

pub fn muted() -> Color {
    palette().muted
}

pub fn green() -> Color {
    palette().green
}

pub fn yellow() -> Color {
    palette().yellow
}

pub fn red() -> Color {
    palette().red
}

pub fn blue() -> Color {
    palette().blue
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r##"{"bg": "#282828", "dim": "#1d2021", "surface": "#32302f", "elevated": "#3c3836", "line": "#504945", "text": "#d4be98", "muted": "#928374", "green": "#a9b665", "yellow": "#d8a657", "red": "#ea6962", "blue": "#7daea3"}"##;

    #[test]
    fn gruvbox_snapshot_matches() {
        assert_eq!(Palette::from_json(SNAPSHOT), Some(GRUVBOX));
    }

    #[test]
    fn matugen_output_parses() {
        let text = r##"{"bg": "#1a1c15", "dim": "#12140d", "surface": "#1e2019", "elevated": "#292b23", "line": "#45483c", "text": "#e3e3d7", "muted": "#909284", "green": "#b8d084", "yellow": "#a0d0c8", "red": "#ffb4ab", "blue": "#c2caaa"}"##;
        let palette = Palette::from_json(text);
        assert_eq!(
            palette.map(|p| p.green.to_rgba8()),
            Some([0xb8, 0xd0, 0x84, 0xff])
        );
    }

    #[test]
    fn broken_input_is_rejected() {
        assert_eq!(Palette::from_json("{"), None);
        assert_eq!(Palette::from_json(r##"{"bg": "#282828"}"##), None);
        assert_eq!(
            Palette::from_json(&SNAPSHOT.replace("#1d2021", "1d2021")),
            None
        );
        assert_eq!(
            Palette::from_json(&SNAPSHOT.replace("#1d2021", "#zzzzzz")),
            None
        );
    }

    #[test]
    fn set_palette_reports_change() {
        assert!(!set_palette(GRUVBOX));
        let mut other = GRUVBOX;
        other.green = Color::rgb(0x123456);
        assert!(set_palette(other));
        assert_eq!(green(), other.green);
        assert!(set_palette(GRUVBOX));
    }
}
