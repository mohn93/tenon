//! The agent API's view of the app, in the window or headless: commands go through the Editor
//! (so the active tool stays consistent), editor commands set it, and `render_view` uses an
//! offscreen renderer.

use api::{Dialog, EditorCommand, EditorState, Host, PhaseName, ToolName};
use render::Snapshotter;
use session::{BatchError, Camera, Command, CommandError, Document, Lighting, Outcome};
use tools::Phase;

use crate::editor::Editor;

/// The offscreen renderer `render_view` uses: one on the window's GPU device, or, without a
/// window, one made on first use (None when there is no GPU).
pub struct Offscreen {
    snapshotter: Option<Snapshotter>,
    tried: bool,
}

impl Offscreen {
    /// `snapshotter`, already made.
    pub fn with(snapshotter: Snapshotter) -> Self {
        Offscreen {
            snapshotter: Some(snapshotter),
            tried: true,
        }
    }

    /// One made on its own GPU device when first needed.
    pub fn headless() -> Self {
        Offscreen {
            snapshotter: None,
            tried: false,
        }
    }

    fn get(&mut self) -> Option<&mut Snapshotter> {
        if !self.tried {
            self.tried = true;
            self.snapshotter = Snapshotter::new_headless();
        }
        self.snapshotter.as_mut()
    }
}

pub struct AppHost<'a> {
    pub editor: &'a mut Editor,
    pub offscreen: &'a mut Offscreen,
    /// Set when an agent asks the app to quit.
    pub quit: &'a mut bool,
}

impl Host for AppHost<'_> {
    fn document(&self) -> &Document {
        self.editor.doc()
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.editor.execute(command)
    }

    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.editor.execute_batch(commands, atomic)
    }

    fn render_view(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        let doc = self.editor.doc();
        self.offscreen.get()?.capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            camera,
            lighting,
            width,
            height,
        )
    }

    fn editor(&self) -> Option<EditorState> {
        Some(editor_state(self.editor))
    }

    fn editor_command(&mut self, command: EditorCommand) -> Result<(), CommandError> {
        match command {
            EditorCommand::SetTool { tool } => self.editor.select_tool(tool.into()),
            EditorCommand::SetPaintMaterial { material } => {
                self.editor.set_paint_material(material)?
            }
            EditorCommand::SetImportUnits { units } => self.editor.set_import_units(units),
            EditorCommand::Cancel => self.editor.cancel(),
        }
        Ok(())
    }

    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.editor.doc().check_unsaved(discard)?;
        *self.quit = true;
        Ok(())
    }
}

/// The editor's state as agents see it.
pub fn editor_state(editor: &Editor) -> EditorState {
    let status = editor.status_parts();
    let (phase, points) = match editor.phase() {
        Phase::Idle => (PhaseName::Idle, Vec::new()),
        Phase::Active { points } => (
            PhaseName::Active,
            points.iter().map(|p| p.to_array()).collect(),
        ),
    };
    let typed = editor.typed();
    let (soften_angle, soften_open) = editor.soften_state();
    let viewport = editor.doc().camera.viewport;
    EditorState {
        tool: ToolName::from(editor.tool_kind()),
        phase,
        points,
        paint_material: editor.current_material(),
        import_units: editor.import_units(),
        flying: editor.flying(),
        fly_speed: editor.flying_speed(),
        viewport: viewport.to_array(),
        status: status.to_string(),
        prompt: status.prompt.clone(),
        message: status.message.clone(),
        lock: status.lock.map(str::to_string),
        measurement_label: status.label.to_string(),
        measurement: if typed.is_empty() {
            status.value.clone()
        } else {
            Some(typed.to_string())
        },
        typing: !typed.is_empty(),
        soften_angle: soften_angle.into(),
        dialog: soften_open.then_some(Dialog::SoftenEdges),
    }
}

#[cfg(test)]
mod tests {
    use api::{ToolName, handle};
    use glam::{DVec2, DVec3};
    use serde_json::{Value, json};
    use session::ImportUnits;
    use tools::ToolKind;

    use super::*;
    use crate::editor::EditorKey;
    use crate::navigation::Button;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// Sends one request through an `AppHost` and returns the reply.
    fn call(e: &mut Editor, quit: &mut bool, method: &str, params: Value) -> Value {
        let mut offscreen = Offscreen::headless();
        let mut host = AppHost {
            editor: e,
            offscreen: &mut offscreen,
            quit,
        };
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        serde_json::from_str(&handle(&mut host, &line.to_string()).unwrap()).unwrap()
    }

    #[test]
    fn the_state_shows_the_tool_its_points_and_the_measurements_box() {
        let mut e = editor();
        let a = DVec3::new(1.0, 1.0, 0.0);
        let px = e.doc().camera.project(a).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        e.key(EditorKey::Char('2'));
        let s = editor_state(&e);
        assert_eq!(s.tool, ToolName::Line);
        assert_eq!(s.phase, PhaseName::Active);
        assert_eq!(s.points.len(), 1);
        assert!(DVec3::from(s.points[0]).distance(a) < 1e-9);
        assert_eq!((s.typing, s.measurement.as_deref()), (true, Some("2")));
        assert_eq!(s.measurement_label, "Length");
        assert_eq!(s.prompt, "Click the next point, or type a length");
        assert!(s.status.starts_with("Line  |  "), "{}", s.status);
        assert_eq!(s.viewport, [800.0, 600.0]);
        assert_eq!((s.soften_angle, s.dialog), (20, None));
        e.show_soften(true);
        assert_eq!(editor_state(&e).dialog, Some(Dialog::SoftenEdges));
    }

    #[test]
    fn editor_commands_reach_the_editor() {
        let (mut e, mut quit) = (editor(), false);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_tool", "tool": "tape_measure"}),
        );
        assert_eq!(r["result"]["editor"]["tool"], "tape_measure", "{r}");
        assert_eq!(e.tool_kind(), ToolKind::TapeMeasure);
        call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_import_units", "units": "inches"}),
        );
        assert_eq!(e.import_units(), ImportUnits::Inches);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_paint_material", "material": "m42"}),
        );
        assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
        let r = call(&mut e, &mut quit, "query", json!({"type": "editor"}));
        assert_eq!(r["result"]["import_units"], "inches");
        assert!(!quit);
    }

    #[test]
    fn quit_is_refused_with_unsaved_changes_unless_discarded() {
        let (mut e, mut quit) = (editor(), false);
        e.execute(Command::AddEdge {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let r = call(&mut e, &mut quit, "quit", Value::Null);
        assert_eq!(r["error"]["data"]["kind"], "unsaved_changes", "{r}");
        assert!(!quit);
        let r = call(&mut e, &mut quit, "quit", json!({"discard": true}));
        assert_eq!(r["result"], json!({}));
        assert!(quit);
    }
}
