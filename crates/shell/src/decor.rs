use ui::{Color, Span, TextSpec};

pub const FAMILY: &str = "chuhshell decor";
pub const FONT: &[u8] = include_bytes!("../assets/decor.otf");

pub const LAUNCH: &str = "起動";
pub const CONTROL: &str = "操作";
pub const CLIPBOARD: &str = "複写";
pub const POWER: &str = "電源";
pub const CALENDAR: &str = "暦";
pub const SOUND: &str = "音";
pub const BATTERY: &str = "電池";
pub const BLUETOOTH: &str = "青歯";
pub const SYSTEM: &str = "機関";
pub const VPN: &str = "隧道";
pub const WIFI: &str = "無線";
pub const NOTIFICATIONS: &str = "通知";
pub const WALLPAPERS: &str = "壁紙";
pub const THEMES: &str = "彩";
pub const QUIET: &str = "静";
pub const SILENT: &str = "黙";
pub const STILL: &str = "静寂";
pub const WEEKDAYS: [&str; 7] = ["月", "火", "水", "木", "金", "土", "日"];

const NUMERALS: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];

pub fn numeral(id: i32) -> Option<&'static str> {
    usize::try_from(id - 1)
        .ok()
        .and_then(|index| NUMERALS.get(index).copied())
}

pub fn span(kanji: &str, size: f32, color: Color) -> Span {
    let mut span = Span::new(kanji, FAMILY, size);
    span.color = Some(color);
    span
}

pub fn separator(size: f32) -> Span {
    let mut span = Span::new("  |  ", crate::theme::font(), size);
    span.color = Some(crate::theme::line());
    span
}

pub fn eyebrow(kanji: &str, text: &str, family: &str) -> TextSpec {
    let mut word = Span::new(text, family, 9.0);
    word.letter_spacing = 1.0;
    word.color = Some(crate::theme::muted());
    TextSpec {
        spans: vec![
            span(kanji, 12.0, crate::theme::green()),
            separator(10.0),
            word,
        ],
        color: crate::theme::green(),
        elide_width: None,
        wrap_width: None,
        max_lines: None,
    }
}

pub fn suffix(spec: &mut TextSpec, kanji: &str) {
    let size = spec.spans.last().map_or(12.0, |span| span.size) * 0.8;
    spec.spans.push(separator(size));
    spec.spans.push(span(kanji, size, crate::theme::muted()));
}

#[cfg(test)]
pub fn all_text() -> String {
    let mut text = [
        LAUNCH,
        CONTROL,
        CLIPBOARD,
        POWER,
        CALENDAR,
        SOUND,
        BATTERY,
        BLUETOOTH,
        SYSTEM,
        VPN,
        WIFI,
        NOTIFICATIONS,
        WALLPAPERS,
        THEMES,
        QUIET,
        SILENT,
        STILL,
    ]
    .concat();
    text.push_str(&WEEKDAYS.concat());
    text.push_str(&NUMERALS.concat());
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numerals_cover_one_to_ten() {
        assert_eq!(numeral(1), Some("一"));
        assert_eq!(numeral(10), Some("十"));
        assert_eq!(numeral(0), None);
        assert_eq!(numeral(11), None);
        assert_eq!(numeral(-3), None);
    }

    #[test]
    fn embedded_font_covers_every_glyph() {
        let face = ttf_parser::Face::parse(FONT, 0).expect("decor font face");
        assert!(
            face.names()
                .into_iter()
                .filter(|name| name.name_id == ttf_parser::name_id::FAMILY)
                .filter_map(|name| name.to_string())
                .any(|name| name == FAMILY)
        );
        let missing: String = all_text()
            .chars()
            .filter(|c| face.glyph_index(*c).is_none())
            .collect();
        assert!(missing.is_empty(), "missing glyphs: {missing}");
    }
}
