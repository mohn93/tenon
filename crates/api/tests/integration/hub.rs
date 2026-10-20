use std::sync::mpsc::Receiver;
use std::time::Duration;

use api::{HeadlessHost, Hub};
use serde_json::{Value, json};

fn next(rx: &Receiver<String>) -> Value {
    let line = rx.recv_timeout(Duration::from_secs(2)).expect("a message");
    serde_json::from_str(&line).unwrap()
}

fn request(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn edge(x: f64) -> Value {
    json!({"type": "add_edge", "start": [x, 0, 0], "end": [x, 1, 0]})
}

#[test]
fn replies_go_back_to_the_asking_connection() {
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (a, a_rx) = inbox.open();
    let (b, b_rx) = inbox.open();
    inbox.send_line(a, request(1, "query", json!({"type": "stats"})));
    inbox.send_line(b, request(2, "execute", edge(0.0)));
    hub.process(&mut host);
    assert_eq!(next(&a_rx)["id"], 1);
    assert_eq!(next(&b_rx)["id"], 2);
    assert!(a_rx.try_recv().is_err() && b_rx.try_recv().is_err());
}

#[test]
fn subscribers_get_every_change_in_order() {
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (watcher, w_rx) = inbox.open();
    let (writer, _r_rx) = inbox.open();

    inbox.send_line(watcher, request(1, "subscribe", json!(null)));
    hub.process(&mut host);
    assert_eq!(next(&w_rx)["result"]["revision"], 0);

    inbox.send_line(writer, request(2, "execute", edge(0.0)));
    inbox.send_line(writer, request(3, "execute", edge(1.0)));
    hub.process(&mut host);
    let first = next(&w_rx);
    assert_eq!(first["method"], "changed");
    assert_eq!(first["params"]["revision"], 1);
    assert_eq!(next(&w_rx)["params"]["revision"], 2);

    // Edits made outside the API (e.g. in the window) are announced by notify().
    host.doc
        .execute(session::Command::Undo {
            count: None,
            expect: None,
        })
        .unwrap();
    hub.notify(&host);
    let undo = next(&w_rx);
    assert_eq!(undo["params"]["revision"], 3);
    assert_eq!(
        undo["params"]["changes"]["deleted"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn closed_connections_are_forgotten() {
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (a, _rx) = inbox.open();
    hub.process(&mut host);
    assert_eq!(hub.open_connections(), 1);
    inbox.close(a);
    hub.process(&mut host);
    assert_eq!(hub.open_connections(), 0);
}

#[test]
fn subscribe_without_connection_is_rejected_by_plain_handle() {
    let mut host = HeadlessHost::new();
    let r = api::handle(&mut host, &request(1, "subscribe", json!(null))).unwrap();
    let r: Value = serde_json::from_str(&r).unwrap();
    assert_eq!(r["error"]["code"], api::codes::INVALID_REQUEST);
}

#[test]
fn notifications_reach_a_connection_that_closes_right_after() {
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (conn, rx) = inbox.open();
    inbox.send_line(conn, request(1, "subscribe", json!(null)));
    inbox.send_line(conn, request(2, "execute", edge(0.0)));
    inbox.close(conn);
    hub.process(&mut host);
    let messages: Vec<Value> = rx
        .try_iter()
        .map(|l| serde_json::from_str(&l).unwrap())
        .collect();
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert_eq!(messages[2]["method"], "changed");
}

#[test]
fn opening_a_file_tells_subscribers_to_read_the_model_again() {
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("hub-open-{}.skr", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path = path.to_str().unwrap();
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (watcher, w_rx) = inbox.open();
    let (writer, r_rx) = inbox.open();
    inbox.send_line(writer, request(1, "execute", edge(0.0)));
    inbox.send_line(
        writer,
        request(2, "execute", json!({"type": "save", "path": path})),
    );
    hub.process(&mut host);
    assert!(next(&r_rx)["result"].is_object());
    assert!(next(&r_rx)["result"].is_object());

    inbox.send_line(watcher, request(3, "subscribe", json!(null)));
    hub.process(&mut host);
    assert_eq!(next(&w_rx)["result"]["revision"], 1);
    inbox.send_line(
        writer,
        request(4, "execute", json!({"type": "open", "path": path})),
    );
    hub.process(&mut host);
    let note = next(&w_rx);
    assert_eq!(note["method"], "resync", "{note}");
    assert_eq!(note["params"]["revision"], 2);
}

/// Every message waiting on `rx`.
fn drain(rx: &Receiver<String>) -> Vec<Value> {
    rx.try_iter()
        .map(|l| serde_json::from_str(&l).unwrap())
        .collect()
}

#[test]
fn selection_and_camera_changes_are_notified_once_per_round() {
    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (watcher, w_rx) = inbox.open();
    let (writer, _r_rx) = inbox.open();
    inbox.send_line(writer, request(1, "execute", edge(0.0)));
    hub.process(&mut host);
    // Nobody listens yet: nothing is kept or sent.
    hub.notify_state(&host);

    inbox.send_line(watcher, request(2, "subscribe", json!(null)));
    hub.process(&mut host);
    hub.notify_state(&host);
    assert_eq!(drain(&w_rx).len(), 1, "only the subscribe reply");

    let select = json!({"type": "select", "ids": ["e2"]});
    let camera = |x: i32| json!({"type": "set_camera", "eye": [x, -10, 5], "target": [0, 0, 0]});
    inbox.send_line(writer, request(3, "execute", select));
    inbox.send_line(writer, request(4, "execute", camera(10)));
    inbox.send_line(writer, request(5, "execute", camera(12)));
    hub.process(&mut host);
    hub.notify_state(&host);
    let messages = drain(&w_rx);
    let methods: Vec<&str> = messages
        .iter()
        .map(|m| m["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, vec!["selection", "camera"]);
    assert_eq!(messages[0]["params"]["ids"], json!(["e2"]));
    assert_eq!(messages[0]["params"]["revision"], 1);
    assert_eq!(messages[1]["params"]["eye"], json!([12.0, -10.0, 5.0]));
    assert!(messages[1]["params"]["type"].is_null());

    // Changes made outside the API are notified the same way; nothing changed, nothing sent.
    host.doc
        .execute(session::Command::Select {
            ids: Vec::new(),
            mode: session::SelectMode::Replace,
            all: false,
        })
        .unwrap();
    hub.notify_state(&host);
    hub.notify_state(&host);
    let messages = drain(&w_rx);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["params"]["ids"], json!([]));
}

#[cfg(unix)]
#[test]
fn closing_waits_for_the_last_replies_to_be_written() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let (served, client) = UnixStream::pair().unwrap();
    api::serve_stream(hub.inbox(), served.try_clone().unwrap(), served);
    writeln!(&client, "{}", request(1, "execute", edge(0.0))).unwrap();
    // The request arrives from the reader thread; handling it bumps the revision.
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.doc.revision() == 0 {
        assert!(Instant::now() < deadline, "no request arrived");
        hub.process(&mut host);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(hub.close(Duration::from_secs(5)));
    client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut line = String::new();
    BufReader::new(&client).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["result"]["revision"], 1);
}

#[test]
fn a_connection_opened_while_closing_does_not_hold_it_up() {
    use std::io::{Read, Write};
    use std::time::Instant;

    /// A reader that sends nothing for longer than the test runs.
    struct Silent;
    impl Read for Silent {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_secs(30));
            Ok(0)
        }
    }
    /// A writer that takes 300 ms per write.
    struct Slow;
    impl Write for Slow {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_millis(300));
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut host = HeadlessHost::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    // A connection still writing its last reply when closing starts.
    let (busy, busy_writer) = api::serve_stream(inbox.clone(), Silent, Slow);
    inbox.reply(busy, "last".into());
    hub.process(&mut host);
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        // Opened while closing waits: its writer must end at once rather than with the hub.
        api::serve_stream(inbox, Silent, std::io::sink()).1
    });
    let start = Instant::now();
    assert!(hub.close(Duration::from_secs(5)));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    late.join().unwrap().join().unwrap();
    busy_writer.join().unwrap();
}

#[test]
fn the_activity_log_records_requests_folds_repeats_and_keeps_the_last_hundred() {
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let mut host = HeadlessHost::new();
    let (conn, rx) = inbox.open();
    inbox.send_line(
        conn,
        request(
            1,
            "execute",
            json!({"type": "save", "path": "/no/such/dir/a.skr"}),
        ),
    );
    inbox.send_line(conn, request(2, "query", json!({"type": "camera"})));
    inbox.send_line(conn, request(3, "query", json!({"type": "camera"})));
    inbox.send_line(conn, "not json".into());
    hub.process(&mut host);
    for _ in 0..4 {
        next(&rx);
    }
    let log: Vec<_> = hub.activity().iter().collect();
    assert_eq!(log[0].request, "connected");
    assert_eq!(log[1].request, "execute save /no/such/dir/a.skr");
    assert!(log[1].error.as_deref().unwrap().contains("a.skr"));
    assert_eq!(log[2].request, "query camera");
    assert_eq!((log[2].repeats, log[2].error.clone()), (2, None));
    assert_eq!(log[3].request, "(not JSON)");
    assert!(log[3].error.as_deref().unwrap().starts_with("parse error"));
    assert_eq!(log.len(), 4);
    let revision = hub.activity_revision();
    for i in 0..150 {
        let command = if i % 2 == 0 {
            json!({"type": "select", "ids": []})
        } else {
            json!({"type": "zoom_extents"})
        };
        inbox.send_line(conn, request(10 + i, "execute", command));
    }
    inbox.close(conn);
    hub.process(&mut host);
    assert_eq!(hub.activity().len(), api::ACTIVITY_LEN);
    assert_eq!(hub.activity().back().unwrap().request, "disconnected");
    assert!(hub.activity_revision() > revision);
}

#[test]
fn a_request_that_fails_before_the_host_is_logged_with_its_error() {
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let mut host = HeadlessHost::new();
    let (conn, rx) = inbox.open();
    inbox.send_line(conn, request(1, "execute", json!({"type": "nope"})));
    hub.process(&mut host);
    let reply = next(&rx);
    assert_eq!(reply["error"]["code"], -32602);
    let entry = hub.activity().back().unwrap();
    assert_eq!(entry.request, "execute nope");
    assert_eq!(
        entry.error.as_deref(),
        reply["error"]["message"].as_str(),
        "the log keeps the -32602 message"
    );
}
