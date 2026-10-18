# M12a MCP Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An MCP server on stdio (`app --mcp`, headless by default, `--live SOCKET` to drive a running window) exposing the agent API as seven tools, plus the API changes it needs: undo origins and `expect`, net batch `changes`, and per-method schemas in `describe`.

**Architecture:** A transport-free MCP layer in the `api` crate (`api::mcp`) talks to a `Backend` trait whose one method runs a JSON-RPC method and returns its result or error. The app supplies two backends: a local one wrapping the headless `Editor` through `AppHost` and `api::call` (the same dispatch the socket uses), and `api::mcp::SocketBackend`, which forwards lines to a live socket. The MCP layer only maps tools onto methods, fills argument defaults and shapes results.

**Tech Stack:** Rust 2024, serde_json =1.0.151, schemars =1.2.2. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`, section 5 "MCP" and the M12a bullet under "Agent integration milestones".

## Global Constraints

- No new crates. `api` keeps depending on `session`, `tools`, `kernel` and `render` only.
- Nothing but MCP messages may be written to stdout in `--mcp` mode. Diagnostics go to stderr (`eprintln!`, `log`).
- Units are meters, Z up, angles in degrees. Model coordinates are bounded by `session::MAX_COORDINATE` (1.0e6).
- Existing JSON-RPC behaviour is unchanged except the additions listed in the spec (new fields, new error kind, new describe keys). Every existing test keeps passing.
- Gate per task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Use `export PATH="$HOME/.cargo/bin:$PATH"`.
- Docs: `docs/agent-api.md` is updated in the task that changes the API. No emoji. Commit messages short, no attribution lines.
- Never `git add -A`; stage the files you changed.

## Spec notes (rulings)

1. Origins are `user` and `agent`. Every API connection, and the MCP server, is `agent`; connections are not told apart.
2. `history` keeps `undo` and `redo` as label strings and adds `undo_origins` and `redo_origins`, aligned with them, so existing clients keep working.
3. `undo` takes `expect: {label, origin?}`. It checks only the first step `undo` would take, also with `count`. On mismatch: `ErrorKind::UnexpectedStep` (`unexpected_step`), nothing undone. `redo` gets no `expect`.
4. A step keeps its origin through undo and redo. `new` and `open` do not reset the document's current origin.
5. Net changes compose per entity from the first "existed before" to the last "exists after", like `ChangeSet::summary`. They are computed from the outcomes' summaries, so a failed non-atomic batch reports no net changes (its error is unchanged).
6. `describe` keeps its existing string `params`/`result`/`notes` and adds `params_schema` and `result_schema` (JSON Schema objects or null). `schema_version` becomes 2.
7. Coordinate bounds go only on fields the executor checks with `check_coordinates` (points, centers, eyes, targets); directions, normals and vectors stay unbounded.
8. MCP `transact`: one command is sent to `execute` (so editor commands, a screen-box select, undo, save and open work); two or more go to `batch` with `atomic` defaulting to true. The result is always `{outcomes, changes, revision}`, or `{editor, revision}` for a lone editor command.
9. MCP `render_view` defaults to 960 x 600 when `width` or `height` is left out.
10. MCP tool results: always a text block with the compact JSON. `structuredContent` (the same JSON object) only when the negotiated protocol version is 2025-06-18 or later. No `outputSchema`.
11. API failures (command, batch, render unavailable, invalid params, connection lost) are tool results with `isError: true` and the error object `{code, message, data?}`. An unknown tool name or a `tools/call` without a `name` is a JSON-RPC error -32602.
12. Before `initialize`, every request except `ping` gets JSON-RPC error -32600 "call initialize first". A second `initialize` is answered again (idempotent).
13. A live socket that closes makes every later call fail with code -32003 (`codes::CONNECTION_LOST`); the server keeps answering until stdin closes.
14. `--mcp` with `--headless`, `--stdio` or `--socket` is an argument error; `--live` needs `--mcp` and refuses a FILE or `--api-root`.

## Review Focus

1. A stray `println!` or log line on stdout in `--mcp` mode breaks every client: the e2e test parses every stdout line as JSON-RPC (Task 5).
2. Notifications (`notifications/initialized`, `notifications/cancelled`) must get no reply, and requests before `initialize` must not crash the server (Task 4 tests).
3. The live window quitting mid-session: later tool calls return `isError` with -32003 and the server exits cleanly at stdin EOF, without hanging (Task 5 test).
4. An atomic `transact` failing partway leaves the model unchanged and reports the failing index in the error data (Task 4 and Task 5 tests).
5. An `undo` with `expect` in a shared window must refuse when a person's step is on top, even when the labels match but the origins differ (Task 1 and Task 2 tests).

---

### Task 1: Undo origins, `expect`, and net change summaries

**Files:**
- Modify: `crates/kernel/src/model.rs` (`ChangeSummary::join`)
- Modify: `crates/session/src/command.rs` (`Origin`, `ExpectedStep`, `Undo.expect`, `ErrorKind::UnexpectedStep`)
- Modify: `crates/session/src/document.rs` (`Step.origin`, `Document::origin`/`set_origin`, history, undo check)
- Modify: `crates/session/src/query.rs` (`QueryResult::History` origins)
- Modify: `crates/session/src/lib.rs` (re-exports)
- Test: `crates/session/tests/history_origin.rs` (new), `crates/kernel/src/model.rs` unit tests
- Docs: `docs/agent-api.md` (history query, undo, error kinds)

**Interfaces:**
- Produces:
  - `kernel::ChangeSummary::join<'a>(summaries: impl IntoIterator<Item = &'a ChangeSummary>) -> ChangeSummary`
  - `session::Origin` (`#[serde(rename_all = "snake_case")]`, variants `User` (default) and `Agent`), `Copy`, `JsonSchema`
  - `session::ExpectedStep { label: String, #[serde(default)] origin: Option<Origin> }`
  - `Command::Undo { count: Option<u32>, #[serde(default)] expect: Option<ExpectedStep> }`
  - `ErrorKind::UnexpectedStep`
  - `Document::set_origin(&mut self, origin: Origin)`, `Document::origin(&self) -> Origin`
  - `QueryResult::History { .., undo_origins: Vec<Origin>, redo_origins: Vec<Origin>, .. }`

