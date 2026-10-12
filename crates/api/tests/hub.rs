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
    host.doc.execute(session::Command::Undo).unwrap();
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
