//! Runs the real binary as an MCP server, the way an MCP client would: on its own model and
//! forwarding to a running headless instance with `--live`.

use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(30);

fn req(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn tool(id: u64, name: &str, arguments: Value) -> Value {
    req(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

fn handshake() -> [Value; 2] {
    [
        req(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        ),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    ]
}

fn square() -> Vec<Value> {
    let c = [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]];
    (0..4)
        .map(|i| json!({"type": "add_edge", "start": c[i], "end": c[(i + 1) % 4]}))
        .collect()
}

/// A running `app --mcp`: its stdin, and its stdout lines arriving on a channel.
struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Mcp {
    fn start(extra: &[&str]) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
            .arg("--mcp")
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start app --mcp");
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                if tx.send(line.unwrap()).is_err() {
                    return;
                }
            }
        });
        Mcp {
            stdin: child.stdin.take(),
            child,
            lines,
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin.as_mut().unwrap(), "{message}").unwrap();
    }

    /// The next message on stdout, which must be a JSON-RPC 2.0 message.
    fn next(&mut self) -> Value {
        let line = match self.lines.recv_timeout(TIMEOUT) {
            Ok(line) => line,
            Err(_) => {
                let _ = self.child.kill();
                panic!("no reply within {TIMEOUT:?}");
            }
        };
        let message: Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
        assert_eq!(message["jsonrpc"], "2.0", "{line}");
        message
    }

    /// Sends `request` and returns its reply.
    fn ask(&mut self, request: &Value) -> Value {
        self.send(request);
        let reply = self.next();
        assert_eq!(reply["id"], request["id"], "{reply}");
        reply
    }

    /// Closes stdin and returns the exit status and any messages still on stdout.
    fn finish(mut self) -> (ExitStatus, Vec<Value>) {
        drop(self.stdin.take());
        let status = exits(&mut self.child);
        let mut rest = Vec::new();
        while let Ok(line) = self.lines.recv_timeout(TIMEOUT) {
            let message: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(message["jsonrpc"], "2.0", "{line}");
            rest.push(message);
        }
        (status, rest)
    }
}

/// Waits up to the timeout for `child` to exit on its own.
fn exits(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app did not exit within {TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The parsed JSON in a tool result's text block.
fn text_json(result: &Value) -> Value {
    let text = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "text")
        .expect("a text block")["text"]
        .as_str()
        .unwrap();
    serde_json::from_str(text).unwrap()
}

/// A scratch directory short enough to hold a Unix socket (about 104 bytes on macOS).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("skmcp-{}-{name}", std::process::id()));
    let dir = if dir.as_os_str().len() > 80 {
        PathBuf::from("/tmp").join(format!("skmcp-{}-{name}", std::process::id()))
    } else {
        dir
    };
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mcp_headless_session() {
    let mut mcp = Mcp::start(&[]);
    let [init, initialized] = handshake();
    let reply = mcp.ask(&init);
    assert_eq!(reply["result"]["serverInfo"]["name"], "sketchup-alt");
    mcp.send(&initialized);
    let tools = mcp.ask(&req(2, "tools/list", json!({})));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 7);
    let created = mcp.ask(&tool(3, "transact", json!({"commands": square()})));
    assert_eq!(created["result"]["isError"], false, "{created}");
    let changes = &created["result"]["structuredContent"]["changes"];
    assert_eq!(changes["created"].as_array().unwrap().len(), 9, "{created}");
    let stats = mcp.ask(&tool(4, "query", json!({"type": "stats"})));
    assert_eq!(text_json(&stats["result"])["faces"], 1, "{stats}");
    let (status, rest) = mcp.finish();
    assert!(status.success(), "exit status {status}");
    assert!(rest.is_empty(), "exactly 4 replies; also {rest:?}");
}

#[test]
fn mcp_opens_a_file_given() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("mcp-open-{}.skr", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_text = path.to_str().unwrap();

    let mut mcp = Mcp::start(&[]);
    let [init, initialized] = handshake();
    mcp.ask(&init);
    mcp.send(&initialized);
    let mut commands = square();
    commands.push(json!({"type": "save", "path": path_text}));
    let saved = mcp.ask(&tool(
        2,
        "transact",
        json!({"commands": commands, "atomic": false}),
    ));
    assert_eq!(saved["result"]["isError"], false, "{saved}");
    assert!(mcp.finish().0.success());

    let mut mcp = Mcp::start(&[path_text]);
    mcp.ask(&init);
    mcp.send(&initialized);
    let stats = mcp.ask(&tool(2, "query", json!({"type": "stats"})));
    assert_eq!(text_json(&stats["result"])["faces"], 1, "{stats}");
    assert!(mcp.finish().0.success());
    std::fs::remove_file(&path).unwrap();
}

