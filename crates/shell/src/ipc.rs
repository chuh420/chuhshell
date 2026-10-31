use std::{
    io::{self, BufRead, BufReader, Read, Write},
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

pub fn read_request(stream: &UnixStream) -> io::Result<Vec<String>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "probe"));
    }
    decode(&line).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad request"))
}

pub enum Reply {
    Ok(String),
    Error(String),
}

pub fn write_reply(mut stream: &UnixStream, reply: Reply) {
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
}
