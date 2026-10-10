use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use api::{HeadlessHost, Hub, listen_unix};
use serde_json::{Value, json};

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

    // A second listener on the same path replaces the stale socket; a regular file is refused.
    drop(client);
    let file = dir.join("not-a-socket");
    std::fs::write(&file, "x").unwrap();
    assert!(listen_unix(&file, hub.inbox()).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
