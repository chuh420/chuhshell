#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn socket() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR");
    PathBuf::from(runtime).join("chuhshell.sock")
}

fn connect() -> UnixStream {
    let stream = UnixStream::connect(socket()).expect("a running chuhshell");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
}

fn exchange(payload: &[u8]) -> String {
    let mut stream = connect();
    stream.write_all(payload).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    reply
}

fn timed_inspect() -> Duration {
    let started = Instant::now();
    let reply = exchange(b"[\"inspect\"]\n");
    assert!(reply.starts_with("ok\n{"), "inspect failed: {reply}");
    started.elapsed()
}

#[test]
#[ignore = "talks to the running shell"]
fn ipc_keeps_answering_under_abuse() {
    let baseline = timed_inspect();
    eprintln!("inspect: {baseline:?}");

    let idle: Vec<UnixStream> = (0..20).map(|_| connect()).collect();
    let busy = timed_inspect();
    assert!(
        busy < Duration::from_millis(300),
        "20 silent clients slowed a request to {busy:?}"
    );
    drop(idle);

    let trickle = thread::spawn(|| {
        let mut stream = connect();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(4) {
            if stream.write_all(b"[").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let mut rest = String::new();
        let _ = stream.read_to_string(&mut rest);
        started.elapsed()
    });
    thread::sleep(Duration::from_millis(300));
    for _ in 0..5 {
        let during = timed_inspect();
        assert!(
            during < Duration::from_millis(300),
            "a trickling client slowed a request to {during:?}"
        );
    }
    let dropped = trickle.join().unwrap();
    assert!(dropped < Duration::from_secs(4) + Duration::from_millis(500));

    let mut huge = vec![b'a'; 1 << 20];
    huge.push(b'\n');
    let mut stream = connect();
    let _ = stream.write_all(&huge);
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    assert_eq!(reply, "error\nrequest too long\n");

    for garbage in [
        &b"garbage\n"[..],
        b"[]\n",
        b"{\"a\":1}\n",
        b"[1,2]\n",
        b"\xff\xfe\n",
        b"\n",
    ] {
        assert_eq!(exchange(garbage), "error\nbad request\n", "{garbage:?}");
    }
    let unknown = exchange(b"[\"no-such-command\",\"x\"]\n");
    assert!(unknown.starts_with("error\n"), "{unknown}");

    let workers: Vec<_> = (0..50)
        .map(|_| thread::spawn(|| exchange(b"[\"inspect\"]\n")))
        .collect();
    for worker in workers {
        assert!(worker.join().unwrap().starts_with("ok\n"));
    }

    for _ in 0..200 {
        drop(connect());
    }
    let after = timed_inspect();
    eprintln!("inspect after abuse: {after:?}");
    assert!(after < Duration::from_millis(300));
}
