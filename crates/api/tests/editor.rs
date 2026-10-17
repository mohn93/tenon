//! Editor commands, the `editor` query and `quit` over JSON-RPC, against a host with a simple
//! editor and against one without.

use api::{
    EditorCommand, EditorState, HeadlessHost, Host, Input, MessageKind, Mods, PhaseName, ToolInput,
    ToolInputError, ToolInputResult, ToolName, codes, describe, handle,
};
use serde_json::{Value, json};
use session::{
    BatchError, Camera, Command, CommandError, Document, ImportUnits, Lighting, Outcome,
};

/// A document with an editor that only records what it was told.
struct Editing {
    inner: HeadlessHost,
    state: EditorState,
    quit: bool,
    /// The events `tool_input` was given.
    events: Vec<Input>,
}

impl Editing {
    fn new() -> Self {
        Editing {
            inner: HeadlessHost::new(),
            state: EditorState {
                tool: ToolName::Line,
                phase: PhaseName::Idle,
                points: Vec::new(),
                paint_material: None,
                import_units: ImportUnits::Meters,
                flying: false,
                fly_speed: 3.0,
                viewport: [1280.0, 800.0],
                status: "Line  |  Click to start a line".into(),
                prompt: "Click to start a line".into(),
                message: None,
                message_kind: None,
                lock: None,
                measurement_label: "Length".into(),
                measurement: None,
                typing: false,
                soften_angle: 20,
                dialog: None,
                modifiers: Mods::default(),
                hovered: None,
                snap_label: None,
            },
            quit: false,
            events: Vec::new(),
        }
    }
}

impl Host for Editing {
    fn document(&self) -> &Document {
        self.inner.document()
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.inner.execute(command)
    }

    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.inner.execute_batch(commands, atomic)
    }

    fn render_view(&mut self, _: &Camera, _: Lighting, _: u32, _: u32) -> Option<Vec<u8>> {
        None
    }

    fn editor(&self) -> Option<EditorState> {
        Some(self.state.clone())
    }

    fn editor_command(&mut self, command: EditorCommand) -> Result<(), CommandError> {
        match command {
            EditorCommand::SetTool { tool } => self.state.tool = tool,
            EditorCommand::SetPaintMaterial { material } => self.state.paint_material = material,
            EditorCommand::SetImportUnits { units } => self.state.import_units = units,
            EditorCommand::Cancel {} => self.state.phase = PhaseName::Idle,
            EditorCommand::SetFlyMode { on, speed } => {
                self.state.flying = on;
                self.state.fly_speed = speed.unwrap_or(self.state.fly_speed);
            }
            EditorCommand::SetDialog { dialog } => self.state.dialog = dialog,
            EditorCommand::SetSoftenAngle { angle } => self.state.soften_angle = angle,
            // The window's Move tool copies; this host has none.
            EditorCommand::Duplicate {} => {}
        }
        Ok(())
    }

    fn tool_input(&mut self, input: ToolInput) -> Result<ToolInputResult, ToolInputError> {
        self.events.extend(input.events);
        Ok(ToolInputResult {
            editor: self.state.clone(),
            inference: None,
            preview: Vec::new(),
            commands: Vec::new(),
            revision: self.document().revision(),
        })
    }

    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.document().check_unsaved(discard)?;
        self.quit = true;
        Ok(())
    }

    fn quitting(&self) -> bool {
        self.quit
    }
}

