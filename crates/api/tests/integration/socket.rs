use std::io::{self, BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use api::{Endpoint, HeadlessHost, Hub, listen};
#[cfg(unix)]
use api::{Inbox, codes};
use serde_json::{Value, json};

/// Mirrors `api::MAX_LINE_BYTES` (16 MiB) so the test checks behavior at the limit.
#[cfg(unix)]
const MAX_LINE: usize = 16 * 1024 * 1024;

#[cfg(unix)]
fn listen_unix(path: &Path, inbox: Inbox) -> io::Result<()> {
    listen(&Endpoint::Socket(path.to_path_buf()), inbox)
}

#[cfg(unix)]
fn test_dir(suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("skapi-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Processes the hub until `want` reply lines have arrived on `reader` (or 5 s pass).
#[cfg(unix)]
fn collect_replies(
    hub: &mut Hub,
    host: &mut HeadlessHost,
    reader: &mut BufReader<UnixStream>,
    want: usize,
) -> Vec<Value> {
    let mut replies = Vec::new();
    let mut pending = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while replies.len() < want && Instant::now() < deadline {
        hub.process(host);
        // A read timeout keeps the partial line in `pending` and returns Err; retry next loop.
        if reader.read_until(b'\n', &mut pending).is_ok() && pending.last() == Some(&b'\n') {
            replies.push(serde_json::from_slice(&pending).expect("a JSON reply"));
            pending.clear();
        }
    }
    replies
}

#[cfg(unix)]
#[test]
fn unix_socket_serves_requests_and_is_private() {
    let dir = std::env::temp_dir().join(format!("sketchup-alt-api-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("api.sock");

    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    listen_unix(&path, hub.inbox()).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let mut client = UnixStream::connect(&path).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "execute",
        "params": {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}});
    writeln!(client, "{request}").unwrap();

    let mut reader = BufReader::new(client.try_clone().unwrap());
    let mut line = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while line.is_empty() && Instant::now() < deadline {
        hub.process(&mut host);
        let _ = reader.read_line(&mut line);
    }
    let reply: Value = serde_json::from_str(&line).expect("reply within 5 s");
    assert_eq!(reply["result"]["revision"], 1);

    // A regular file at the socket path is refused and left alone.
    drop(client);
    let file = dir.join("not-a-socket");
    std::fs::write(&file, "x").unwrap();
    assert!(listen_unix(&file, hub.inbox()).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn live_socket_is_not_taken_over() {
    let dir = test_dir("live");
    let path = dir.join("a.sock");
    let hub = Hub::new(|| {});
    listen_unix(&path, hub.inbox()).unwrap();

    let err = listen_unix(&path, Hub::new(|| {}).inbox()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
    assert!(
        UnixStream::connect(&path).is_ok(),
        "the first listener still works"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn stale_socket_is_replaced() {
    let dir = test_dir("stale");
    let path = dir.join("b.sock");
    // Dropping the listener leaves the socket file behind with nobody listening.
    drop(UnixListener::bind(&path).unwrap());

    let hub = Hub::new(|| {});
    listen_unix(&path, hub.inbox()).unwrap();
    assert!(UnixStream::connect(&path).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn symlink_is_refused() {
    let dir = test_dir("link");
    let target = dir.join("a.sock");
    let link = dir.join("c.sock");
    let hub = Hub::new(|| {});
    listen_unix(&target, hub.inbox()).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(listen_unix(&link, Hub::new(|| {}).inbox()).is_err());
    assert!(
        UnixStream::connect(&target).is_ok(),
        "the target still accepts connections"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn oversized_and_invalid_lines_get_errors_and_the_connection_survives() {
    let dir = test_dir("lines");
    let path = dir.join("api.sock");
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    listen_unix(&path, hub.inbox()).unwrap();

    let mut client = UnixStream::connect(&path).unwrap();
    let mut reader = BufReader::new(client.try_clone().unwrap());
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();

    let mut oversized = vec![b'a'; MAX_LINE + 10];
    oversized.push(b'\n');
    client.write_all(&oversized).unwrap();
    client.write_all(b"\xff\xfe\n").unwrap();
    writeln!(
        client,
        "{}",
        json!({"jsonrpc": "2.0", "id": 3, "method": "query", "params": {"type": "stats"}})
    )
    .unwrap();

    let replies = collect_replies(&mut hub, &mut host, &mut reader, 3);
    assert_eq!(replies.len(), 3, "{replies:?}");
    assert_eq!(replies[0]["error"]["code"], codes::INVALID_REQUEST);
    assert_eq!(replies[1]["error"]["code"], codes::PARSE_ERROR);
    assert_eq!(replies[2]["id"], 3);
    assert!(replies[2]["result"].is_object(), "{:?}", replies[2]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn the_socket_is_private_from_the_start_and_leaves_no_folder() {
    let dir = test_dir("private-bind");
    let path = dir.join("p.sock");
    let hub = Hub::new(|| {});
    listen_unix(&path, hub.inbox()).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("p.sock")],
        "no bind folder left"
    );
    UnixStream::connect(&path).expect("connects at the final path");
    std::fs::remove_file(&path).unwrap();
}

/// A pipe name no other test uses.
#[cfg(windows)]
fn pipe(name: &str) -> Endpoint {
    Endpoint::from_arg(&format!("sketchup-alt-test-{}-{name}", std::process::id()))
}

#[cfg(windows)]
#[test]
fn a_bare_pipe_name_gets_the_prefix() {
    assert_eq!(
        Endpoint::from_arg("main"),
        Endpoint::Pipe(r"\\.\pipe\main".into())
    );
    assert_eq!(
        Endpoint::from_arg(r"\\.\PIPE\main"),
        Endpoint::Pipe(r"\\.\PIPE\main".into())
    );
}

#[cfg(windows)]
#[test]
fn a_named_pipe_serves_requests() {
    let endpoint = pipe("describe");
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    listen(&endpoint, hub.inbox()).unwrap();

    // The client runs on its own thread while this one serves the hub. Two requests on one
    // connection: the server reads the second while the first reply is written.
    let (sent, replies) = std::sync::mpsc::channel();
    let client = endpoint.clone();
    std::thread::spawn(move || {
        let (reader, mut writer) = api::connect(&client).unwrap();
        let mut reader = BufReader::new(reader);
        for id in [1, 2] {
            let request = json!({"jsonrpc": "2.0", "id": id, "method": "describe"});
            writeln!(writer, "{request}").unwrap();
            writer.flush().unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            sent.send(line).unwrap();
        }
    });
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while got.len() < 2 && Instant::now() < deadline {
        hub.process(&mut host);
        if let Ok(line) = replies.recv_timeout(Duration::from_millis(20)) {
            got.push(serde_json::from_str::<Value>(&line).expect("a JSON reply"));
        }
    }
    assert_eq!(got.len(), 2, "{got:?}");
    for (reply, id) in got.iter().zip([1, 2]) {
        assert_eq!(reply["id"], id, "{reply}");
        assert!(reply["result"]["methods"].is_object(), "{reply}");
    }
}

#[cfg(windows)]
#[test]
fn a_live_pipe_is_not_taken_over() {
    let endpoint = pipe("live");
    let hub = Hub::new(|| {});
    listen(&endpoint, hub.inbox()).unwrap();
    let err = listen(&endpoint, Hub::new(|| {}).inbox()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AddrInUse, "{err}");
    assert!(
        api::connect(&endpoint).is_ok(),
        "the first listener still works"
    );
}

#[cfg(windows)]
#[test]
fn connecting_to_a_missing_pipe_fails() {
    assert!(api::connect(&pipe("missing")).is_err());
}
