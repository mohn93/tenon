//! Runs the real binary headless and drives its editor the way an agent would, ending with
//! `quit` while the connection is still open.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// Waits up to 30 seconds for `child` to exit on its own.
fn exits(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app did not quit within 30 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_agent_sets_the_editor_and_quits_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = child.stdin.take().unwrap();
    let requests = [
        request(1, "subscribe", Value::Null),
        request(
            2,
            "execute",
            json!({"type": "set_tool", "tool": "rectangle"}),
        ),
        request(3, "query", json!({"type": "editor"})),
        request(
            4,
            "execute",
            json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
        ),
        request(5, "query", json!({"type": "history"})),
        request(6, "quit", Value::Null),
        request(7, "quit", json!({"discard": true})),
    ];
    for r in &requests {
        writeln!(stdin, "{r}").unwrap();
    }
    // Stdin stays open: only `quit` can end the app.
    let status = exits(&mut child);
    drop(stdin);
    assert!(status.success(), "exit status {status}");
    let messages: Vec<Value> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let replies: HashMap<u64, &Value> = messages
        .iter()
        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
        .collect();
    assert_eq!(replies.len(), 7, "{messages:?}");
    assert_eq!(replies[&2]["result"]["editor"]["tool"], "rectangle");
    let editor = &replies[&3]["result"];
    assert_eq!(editor["type"], "editor");
    assert_eq!(editor["phase"], "idle");
    assert_eq!(editor["viewport"], json!([1280.0, 800.0]));
    assert_eq!(replies[&5]["result"]["undo"], json!(["add_edge"]));
    assert_eq!(replies[&6]["error"]["data"]["kind"], "unsaved_changes");
    assert_eq!(replies[&7]["result"], json!({}));
    let tools: Vec<&Value> = messages
        .iter()
        .filter(|m| m["method"] == "editor")
        .map(|m| &m["params"]["tool"])
        .collect();
    assert_eq!(tools, vec!["rectangle"], "{messages:?}");
}

#[test]
fn quitting_over_a_socket_answers_and_removes_the_socket() {
    let dir = std::env::temp_dir().join(format!("skq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("q.sock");
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--socket", path.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start app");
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        if let Ok(s) = UnixStream::connect(&path) {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app never listened");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    writeln!(&stream, "{}", request(1, "quit", Value::Null)).unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["result"], json!({}), "{line}");
    assert!(exits(&mut child).success());
    assert!(!path.exists(), "the socket file is removed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_agent_draws_a_box_with_the_tools_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = child.stdin.take().unwrap();
    let requests = [
        request(1, "subscribe", Value::Null),
        request(
            2,
            "execute",
            json!({"type": "set_tool", "tool": "rectangle"}),
        ),
        request(
            3,
            "tool_input",
            json!({"events": [
                {"type": "click", "point": [0, 0, 0]},
                {"type": "move", "point": [1, 1, 0]},
                {"type": "type", "text": "2,3"},
                {"type": "key", "key": "p"},
                {"type": "click", "point": [1, 1, 0]},
                {"type": "move", "point": [1, 1, 0.5]},
                {"type": "type", "text": "1"}
            ]}),
        ),
        request(4, "query", json!({"type": "stats"})),
        request(5, "quit", json!({"discard": true})),
    ];
    for r in &requests {
        writeln!(stdin, "{r}").unwrap();
    }
    let status = exits(&mut child);
    drop(stdin);
    assert!(status.success(), "exit status {status}");
    let messages: Vec<Value> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let replies: HashMap<u64, &Value> = messages
        .iter()
        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
        .collect();
    let drawn = &replies[&3]["result"];
    let ran: Vec<&str> = drawn["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["command"]["type"].as_str().unwrap())
        .collect();
    assert_eq!(ran, ["add_rectangle", "push_pull"], "{drawn}");
    assert_eq!(drawn["editor"]["tool"], "push_pull");
    assert_eq!(replies[&4]["result"]["faces"], 6);
    // Subscribers hear of both edits.
    let changed = messages.iter().filter(|m| m["method"] == "changed").count();
    assert_eq!(changed, 2, "{messages:?}");
}
