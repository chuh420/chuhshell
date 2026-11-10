use std::cell::Cell;

use ui::Color;

pub const FAST: f64 = 140.0;
pub const NORMAL: f64 = 220.0;
pub const SLOW: f64 = 340.0;

const EXTRA_FAMILIES: &[&str] = &[
    "Noto Sans",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans CJK JP",
    "Noto Color Emoji",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fonts {
    pub ui: &'static str,
    pub icons: &'static str,
    pub icon_fallback: &'static str,
}

thread_local! {
    static FONTS: Cell<Fonts> = const {
        Cell::new(Fonts {
            ui: "Adwaita Sans",
            icons: "JetBrainsMono Nerd Font Mono",
            icon_fallback: "JetBrainsMonoNL Nerd Font",
        })
    };
}

fn intern(value: &str, current: &'static str) -> &'static str {
    if value == current {
        current
    } else {
        Box::leak(value.to_owned().into_boxed_str())
    }
}

pub fn scale() -> f32 {
    ui::widgets::ui_scale()
}

pub fn fonts() -> Fonts {
    FONTS.with(Cell::get)
}

pub fn font() -> &'static str {
    fonts().ui
}

pub fn icons() -> &'static str {
    fonts().icons
}

pub fn icon_fallback() -> &'static str {
    fonts().icon_fallback
}

pub fn font_families() -> Vec<String> {
    let fonts = fonts();
    let mut families = vec![
        fonts.ui.to_string(),
        fonts.icons.to_string(),
        format!("{}:weight=80:slant=0", fonts.icon_fallback),
    ];
    families.extend(EXTRA_FAMILIES.iter().map(|family| family.to_string()));
    families
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub scale: bool,
    pub fonts: bool,
}

pub fn apply_config(config: &config::Config) -> Changes {
    let scale = config.general.scale;
    let scale_changed = (ui::widgets::ui_scale() - scale).abs() > f32::EPSILON;
    if scale_changed {
        ui::widgets::set_ui_scale(scale);
    }
    let current = fonts();
    let next = Fonts {
        ui: intern(&config.fonts.ui, current.ui),
        icons: intern(&config.fonts.icons, current.icons),
        icon_fallback: intern(&config.fonts.icon_fallback, current.icon_fallback),
    };
    let fonts_changed = next != current;
    if fonts_changed {
        FONTS.with(|cell| cell.set(next));
    }
    Changes {
        scale: scale_changed,
        fonts: fonts_changed,
    }
}

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

pub const CATPPUCCIN_MOCHA: Palette = Palette {
    bg: Color::rgb(0x1e1e2e),
    dim: Color::rgb(0x181825),
    surface: Color::rgb(0x313244),
    elevated: Color::rgb(0x45475a),
    line: Color::rgb(0x585b70),
    text: Color::rgb(0xcdd6f4),
    muted: Color::rgb(0x7f849c),
    green: Color::rgb(0xa6e3a1),
    yellow: Color::rgb(0xf9e2af),
    red: Color::rgb(0xf38ba8),
    blue: Color::rgb(0x89b4fa),
};

pub const NORD: Palette = Palette {
    bg: Color::rgb(0x2e3440),
    dim: Color::rgb(0x242933),
    surface: Color::rgb(0x3b4252),
    elevated: Color::rgb(0x434c5e),
    line: Color::rgb(0x4c566a),
    text: Color::rgb(0xd8dee9),
    muted: Color::rgb(0x7b88a1),
    green: Color::rgb(0xa3be8c),
    yellow: Color::rgb(0xebcb8b),
    red: Color::rgb(0xbf616a),
    blue: Color::rgb(0x88c0d0),
};

pub const TOKYO_NIGHT: Palette = Palette {
    bg: Color::rgb(0x1a1b26),
    dim: Color::rgb(0x16161e),
    surface: Color::rgb(0x24283b),
    elevated: Color::rgb(0x292e42),
    line: Color::rgb(0x3b4261),
    text: Color::rgb(0xc0caf5),
    muted: Color::rgb(0x565f89),
    green: Color::rgb(0x9ece6a),
    yellow: Color::rgb(0xe0af68),
    red: Color::rgb(0xf7768e),
    blue: Color::rgb(0x7aa2f7),
};

