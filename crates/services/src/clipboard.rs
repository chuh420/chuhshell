use std::{
    io::{self, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipEntry {
    pub id: String,
    pub kind: Kind,
    pub text: String,
    pub size: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
}

impl ClipEntry {
    pub fn is_image(&self) -> bool {
        self.kind == Kind::Image
    }
}

pub fn available() -> bool {
    crate::programs::exists("cliphist") && crate::programs::exists("wl-copy")
}

pub fn cache_dir() -> PathBuf {
    config::paths().cache_dir("clipboard")
}

fn python_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let breaks = matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !breaks {
            continue;
        }
        lines.push(&text[start..index]);
        let mut end = index + c.len_utf8();
        if c == '\r'
            && let Some((next, '\n')) = chars.peek().copied()
        {
            chars.next();
            end = next + 1;
        }
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

fn parse_binary(preview: &str) -> Option<(String, String, u32, u32)> {
    let inner = preview
        .strip_prefix("[[ binary data ")?
        .strip_suffix(" ]]")?;
    let (rest, dims) = inner.rsplit_once(' ')?;
    let (size, format) = rest.rsplit_once(' ')?;
    let (width, height) = dims.split_once('x')?;
    if size.is_empty() || !is_word(format) || !is_number(width) || !is_number(height) {
        return None;
    }
    Some((
        size.to_string(),
        format.to_lowercase(),
        width.parse().ok()?,
        height.parse().ok()?,
    ))
}

pub fn parse_list(output: &str) -> Vec<ClipEntry> {
    let mut result = Vec::new();
    for line in python_lines(output).into_iter().take(400) {
        let (identifier, preview) = line.split_once('\t').unwrap_or((line, ""));
        if !is_number(identifier) {
            continue;
        }
        let entry = match parse_binary(preview) {
            Some((size, format, width, height)) => ClipEntry {
                id: identifier.to_string(),
                kind: Kind::Image,
                text: String::new(),
                size,
                format,
                width,
                height,
            },
            None => ClipEntry {
                id: identifier.to_string(),
                kind: Kind::Text,
                text: preview.to_string(),
                size: String::new(),
                format: String::new(),
                width: 0,
                height: 0,
            },
        };
        result.push(entry);
    }
    result
}

pub fn list() -> Vec<ClipEntry> {
    match Command::new("cliphist")
        .arg("list")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => parse_list(&String::from_utf8_lossy(&output.stdout)),
        Err(err) => {
            log::warn!("cliphist list: {err}");
            Vec::new()
        }
    }
}

fn run_with_input(program: &str, args: &[&str], input: &[u8]) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input)?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} exited with {status}")))
    }
}

pub fn decode(id: &str) -> io::Result<Vec<u8>> {
    let output = Command::new("cliphist")
        .args(["decode", id])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(io::Error::other(format!(
            "cliphist decode exited with {}",
            output.status
        )))
    }
}

pub fn copy(id: &str) -> io::Result<()> {
    let data = decode(id)?;
    run_with_input("wl-copy", &[], &data)
}

pub fn thumbnail(id: &str, extension: &str) -> io::Result<PathBuf> {
    if !is_number(id) || extension.is_empty() || !extension.chars().all(char::is_alphanumeric) {
        return Err(io::Error::other("Invalid clipboard entry"));
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{id}.{extension}"));
    if !target.is_file() {
        let temporary = dir.join(format!("{id}.tmp"));
        std::fs::write(&temporary, decode(id)?)?;
        std::fs::rename(&temporary, &target)?;
    }
    Ok(target)
}

fn remove_cached(matches: impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(cache_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if matches(&name)
            && let Err(err) = std::fs::remove_file(entry.path())
        {
            log::warn!("clipboard cache {name}: {err}");
        }
    }
}

pub fn delete(id: &str) -> io::Result<()> {
    run_with_input("cliphist", &["delete"], format!("{id}\t\n").as_bytes())?;
    let prefix = format!("{id}.");
    remove_cached(|name| name.starts_with(&prefix));
    Ok(())
}

pub fn wipe() -> io::Result<()> {
    let status = Command::new("cliphist")
        .arg("wipe")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "cliphist wipe exited with {status}"
        )));
    }
    remove_cached(|_| true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_images() {
        let output = "12\thello  world\n11\t[[ binary data 12 KiB png 640x480 ]]\nbad\tline\n10\t[[ binary data x ]]\n";
        let entries = parse_list(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, Kind::Text);
        assert_eq!(entries[0].text, "hello  world");
        assert_eq!(entries[1].kind, Kind::Image);
        assert_eq!(entries[1].size, "12 KiB");
        assert_eq!(entries[1].format, "png");
        assert_eq!((entries[1].width, entries[1].height), (640, 480));
        assert_eq!(entries[2].kind, Kind::Text);
    }

    #[test]
    fn caps_at_400_entries() {
        let output: String = (0..500).map(|i| format!("{i}\tx\n")).collect();
        assert_eq!(parse_list(&output).len(), 400);
    }

    #[test]
    fn splits_like_python_splitlines() {
        assert_eq!(python_lines("a\r\nb\u{2028}c\n"), vec!["a", "b", "c"]);
    }

    #[test]
    fn rejects_bad_thumbnail_arguments() {
        assert!(thumbnail("1a", "png").is_err());
        assert!(thumbnail("1", "p/g").is_err());
    }
}
