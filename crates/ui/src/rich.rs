use crate::text::Span;

pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn is_icon(c: char) -> bool {
    (0xe000..=0xffff).contains(&(c as u32))
}

pub fn chip_spans(label: &str, family: &str, font_size: f32, icon_size: f32) -> Vec<Span> {
    let text = collapse_whitespace(label);
    let mut spans: Vec<Span> = Vec::new();
    for c in text.chars() {
        let size = if is_icon(c) { icon_size } else { font_size };
        match spans.last_mut() {
            Some(span) if span.size == size => span.text.push(c),
            _ => spans.push(Span::new(c.to_string(), family, size)),
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_collapses_like_html() {
        assert_eq!(
            collapse_whitespace(" Sat, 31 Oct  ·  14:41 "),
            "Sat, 31 Oct · 14:41"
        );
    }

    #[test]
    fn bmp_icons_get_icon_size() {
        let spans = chip_spans("\u{f240} 100%", "Mono", 11.0, 13.0);
        let sizes: Vec<(String, f32)> = spans.into_iter().map(|s| (s.text, s.size)).collect();
        assert_eq!(
            sizes,
            vec![("\u{f240}".to_string(), 13.0), (" 100%".to_string(), 11.0)]
        );
    }

    #[test]
    fn supplementary_icons_keep_font_size_like_qt_utf16() {
        let spans = chip_spans("\u{f0079} 100%", "Mono", 11.0, 13.0);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].size, 11.0);
    }

    #[test]
    fn plain_label_single_span() {
        let spans = chip_spans("01", "Mono", 11.0, 13.0);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].size, 11.0);
    }
}