- [ ] **Step 1: Write the failing tests**

In `crates/kernel/src/model.rs` tests:

```rust
#[test]
fn summaries_join_to_the_net_change() {
    let a = ChangeSummary { created: vec![v(1), v(2)], deleted: vec![v(9)], modified: vec![v(5)], materials: vec![] };
    let b = ChangeSummary { created: vec![v(3)], deleted: vec![v(2), v(5)], modified: vec![v(1)], materials: vec![] };
    let net = ChangeSummary::join([&a, &b]);
    assert_eq!(net.created, vec![v(1), v(3)]); // made then changed is made; made then erased is gone
    assert_eq!(net.deleted, vec![v(5), v(9)]);
    assert!(net.modified.is_empty());
}
```

(`v(n)` builds a vertex `EntityId`; use whatever constructor the module's tests already use. Materials join the same way.)

In `crates/session/tests/history_origin.rs`:

```rust
#[test]
fn steps_carry_the_origin_they_were_made_with() {
    let mut doc = Document::new(DVec2::new(800.0, 600.0));
    doc.execute(edge([0., 0., 0.], [1., 0., 0.])).unwrap();       // user (default)
    doc.set_origin(Origin::Agent);
    doc.execute(edge([0., 0., 0.], [0., 1., 0.])).unwrap();
    let QueryResult::History { undo, undo_origins, .. } = doc.query(&Query::History {}).unwrap() else { panic!() };
    assert_eq!(undo, ["add_edge", "add_edge"]);
    assert_eq!(undo_origins, [Origin::Agent, Origin::User]);
    doc.execute(Command::Undo { count: None, expect: None }).unwrap();
    let QueryResult::History { redo_origins, .. } = doc.query(&Query::History {}).unwrap() else { panic!() };
    assert_eq!(redo_origins, [Origin::Agent]);
}

#[test]
fn undo_refuses_an_unexpected_step() {
    // a user step on top; the agent expects its own add_edge
    let mut doc = Document::new(DVec2::new(800.0, 600.0));
    doc.set_origin(Origin::Agent);
    doc.execute(edge([0., 0., 0.], [1., 0., 0.])).unwrap();
    doc.set_origin(Origin::User);
    doc.execute(edge([0., 0., 0.], [0., 1., 0.])).unwrap();
    let expect = |label: &str, origin| Some(ExpectedStep { label: label.into(), origin });
    let e = doc.execute(Command::Undo { count: None, expect: expect("add_edge", Some(Origin::Agent)) }).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnexpectedStep);
    assert!(e.message.contains("add_edge") && e.message.contains("user"));
    let e = doc.execute(Command::Undo { count: None, expect: expect("push_pull", None) }).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnexpectedStep);
    assert_eq!(doc.query(&Query::Stats {}).unwrap().revision(), 2); // nothing undone
    doc.execute(Command::Undo { count: None, expect: expect("add_edge", Some(Origin::User)) }).unwrap();
}

#[test]
fn new_and_open_keep_the_current_origin() {
    let mut doc = Document::new(DVec2::new(800.0, 600.0));
    doc.set_origin(Origin::Agent);
    doc.execute(Command::New { discard: true }).unwrap();
    assert_eq!(doc.origin(), Origin::Agent);
}
```

(Write `edge(..)` as a small helper returning `Command::AddEdge`. If `QueryResult` has no `revision()` helper, match `QueryResult::Stats { revision, .. }`.) Also: `undo` with `expect` and nothing to undo still fails `nothing_to_undo`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p kernel summaries_join && cargo test -p session --test history_origin`
Expected: compile errors (`join`, `Origin`, `expect` missing).

- [ ] **Step 3: Implement**

- `ChangeSummary::join`: per id, `(before, after)` from each summary (created: false→true, deleted: true→false, modified: true→true); keep the first `before` and the last `after`; output sorted like `ChangeSet::summary`. The same for `materials`, where any listed material counts as before = after = true unless the next summary says otherwise. Keep it simple: a material listed anywhere is listed in the net result.
- `Step` gets `origin: Origin`, stamped from `self.origin` in `record`. Undo and redo move steps between stacks unchanged.
- `Document` gets an `origin` field (default `User`); `new_model` and `open_file` must not reset it.
- In `Command::Undo`, before popping: if `expect` is given and the top step's label differs, or `expect.origin` is given and differs, return `UnexpectedStep` with the message `the next step to undo is "<label>" (by the <origin>), not "<expected label>"`. An empty stack is still `nothing_to_undo`.
- `History` lists `undo_origins` and `redo_origins` in the same order as `undo`/`redo`.
- Fix every other `Command::Undo { count }` pattern and construction in the workspace (`grep -rn "Undo {" crates`).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p kernel && cargo test -p session`
Expected: PASS.

- [ ] **Step 5: Docs**

`docs/agent-api.md`: in the `history` row add `undo_origins`/`redo_origins` (`user`: made in the window; `agent`: made through the API); in the undo/redo paragraph document `expect` with an example `{"type": "undo", "expect": {"label": "push_pull", "origin": "agent"}}` and the `unexpected_step` error; add `unexpected_step` wherever error kinds are listed.

- [ ] **Step 6: Gate and commit**

```bash
git add crates/kernel/src/model.rs crates/session/src crates/session/tests/history_origin.rs docs/agent-api.md
git commit -m "Session: undo step origins, undo expect, net change summaries"
```

---

### Task 2: The API tags agent steps and reports net batch changes

**Files:**
- Modify: `crates/app/src/editor.rs` (`Editor::set_origin`)
- Modify: `crates/app/src/host.rs` (tag `Agent` around every mutating `Host` method)
- Modify: `crates/api/src/headless.rs` (`HeadlessHost` documents start as `Agent`)
- Modify: `crates/api/src/dispatch.rs` (`ExecuteResult`, `BatchResult`, net `changes` on `batch`)
- Test: `crates/api/tests/dispatch.rs`, `crates/app/src/host.rs` tests (or the existing host test module)
- Docs: `docs/agent-api.md` (`batch` result)

**Interfaces:**
- Consumes: `Document::set_origin`, `Origin`, `ChangeSummary::join` (Task 1).
- Produces:
  - `api::ExecuteResult { #[serde(flatten)] outcome: Outcome, revision: u64 }` (`Serialize`, `JsonSchema`)
  - `api::BatchResult { outcomes: Vec<Outcome>, changes: ChangeSummary, revision: u64 }` (`Serialize`, `JsonSchema`)
  - `Editor::set_origin(&mut self, origin: Origin)`

- [ ] **Step 1: Write the failing tests**

`crates/api/tests/dispatch.rs`:

```rust
#[test]
fn batch_reports_the_net_changes() {
    let mut h = HeadlessHost::new();
    let r = call(&mut h, "batch", json!({"commands": square(), "atomic": true}));
    let net = &r["result"]["changes"];
    assert_eq!(net["created"].as_array().unwrap().len(), 4 + 4 + 1); // 4 vertices, 4 edges, the face
    assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 4);
}

#[test]
fn api_steps_are_agent_steps() {
    let mut h = HeadlessHost::new();
    call(&mut h, "execute", json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}));
    let r = call(&mut h, "query", json!({"type": "history"}));
    assert_eq!(r["result"]["undo_origins"], json!(["agent"]));
}
```

In the app host tests: an edit through `AppHost::execute` is `agent`, and an edit through `Editor::execute` directly afterwards (as the window makes it) is `user`; the same for a `tool_input` that draws a line (`agent`) and for a non-atomic API batch `[new {discard: true}, add_edge]` (the edge step is `agent`). Then an `undo` with `expect {label: "add_edge", origin: "agent"}` through the API refuses because the user's step is on top.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p api --test dispatch && cargo test -p app host`
Expected: FAIL (`changes` null, origins `user`).

- [ ] **Step 3: Implement**

- `Editor::set_origin` forwards to its document.
- `AppHost`: `execute`, `execute_batch`, `editor_command` and `tool_input` set `Origin::Agent` before running and `Origin::User` after, on success and failure alike (a small helper that takes a closure). The window never sets `Agent`.
- `HeadlessHost::new` sets the document's origin to `Agent` (everything in it comes through the API).
- `dispatch.rs`: `execute` serializes `ExecuteResult`; `batch` returns `BatchResult` with `changes: ChangeSummary::join(outcomes.iter().map(|o| &o.changes))`. Export both from `lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p api && cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Docs, gate and commit**

`docs/agent-api.md`: `batch` result is `{outcomes, changes, revision}`, `changes` being the net effect of the whole batch; a note that steps made through the API are `agent` steps. Then:

```bash
git add crates/app/src/editor.rs crates/app/src/host.rs crates/api/src crates/api/tests/dispatch.rs docs/agent-api.md
git commit -m "API: agent origin on API steps, net changes in batch results"
```

---

### Task 3: `describe` gives per-method schemas, coordinate bounds and object roots

**Files:**
- Modify: `crates/session/src/command.rs`, `crates/session/src/query.rs` (coordinate schema attribute on point fields)
- Create: `crates/session/src/schema.rs` (`ModelPoint` schema-only type)
- Create: `crates/api/src/schema.rs` (`object_root`, `hoist`, `any_of_root`)
- Modify: `crates/api/src/dispatch.rs` (`describe`, `schema_version` 2, object roots, `params_schema`/`result_schema`)
- Modify: `crates/api/src/input.rs`, `crates/api/src/probe.rs` (point fields that are model points)
- Test: `crates/api/tests/dispatch.rs`
- Docs: `docs/agent-api.md` (describe section)

**Interfaces:**
- Consumes: `ExecuteResult`, `BatchResult` (Task 2).
- Produces:
  - `session::ModelPoint` (schema only; `impl JsonSchema` giving `{"type": "array", "items": {"type": "number", "minimum": -1e6, "maximum": 1e6}, "minItems": 3, "maxItems": 3}`), used as `#[schemars(with = "ModelPoint")]` and `#[schemars(with = "Option<ModelPoint>")]`
  - `api::schema::object_root(schema: Value) -> Value`: the schema as JSON with `"type": "object"` at its root (beside `oneOf`/`anyOf`), unchanged otherwise
  - `api::schema::hoist(root_defs: &mut Map<String, Value>, schema: Value) -> Value`: moves the schema's `$defs` into `root_defs` (an existing name must have identical content, else panic) and returns the rest without `$schema`, `$defs` or `title`, its `#/$defs/<Name>` refs now resolving against the root
  - `api::schema::any_of_root(schemas: Vec<Value>) -> Value`: `{"type": "object", "anyOf": [...], "$defs": {...}}` built with `hoist`
  - `describe()["methods"][m]["params_schema"]` and `["result_schema"]` for every method

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn describe_has_schemas_per_method() {
    let d = api::describe();
    assert_eq!(d["schema_version"], 2);
    for m in ["execute", "batch", "query", "render_view", "tool_input", "quit"] {
        assert!(d["methods"][m]["params_schema"].is_object(), "{m}");
        assert!(d["methods"][m]["result_schema"].is_object(), "{m}");
    }
    let exec = d["methods"]["execute"]["result_schema"].to_string();
    assert!(exec.contains("\"revision\""));
    assert!(d["methods"]["batch"]["result_schema"].to_string().contains("\"changes\""));
}

#[test]
fn command_schemas_have_an_object_root_and_bounded_points() {
    let d = api::describe();
    for name in ["Command", "Query", "EditorCommand", "EditorQuery", "ScreenCommand"] {
        assert_eq!(d["schemas"][name]["type"], "object", "{name}");
    }
    let add_edge = d["schemas"]["Command"]["oneOf"].as_array().unwrap().iter()
        .find(|v| v["properties"]["type"]["const"] == "add_edge").unwrap();
    assert_eq!(add_edge["properties"]["start"]["items"]["minimum"], -1e6);
    assert_eq!(add_edge["properties"]["start"]["items"]["maximum"], 1e6);
}
```

(Adjust the `oneOf` walk to how schemars 1.2.2 lays out the internally tagged enum: `const` or a one-value `enum` on `type`; resolve a `$ref` if the variant is referenced. The assertions on bounds and root type stay.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p api --test dispatch describe`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `ModelPoint` in `session::schema`; put `#[schemars(with = ...)]` on every field whose value the executor passes to `check_coordinates` (find them by following `check_coordinates` calls in `document.rs` and the files it calls into), and on model points in `PickQuery`/`InferQuery`/`InputEvent` (`point`, `from`, ray `origin`) in `api`. Directions, normals and move vectors stay `[f64; 3]`.
- `object_root`: serialize the schema, insert `"type": "object"` at the root when absent.
- `describe`: `schema_version: 2`; `schemas` entries for Command, Query, EditorCommand, EditorQuery and ScreenCommand go through `object_root`; add `ExecuteResult` and `BatchResult` schemas; each method gets `params_schema` and `result_schema`. `execute`'s params schema is `any_of_root(vec![Command, ScreenCommand, EditorCommand])` and `query`'s `any_of_root(vec![Query, EditorQuery])`; a ref like `#/schemas/<Name>` is not valid JSON Schema, so every params and result schema is self-contained with its `$defs` at its own root. Test that every `$ref` in every `params_schema`/`result_schema` resolves in that schema's own `$defs`. `subscribe` and `describe` take null params: `params_schema: null`. Quit: `{"type": "object", "properties": {"discard": {"type": "boolean"}}}`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p session && cargo test -p api`
Expected: PASS.

- [ ] **Step 5: Docs, gate and commit**

`docs/agent-api.md`: describe section lists `params_schema`/`result_schema`, schema version 2, object roots, coordinate bounds.

```bash
git add crates/session/src crates/api/src crates/api/tests/dispatch.rs docs/agent-api.md
git commit -m "API: per-method schemas in describe, coordinate bounds, object roots"
```

---

### Task 4: The MCP protocol layer (`api::mcp`)

**Files:**
- Create: `crates/api/src/mcp/mod.rs` (`Backend`, `serve_mcp`, lifecycle and method routing)
- Create: `crates/api/src/mcp/tools.rs` (tool list with input schemas, argument mapping, result shaping)
- Modify: `crates/api/src/lib.rs` (`pub mod mcp`; `pub use protocol::RpcError`; `pub use dispatch::call`)
- Modify: `crates/api/src/dispatch.rs` (`call` becomes `pub`)
- Modify: `crates/api/src/protocol.rs` (`codes::CONNECTION_LOST = -32003`)
- Test: `crates/api/tests/mcp.rs` (new, with a `HeadlessHost`-backed backend)

**Interfaces:**
- Consumes: `api::call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcError>`, `describe()`, `schema::{object_root, hoist, any_of_root}` (Task 3).
- Produces:
  - `pub trait Backend { fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError>; }`
  - `pub struct HostBackend<'a>(pub &'a mut dyn Host)` implementing `Backend` via `api::call`
  - `pub fn serve_mcp(backend: &mut dyn Backend, input: impl BufRead, output: impl Write) -> io::Result<()>`: one JSON-RPC message per line; returns at EOF
  - `pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]`

Tools (name: arguments → method; annotations):

| Tool | Arguments | Forwards to | readOnlyHint |
|---|---|---|---|
| `describe` | `{schema?: string}` | `describe`; without `schema`: the document minus `schemas`, plus `schema_names`; with it: that one schema (unknown name: `isError`) | true |
| `transact` | `{commands: [Command \| ScreenCommand \| EditorCommand] (minItems 1), atomic?: bool}` | 1 command: `execute`; more: `batch` with `atomic` default true | false |
| `query` | a `Query` or `EditorQuery` object | `query` | true |
| `pick` | a `PickQuery` object | `query` with `type: "pick"` | true |
| `infer` | an `InferQuery` object | `query` with `type: "infer"` | true |
| `render_view` | `{width?, height?, camera?, lighting?}` | `render_view`, sides default 960 x 600 | true |
| `tool_input` | `ToolInputParams` | `tool_input` | false |

Each `inputSchema` is built with Task 3's `schema::object_root`, `hoist` and `any_of_root`, so every `$ref` resolves in the tool schema's root `$defs`. Descriptions, one sentence each, say what the tool does and mention meters, Z up and `describe` for schemas.

`initialize` result: `{protocolVersion, capabilities: {tools: {listChanged: false}}, serverInfo: {name: "sketchup-alt", version: env!("CARGO_PKG_VERSION")}, instructions}` with this `instructions` text:

> A SketchUp-style 3D modeler. Units are meters, Z is up, angles are in degrees. Change the model with transact (several commands run as one atomic undo step by default), read it with query, see it with render_view, and drive the drawing tools like a person with tool_input. Call describe for the full command and query schemas.

- [ ] **Step 1: Write the failing tests** (`crates/api/tests/mcp.rs`, driving `serve_mcp` over in-memory buffers with `HostBackend(&mut HeadlessHost::new())`)

```rust
fn run(lines: &[Value]) -> Vec<Value>  // feeds the lines, returns every output line parsed

#[test]
fn handshake_lists_seven_tools() {
    let out = run(&[init(1, "2025-06-18"), notif("notifications/initialized"), req(2, "tools/list", json!({}))]);
    assert_eq!(out.len(), 2); // the notification got no reply
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "sketchup-alt");
    let names: Vec<_> = out[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["describe", "transact", "query", "pick", "infer", "render_view", "tool_input"]);
    for t in out[1]["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[test]
fn an_unknown_version_gets_the_newest() { /* "1999-01-01" -> "2025-11-25" */ }

#[test]
fn requests_before_initialize_are_refused_but_ping_is_not() { /* tools/list -> error -32600; ping -> {} */ }

#[test]
fn transact_runs_one_atomic_step_and_reports_net_changes() {
    // square() through transact: result.structuredContent.changes.created has 9 ids, isError false,
    // and query history shows undo_depth 1 with origin "agent".
}

#[test]
fn a_failed_atomic_transact_changes_nothing() {
    // two good add_edge then an add_edge with a non-finite point: isError true,
    // structuredContent.code == -32001, data.index == 2, data.rolled_back == true; stats revision 0.
}

#[test]
fn a_lone_editor_command_goes_through_execute() {
    // transact {commands: [{"type": "set_tool", "tool": "line"}]} -> structuredContent.editor.tool == "line"
}

#[test]
fn render_view_returns_an_image_block() {
    // skip if the reply is isError with code -32002 (no GPU); else content[0].type == "image",
    // mimeType "image/png", data decodes to a PNG of 960 x 600 (read the IHDR), content[1] is text
}

#[test]
fn structured_content_only_for_newer_clients() { /* 2024-11-05: no structuredContent key */ }

#[test]
fn an_unknown_tool_is_a_protocol_error() { /* tools/call name "nope" -> error -32602 */ }

#[test]
fn bad_arguments_are_a_tool_error() { /* query {"type": "nope"} -> isError, code -32602 */ }

#[test]
fn tool_schemas_hoist_defs_consistently() { /* every $ref in each inputSchema resolves in its $defs */ }
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p api --test mcp`
Expected: compile errors (`api::mcp` missing).

- [ ] **Step 3: Implement** `mod.rs` and `tools.rs` per the Interfaces block and the rulings 8-12. Parse lines with `serde_json`; a line that is not an object, or a JSON array, gets -32600 with a null id; a message with `result` or `error` and no `method` (a client's reply) is ignored. Unknown methods: -32601.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p api`
Expected: PASS.

- [ ] **Step 5: Gate and commit**

```bash
git add crates/api/src crates/api/tests/mcp.rs
git commit -m "MCP: protocol layer and the seven tools over a backend"
```

---

### Task 5: `app --mcp` (headless and `--live`), end-to-end tests, setup docs

**Files:**
- Modify: `crates/app/src/cli.rs` (`Mode::Mcp`, USAGE, argument rules)
- Create: `crates/app/src/mcp.rs` (`LocalBackend`, `run`)
- Modify: `crates/app/src/main.rs` (dispatch `Mode::Mcp`)
- Create: `crates/api/src/mcp/socket.rs` (`SocketBackend`)
- Test: `crates/app/tests/mcp_stdio.rs` (new), `crates/app/src/cli.rs` unit tests
- Docs: create `docs/mcp.md`; link it from `docs/agent-api.md` and `README` if one mentions the API

**Interfaces:**
- Consumes: `serve_mcp`, `Backend`, `api::call` (Task 4); `AppHost`, `Offscreen`, `Editor` (app).
- Produces:
  - `cli::Mode::Mcp { live: Option<PathBuf>, file: Option<PathBuf>, api_roots: Vec<PathBuf> }`
  - `api::mcp::SocketBackend::connect(path: &Path) -> io::Result<SocketBackend>`, implementing `Backend`: writes one request line with a fresh numeric id, reads lines until the reply with that id (skipping notifications), maps a JSON-RPC `error` to `RpcError`, and any I/O failure or EOF to `codes::CONNECTION_LOST` with the message `the connection to <path> closed` (and keeps failing afterwards)
  - `mcp::run(live: Option<PathBuf>, file: Option<PathBuf>, roots: ApiRoots) -> i32`

- [ ] **Step 1: Write the failing tests**

`cli.rs`:

```rust
#[test]
fn mcp_modes_parse() {
    assert_eq!(parse(args("--mcp")).unwrap(), Mode::Mcp { live: None, file: None, api_roots: vec![] });
    assert_eq!(parse(args("--mcp --api-root /a f.skr")).unwrap(),
        Mode::Mcp { live: None, file: Some("f.skr".into()), api_roots: vec!["/a".into()] });
    assert_eq!(parse(args("--mcp --live /tmp/w.sock")).unwrap(),
        Mode::Mcp { live: Some("/tmp/w.sock".into()), file: None, api_roots: vec![] });
    assert!(parse(args("--live /tmp/w.sock")).unwrap_err().contains("--mcp"));
    assert!(parse(args("--mcp --live /tmp/w.sock f.skr")).is_err());
    assert!(parse(args("--mcp --live /tmp/w.sock --api-root /a")).is_err());
    assert!(parse(args("--mcp --headless")).is_err());
    assert!(parse(args("--mcp --socket /tmp/x.sock")).is_err());
}
```

`crates/app/tests/mcp_stdio.rs` (spawn `env!("CARGO_BIN_EXE_app")`, same thread-and-timeout pattern as `headless_stdio.rs`; every stdout line must parse as a JSON-RPC message with `"jsonrpc": "2.0"`):

```rust
#[test]
fn mcp_headless_session() {
    // initialize, notifications/initialized, tools/list (7 tools), transact square() (9 created),
    // query {"type": "stats"} (faces 1), close stdin: exit status 0, exactly 4 replies.
}

#[test]
fn mcp_opens_a_file_given() { /* save a model with the headless stdio app first, then --mcp FILE: stats faces 1 */ }

#[test]
fn mcp_live_forwards_to_a_running_instance() {
    // start `app --headless --socket <scratch>/w.sock`, wait for the socket file;
    // `app --mcp --live <socket>`: transact an edge; then query the headless instance over
    // the socket directly: one edge, history undo_origins ["agent"].
    // Kill the headless instance, call query through MCP again: isError, code -32003;
    // close stdin: the MCP process exits 0 within the timeout.
}

#[test]
fn mcp_live_to_a_missing_socket_fails_at_start() { /* exit status 1, message on stderr */ }
```

(Scratch paths: `std::env::temp_dir()` plus the process id, short enough for a socket; remove them at the end.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p app --test mcp_stdio && cargo test -p app cli`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `cli.rs`: `--mcp` and `--live PATH` flags with the rules of ruling 14; USAGE gains two lines:
  `app --mcp [--api-root DIR]... [FILE]                MCP server on stdin/stdout with its own model`
  `app --mcp --live SOCKET                             MCP server driving the app listening on SOCKET`
- `crates/app/src/mcp.rs`: `LocalBackend { editor: Editor, offscreen: Offscreen, roots: ApiRoots, quit: bool }` whose `call` builds an `AppHost { recovery: None, .. }` and runs `api::call`. `run` opens `file` like `headless::run` (on failure: stderr, return 1), or connects `SocketBackend` (on failure: `cannot connect to <path>: <e>` on stderr, return 1), then `serve_mcp(backend, stdin.lock(), stdout.lock())` and returns 0.
- `main.rs`: `Mode::Mcp` resolves roots with `roots(..)` and calls `mcp::run`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Docs**

`docs/mcp.md`: what it is (one paragraph); the seven tools with one line each; headless setup for Claude Code:

```bash
claude mcp add sketchup-alt -- /path/to/app --mcp --api-root ~/Models
```

and for Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{"mcpServers": {"sketchup-alt": {"command": "/path/to/app", "args": ["--mcp", "--api-root", "/Users/you/Models"]}}}
```

the live setup (start the window with a fixed `--socket ~/.sketchup-alt/main.sock`, then `--mcp --live ~/.sketchup-alt/main.sock`); notes: undo with `expect` in a shared window, `--api-root` for headless file access, how to build (`cargo build --release -p app`).

- [ ] **Step 6: Gate and commit**

```bash
git add crates/app/src/cli.rs crates/app/src/mcp.rs crates/app/src/main.rs crates/api/src/mcp/socket.rs crates/api/src/mcp/mod.rs crates/app/tests/mcp_stdio.rs docs/mcp.md docs/agent-api.md
git commit -m "MCP: app --mcp, headless and live, with setup docs"
```
