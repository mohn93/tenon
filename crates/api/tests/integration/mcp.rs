use api::mcp::{HostBackend, PROTOCOL_VERSIONS, serve_mcp};
use api::{HeadlessHost, codes};
use base64::Engine;
use serde_json::{Value, json};

/// Feeds `lines` to a fresh headless server and returns every output line, parsed.
fn run(lines: &[Value]) -> Vec<Value> {
    run_raw(
        &lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn run_raw(input: &str) -> Vec<Value> {
    let mut host = HeadlessHost::new();
    let mut out = Vec::new();
    serve_mcp(&mut HostBackend(&mut host), input.as_bytes(), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn req(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn notif(method: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": method})
}

fn init(id: i64, version: &str) -> Value {
    req(
        id,
        "initialize",
        json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }),
    )
}

fn tool(id: i64, name: &str, arguments: Value) -> Value {
    req(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

/// Runs `calls` after a 2025-06-18 handshake and returns their replies.
fn session(calls: &[Value]) -> Vec<Value> {
    let mut lines = vec![init(0, "2025-06-18"), notif("notifications/initialized")];
    lines.extend_from_slice(calls);
    let mut out = run(&lines);
    out.remove(0);
    out
}

fn square() -> Value {
    let c = [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]];
    json!(
        (0..4)
            .map(|i| json!({"type": "add_edge", "start": c[i], "end": c[(i + 1) % 4]}))
            .collect::<Vec<_>>()
    )
}

/// The parsed text block of a tool result.
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

#[test]
fn handshake_lists_seven_tools() {
    let out = run(&[
        init(1, "2025-06-18"),
        notif("notifications/initialized"),
        req(2, "tools/list", json!({})),
    ]);
    assert_eq!(out.len(), 2); // the notification got no reply
    assert_eq!(out[0]["id"], 1);
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "tenon");
    assert_eq!(
        out[0]["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert!(
        out[0]["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("meters")
    );
    let names: Vec<_> = out[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "describe",
            "transact",
            "query",
            "pick",
            "infer",
            "render_view",
            "tool_input"
        ]
    );
    for t in out[1]["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(t["description"].as_str().unwrap().len() > 20);
        assert!(t["annotations"]["readOnlyHint"].is_boolean());
        assert!(t.get("outputSchema").is_none());
    }
    let read_only: Vec<_> = out[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == true)
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        read_only,
        ["describe", "query", "pick", "infer", "render_view"]
    );
}

#[test]
fn an_unknown_version_gets_the_newest() {
    let out = run(&[init(1, "1999-01-01")]);
    assert_eq!(out[0]["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    assert_eq!(PROTOCOL_VERSIONS[0], "2025-11-25");
}

#[test]
fn requests_before_initialize_are_refused_but_ping_is_not() {
    let out = run(&[
        req(1, "tools/list", json!({})),
        req(2, "ping", json!({})),
        init(3, "2025-03-26"),
        init(4, "2025-06-18"),
        req(5, "tools/list", json!({})),
    ]);
    assert_eq!(out[0]["error"]["code"], codes::INVALID_REQUEST);
    assert_eq!(out[0]["error"]["message"], "call initialize first");
    assert_eq!(out[1]["result"], json!({}));
    assert_eq!(out[2]["result"]["protocolVersion"], "2025-03-26");
    // A second initialize is answered again.
    assert_eq!(out[3]["result"]["protocolVersion"], "2025-06-18");
    assert!(out[4]["result"]["tools"].is_array());
}

#[test]
fn malformed_lines_and_client_replies() {
    let lines = [
        "not json".to_string(),
        "[1, 2]".to_string(),
        "\"text\"".to_string(),
        json!({"jsonrpc": "2.0", "id": 9, "result": {}}).to_string(),
        String::new(),
        req(1, "ping", json!({})).to_string(),
        init(2, "2025-06-18").to_string(),
        req(3, "resources/list", json!({})).to_string(),
    ];
    let out = run_raw(&lines.join("\n"));
    assert_eq!(out.len(), 6, "{out:?}"); // the client's reply and the blank line get none
    assert_eq!(out[0]["error"]["code"], codes::PARSE_ERROR);
    assert_eq!(out[1]["error"]["code"], codes::INVALID_REQUEST);
    assert_eq!(out[1]["id"], Value::Null);
    assert_eq!(out[2]["error"]["code"], codes::INVALID_REQUEST);
    assert_eq!(out[3]["id"], 1);
    assert_eq!(out[5]["error"]["code"], codes::METHOD_NOT_FOUND);
}

#[test]
fn transact_runs_one_atomic_step_and_reports_net_changes() {
    let out = session(&[
        tool(1, "transact", json!({"commands": square()})),
        tool(2, "query", json!({"type": "history"})),
    ]);
    let r = &out[0]["result"];
    assert_eq!(r["isError"], false);
    let s = &r["structuredContent"];
    assert_eq!(s["changes"]["created"].as_array().unwrap().len(), 9);
    assert_eq!(s["outcomes"].as_array().unwrap().len(), 4);
    assert_eq!(s["revision"], 1);
    assert_eq!(text_json(r), *s);
    let h = &out[1]["result"]["structuredContent"];
    assert_eq!(h["undo_depth"], 1);
    assert_eq!(h["undo_origins"], json!(["agent"]));
}

#[test]
fn transact_of_one_command_has_the_batch_shape() {
    let out = session(&[tool(
        1,
        "transact",
        json!({"commands": [{"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}]}),
    )]);
    let s = &out[0]["result"]["structuredContent"];
    assert_eq!(s["outcomes"].as_array().unwrap().len(), 1);
    assert_eq!(s["changes"], s["outcomes"][0]["changes"]);
    assert_eq!(s["changes"]["created"].as_array().unwrap().len(), 3);
    assert_eq!(s["revision"], 1);
}

#[test]
fn a_failed_atomic_transact_changes_nothing() {
    let mut cmds = square();
    let cmds = cmds.as_array_mut().unwrap();
    cmds.truncate(2);
    cmds.push(json!({"type": "add_edge", "start": [0, 0, 0], "end": [2e6, 0, 0]}));
    let out = session(&[
        tool(1, "transact", json!({"commands": cmds})),
        tool(2, "query", json!({"type": "stats"})),
    ]);
    let r = &out[0]["result"];
    assert_eq!(r["isError"], true);
    let e = &r["structuredContent"];
    assert_eq!(e["code"], codes::BATCH_FAILED);
    assert_eq!(e["data"]["index"], 2);
    assert_eq!(e["data"]["rolled_back"], true);
    assert_eq!(text_json(r), *e);
    assert_eq!(out[1]["result"]["structuredContent"]["revision"], 0);
}

#[test]
fn transact_can_opt_out_of_atomic() {
    let mut cmds = square();
    cmds.as_array_mut().unwrap().push(json!({"type": "undo"}));
    let out = session(&[
        tool(1, "transact", json!({"commands": cmds, "atomic": false})),
        tool(2, "transact", json!({"commands": []})),
    ]);
    // Not atomic: undo runs inside the batch.
    assert_eq!(out[0]["result"]["isError"], false);
    assert_eq!(
        out[0]["result"]["structuredContent"]["outcomes"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(out[1]["result"]["isError"], true);
    assert_eq!(
        out[1]["result"]["structuredContent"]["code"],
        codes::INVALID_PARAMS
    );
}

/// A backend that records each call and answers with a canned reply.
struct Recording {
    calls: Vec<(String, Value)>,
    reply: Value,
}

impl api::mcp::Backend for Recording {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, api::RpcError> {
        self.calls.push((method.to_string(), params));
        Ok(self.reply.clone())
    }
}

fn run_with(backend: &mut dyn api::mcp::Backend, lines: &[Value]) -> Vec<Value> {
    let input = lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    serve_mcp(backend, input.as_bytes(), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_lone_editor_command_goes_through_execute() {
    let mut backend = Recording {
        calls: Vec::new(),
        reply: json!({"editor": {"tool": "line"}, "revision": 0}),
    };
    let set_tool = json!({"type": "set_tool", "tool": "line"});
    let out = run_with(
        &mut backend,
        &[
            init(1, "2025-06-18"),
            tool(2, "transact", json!({"commands": [set_tool]})),
        ],
    );
    assert_eq!(backend.calls, [("execute".to_string(), set_tool)]);
    let s = &out[1]["result"]["structuredContent"];
    assert_eq!(s["editor"]["tool"], "line");
    assert_eq!(*s, json!({"editor": {"tool": "line"}, "revision": 0}));
}

#[test]
fn forwarded_arguments() {
    let mut backend = Recording {
        calls: Vec::new(),
        reply: json!({}),
    };
    let events = json!({"events": [{"type": "key", "key": "escape"}]});
    run_with(
        &mut backend,
        &[
            init(1, "2025-06-18"),
            tool(2, "pick", json!({"pixel": [1, 2]})),
            tool(3, "infer", json!({"point": [1, 2, 3]})),
            tool(4, "render_view", json!({"width": 10})),
            tool(5, "tool_input", events.clone()),
            tool(
                6,
                "transact",
                json!({"commands": [{"type": "undo"}, {"type": "redo"}]}),
            ),
        ],
    );
    assert_eq!(
        backend.calls,
        [
            (
                "query".to_string(),
                json!({"type": "pick", "pixel": [1, 2]})
            ),
            (
                "query".to_string(),
                json!({"type": "infer", "point": [1, 2, 3]})
            ),
            (
                "render_view".to_string(),
                json!({"width": 10, "height": 600})
            ),
            ("tool_input".to_string(), events),
            (
                "batch".to_string(),
                json!({"commands": [{"type": "undo"}, {"type": "redo"}], "atomic": true})
            ),
        ]
    );
}

#[test]
fn render_view_treats_null_sides_as_absent() {
    let mut backend = Recording {
        calls: Vec::new(),
        reply: json!({}),
    };
    run_with(
        &mut backend,
        &[
            init(1, "2025-06-18"),
            tool(2, "render_view", json!({"width": null, "height": null})),
            tool(3, "render_view", json!({"width": 10, "height": null})),
        ],
    );
    assert_eq!(
        backend.calls,
        [
            (
                "render_view".to_string(),
                json!({"width": 960, "height": 600})
            ),
            (
                "render_view".to_string(),
                json!({"width": 10, "height": 600})
            ),
        ]
    );
}

#[test]
fn pick_and_infer_forward_as_queries() {
    let out = session(&[
        tool(1, "transact", json!({"commands": square()})),
        tool(
            2,
            "pick",
            json!({"ray": {"origin": [1, 1, 5], "direction": [0, 0, -1]}}),
        ),
        tool(3, "infer", json!({"point": [2, 0, 0]})),
    ]);
    let pick = &out[1]["result"];
    assert_eq!(pick["isError"], false, "{pick}");
    assert_eq!(pick["structuredContent"]["type"], "pick");
    assert_eq!(
        pick["structuredContent"]["hits"].as_array().unwrap().len(),
        1
    );
    let infer = &out[2]["result"];
    assert_eq!(infer["isError"], false, "{infer}");
    assert_eq!(infer["structuredContent"]["type"], "infer");
}

#[test]
fn render_view_returns_an_image_block() {
    let out = session(&[
        tool(1, "transact", json!({"commands": square()})),
        tool(2, "render_view", json!({})),
    ]);
    let r = &out[1]["result"];
    if r["isError"] == true {
        assert_eq!(r["structuredContent"]["code"], codes::RENDER_UNAVAILABLE);
        eprintln!("no GPU adapter, skipping");
        return;
    }
    let content = r["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mimeType"], "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content[0]["data"].as_str().unwrap())
        .unwrap();
    let reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    assert_eq!((reader.info().width, reader.info().height), (960, 600));
    assert_eq!(content[1]["type"], "text");
    assert_eq!(text_json(r), json!({"width": 960, "height": 600}));
}

#[test]
fn render_view_takes_the_render_options() {
    let out = run(&[init(1, "2025-11-25"), req(2, "tools/list", json!({}))]);
    let tools = out[1]["result"]["tools"].as_array().unwrap();
    let schema = &tools.iter().find(|t| t["name"] == "render_view").unwrap()["inputSchema"];
    for field in [
        "lighting",
        "style",
        "projection",
        "section",
        "shadows",
        "supersample",
        "background",
        "joints",
    ] {
        assert!(schema["properties"][field].is_object(), "{field}");
    }
    let out = session(&[
        tool(1, "transact", json!({"commands": square()})),
        tool(
            2,
            "render_view",
            json!({"width": 32, "height": 24, "projection": "parallel", "shadows": true,
                   "supersample": 2, "background": "transparent", "lighting": "studio",
                   "section": {"cuts": false, "fill_color": "material"}}),
        ),
        tool(3, "render_view", json!({"supersample": 9})),
    ]);
    let r = &out[1]["result"];
    if r["isError"] == true {
        assert_eq!(r["structuredContent"]["code"], codes::RENDER_UNAVAILABLE);
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(text_json(r), json!({"width": 32, "height": 24}));
    let r = &out[2]["result"];
    assert_eq!(r["isError"], true);
    assert_eq!(
        r["structuredContent"]["data"]["kind"], "invalid_input",
        "{r}"
    );
}

#[test]
fn render_view_checks_its_sides() {
    let out = session(&[tool(1, "render_view", json!({"width": 0}))]);
    assert_eq!(out[0]["result"]["isError"], true);
    assert_eq!(
        out[0]["result"]["structuredContent"]["code"],
        codes::INVALID_PARAMS
    );
}

#[test]
fn structured_content_only_for_newer_clients() {
    let out = run(&[
        init(1, "2024-11-05"),
        tool(2, "query", json!({"type": "stats"})),
        tool(3, "query", json!({"type": "nope"})),
    ]);
    for r in [&out[1]["result"], &out[2]["result"]] {
        assert!(r.get("structuredContent").is_none(), "{r}");
        assert_eq!(r["content"][0]["type"], "text");
    }
    assert_eq!(text_json(&out[1]["result"])["revision"], 0);
    assert_eq!(text_json(&out[2]["result"])["code"], codes::INVALID_PARAMS);
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let out = session(&[
        tool(1, "nope", json!({})),
        req(2, "tools/call", json!({"arguments": {}})),
    ]);
    assert_eq!(out[0]["error"]["code"], codes::INVALID_PARAMS);
    assert_eq!(out[1]["error"]["code"], codes::INVALID_PARAMS);
}

#[test]
fn bad_arguments_are_a_tool_error() {
    let out = session(&[
        tool(1, "query", json!({"type": "nope"})),
        tool(2, "pick", json!([1, 2])),
        tool(3, "transact", json!({"commands": square(), "extra": 1})),
    ]);
    for o in &out {
        let r = &o["result"];
        assert_eq!(r["isError"], true, "{r}");
        assert_eq!(r["structuredContent"]["code"], codes::INVALID_PARAMS, "{r}");
        assert!(r["structuredContent"]["message"].is_string());
    }
}

#[test]
fn describe_without_a_schema_is_the_slim_document() {
    let out = session(&[
        tool(1, "describe", json!({})),
        tool(2, "describe", json!({"schema": "Command"})),
        tool(3, "describe", json!({"schema": "batch"})),
        tool(4, "describe", json!({"schema": "Nope"})),
    ]);
    let slim = &out[0]["result"]["structuredContent"];
    assert_eq!(slim["schema_version"], 2);
    assert!(slim.get("schemas").is_none());
    assert!(slim["methods"]["execute"].get("params_schema").is_none());
    assert!(slim["methods"]["execute"].get("result_schema").is_none());
    assert!(slim["methods"]["execute"]["params"].is_string());
    let names: Vec<_> = slim["schema_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert!(names.contains(&"Command") && names.contains(&"QueryResult"));
    assert!(
        out[0]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .len()
            < 20_000
    );

    let full = api::describe();
    assert_eq!(
        out[1]["result"]["structuredContent"],
        full["schemas"]["Command"]
    );
    assert_eq!(
        out[2]["result"]["structuredContent"],
        json!({
            "params_schema": full["methods"]["batch"]["params_schema"],
            "result_schema": full["methods"]["batch"]["result_schema"],
        })
    );
    assert_eq!(out[3]["result"]["isError"], true);
    assert_eq!(
        out[3]["result"]["structuredContent"]["code"],
        codes::INVALID_PARAMS
    );
}

/// Every `$ref` under `v`.
fn refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x) {
                    ("$ref", Value::String(r)) => out.push(r.clone()),
                    _ => refs(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
        _ => {}
    }
}

#[test]
fn tool_schemas_hoist_defs_consistently() {
    let out = session(&[req(1, "tools/list", json!({}))]);
    for t in out[0]["result"]["tools"].as_array().unwrap() {
        let schema = &t["inputSchema"];
        let mut found = Vec::new();
        refs(schema, &mut found);
        for r in found {
            let def = r
                .strip_prefix("#/$defs/")
                .unwrap_or_else(|| panic!("{}: ref {r} is not to $defs", t["name"]));
            assert!(
                schema["$defs"][def].is_object(),
                "{}: ref {r} does not resolve",
                t["name"]
            );
        }
        assert!(schema.get("$schema").is_none(), "{}", t["name"]);
    }
    let transact = &out[0]["result"]["tools"][1]["inputSchema"];
    assert_eq!(transact["properties"]["commands"]["minItems"], 1);
    assert_eq!(transact["required"], json!(["commands"]));
}

/// The Anthropic API refuses a tool whose input schema has `anyOf`, `oneOf` or `allOf` at its
/// root, and Claude Code and Claude Desktop pass MCP tool schemas on to it.
#[test]
fn tool_schemas_have_plain_object_roots() {
    let out = session(&[req(1, "tools/list", json!({}))]);
    for t in out[0]["result"]["tools"].as_array().unwrap() {
        let schema = &t["inputSchema"];
        for key in ["anyOf", "oneOf", "allOf"] {
            assert!(schema.get(key).is_none(), "{}: root {key}", t["name"]);
        }
        assert_eq!(schema["type"], "object", "{}", t["name"]);
        assert!(schema["properties"].is_object(), "{}", t["name"]);
    }
}

#[test]
fn the_query_tool_names_every_query_type() {
    let out = session(&[req(1, "tools/list", json!({}))]);
    let query = &out[0]["result"]["tools"][2];
    assert_eq!(query["name"], "query");
    let schema = &query["inputSchema"];
    assert_eq!(schema["required"], json!(["type"]));
    let types: Vec<_> = schema["properties"]["type"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    // The variants of session's Query and the API's EditorQuery, as describe's query schema has them.
    let api_out = session(&[tool(1, "describe", json!({"schema": "query"}))]);
    let params = &api_out[0]["result"]["structuredContent"]["params_schema"];
    let mut expected = Vec::new();
    for part in params["anyOf"].as_array().unwrap() {
        for variant in part["oneOf"].as_array().unwrap() {
            expected.push(variant["properties"]["type"]["const"].as_str().unwrap());
        }
    }
    assert_eq!(types, expected);
    for name in ["entities", "selection", "camera", "pick", "infer"] {
        assert!(types.contains(&name), "{name}");
    }
}

#[test]
fn a_lost_connection_is_a_tool_error() {
    struct Lost;
    impl api::mcp::Backend for Lost {
        fn call(&mut self, _: &str, _: Value) -> Result<Value, api::RpcError> {
            Err(api::RpcError::new(
                codes::CONNECTION_LOST,
                "the connection to the app was lost",
            ))
        }
    }
    let input = [
        init(1, "2025-06-18"),
        tool(2, "query", json!({"type": "stats"})),
        req(3, "ping", json!({})),
    ]
    .map(|v| v.to_string())
    .join("\n");
    let mut out = Vec::new();
    serve_mcp(&mut Lost, input.as_bytes(), &mut out).unwrap();
    let out: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(out[1]["result"]["isError"], true);
    assert_eq!(
        out[1]["result"]["structuredContent"]["code"],
        codes::CONNECTION_LOST
    );
    assert_eq!(out[2]["result"], json!({}));
}

#[cfg(unix)]
#[test]
fn a_socket_backend_maps_replies_and_stays_lost() {
    use api::Endpoint;
    use api::mcp::{Backend, SocketBackend};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    let dir = std::env::temp_dir().join(format!("skmcpb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = UnixListener::bind(&path).unwrap();
    // A fake app: a notification and a stray reply before each real one, then it hangs up.
    let app = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut lines = BufReader::new(stream).lines();
        for reply in [
            json!({"result": {"ok": true}}),
            json!({"error": {"code": -32000, "message": "no", "data": {"kind": "not_found"}}}),
        ] {
            let request: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            let mut reply = reply;
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = request["id"].clone();
            writeln!(writer, "{}", json!({"jsonrpc": "2.0", "method": "changed"})).unwrap();
            writeln!(
                writer,
                "{}",
                json!({"jsonrpc": "2.0", "id": 999, "result": 0})
            )
            .unwrap();
            writeln!(writer, "{reply}").unwrap();
        }
    });
    let mut backend = SocketBackend::connect(&Endpoint::Socket(path.clone())).unwrap();
    assert_eq!(
        backend.call("query", json!({"type": "stats"})).unwrap(),
        json!({"ok": true})
    );
    let e = backend.call("execute", json!({})).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (-32000, "no"));
    assert_eq!(e.data, Some(json!({"kind": "not_found"})));
    app.join().unwrap();
    for _ in 0..2 {
        let e = backend.call("query", json!({"type": "stats"})).unwrap_err();
        assert_eq!(e.code, codes::CONNECTION_LOST);
        assert_eq!(
            e.message,
            format!("the connection to {} closed", path.display())
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn transact_accepts_references() {
    let out = session(&[
        tool(
            1,
            "transact",
            json!({"commands": [
                {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 3, 0]},
                {"type": "push_pull", "face": "$0:f0", "distance": 1}
            ]}),
        ),
        req(2, "tools/list", json!({})),
    ]);
    let r = &out[0]["result"];
    assert_eq!(r["isError"], false, "{r}");
    let faces = r["structuredContent"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with('f'))
        .count();
    assert_eq!(faces, 6, "{r}");
    let transact = out[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "transact")
        .unwrap();
    assert!(transact["inputSchema"]["$defs"]["BatchRef"].is_object());
}
