use ui::Color;

pub const UI_SCALE: f32 = 1.22;
pub const BG: Color = Color::rgb(0x282828);
pub const DIM: Color = Color::rgb(0x1d2021);
pub const SURFACE: Color = Color::rgb(0x32302f);
pub const ELEVATED: Color = Color::rgb(0x3c3836);
pub const LINE: Color = Color::rgb(0x504945);
pub const TEXT: Color = Color::rgb(0xd4be98);
pub const MUTED: Color = Color::rgb(0x928374);
pub const GREEN: Color = Color::rgb(0xa9b665);
pub const YELLOW: Color = Color::rgb(0xd8a657);
pub const RED: Color = Color::rgb(0xea6962);
pub const BLUE: Color = Color::rgb(0x7daea3);
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
