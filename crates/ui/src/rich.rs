use crate::{
    Color,
    text::{Span, Weight},
};

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

#[derive(Clone, Default)]
struct Style {
    bold: usize,
    italic: usize,
    underline: usize,
    links: Vec<Option<String>>,
}

fn entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let start = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[start..];
    let value = match rest.chars().next()? {
        quote @ ('"' | '\'') => rest[1..].split(quote).next()?,
        _ => rest.split(|c: char| c.is_whitespace() || c == '>').next()?,
    };
    Some(value.to_string())
}

pub fn styled_spans(text: &str, family: &str, size: f32, link_color: Color) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Style::default();
    let mut pending_space = false;
    let push = |spans: &mut Vec<Span>, style: &Style, piece: &str| {
        let link = style.links.iter().rev().find_map(Clone::clone);
        let bold = style.bold > 0;
        let italic = style.italic > 0;
        let underline = style.underline > 0 || link.is_some();
        if let Some(last) = spans.last_mut()
            && (last.weight == Weight::Bold) == bold
            && last.italic == italic
            && last.underline == underline
            && last.link == link
        {
            last.text.push_str(piece);
            return;
        }
        let mut span = Span::new(piece, family, size);
        if bold {
            span.weight = Weight::Bold;
        }
        span.italic = italic;
        span.underline = underline;
        if link.is_some() {
            span.color = Some(link_color);
        }
        span.link = link;
        spans.push(span);
    };
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
        {
            let tag = rest[1..end].trim();
            rest = &rest[end + 1..];
            let at_line_start = spans.last().is_none_or(|span| span.text.ends_with('\n'));
            if pending_space && !at_line_start {
                push(&mut spans, &style, " ");
            }
            pending_space = false;
            let closing = tag.starts_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            let step = |count: &mut usize| {
                if closing {
                    *count = count.saturating_sub(1);
                } else {
                    *count += 1;
                }
            };
            match name.as_str() {
                "b" | "strong" => step(&mut style.bold),
                "i" | "em" => step(&mut style.italic),
                "u" => step(&mut style.underline),
                "a" => {
                    if closing {
                        style.links.pop();
                    } else {
                        style.links.push(attribute(tag, "href"));
                    }
                }
                "br" => {
                    push(&mut spans, &style, "\n");
                    pending_space = false;
                }
                "p" if closing => {
                    push(&mut spans, &style, "\n");
                    pending_space = false;
                }
                _ => {}
            }
            continue;
        }
        rest = &rest[c.len_utf8()..];
        let ch = if c == '&' {
            match rest.find(';') {
                Some(end) if end <= 10 => match entity(&rest[..end]) {
                    Some(decoded) => {
                        rest = &rest[end + 1..];
                        decoded
                    }
                    None => c,
                },
                _ => c,
            }
        } else {
            c
        };
        if ch == '\n' {
            push(&mut spans, &style, "\n");
            pending_space = false;
            continue;
        }
        if ch.is_whitespace() && ch != '\u{a0}' {
            pending_space = true;
            continue;
        }
        let mut piece = String::new();
        let at_line_start = spans.last().is_none_or(|span| span.text.ends_with('\n'));
        if pending_space && !at_line_start {
            piece.push(' ');
        }
        pending_space = false;
        piece.push(ch);
        push(&mut spans, &style, &piece);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_text_tags_and_entities() {
        let spans = styled_spans(
            "a <b>bold</b> &amp; <a href=\"https://x.y\">link</a><br>next",
            "Sans",
            11.0,
            Color::rgb(0x00ff00),
        );
        let text: String = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(text, "a bold & link\nnext");
        assert!(
            spans
                .iter()
                .any(|span| span.weight == Weight::Bold && span.text == "bold")
        );
        assert!(
            spans
                .iter()
                .any(|span| span.link.as_deref() == Some("https://x.y") && span.underline)
        );
    }

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
