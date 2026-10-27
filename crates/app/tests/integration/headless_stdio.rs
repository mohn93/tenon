//! Runs the real binary in headless stdio mode, the way an agent would.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

/// Runs `app --headless --stdio` with `extra` arguments, sends `requests` and closes stdin.
/// Returns the exit status and every message written to stdout.
fn session(extra: &[&str], requests: &[Value]) -> (ExitStatus, Vec<Value>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            // A process that already exited (a file it could not open) closed its stdin.
            let _ = writeln!(stdin, "{r}");
        }
        // Dropping stdin ends the session; the process must flush its replies and exit.
    }
    // Read on another thread so a shutdown regression fails instead of hanging the test.
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let messages: Vec<Value> = stdout
            .lines()
            .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
            .collect();
        let _ = tx.send(messages);
    });
    let messages = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(messages) => messages,
        Err(_) => {
            let _ = child.kill();
            panic!("the session did not end within 30 seconds");
        }
    };
    (child.wait().unwrap(), messages)
}

fn square() -> Vec<Value> {
    let c = [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]];
    (0..4)
        .map(|i| json!({"type": "add_edge", "start": c[i], "end": c[(i + 1) % 4]}))
        .collect()
}

#[test]
fn headless_stdio_session() {
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "subscribe"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "batch", "params": {"commands": square(), "atomic": true}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "query", "params": {"type": "stats"}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "render_view", "params": {"width": 32, "height": 24}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "describe"}),
    ];
    let (status, messages) = session(&[], &requests);
    assert!(status.success(), "exit status {status}");

    let replies: HashMap<u64, &Value> = messages
        .iter()
        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
        .collect();
    assert_eq!(replies.len(), 5, "one reply per request: {messages:?}");
    assert_eq!(replies[&1]["result"]["revision"], 0);
    assert_eq!(replies[&2]["result"]["revision"], 1);
    assert_eq!(replies[&3]["result"]["faces"], 1);
    let render = replies[&4];
    assert!(
        render["result"]["png_base64"].is_string() || render["error"]["code"] == -32002,
        "render_view: {render}"
    );
    assert_eq!(replies[&5]["result"]["schema_version"], 2);

    let changed: Vec<&Value> = messages
        .iter()
        .filter(|m| m["method"] == "changed")
        .collect();
    assert_eq!(changed.len(), 1, "one notification for the atomic batch");
    assert_eq!(changed[0]["params"]["revision"], 1);
}

#[test]
fn headless_opens_the_file_it_is_given() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("headless-open-{}.tnn", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_text = path.to_str().unwrap();
    let save = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "batch", "params": {"commands": square(), "atomic": true}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "execute", "params": {"type": "save", "path": path_text}}),
    ];
    let (status, messages) = session(&[], &save);
    assert!(status.success() && messages.iter().all(|m| m.get("error").is_none()));

    let ask = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "query", "params": {"type": "stats"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "query", "params": {"type": "file"}}),
    ];
    let (status, messages) = session(&[path_text], &ask);
    assert!(status.success(), "exit status {status}");
    assert_eq!(messages[0]["result"]["faces"], 1);
    assert_eq!(messages[1]["result"]["path"], json!(path_text));

    let missing = dir.join("headless-missing.tnn");
    let (status, messages) = session(&[missing.to_str().unwrap()], &ask);
    assert_eq!(
        status.code(),
        Some(1),
        "a file that cannot be opened stops the app"
    );
    assert!(messages.is_empty());
}

#[test]
fn headless_opens_a_sketchup_file_as_a_new_model() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../skp/tests/fixtures/cupboard.skp");
    let ask = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "query", "params": {"type": "stats"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "query", "params": {"type": "file"}}),
    ];
    let (status, messages) = session(&[path.to_str().unwrap()], &ask);
    assert!(status.success(), "exit status {status}");
    assert!(
        messages[0]["result"]["instances"].as_u64().unwrap() > 1,
        "{messages:?}"
    );
    assert_eq!(messages[1]["result"]["path"], Value::Null);
    assert_eq!(messages[1]["result"]["modified"], false);
}
