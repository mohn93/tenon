//! Runs the real binary in headless stdio mode, the way an agent would.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn headless_stdio_session() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");

    let c = [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]];
    let square: Vec<Value> = (0..4)
        .map(|i| json!({"type": "add_edge", "start": c[i], "end": c[(i + 1) % 4]}))
        .collect();
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "subscribe"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "batch", "params": {"commands": square, "atomic": true}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "query", "params": {"type": "stats"}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "render_view", "params": {"width": 32, "height": 24}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "describe"}),
    ];
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in &requests {
            writeln!(stdin, "{r}").unwrap();
        }
        // Dropping stdin ends the session; the process must flush its replies and exit.
    }
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let messages: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let status = child.wait().unwrap();
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
    assert_eq!(replies[&5]["result"]["schema_version"], 1);

    let changed: Vec<&Value> = messages
        .iter()
        .filter(|m| m["method"] == "changed")
        .collect();
    assert_eq!(changed.len(), 1, "one notification for the atomic batch");
    assert_eq!(changed[0]["params"]["revision"], 1);
}