fn call(host: &mut dyn Host, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

#[test]
fn editor_commands_set_the_editor_and_return_its_state() {
    let mut h = Editing::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_tool", "tool": "push_pull"}),
    );
    assert_eq!(r["result"]["editor"]["tool"], "push_pull", "{r}");
    assert_eq!(r["result"]["revision"], 0);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_import_units", "units": "millimeters"}),
    );
    assert_eq!(r["result"]["editor"]["import_units"], "millimeters");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_paint_material", "material": "m3"}),
    );
    assert_eq!(r["result"]["editor"]["paint_material"], "m3");
    let r = call(&mut h, "execute", json!({"type": "set_paint_material"}));
    assert_eq!(r["result"]["editor"]["paint_material"], Value::Null);
    let r = call(&mut h, "execute", json!({"type": "cancel"}));
    assert_eq!(r["result"]["editor"]["phase"], "idle");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_fly_mode", "on": true, "speed": 6.0}),
    );
    assert_eq!(r["result"]["editor"]["flying"], true, "{r}");
    assert_eq!(r["result"]["editor"]["fly_speed"], 6.0);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_dialog", "dialog": "soften_edges"}),
    );
    assert_eq!(r["result"]["editor"]["dialog"], "soften_edges", "{r}");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_soften_angle", "angle": 45}),
    );
    assert_eq!(r["result"]["editor"]["soften_angle"], 45, "{r}");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_dialog", "dialog": null}),
    );
    assert_eq!(r["result"]["editor"]["dialog"], Value::Null, "{r}");
    let r = call(&mut h, "execute", json!({"type": "duplicate"}));
    assert_eq!(r["result"]["editor"]["tool"], "push_pull", "{r}");

    let state = call(&mut h, "query", json!({"type": "editor"}))["result"].clone();
    assert_eq!(state["type"], "editor");
    assert_eq!(state["tool"], "push_pull");
    assert_eq!(state["viewport"], json!([1280.0, 800.0]));
    assert_eq!(state["dialog"], Value::Null);
    assert_eq!(state["message_kind"], Value::Null, "no message: {state}");
    h.state.message = Some("Not a length: 2,x".into());
    h.state.message_kind = Some(MessageKind::Error);
    let state = call(&mut h, "query", json!({"type": "editor"}))["result"].clone();
    assert_eq!(state["message_kind"], "error", "{state}");
    h.state.message_kind = Some(MessageKind::Info);
    let state = call(&mut h, "query", json!({"type": "editor"}))["result"].clone();
    assert_eq!(state["message_kind"], "info", "{state}");
}

#[test]
fn editor_commands_and_queries_check_their_params() {
    let mut h = Editing::new();
    for (method, params) in [
        ("execute", json!({"type": "set_tool", "tool": "lasso"})),
        ("execute", json!({"type": "set_tool"})),
        (
            "execute",
            json!({"type": "set_import_units", "units": "yards"}),
        ),
        (
            "batch",
            json!({"commands": [{"type": "set_tool", "tool": "line"}]}),
        ),
        ("quit", json!({"discard": "yes"})),
        ("quit", json!([true])),
        ("execute", json!({"type": "set_fly_mode"})),
        ("execute", json!({"type": "set_dialog", "dialog": "export"})),
        (
            "execute",
            json!({"type": "set_soften_angle", "angle": 12.5}),
        ),
    ] {
        let r = call(&mut h, method, params.clone());
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{params}: {r}");
    }
}

#[test]
fn quit_refuses_unsaved_changes_unless_discarded() {
    let mut h = Editing::new();
    let r = call(&mut h, "quit", Value::Null);
    assert_eq!(r["result"], json!({}));
    assert!(h.quit);

    let mut h = Editing::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
    let r = call(&mut h, "quit", json!({}));
    assert_eq!(r["error"]["data"]["kind"], "unsaved_changes", "{r}");
    assert!(!h.quit);
    let r = call(&mut h, "quit", json!({"discard": true}));
    assert_eq!(r["result"], json!({}));
    assert!(h.quit);
}

#[test]
fn a_host_without_an_editor_says_so() {
    let mut h = HeadlessHost::new();
    for (method, params) in [
        ("execute", json!({"type": "set_tool", "tool": "line"})),
        ("query", json!({"type": "editor"})),
        ("quit", json!({"discard": true})),
    ] {
        let r = call(&mut h, method, params.clone());
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{params}: {r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input");
    }
}

#[test]
fn describe_lists_quit_and_the_editor_schemas() {
    let d = describe();
    assert!(d["methods"]["quit"].is_object());
    for name in ["selection", "camera", "editor"] {
        assert!(d["notifications"][name].is_string(), "{name}");
    }
    assert!(d["methods"]["tool_input"].is_object());
    for name in [
        "EditorCommand",
        "EditorState",
        "EditorQuery",
        "ToolInput",
        "ToolInputResult",
        "ScreenCommand",
    ] {
        assert!(d["schemas"][name].is_object(), "{name}");
    }
}

#[test]
fn subscribers_hear_when_the_editor_changes() {
    use std::time::Duration;

    let mut host = Editing::new();
    let mut hub = api::Hub::new(|| {});
    let inbox = hub.inbox();
    let (conn, rx) = inbox.open();
    let line = |id: u64, method: &str, params: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    };
    inbox.send_line(conn, line(1, "subscribe", Value::Null));
    hub.process(&mut host);
    hub.notify_state(&host);
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(rx.try_recv().is_err());

    // Changed by a request or by the program itself, it is told once per round.
    inbox.send_line(
        conn,
        line(2, "execute", json!({"type": "set_tool", "tool": "move"})),
    );
    hub.process(&mut host);
    host.state.import_units = ImportUnits::Feet;
    hub.notify_state(&host);
    let messages: Vec<Value> = rx
        .try_iter()
        .map(|l| serde_json::from_str(&l).unwrap())
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_eq!(messages[0]["id"], 2);
    assert_eq!(messages[1]["method"], "editor");
    assert_eq!(messages[1]["params"]["tool"], "move");
    assert_eq!(messages[1]["params"]["import_units"], "feet");
}

