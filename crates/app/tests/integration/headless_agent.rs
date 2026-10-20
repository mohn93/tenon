//! Runs the real binary headless and drives its editor the way an agent would, ending with
//! `quit` while the connection is still open.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(windows)]
#[test]
fn quitting_over_a_named_pipe_answers() {
    // A bare name: the app adds the `\\.\pipe\` prefix.
    let name = format!("skq-{}", std::process::id());
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--socket", &name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start app");
    let path = format!(r"\\.\pipe\{name}");
    let deadline = Instant::now() + Duration::from_secs(30);
    let pipe = loop {
        if let Ok(p) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            break p;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app never listened");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    writeln!(&pipe, "{}", request(1, "quit", Value::Null)).unwrap();
    let mut line = String::new();
    BufReader::new(&pipe).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["result"], json!({}), "{line}");
    assert!(exits(&mut child).success());
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

#[test]
fn an_agent_is_kept_inside_its_api_root() {
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("agent-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio", "--api-root"])
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = child.stdin.take().unwrap();
    let requests = [
        request(
            1,
            "execute",
            json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
        ),
        request(2, "execute", json!({"type": "save", "path": "inside.skr"})),
        request(
            3,
            "execute",
            json!({"type": "save", "path": base.join("outside.skr")}),
        ),
        request(4, "query", json!({"type": "file"})),
    ];
    for r in &requests {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let status = exits(&mut child);
    assert!(status.success(), "exit status {status}");
    let replies: HashMap<u64, Value> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
        .collect();
    assert!(replies[&2]["result"].is_object(), "{:?}", replies[&2]);
    assert_eq!(replies[&3]["error"]["data"]["kind"], "invalid_input");
    assert!(root.join("inside.skr").exists() && !base.join("outside.skr").exists());
    assert_eq!(
        replies[&4]["result"]["api_roots"],
        json!([root.canonicalize().unwrap()])
    );
}

#[test]
fn a_missing_api_root_stops_the_app() {
    let status = Command::new(env!("CARGO_BIN_EXE_app"))
        .args([
            "--headless",
            "--stdio",
            "--api-root",
            "/no/such/folder/at/all",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

#[test]
fn a_relative_file_argument_saves_where_it_lies_not_in_the_root() {
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("agent-relfile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, work) = (base.join("root"), base.join("work"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // A model to open: made by a first run with no roots.
    let mut first = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio"])
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = first.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        request(1, "execute", json!({"type": "save", "path": "house.skr"}))
    )
    .unwrap();
    drop(stdin);
    assert!(exits(&mut first).success());
    assert!(work.join("house.skr").exists());

    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio", "--api-root"])
        .arg(&root)
        .arg("house.skr")
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{}", request(1, "execute", json!({"type": "save"}))).unwrap();
    drop(stdin);
    assert!(exits(&mut child).success());
    let line = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .next()
        .unwrap()
        .unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["error"]["data"]["kind"], "invalid_input", "{reply}");
    assert!(!root.join("house.skr").exists());
}