/// The agent API endpoint named `name` for this test: a socket in `dir`, or a pipe on Windows.
#[cfg(unix)]
fn endpoint(dir: &std::path::Path, name: &str) -> String {
    dir.join(format!("{name}.sock"))
        .to_str()
        .unwrap()
        .to_string()
}

#[cfg(windows)]
fn endpoint(_: &std::path::Path, name: &str) -> String {
    format!(r"\\.\pipe\skmcp-{}-{name}", std::process::id())
}

/// Waits up to the timeout for the socket at `path` to exist.
#[cfg(unix)]
fn wait_for(path: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while !std::path::Path::new(path).exists() {
        assert!(Instant::now() < deadline, "no socket at {path}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits up to the timeout for the pipe `name` to accept a client.
#[cfg(windows)]
fn wait_for(name: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while open_pipe(name).is_err() {
        assert!(Instant::now() < deadline, "no pipe at {name}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn open_pipe(name: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
}

/// Sends one request over the agent API endpoint and returns its reply.
#[cfg(unix)]
fn ask_socket(path: &str, request: &Value) -> Value {
    let mut stream = UnixStream::connect(path).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    writeln!(stream, "{request}").unwrap();
    read_reply(BufReader::new(stream), request)
}

#[cfg(windows)]
fn ask_socket(name: &str, request: &Value) -> Value {
    let mut pipe = open_pipe(name).unwrap();
    writeln!(pipe, "{request}").unwrap();
    read_reply(BufReader::new(pipe), request)
}

/// Reads lines until the reply to `request`.
fn read_reply(mut reader: impl BufRead, request: &Value) -> Value {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        if reply["id"] == request["id"] {
            return reply;
        }
    }
}

/// Kills the child when dropped, so a failing test leaves no instance behind.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn mcp_live_forwards_to_a_running_instance() {
    let dir = scratch("live");
    let socket = endpoint(&dir, "w");
    let mut instance = KillOnDrop(
        Command::new(env!("CARGO_BIN_EXE_app"))
            .args(["--headless", "--socket", &socket])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the headless instance"),
    );
    wait_for(&socket);

    let mut mcp = Mcp::start(&["--live", &socket]);
    let [init, initialized] = handshake();
    mcp.ask(&init);
    mcp.send(&initialized);
    let edge = json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]});
    let made = mcp.ask(&tool(2, "transact", json!({"commands": [edge]})));
    assert_eq!(made["result"]["isError"], false, "{made}");

    let stats = ask_socket(&socket, &req(1, "query", json!({"type": "stats"})));
    assert_eq!(stats["result"]["edges"], 1, "{stats}");
    let history = ask_socket(&socket, &req(2, "query", json!({"type": "history"})));
    assert_eq!(
        history["result"]["undo_origins"],
        json!(["agent"]),
        "{history}"
    );

    instance.0.kill().unwrap();
    instance.0.wait().unwrap();
    let lost = mcp.ask(&tool(3, "query", json!({"type": "stats"})));
    assert_eq!(lost["result"]["isError"], true, "{lost}");
    assert_eq!(text_json(&lost["result"])["code"], -32003, "{lost}");
    // The server keeps answering until stdin closes.
    let again = mcp.ask(&tool(4, "query", json!({"type": "stats"})));
    assert_eq!(text_json(&again["result"])["code"], -32003, "{again}");
    let (status, rest) = mcp.finish();
    assert!(status.success(), "exit status {status}");
    assert!(rest.is_empty(), "{rest:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_live_to_a_missing_socket_fails_at_start() {
    let dir = scratch("missing");
    let socket = endpoint(&dir, "none");
    let output = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--mcp", "--live", &socket])
        .stdin(Stdio::null())
        .output()
        .expect("run app --mcp --live");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout is for MCP messages only");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot connect to"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