#[test]
fn quit_takes_its_params_by_name_only() {
    let mut h = Editing::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
    for params in [json!([true]), json!([])] {
        let r = call(&mut h, "quit", params.clone());
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{params}: {r}");
        assert!(!h.quit, "{params}");
    }
}

#[test]
fn argument_less_commands_and_queries_refuse_unknown_fields() {
    let mut h = Editing::new();
    let queries = [
        "editor",
        "history",
        "stats",
        "selection",
        "camera",
        "context",
        "file",
        "definitions",
        "materials",
    ];
    for t in queries {
        let r = call(&mut h, "query", json!({"type": t, "x": 1}));
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{t}: {r}");
        let r = call(&mut h, "query", json!({"type": t}));
        assert!(r["result"].is_object(), "{t}: {r}");
    }
    let commands = [
        "cancel",
        "undo",
        "redo",
        "close_instance",
        "delete_guides",
        "purge_definitions",
        "zoom_extents",
    ];
    for t in commands {
        let r = call(&mut h, "execute", json!({"type": t, "x": 1}));
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{t}: {r}");
    }
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [{"type": "zoom_extents", "x": 1}]}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
}

#[test]
fn requests_after_an_accepted_quit_are_refused() {
    use api::Hub;
    use std::sync::mpsc::Receiver;

    fn replies(rx: &Receiver<String>) -> Vec<Value> {
        rx.try_iter()
            .map(|l| serde_json::from_str(&l).unwrap())
            .collect()
    }

    let mut host = Editing::new();
    let mut hub = Hub::new(|| {});
    let inbox = hub.inbox();
    let (a, a_rx) = inbox.open();
    let (b, b_rx) = inbox.open();
    let line = |id: u64, method: &str, params: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    };
    // One round: A quits (the model is clean), then B edits, queries and subscribes.
    inbox.send_line(a, line(1, "quit", json!({})));
    inbox.send_line(
        b,
        line(
            2,
            "execute",
            json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
        ),
    );
    inbox.send_line(b, line(3, "query", json!({"type": "stats"})));
    inbox.send_line(b, line(4, "subscribe", Value::Null));
    hub.process(&mut host);

    assert!(host.quit);
    assert_eq!(replies(&a_rx)[0]["result"], json!({}));
    let b_replies = replies(&b_rx);
    assert_eq!(b_replies.len(), 3, "{b_replies:?}");
    for r in &b_replies {
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
        assert_eq!(r["error"]["message"], "the program is quitting", "{r}");
    }
    assert_eq!(host.document().revision(), 0);
    assert!(
        !host.document().is_modified(),
        "no edit that will not be saved"
    );

    // A request in a later round is refused too, and so is a second quit.
    let r = call(&mut host, "quit", json!({"discard": true}));
    assert_eq!(r["error"]["message"], "the program is quitting", "{r}");
}

#[test]
fn tool_input_reaches_the_editor_checked() {
    let mut h = Editing::new();
    let r = call(
        &mut h,
        "tool_input",
        json!({"events": [{"type": "click", "point": [1, 2, 0]}, {"type": "type", "text": "3"}]}),
    );
    assert_eq!(r["result"]["editor"]["tool"], "line", "{r}");
    assert_eq!(r["result"]["commands"], json!([]));
    assert_eq!(h.events.len(), 2);
    // A malformed event is -32602; one that makes no sense is invalid_input, and runs nothing.
    let r = call(
        &mut h,
        "tool_input",
        json!({"events": [{"type": "hover", "pixel": [1, 1]}]}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    let r = call(
        &mut h,
        "tool_input",
        json!({"events": [{"type": "move", "pixel": [1, 1]}, {"type": "key", "key": "tab"}]}),
    );
    assert_eq!(r["error"]["message"], "event 1: unknown key \"tab\"", "{r}");
    assert_eq!(h.events.len(), 2);
    // Without an editor there is nothing to drive.
    let r = call(
        &mut HeadlessHost::new(),
        "tool_input",
        json!({"events": [{"type": "click", "pixel": [1, 1]}]}),
    );
    assert_eq!(r["error"]["message"], "this program has no editor", "{r}");
}