pub const ROSE_PINE: Palette = Palette {
    bg: Color::rgb(0x191724),
    dim: Color::rgb(0x13111e),
    surface: Color::rgb(0x1f1d2e),
    elevated: Color::rgb(0x26233a),
    line: Color::rgb(0x403d52),
    text: Color::rgb(0xe0def4),
    muted: Color::rgb(0x6e6a86),
    green: Color::rgb(0x9ccfd8),
    yellow: Color::rgb(0xf6c177),
    red: Color::rgb(0xeb6f92),
    blue: Color::rgb(0xc4a7e7),
};

pub const EVERFOREST: Palette = Palette {
    bg: Color::rgb(0x2d353b),
    dim: Color::rgb(0x232a2e),
    surface: Color::rgb(0x343f44),
    elevated: Color::rgb(0x3d484d),
    line: Color::rgb(0x475258),
    text: Color::rgb(0xd3c6aa),
    muted: Color::rgb(0x859289),
    green: Color::rgb(0xa7c080),
    yellow: Color::rgb(0xdbbc7f),
    red: Color::rgb(0xe67e80),
    blue: Color::rgb(0x7fbbb3),
};

pub const BUILTIN: &[(&str, Palette)] = &[
    ("gruvbox", GRUVBOX),
    ("catppuccin-mocha", CATPPUCCIN_MOCHA),
    ("nord", NORD),
    ("tokyo-night", TOKYO_NIGHT),
    ("rose-pine", ROSE_PINE),
    ("everforest", EVERFOREST),
];

pub fn builtin(name: &str) -> Option<Palette> {
    BUILTIN
        .iter()
        .find(|(builtin, _)| *builtin == name)
        .map(|(_, palette)| *palette)
}

pub fn hex(color: Color) -> String {
    let [r, g, b, _] = color.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

impl Palette {
    pub fn with_overrides(mut self, colors: &config::Colors) -> Palette {
        for (key, value) in colors.entries() {
            let Some(color) = value.and_then(config::parse_hex).map(Color::rgb) else {
                continue;
            };
            let slot = match key {
                "bg" => &mut self.bg,
                "dim" => &mut self.dim,
                "surface" => &mut self.surface,
                "elevated" => &mut self.elevated,
                "line" => &mut self.line,
                "text" => &mut self.text,
                "muted" => &mut self.muted,
                "green" => &mut self.green,
                "yellow" => &mut self.yellow,
                "red" => &mut self.red,
                "blue" => &mut self.blue,
                _ => continue,
            };
            *slot = color;
        }
        self
    }

    pub fn configured(theme: &config::Theme) -> Palette {
        builtin(&theme.name)
            .unwrap_or(GRUVBOX)
            .with_overrides(&theme.colors)
    }
}

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
    fn builtin_palettes_and_overrides() {
        assert_eq!(builtin("gruvbox"), Some(GRUVBOX));
        assert_eq!(builtin("nord").map(|p| hex(p.bg)), Some("#2e3440".into()));
        assert_eq!(builtin("solarized"), None);
        assert_eq!(BUILTIN.len(), 6);
        for (name, palette) in BUILTIN {
            assert_ne!(palette.bg, palette.text, "{name}");
        }
        let mut theme = config::Theme::default();
        assert_eq!(Palette::configured(&theme), GRUVBOX);
        theme.name = "tokyo-night".into();
        theme.colors.green = Some("#123456".into());
        theme.colors.red = Some("broken".into());
        let palette = Palette::configured(&theme);
        assert_eq!(hex(palette.green), "#123456");
        assert_eq!(palette.red, TOKYO_NIGHT.red);
        assert_eq!(palette.bg, TOKYO_NIGHT.bg);
        theme.name = "unknown".into();
        assert_eq!(Palette::configured(&theme).bg, GRUVBOX.bg);
    }

    #[test]
    fn config_sets_scale_and_fonts() {
        let mut config = config::Config::default();
        assert_eq!(apply_config(&config), Changes::default());
        assert_eq!(scale(), 1.22);
        config.general.scale = 1.5;
        config.fonts.ui = "Inter".into();
        let changes = apply_config(&config);
        assert!(changes.scale && changes.fonts);
        assert_eq!(scale(), 1.5);
        assert_eq!(font(), "Inter");
        assert_eq!(font_families()[0], "Inter");
        assert_eq!(apply_config(&config), Changes::default());
        apply_config(&config::Config::default());
        assert_eq!(font(), "Adwaita Sans");
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
