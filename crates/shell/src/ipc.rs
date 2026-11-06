use std::{
    io::{self, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Duration,
};

pub fn socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    runtime.join(format!("{}.sock", crate::APP_NAME))
}

pub fn encode(args: &[String]) -> String {
    let mut line = serde_json::to_string(args).unwrap_or_else(|_| "[]".into());
    line.push('\n');
    line
}

pub fn decode(line: &str) -> Option<Vec<String>> {
    serde_json::from_str(line.trim_end()).ok()
}

pub fn send(args: &[String]) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(encode(args).as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

pub fn instance_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

pub fn listen() -> io::Result<UnixListener> {
    let path = socket_path();
    if path.exists() && !instance_running() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

pub const MAX_REQUEST: usize = 64 * 1024;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq)]
pub enum Feed {
    More,
    Request(Vec<String>),
    Invalid,
    TooLong,
}

pub fn feed(buffer: &mut Vec<u8>, chunk: &[u8]) -> Feed {
    let start = buffer.len();
    buffer.extend_from_slice(chunk);
    if let Some(end) = buffer[start..].iter().position(|byte| *byte == b'\n') {
        let line = &buffer[..start + end];
        return match std::str::from_utf8(line).ok().and_then(decode) {
            Some(args) if !args.is_empty() => Feed::Request(args),
            _ => Feed::Invalid,
        };
    }
    if buffer.len() > MAX_REQUEST {
        Feed::TooLong
    } else {
        Feed::More
    }
}

pub enum Reply {
    Ok(String),
    Error(String),
}

pub fn write_reply(mut stream: &UnixStream, reply: Reply) {
    if let Err(err) = stream
        .set_nonblocking(false)
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(1))))
    {
        log::warn!("ipc reply: {err}");
        return;
    }
    let text = match reply {
        Reply::Ok(output) if output.is_empty() => "ok\n".to_string(),
        Reply::Ok(output) => format!("ok\n{output}\n"),
        Reply::Error(message) => format!("error\n{message}\n"),
    };
    if let Err(err) = stream.write_all(text.as_bytes()) {
        log::warn!("ipc reply: {err}");
    }
}

pub fn client(args: &[String]) -> i32 {
    match send(args) {
        Ok(reply) => {
            let (status, output) = reply.split_once('\n').unwrap_or((reply.as_str(), ""));
            let output = output.trim_end_matches('\n');
            if status == "ok" {
                if !output.is_empty() {
                    println!("{output}");
                }
                0
            } else {
                eprintln!("{output}");
                1
            }
        }
        Err(err) => {
            eprintln!("{}: {err}", crate::APP_NAME);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let args = vec![
            "osd".to_string(),
            "layout".to_string(),
            "English (US)".to_string(),
        ];
        assert_eq!(decode(&encode(&args)), Some(args));
    }

    #[test]
    fn requests_arrive_in_pieces() {
        let mut buffer = Vec::new();
        let line = encode(&["wifi".to_string()]);
        let (head, tail) = line.as_bytes().split_at(3);
        assert_eq!(feed(&mut buffer, head), Feed::More);
        assert_eq!(feed(&mut buffer, tail), Feed::Request(vec!["wifi".into()]));
        let mut buffer = Vec::new();
        assert_eq!(
            feed(&mut buffer, b"[\"osd\",\"x\"]\n[\"close\"]\n"),
            Feed::Request(vec!["osd".into(), "x".into()])
        );
    }

    #[test]
    fn bad_requests_are_refused() {
        for line in [
            &b"garbage\n"[..],
            b"[]\n",
            b"{\"a\":1}\n",
            b"[1,2]\n",
            b"\xff\xfe\n",
            b"\n",
        ] {
            assert_eq!(feed(&mut Vec::new(), line), Feed::Invalid, "{line:?}");
        }
        let mut buffer = Vec::new();
        let chunk = vec![b'a'; 4096];
        let mut result = Feed::More;
        for _ in 0..=MAX_REQUEST / chunk.len() {
            result = feed(&mut buffer, &chunk);
        }
        assert_eq!(result, Feed::TooLong);
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn requests_survive_any_split(args in prop::collection::vec("\\PC{0,24}", 1..6), cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..6)) {
            let line = encode(&args);
            let bytes = line.as_bytes();
            let mut points: Vec<usize> = cuts.iter().map(|cut| cut.index(bytes.len())).collect();
            points.sort();
            points.dedup();
            let mut buffer = Vec::new();
            let mut start = 0;
            let mut result = Feed::More;
            for point in points.into_iter().chain([bytes.len()]) {
                prop_assert_eq!(&result, &Feed::More);
                result = feed(&mut buffer, &bytes[start..point]);
                start = point;
            }
            prop_assert_eq!(result, Feed::Request(args));
        }

        #[test]
        fn garbage_never_panics(chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..512), 0..8)) {
            let mut buffer = Vec::new();
            for chunk in &chunks {
                if feed(&mut buffer, chunk) != Feed::More {
                    break;
                }
            }
            prop_assert!(buffer.len() <= MAX_REQUEST + 512);
        }
    }
}
