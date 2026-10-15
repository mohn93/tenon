//! Editor commands, the `editor` query and `quit` over JSON-RPC, against a host with a simple
//! editor and against one without.

use api::{
    EditorCommand, EditorState, HeadlessHost, Host, PhaseName, ToolName, codes, describe, handle,
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
                lock: None,
                measurement_label: "Length".into(),
                measurement: None,
                typing: false,
                soften_angle: 20,
                dialog: None,
            },
            quit: false,
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
            EditorCommand::Cancel => self.state.phase = PhaseName::Idle,
        }
        Ok(())
    }

    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.document().check_unsaved(discard)?;
        self.quit = true;
        Ok(())
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

    let state = call(&mut h, "query", json!({"type": "editor"}))["result"].clone();
    assert_eq!(state["type"], "editor");
    assert_eq!(state["tool"], "push_pull");
    assert_eq!(state["viewport"], json!([1280.0, 800.0]));
    assert_eq!(state["dialog"], Value::Null);
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
    for name in ["EditorCommand", "EditorState", "EditorQuery"] {
        assert!(d["schemas"][name].is_object(), "{name}");
    }
}
